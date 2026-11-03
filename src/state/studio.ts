import type { SqlExecutionOrigin } from "../features/studio/sql-navigation";
import { create } from "zustand";
import type {
  AppError,
  BookmarkTarget,
  Bootstrap,
  BrowseApproval,
  BrowseQuery,
  ConnectionProfile,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  OpenedSqlFile,
  PreparedPlan,
  QueryEvent,
  ResultInfo,
  SavedBookmark,
  SavedQuery,
  SavedQueryInput,
  SavedSqlFile,
  SavedTableLayout,
  SessionContextOptions,
  SessionInfo,
  SessionRuntimeState,
  SqlFormatSettings,
  SqlSnippet,
  SqlSnippetInput,
  TableInfo,
  TableRef,
  TableSchema,
  TransactionMode,
} from "../ipc/generated";
import { client, messageOf } from "../ipc/client";
import { DEFAULT_SQL_FORMAT_SETTINGS } from "../data/sql-format";
import { resultCache } from "../data/result-cache";
import { dirtyStore } from "../data/dirty-store";
import {
  databaseObjectKey,
  isTabularObjectKind,
  objectTabTitle,
  sameDatabaseObject,
  tableInfoKindToObjectKind,
} from "../features/connections/explorer-tree";
import {
  MAX_TABLE_LAYOUTS,
  isDefaultTableLayout,
  sameTableLayoutTarget,
} from "../features/grid/table-layout";
import {
  EMPTY_QUERY,
  hasRawClauses,
  isEmptyQuery,
  normalizeQuery,
  sameQuery,
  tableTabTitle,
} from "../features/grid/table-query";

import { planSnapshots, type PlanSource } from "../features/studio/plan-snapshots";
import {
  SHORTCUT_BY_ID,
  sanitizeShortcutOverrides,
  validateShortcutOverride,
  type ShortcutActionId,
  type ShortcutValidationResult,
} from "../features/studio/shortcuts";

export type TabKind =
  "query" | "table" | "object" | "benchmark" | "history" | "processes" | "diagram" | "schemaDiff";
export type TabStatus =
  "idle" | "queued" | "running" | "completed" | "failed" | "cancelled" | "outcomeUnknown";
export interface TablePageState {
  start: number;
  result: ResultInfo;
  after: (string | null)[] | null;
}
export interface MetadataOrigin {
  tabId: string;
  sessionId: string;
  contextRevision: number;
  catalogRevision?: string;
}
export interface StudioTab {
  metadataOrigin?: MetadataOrigin;
  errorOrigin?: SqlExecutionOrigin;
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  results: ResultInfo[];
  planSource?: PlanSource;
  activeResultId: string | null;
  status: TabStatus;
  runId: string | null;
  elapsedMs: number;
  runMetrics?: Extract<QueryEvent, { type: "runMetrics" }>;
  clientStartedAt?: number;
  clientElapsedMs?: number;
  error: AppError | null;
  table: TableRef | null;
  object: DatabaseObjectRef | null;
  /** Server-side sort/filter for table tabs; never persisted. */
  query: BrowseQuery;
  schema: TableSchema | null;
  pages: TablePageState[];
  pageKeys: Record<number, (string | null)[] | null>;
  loadedRows: number;
  hasMore: boolean;
  nextKey: (string | null)[] | null;
  loadingPage: boolean;
  view: "data" | "structure" | "ddl";
  highlightedColumn?: string | null;
  revision: number;
  metrics: Record<number, { affectedRows: string; elapsedMs: number }>;
  dataSessionId: string | null;
  querySession: SessionInfo | null;
  querySessionClosed: boolean;
  fileToken: string | null;
  fileName: string | null;
  savedSql: string | null;
  savedQueryId: string | null;
  /** Whether the tab is pinned in the workspace tab strip. */
  pinned?: boolean;
  /** Schema shown by a diagram tab. */
  diagramSchema?: string;
}

export const MAX_RECENT_LOCATIONS = 50;
export const MAX_CLOSED_TABS = 10;

export interface ClosedTabSnapshot {
  id: string;
  kind: TabKind;
  title: string;
  connectionId: string | null;
  sql: string;
  table: TableRef | null;
  object: DatabaseObjectRef | null;
  query: BrowseQuery;
  view: StudioTab["view"];
  savedQueryId: string | null;
  pinned: boolean;
  diagramSchema?: string;
  closedAtMs: number;
}

export type RecentLocationTarget =
  | { kind: "table"; connectionId: string; table: TableRef }
  | { kind: "object"; connectionId: string; object: DatabaseObjectRef }
  | { kind: "savedQuery"; savedQueryId: string }
  | { kind: "tab"; tabId: string };

export interface RecentLocation {
  key: string;
  label: string;
  subtitle: string;
  visitedAtMs: number;
  target: RecentLocationTarget;
}

export type OpenLocationOutcome =
  { kind: "opened" } | { kind: "needsConnect"; profile: ConnectionProfile } | { kind: "missing" };

export type TableQueryOutcome =
  | { kind: "applied" }
  | { kind: "refused"; reason: string }
  | { kind: "review"; plan: PreparedPlan; query: BrowseQuery };

export interface NamedSession {
  info: SessionInfo;
  label: string;
  primary: boolean;
}

let sqlFormatSaveTail: Promise<void> = Promise.resolve();
let sqlFormatSaveSequence = 0;
let sqlFormatPending = 0;
let sqlFormatPersisted = DEFAULT_SQL_FORMAT_SETTINGS;

interface StudioState {
  bootstrap: Bootstrap | null;
  error: string | null;
  sessions: Record<string, SessionInfo>;
  sessionPool: Record<string, NamedSession[]>;
  sessionStates: Record<string, SessionRuntimeState>;
  sessionContexts: Record<string, string[]>;
  sessionContextRevision: Record<string, number>;
  catalog: Record<string, TableInfo[]>;
  objects: Record<string, DatabaseObjectSummary[]>;
  selectedConnectionId: string | null;
  tabs: StudioTab[];
  activeTabId: string | null;
  closedTabs: ClosedTabSnapshot[];
  tableLayouts: SavedTableLayout[];
  savedQueries: SavedQuery[];
  bookmarks: SavedBookmark[];
  sqlFormatSettings: SqlFormatSettings;
  sqlSnippets: SqlSnippet[];
  recentLocations: RecentLocation[];
  shortcuts: Record<string, string>;
  theme: "dark" | "light" | "contrast";
  density: "comfortable" | "compact";
  sidebarVisible: boolean;
  sidebarCollapsed: boolean;
  inspectorVisible: boolean;
  toast: string | null;
  initialize(): Promise<void>;
  notify(message: string): void;
  setTheme(theme: StudioState["theme"]): void;
  toggleSidebar(): void;
  toggleInspector(): void;
  setShortcut(actionId: ShortcutActionId, binding: string | null): ShortcutValidationResult;
  resetShortcuts(): void;
  setTableLayout(connectionId: string, table: TableRef, layout: SavedTableLayout | null): void;
  resetTableLayout(connectionId: string, table: TableRef): void;
  setSession(session: SessionInfo): void;
  bindQuerySession(tabId: string, session: SessionInfo, label?: string): boolean;
  attachQuerySession(tabId: string, sessionId: string | null): boolean;
  renameQuerySession(sessionId: string, label: string): void;
  disconnectQuerySession(sessionId: string): Promise<void>;
  updateSessionLatency(sessionId: string, latencyMs: number): void;
  refreshSessionContexts(sessionId: string): Promise<void>;
  switchSessionContext(sessionId: string, target: string): Promise<void>;
  setTransactionMode(sessionId: string, mode: TransactionMode): Promise<void>;
  commitTransaction(sessionId: string): Promise<void>;
  rollbackTransaction(sessionId: string): Promise<void>;
  retireSession(sessionId: string): void;
  updateProfile(profile: ConnectionProfile): void;
  selectConnection(id: string): void;
  refreshCatalog(id: string): Promise<void>;
  openQuery(connectionId?: string | null, sql?: string): string;
  openSqlFile(connectionId?: string | null): Promise<string | null>;
  saveSqlFile(tabId: string, options?: { saveAs?: boolean }): Promise<boolean>;
  saveQueryToLibrary(input: SavedQueryInput, attachTabId?: string): Promise<SavedQuery>;
  deleteSavedQuery(id: string): Promise<void>;
  openSavedQuery(id: string): string | null;
  toggleBookmark(target: BookmarkTarget, label: string): Promise<boolean>;
  deleteBookmark(id: string): Promise<void>;
  saveSqlFormatSettings(settings: SqlFormatSettings): Promise<SqlFormatSettings>;
  saveSqlSnippet(input: SqlSnippetInput): Promise<SqlSnippet>;
  deleteSqlSnippet(id: string): Promise<void>;
  openLocation(target: RecentLocationTarget): Promise<OpenLocationOutcome>;
  recordRecentLocation(entry: Omit<RecentLocation, "visitedAtMs">): void;
  openUtility(kind: "history" | "processes" | "benchmark"): void;
  openDiagram(connectionId: string, schema: string): void;
  openSchemaDiff(connectionId: string, schema: string): void;
  openTable(
    connectionId: string,
    table: TableRef,
    query?: BrowseQuery,
    preferredView?: StudioTab["view"],
    highlightedColumn?: string | null,
  ): Promise<void>;
  openObject(
    connectionId: string,
    object: DatabaseObjectRef,
    preferredView?: StudioTab["view"],
    highlightedColumn?: string | null,
  ): Promise<void>;
  openSessionObject(
    tabId: string,
    object: DatabaseObjectRef,
    highlightedColumn?: string,
    catalogRevision?: string,
  ): void;
  openLinkedSessionObject(inspectorId: string, object: DatabaseObjectRef): void;
  appendSessionDdl(inspectorId: string, ddl: string): boolean;
  /** Applies a new sort/filter or requests review for non-Read raw SQL clauses. */
  setTableQuery(
    id: string,
    query: BrowseQuery,
    options?: { force?: boolean; approval?: BrowseApproval },
  ): Promise<TableQueryOutcome>;
  loadNextPage(id: string, approval?: BrowseApproval): Promise<void>;
  ensureTablePage(id: string, row: number): Promise<void>;
  refreshTable(id: string, approval?: BrowseApproval): Promise<void>;
  updateTab(
    id: string,
    update: Partial<StudioTab> | ((tab: StudioTab) => Partial<StudioTab>),
  ): void;
  togglePinTab(id: string): void;
  closeTab(id: string): void;
  closeOtherTabs(keepTabId: string): number;
  closeCleanTabs(): number;
  reopenClosedTab(): Promise<string | null>;
  removeConnection(id: string): void;
  disconnected(connectionId: string): void;
  resetConnectionResults(connectionId: string): void;
  applyEvents(tabId: string, events: QueryEvent[], revision: number): void;
}

