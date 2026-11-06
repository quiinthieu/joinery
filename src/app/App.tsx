import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { planSourceForExecution } from "../features/studio/plan-snapshots";
import {
  Activity,
  ArrowDownLeft,
  ArrowLeftRight,
  ArrowUpDown,
  ArrowUpRight,
  Bookmark,
  BookmarkPlus,
  Check,
  Clock3,
  Code2,
  Columns3,
  Command,
  Copy,
  Database,
  Download,
  FileCode2,
  Filter,
  FolderOpen,
  Gauge,
  History,
  Keyboard,
  LockKeyhole,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRight,
  Pin,
  PinOff,
  Play,
  Plus,
  RotateCcw,
  Save,
  Search,
  Star,
  Upload,
  Wifi,
  X,
  ChevronRight,
} from "lucide-react";
import { Sidebar } from "../features/connections/Sidebar";
import { ConnectionDialog, ConnectDialog } from "../features/connections/ConnectionDialog";
import { Preferences } from "../features/studio/Preferences";
import { HistoryView, ProcessesView } from "../features/studio/UtilityViews";
import { DiagramView } from "../features/diagram/DiagramView";
import { SchemaDiffView } from "../features/schema-diff/SchemaDiffView";
import { ReviewDialog } from "../features/safety/ReviewDialog";
import {
  buildMutationDiffs,
  filterTableChanges,
  type RowDiff,
} from "../features/safety/mutation-diff";
import { ExportDialog } from "../features/grid/ExportDialog";
import { ImportCsvDialog } from "../features/grid/ImportCsvDialog";
import { DataCompareDialog } from "../features/grid/DataCompareDialog";
import { TableColumnsDialog } from "../features/grid/TableColumnsDialog";
import { ValueDialog } from "../features/grid/ValueDialog";
import { ValueInspectorPanel } from "../features/inspector/ValueInspectorPanel";
import { Modal, InlineError } from "../components/ui/Modal";
import { tabNavigation } from "../components/ui/tabs";
import { appErrorOf, client, messageOf } from "../ipc/client";
import {
  editorDocument,
  resolveExecutionError,
  type SqlExecutionOrigin,
} from "../features/studio/sql-navigation";
import type {
  BrowseQuery,
  ChangePreview,
  ConnectionProfile,
  ForeignKey,
  InboundForeignKey,
  ParameterHistoryEntry,
  PreparedPlan,
  ProcessInfo,
  QueryParamInput,
  QueryParamSlot,
  ResourceUsage,
  TableChanges,
} from "../ipc/generated";
import {
  useStudio,
  beginEventBatch,
  querySessionForTab,
  metadataSessionForTab,
  isLastOwnerOfSecondarySession,
  sessionHasOpenTransaction,
  isTabFileDirty,
  isTabProtectedFromBulkClose,
  bookmarkTargetForTab,
  type StudioTab,
} from "../state/studio";
import {
  effectiveShortcutBadge,
  effectiveShortcutBinding,
  matchesKeyboardEvent,
  type ShortcutActionId,
} from "../features/studio/shortcuts";
import { flushWorkspace, watchWorkspace } from "../state/workspace";
import { dirtyStore } from "../data/dirty-store";
import { resultCache } from "../data/result-cache";
import { COPY_FORMATS } from "../features/grid/copy-format";
import type { CellTarget, DatabaseGridHandle } from "../features/grid/DatabaseGrid";
import { findTableLayout, resolveTableLayout } from "../features/grid/table-layout";
import {
  canOrder,
  constraintsForColumn,
  hasRawClauses,
  inboundConstraintsForCell,
  referencedRowsQuery,
  referencingRowsQuery,
  withFilter,
} from "../features/grid/table-query";
import type { EditorHandle } from "../features/studio/SqlEditor";
import {
  ParameterDialog,
  clearTabParamDrafts,
  pruneTabParamDrafts,
  getTabParamDrafts,
  setTabParamDrafts,
  buildQueryParamInputs,
  type QueryParamDraft,
} from "../features/studio/ParameterDialog";
import { SearchEverywhere } from "../features/studio/SearchEverywhere";
import type { SearchScope } from "../features/studio/search-everywhere";
import { LibraryModal, SaveQueryDialog, type LibraryTab } from "../features/studio/LibraryModal";
import { Workbench, Welcome } from "./Workbench";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Dialog =
  | { kind: "connection"; profile?: ConnectionProfile; groupId?: string }
  | { kind: "connect"; profile: ConnectionProfile; tabId?: string }
  | { kind: "settings" }
  | { kind: "palette"; initialScope?: SearchScope }
  | { kind: "value"; target: CellTarget }
  | {
      kind: "review";
      origin?: SqlExecutionOrigin;
      plan: PreparedPlan;
      preview?: ChangePreview;
      tabId: string;
      profile?: ConnectionProfile;
      diffRows?: RowDiff[];
      onRecomputePlan?(
        selectedIds: Set<string>,
      ): Promise<{ plan: PreparedPlan; preview: ChangePreview }>;
      onSuccess?(): void;
      onApprove?(approval: string): Promise<void>;
    }
  | { kind: "confirm"; title: string; description: string; action(): void | Promise<void> }
  | { kind: "columns"; tabId: string }
  | { kind: "export"; tabId: string }
  | { kind: "importCsv"; tabId: string }
  | { kind: "compare"; tabId?: string }
  | { kind: "library"; tab: LibraryTab }
  | { kind: "saveQuery"; tabId: string }
  | {
      kind: "parameters";
      origin: SqlExecutionOrigin;
      tabId: string;
      sessionId: string | null;
      sql: string;
      slots: QueryParamSlot[];
      profile?: ConnectionProfile;
      initialDrafts?: Record<string, QueryParamDraft>;
      allowSaveOnly?: boolean;
      explain?: "estimate" | "analyze";
    }
  | { kind: "references"; keys: ForeignKey[]; target: CellTarget; tabId: string }
  | { kind: "referencing"; keys: InboundForeignKey[]; target: CellTarget; tabId: string };
export function dirtyFor(tab: StudioTab) {
  if (tab.kind === "table") {
    return tab.pages.flatMap((page) => dirtyStore.forResult(page.result.id));
  }
  if (tab.kind === "query") {
    const activeResult = tab.results.find((r) => r.id === tab.activeResultId) ?? tab.results[0];
    return activeResult ? dirtyStore.forResult(activeResult.id) : [];
  }
  return [];
}
export function pendingCountFor(tab: StudioTab) {
  if (tab.kind === "table") {
    return dirtyStore.pendingCountForTab(
      tab.id,
      tab.pages.map((page) => page.result.id),
    );
  }
  if (tab.kind === "query") {
    return tab.results.reduce((sum, r) => sum + dirtyStore.pendingCountForTab(r.id, [r.id]), 0);
  }
  return 0;
}

