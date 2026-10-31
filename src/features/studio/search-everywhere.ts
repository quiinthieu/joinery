import type {
  ConnectionProfile,
  DatabaseObjectKind,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  Environment,
  HistoryEntry,
  SessionInfo,
  TableInfo,
} from "../../ipc/generated";
import type { TabKind } from "../../state/studio";
import {
  databaseObjectKey,
  formatRoutinePrimaryLabel,
  tableInfoKindToObjectKind,
} from "../connections/explorer-tree";

export type SearchScope = "all" | "objects" | "actions" | "history";

export const SEARCH_SCOPES: readonly { id: SearchScope; label: string }[] = [
  { id: "all", label: "All" },
  { id: "objects", label: "Objects" },
  { id: "actions", label: "Actions" },
  { id: "history", label: "History" },
];

export const MAX_SEARCH_RESULTS = 80;

const OBJECT_KIND_LABELS: Record<DatabaseObjectKind, string> = {
  table: "Table",
  partitionedTable: "Partitioned table",
  view: "View",
  materializedView: "Materialized view",
  function: "Function",
  procedure: "Procedure",
  sequence: "Sequence",
  trigger: "Trigger",
  index: "Index",
};

const OBJECT_KIND_PRIORITY: Record<DatabaseObjectKind, number> = {
  table: 8,
  partitionedTable: 8,
  view: 7,
  materializedView: 7,
  function: 6,
  procedure: 6,
  sequence: 5,
  trigger: 4,
  index: 3,
};

export interface SearchCommandInput<TIcon = unknown> {
  label: string;
  shortcut: string;
  icon: TIcon;
  keepOpen?: boolean;
  action(): unknown;
}

export interface SearchTabInput {
  id: string;
  title: string;
  kind: TabKind;
  connectionId: string | null;
}

export type SearchResultItem<TIcon = unknown> =
  | {
      id: string;
      kind: "command";
      label: string;
      subtitle: string | null;
      shortcut: string;
      icon: TIcon;
      keepOpen: boolean;
      action(): unknown;
    }
  | {
      id: string;
      kind: "tab";
      tabId: string;
      tabKind: TabKind;
      label: string;
      subtitle: string | null;
      shortcut: string;
      isActive: boolean;
    }
  | {
      id: string;
      kind: "object";
      connectionId: string;
      connectionName: string;
      environment: Environment | null;
      object: DatabaseObjectRef;
      label: string;
      subtitle: string;
      shortcut: string;
    }
  | {
      id: string;
      kind: "history";
      entry: HistoryEntry;
      label: string;
      subtitle: string;
      shortcut: string;
    };

export interface SearchEverywhereInput<TIcon = unknown> {
  query: string;
  scope: SearchScope;
  commands: readonly SearchCommandInput<TIcon>[];
  tabs: readonly SearchTabInput[];
  activeTabId: string | null;
  preferredConnectionId: string | null;
  profiles: readonly ConnectionProfile[];
  sessions: Readonly<Record<string, SessionInfo | undefined>>;
  catalog: Readonly<Record<string, readonly TableInfo[] | undefined>>;
  objects: Readonly<Record<string, readonly DatabaseObjectSummary[] | undefined>>;
  history: readonly HistoryEntry[];
  limit?: number;
}

export interface SearchEverywhereOutput<TIcon = unknown> {
  items: SearchResultItem<TIcon>[];
  connectedConnectionCount: number;
  offlineConnectionCount: number;
}

interface ScoredCandidate<TIcon> {
  item: SearchResultItem<TIcon>;
  score: number;
  tieOrder: number;
}

function splitIdentifierWords(value: string): string[] {
  return value
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean);
}

function scoreTextMatch(
  queryLower: string,
  tokens: readonly string[],
  primary: string,
  qualified: string | null,
  rawName: string | null,
  haystack: string,
): number | null {
  const haystackLower = haystack.toLowerCase();
  for (const token of tokens) {
    if (!haystackLower.includes(token)) return null;
  }
  const primaryLower = primary.toLowerCase();
  if (primaryLower === queryLower) return 1000;
  if (rawName && rawName.toLowerCase() === queryLower) return 940;
  const qualifiedLower = qualified?.toLowerCase() ?? null;
  if (qualifiedLower === queryLower) return 920;
  if (primaryLower.startsWith(queryLower)) return 820;
  if (rawName && rawName.toLowerCase().startsWith(queryLower)) return 780;
  if (qualifiedLower && qualifiedLower.startsWith(queryLower)) return 720;

  const words = splitIdentifierWords(primaryLower);
  if (words.some((w) => w === queryLower)) return 640;
  if (words.some((w) => w.startsWith(queryLower))) return 560;
  if (primaryLower.includes(queryLower)) return 440;
  if (qualifiedLower && qualifiedLower.includes(queryLower)) return 360;
  return 260;
}