let toastTimer: ReturnType<typeof setTimeout> | undefined;
function tab(kind: TabKind, title: string, connectionId: string | null): StudioTab {
  return {
    id: crypto.randomUUID(),
    kind,
    title,
    connectionId,
    sql: "",
    results: [],
    activeResultId: null,
    status: "idle",
    runId: null,
    elapsedMs: 0,
    runMetrics: undefined,
    clientStartedAt: undefined,
    clientElapsedMs: undefined,
    error: null,
    table: null,
    object: null,
    query: EMPTY_QUERY,
    schema: null,
    pages: [],
    pageKeys: {},
    loadedRows: 0,
    hasMore: false,
    nextKey: null,
    loadingPage: false,
    view: "data",
    highlightedColumn: null,
    revision: 0,
    metrics: {},
    dataSessionId: null,
    querySession: null,
    querySessionClosed: false,
    fileToken: null,
    fileName: null,
    savedSql: null,
    savedQueryId: null,
    pinned: false,
  };
}
export function isTabFileDirty(tab: StudioTab): boolean {
  return tab.kind === "query" && tab.fileName !== null && tab.sql !== tab.savedSql;
}
export function isTabProtectedFromBulkClose(
  allTabs: readonly StudioTab[],
  candidate: StudioTab,
): boolean {
  const resultIds = [
    ...candidate.results.map((r) => r.id),
    ...candidate.pages.map((p) => p.result.id),
  ];
  return (
    dirtyStore.pendingCountForTab(candidate.id, resultIds) > 0 ||
    candidate.results.some((r) => dirtyStore.pendingCountForTab(r.id, [r.id]) > 0) ||
    isTabFileDirty(candidate) ||
    ["queued", "running"].includes(candidate.status) ||
    isLastOwnerOfSecondarySession(allTabs, candidate)
  );
}
function snapshotClosedTab(candidate: StudioTab): ClosedTabSnapshot | null {
  if (candidate.metadataOrigin) return null;
  return {
    id: candidate.id,
    kind: candidate.kind,
    title: candidate.title,
    connectionId: candidate.connectionId,
    sql: candidate.sql,
    table: candidate.table,
    object: candidate.object,
    query: candidate.query,
    view: candidate.view,
    savedQueryId: candidate.savedQueryId,
    pinned: Boolean(candidate.pinned),
    diagramSchema: candidate.diagramSchema,
    closedAtMs: Date.now(),
  };
}
function orderPinnedFirst(tabs: readonly StudioTab[]): StudioTab[] {
  const pinned: StudioTab[] = [];
  const unpinned: StudioTab[] = [];
  for (const entry of tabs) {
    if (entry.pinned) pinned.push(entry);
    else unpinned.push(entry);
  }
  return [...pinned, ...unpinned];
}
export function bookmarkTargetKey(target: BookmarkTarget): string {
  if (target.kind === "savedQuery") return `saved:${target.savedQueryId}`;
  return `obj:${target.connectionId}:${databaseObjectKey(target.object)}`;
}
export function findBookmarkForTarget(
  bookmarks: readonly SavedBookmark[],
  target: BookmarkTarget,
): SavedBookmark | undefined {
  const key = bookmarkTargetKey(target);
  return bookmarks.find((entry) => bookmarkTargetKey(entry.target) === key);
}
export function bookmarkTargetForTab(
  tab: StudioTab,
  catalog: Readonly<Record<string, readonly TableInfo[] | undefined>>,
): { target: BookmarkTarget; defaultLabel: string } | null {
  if (tab.metadataOrigin) return null;
  if (tab.kind === "table" && tab.connectionId && tab.table) {
    const match = catalog[tab.connectionId]?.find(
      (entry) => entry.table.schema === tab.table!.schema && entry.table.name === tab.table!.name,
    );
    const kind = match ? tableInfoKindToObjectKind(match.kind) : "table";
    return {
      target: {
        kind: "object",
        connectionId: tab.connectionId,
        object: {
          schema: tab.table.schema,
          name: tab.table.name,
          kind,
          identity: null,
        },
      },
      defaultLabel: `${tab.table.schema}.${tab.table.name}`,
    };
  }
  if (tab.kind === "object" && tab.connectionId && tab.object) {
    return {
      target: {
        kind: "object",
        connectionId: tab.connectionId,
        object: tab.object,
      },
      defaultLabel: `${tab.object.schema}.${objectTabTitle(tab.object)}`,
    };
  }
  if (tab.kind === "query" && tab.savedQueryId) {
    return {
      target: {
        kind: "savedQuery",
        savedQueryId: tab.savedQueryId,
      },
      defaultLabel: tab.title,
    };
  }
  return null;
}
export function querySessionForTab(tab: StudioTab, sessions: Record<string, SessionInfo>) {
  if (tab.metadataOrigin) return undefined;
  if (tab.kind === "query" && tab.querySession)
    return tab.querySessionClosed ? undefined : tab.querySession;
  return tab.connectionId ? sessions[tab.connectionId] : undefined;
}
/** Metadata never adopts another session after its source console is closed or rebound. */
export function metadataSessionForTab(
  tab: StudioTab,
  state: Pick<StudioState, "tabs" | "sessions" | "sessionContextRevision">,
): SessionInfo | undefined {
  const origin = tab.metadataOrigin;
  if (!origin) return undefined;
  const console = state.tabs.find((entry) => entry.id === origin.tabId && entry.kind === "query");
  const session = console && querySessionForTab(console, state.sessions);
  return session?.id === origin.sessionId &&
    session.connectionId === tab.connectionId &&
    (state.sessionContextRevision[session.id] ?? 0) === origin.contextRevision
    ? session
    : undefined;
}
export function isLastOwnerOfSecondarySession(tabs: readonly StudioTab[], tab: StudioTab): boolean {
  if (!tab.querySession || tab.querySessionClosed) return false;
  const sessionId = tab.querySession.id;
  return !tabs.some(
    (other) =>
      other.id !== tab.id && other.querySession?.id === sessionId && !other.querySessionClosed,
  );
}
export function sessionHasOpenTransaction(
  sessionStates: Readonly<Record<string, SessionRuntimeState | undefined>>,
  sessionId: string | null | undefined,
): boolean {
  if (!sessionId) return false;
  const txState = sessionStates[sessionId]?.transactionState;
  return txState === "inTransaction" || txState === "failed";
}
function isSessionActive(
  state: Pick<StudioState, "sessions" | "sessionPool" | "tabs">,
  sessionId: string,
): boolean {
  return (
    Object.values(state.sessions).some((s) => s.id === sessionId) ||
    Object.values(state.sessionPool).some((list) =>
      list.some((entry) => entry.info.id === sessionId),
    ) ||
    state.tabs.some((t) => t.querySession?.id === sessionId && !t.querySessionClosed)
  );
}
function normalizeSessionLabel(raw: string): string {
  // eslint-disable-next-line no-control-regex
  return raw
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 40);
}
function defaultSecondarySessionLabel(existing: readonly NamedSession[]): string {
  if (!existing.some((entry) => entry.label === "Isolated session")) return "Isolated session";
  let index = 2;
  while (existing.some((entry) => entry.label === `Isolated session ${index}`)) index += 1;
  return `Isolated session ${index}`;
}
function purgeSessionRuntimeMaps(
  state: Pick<StudioState, "sessionStates" | "sessionContexts" | "sessionContextRevision">,
  sessionIds: Iterable<string>,
) {
  const sessionStates = { ...state.sessionStates };
  const sessionContexts = { ...state.sessionContexts };
  const sessionContextRevision = { ...state.sessionContextRevision };
  for (const id of sessionIds) {
    delete sessionStates[id];
    delete sessionContexts[id];
    delete sessionContextRevision[id];
  }
  return { sessionStates, sessionContexts, sessionContextRevision };
}
const storedTheme = localStorage.getItem("joinery.theme");