export function App() {
  const state = useStudio(),
    [dialog, setDialog] = useState<Dialog | null>(null),
    [selectedCell, setSelectedCell] = useState<CellTarget | null>(null),
    [usage, setUsage] = useState<ResourceUsage | null>(null);
  const editor = useRef<EditorHandle>(null);
  const grid = useRef<DatabaseGridHandle>(null);
  const importRawRefreshTabRef = useRef<string | null>(null);
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const current = state.tabs.find((tab) => tab.id === state.activeTabId);
  const profile = state.bootstrap?.profiles.find(
    (p) => p.id === (current?.connectionId ?? state.selectedConnectionId),
  );
  const session = current?.metadataOrigin
    ? metadataSessionForTab(current, state)
    : current?.kind === "query"
      ? querySessionForTab(current, state.sessions)
      : profile
        ? state.sessions[profile.id]
        : null;
  const references = useRef({ current, dialog, profile });
  references.current = { current, dialog, profile };
  const tableColumn =
    current?.kind === "table" &&
    session?.capabilities.tableFiltering &&
    selectedCell &&
    !selectedCell.insert &&
    current.pages.some((page) => page.result.id === selectedCell.resultId) &&
    current.schema?.columns.some((column) => column.id === selectedCell.column.id)
      ? selectedCell
      : null;
  const selectedKeys = tableColumn
    ? constraintsForColumn(current!.schema, tableColumn.column.name)
    : [];
  /** Opens the row referenced by `key` in a new tab, as DataGrip does. */
  function followKey(tab: StudioTab, key: ForeignKey, target: CellTarget) {
    const query = referencedRowsQuery(key, (name) => {
      const column = tab.schema?.columns.find((c) => c.name === name);
      return column ? resultCache.read(target.resultId, column.id, target.row, true)?.value : null;
    });
    if (!query || !tab.connectionId) {
      state.notify("This row's foreign key contains NULL, so it references no row.");
      return;
    }
    void state.openTable(tab.connectionId, key.target, query);
  }
  function goToReferenced() {
    if (!current || !tableColumn) {
      state.notify("Select a foreign-key cell in a table to open the row it references.");
      return;
    }
    if (!selectedKeys.length) {
      state.notify(`${tableColumn.column.name} is not part of a foreign key.`);
      return;
    }
    if (selectedKeys.length === 1) followKey(current, selectedKeys[0], tableColumn);
    else
      setDialog({ kind: "references", keys: selectedKeys, target: tableColumn, tabId: current.id });
  }
  function followReferencingKey(tab: StudioTab, key: InboundForeignKey, target: CellTarget) {
    const query = referencingRowsQuery(key, (name) => {
      const column = tab.schema?.columns.find((c) => c.name === name);
      return column ? resultCache.read(target.resultId, column.id, target.row, true)?.value : null;
    });
    if (!query || !tab.connectionId) {
      state.notify("This row's referenced key contains NULL, so no rows can reference it.");
      return;
    }
    void state.openTable(tab.connectionId, key.source, query);
  }
  async function goToReferencing() {
    if (!current || current.kind !== "table" || !current.table || !tableColumn || !session) {
      state.notify("Select a row in a table to open rows that reference it.");
      return;
    }
    try {
      const all = await client.call<InboundForeignKey[]>("referencing_foreign_keys", {
        sessionId: session.id,
        table: current.table,
      });
      if (!all.length) {
        state.notify(`No visible tables reference ${current.table.schema}.${current.table.name}.`);
        return;
      }
      const relevant = inboundConstraintsForCell(all, tableColumn.column.name);
      if (relevant.length === 1) followReferencingKey(current, relevant[0], tableColumn);
      else
        setDialog({
          kind: "referencing",
          keys: relevant,
          target: tableColumn,
          tabId: current.id,
        });
    } catch (error) {
      state.notify(messageOf(error));
    }
  }
  async function applyTableQuery(tabId: string, query: BrowseQuery, options?: { force?: boolean }) {
    const outcome = await state.setTableQuery(tabId, query, options);
    if (outcome.kind === "refused") {
      state.notify(outcome.reason);
      return;
    }
    if (outcome.kind === "review") {
      const targetTab = useStudio.getState().tabs.find((t) => t.id === tabId);
      const targetProfile = useStudio
        .getState()
        .bootstrap?.profiles.find((p) => p.id === targetTab?.connectionId);
      setDialog({
        kind: "review",
        plan: outcome.plan,
        tabId,
        profile: targetProfile,
        onApprove: async (approval) => {
          const next = await useStudio.getState().setTableQuery(tabId, outcome.query, {
            approval: { planId: outcome.plan.id, approval },
          });
          if (next.kind === "refused") useStudio.getState().notify(next.reason);
        },
      });
    }
  }
  function changeTableQuery(query: BrowseQuery) {
    if (!current) return;
    void applyTableQuery(current.id, query);
  }
  const currentLayout =
    current?.kind === "table"
      ? findTableLayout(state.tableLayouts, current.connectionId, current.table)
      : null;
  const resolvedCurrentLayout =
    current?.kind === "table" && current.schema
      ? resolveTableLayout(current.schema.columns, currentLayout)
      : null;
  const layoutCommands =
    current?.kind === "table" &&
    current.connectionId &&
    current.table &&
    (current.schema?.columns.length ?? 0) > 0
      ? [
          {
            label: "Configure table columns",
            shortcut: "",
            icon: Columns3,
            action: () => setDialog({ kind: "columns", tabId: current.id }),
          },
          ...(resolvedCurrentLayout?.customized
            ? [
                {
                  label: "Reset column layout",
                  shortcut: "",
                  icon: RotateCcw,
                  action: () => state.resetTableLayout(current.connectionId!, current.table!),
                },
              ]
            : []),
        ]
      : [];
  const activeQueryResult =
    current?.kind === "query"
      ? (current.results.find((r) => r.id === current.activeResultId) ?? current.results[0])
      : undefined;
  const hasCopyableGrid =
    (current?.kind === "query" && !!activeQueryResult && activeQueryResult.columns.length > 0) ||
    (current?.kind === "table" &&
      current.view === "data" &&
      (current.schema?.columns.length ?? 0) > 0) ||
    current?.kind === "benchmark";
  const hasExportableData =
    (current?.kind === "query" && !!activeQueryResult && activeQueryResult.columns.length > 0) ||
    (current?.kind === "table" &&
      current.view === "data" &&
      (current.schema?.columns.length ?? 0) > 0);
  const hasImportableTable =
    current?.kind === "table" &&
    current.view === "data" &&
    !!current.table &&
    !!current.schema?.editable &&
    current.schema.columns.some((col) => col.insertable) &&
    !!session &&
    current.dataSessionId === session.id &&
    profile?.safeMode !== "readOnly" &&
    (profile?.engine === "postgres" || profile?.engine === "mysql");
  const hasComparableSources =
    (state.bootstrap?.profiles.some(
      (p) => (p.engine === "postgres" || p.engine === "mysql") && Boolean(state.sessions[p.id]),
    ) ??
      false) ||
    state.tabs.some(
      (t) =>
        (t.kind === "query" && t.results.some((r) => r.columns.length > 0)) ||
        (t.kind === "table" && t.pages.length > 0),
    );
  const copyCommands = [
    ...(hasComparableSources
      ? [
          {
            label: "Compare data…",
            shortcut: "Tables & results",
            icon: ArrowLeftRight,
            action: () => setDialog({ kind: "compare", tabId: current?.id }),
          },
        ]
      : []),
    ...(hasImportableTable && current
      ? [
          {
            label: "Import CSV into table…",
            shortcut: "Reviewed batches",
            icon: Upload,
            action: () => setDialog({ kind: "importCsv", tabId: current.id }),
          },
        ]
      : []),
    ...(hasExportableData && current
      ? [
          {
            label: "Export data to file…",
            shortcut: "CSV · JSON · SQL",
            icon: Download,
            action: () => setDialog({ kind: "export", tabId: current.id }),
          },
        ]
      : []),
    ...(hasCopyableGrid
      ? COPY_FORMATS.map((item) => ({
          label: `Copy as ${item.label}`,
          shortcut: item.hint,
          icon: Copy,
          action: () => void grid.current?.copyAs(item.id),
        }))
      : []),
  ];
  const tableCommands = [
    ...copyCommands,
    ...layoutCommands,
    ...(current?.kind === "table" && session?.capabilities.tableFiltering
      ? [
          ...selectedKeys.map((key, index) => ({
            label: `Go to referenced row in ${key.target.name}${
              selectedKeys.length > 1 ? ` (${key.columns.join(", ")})` : ""
            }`,
            shortcut: index === 0 && selectedKeys.length === 1 ? "⇧⌘ B" : "",
            icon: ArrowUpRight,
            action: () => current && tableColumn && followKey(current, key, tableColumn),
          })),
          ...(tableColumn
            ? [
                {
                  label: "Go to referencing rows",
                  shortcut: "⇧⌘ G",
                  icon: ArrowDownLeft,
                  action: () => void goToReferencing(),
                },
              ]
            : []),
          ...(tableColumn && canOrder(tableColumn.column)
            ? [false, true].map((descending) => ({
                label: `Sort ${descending ? "descending" : "ascending"} by ${tableColumn.column.name}`,
                shortcut: "",
                icon: ArrowUpDown,
                action: () =>
                  changeTableQuery({
                    ...current.query,
                    sort: [{ column: tableColumn.column.name, descending }],
                  }),
              }))
            : []),
          ...(tableColumn && !tableColumn.large && tableColumn.column.kind !== "binary"
            ? [
                {
                  label: `Filter where ${tableColumn.column.name} ${tableColumn.value === null ? "is NULL" : "= selected value"}`,
                  shortcut: "",
                  icon: Filter,
                  action: () =>
                    changeTableQuery(
                      withFilter(current.query, {
                        column: tableColumn.column.name,
                        op: tableColumn.value === null ? "isNull" : "eq",
                        value: tableColumn.value,
                      }),
                    ),
                },
              ]
            : []),
          ...(current.query.sort.length
            ? [
                {
                  label: "Clear table sort",
                  shortcut: "",
                  icon: ArrowUpDown,
                  action: () => changeTableQuery({ ...current.query, sort: [] }),
                },
              ]
            : []),
          ...(current.query.filters.length
            ? [
                {
                  label: "Clear table filters",
                  shortcut: "",
                  icon: Filter,
                  action: () => changeTableQuery({ ...current.query, filters: [] }),
                },
              ]
            : []),
          ...(hasRawClauses(current.query)
            ? [
                {
                  label: "Reset table SQL clauses",
                  shortcut: "",
                  icon: Filter,
                  action: () =>
                    changeTableQuery({
                      ...current.query,
                      whereClause: null,
                      orderClause: null,
                    }),
                },
              ]
            : []),
        ]
      : []),
  ];
  useEffect(() => {
    void state.initialize();
  }, []);
  useEffect(watchWorkspace, []);
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = state.theme;
    document.documentElement.dataset.palette = state.palette;
    document.documentElement.dataset.density = state.density;
  }, [state.theme, state.palette, state.density]);
  useEffect(() => {
    const hasPendingWork = () => {
      const snap = useStudio.getState();
      return (
        snap.tabs.some(
          (tab) =>
            pendingCountFor(tab) > 0 ||
            isTabFileDirty(tab) ||
            ["queued", "running"].includes(tab.status),
        ) ||
        Object.values(snap.sessionStates).some(
          (st) => st.transactionState === "inTransaction" || st.transactionState === "failed",
        )
      );
    };
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (hasPendingWork()) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    if (!isTauri()) {
      window.addEventListener("beforeunload", beforeUnload);
      return () => window.removeEventListener("beforeunload", beforeUnload);
    }
    const windowHandle = getCurrentWindow();
    const closeSafely = async () => {
      try {
        await flushWorkspace();
      } catch {
        // Proceed with closing even if workspace persistence fails.
      }
      await windowHandle.destroy();
    };
    const unsubscribe = windowHandle.onCloseRequested((event) => {
      event.preventDefault();
      if (!hasPendingWork()) {
        void closeSafely().catch((error) => useStudio.getState().notify(messageOf(error)));
        return;
      }
      setDialog({
        kind: "confirm",
        title: "Close Joinery?",
        description:
          "Pending edits will be discarded, running queries cancelled, and open SQL transactions rolled back. Completed writes are not undone.",
        action: closeSafely,
      });
    });
    const quit = windowHandle.listen<number>("joinery:quit-requested", (event) => {
      void client.call("acknowledge_quit", { generation: event.payload });
      if (!hasPendingWork()) {
        void closeSafely().catch((error) => useStudio.getState().notify(messageOf(error)));
        return;
      }
      setDialog({
        kind: "confirm",
        title: "Close Joinery?",
        description:
          "Pending edits will be discarded, running queries cancelled, and open SQL transactions rolled back. Completed writes are not undone.",
        action: closeSafely,
      });
    });
    return () => {
      void unsubscribe.then((off) => off());
      void quit.then((off) => off());
    };
  }, []);
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState !== "hidden")
        void client
          .call<ResourceUsage>("resource_usage")
          .then(setUsage)
          .catch(() => {});
    };
    refresh();
    const timer = setInterval(refresh, 10000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!session) return;
    const timer = setInterval(() => {
      if (document.visibilityState !== "hidden")
        void client
          .call<number>("health", { sessionId: session.id })
          .then((latencyMs) => {
            useStudio.getState().updateSessionLatency(session.id, latencyMs);
          })
          .catch(() => useStudio.getState().retireSession(session.id));
    }, 30000);
    return () => clearInterval(timer);
  }, [session?.id]);

  useEffect(() => {
    pruneTabParamDrafts(state.tabs.map((tab) => tab.id));
  }, [state.tabs]);

  async function executePrepared(
    tabId: string,
    plan: PreparedPlan,
    approval: string | null,
    onSuccess?: () => void,
    origin?: SqlExecutionOrigin,
    preserveResults?: boolean,
  ) {
    const active = useStudio.getState().tabs.find((tab) => tab.id === tabId);
    if (!active) return;
    if (
      active.kind === "query" &&
      querySessionForTab(active, useStudio.getState().sessions)?.id !== plan.sessionId
    ) {
      state.notify("The query session changed. Run the statement again in its current session.");
      return;
    }
    if (active.kind === "query" && !preserveResults) {
      for (const previous of active.results) {
        resultCache.release(previous.id);
        dirtyStore.clearTab(previous.id, [previous.id]);
        await client.call("release_result", { resultId: previous.id });
      }
      state.updateTab(tabId, { results: [], activeResultId: null });
    }
    const revision = active.revision + 1;
    const executionState = useStudio.getState();
    const executionSession = querySessionForTab(active, executionState.sessions);
    state.updateTab(tabId, {
      status: "queued",
      error: null,
      statementErrors: {},
      activeQueryId: null,
      errorOrigin: origin,
      elapsedMs: 0,
      metrics: {},
      revision,
      dataSessionId: plan.sessionId,
      planSource: planSourceForExecution(plan, executionSession, undefined),
      runMetrics: undefined,
      clientStartedAt: performance.now(),
      clientElapsedMs: undefined,
    });
    const batch = beginEventBatch(tabId);
    try {
      const runId = await client.execute(plan.id, approval, (event) => {
        if (event.type === "planContext") {
          const latest = useStudio.getState().tabs.find((tab) => tab.id === tabId);
          if (latest?.revision === revision && (!latest.runId || latest.runId === event.runId))
            state.updateTab(tabId, {
              planSource: planSourceForExecution(plan, executionSession, event.state ?? undefined),
            });
        }
        batch(event);
        if (event.type === "finished" && event.status === "completed") onSuccess?.();
      });
      const latest = useStudio.getState().tabs.find((tab) => tab.id === tabId);
      if (!latest || latest.revision !== revision)
        void client.call("cancel_query", { runId }).catch(() => {});
      else if (latest.status === "queued") state.updateTab(tabId, { runId });
    } catch (error) {
      if (useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision === revision)
        state.updateTab(tabId, {
          status: "failed",
          clientElapsedMs:
            performance.now() -
            (useStudio.getState().tabs.find((tab) => tab.id === tabId)?.clientStartedAt ??
              performance.now()),
          error: appErrorOf(error),
        });
    }
  }
  async function prepareAndExecuteSql(
    tabId: string,
    sessionId: string,
    targetSql: string,
    params: QueryParamInput[] | undefined,
    targetProfile?: ConnectionProfile,
    explain?: "estimate" | "analyze",
    origin?: SqlExecutionOrigin,
  ) {
    const revision = useStudio.getState().tabs.find((tab) => tab.id === tabId)?.revision;
    try {
      const plan = await client.call<PreparedPlan>(
        explain ? "prepare_explain" : "prepare_execution",
        {
          sessionId,
          sql: targetSql,
          ...(params ? { params } : {}),
          ...(explain ? { analyze: explain === "analyze" } : {}),
          errorPolicy: useStudio.getState().batchErrorPolicy,
        },
      );
      if (plan.requiresApproval) {
        setDialog({ kind: "review", plan, tabId, profile: targetProfile, origin });
      } else {
        setDialog((prev) => (prev?.kind === "parameters" ? null : prev));
        await executePrepared(tabId, plan, null, undefined, origin);
      }
    } catch (error) {
      const latest = useStudio.getState().tabs.find((tab) => tab.id === tabId);
      if (
        latest &&
        latest.revision === revision &&
        querySessionForTab(latest, useStudio.getState().sessions)?.id === sessionId
      )
        state.updateTab(tabId, { status: "failed", error: appErrorOf(error), errorOrigin: origin });
      throw error;
    }
  }
  async function runSql(sql?: string, explain?: "estimate" | "analyze", fromUtf16?: number) {
    const active = references.current.current;
    if (!active || active.kind !== "query" || ["queued", "running"].includes(active.status)) return;
    const selected = querySessionForTab(active, useStudio.getState().sessions);
    if (!selected) {
      state.notify("Connect this query tab to a database first.");
      return;
    }
    const target = state.bootstrap?.profiles.find((p) => p.id === active.connectionId);
    const engine = target?.engine ?? selected.engine;
    let origin: SqlExecutionOrigin | undefined;
    try {
      const slice =
        sql !== undefined
          ? { sql, fromUtf16: fromUtf16 ?? 0 }
          : (editor.current?.executionSlice() ?? { sql: active.sql, fromUtf16: 0 });
      const targetSql = slice.sql;
      origin = {
        document: sql === active.sql ? active.sql : editorDocument(active.sql),
        query: targetSql,
        fromUtf16: slice.fromUtf16,
      };
      if (engine !== "redis") {
        const slots = await client.call<QueryParamSlot[]>("extract_query_params", {
          engine,
          sql: targetSql,
        });
        if (slots.length > 0) {
          const profileId = target?.id ?? "";
          let historyEntries: ParameterHistoryEntry[] = [];
          if (profileId) {
            try {
              historyEntries = await client.call<ParameterHistoryEntry[]>("get_parameter_history", {
                profileId,
              });
            } catch {
              // fallback if offline/mocked
            }
          }
          const tabDrafts = getTabParamDrafts(active.id);
          const initialDrafts: Record<string, QueryParamDraft> = {};
          for (const slot of slots) {
            if (tabDrafts[slot.key]) {
              initialDrafts[slot.key] = tabDrafts[slot.key];
            } else {
              const hist = historyEntries.find((h) => h.paramKey === slot.key);
              if (hist) {
                initialDrafts[slot.key] = {
                  kind: hist.kind,
                  value: hist.value ?? "",
                  isNull: hist.isNull,
                };
              }
            }
          }

          setDialog({
            kind: "parameters",
            origin,
            tabId: active.id,
            sessionId: selected.id,
            sql: targetSql,
            slots,
            profile: target,
            initialDrafts,
            allowSaveOnly: false,
            explain,
          });
          return;
        }
      }
      await prepareAndExecuteSql(
        active.id,
        selected.id,
        targetSql,
        undefined,
        target,
        explain,
        origin,
      );
    } catch (error) {
      const latest = useStudio.getState().tabs.find((tab) => tab.id === active.id);
      if (
        latest?.revision === active.revision &&
        querySessionForTab(latest, useStudio.getState().sessions)?.id === selected.id
      )
        state.updateTab(active.id, {
          status: "failed",
          error: appErrorOf(error),
          errorOrigin: origin,
        });
    }
  }
  async function reviewEdits() {
    const active = references.current.current;
    if (!active || (active.kind !== "table" && active.kind !== "query")) return;
    const activeQueryResult =
      active.kind === "query"
        ? (active.results.find((r) => r.id === active.activeResultId) ?? active.results[0])
        : undefined;
    if (active.kind === "query" && activeQueryResult?.editability?.status !== "editable") {
      return;
    }
    const session =
      active.kind === "query"
        ? querySessionForTab(active, useStudio.getState().sessions)
        : active.connectionId
          ? useStudio.getState().sessions[active.connectionId]
          : null;
    if (!session) {
      state.notify(
        active.kind === "query"
          ? "Reconnect and re-run the query before applying edits."
          : "Reconnect and refresh the table before applying edits.",
      );
      return;
    }
    if (active.dataSessionId !== session.id) {
      state.notify(
        active.kind === "query"
          ? "This result belongs to an earlier session. Re-run the query before editing."
          : "This page belongs to an earlier session. Refresh the table before editing.",
      );
      return;
    }
    const scopeId = active.kind === "table" ? active.id : (activeQueryResult?.id ?? active.id);
    const resultIds =
      active.kind === "table"
        ? active.pages.map((page) => page.result.id)
        : activeQueryResult
          ? [activeQueryResult.id]
          : [];
    if (!dirtyStore.pendingCountForTab(scopeId, resultIds)) {
      state.notify("There are no pending edits.");
      return;
    }
    if (active.status === "outcomeUnknown") {
      state.notify("Verify the previous commit, then refresh before editing again.");
      return;
    }
    try {
      const cellUpdates = dirtyFor(active);
      const grouped = new Map<string, typeof cellUpdates>();
      for (const change of cellUpdates)
        grouped.set(change.resultId, [...(grouped.get(change.resultId) ?? []), change]);
      const deletes = resultIds
        .map((resultId) => ({ resultId, rows: dirtyStore.deletesForResult(resultId) }))
        .filter((entry) => entry.rows.length > 0);
      const schemaColumns =
        active.kind === "table"
          ? (active.schema?.columns ?? [])
          : (activeQueryResult?.columns ?? []);
      const columnOrder = new Map(schemaColumns.map((column, index) => [column.id, index]));
      const inserts = dirtyStore.insertsForTab(scopeId).map((draft) => ({
        values: Object.entries(draft.values)
          .sort(([a], [b]) => (columnOrder.get(a) ?? 999999) - (columnOrder.get(b) ?? 999999))
          .map(([column, value]) => ({ column, value })),
      }));
      const changes: TableChanges = {
        resultId: resultIds[0] ?? null,
        updates: [...grouped].map(([resultId, items]) => ({
          resultId,
          changes: items.map(({ row, column, value }) => ({ row, column, value })),
        })),
        deletes,
        inserts,
      };
      const preview = await client.call<ChangePreview>("prepare_table_changes", {
        sessionId: session.id,
        changes,
      });
      const diffRows = buildMutationDiffs(active, dirtyStore, resultCache);
      let currentSelectedIds = new Set(diffRows.map((r) => r.id));

      const onRecomputePlan = async (selectedIds: Set<string>) => {
        currentSelectedIds = selectedIds;
        const filtered = filterTableChanges(changes, selectedIds, diffRows);
        const updatedPreview = await client.call<ChangePreview>("prepare_table_changes", {
          sessionId: session.id,
          changes: filtered,
        });
        return { plan: updatedPreview.plan, preview: updatedPreview };
      };

      setDialog({
        kind: "review",
        plan: preview.plan,
        preview,
        profile,
        tabId: active.id,
        diffRows,
        onRecomputePlan,
        onSuccess: () => {
          const latestTab = useStudio.getState().tabs.find((t) => t.id === active.id);
          const committedDiffs = diffRows.filter((r) => currentSelectedIds.has(r.id));
          const uncommittedCount = diffRows.length - committedDiffs.length;

          const committedUpdates: { resultId: string; row: number; column: string }[] = [];
          for (const diff of committedDiffs) {
            if (diff.ref.kind === "update") {
              const u = diff.ref;
              for (const c of diff.cells) {
                if (c.changed)
                  committedUpdates.push({
                    resultId: u.resultId,
                    row: u.row,
                    column: c.columnId,
                  });
              }
            } else if (diff.ref.kind === "delete") {
              dirtyStore.setDeleted(diff.ref.resultId, [diff.ref.row], false);
            } else if (diff.ref.kind === "insert") {
              dirtyStore.removeInsertRows(diff.ref.tabId, [diff.ref.insertId]);
            }
          }
          if (committedUpdates.length > 0) {
            dirtyStore.revertBatch(committedUpdates);
          }

          if (active.kind === "query" && activeQueryResult) {
            resultCache.release(activeQueryResult.id);
            setSelectedCell(null);
          } else {
            setSelectedCell(null);
            if (latestTab && hasRawClauses(latestTab.query))
              void applyTableQuery(active.id, latestTab.query, { force: true });
            else void state.refreshTable(active.id);
          }

          if (uncommittedCount > 0) {
            state.notify(
              `Committed ${committedDiffs.length} change${committedDiffs.length === 1 ? "" : "s"}; ${uncommittedCount} unstaged change${uncommittedCount === 1 ? "" : "s"} preserved.`,
            );
          } else {
            state.notify("Changes committed");
          }
        },
      });
    } catch (error) {
      state.notify(messageOf(error));
    }
  }
  function confirmDiscard(active: StudioTab, action: () => void) {
    const dirtyFile = isTabFileDirty(active);
    if (
      pendingCountFor(active) > 0 ||
      dirtyFile ||
      ["queued", "running"].includes(active.status) ||
      isLastOwnerOfSecondarySession(useStudio.getState().tabs, active)
    )
      setDialog({
        kind: "confirm",
        title: dirtyFile
          ? `Close ${active.fileName ?? "this workspace"}?`
          : "Close this workspace?",
        description: dirtyFile
          ? `Unsaved changes in ${active.fileName} will be lost and running queries cancelled. Closing an isolated session rolls back its open transactions.`
          : "Pending edits will be discarded and running queries cancelled. Closing an isolated session rolls back its open transactions. Completed writes are not undone.",
        action,
      });
    else action();
  }
  function closeWorkspace(id: string) {
    clearTabParamDrafts(id);
    useStudio.getState().closeTab(id);
    requestAnimationFrame(() =>
      document
        .querySelector<HTMLButtonElement>('.workspace-tabs [role="tab"][aria-selected="true"]')
        ?.focus(),
    );
  }
  function disconnectProfile(profile: ConnectionProfile) {
    const action = async () => {
      const snap = useStudio.getState();
      const session = snap.sessions[profile.id];
      const pooled = (snap.sessionPool[profile.id] ?? []).map((entry) => entry.info.id);
      const owned = snap.tabs
        .filter((t) => t.connectionId === profile.id && t.querySession && !t.querySessionClosed)
        .map((t) => t.querySession!.id);
      await Promise.all(
        [...new Set([...(session ? [session.id] : []), ...pooled, ...owned])].map((sessionId) =>
          client.call("disconnect", { sessionId }),
        ),
      );
      state.resetConnectionResults(profile.id);
    };
    const primarySessionId = state.sessions[profile.id]?.id;
    const pooledSessions = state.sessionPool[profile.id] ?? [];
    if (
      pooledSessions.length > 0 ||
      sessionHasOpenTransaction(state.sessionStates, primarySessionId) ||
      pooledSessions.some((entry) =>
        sessionHasOpenTransaction(state.sessionStates, entry.info.id),
      ) ||
      state.tabs.some(
        (tab) =>
          tab.connectionId === profile.id &&
          (pendingCountFor(tab) > 0 ||
            ["queued", "running"].includes(tab.status) ||
            (tab.querySession && !tab.querySessionClosed)),
      )
    )
      setDialog({
        kind: "confirm",
        title: `Disconnect ${profile.name}?`,
        description:
          "This closes all its sessions and results, discards pending edits, cancels running queries, and rolls back open transactions.",
        action,
      });
    else void action().catch((error) => state.notify(messageOf(error)));
  }
  function handleOpenSqlFile() {
    const active = references.current.current;
    const connId = active?.connectionId ?? useStudio.getState().selectedConnectionId;
    void useStudio
      .getState()
      .openSqlFile(connId)
      .catch((error) => useStudio.getState().notify(messageOf(error)));
  }
  function handleSaveSqlFile(tabId: string, saveAs = false) {
    void useStudio
      .getState()
      .saveSqlFile(tabId, { saveAs })
      .catch((error) => useStudio.getState().notify(messageOf(error)));
  }
  function handleToggleBookmark(activeTab?: StudioTab) {
    const snap = useStudio.getState();
    const targetTab = activeTab ?? references.current.current;
    if (!targetTab) return;
    const candidate = bookmarkTargetForTab(targetTab, snap.catalog);
    if (!candidate) {
      if (targetTab.kind === "query") {
        snap.notify("Save this query to the library first to bookmark it.");
      } else {
        snap.notify("Open a database object or saved query to bookmark it.");
      }
      return;
    }
    void snap
      .toggleBookmark(candidate.target, candidate.defaultLabel)
      .catch((error) => snap.notify(messageOf(error)));
  }
  const actions = useRef({
    runSql,
    reviewEdits,
    goToReferenced,
    goToReferencing,
    handleOpenSqlFile,
    handleSaveSqlFile,
    handleToggleBookmark,
  });
  actions.current = {
    runSql,
    reviewEdits,
    goToReferenced,
    goToReferencing,
    handleOpenSqlFile,
    handleSaveSqlFile,
    handleToggleBookmark,
  };
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (references.current.dialog) return;
      const snap = useStudio.getState();
      const active = references.current.current;
      const matchesAction = (id: ShortcutActionId) =>
        matchesKeyboardEvent(event, effectiveShortcutBinding(id, snap.shortcuts));

      if (matchesAction("openPalette")) {
        event.preventDefault();
        setDialog({ kind: "palette", initialScope: "all" });
        return;
      }
      if (matchesAction("searchObjects")) {
        event.preventDefault();
        setDialog({ kind: "palette", initialScope: "objects" });
        return;
      }
      if (matchesAction("recentLocations")) {
        event.preventDefault();
        setDialog({ kind: "library", tab: "recents" });
        return;
      }
      if (matchesAction("openSqlFile")) {
        event.preventDefault();
        actions.current.handleOpenSqlFile();
        return;
      }
      if (matchesAction("toggleBookmark") && active) {
        event.preventDefault();
        actions.current.handleToggleBookmark(active);
        return;
      }
      if (matchesAction("reopenClosedTab")) {
        event.preventDefault();
        void snap.reopenClosedTab();
        return;
      }
      if (matchesAction("newQuery")) {
        event.preventDefault();
        snap.openQuery();
        return;
      }
      if (matchesAction("togglePinTab") && active) {
        event.preventDefault();
        snap.togglePinTab(active.id);
        return;
      }
      if (matchesAction("closeCleanTabs")) {
        event.preventDefault();
        snap.closeCleanTabs();
        return;
      }
      if (matchesAction("closeOtherTabs") && active) {
        event.preventDefault();
        snap.closeOtherTabs(active.id);
        return;
      }
      if (matchesAction("closeActiveTab") && active) {
        event.preventDefault();
        confirmDiscard(active, () => closeWorkspace(active.id));
        return;
      }
      if (matchesAction("toggleSidebar")) {
        event.preventDefault();
        snap.toggleSidebar();
        return;
      }
      if (matchesAction("toggleInspector")) {
        event.preventDefault();
        snap.toggleInspector();
        return;
      }
      if (matchesAction("preferences")) {
        event.preventDefault();
        setDialog({ kind: "settings" });
        return;
      }
      if (matchesAction("saveSqlFileAs") && active?.kind === "query") {
        event.preventDefault();
        actions.current.handleSaveSqlFile(active.id, true);
        return;
      }
      if (matchesAction("saveSqlFile")) {
        event.preventDefault();
        if (active?.kind === "query") actions.current.handleSaveSqlFile(active.id, false);
        else void actions.current.reviewEdits();
        return;
      }

      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "b" && event.shiftKey) {
        event.preventDefault();
        actions.current.goToReferenced();
      } else if (key === "g" && event.shiftKey) {
        event.preventDefault();
        void actions.current.goToReferencing();
      } else if (key === "." && active?.runId) {
        event.preventDefault();
        void client
          .call("cancel_run", { runId: active.runId })
          .catch((error) => useStudio.getState().notify(messageOf(error)));
      } else if (key === "enter" && event.shiftKey && active?.kind === "query") {
        event.preventDefault();
        void actions.current.runSql(active.sql);
      } else if (key === "f" && event.shiftKey) {
        event.preventDefault();
        editor.current?.format();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
  async function cancelProcess(process: ProcessInfo) {
    if (!current || !session) return;
    try {
      const plan = await client.call<PreparedPlan>("prepare_process_cancel", {
        sessionId: session.id,
        id: process.id,
        identity: process.identity,
      });
      setDialog({ kind: "review", plan, tabId: current.id, profile });
    } catch (error) {
      state.notify(messageOf(error));
    }
  }

  return (
    <div className={`app-shell ${!state.sidebarVisible ? "sidebar-hidden" : ""}`}>
      {state.sidebarVisible && (
        <Sidebar
          onNew={(groupId) => setDialog({ kind: "connection", groupId })}
          onConnect={(profile) => setDialog({ kind: "connect", profile })}
          onEdit={(profile) => {
            if (
              state.tabs.some((tab) => tab.connectionId === profile.id && pendingCountFor(tab) > 0)
            )
              setDialog({
                kind: "confirm",
                title: "Discard pending edits?",
                description:
                  "Editing this connection will reconnect it and close its existing results.",
                action: () => setDialog({ kind: "connection", profile }),
              });
            else setDialog({ kind: "connection", profile });
          }}
          onSettings={() => setDialog({ kind: "settings" })}
          onDisconnect={disconnectProfile}
          onOpenLibrary={(tab) => setDialog({ kind: "library", tab })}
        />
      )}
      <div className="main-shell">
        <header className="workspace-header">
          <div className="workspace-heading">
            <button
              className="icon-button"
              aria-label="Toggle sidebar"
              aria-expanded={state.sidebarVisible}
              title={`Toggle sidebar · ${effectiveShortcutBadge("toggleSidebar", state.shortcuts)}`}
              onClick={() => state.toggleSidebar()}
            >
              {state.sidebarVisible ? <PanelLeftClose size={17} /> : <PanelLeftOpen size={17} />}
            </button>
            <span className="workspace-label">Workspace</span>
            <ChevronRight size={12} />
            {profile ? (
              <>
                <Database size={14} />
                <span>{profile.database}</span>
                <span className={`environment ${profile.environment}`}>{profile.environment}</span>
              </>
            ) : (
              <span className="muted">Overview</span>
            )}
          </div>
          <div className="workspace-tools">
            <button className="command-trigger" onClick={() => setDialog({ kind: "palette" })}>
              <Search size={13} />
              <span>Jump to anything</span>
              <kbd>{effectiveShortcutBadge("openPalette", state.shortcuts)}</kbd>
            </button>
            <button
              className={`icon-button ${state.inspectorVisible ? "active" : ""}`}
              aria-label="Toggle inspector"
              aria-expanded={state.inspectorVisible}
              title={`Toggle inspector · ${effectiveShortcutBadge("toggleInspector", state.shortcuts)}`}
              onClick={() => state.toggleInspector()}
            >
              <PanelRight size={16} />
            </button>
          </div>
        </header>
        <div
          className="workspace-tabs"
          role="tablist"
          aria-label="Workspaces"
          onKeyDown={tabNavigation}
        >
          {state.tabs.map((tab) => (
            <div
              className={`workspace-tab ${tab.id === current?.id ? "active" : ""} ${tab.pinned ? "pinned" : ""}`.trim()}
              key={tab.id}
            >
              <button
                role="tab"
                id={`workspace-tab-${tab.id}`}
                aria-controls="workspace-panel"
                aria-selected={tab.id === current?.id}
                tabIndex={tab.id === current?.id ? 0 : -1}
                onClick={() => {
                  useStudio.setState({ activeTabId: tab.id });
                  setSelectedCell(null);
                }}
              >
                {tab.kind === "query" ? (
                  <FileCode2 size={13} />
                ) : tab.kind === "table" ? (
                  <Database size={13} />
                ) : tab.kind === "object" ? (
                  <Code2 size={13} />
                ) : tab.kind === "history" ? (
                  <History size={13} />
                ) : tab.kind === "processes" ? (
                  <Activity size={13} />
                ) : tab.kind === "diagram" ? (
                  <Network size={13} />
                ) : tab.kind === "schemaDiff" ? (
                  <ArrowLeftRight size={13} />
                ) : (
                  <Gauge size={13} />
                )}
                <span title={tab.title}>{tab.title}</span>
                {(pendingCountFor(tab) > 0 || isTabFileDirty(tab)) && <i className="dirty-dot" />}
                {["queued", "running"].includes(tab.status) && (
                  <span className="tab-running">●</span>
                )}
              </button>
              <button
                type="button"
                className={`tab-pin ${tab.pinned ? "pinned" : ""}`}
                aria-label={tab.pinned ? `Unpin ${tab.title}` : `Pin ${tab.title}`}
                aria-pressed={Boolean(tab.pinned)}
                title={
                  tab.pinned
                    ? `Unpin ${tab.title} · ${effectiveShortcutBadge("togglePinTab", state.shortcuts)}`
                    : `Pin ${tab.title} · ${effectiveShortcutBadge("togglePinTab", state.shortcuts)}`
                }
                onClick={() => state.togglePinTab(tab.id)}
              >
                {tab.pinned ? <PinOff size={11} /> : <Pin size={11} />}
              </button>
              <button
                className="tab-close"
                aria-label={`Close ${tab.title}`}
                onClick={() => confirmDiscard(tab, () => closeWorkspace(tab.id))}
              >
                <X size={12} />
              </button>
            </div>
          ))}
          <button
            className="icon-button new-query"
            aria-label="New query tab"
            title={`New query tab · ${effectiveShortcutBadge("newQuery", state.shortcuts)}`}
            onClick={() => state.openQuery()}
          >
            <Plus size={15} />
          </button>
          {current && state.tabs.some((t) => t.id !== current.id && !t.pinned) && (
            <button
              type="button"
              className="icon-button tab-strip-action"
              aria-label="Close other tabs"
              title={`Close other tabs · ${effectiveShortcutBadge("closeOtherTabs", state.shortcuts)}`}
              onClick={() => state.closeOtherTabs(current.id)}
            >
              Close others
            </button>
          )}
          {state.closedTabs.length > 0 && (
            <button
              type="button"
              className="icon-button tab-strip-action"
              aria-label="Reopen closed tab"
              title={`Reopen closed tab · ${effectiveShortcutBadge("reopenClosedTab", state.shortcuts)}`}
              onClick={() => void state.reopenClosedTab()}
            >
              <RotateCcw size={13} />
            </button>
          )}
        </div>
        <div className="workspace-body">
          <section
            className="workspace-content"
            id={current ? "workspace-panel" : undefined}
            role={current ? "tabpanel" : undefined}
            aria-labelledby={current ? `workspace-tab-${current.id}` : undefined}
          >
            {!current ? (
              <Welcome
                onConnect={() => setDialog({ kind: "connection" })}
                onQuery={() => state.openQuery()}
                onBenchmark={() => state.openUtility("benchmark")}
              />
            ) : current.kind === "history" ? (
              <HistoryView />
            ) : current.kind === "processes" ? (
              <ProcessesView tab={current} onCancel={(process) => void cancelProcess(process)} />
            ) : current.kind === "diagram" ? (
              <DiagramView
                tab={current}
                onOpenTable={(table) => {
                  if (!current.connectionId) return;
                  void state
                    .openTable(current.connectionId, table)
                    .catch((error) => state.notify(messageOf(error)));
                }}
              />
            ) : current.kind === "schemaDiff" ? (
              <SchemaDiffView
                tab={current}
                onOpenObject={(connectionId, object) => {
                  void state
                    .openObject(connectionId, object)
                    .catch((error) => state.notify(messageOf(error)));
                }}
              />
            ) : (
              <Workbench
                tab={current}
                profile={profile}
                session={session ?? undefined}
                editorRef={editor}
                gridRef={grid}
                onRun={(sql, from) => void runSql(sql, undefined, from)}
                onExplain={() => void runSql(undefined, "estimate")}
                onAnalyze={() => void runSql(undefined, "analyze")}
                onOpenParameters={(slots) => {
                  if (current.kind !== "query") return;
                  const selectedSession = querySessionForTab(
                    current,
                    useStudio.getState().sessions,
                  );
                  setDialog({
                    kind: "parameters",
                    origin: { document: current.sql, query: current.sql, fromUtf16: 0 },
                    tabId: current.id,
                    sessionId: selectedSession?.id ?? null,
                    sql: current.sql,
                    slots,
                    profile,
                    allowSaveOnly: true,
                  });
                }}
                onReview={() => void reviewEdits()}
                onConfirm={(title, description, action) =>
                  setDialog({ kind: "confirm", title, description, action })
                }
                onSelect={setSelectedCell}
                onOpenValue={(target) => setDialog({ kind: "value", target })}
                onOpenColumns={() => setDialog({ kind: "columns", tabId: current.id })}
                onOpenExport={() => setDialog({ kind: "export", tabId: current.id })}
                onOpenImport={() => setDialog({ kind: "importCsv", tabId: current.id })}
                onOpenCompare={() => setDialog({ kind: "compare", tabId: current.id })}
                onNewSession={() =>
                  profile && setDialog({ kind: "connect", profile, tabId: current.id })
                }
                onOpenSqlFile={handleOpenSqlFile}
                onSaveSqlFile={(saveAs) => handleSaveSqlFile(current.id, Boolean(saveAs))}
                onSaveToLibrary={() => setDialog({ kind: "saveQuery", tabId: current.id })}
                onToggleBookmark={() => handleToggleBookmark(current)}
                onQueryChange={(query, options) => void applyTableQuery(current.id, query, options)}
              />
            )}
          </section>
          {state.inspectorVisible && (
            <ValueInspectorPanel
              target={selectedCell}
              onClose={() => useStudio.setState({ inspectorVisible: false })}
              onOpenValue={(target) => setDialog({ kind: "value", target })}
              onGoToReferenced={selectedKeys.length > 0 ? goToReferenced : undefined}
              onGoToReferencing={tableColumn ? () => void goToReferencing() : undefined}
              hasReferencedKeys={selectedKeys.length > 0}
              hasReferencingKeys={Boolean(tableColumn)}
            />
          )}
        </div>
        <footer className="status-bar">
          <div>
            {session ? (
              <>
                <span className="online-dot" />
                <span>{profile?.name}</span>
                <span className="status-separator">/</span>
                <span>{session.serverVersion.split(" ").slice(0, 2).join(" ")}</span>
                <span className="latency">
                  <Wifi size={11} />
                  {session.latencyMs} ms
                </span>
                {session.encrypted && <LockKeyhole size={11} />}
              </>
            ) : (
              <>
                <span className="offline-dot" />
                <span>{profile ? `${profile.name} · disconnected` : "No active connection"}</span>
                {profile && (
                  <button onClick={() => setDialog({ kind: "connect", profile })}>Connect</button>
                )}
              </>
            )}
          </div>
          <div>
            <span>
              {usage
                ? `${(usage.retainedBytes / 1024 / 1024).toFixed(1)} MB results`
                : "Memory only"}
            </span>
            <span className="status-separator">·</span>
            <button onClick={() => setDialog({ kind: "palette" })}>
              <Keyboard size={12} />
              Shortcuts
            </button>
          </div>
        </footer>
      </div>
      {state.error && (
        <div className="startup-error" role="alert">
          <InlineError message={state.error} />
          <button className="button ghost" onClick={() => void state.initialize()}>
            Retry
          </button>
        </div>
      )}
      {state.toast && (
        <div className="toast" role="status">
          <Check size={14} />
          <span>{state.toast}</span>
          <button
            className="icon-button"
            aria-label="Dismiss notification"
            onClick={() => useStudio.setState({ toast: null })}
          >
            <X size={12} />
          </button>
        </div>
      )}
      {dialog?.kind === "connection" && (
        <ConnectionDialog
          profile={dialog.profile}
          groupId={dialog.groupId}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "connect" && (
        <ConnectDialog
          profile={dialog.profile}
          onClose={() => setDialog(null)}
          onConnected={
            dialog.tabId
              ? (session) => {
                  if (state.bindQuerySession(dialog.tabId!, session))
                    state.notify("Isolated SQL session connected");
                }
              : undefined
          }
        />
      )}
      {dialog?.kind === "settings" && <Preferences onClose={() => setDialog(null)} />}
      {dialog?.kind === "parameters" && (
        <ParameterDialog
          slots={dialog.slots}
          initialDrafts={dialog.initialDrafts ?? getTabParamDrafts(dialog.tabId)}
          profile={dialog.profile}
          canRun={Boolean(dialog.sessionId)}
          allowSaveOnly={Boolean(dialog.allowSaveOnly)}
          onSaveDrafts={(drafts) => {
            setTabParamDrafts(dialog.tabId, drafts);
            if (dialog.profile?.id) {
              const built = buildQueryParamInputs(dialog.slots, drafts);
              if (built.inputs) {
                void client
                  .call("save_parameter_history", {
                    profileId: dialog.profile.id,
                    entries: built.inputs,
                  })
                  .catch(() => {});
              }
            }
          }}
          onRun={
            dialog.sessionId
              ? async (inputs, drafts) => {
                  setTabParamDrafts(dialog.tabId, drafts);
                  if (dialog.profile?.id) {
                    void client
                      .call("save_parameter_history", {
                        profileId: dialog.profile.id,
                        entries: inputs,
                      })
                      .catch(() => {});
                  }
                  await prepareAndExecuteSql(
                    dialog.tabId,
                    dialog.sessionId!,
                    dialog.sql,
                    inputs,
                    dialog.profile,
                    dialog.explain,
                    dialog.origin,
                  );
                }
              : undefined
          }
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "review" && (
        <ReviewDialog
          plan={dialog.plan}
          preview={dialog.preview}
          profile={dialog.profile}
          diffRows={dialog.diffRows}
          onRecomputePlan={dialog.onRecomputePlan}
          onExecute={async (approval) => {
            if (dialog.onApprove) await dialog.onApprove(approval);
            else
              await executePrepared(
                dialog.tabId,
                dialog.plan,
                approval,
                dialog.onSuccess,
                dialog.origin,
                Boolean(dialog.preview),
              );
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "value" && (
        <ValueDialog target={dialog.target} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === "columns" &&
        (() => {
          const targetTab = state.tabs.find((t) => t.id === dialog.tabId);
          if (!targetTab?.connectionId || !targetTab.table || !targetTab.schema) return null;
          return (
            <TableColumnsDialog
              connectionId={targetTab.connectionId}
              table={targetTab.table}
              columns={targetTab.schema.columns}
              layout={findTableLayout(state.tableLayouts, targetTab.connectionId, targetTab.table)}
              onChange={(next) =>
                state.setTableLayout(targetTab.connectionId!, targetTab.table!, next)
              }
              onClose={() => setDialog(null)}
            />
          );
        })()}
      {dialog?.kind === "export" &&
        (() => {
          const targetTab = state.tabs.find((t) => t.id === dialog.tabId);
          if (!targetTab) return null;
          const targetProfile = state.bootstrap?.profiles.find(
            (p) => p.id === (targetTab.connectionId ?? state.selectedConnectionId),
          );
          const targetSession =
            targetTab.kind === "query"
              ? (querySessionForTab(targetTab, state.sessions) ?? undefined)
              : targetProfile
                ? state.sessions[targetProfile.id]
                : undefined;
          const targetResult =
            targetTab.kind === "query"
              ? (targetTab.results.find((r) => r.id === targetTab.activeResultId) ??
                targetTab.results[0])
              : undefined;
          const allCols =
            targetTab.kind === "table"
              ? (targetTab.schema?.columns ?? [])
              : (targetResult?.columns ?? []);
          const visibleCols =
            targetTab.kind === "table" && targetTab.schema
              ? resolveTableLayout(
                  targetTab.schema.columns,
                  findTableLayout(state.tableLayouts, targetTab.connectionId, targetTab.table),
                ).visibleColumns
              : allCols;
          return (
            <ExportDialog
              tab={targetTab}
              profile={targetProfile}
              session={targetSession}
              visibleColumns={visibleCols}
              allColumns={allCols}
              result={targetResult}
              onClose={() => setDialog(null)}
            />
          );
        })()}
      {dialog?.kind === "importCsv" &&
        (() => {
          const targetTab = state.tabs.find((t) => t.id === dialog.tabId);
          if (!targetTab || targetTab.kind !== "table" || !targetTab.table) {
            return null;
          }
          const targetProfile = state.bootstrap?.profiles.find(
            (p) => p.id === (targetTab.connectionId ?? state.selectedConnectionId),
          );
          const targetSession = targetProfile ? state.sessions[targetProfile.id] : undefined;
          return (
            <ImportCsvDialog
              table={targetTab.table}
              schema={targetTab.schema}
              profile={targetProfile}
              session={targetSession}
              onImported={() => {
                const latestTab = useStudio.getState().tabs.find((t) => t.id === targetTab.id);
                if (!latestTab) return;
                if (hasRawClauses(latestTab.query)) {
                  importRawRefreshTabRef.current = targetTab.id;
                } else {
                  void state.refreshTable(targetTab.id);
                }
              }}
              onClose={() => {
                const pendingRefreshTabId = importRawRefreshTabRef.current;
                importRawRefreshTabRef.current = null;
                setDialog(null);
                if (pendingRefreshTabId === targetTab.id) {
                  const latestTab = useStudio.getState().tabs.find((t) => t.id === targetTab.id);
                  if (latestTab) {
                    void applyTableQuery(targetTab.id, latestTab.query, { force: true });
                  }
                }
              }}
            />
          );
        })()}
      {dialog?.kind === "compare" && (
        <DataCompareDialog
          initialTab={dialog.tabId ? state.tabs.find((t) => t.id === dialog.tabId) : current}
          tabs={state.tabs}
          profiles={state.bootstrap?.profiles ?? []}
          sessions={state.sessions}
          catalog={state.catalog}
          onCatalogLoaded={(connectionId, tables) =>
            useStudio.setState((prev) => ({
              catalog: { ...prev.catalog, [connectionId]: tables },
            }))
          }
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "library" && (
        <LibraryModal
          initialTab={dialog.tab}
          onNeedsConnect={(targetProfile) => setDialog({ kind: "connect", profile: targetProfile })}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "saveQuery" &&
        (() => {
          const targetTab = state.tabs.find((t) => t.id === dialog.tabId);
          if (!targetTab || targetTab.kind !== "query") return null;
          return (
            <SaveQueryDialog
              tab={targetTab}
              profiles={state.bootstrap?.profiles ?? []}
              onClose={() => setDialog(null)}
            />
          );
        })()}
      {dialog?.kind === "confirm" && (
        <Confirmation
          title={dialog.title}
          description={dialog.description}
          onConfirm={dialog.action}
          onClose={() => setDialog((current) => (current === dialog ? null : current))}
        />
      )}
      {dialog?.kind === "references" && (
        <Modal
          title="Go to referenced row"
          subtitle={`${dialog.target.column.name} belongs to more than one foreign key`}
          onClose={() => setDialog(null)}
        >
          <div className="palette-commands">
            {dialog.keys.map((key) => (
              <button
                key={key.name}
                onClick={() => {
                  setDialog(null);
                  const tab = useStudio.getState().tabs.find((t) => t.id === dialog.tabId);
                  if (tab) followKey(tab, key, dialog.target);
                }}
              >
                <ArrowUpRight size={16} />
                <span>
                  {`${key.target.schema}.${key.target.name} (${key.targetColumns.join(", ")})`}
                </span>
                <kbd>{key.name}</kbd>
              </button>
            ))}
          </div>
        </Modal>
      )}
      {dialog?.kind === "referencing" && (
        <Modal
          title="Go to referencing rows"
          subtitle={`More than one foreign key references ${
            state.tabs.find((t) => t.id === dialog.tabId)?.table?.name ?? "this table"
          }`}
          onClose={() => setDialog(null)}
        >
          <div className="palette-commands">
            {dialog.keys.map((key) => (
              <button
                key={`${key.source.schema}.${key.source.name}:${key.name}`}
                onClick={() => {
                  setDialog(null);
                  const tab = useStudio.getState().tabs.find((t) => t.id === dialog.tabId);
                  if (tab) followReferencingKey(tab, key, dialog.target);
                }}
              >
                <ArrowDownLeft size={16} />
                <span>{`${key.source.schema}.${key.source.name} (${key.columns.join(", ")})`}</span>
                <kbd>{key.name}</kbd>
              </button>
            ))}
          </div>
        </Modal>
      )}
      {dialog?.kind === "palette" && (
        <SearchEverywhere
          initialScope={dialog.initialScope ?? "all"}
          onActivateTab={(tabId) => {
            useStudio.setState({ activeTabId: tabId });
            setSelectedCell(null);
          }}
          onClose={() => setDialog(null)}
          commands={[
            {
              label: "New query tab",
              shortcut: effectiveShortcutBadge("newQuery", state.shortcuts),
              contextBadge: "Workspace",
              icon: FileCode2,
              action: () => state.openQuery(),
            },
            {
              label: "Open SQL file…",
              shortcut: effectiveShortcutBadge("openSqlFile", state.shortcuts),
              contextBadge: "SQL & Files",
              icon: FolderOpen,
              action: () => handleOpenSqlFile(),
            },
            ...(current?.kind === "query"
              ? [
                  {
                    label: "Run SQL statement",
                    shortcut: "⌘ ↩",
                    contextBadge: "Active tab",
                    disabledReason: !session
                      ? "Connect this query tab to a database first."
                      : ["queued", "running"].includes(current.status)
                        ? "A query is already running in this tab."
                        : null,
                    icon: Play,
                    action: () => void runSql(),
                  },
                  {
                    label: "Explain query plan",
                    shortcut: "",
                    contextBadge: "Active tab",
                    disabledReason: !session
                      ? "Connect this query tab to a database first."
                      : profile?.engine === "redis"
                        ? "This database engine does not support EXPLAIN plans."
                        : null,
                    icon: Gauge,
                    action: () => void runSql(undefined, "estimate"),
                  },
                  {
                    label: "Save SQL file",
                    shortcut: effectiveShortcutBadge("saveSqlFile", state.shortcuts),
                    contextBadge: "Active tab",
                    icon: Save,
                    action: () => handleSaveSqlFile(current.id, false),
                  },
                  {
                    label: "Save SQL file as…",
                    shortcut: effectiveShortcutBadge("saveSqlFileAs", state.shortcuts),
                    contextBadge: "Active tab",
                    icon: Save,
                    action: () => handleSaveSqlFile(current.id, true),
                  },
                  {
                    label: current.savedQueryId
                      ? "Update query in library…"
                      : "Save query to library…",
                    shortcut: "",
                    contextBadge: "Active tab",
                    icon: BookmarkPlus,
                    action: () => setDialog({ kind: "saveQuery", tabId: current.id }),
                  },
                  ...(profile?.engine !== "redis"
                    ? [
                        {
                          label: "Format SQL",
                          shortcut: "⇧⌘ F",
                          contextBadge: "Active tab",
                          icon: Code2,
                          action: () => {
                            editor.current?.format();
                          },
                        },
                        {
                          label: "Go to SQL definition",
                          shortcut: "F12",
                          contextBadge: "Active tab",
                          icon: ArrowUpRight,
                          action: () => {
                            void editor.current?.goToDefinition();
                          },
                        },
                        {
                          label: "Find SQL usages",
                          shortcut: "⇧F12",
                          contextBadge: "Active tab",
                          icon: Search,
                          action: () => {
                            void editor.current?.findUsages();
                          },
                        },
                        {
                          label: "Quick SQL documentation",
                          shortcut: "F1",
                          contextBadge: "Active tab",
                          icon: Code2,
                          action: () => {
                            void editor.current?.showQuickDocs();
                          },
                        },
                        {
                          label: "Previous SQL statement",
                          shortcut: "⌥⌘ ↑",
                          contextBadge: "Active tab",
                          icon: Code2,
                          action: () => {
                            editor.current?.navigateStatement(-1);
                          },
                        },
                        {
                          label: "Next SQL statement",
                          shortcut: "⌥⌘ ↓",
                          contextBadge: "Active tab",
                          icon: Code2,
                          action: () => {
                            editor.current?.navigateStatement(1);
                          },
                        },
                        ...(resolveExecutionError(current.sql, current.error, current.errorOrigin)
                          ? [
                              {
                                label: "Jump to SQL error",
                                shortcut: "",
                                contextBadge: "Active tab",
                                icon: Code2,
                                action: () => {
                                  const latest = useStudio
                                    .getState()
                                    .tabs.find((tab) => tab.id === current.id);
                                  const resolved =
                                    latest &&
                                    resolveExecutionError(
                                      latest.sql,
                                      latest.error,
                                      latest.errorOrigin,
                                    );
                                  if (resolved)
                                    editor.current?.revealRange(resolved.from, resolved.to);
                                },
                              },
                            ]
                          : []),
                      ]
                    : []),
                ]
              : []),
            ...(current?.kind === "table"
              ? [
                  {
                    label: "Switch table view to Data",
                    shortcut: "",
                    contextBadge: "Active tab",
                    disabledReason: current.view === "data" ? "Already viewing table data." : null,
                    icon: Database,
                    action: () => state.updateTab(current.id, { view: "data" }),
                  },
                  {
                    label: "Switch table view to Structure",
                    shortcut: "",
                    contextBadge: "Active tab",
                    disabledReason:
                      current.view === "structure" ? "Already viewing table structure." : null,
                    icon: Columns3,
                    action: () => state.updateTab(current.id, { view: "structure" }),
                  },
                  {
                    label: "Switch table view to DDL",
                    shortcut: "",
                    contextBadge: "Active tab",
                    disabledReason: current.view === "ddl" ? "Already viewing table DDL." : null,
                    icon: Code2,
                    action: () => state.updateTab(current.id, { view: "ddl" }),
                  },
                ]
              : []),
            ...(current && bookmarkTargetForTab(current, state.catalog)
              ? [
                  {
                    label: "Toggle bookmark",
                    shortcut: effectiveShortcutBadge("toggleBookmark", state.shortcuts),
                    contextBadge: "Active tab",
                    icon: Star,
                    action: () => handleToggleBookmark(current),
                  },
                ]
              : []),
            {
              label: current?.pinned ? "Unpin active tab" : "Pin active tab",
              shortcut: effectiveShortcutBadge("togglePinTab", state.shortcuts),
              contextBadge: "Tabs",
              disabledReason: !current ? "Open a tab first to pin or unpin it." : null,
              icon: current?.pinned ? PinOff : Pin,
              action: () => current && state.togglePinTab(current.id),
            },
            {
              label: "Close active tab",
              shortcut: effectiveShortcutBadge("closeActiveTab", state.shortcuts),
              contextBadge: "Tabs",
              disabledReason: !current ? "No active tab to close." : null,
              icon: X,
              action: () => current && confirmDiscard(current, () => closeWorkspace(current.id)),
            },
            {
              label: "Close other tabs",
              shortcut: effectiveShortcutBadge("closeOtherTabs", state.shortcuts),
              contextBadge: "Tabs",
              disabledReason: !current
                ? "No active tab selected."
                : !state.tabs.some((t) => t.id !== current.id && !t.pinned)
                  ? "No other unpinned tabs to close."
                  : null,
              icon: X,
              action: () => current && state.closeOtherTabs(current.id),
            },
            {
              label: "Close unmodified tabs",
              shortcut: effectiveShortcutBadge("closeCleanTabs", state.shortcuts),
              contextBadge: "Tabs",
              disabledReason: !state.tabs.some(
                (t) => !t.pinned && !isTabProtectedFromBulkClose(state.tabs, t),
              )
                ? "No unmodified unpinned tabs to close."
                : null,
              icon: X,
              action: () => state.closeCleanTabs(),
            },
            {
              label: "Reopen closed tab",
              shortcut: effectiveShortcutBadge("reopenClosedTab", state.shortcuts),
              contextBadge: "Tabs",
              disabledReason:
                state.closedTabs.length === 0 ? "No recently closed tabs to reopen." : null,
              icon: RotateCcw,
              action: () => void state.reopenClosedTab(),
            },
            {
              label: "Toggle left sidebar",
              shortcut: effectiveShortcutBadge("toggleSidebar", state.shortcuts),
              contextBadge: "Layout",
              icon: state.sidebarVisible ? PanelLeftClose : PanelLeftOpen,
              action: () => state.toggleSidebar(),
            },
            {
              label: "Toggle right inspector",
              shortcut: effectiveShortcutBadge("toggleInspector", state.shortcuts),
              contextBadge: "Layout",
              icon: PanelRight,
              action: () => state.toggleInspector(),
            },
            {
              label: "Saved queries library",
              shortcut: "",
              contextBadge: "Navigation",
              icon: Bookmark,
              action: () => setDialog({ kind: "library", tab: "saved" }),
            },
            {
              label: "Bookmarks",
              shortcut: "",
              contextBadge: "Navigation",
              icon: Star,
              action: () => setDialog({ kind: "library", tab: "bookmarks" }),
            },
            {
              label: "Recent locations",
              shortcut: effectiveShortcutBadge("recentLocations", state.shortcuts),
              contextBadge: "Navigation",
              icon: Clock3,
              action: () => setDialog({ kind: "library", tab: "recents" }),
            },
            {
              label: "New database connection",
              shortcut: "",
              contextBadge: "Workspace",
              icon: Database,
              action: () => setDialog({ kind: "connection" }),
            },
            {
              label: "Query history",
              shortcut: "",
              contextBadge: "Workspace",
              icon: History,
              action: () => state.openUtility("history"),
            },
            {
              label: "Grid benchmark",
              shortcut: "1M rows",
              contextBadge: "Workspace",
              icon: Gauge,
              action: () => state.openUtility("benchmark"),
            },
            ...(() => {
              const connectionId = current?.connectionId ?? state.selectedConnectionId;
              if (!connectionId || !state.sessions[connectionId]) return [];
              const schemas = [
                ...new Set((state.catalog[connectionId] ?? []).map((t) => t.table.schema)),
              ];
              const preferred = current?.table?.schema ?? current?.diagramSchema;
              const ordered =
                preferred && schemas.includes(preferred)
                  ? [preferred, ...schemas.filter((s) => s !== preferred)]
                  : schemas;
              return ordered.slice(0, 20).flatMap((schema) => [
                {
                  label: `Open schema diagram: ${schema}`,
                  shortcut: "",
                  contextBadge: "Schema",
                  icon: Network,
                  action: () => state.openDiagram(connectionId, schema),
                },
                {
                  label: `Compare schema: ${schema}`,
                  shortcut: "",
                  contextBadge: "Schema",
                  icon: ArrowLeftRight,
                  action: () => state.openSchemaDiff(connectionId, schema),
                },
              ]);
            })(),
            {
              label: "Preferences",
              shortcut: effectiveShortcutBadge("preferences", state.shortcuts),
              contextBadge: "Workspace",
              icon: Command,
              action: () => setDialog({ kind: "settings" }),
            },
            ...(current?.kind === "query" && session?.capabilities.transactions
              ? (() => {
                  const runtime = state.sessionStates[session.id];
                  const txMode = runtime?.transactionMode ?? "autoCommit";
                  const txState = runtime?.transactionState ?? "idle";
                  return [
                    {
                      label:
                        txMode === "manual"
                          ? "Switch session to auto-commit"
                          : "Switch session to manual commit",
                      shortcut: "",
                      icon: Activity,
                      action: () =>
                        void state
                          .setTransactionMode(
                            session.id,
                            txMode === "manual" ? "autoCommit" : "manual",
                          )
                          .catch((error) => state.notify(messageOf(error))),
                    },
                    ...(txState === "inTransaction" && profile?.safeMode !== "readOnly"
                      ? [
                          {
                            label: "Commit transaction",
                            shortcut: "",
                            icon: Check,
                            action: () =>
                              void state
                                .commitTransaction(session.id)
                                .catch((error) => state.notify(messageOf(error))),
                          },
                        ]
                      : []),
                    ...(txState !== "idle"
                      ? [
                          {
                            label: "Rollback transaction",
                            shortcut: "",
                            icon: RotateCcw,
                            action: () =>
                              void state
                                .rollbackTransaction(session.id)
                                .catch((error) => state.notify(messageOf(error))),
                          },
                        ]
                      : []),
                  ];
                })()
              : []),
            ...tableCommands,
          ]}
        />
      )}
    </div>
  );
}

function Confirmation({
  title,
  description,
  onConfirm,
  onClose,
}: {
  title: string;
  description: string;
  onConfirm(): void | Promise<void>;
  onClose(): void;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  return (
    <Modal title={title} onClose={onClose} busy={busy}>
      <div className="modal-body">
        <p className="confirmation-description">{description}</p>
        <InlineError message={error} />
      </div>
      <footer className="modal-footer">
        <button className="button ghost" disabled={busy} onClick={onClose}>
          Keep working
        </button>
        <button
          className="button danger"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void Promise.resolve(onConfirm())
              .then(onClose)
              .catch((error) => {
                setError(messageOf(error));
                setBusy(false);
              });
          }}
        >
          Continue
        </button>
      </footer>
    </Modal>
  );
}