function formatHistorySingleLine(sql: string | null, statementCount: number): string {
  if (!sql) {
    return `${statementCount} statement${statementCount === 1 ? "" : "s"} · SQL text not stored`;
  }
  const collapsed = sql.replace(/\s+/g, " ").trim();
  return collapsed.length > 110 ? `${collapsed.slice(0, 109)}…` : collapsed;
}

export function buildSearchEverywhereResults<TIcon = unknown>(
  input: SearchEverywhereInput<TIcon>,
): SearchEverywhereOutput<TIcon> {
  const limit = input.limit ?? MAX_SEARCH_RESULTS;
  const trimmed = input.query.trim();
  const queryLower = trimmed.toLowerCase();
  const tokens = queryLower ? queryLower.split(/\s+/).filter(Boolean) : [];

  const profileById = new Map<string, ConnectionProfile>();
  for (const profile of input.profiles) {
    profileById.set(profile.id, profile);
  }

  let connectedConnectionCount = 0;
  let offlineConnectionCount = 0;
  for (const profile of input.profiles) {
    if (input.sessions[profile.id]) connectedConnectionCount += 1;
    else offlineConnectionCount += 1;
  }
  const connectedIds: string[] = input.profiles
    .filter((p) => Boolean(input.sessions[p.id]))
    .map((p) => p.id);
  for (const connId of Object.keys(input.sessions)) {
    if (input.sessions[connId] && !profileById.has(connId)) {
      connectedIds.push(connId);
      connectedConnectionCount += 1;
    }
  }

  const includeActions = input.scope === "all" || input.scope === "actions";
  const includeObjects = input.scope === "all" || input.scope === "objects";
  const includeHistory = input.scope === "all" || input.scope === "history";

  const actionItems: {
    item: SearchResultItem<TIcon>;
    primary: string;
    haystack: string;
  }[] = [];

  if (includeActions) {
    for (const command of input.commands) {
      actionItems.push({
        item: {
          id: `cmd:${command.label}`,
          kind: "command",
          label: command.label,
          subtitle: null,
          shortcut: command.shortcut,
          icon: command.icon,
          keepOpen: Boolean(command.keepOpen),
          action: command.action,
        },
        primary: command.label,
        haystack: `${command.label} ${command.shortcut}`,
      });
    }

    for (const tab of input.tabs) {
      // Utility singleton tabs ("Grid benchmark", "Query history") already have
      // dedicated top-level commands that focus them without duplicating labels.
      if (tab.kind === "benchmark" || tab.kind === "history") continue;
      if (!queryLower && input.scope === "all" && tab.id === input.activeTabId) continue;
      const connName = tab.connectionId
        ? (profileById.get(tab.connectionId)?.name ?? tab.connectionId)
        : null;
      const isActive = tab.id === input.activeTabId;
      const label = `Switch to tab: ${tab.title}`;
      actionItems.push({
        item: {
          id: `tab:${tab.id}`,
          kind: "tab",
          tabId: tab.id,
          tabKind: tab.kind,
          label,
          subtitle: connName,
          shortcut: isActive ? "Active tab" : "Open tab",
          isActive,
        },
        primary: tab.title,
        haystack: `${label} ${tab.title} ${connName ?? ""} ${tab.kind}`,
      });
    }
  }

  const objectItems: {
    item: Extract<SearchResultItem<TIcon>, { kind: "object" }>;
    primary: string;
    qualified: string;
    rawName: string;
    haystack: string;
  }[] = [];

  if (includeObjects) {
    for (const connId of connectedIds) {
      const profile = profileById.get(connId);
      const connectionName = profile?.name ?? connId;
      const environment = profile?.environment ?? null;
      const seen = new Set<string>();

      for (const entry of input.catalog[connId] ?? []) {
        const kind = tableInfoKindToObjectKind(entry.kind);
        const object: DatabaseObjectRef = {
          schema: entry.table.schema,
          name: entry.table.name,
          kind,
          identity: null,
        };
        const key = databaseObjectKey(object);
        if (seen.has(key)) continue;
        seen.add(key);
        const kindLabel = OBJECT_KIND_LABELS[kind];
        const subtitle = `${connectionName} · ${object.schema}`;
        const qualified = `${object.schema}.${object.name}`;
        objectItems.push({
          item: {
            id: `obj:${connId}:${key}`,
            kind: "object",
            connectionId: connId,
            connectionName,
            environment,
            object,
            label: object.name,
            subtitle,
            shortcut: kindLabel,
          },
          primary: object.name,
          qualified,
          rawName: object.name,
          haystack: `${qualified} ${object.name} ${connectionName} ${kindLabel}`,
        });
      }

      for (const summary of input.objects[connId] ?? []) {
        const object: DatabaseObjectRef = {
          schema: summary.object.schema,
          name: summary.object.name,
          kind: summary.object.kind,
          identity: summary.object.identity ?? null,
        };
        const key = databaseObjectKey(object);
        if (seen.has(key)) continue;
        seen.add(key);
        const isRoutine = object.kind === "function" || object.kind === "procedure";
        const label = isRoutine
          ? formatRoutinePrimaryLabel(object, summary.signature)
          : object.name;
        const kindLabel = OBJECT_KIND_LABELS[object.kind];
        const ownerPart = summary.ownerTable ? ` · on ${summary.ownerTable}` : "";
        const subtitle = `${connectionName} · ${object.schema}${ownerPart}`;
        const qualified = `${object.schema}.${label}`;
        objectItems.push({
          item: {
            id: `obj:${connId}:${key}`,
            kind: "object",
            connectionId: connId,
            connectionName,
            environment,
            object,
            label,
            subtitle,
            shortcut: kindLabel,
          },
          primary: label,
          qualified,
          rawName: object.name,
          haystack: [
            qualified,
            `${object.schema}.${object.name}`,
            label,
            summary.ownerTable ?? "",
            summary.signature ?? "",
            connectionName,
            kindLabel,
          ].join(" "),
        });
      }
    }
  }

  const historyItems: {
    item: Extract<SearchResultItem<TIcon>, { kind: "history" }>;
    primary: string;
    haystack: string;
  }[] = [];

  if (includeHistory) {
    for (const entry of input.history) {
      if (!queryLower && input.scope === "all" && !entry.sql) continue;
      const preview = formatHistorySingleLine(entry.sql, entry.statementCount);
      historyItems.push({
        item: {
          id: `hist:${entry.id}`,
          kind: "history",
          entry,
          label: preview,
          subtitle: `${entry.connectionName} · ${entry.elapsedMs} ms`,
          shortcut: "History",
        },
        primary: preview,
        haystack: `${entry.sql ?? ""} ${entry.connectionName} history`,
      });
    }
  }

  if (!queryLower) {
    const out: SearchResultItem<TIcon>[] = [];
    if (input.scope === "all") {
      for (const a of actionItems) {
        if (out.length >= limit) break;
        out.push(a.item);
      }
      const objectCap = Math.min(limit - out.length, 24);
      for (let i = 0; i < objectItems.length && i < objectCap; i += 1) {
        out.push(objectItems[i].item);
      }
      for (const h of historyItems) {
        if (out.length >= limit) break;
        out.push(h.item);
      }
    } else if (input.scope === "actions") {
      for (const a of actionItems) {
        if (out.length >= limit) break;
        out.push(a.item);
      }
    } else if (input.scope === "objects") {
      for (const o of objectItems) {
        if (out.length >= limit) break;
        out.push(o.item);
      }
    } else {
      for (const h of historyItems) {
        if (out.length >= limit) break;
        out.push(h.item);
      }
    }
    return {
      items: out,
      connectedConnectionCount,
      offlineConnectionCount,
    };
  }

  const scored: ScoredCandidate<TIcon>[] = [];
  let tieOrder = 0;

  for (const a of actionItems) {
    const base = scoreTextMatch(queryLower, tokens, a.primary, null, null, a.haystack);
    if (base === null) continue;
    scored.push({
      item: a.item,
      score: base + (a.item.kind === "command" ? 12 : 8),
      tieOrder: tieOrder++,
    });
  }

  for (const o of objectItems) {
    const base = scoreTextMatch(queryLower, tokens, o.primary, o.qualified, o.rawName, o.haystack);
    if (base === null) continue;
    const preferredBoost =
      input.preferredConnectionId && o.item.connectionId === input.preferredConnectionId ? 15 : 0;
    const kindBoost = OBJECT_KIND_PRIORITY[o.item.object.kind];
    scored.push({
      item: o.item,
      score: base + preferredBoost + kindBoost,
      tieOrder: tieOrder++,
    });
  }

  for (const h of historyItems) {
    const base = scoreTextMatch(queryLower, tokens, h.primary, null, null, h.haystack);
    if (base === null) continue;
    scored.push({
      item: h.item,
      score: base + (h.item.entry.sql ? 4 : 0),
      tieOrder: tieOrder++,
    });
  }

  scored.sort((left, right) => {
    if (right.score !== left.score) return right.score - left.score;
    return left.tieOrder - right.tieOrder;
  });

  return {
    items: scored.slice(0, limit).map((entry) => entry.item),
    connectedConnectionCount,
    offlineConnectionCount,
  };
}