function requestIsCurrent(id: string, revision: number, connectionId: string, sessionId: string) {
  const state = useStudio.getState();
  return (
    state.tabs.some((tab) => tab.id === id && tab.revision === revision) &&
    state.sessions[connectionId]?.id === sessionId
  );
}
function asAppError(error: unknown): AppError {
  if (error && typeof error === "object" && "code" in error && "message" in error)
    return error as AppError;
  return { code: "database", message: messageOf(error) };
}
function verifyPageColumns(schema: TableSchema | null, result: ResultInfo) {
  if (!schema || JSON.stringify(schema.columns) !== JSON.stringify(result.columns)) {
    void client.call("release_result", { resultId: result.id }).catch(() => {});
    throw new Error("The table schema changed. Refresh the table before continuing.");
  }
}

export const useStudio = create<StudioState>((set, get) => ({
  bootstrap: null,
  error: null,
  sessions: {},
  sessionPool: {},
  sessionStates: {},
  sessionContexts: {},
  sessionContextRevision: {},
  catalog: {},
  objects: {},
  selectedConnectionId: null,
  tabs: [],
  activeTabId: null,
  closedTabs: [],
  tableLayouts: [],
  savedQueries: [],
  bookmarks: [],
  sqlFormatSettings: DEFAULT_SQL_FORMAT_SETTINGS,
  sqlSnippets: [],
  recentLocations: [],
  shortcuts: {},
  theme: storedTheme === "light" || storedTheme === "contrast" ? storedTheme : "dark",
  density: "comfortable",
  sidebarVisible: true,
  sidebarCollapsed: false,
  inspectorVisible: false,
  toast: null,
  async initialize() {
    try {
      const bootstrap = await client.call<Bootstrap>("bootstrap");
      const workspace = !get().bootstrap ? bootstrap.workspace : null;
      const restored =
        workspace?.drafts
          .filter((draft) =>
            bootstrap.profiles.some(
              (profile) => profile.id === draft.connectionId && profile.storeDrafts,
            ),
          )
          .map((draft) => ({
            ...tab("query", draft.title, draft.connectionId),
            id: draft.id,
            sql: draft.sql,
            pinned: Boolean(draft.pinned),
          })) ?? [];
      const restoredLayouts =
        workspace?.tableLayouts
          ?.filter(
            (layout) =>
              !isDefaultTableLayout(layout) &&
              bootstrap.profiles.some((profile) => profile.id === layout.connectionId),
          )
          .slice(-MAX_TABLE_LAYOUTS) ?? [];
      const existing = get().tabs;
      const tabs = orderPinnedFirst([
        ...restored.filter((draft) => !existing.some((tab) => tab.id === draft.id)),
        ...existing,
      ]);
      const sidebarVisible = workspace
        ? workspace.sidebarCollapsed
          ? false
          : workspace.sidebarVisible
        : get().sidebarVisible;
      set({
        bootstrap,
        savedQueries: bootstrap.savedQueries ?? [],
        bookmarks: bootstrap.bookmarks ?? [],
        sqlFormatSettings: bootstrap.sqlFormatSettings ?? DEFAULT_SQL_FORMAT_SETTINGS,
        sqlSnippets: bootstrap.sqlSnippets ?? [],
        error: null,
        ...(workspace
          ? {
              theme: workspace.theme as StudioState["theme"],
              density: workspace.density as StudioState["density"],
              sidebarVisible,
              sidebarCollapsed: !sidebarVisible,
              inspectorVisible: workspace.inspectorVisible,
              shortcuts: sanitizeShortcutOverrides(
                (workspace.shortcuts ?? {}) as Record<string, string>,
              ),
              tabs,
              tableLayouts: restoredLayouts,
              activeTabId:
                get().activeTabId ??
                restored.find((draft) => draft.id === workspace.activeDraftId)?.id ??
                restored[0]?.id ??
                null,
            }
          : {}),
      });
    } catch (error) {
      set({ error: messageOf(error) });
    }
  },
  notify(message) {
    clearTimeout(toastTimer);
    set({ toast: message });
    toastTimer = setTimeout(() => set({ toast: null }), 6000);
  },
  setTheme(theme) {
    localStorage.setItem("joinery.theme", theme);
    document.documentElement.dataset.theme = theme;
    set({ theme });
  },
  toggleSidebar() {
    set((state) => {
      const sidebarVisible = !state.sidebarVisible;
      return { sidebarVisible, sidebarCollapsed: !sidebarVisible };
    });
  },
  toggleInspector() {
    set((state) => ({ inspectorVisible: !state.inspectorVisible }));
  },
  setShortcut(actionId, binding) {
    const currentOverrides = get().shortcuts;
    if (binding === null || binding.trim() === "") {
      const next = { ...currentOverrides };
      delete next[actionId];
      set({ shortcuts: next });
      const def = SHORTCUT_BY_ID[actionId];
      return { valid: true, canonical: def.defaultBinding };
    }
    const validation = validateShortcutOverride(actionId, binding, currentOverrides);
    if (!validation.valid || !validation.canonical) {
      return validation;
    }
    const next = { ...currentOverrides };
    if (validation.canonical === SHORTCUT_BY_ID[actionId].defaultBinding) {
      delete next[actionId];
    } else {
      next[actionId] = validation.canonical;
    }
    set({ shortcuts: next });
    return validation;
  },
  resetShortcuts() {
    set({ shortcuts: {} });
  },
  setTableLayout(connectionId, table, layout) {
    set((state) => {
      const remaining = state.tableLayouts.filter(
        (entry) => !sameTableLayoutTarget(entry, connectionId, table),
      );
      if (!layout || isDefaultTableLayout(layout)) return { tableLayouts: remaining };
      return { tableLayouts: [...remaining, layout].slice(-MAX_TABLE_LAYOUTS) };
    });
  },
  resetTableLayout(connectionId, table) {
    get().setTableLayout(connectionId, table, null);
  },
  setSession(session) {
    set((state) => ({
      sessions: { ...state.sessions, [session.connectionId]: session },
      selectedConnectionId: session.connectionId,
    }));
  },
  bindQuerySession(tabId, session, preferredLabel) {
    const current = get().tabs.find((t) => t.id === tabId);
    if (
      !current ||
      current.kind !== "query" ||
      current.connectionId !== session.connectionId ||
      !["postgres", "mysql"].includes(session.engine) ||
      ["queued", "running"].includes(current.status) ||
      (current.querySession && !current.querySessionClosed)
    ) {
      void client
        .call("disconnect", { sessionId: session.id })
        .catch((error) => get().notify(messageOf(error)));
      return false;
    }
    for (const result of current.results) {
      resultCache.release(result.id);
      dirtyStore.clearTab(result.id, [result.id]);
      void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    set((state) => {
      const existing = (state.sessionPool[session.connectionId] ?? []).filter(
        (entry) => entry.info.id !== session.id,
      );
      const normalized = preferredLabel ? normalizeSessionLabel(preferredLabel) : "";
      const label = normalized || defaultSecondarySessionLabel(existing);
      return {
        sessionPool: {
          ...state.sessionPool,
          [session.connectionId]: [...existing, { info: session, label, primary: false }],
        },
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? {
                ...t,
                querySession: session,
                querySessionClosed: false,
                results: [],
                planSource: undefined,
                activeResultId: null,
                dataSessionId: null,
                runId: null,
                error: null,
                status: "idle",
                metrics: {},
                elapsedMs: 0,
                runMetrics: undefined,
                clientStartedAt: undefined,
                clientElapsedMs: undefined,
                revision: current.revision + 1,
              }
            : t,
        ),
      };
    });
    return true;
  },
  attachQuerySession(tabId, sessionId) {
    const state = get();
    const current = state.tabs.find((t) => t.id === tabId);
    if (
      !current ||
      current.kind !== "query" ||
      !current.connectionId ||
      ["queued", "running"].includes(current.status)
    )
      return false;
    let targetInfo: SessionInfo | null = null;
    if (sessionId !== null) {
      const match = (state.sessionPool[current.connectionId] ?? []).find(
        (entry) => entry.info.id === sessionId,
      );
      if (!match) return false;
      if (current.querySession?.id === sessionId && !current.querySessionClosed) return true;
      targetInfo = match.info;
    } else if (!current.querySession && !current.querySessionClosed) {
      return true;
    }
    for (const result of current.results) {
      resultCache.release(result.id);
      dirtyStore.clearTab(result.id, [result.id]);
      void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    get().updateTab(tabId, {
      querySession: targetInfo,
      querySessionClosed: false,
      results: [],
      planSource: undefined,
      activeResultId: null,
      dataSessionId: null,
      runId: null,
      error: null,
      status: "idle",
      metrics: {},
      elapsedMs: 0,
      runMetrics: undefined,
      clientStartedAt: undefined,
      clientElapsedMs: undefined,
      revision: current.revision + 1,
    });
    return true;
  },
  renameQuerySession(sessionId, label) {
    const normalized = normalizeSessionLabel(label);
    if (!normalized) return;
    set((state) => {
      const nextPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        nextPool[connId] = entries.map((entry) =>
          entry.info.id === sessionId ? { ...entry, label: normalized } : entry,
        );
      }
      return { sessionPool: nextPool };
    });
  },
  async disconnectQuerySession(sessionId) {
    try {
      await client.call("disconnect", { sessionId });
    } finally {
      get().retireSession(sessionId);
    }
  },
  updateSessionLatency(sessionId, latencyMs) {
    set((state) => {
      const sessions = { ...state.sessions };
      for (const [connId, info] of Object.entries(sessions)) {
        if (info.id === sessionId) sessions[connId] = { ...info, latencyMs };
      }
      const sessionPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        sessionPool[connId] = entries.map((entry) =>
          entry.info.id === sessionId ? { ...entry, info: { ...entry.info, latencyMs } } : entry,
        );
      }
      const tabs = state.tabs.map((t) =>
        t.querySession?.id === sessionId
          ? { ...t, querySession: { ...t.querySession, latencyMs } }
          : t,
      );
      return { sessions, sessionPool, tabs };
    });
  },
  async refreshSessionContexts(sessionId) {
    const options = await client
      .call<SessionContextOptions | null>("session_contexts", {
        sessionId,
      })
      .catch(() => null);
    if (!options?.state) return;
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: options.state },
        sessionContexts: { ...state.sessionContexts, [sessionId]: options.available },
      };
    });
  },
  async switchSessionContext(sessionId, target) {
    const options = await client.call<SessionContextOptions>("switch_session_context", {
      sessionId,
      target,
    });
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: options.state },
        sessionContexts: { ...state.sessionContexts, [sessionId]: options.available },
        sessionContextRevision: {
          ...state.sessionContextRevision,
          [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
        },
      };
    });
  },
  async setTransactionMode(sessionId, mode) {
    const runtime = await client.call<SessionRuntimeState>("set_transaction_mode", {
      sessionId,
      mode,
    });
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      const prev = state.sessionStates[sessionId];
      const contextChanged =
        prev &&
        (prev.activeContext !== runtime.activeContext ||
          prev.database !== runtime.database ||
          prev.role !== runtime.role ||
          JSON.stringify(prev.searchPath) !== JSON.stringify(runtime.searchPath));
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        ...(contextChanged
          ? {
              sessionContextRevision: {
                ...state.sessionContextRevision,
                [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
              },
            }
          : {}),
      };
    });
  },
  async commitTransaction(sessionId) {
    const runtime = await client.call<SessionRuntimeState>("commit_transaction", { sessionId });
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        sessionContextRevision: {
          ...state.sessionContextRevision,
          [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
        },
      };
    });
    get().notify("Transaction committed");
  },
  async rollbackTransaction(sessionId) {
    const runtime = await client.call<SessionRuntimeState>("rollback_transaction", { sessionId });
    set((state) => {
      if (!isSessionActive(state, sessionId)) return {};
      return {
        sessionStates: { ...state.sessionStates, [sessionId]: runtime },
        sessionContextRevision: {
          ...state.sessionContextRevision,
          [sessionId]: (state.sessionContextRevision[sessionId] ?? 0) + 1,
        },
      };
    });
    get().notify("Transaction rolled back");
  },
  retireSession(sessionId) {
    planSnapshots.removeSession(sessionId);
    set((state) => {
      const retiredConnections = Object.entries(state.sessions)
        .filter(([, s]) => s.id === sessionId)
        .map(([connId]) => connId);
      const objects = { ...state.objects };
      for (const connId of retiredConnections) delete objects[connId];
      const sessionPool: Record<string, NamedSession[]> = {};
      for (const [connId, entries] of Object.entries(state.sessionPool)) {
        const remaining = entries.filter((entry) => entry.info.id !== sessionId);
        if (remaining.length > 0) sessionPool[connId] = remaining;
      }
      return {
        sessions: Object.fromEntries(
          Object.entries(state.sessions).filter(([, s]) => s.id !== sessionId),
        ),
        sessionPool,
        ...purgeSessionRuntimeMaps(state, [sessionId]),
        objects,
        tabs: state.tabs.map((t) => ({
          ...t,
          ...(t.querySession?.id === sessionId ? { querySessionClosed: true } : {}),
          ...(t.planSource?.sessionId === sessionId ? { planSource: undefined } : {}),
        })),
      };
    });
  },
  updateProfile(profile) {
    set((state) => ({
      bootstrap: state.bootstrap
        ? {
            ...state.bootstrap,
            profiles: [...state.bootstrap.profiles.filter((p) => p.id !== profile.id), profile],
          }
        : null,
    }));
  },
  selectConnection(id) {
    set({ selectedConnectionId: id });
  },
  async refreshCatalog(id) {
    const session = get().sessions[id];
    if (!session) return;
    const catalog = await client.call<TableInfo[]>("catalog", { sessionId: session.id });
    const objects = session.capabilities.objectExplorer
      ? await client
          .call<DatabaseObjectSummary[]>("database_objects", { sessionId: session.id })
          .catch((error) => {
            if ((error as Partial<AppError> | null)?.code === "unsupported") return [];
            throw error;
          })
      : [];
    if (get().sessions[id]?.id === session.id)
      set((state) => ({
        catalog: { ...state.catalog, [id]: catalog },
        objects: { ...state.objects, [id]: objects },
      }));
  },
  openQuery(connectionId = get().selectedConnectionId, sql) {
    const count = get().tabs.filter((t) => t.kind === "query").length + 1;
    const next = tab("query", `Query ${count}`, connectionId);
    const engine = get().bootstrap?.profiles.find((profile) => profile.id === connectionId)?.engine;
    next.sql =
      sql ??
      (engine === "redis"
        ? "PING"
        : engine === "snowflake"
          ? "-- A little space to think.\nSELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();"
          : engine === "mysql"
            ? "-- A little space to think.\nSELECT DATABASE(), CURRENT_USER(), VERSION();"
            : "-- A little space to think.\nSELECT current_database(), current_user, version();");
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    return next.id;
  },
  recordRecentLocation(entry) {
    const nextEntry: RecentLocation = { ...entry, visitedAtMs: Date.now() };
    set((state) => ({
      recentLocations: [
        nextEntry,
        ...state.recentLocations.filter((item) => item.key !== nextEntry.key),
      ].slice(0, MAX_RECENT_LOCATIONS),
    }));
  },
  async openSqlFile(connectionId = get().selectedConnectionId) {
    const opened = await client.call<OpenedSqlFile | null>("open_sql_file");
    if (!opened) return null;
    const next = tab("query", opened.fileName, connectionId);
    next.sql = opened.content;
    next.fileToken = opened.token;
    next.fileName = opened.fileName;
    next.savedSql = opened.content;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `tab:${next.id}`,
      label: opened.fileName,
      subtitle: "SQL file",
      target: { kind: "tab", tabId: next.id },
    });
    get().notify(`Opened ${opened.fileName}`);
    return next.id;
  },
  async saveSqlFile(tabId, options) {
    const current = get().tabs.find((t) => t.id === tabId);
    if (!current || current.kind !== "query") return false;
    const previousToken = current.fileToken;
    const contentToSave = current.sql;
    const rawSuggested = current.fileName ?? `${current.title.trim() || "query"}.sql`;
    const suggestedName = /\.sql$/i.test(rawSuggested) ? rawSuggested : `${rawSuggested}.sql`;
    let saved: SavedSqlFile | null;
    try {
      saved = await client.call<SavedSqlFile | null>("save_sql_file", {
        token: options?.saveAs ? null : previousToken,
        suggestedName,
        content: contentToSave,
      });
    } catch (error) {
      if (
        !options?.saveAs &&
        previousToken &&
        (error as Partial<AppError> | null)?.code === "approvalExpired"
      ) {
        saved = await client.call<SavedSqlFile | null>("save_sql_file", {
          token: null,
          suggestedName,
          content: contentToSave,
        });
      } else {
        throw error;
      }
    }
    if (!saved) return false;
    if (previousToken && previousToken !== saved.token) {
      void client.call("release_sql_file_grant", { token: previousToken }).catch(() => {});
    }
    get().updateTab(tabId, {
      fileToken: saved.token,
      fileName: saved.fileName,
      title: saved.fileName,
      savedSql: contentToSave,
    });
    get().recordRecentLocation({
      key: `tab:${tabId}`,
      label: saved.fileName,
      subtitle: "SQL file",
      target: { kind: "tab", tabId },
    });
    get().notify(`Saved ${saved.fileName}`);
    return true;
  },
  async saveQueryToLibrary(input, attachTabId) {
    const saved = await client.call<SavedQuery>("save_saved_query", { input });
    set((state) => {
      const savedQueries = [
        saved,
        ...state.savedQueries.filter((entry) => entry.id !== saved.id),
      ].sort((a, b) => b.updatedAtMs - a.updatedAtMs);
      const tabs = attachTabId
        ? state.tabs.map((t) =>
            t.id === attachTabId
              ? {
                  ...t,
                  savedQueryId: saved.id,
                  ...(t.fileName ? {} : { title: saved.title }),
                }
              : t,
          )
        : state.tabs;
      return {
        savedQueries,
        tabs,
        bootstrap: state.bootstrap ? { ...state.bootstrap, savedQueries } : null,
      };
    });
    const connName = saved.connectionId
      ? (get().bootstrap?.profiles.find((p) => p.id === saved.connectionId)?.name ?? null)
      : null;
    get().recordRecentLocation({
      key: `saved:${saved.id}`,
      label: saved.title,
      subtitle: `${connName ?? "Any connection"} · Saved query`,
      target: { kind: "savedQuery", savedQueryId: saved.id },
    });
    get().notify(`Saved "${saved.title}" to library`);
    return saved;
  },
  async deleteSavedQuery(id) {
    await client.call("delete_saved_query", { id });
    set((state) => {
      const savedQueries = state.savedQueries.filter((entry) => entry.id !== id);
      const bookmarks = state.bookmarks.filter(
        (entry) => !(entry.target.kind === "savedQuery" && entry.target.savedQueryId === id),
      );
      return {
        savedQueries,
        bookmarks,
        recentLocations: state.recentLocations.filter((entry) => entry.key !== `saved:${id}`),
        tabs: state.tabs.map((t) => (t.savedQueryId === id ? { ...t, savedQueryId: null } : t)),
        bootstrap: state.bootstrap ? { ...state.bootstrap, savedQueries, bookmarks } : null,
      };
    });
    get().notify("Deleted saved query");
  },
  openSavedQuery(id) {
    const state = get();
    const entry = state.savedQueries.find((item) => item.id === id);
    if (!entry) {
      state.notify("This saved query is no longer in the library.");
      return null;
    }
    const connName = entry.connectionId
      ? (state.bootstrap?.profiles.find((p) => p.id === entry.connectionId)?.name ?? null)
      : null;
    const existing = state.tabs.find(
      (t) => t.kind === "query" && t.savedQueryId === entry.id && t.fileName === null,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      get().recordRecentLocation({
        key: `saved:${entry.id}`,
        label: entry.title,
        subtitle: `${connName ?? "Any connection"} · Saved query`,
        target: { kind: "savedQuery", savedQueryId: entry.id },
      });
      return existing.id;
    }
    const targetConnectionId =
      entry.connectionId && state.bootstrap?.profiles.some((p) => p.id === entry.connectionId)
        ? entry.connectionId
        : state.selectedConnectionId;
    const next = tab("query", entry.title, targetConnectionId);
    next.sql = entry.sql;
    next.savedQueryId = entry.id;
    set((prev) => ({ tabs: [...prev.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `saved:${entry.id}`,
      label: entry.title,
      subtitle: `${connName ?? "Any connection"} · Saved query`,
      target: { kind: "savedQuery", savedQueryId: entry.id },
    });
    return next.id;
  },
  async toggleBookmark(target, label) {
    const existing = findBookmarkForTarget(get().bookmarks, target);
    if (existing) {
      await get().deleteBookmark(existing.id);
      get().notify(`Removed bookmark for ${existing.label}`);
      return false;
    }
    const created = await client.call<SavedBookmark>("save_bookmark", { label, target });
    set((state) => {
      const bookmarks = [
        created,
        ...state.bookmarks.filter((entry) => entry.id !== created.id),
      ].sort((a, b) => b.createdAtMs - a.createdAtMs);
      return {
        bookmarks,
        bootstrap: state.bootstrap ? { ...state.bootstrap, bookmarks } : null,
      };
    });
    get().notify(`Bookmarked ${created.label}`);
    return true;
  },
  async deleteBookmark(id) {
    await client.call("delete_bookmark", { id });
    set((state) => {
      const bookmarks = state.bookmarks.filter((entry) => entry.id !== id);
      return {
        bookmarks,
        bootstrap: state.bootstrap ? { ...state.bootstrap, bookmarks } : null,
      };
    });
  },
  async saveSqlFormatSettings(settings) {
    const sequence = ++sqlFormatSaveSequence;
    if (sqlFormatPending === 0) sqlFormatPersisted = get().sqlFormatSettings;
    sqlFormatPending++;
    set((state) => ({
      sqlFormatSettings: settings,
      bootstrap: state.bootstrap ? { ...state.bootstrap, sqlFormatSettings: settings } : null,
    }));
    const operation = sqlFormatSaveTail.then(async () => {
      try {
        const saved = await client.call<SqlFormatSettings>("save_sql_format_settings", {
          settings,
        });
        sqlFormatPersisted = saved;
        if (sequence === sqlFormatSaveSequence) {
          set((state) => ({
            sqlFormatSettings: saved,
            bootstrap: state.bootstrap ? { ...state.bootstrap, sqlFormatSettings: saved } : null,
          }));
        }
        return saved;
      } catch (error) {
        if (sequence === sqlFormatSaveSequence) {
          set((state) => ({
            sqlFormatSettings: sqlFormatPersisted,
            bootstrap: state.bootstrap
              ? { ...state.bootstrap, sqlFormatSettings: sqlFormatPersisted }
              : null,
          }));
        }
        throw error;
      } finally {
        sqlFormatPending--;
      }
    });
    sqlFormatSaveTail = operation.then(
      () => {},
      () => {},
    );
    return operation;
  },
  async saveSqlSnippet(input) {
    const saved = await client.call<SqlSnippet>("save_sql_snippet", { input });
    set((state) => {
      const sqlSnippets = [
        saved,
        ...state.sqlSnippets.filter((entry) => entry.id !== saved.id),
      ].sort((a, b) => a.trigger.toLowerCase().localeCompare(b.trigger.toLowerCase()));
      return {
        sqlSnippets,
        bootstrap: state.bootstrap ? { ...state.bootstrap, sqlSnippets } : null,
      };
    });
    return saved;
  },
  async deleteSqlSnippet(id) {
    await client.call("delete_sql_snippet", { id });
    set((state) => {
      const sqlSnippets = state.sqlSnippets.filter((entry) => entry.id !== id);
      return {
        sqlSnippets,
        bootstrap: state.bootstrap ? { ...state.bootstrap, sqlSnippets } : null,
      };
    });
  },
  async openLocation(target) {
    const state = get();
    if (target.kind === "savedQuery") {
      return state.openSavedQuery(target.savedQueryId) ? { kind: "opened" } : { kind: "missing" };
    }
    if (target.kind === "tab") {
      const found = state.tabs.find((t) => t.id === target.tabId);
      if (!found) {
        state.notify("That tab has already been closed.");
        return { kind: "missing" };
      }
      set({ activeTabId: found.id });
      return { kind: "opened" };
    }
    const profile = state.bootstrap?.profiles.find((p) => p.id === target.connectionId);
    if (!profile) {
      state.notify("The connection profile for this item was deleted.");
      return { kind: "missing" };
    }
    if (!state.sessions[profile.id]) {
      state.selectConnection(profile.id);
      return { kind: "needsConnect", profile };
    }
    if (target.kind === "table") {
      await state.openTable(profile.id, target.table);
      return { kind: "opened" };
    }
    await state.openObject(profile.id, target.object);
    return { kind: "opened" };
  },
  openUtility(kind) {
    const existing = get().tabs.find(
      (t) =>
        t.kind === kind && (kind !== "processes" || t.connectionId === get().selectedConnectionId),
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = tab(
      kind,
      { history: "Query history", processes: "Processes", benchmark: "Grid benchmark" }[kind],
      get().selectedConnectionId,
    );
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  openDiagram(connectionId, schema) {
    const existing = get().tabs.find(
      (t) => t.kind === "diagram" && t.connectionId === connectionId && t.diagramSchema === schema,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = { ...tab("diagram", `${schema} diagram`, connectionId), diagramSchema: schema };
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  openSchemaDiff(connectionId, schema) {
    const existing = get().tabs.find(
      (t) =>
        t.kind === "schemaDiff" && t.connectionId === connectionId && t.diagramSchema === schema,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const next = {
      ...tab("schemaDiff", `${schema} schema diff`, connectionId),
      diagramSchema: schema,
    };
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
  },
  async openTable(connectionId, table, rawQuery = EMPTY_QUERY, preferredView, highlightedColumn) {
    const query = normalizeQuery(rawQuery);
    const profileName =
      get().bootstrap?.profiles.find((p) => p.id === connectionId)?.name ?? connectionId;
    const existing = get().tabs.find(
      (t) =>
        t.kind === "table" &&
        t.connectionId === connectionId &&
        t.table?.schema === table.schema &&
        t.table.name === table.name &&
        sameQuery(t.query, query),
    );
    if (existing) {
      set((state) => ({
        activeTabId: existing.id,
        tabs:
          preferredView || highlightedColumn !== undefined
            ? state.tabs.map((t) =>
                t.id === existing.id
                  ? {
                      ...t,
                      ...(preferredView ? { view: preferredView } : {}),
                      ...(highlightedColumn !== undefined ? { highlightedColumn } : {}),
                    }
                  : t,
              )
            : state.tabs,
      }));
      get().recordRecentLocation({
        key: `table:${connectionId}:${table.schema}.${table.name}`,
        label: `${table.schema}.${table.name}`,
        subtitle: `${profileName} · Table`,
        target: { kind: "table", connectionId, table },
      });
      return;
    }
    const next = tab("table", tableTabTitle(table.name, query), connectionId);
    next.table = table;
    next.query = query;
    if (preferredView) next.view = preferredView;
    if (highlightedColumn !== undefined) next.highlightedColumn = highlightedColumn;
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `table:${connectionId}:${table.schema}.${table.name}`,
      label: `${table.schema}.${table.name}`,
      subtitle: `${profileName} · Table`,
      target: { kind: "table", connectionId, table },
    });
    await get().refreshTable(next.id);
  },
  openLinkedSessionObject(inspectorId, object) {
    const state = get();
    const inspector = state.tabs.find((entry) => entry.id === inspectorId);
    if (!inspector?.metadataOrigin?.catalogRevision || !metadataSessionForTab(inspector, state))
      return;
    get().openSessionObject(
      inspector.metadataOrigin.tabId,
      object,
      undefined,
      inspector.metadataOrigin.catalogRevision,
    );
  },
  appendSessionDdl(inspectorId, ddl) {
    const state = get();
    const inspector = state.tabs.find((entry) => entry.id === inspectorId);
    if (!inspector?.metadataOrigin?.catalogRevision || !metadataSessionForTab(inspector, state))
      return false;
    const source = state.tabs.find((entry) => entry.id === inspector.metadataOrigin!.tabId)!;
    if (!ddl.trim() || ["queued", "running"].includes(source.status)) return false;
    // Preserve the editor text, file dirty tracking, results and session ownership.
    set({
      tabs: state.tabs.map((entry) =>
        entry.id === source.id
          ? { ...entry, sql: entry.sql + (entry.sql ? "\n\n" : "") + ddl }
          : entry,
      ),
      activeTabId: source.id,
    });
    return true;
  },
  openSessionObject(tabId, object, highlightedColumn, catalogRevision) {
    const state = get();
    const source = state.tabs.find((entry) => entry.id === tabId && entry.kind === "query");
    const session = source && querySessionForTab(source, state.sessions);
    if (!source?.connectionId || !session || session.connectionId !== source.connectionId) return;
    const origin: MetadataOrigin = {
      tabId,
      sessionId: session.id,
      contextRevision: state.sessionContextRevision[session.id] ?? 0,
      catalogRevision,
    };
    const existing = state.tabs.find(
      (entry) =>
        entry.kind === "object" &&
        entry.connectionId === source.connectionId &&
        entry.object &&
        sameDatabaseObject(entry.object, object) &&
        entry.metadataOrigin?.tabId === tabId &&
        entry.metadataOrigin.sessionId === session.id &&
        entry.metadataOrigin.contextRevision === origin.contextRevision &&
        entry.metadataOrigin.catalogRevision === catalogRevision,
    );
    if (existing) {
      set({
        activeTabId: existing.id,
        tabs: state.tabs.map((entry) =>
          entry.id === existing.id
            ? {
                ...entry,
                view: "structure",
                highlightedColumn: highlightedColumn ?? null,
                revision: entry.revision + 1,
              }
            : entry,
        ),
      });
      return;
    }
    const label =
      state.sessionPool[source.connectionId]?.find((entry) => entry.info.id === session.id)
        ?.label ?? "Shared session";
    const next = tab("object", `${objectTabTitle(object)} · ${label}`, source.connectionId);
    next.object = object;
    next.view = "structure";
    next.highlightedColumn = highlightedColumn ?? null;
    next.metadataOrigin = origin;
    set({ tabs: [...state.tabs, next], activeTabId: next.id });
  },
  async openObject(connectionId, object, preferredView, highlightedColumn) {
    if (isTabularObjectKind(object.kind)) {
      await get().openTable(
        connectionId,
        { schema: object.schema, name: object.name },
        EMPTY_QUERY,
        preferredView,
        highlightedColumn,
      );
      return;
    }
    const profileName =
      get().bootstrap?.profiles.find((p) => p.id === connectionId)?.name ?? connectionId;
    const existing = get().tabs.find(
      (t) =>
        t.kind === "object" &&
        t.connectionId === connectionId &&
        sameDatabaseObject(t.object, object),
    );
    if (existing) {
      set((state) => ({
        activeTabId: existing.id,
        tabs: preferredView
          ? state.tabs.map((t) => (t.id === existing.id ? { ...t, view: preferredView } : t))
          : state.tabs,
      }));
      get().recordRecentLocation({
        key: `object:${connectionId}:${databaseObjectKey(object)}`,
        label: `${object.schema}.${objectTabTitle(object)}`,
        subtitle: `${profileName} · ${object.kind}`,
        target: { kind: "object", connectionId, object },
      });
      return;
    }
    const next = tab("object", objectTabTitle(object), connectionId);
    next.object = object;
    next.view = preferredView ?? "structure";
    set((state) => ({ tabs: [...state.tabs, next], activeTabId: next.id }));
    get().recordRecentLocation({
      key: `object:${connectionId}:${databaseObjectKey(object)}`,
      label: `${object.schema}.${objectTabTitle(object)}`,
      subtitle: `${profileName} · ${object.kind}`,
      target: { kind: "object", connectionId, object },
    });
  },
  async setTableQuery(id, rawQuery, options) {
    const current = get().tabs.find((t) => t.id === id);
    if (!current?.table) return { kind: "refused", reason: "This tab does not browse a table" };
    if (
      dirtyStore.pendingCountForTab(
        current.id,
        current.pages.map((page) => page.result.id),
      ) > 0
    )
      return {
        kind: "refused",
        reason: "Review or discard pending edits before changing sort or filters",
      };
    const session = current.connectionId ? get().sessions[current.connectionId] : null;
    if (!session) return { kind: "refused", reason: "Connect to change sort or filters" };
    const query = normalizeQuery(rawQuery);
    if (!options?.force && !options?.approval && sameQuery(current.query, query))
      return { kind: "applied" };
    if (hasRawClauses(query) && !options?.approval) {
      try {
        const plan = await client.call<PreparedPlan | null>("prepare_browse_table", {
          sessionId: session.id,
          table: current.table,
          query,
        });
        if (plan) return { kind: "review", plan, query };
      } catch (error) {
        get().updateTab(id, { error: asAppError(error) });
        return { kind: "applied" };
      }
    }
    get().updateTab(id, { query, error: null });
    await get().refreshTable(id, options?.approval);
    return { kind: "applied" };
  },
  async refreshTable(id, approval) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (!current?.table || !session) return;
    dirtyStore.clearInserts(current.id);
    for (const page of current.pages) {
      resultCache.release(page.result.id);
      dirtyStore.clear(page.result.id);
      void client.call("release_result", { resultId: page.result.id });
    }
    get().updateTab(id, {
      pages: [],
      pageKeys: {},
      loadedRows: 0,
      nextKey: null,
      hasMore: true,
      loadingPage: true,
      schema: null,
      error: null,
      status: "idle",
      runId: null,
      revision: current.revision + 1,
      dataSessionId: session.id,
    });
    try {
      const schema = await client.call<TableSchema>("table_schema", {
        sessionId: session.id,
        table: current.table,
      });
      if (!requestIsCurrent(id, current.revision + 1, current.connectionId!, session.id)) return;
      get().updateTab(id, { schema, loadingPage: false });
      await get().loadNextPage(id, approval);
    } catch (error) {
      if (requestIsCurrent(id, current.revision + 1, current.connectionId!, session.id))
        get().updateTab(id, {
          error: asAppError(error),
          loadingPage: false,
          hasMore: false,
        });
    }
  },
  async loadNextPage(id, approval) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (
      !current?.table ||
      !session ||
      current.dataSessionId !== session.id ||
      current.loadingPage ||
      !current.hasMore ||
      current.error
    )
      return;
    get().updateTab(id, { loadingPage: true });
    const start = current.loadedRows,
      after = current.nextKey;
    try {
      const page = await client.call<import("../ipc/generated").TablePage>("browse_table", {
        sessionId: session.id,
        table: current.table,
        after,
        query: isEmptyQuery(current.query) ? null : current.query,
        approval: approval ?? null,
      });
      if (
        !get().tabs.some((t) => t.id === id && t.revision === current.revision) ||
        get().sessions[current.connectionId!]?.id !== session.id
      ) {
        void client.call("release_result", { resultId: page.result.id });
        return;
      }
      const latest = get().tabs.find((t) => t.id === id)!;
      verifyPageColumns(latest.schema, page.result);
      const pages = [...latest.pages, { start, after, result: page.result }];
      if (pages.length > 8) {
        const evict = pages.findIndex(
          (p) => !dirtyStore.hasResult(p.result.id) && p.start < start - 1000,
        );
        if (evict >= 0) {
          const [old] = pages.splice(evict, 1);
          resultCache.release(old.result.id);
          void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, {
        pages,
        pageKeys: { ...latest.pageKeys, [start]: after },
        loadedRows: start + page.result.retainedRows,
        nextKey: page.nextKey,
        hasMore: page.hasMore,
      });
    } catch (error) {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, {
          error: asAppError(error),
          hasMore: false,
        });
    } finally {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { loadingPage: false });
    }
  },
  async ensureTablePage(id, row) {
    const current = get().tabs.find((t) => t.id === id),
      session = current?.connectionId ? get().sessions[current.connectionId] : null;
    if (
      !current?.table ||
      !session ||
      current.dataSessionId !== session.id ||
      current.error ||
      current.loadingPage ||
      current.pages.some((p) => row >= p.start && row < p.start + p.result.retainedRows)
    )
      return;
    const start = Object.keys(current.pageKeys)
      .map(Number)
      .filter((key) => key <= row)
      .reduce((a, b) => Math.max(a, b), -1);
    if (start < 0) return;
    get().updateTab(id, { loadingPage: true });
    try {
      const page = await client.call<import("../ipc/generated").TablePage>("browse_table", {
        sessionId: session.id,
        table: current.table,
        after: current.pageKeys[start],
        query: isEmptyQuery(current.query) ? null : current.query,
      });
      const latest = get().tabs.find((t) => t.id === id);
      if (!latest || latest.revision !== current.revision) {
        void client.call("release_result", { resultId: page.result.id });
        return;
      }
      verifyPageColumns(latest.schema, page.result);
      const pages = [
        ...latest.pages,
        { start, after: current.pageKeys[start], result: page.result },
      ];
      if (pages.length > 8) {
        const evict = pages.findIndex(
          (p) => Math.abs(p.start - start) > 1500 && !dirtyStore.hasResult(p.result.id),
        );
        if (evict >= 0) {
          const [old] = pages.splice(evict, 1);
          resultCache.release(old.result.id);
          void client.call("release_result", { resultId: old.result.id });
        }
      }
      get().updateTab(id, { pages });
    } catch (error) {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { error: { code: "database", message: messageOf(error) } });
    } finally {
      if (requestIsCurrent(id, current.revision, current.connectionId!, session.id))
        get().updateTab(id, { loadingPage: false });
    }
  },
  updateTab(id, update) {
    set((state) => ({
      tabs: state.tabs.map((t) => {
        if (t.id !== id) return t;
        const patch = typeof update === "function" ? update(t) : update;
        const changedSql = patch.sql !== undefined && patch.sql !== t.sql;
        return {
          ...t,
          ...patch,
          ...(changedSql || (patch.error === null && !patch.errorOrigin)
            ? { errorOrigin: undefined }
            : {}),
        };
      }),
    }));
  },
  togglePinTab(id) {
    set((state) => {
      const target = state.tabs.find((t) => t.id === id);
      if (!target) return {};
      const nextPinned = !target.pinned;
      const updated = state.tabs.map((t) => (t.id === id ? { ...t, pinned: nextPinned } : t));
      return { tabs: orderPinnedFirst(updated) };
    });
  },
  closeTab(id) {
    planSnapshots.removeOwner(id);
    const stateSnapshot = get();
    const current = stateSnapshot.tabs.find((t) => t.id === id);
    if (!current) return;
    const closedSnapshot = snapshotClosedTab(current);
    if (current.fileToken) {
      void client.call("release_sql_file_grant", { token: current.fileToken }).catch(() => {});
    }
    const disconnectSecondary =
      current.querySession &&
      !current.querySessionClosed &&
      isLastOwnerOfSecondarySession(stateSnapshot.tabs, current)
        ? current.querySession.id
        : null;
    if (disconnectSecondary)
      void client
        .call("disconnect", { sessionId: disconnectSecondary })
        .catch((error) => get().notify(messageOf(error)));
    else if (current.runId && ["queued", "running"].includes(current.status))
      void client.call("cancel_run", { runId: current.runId }).catch(() => {});
    dirtyStore.clearInserts(current.id);
    for (const result of [...current.results, ...current.pages.map((p) => p.result)]) {
      dirtyStore.clearTab(result.id, [result.id]);
      resultCache.release(result.id);
      void client.call("release_result", { resultId: result.id }).catch(() => {});
    }
    set((state) => {
      const index = state.tabs.findIndex((t) => t.id === id),
        tabs = state.tabs.filter((t) => t.id !== id);
      let sessionPool = state.sessionPool;
      if (disconnectSecondary) {
        sessionPool = {};
        for (const [connId, entries] of Object.entries(state.sessionPool)) {
          const remaining = entries.filter((entry) => entry.info.id !== disconnectSecondary);
          if (remaining.length > 0) sessionPool[connId] = remaining;
        }
      }
      const closedTabs = closedSnapshot
        ? [
            closedSnapshot,
            ...state.closedTabs.filter((entry) => entry.id !== closedSnapshot.id),
          ].slice(0, MAX_CLOSED_TABS)
        : state.closedTabs;
      return {
        tabs,
        closedTabs,
        sessionPool,
        recentLocations: state.recentLocations.filter((entry) => entry.key !== `tab:${id}`),
        ...(disconnectSecondary ? purgeSessionRuntimeMaps(state, [disconnectSecondary]) : {}),
        activeTabId:
          state.activeTabId === id ? (tabs[Math.max(0, index - 1)]?.id ?? null) : state.activeTabId,
      };
    });
  },
  closeOtherTabs(keepTabId) {
    const stateSnapshot = get();
    const keepExists = stateSnapshot.tabs.some((t) => t.id === keepTabId);
    if (!keepExists) return 0;
    let skippedProtected = 0;
    const toCloseIds: string[] = [];
    for (const candidate of stateSnapshot.tabs) {
      if (candidate.id === keepTabId || candidate.pinned) continue;
      if (isTabProtectedFromBulkClose(stateSnapshot.tabs, candidate)) {
        skippedProtected += 1;
        continue;
      }
      toCloseIds.push(candidate.id);
    }
    for (const id of toCloseIds) {
      get().closeTab(id);
    }
    set({ activeTabId: keepTabId });
    if (skippedProtected > 0) {
      get().notify(
        `Kept ${skippedProtected} tab${skippedProtected === 1 ? "" : "s"} with unsaved changes or active sessions open.`,
      );
    }
    return toCloseIds.length;
  },
  closeCleanTabs() {
    const stateSnapshot = get();
    let skippedProtected = 0;
    const toCloseIds: string[] = [];
    for (const candidate of stateSnapshot.tabs) {
      if (candidate.pinned) continue;
      if (isTabProtectedFromBulkClose(stateSnapshot.tabs, candidate)) {
        skippedProtected += 1;
        continue;
      }
      toCloseIds.push(candidate.id);
    }
    for (const id of toCloseIds) {
      get().closeTab(id);
    }
    if (skippedProtected > 0) {
      get().notify(
        `Kept ${skippedProtected} modified or active tab${skippedProtected === 1 ? "" : "s"} open.`,
      );
    }
    return toCloseIds.length;
  },
  async reopenClosedTab() {
    const stateSnapshot = get();
    const [entry, ...remaining] = stateSnapshot.closedTabs;
    if (!entry) {
      get().notify("No recently closed tabs to reopen.");
      return null;
    }
    set({ closedTabs: remaining });

    if (entry.kind === "table" && entry.connectionId && entry.table) {
      if (get().sessions[entry.connectionId]) {
        await get().openTable(entry.connectionId, entry.table, entry.query, entry.view);
        const activeId = get().activeTabId;
        if (activeId && entry.pinned) {
          get().togglePinTab(activeId);
        }
        return activeId;
      }
      const reopened: StudioTab = {
        ...tab("table", entry.title, entry.connectionId),
        id: entry.id,
        table: entry.table,
        object: {
          schema: entry.table.schema,
          name: entry.table.name,
          kind: "table",
          identity: null,
        },
        query: entry.query,
        view: entry.view,
        pinned: entry.pinned,
      };
      set((state) => ({
        tabs: orderPinnedFirst([...state.tabs, reopened]),
        activeTabId: reopened.id,
      }));
      return reopened.id;
    }

    if (entry.kind === "object" && entry.connectionId && entry.object) {
      if (get().sessions[entry.connectionId]) {
        await get().openObject(entry.connectionId, entry.object, entry.view);
        const activeId = get().activeTabId;
        if (activeId && entry.pinned) {
          get().togglePinTab(activeId);
        }
        return activeId;
      }
      const reopened: StudioTab = {
        ...tab("object", entry.title, entry.connectionId),
        id: entry.id,
        object: entry.object,
        view: entry.view,
        pinned: entry.pinned,
      };
      set((state) => ({
        tabs: orderPinnedFirst([...state.tabs, reopened]),
        activeTabId: reopened.id,
      }));
      return reopened.id;
    }

    const reopenedId = get().tabs.some((t) => t.id === entry.id) ? crypto.randomUUID() : entry.id;
    const reopened: StudioTab = {
      ...tab(entry.kind, entry.title, entry.connectionId),
      id: reopenedId,
      sql: entry.sql,
      savedQueryId: entry.savedQueryId,
      pinned: entry.pinned,
      diagramSchema: entry.diagramSchema,
    };
    set((state) => ({
      tabs: orderPinnedFirst([...state.tabs, reopened]),
      activeTabId: reopened.id,
    }));
    return reopened.id;
  },
  removeConnection(id) {
    const state = get();
    const deletedProfile = state.bootstrap?.profiles.find((p) => p.id === id);
    const purgeSensitiveQueries = Boolean(
      deletedProfile &&
      (deletedProfile.environment === "production" || !deletedProfile.storeQueryText),
    );
    const sessionIds = new Set<string>();
    if (state.sessions[id]) sessionIds.add(state.sessions[id].id);
    for (const entry of state.sessionPool[id] ?? []) sessionIds.add(entry.info.id);
    for (const t of state.tabs) {
      if (t.connectionId === id && t.querySession && !t.querySessionClosed)
        sessionIds.add(t.querySession.id);
    }
    for (const sessionId of sessionIds) void client.call("disconnect", { sessionId });
    get().disconnected(id);
    for (const t of get().tabs.filter((t) => t.connectionId === id)) get().closeTab(t.id);
    set((state) => {
      const catalog = { ...state.catalog };
      delete catalog[id];
      const purgedQueryIds = new Set<string>();
      const savedQueries = state.savedQueries.flatMap((entry) => {
        if (entry.connectionId !== id) return [entry];
        if (purgeSensitiveQueries) {
          purgedQueryIds.add(entry.id);
          return [];
        }
        return [{ ...entry, connectionId: null }];
      });
      const bookmarks = state.bookmarks.filter((entry) => {
        if (entry.target.kind === "object" && entry.target.connectionId === id) return false;
        if (entry.target.kind === "savedQuery" && purgedQueryIds.has(entry.target.savedQueryId))
          return false;
        return true;
      });
      const recentLocations = state.recentLocations.filter((entry) => {
        if (
          (entry.target.kind === "table" || entry.target.kind === "object") &&
          entry.target.connectionId === id
        )
          return false;
        if (entry.target.kind === "savedQuery" && purgedQueryIds.has(entry.target.savedQueryId))
          return false;
        return true;
      });
      return {
        catalog,
        savedQueries,
        bookmarks,
        recentLocations,
        closedTabs: state.closedTabs.filter((entry) => entry.connectionId !== id),
        tabs:
          purgedQueryIds.size > 0
            ? state.tabs.map((t) =>
                t.savedQueryId && purgedQueryIds.has(t.savedQueryId)
                  ? { ...t, savedQueryId: null }
                  : t,
              )
            : state.tabs,
        bootstrap: state.bootstrap
          ? {
              ...state.bootstrap,
              profiles: state.bootstrap.profiles.filter((p) => p.id !== id),
              savedQueries,
              bookmarks,
            }
          : null,
        tableLayouts: state.tableLayouts.filter((layout) => layout.connectionId !== id),
        selectedConnectionId: state.selectedConnectionId === id ? null : state.selectedConnectionId,
      };
    });
  },
  disconnected(connectionId) {
    planSnapshots.removeConnection(connectionId);
    set((state) => {
      const retiredIds = new Set<string>();
      if (state.sessions[connectionId]) retiredIds.add(state.sessions[connectionId].id);
      for (const entry of state.sessionPool[connectionId] ?? []) retiredIds.add(entry.info.id);
      for (const t of state.tabs) {
        if (t.connectionId === connectionId && t.querySession) retiredIds.add(t.querySession.id);
      }
      const sessions = { ...state.sessions };
      delete sessions[connectionId];
      const sessionPool = { ...state.sessionPool };
      delete sessionPool[connectionId];
      const objects = { ...state.objects };
      delete objects[connectionId];
      return {
        sessions,
        sessionPool,
        ...purgeSessionRuntimeMaps(state, retiredIds),
        objects,
        tabs: state.tabs.map((t) =>
          t.connectionId === connectionId
            ? {
                ...t,
                planSource: undefined,
                ...(t.querySession ? { querySessionClosed: true } : {}),
              }
            : t,
        ),
      };
    });
  },
  resetConnectionResults(connectionId) {
    get().disconnected(connectionId);
    for (const tab of get().tabs.filter((t) => t.connectionId === connectionId)) {
      dirtyStore.clearInserts(tab.id);
      for (const result of [...tab.results, ...tab.pages.map((p) => p.result)]) {
        resultCache.release(result.id);
        dirtyStore.clearTab(result.id, [result.id]);
        void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
      get().updateTab(tab.id, {
        results: [],
        planSource: undefined,
        pages: [],
        pageKeys: {},
        loadedRows: 0,
        hasMore: false,
        schema: null,
        nextKey: null,
        loadingPage: false,
        runId: null,
        activeResultId: null,
        status: "idle",
        error: null,
        revision: tab.revision + 1,
      });
    }
  },
  applyEvents(tabId, events, revision) {
    const runtimeUpdates: SessionRuntimeState[] = [];
    for (const event of events) {
      if (event.type === "sessionClosed") {
        get().retireSession(event.sessionId);
        get().notify(event.reason);
      }
      if (event.type === "notice") get().notify(event.message);
      if (event.type === "sessionState") runtimeUpdates.push(event.state);
    }
    if (runtimeUpdates.length > 0) {
      set((state) => {
        let changed = false;
        const sessionStates = { ...state.sessionStates };
        const sessionContextRevision = { ...state.sessionContextRevision };
        for (const runtime of runtimeUpdates) {
          if (!isSessionActive(state, runtime.sessionId)) continue;
          const prev = sessionStates[runtime.sessionId];
          sessionStates[runtime.sessionId] = runtime;
          changed = true;
          if (
            prev &&
            (prev.activeContext !== runtime.activeContext ||
              prev.database !== runtime.database ||
              prev.role !== runtime.role ||
              JSON.stringify(prev.searchPath) !== JSON.stringify(runtime.searchPath))
          ) {
            sessionContextRevision[runtime.sessionId] =
              (sessionContextRevision[runtime.sessionId] ?? 0) + 1;
          }
        }
        return changed ? { sessionStates, sessionContextRevision } : {};
      });
    }
    set((state) => {
      const current = state.tabs.find((t) => t.id === tabId);
      if (!current || current.revision !== revision) {
        for (const event of events)
          if (event.type === "result")
            void client.call("release_result", { resultId: event.result.id }).catch(() => {});
        return {};
      }
      const next = { ...current, results: [...current.results] };
      for (const event of events) {
        if (event.type === "started") {
          next.runId = event.runId;
          next.status = "running";
        }
        if (event.type === "result") {
          const at = next.results.findIndex((r) => r.id === event.result.id);
          if (at >= 0) {
            if (event.result.status !== "streaming") {
              resultCache.release(event.result.id);
            }
            next.results[at] = event.result;
          } else {
            next.results.push(event.result);
          }
          if (!next.activeResultId) next.activeResultId = event.result.id;
        }
        if (event.type === "progress")
          next.results = next.results.map((r) =>
            r.id === event.resultId ? { ...r, retainedRows: event.retainedRows } : r,
          );
        if (event.type === "error") next.error = event.error;
        if (event.type === "statementCompleted") {
          next.elapsedMs += event.elapsedMs;
          next.metrics = {
            ...next.metrics,
            [event.index]: { affectedRows: event.affectedRows, elapsedMs: event.elapsedMs },
          };
        }
        if (event.type === "runMetrics" && event.runId === next.runId) next.runMetrics = event;
        if (event.type === "finished") {
          if (next.clientStartedAt !== undefined)
            next.clientElapsedMs = Math.max(0, performance.now() - next.clientStartedAt);
          next.status = event.status;
          next.elapsedMs = event.elapsedMs;
          next.runId = null;
        }
      }
      return { tabs: state.tabs.map((t) => (t.id === tabId ? next : t)) };
    });
  },
}));

export function beginEventBatch(tabId: string) {
  const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision ?? -1;
  let events: QueryEvent[] = [],
    frame = 0;
  const flush = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    const batch = events;
    events = [];
    useStudio.getState().applyEvents(tabId, batch, revision);
  };
  return (event: QueryEvent) => {
    events.push(event);
    if (event.type === "finished") flush();
    else if (!frame) frame = requestAnimationFrame(flush);
  };
}
