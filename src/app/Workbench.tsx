import { ExecutionTiming } from "../features/studio/ExecutionTiming";
import { ExecutionSummary } from "../features/studio/ExecutionSummary";
import {
  lazy,
  Suspense,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";
import {
  ArrowLeftRight,
  ArrowRight,
  BookmarkPlus,
  Check,
  ChevronDown,
  Clock3,
  Code2,
  Columns3,
  Copy,
  Download,
  ExternalLink,
  FileCode2,
  FolderOpen,
  Gauge,
  Loader2,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  ShieldCheck,
  Square,
  Star,
  Table2,
  Trash2,
  Unplug,
  Upload,
  X,
} from "lucide-react";
import type {
  ConnectionProfile,
  DatabaseObjectRef,
  ObjectDetails,
  QueryParamSlot,
  ResultInfo,
  SessionInfo,
  TableInfo,
  TransactionMode,
  ExplicitTransactionAction,
} from "../ipc/generated";
import type { StudioTab } from "../state/studio";
import {
  useStudio,
  querySessionForTab,
  isLastOwnerOfSecondarySession,
  sessionHasOpenTransaction,
  isTabFileDirty,
  bookmarkTargetForTab,
  findBookmarkForTarget,
  isSnowflakeConsoleSession,
  assertOrdinarySession,
  isConsoleOwnedResult,
} from "../state/studio";
import { client, messageOf } from "../ipc/client";
import { dirtyStore } from "../data/dirty-store";
import type { CellTarget, DatabaseGridHandle } from "../features/grid/DatabaseGrid";
import {
  RecordCardView,
  TransposeGridView,
  type GridDisplayMode,
} from "../features/grid/GridAltViews";
import { CopyAsMenu } from "../features/grid/CopyAsMenu";
import { TableFilterBar } from "../features/grid/TableFilterBar";
import { findTableLayout, resolveTableLayout } from "../features/grid/table-layout";
import { cycleSort, hasRawClauses } from "../features/grid/table-query";
import { resolveExecutionError } from "../features/studio/sql-navigation";
import type { EditorHandle } from "../features/studio/SqlEditor";
import {
  DdlPanel,
  ObjectInspectorView,
  ObjectKindGlyph,
} from "../features/studio/ObjectInspectorView";
import { SessionStructureView } from "../features/studio/SessionStructureView";
import { RelationBrowser } from "../features/studio/RelationBrowser";
import { SnowflakeDdlNotice } from "../features/studio/SnowflakeDdlNotice";
import { StructureView } from "../features/studio/UtilityViews";
import { objectTabTitle, tableInfoKindToObjectKind } from "../features/connections/explorer-tree";
import { RedisTools } from "../features/redis/RedisTools";
import { resultCache } from "../data/result-cache";
import { tabNavigation } from "../components/ui/tabs";
import { QueryPlanView, isQueryPlanResult } from "../features/studio/QueryPlanView";
import {
  SnowflakeDmlNotice,
  SnowflakeDmlPendingControl,
} from "../features/studio/SnowflakeDmlNotice";

import { consoleReceiptBlocksSession } from "../features/studio/SnowflakeConsole";

const SqlEditor = lazy(() =>
  import("../features/studio/SqlEditor").then((module) => ({ default: module.SqlEditor })),
);
const DatabaseGrid = lazy(() => import("../features/grid/DatabaseGrid"));
const EMPTY_TABLES: TableInfo[] = [];
const BENCHMARK_ALT_COLUMNS = [
  ["id", "bigint", "integer"],
  ["workspace", "text", "text"],
  ["status", "text", "text"],
  ["owner", "text", "text"],
  ["email", "text", "text"],
  ["amount", "numeric", "decimal"],
  ["region", "text", "text"],
  ["created_at", "timestamptz", "temporal"],
].map(([name, databaseType, kind], i) => ({
  id: `c${i}`,
  name,
  databaseType,
  kind: kind as ResultInfo["columns"][number]["kind"],
  nullable: false,
  primaryKey: i === 0,
  foreignKey: false,
  editable: false,
  insertable: false,
  hasDefault: false,
}));
interface Props {
  tab: StudioTab;
  profile?: ConnectionProfile;
  session?: SessionInfo;
  editorRef: RefObject<EditorHandle>;
  gridRef: RefObject<DatabaseGridHandle>;
  onRun(sql?: string, fromUtf16?: number): void;
  onRunConsoleScript?(): void;
  onRunConsoleSelected?(): void;
  onExplain(): void;
  onAnalyze(): void;
  onOpenParameters(slots: QueryParamSlot[]): void;
  onReview(): void;
  onTransactionControl(action: ExplicitTransactionAction): Promise<void>;
  onConfirm(title: string, description: string, action: () => void): void;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
  onOpenColumns(): void;
  onOpenExport(): void;
  onOpenImport(): void;
  onOpenCompare(): void;
  onNewSession(): void;
  onOpenSqlFile(): void;
  onSaveSqlFile(saveAs?: boolean): void;
  onSaveToLibrary(): void;
  onToggleBookmark(): void;
  onQueryChange(query: StudioTab["query"], options?: { force?: boolean }): void;
}

export function Workbench({
  tab,
  profile,
  session,
  editorRef,
  gridRef,
  onRun,
  onRunConsoleScript,
  onRunConsoleSelected,
  onExplain,
  onAnalyze,
  onOpenParameters,
  onReview,
  onTransactionControl,
  onConfirm,
  onSelect,
  onOpenValue,
  onOpenColumns,
  onOpenExport,
  onOpenImport,
  onOpenCompare,
  onNewSession,
  onOpenSqlFile,
  onSaveSqlFile,
  onSaveToLibrary,
  onToggleBookmark,
  onQueryChange,
}: Props) {
  const state = useStudio(),
    [editorSize, setEditorSize] = useState(290),
    [runningTime, setRunningTime] = useState(0),
    [planResultId, setPlanResultId] = useState<string | null>(null),
    [selectedRows, setSelectedRows] = useState<number[]>([]),
    [activeCell, setActiveCell] = useState<CellTarget | null>(null),
    [focusRowRequest, setFocusRowRequest] = useState<{ seq: number; row: number } | null>(null),
    [tableDetailsState, setTableDetailsState] = useState<{
      key: string;
      data: ObjectDetails;
    } | null>(null),
    [tableDetailsErrorState, setTableDetailsErrorState] = useState<{
      key: string;
      message: string;
    } | null>(null),
    [tableDetailsLoadingKey, setTableDetailsLoadingKey] = useState<string | null>(null),
    [renamingSessionId, setRenamingSessionId] = useState<string | null>(null),
    [renameDraft, setRenameDraft] = useState(""),
    [txBusy, setTxBusy] = useState(false),
    [relationBrowserSession, setRelationBrowserSession] = useState<string | null>(null),
    [relationBrowserRevision, setRelationBrowserRevision] = useState<number | null>(null),
    editorHeight = useRef(290);
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const consoleMode = !!session && isSnowflakeConsoleSession(state, session.id);
  const heldReceipt = Object.values(state.snowflakeConsoleReceipts).find(
    (receipt) => !!session && consoleReceiptBlocksSession(receipt, session.id),
  );
  const objectExplorerSupported = !consoleMode && Boolean(session?.capabilities.objectExplorer);
  const tableCatalogMatch =
    tab.kind === "table" && tab.table && tab.connectionId
      ? state.catalog[tab.connectionId]?.find(
          (entry) =>
            entry.table.schema === tab.table!.schema && entry.table.name === tab.table!.name,
        )
      : undefined;
  const tableObjectKind = tableCatalogMatch
    ? tableInfoKindToObjectKind(tableCatalogMatch.kind)
    : "table";
  const tableObjectKey =
    tab.kind === "table" && tab.table && session?.id
      ? `${session.id}:${tab.id}:${tab.revision}:${tab.table.schema}:${tab.table.name}`
      : null;
  const activeTableDetails =
    tableObjectKey && tableDetailsState?.key === tableObjectKey ? tableDetailsState.data : null;
  const activeTableDetailsError =
    tableObjectKey && tableDetailsErrorState?.key === tableObjectKey
      ? tableDetailsErrorState.message
      : null;
  const activeTableDetailsLoading = Boolean(
    tableObjectKey && tableDetailsLoadingKey === tableObjectKey,
  );
  const controlPending = !!session && !!state.sessionTransactionPending[session.id];
  const sessionQueryPending =
    !!session &&
    state.tabs.some(
      (item) =>
        querySessionForTab(item, state.sessions)?.id === session.id &&
        (item.status === "queued" || item.status === "running"),
    );
  const rollbackOnly =
    session &&
    state.sessionStates[session.id]?.transactionControlAuthority?.status === "rollbackOnly";
  const running =
    tab.status === "running" || tab.status === "queued" || controlPending || !!heldReceipt;
  const dmlPending =
    tab.kind === "query" && tab.status === "running" && !tab.runId && controlPending;
  const result = tab.results.find((r) => r.id === tab.activeResultId) ?? tab.results[0];
  const queryResultEditable =
    tab.kind === "query" &&
    !consoleMode &&
    !!result &&
    result.editability?.status === "editable" &&
    profile?.safeMode !== "readOnly" &&
    !!session &&
    tab.dataSessionId === session.id &&
    (!isQueryPlanResult(result, tab.planSource) || planResultId !== result.id);
  const insertScopeId = tab.kind === "table" ? tab.id : (result?.id ?? tab.id);
  const resultIds =
    tab.kind === "table" ? tab.pages.map((page) => page.result.id) : result ? [result.id] : [];
  const pendingCount =
    tab.kind === "table"
      ? dirtyStore.pendingCountForTab(tab.id, resultIds)
      : result
        ? dirtyStore.pendingCountForTab(result.id, [result.id])
        : 0;
  const draftInserts =
    tab.kind === "table" || queryResultEditable ? dirtyStore.insertsForTab(insertScopeId) : [];
  const rawActiveColumns =
    tab.kind === "table"
      ? (tab.schema?.columns ?? [])
      : tab.kind === "benchmark"
        ? BENCHMARK_ALT_COLUMNS
        : (result?.columns ?? []);
  const tableColumns = tab.kind === "table" ? (tab.schema?.columns ?? []) : [];
  const activeDataLayout =
    tab.kind === "table"
      ? findTableLayout(state.tableLayouts, tab.connectionId, tab.table)
      : result
        ? (tab.resultLayouts?.[result.id] ?? null)
        : null;
  const resolvedLayout = resolveTableLayout(rawActiveColumns, activeDataLayout);
  const gridMode: GridDisplayMode = tab.gridMode ?? "grid";
  const handleSelectCell = (target: CellTarget | null) => {
    setActiveCell(target);
    onSelect(target);
  };
  const filtering = tab.kind === "table" && !!session?.capabilities.tableFiltering;
  const tableEditable =
    tab.kind === "table" &&
    !consoleMode &&
    tab.view === "data" &&
    !!tab.schema?.editable &&
    !!session &&
    tab.dataSessionId === session.id;
  const tableImportable =
    tableEditable &&
    profile?.safeMode !== "readOnly" &&
    (profile?.engine === "postgres" || profile?.engine === "mysql") &&
    tab.schema!.columns.some((col) => col.insertable);
  const mutatingDisabled = running || tab.loadingPage || tab.status === "outcomeUnknown";
  const selectedDraftIds: string[] = [];
  const selectedExisting: { resultId: string; row: number }[] = [];
  if (tab.kind === "table") {
    for (const visualRow of selectedRows) {
      if (visualRow >= tab.loadedRows) {
        const draft = draftInserts[visualRow - tab.loadedRows];
        if (draft) selectedDraftIds.push(draft.id);
      } else if (visualRow >= 0) {
        const page = tab.pages.find(
          (entry) =>
            visualRow >= entry.start && visualRow < entry.start + entry.result.retainedRows,
        );
        if (page) selectedExisting.push({ resultId: page.result.id, row: visualRow - page.start });
      }
    }
  } else if (tab.kind === "query" && result) {
    for (const visualRow of selectedRows) {
      if (visualRow >= result.retainedRows) {
        const draft = draftInserts[visualRow - result.retainedRows];
        if (draft) selectedDraftIds.push(draft.id);
      } else if (visualRow >= 0) {
        selectedExisting.push({ resultId: result.id, row: visualRow });
      }
    }
  }
  const onlyDraftsSelected = selectedDraftIds.length > 0 && selectedExisting.length === 0;
  const allExistingDeleted =
    selectedExisting.length > 0 &&
    selectedDraftIds.length === 0 &&
    selectedExisting.every((entry) => dirtyStore.isDeleted(entry.resultId, entry.row));
  const undeletedExisting = selectedExisting.filter(
    (entry) => !dirtyStore.isDeleted(entry.resultId, entry.row),
  );
  const deleteTargetCount = undeletedExisting.length + selectedDraftIds.length;
  const queryLock = !session
    ? "Connect to change sort or filters"
    : tab.dataSessionId !== session.id
      ? "Refresh after reconnecting"
      : pendingCount
        ? "Review or discard pending edits to change sort or filters"
        : null;
  const hasSelectedDirtyRows =
    selectedDraftIds.length > 0 ||
    selectedExisting.some(
      (entry) =>
        dirtyStore.isDeleted(entry.resultId, entry.row) ||
        dirtyStore.forResult(entry.resultId).some((c) => c.row === entry.row),
    );
  const hasActiveDirtyCell =
    Boolean(activeCell) &&
    (Boolean(activeCell!.insert) ||
      Boolean(activeCell!.deleted) ||
      Boolean(dirtyStore.get(activeCell!.resultId, activeCell!.row, activeCell!.column.id)));
  const canRevertSelected = hasSelectedDirtyRows || hasActiveDirtyCell;
  const cloneCandidateVisualRow: number | null =
    selectedRows.length > 0
      ? selectedRows[0]
      : activeCell
        ? activeCell.insert
          ? activeCell.row
          : tab.kind === "table"
            ? (() => {
                const page = tab.pages.find((p) => p.result.id === activeCell.resultId);
                return page ? page.start + activeCell.row : null;
              })()
            : activeCell.row
        : null;
  function stageInsertRow() {
    try {
      const baseRows = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
      dirtyStore.addInsertRow(insertScopeId);
      const nextRow = baseRows + dirtyStore.insertsForTab(insertScopeId).length - 1;
      setFocusRowRequest((prev) => ({ seq: (prev?.seq ?? 0) + 1, row: nextRow }));
    } catch (error) {
      state.notify(error instanceof Error ? error.message : "Could not stage inserted row");
    }
  }
  async function cloneSelectedRow() {
    if (cloneCandidateVisualRow === null) {
      state.notify("Select a row or cell first to clone it.");
      return;
    }
    try {
      const baseRows = tab.kind === "table" ? tab.loadedRows : (result?.retainedRows ?? 0);
      const cols = tab.kind === "table" ? (tab.schema?.columns ?? []) : (result?.columns ?? []);
      const values: Record<string, string | null> = {};
      if (cloneCandidateVisualRow >= baseRows) {
        const sourceDraft = draftInserts[cloneCandidateVisualRow - baseRows];
        if (!sourceDraft) return;
        for (const col of cols) {
          if (!col.insertable || (col.primaryKey && col.hasDefault)) continue;
          if (col.id in sourceDraft.values) {
            values[col.id] = sourceDraft.values[col.id] ?? null;
          }
        }
      } else {
        const source =
          tab.kind === "table"
            ? (() => {
                const page = tab.pages.find(
                  (p) =>
                    cloneCandidateVisualRow >= p.start &&
                    cloneCandidateVisualRow < p.start + p.result.retainedRows,
                );
                return page
                  ? { result: page.result, row: cloneCandidateVisualRow - page.start }
                  : null;
              })()
            : result
              ? { result, row: cloneCandidateVisualRow }
              : null;
        if (!source) {
          state.notify("Scroll to load that row before cloning it.");
          return;
        }
        await resultCache.ensure(source.result.id, source.row, source.result.retainedRows);
        for (const col of cols) {
          if (!col.insertable || (col.primaryKey && col.hasDefault)) continue;
          const dirty = dirtyStore.get(source.result.id, source.row, col.id);
          const raw = dirty
            ? dirty.value
            : (resultCache.read(source.result.id, col.id, source.row, true)?.value ?? null);
          values[col.id] = raw;
        }
      }
      dirtyStore.cloneInsertRow(insertScopeId, values);
      const nextRow = baseRows + dirtyStore.insertsForTab(insertScopeId).length - 1;
      setFocusRowRequest((prev) => ({ seq: (prev?.seq ?? 0) + 1, row: nextRow }));
      state.notify("Cloned row into staged inserts");
    } catch (error) {
      state.notify(error instanceof Error ? error.message : "Could not clone row");
    }
  }
  function handleRevertSelection() {
    try {
      if (hasSelectedDirtyRows) {
        if (selectedDraftIds.length) {
          dirtyStore.removeInsertRows(insertScopeId, selectedDraftIds);
        }
        const byResult = new Map<string, number[]>();
        for (const entry of selectedExisting) {
          byResult.set(entry.resultId, [...(byResult.get(entry.resultId) ?? []), entry.row]);
        }
        for (const [resultId, rows] of byResult) {
          dirtyStore.revertRows(resultId, rows);
        }
        return;
      }
      if (activeCell) {
        if (activeCell.insert) {
          dirtyStore.removeInsertRows(activeCell.insert.tabId, [activeCell.insert.insertId]);
        } else if (activeCell.deleted) {
          dirtyStore.setDeleted(activeCell.resultId, [activeCell.row], false);
        } else {
          dirtyStore.revertCell(activeCell.resultId, activeCell.row, activeCell.column.id);
        }
      }
    } catch (error) {
      state.notify(error instanceof Error ? error.message : "Could not revert selected edits");
    }
  }
  function handleDeleteSelection() {
    try {
      if (onlyDraftsSelected) {
        dirtyStore.removeInsertRows(insertScopeId, selectedDraftIds);
        return;
      }
      if (allExistingDeleted) {
        const byResult = new Map<string, number[]>();
        for (const entry of selectedExisting) {
          byResult.set(entry.resultId, [...(byResult.get(entry.resultId) ?? []), entry.row]);
        }
        for (const [resultId, rows] of byResult) dirtyStore.setDeleted(resultId, rows, false);
        return;
      }
      if (selectedDraftIds.length) dirtyStore.removeInsertRows(insertScopeId, selectedDraftIds);
      const byResult = new Map<string, number[]>();
      for (const entry of undeletedExisting) {
        byResult.set(entry.resultId, [...(byResult.get(entry.resultId) ?? []), entry.row]);
      }
      for (const [resultId, rows] of byResult) dirtyStore.setDeleted(resultId, rows, true);
    } catch (error) {
      state.notify(error instanceof Error ? error.message : "Could not update row deletion");
    }
  }
  function applyQuery(query: StudioTab["query"]) {
    onQueryChange(query);
  }
  const pool = tab.connectionId ? (state.sessionPool[tab.connectionId] ?? []) : [];
  const primarySession = tab.connectionId ? state.sessions[tab.connectionId] : undefined;
  const totalSessions = (primarySession ? 1 : 0) + pool.length;
  const activePoolEntry = tab.querySession
    ? pool.find((entry) => entry.info.id === tab.querySession!.id)
    : undefined;
  const sessionLabel = activePoolEntry?.label ?? null;
  const runtimeState = session ? state.sessionStates[session.id] : undefined;
  useEffect(() => {
    if (controlPending) setRelationBrowserSession(null);
  }, [controlPending]);
  const txMode: TransactionMode = runtimeState?.transactionMode ?? "unknown";
  const txState = runtimeState?.transactionState ?? "unknown";
  const fileDirty = isTabFileDirty(tab);
  const bookmarkCandidate = bookmarkTargetForTab(tab, state.catalog);
  const activeBookmark = bookmarkCandidate
    ? findBookmarkForTarget(state.bookmarks, bookmarkCandidate.target)
    : undefined;
  function changeConnection(connectionId: string) {
    const lastOwner = isLastOwnerOfSecondarySession(state.tabs, tab);
    const apply = () => {
      if (lastOwner && tab.querySession && !tab.querySessionClosed)
        void state
          .disconnectQuerySession(tab.querySession.id)
          .catch((error) => state.notify(messageOf(error)));
      for (const result of tab.results) {
        resultCache.release(result.id);
        dirtyStore.clearTab(result.id, [result.id]);
        if (!isConsoleOwnedResult(result))
          void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
      state.updateTab(tab.id, {
        connectionId: connectionId || null,
        results: [],
        activeResultId: null,
        status: "idle",
        error: null,
        metrics: {},
        executionSummary: undefined,
        dataSessionId: null,
        querySession: null,
        querySessionClosed: false,
        revision: tab.revision + 1,
      });
      onSelect(null);
    };
    if (lastOwner)
      onConfirm(
        "Change query connection?",
        "This closes the isolated SQL session. Verify any open or unknown transaction on the database; remote rollback may not be immediate.",
        apply,
      );
    else apply();
  }
  useEffect(() => {
    if (!running) return;
    const start = performance.now();
    setRunningTime(0);
    const timer = setInterval(() => setRunningTime(performance.now() - start), 200);
    return () => clearInterval(timer);
  }, [running, tab.id]);
  useEffect(() => {
    if (
      tab.kind !== "table" ||
      !tab.table ||
      !session?.id ||
      !objectExplorerSupported ||
      (tab.view !== "structure" && tab.view !== "ddl") ||
      !tableObjectKey
    ) {
      return;
    }
    if (tableDetailsState?.key === tableObjectKey) return;
    const objectRef: DatabaseObjectRef = {
      schema: tab.table.schema,
      name: tab.table.name,
      kind: tableObjectKind,
      identity: null,
    };
    let active = true;
    setTableDetailsLoadingKey(tableObjectKey);
    setTableDetailsErrorState((prev) => (prev?.key === tableObjectKey ? null : prev));
    assertOrdinarySession(session.id);
    void client
      .call<ObjectDetails>("object_details", { sessionId: session.id, object: objectRef })
      .then((data) => {
        if (active) setTableDetailsState({ key: tableObjectKey, data });
      })
      .catch((error) => {
        if (active) setTableDetailsErrorState({ key: tableObjectKey, message: messageOf(error) });
      })
      .finally(() => {
        if (active) setTableDetailsLoadingKey((prev) => (prev === tableObjectKey ? null : prev));
      });
    return () => {
      active = false;
    };
  }, [
    objectExplorerSupported,
    session?.id,
    tab.kind,
    tab.table?.name,
    tab.table?.schema,
    tab.view,
    tableDetailsState?.key,
    tableObjectKey,
    tableObjectKind,
  ]);
  function resizeEditor(event: React.PointerEvent) {
    const start = event.clientY,
      before = editorHeight.current;
    const move = (event: PointerEvent) => {
      const height = Math.min(
        window.innerHeight - 350,
        Math.max(140, before + event.clientY - start),
      );
      editorHeight.current = height;
      setEditorSize(height);
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      document.body.style.cursor = "";
    };
    document.body.style.cursor = "row-resize";
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end, { once: true });
  }
  if (consoleMode && tab.kind !== "query" && tab.kind !== "result" && tab.kind !== "benchmark")
    return (
      <p role="status">
        Metadata is disabled on this reviewed SQL console session. Open a separate profile with
        reviewed console off; temporary objects and transactions are not shared.
      </p>
    );
  const renderGridModeSwitch = () => (
    <div className="grid-mode-switch" role="group" aria-label="Data display mode">
      {(
        [
          ["grid", "Grid"],
          ["transpose", "Transpose"],
          ["record", "Record"],
        ] as const
      ).map(([mode, label]) => (
        <button
          key={mode}
          type="button"
          className={`button ghost small ${gridMode === mode ? "active" : ""}`.trim()}
          aria-pressed={gridMode === mode}
          onClick={() => state.updateTab(tab.id, { gridMode: mode })}
        >
          {label}
        </button>
      ))}
    </div>
  );
  return (
    <>
      <div className="query-toolbar">
        <div className="query-context">
          {tab.kind === "query" ? (
            <Code2 size={15} />
          ) : tab.kind === "benchmark" ? (
            <Gauge size={15} />
          ) : tab.kind === "object" && tab.object ? (
            <ObjectKindGlyph kind={tab.object.kind} size={15} />
          ) : (
            <Table2 size={15} />
          )}
          <span>
            {tab.kind === "benchmark"
              ? "1,000,000 virtual rows"
              : tab.kind === "result"
                ? tab.title
                : tab.kind === "table"
                  ? `${tab.table?.schema}.${tab.table?.name}`
                  : tab.kind === "object" && tab.object
                    ? `${tab.object.schema}.${objectTabTitle(tab.object)}`
                    : profile?.engine === "redis"
                      ? "Redis console"
                      : "SQL editor"}
          </span>
          {tab.kind === "query" && (
            <select
              aria-label="Query connection"
              value={tab.connectionId ?? ""}
              disabled={running}
              onChange={(e) => changeConnection(e.target.value)}
            >
              <option value="">Choose connection</option>
              {state.bootstrap?.profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name}
                </option>
              ))}
            </select>
          )}
          {profile && (
            <span className="read-policy">
              <ShieldCheck size={12} />
              {profile.safeMode === "readOnly"
                ? "Read only"
                : profile.safeMode === "silent"
                  ? "Local session"
                  : "Safe Mode"}
            </span>
          )}
          {tab.kind === "query" && profile && ["postgres", "mysql"].includes(profile.engine) && (
            <>
              <span className="query-session" aria-live="polite">
                {tab.querySession
                  ? tab.querySessionClosed
                    ? "Isolated session · disconnected"
                    : sessionLabel && sessionLabel !== "Isolated session"
                      ? `Isolated session · ${sessionLabel}`
                      : "Isolated session"
                  : session
                    ? "Shared session"
                    : "Disconnected"}
              </span>
              {(pool.length > 0 || Boolean(tab.querySession)) && (
                <select
                  aria-label="Console session"
                  disabled={running}
                  value={
                    tab.querySession
                      ? tab.querySessionClosed && !activePoolEntry
                        ? "__disconnected__"
                        : tab.querySession.id
                      : "shared"
                  }
                  onChange={(event) => {
                    const value = event.target.value;
                    if (value === "__disconnected__") return;
                    setRenamingSessionId(null);
                    state.attachQuerySession(tab.id, value === "shared" ? null : value);
                  }}
                >
                  <option value="shared">Shared session</option>
                  {pool.map((entry) => (
                    <option key={entry.info.id} value={entry.info.id}>
                      {entry.label}
                    </option>
                  ))}
                  {tab.querySession && tab.querySessionClosed && !activePoolEntry && (
                    <option value="__disconnected__" disabled>
                      Isolated session (disconnected)
                    </option>
                  )}
                </select>
              )}
              {tab.querySession &&
                !tab.querySessionClosed &&
                activePoolEntry &&
                (renamingSessionId === tab.querySession.id ? (
                  <input
                    className="session-rename-input"
                    aria-label="Console session name"
                    value={renameDraft}
                    autoFocus
                    maxLength={40}
                    onChange={(event) => setRenameDraft(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        state.renameQuerySession(tab.querySession!.id, renameDraft);
                        setRenamingSessionId(null);
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        setRenamingSessionId(null);
                      }
                    }}
                    onBlur={() => {
                      if (renameDraft.trim())
                        state.renameQuerySession(tab.querySession!.id, renameDraft);
                      setRenamingSessionId(null);
                    }}
                  />
                ) : (
                  <>
                    <button
                      className="icon-button"
                      aria-label="Rename console session"
                      title="Rename console session"
                      disabled={running}
                      onClick={() => {
                        setRenamingSessionId(tab.querySession!.id);
                        setRenameDraft(activePoolEntry.label);
                      }}
                    >
                      <Pencil size={12} />
                    </button>
                    <button
                      className="icon-button"
                      aria-label="Disconnect console session"
                      title="Disconnect console session"
                      disabled={running}
                      onClick={() => {
                        const targetSessionId = tab.querySession!.id;
                        const runDisconnect = () => {
                          void state
                            .disconnectQuerySession(targetSessionId)
                            .catch((error) => state.notify(messageOf(error)));
                        };
                        if (sessionHasOpenTransaction(state.sessionStates, targetSessionId))
                          onConfirm(
                            "Disconnect console session?",
                            "Disconnecting closes this session. Verify any open or unknown transaction on the database; remote rollback may not be immediate.",
                            runDisconnect,
                          );
                        else runDisconnect();
                      }}
                    >
                      <Unplug size={12} />
                    </button>
                  </>
                ))}
              {(!tab.querySession || tab.querySessionClosed) && (
                <button
                  className="button ghost small"
                  disabled={running || !tab.connectionId || totalSessions >= 4}
                  aria-label={
                    tab.querySession ? "Reconnect isolated session" : "New isolated session"
                  }
                  title="Open a separate SQL session for this tab. Maximum four sessions per connection."
                  onClick={onNewSession}
                >
                  {tab.querySession ? "Reconnect" : "New session"}
                </button>
              )}
            </>
          )}
          {tab.kind === "query" &&
            session &&
            !consoleMode &&
            (session.capabilities.transactions || session.engine === "snowflake") && (
              <>
                <select
                  aria-label="Transaction mode"
                  value={txMode}
                  disabled={
                    running || txBusy || !session.capabilities.transactions || txMode === "unknown"
                  }
                  onChange={(event) => {
                    const nextMode = event.target.value as TransactionMode;
                    if (nextMode === "unknown") return;
                    setTxBusy(true);
                    void state
                      .setTransactionMode(session.id, nextMode)
                      .catch((error) => state.notify(messageOf(error)))
                      .finally(() => setTxBusy(false));
                  }}
                >
                  {txMode === "unknown" && (
                    <option value="unknown" disabled>
                      Mode unknown
                    </option>
                  )}
                  <option value="autoCommit">Auto-commit</option>
                  <option value="manual">Manual commit</option>
                </select>
                {txState !== "idle" && (
                  <span className={`tx-badge ${txState}`} aria-live="polite">
                    {txState === "unknown"
                      ? "Transaction state unknown"
                      : txState === "failed"
                        ? "Failed transaction"
                        : runtimeState?.transactionObservation?.status === "observed"
                          ? "Open transaction · usability unknown"
                          : "In transaction"}
                  </span>
                )}
                {session.capabilities.explicitTransactionControls && (
                  <>
                    {runtimeState?.transactionControlAuthority?.status === "rollbackOnly" && (
                      <span role="status">Only rollback recovery is available</span>
                    )}
                    {(["begin", "commit", "rollback"] as const).map((action) => {
                      const authority = runtimeState?.transactionControlAuthority;
                      const allowed =
                        authority?.status === "rollbackOnly"
                          ? action === "rollback"
                          : authority?.allowedActions.includes(action) === true;
                      const label =
                        action === "begin" ? "Begin" : action === "commit" ? "Commit" : "Rollback";
                      return (
                        <button
                          key={action}
                          className="button ghost small"
                          aria-label={`${label} transaction`}
                          disabled={
                            running ||
                            sessionQueryPending ||
                            txBusy ||
                            !allowed ||
                            (action !== "rollback" && profile?.safeMode === "readOnly")
                          }
                          onClick={() => {
                            setTxBusy(true);
                            void onTransactionControl(action)
                              .catch((error) => state.notify(messageOf(error)))
                              .finally(() => setTxBusy(false));
                          }}
                        >
                          {label}
                        </button>
                      );
                    })}
                  </>
                )}
                {session.capabilities.transactions &&
                  (txMode === "manual" || txState !== "idle") && (
                    <>
                      <button
                        className="button ghost small"
                        aria-label="Commit transaction"
                        disabled={
                          running ||
                          txBusy ||
                          !session.capabilities.transactions ||
                          txState !== "inTransaction" ||
                          profile?.safeMode === "readOnly"
                        }
                        onClick={() => {
                          setTxBusy(true);
                          void state
                            .commitTransaction(session.id)
                            .catch((error) => state.notify(messageOf(error)))
                            .finally(() => setTxBusy(false));
                        }}
                      >
                        <Check size={12} />
                        Commit
                      </button>
                      <button
                        className="button ghost small"
                        aria-label="Rollback transaction"
                        disabled={
                          running ||
                          txBusy ||
                          !session.capabilities.transactions ||
                          txState === "idle" ||
                          txState === "unknown"
                        }
                        onClick={() => {
                          setTxBusy(true);
                          void state
                            .rollbackTransaction(session.id)
                            .catch((error) => state.notify(messageOf(error)))
                            .finally(() => setTxBusy(false));
                        }}
                      >
                        <RotateCcw size={12} />
                        Rollback
                      </button>
                    </>
                  )}
              </>
            )}
        </div>
        <div className="query-actions">
          {bookmarkCandidate && (
            <button
              className={`icon-button bookmark-button ${activeBookmark ? "active" : ""}`}
              aria-label={activeBookmark ? "Remove bookmark" : "Bookmark item"}
              aria-pressed={Boolean(activeBookmark)}
              title={activeBookmark ? "Remove bookmark · ⌘D" : "Bookmark · ⌘D"}
              onClick={onToggleBookmark}
            >
              <Star size={14} fill={activeBookmark ? "currentColor" : "none"} />
            </button>
          )}
          {tab.kind === "query" && (
            <>
              {!consoleMode && session?.capabilities.relationBrowsing && (
                <button
                  className="button ghost small"
                  disabled={running || !!state.sessionMetadataRefreshRequired[session.id]}
                  title={
                    state.sessionMetadataRefreshRequired[session.id]
                      ? "Refresh completion context before browsing relations"
                      : undefined
                  }
                  onClick={() => {
                    assertOrdinarySession(session.id);
                    setRelationBrowserRevision(state.sessionContextRevision[session.id] ?? 0);
                    setRelationBrowserSession(session.id);
                  }}
                >
                  <Table2 size={13} /> Browse relations
                </button>
              )}
              <button
                className="button ghost small"
                aria-label="Open SQL file"
                title="Open SQL file · ⌘O"
                disabled={running}
                onClick={onOpenSqlFile}
              >
                <FolderOpen size={13} />
                Open
              </button>
              <button
                className="button ghost small"
                aria-label="Save SQL file"
                title="Save SQL file · ⌘S"
                disabled={running}
                onClick={() => onSaveSqlFile(false)}
              >
                <Save size={13} />
                {fileDirty ? "Save ●" : "Save"}
              </button>
              <button
                className="button ghost small"
                aria-label="Save SQL file as"
                title="Save SQL file as · ⇧⌘S"
                disabled={running}
                onClick={() => onSaveSqlFile(true)}
              >
                Save as
              </button>
              <button
                className="button ghost small"
                aria-label="Save query to library"
                title="Save query to library"
                disabled={running || !tab.sql.trim()}
                onClick={onSaveToLibrary}
              >
                <BookmarkPlus size={13} />
                {tab.savedQueryId ? "Update library" : "Save to library"}
              </button>
              <button
                className="button ghost small"
                title="Format SQL · ⇧⌘F"
                disabled={profile?.engine === "redis"}
                onClick={() => editorRef.current?.format()}
              >
                <Code2 size={13} />
                Format
              </button>
              {dmlPending && session ? (
                <SnowflakeDmlPendingControl
                  onDisconnect={() =>
                    void state
                      .disconnectQuerySession(session.id)
                      .catch((error) => state.notify(messageOf(error)))
                  }
                />
              ) : running && !controlPending ? (
                <button
                  className="button cancel-query"
                  onClick={() => {
                    if (heldReceipt) void state.cancelSnowflakeConsole(heldReceipt.clientRequestId);
                    else if (tab.runId)
                      void client
                        .call("cancel_run", { runId: tab.runId })
                        .catch((error) => state.notify(messageOf(error)));
                  }}
                >
                  <Square size={11} />
                  Cancel<kbd>⌘ .</kbd>
                </button>
              ) : (
                <button
                  className="button primary run-button"
                  onClick={() => onRun()}
                  disabled={!session || controlPending || rollbackOnly}
                >
                  <Play size={12} fill="currentColor" />
                  {consoleMode ? "Review and run" : "Run"}
                  <kbd>⌘ ↵</kbd>
                </button>
              )}
              {!consoleMode &&
                profile &&
                ["postgres", "mysql", "snowflake"].includes(profile.engine) && (
                  <button
                    className="button ghost small"
                    disabled={running || !session || rollbackOnly}
                    title="Estimate the selected query or current statement without ANALYZE"
                    onClick={onExplain}
                  >
                    <Gauge size={13} />
                    Explain
                  </button>
                )}
              {!consoleMode &&
                Boolean(
                  session
                    ? (session.capabilities.explainAnalyze ??
                        (profile?.engine === "postgres" ||
                          session.capabilities.actualPlan === "mariaDbJson" ||
                          session.capabilities.actualPlan === "mysqlTree"))
                    : profile?.engine === "postgres",
                ) && (
                  <button
                    className="button ghost small"
                    disabled={running || !session || rollbackOnly}
                    title="Executes the selected query or current statement after review to collect actual measurements"
                    onClick={onAnalyze}
                  >
                    <Gauge size={13} />
                    Analyze
                  </button>
                )}
              {consoleMode && (
                <button
                  className="button ghost small"
                  disabled={running || !session}
                  onClick={onRunConsoleSelected}
                >
                  Review selected server unit
                </button>
              )}
              <button
                className="icon-button run-all"
                title="Run all statements · ⇧⌘↵"
                aria-label="Run all statements"
                disabled={running || !session || rollbackOnly}
                onClick={() => (consoleMode ? onRunConsoleScript?.() : onRun(tab.sql, 0))}
              >
                <ChevronDown size={13} />
              </button>
            </>
          )}
          {tab.kind === "table" && (
            <>
              <button
                className="button ghost small"
                disabled={tab.loadingPage || running}
                onClick={() => {
                  const reload = () => {
                    dirtyStore.clearTab(tab.id, resultIds);
                    if (hasRawClauses(tab.query)) onQueryChange(tab.query, { force: true });
                    else void state.refreshTable(tab.id);
                  };
                  if (pendingCount)
                    onConfirm(
                      "Refresh and discard edits?",
                      "This reloads current database values and discards pending cell edits.",
                      reload,
                    );
                  else reload();
                }}
              >
                <RefreshCw size={13} className={tab.loadingPage ? "spin" : ""} />
                Refresh
              </button>
              {pendingCount > 0 && (
                <>
                  {canRevertSelected && (
                    <button
                      type="button"
                      className="button ghost small"
                      disabled={running}
                      onClick={handleRevertSelection}
                    >
                      <RotateCcw size={12} />
                      Revert selected
                    </button>
                  )}
                  <button
                    className="button ghost small"
                    disabled={running}
                    onClick={() => dirtyStore.clearTab(tab.id, resultIds)}
                  >
                    Discard
                  </button>
                  <button
                    className="button primary small"
                    disabled={
                      running ||
                      !session ||
                      session.id !== tab.dataSessionId ||
                      tab.status === "outcomeUnknown"
                    }
                    onClick={onReview}
                  >
                    <Check size={13} />
                    Review {pendingCount} edit{pendingCount === 1 ? "" : "s"}
                  </button>
                </>
              )}
            </>
          )}
          {tab.kind === "result" && tab.originSql != null && (
            <button
              type="button"
              className="button ghost small"
              onClick={() => state.openOriginatingSql(tab.id)}
            >
              <Code2 size={13} />
              Open originating SQL
            </button>
          )}
          {tab.kind === "object" && (
            <>
              {!consoleMode &&
                session?.capabilities.relationBrowsing &&
                tab.object &&
                (tab.object.kind === "table" ||
                  tab.object.kind === "view" ||
                  tab.object.kind === "materializedView") && (
                  <button
                    className="button ghost small"
                    disabled={!!state.sessionMetadataRefreshRequired[session.id]}
                    onClick={() => {
                      assertOrdinarySession(session.id);
                      setRelationBrowserRevision(state.sessionContextRevision[session.id] ?? 0);
                      setRelationBrowserSession(session.id);
                    }}
                  >
                    <Table2 size={13} /> Browse relations
                  </button>
                )}
              <button
                className="button ghost small"
                disabled={!session}
                onClick={() => state.updateTab(tab.id, { revision: tab.revision + 1 })}
              >
                <RefreshCw size={13} />
                Refresh
              </button>
            </>
          )}
        </div>
      </div>
      {consoleMode && tab.kind === "query" && (
        <div className="query-notice" role="note">
          Reviewed Snowflake console · One retained physical session across runs. Write explicit
          BEGIN, COMMIT or ROLLBACK in reviewed SQL. Automatic metadata and context probes are
          disabled. For metadata, explicitly open a separate profile with reviewed console off; it
          cannot share temporary objects or transactions. Each review shows the execution and
          result-delivery limit per unit; profile changes apply to fresh sessions. Expiration closes
          the console connection. Safe outcome recording and settlement may take longer; closing
          does not prove server cancellation or transaction effects. SQL is never replayed
          automatically.
        </div>
      )}
      {!consoleMode &&
        relationBrowserSession &&
        session?.id === relationBrowserSession &&
        relationBrowserRevision === (state.sessionContextRevision[session.id] ?? 0) &&
        !state.sessionMetadataRefreshRequired[session.id] &&
        !running && (
          <RelationBrowser
            key={`${tab.id}:${session.id}:${state.sessionContextRevision[session.id] ?? 0}:${tab.revision}`}
            sessionId={session.id}
            ownerTabId={tab.id}
            onClose={() => setRelationBrowserSession(null)}
          />
        )}
      {tab.kind === "query" && profile?.engine === "redis" && (
        <RedisTools
          sessionId={session?.id ?? null}
          disabled={running}
          readOnly={profile.safeMode === "readOnly"}
          onCommand={(command) => {
            state.updateTab(tab.id, { sql: command });
            onRun(command, 0);
          }}
        />
      )}
      {tab.kind === "query" && (
        <>
          <div className="editor-panel" style={{ height: editorSize }}>
            <Suspense fallback={<div className="editor-loading">Opening SQL editor…</div>}>
              <SqlEditor
                ref={editorRef}
                key={tab.id}
                value={tab.sql}
                onChange={(sql) => state.updateTab(tab.id, { sql })}
                onRun={onRun}
                onOpenParameters={onOpenParameters}
                onOpenObject={
                  !consoleMode && session && tab.connectionId
                    ? (object, highlightedColumn, catalogRevision) =>
                        state.openSessionObject(tab.id, object, highlightedColumn, catalogRevision)
                    : undefined
                }
                error={tab.error}
                errorOrigin={tab.errorOrigin}
                engine={profile?.engine ?? "postgres"}
                sessionId={session?.id ?? null}
                tables={profile ? (state.catalog[profile.id] ?? EMPTY_TABLES) : EMPTY_TABLES}
                objectExplorerSupported={objectExplorerSupported}
                database={profile?.engine === "snowflake" ? profile.database : undefined}
                defaultSchema={
                  profile?.engine === "snowflake" ? profile.snowflake?.schema : undefined
                }
                contextEpoch={JSON.stringify([
                  session ? (state.sessionContextRevision[session.id] ?? 0) : 0,
                  state.tabs
                    .filter(
                      (t) => session && querySessionForTab(t, state.sessions)?.id === session.id,
                    )
                    .map((t) => [t.id, t.status, t.revision]),
                ])}
                theme={state.theme}
              />
            </Suspense>
          </div>
          <div
            className="editor-divider"
            role="separator"
            aria-label="Resize SQL editor"
            aria-orientation="horizontal"
            tabIndex={0}
            aria-valuemin={140}
            aria-valuemax={Math.max(140, window.innerHeight - 350)}
            aria-valuenow={editorSize}
            onKeyDown={(event) => {
              const max = Math.max(140, window.innerHeight - 350);
              const next =
                event.key === "ArrowUp"
                  ? editorSize - 20
                  : event.key === "ArrowDown"
                    ? editorSize + 20
                    : event.key === "Home"
                      ? 140
                      : event.key === "End"
                        ? max
                        : null;
              if (next === null) return;
              event.preventDefault();
              editorHeight.current = Math.min(max, Math.max(140, next));
              setEditorSize(editorHeight.current);
            }}
            onPointerDown={resizeEditor}
          >
            <span />
          </div>
        </>
      )}
      {tab.kind === "query" ? (
        <div className="results-header">
          <div
            className="result-tabs"
            role="tablist"
            aria-label="Query results"
            onKeyDown={tabNavigation}
          >
            {tab.results.length ? (
              tab.results.map((result, i) => (
                <button
                  role="tab"
                  id={`result-tab-${result.id}`}
                  aria-controls={`result-panel-${tab.id}`}
                  aria-selected={result.id === (tab.activeResultId ?? tab.results[0]?.id)}
                  tabIndex={result.id === (tab.activeResultId ?? tab.results[0]?.id) ? 0 : -1}
                  className={result.id === tab.activeResultId ? "active" : ""}
                  key={result.id}
                  onClick={() => state.updateTab(tab.id, { activeResultId: result.id })}
                >
                  <Table2 size={12} />
                  Query {result.statementIndex + 1}
                  {tab.results.filter((r) => r.statementIndex === result.statementIndex).length > 1
                    ? ` · ${i + 1}`
                    : ""}
                  <span>{result.retainedRows.toLocaleString()}</span>
                </button>
              ))
            ) : (
              <span className="result-placeholder">Results</span>
            )}
          </div>
          <div className="table-view-actions">
            {result && result.editability && (
              <span
                className={`result-provenance-badge ${
                  result.editability.status === "editable" && profile?.safeMode !== "readOnly"
                    ? "editable"
                    : "read-only"
                }`}
                data-testid="result-provenance-badge"
              >
                {result.editability.status === "editable" && profile?.safeMode !== "readOnly"
                  ? `Editable result · ${result.editability.table.schema}.${result.editability.table.name} (PK: ${result.editability.primaryKey.join(", ")})`
                  : `Read-only result · ${
                      result.editability.status === "editable"
                        ? "Connection is in read-only mode"
                        : result.editability.reason
                    }`}
              </span>
            )}
            {queryResultEditable && (
              <>
                {result.columns.some((col) => col.insertable) && (
                  <>
                    <button
                      type="button"
                      className="button ghost small"
                      disabled={mutatingDisabled}
                      onClick={stageInsertRow}
                    >
                      <Plus size={12} />
                      Insert row
                    </button>
                    <button
                      type="button"
                      className="button ghost small"
                      disabled={mutatingDisabled || cloneCandidateVisualRow === null}
                      onClick={() => void cloneSelectedRow()}
                    >
                      <Copy size={12} />
                      Clone row
                    </button>
                  </>
                )}
                <button
                  type="button"
                  className="button ghost small"
                  disabled={
                    mutatingDisabled ||
                    (!onlyDraftsSelected && !allExistingDeleted && deleteTargetCount === 0)
                  }
                  onClick={handleDeleteSelection}
                >
                  {allExistingDeleted ? <RotateCcw size={12} /> : <Trash2 size={12} />}
                  {onlyDraftsSelected
                    ? selectedDraftIds.length === 1
                      ? "Remove insert"
                      : `Remove ${selectedDraftIds.length} inserts`
                    : allExistingDeleted
                      ? selectedExisting.length === 1
                        ? "Undo delete"
                        : `Undo ${selectedExisting.length} deletes`
                      : deleteTargetCount > 1
                        ? `Delete ${deleteTargetCount} rows`
                        : "Delete row"}
                </button>
              </>
            )}
            {tab.kind === "query" && pendingCount > 0 && (
              <>
                {canRevertSelected && (
                  <button
                    type="button"
                    className="button ghost small"
                    disabled={running}
                    onClick={handleRevertSelection}
                  >
                    <RotateCcw size={12} />
                    Revert selected
                  </button>
                )}
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running}
                  onClick={() => dirtyStore.clearTab(insertScopeId, resultIds)}
                >
                  Discard
                </button>
                <button
                  type="button"
                  className="button primary small"
                  disabled={
                    running ||
                    !session ||
                    session.id !== tab.dataSessionId ||
                    tab.status === "outcomeUnknown"
                  }
                  onClick={onReview}
                >
                  <Check size={13} />
                  Review {pendingCount} edit{pendingCount === 1 ? "" : "s"}
                </button>
              </>
            )}
            {result && result.columns.length > 0 && (
              <>
                {renderGridModeSwitch()}
                {isQueryPlanResult(result, tab.planSource) && (
                  <button
                    className="button ghost small"
                    aria-pressed={planResultId === result.id}
                    onClick={() => setPlanResultId(planResultId === result.id ? null : result.id)}
                  >
                    {planResultId === result.id ? "Data grid" : "View plan"}
                  </button>
                )}
                <CopyAsMenu gridRef={gridRef} disabled={running || gridMode !== "grid"} />
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running}
                  onClick={onOpenCompare}
                >
                  <ArrowLeftRight size={12} />
                  Compare…
                </button>
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running}
                  onClick={onOpenExport}
                >
                  <Download size={12} />
                  Export…
                </button>
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running}
                  onClick={onOpenColumns}
                >
                  <Columns3 size={12} />
                  Columns
                  {resolvedLayout.hiddenSet.size > 0
                    ? ` (${resolvedLayout.visibleColumns.length}/${resolvedLayout.orderedColumns.length})`
                    : ""}
                </button>
                <button
                  type="button"
                  className="button ghost small"
                  aria-label="Open result in tab"
                  disabled={running || isConsoleOwnedResult(result)}
                  onClick={() => state.detachResultToTab(tab.id, result.id)}
                >
                  <ExternalLink size={12} />
                  Open in tab
                </button>
              </>
            )}
            <span className="result-caption">
              {running ? (
                <>
                  <Loader2 size={12} className="spin" />
                  {(runningTime / 1000).toFixed(1)} s
                </>
              ) : (
                <>
                  <Clock3 size={12} />
                  {tab.runMetrics
                    ? `Backend total ${tab.runMetrics.totalMs} ms`
                    : tab.clientElapsedMs !== undefined
                      ? `Client ${Math.round(tab.clientElapsedMs)} ms`
                      : "Ready when you are"}
                </>
              )}
            </span>
          </div>
        </div>
      ) : tab.kind === "result" ? (
        <div className="results-header table-views">
          <div className="result-tabs">
            <button className="active" aria-pressed="true" type="button">
              <Table2 size={12} />
              Result snapshot
              {result ? <span>{result.retainedRows.toLocaleString()}</span> : null}
            </button>
          </div>
          <div className="table-view-actions">
            {result && result.columns.length > 0 && (
              <>
                {renderGridModeSwitch()}
                <CopyAsMenu gridRef={gridRef} disabled={gridMode !== "grid"} />
                <button type="button" className="button ghost small" onClick={onOpenCompare}>
                  <ArrowLeftRight size={12} />
                  Compare…
                </button>
                <button type="button" className="button ghost small" onClick={onOpenExport}>
                  <Download size={12} />
                  Export…
                </button>
                <button type="button" className="button ghost small" onClick={onOpenColumns}>
                  <Columns3 size={12} />
                  Columns
                  {resolvedLayout.hiddenSet.size > 0
                    ? ` (${resolvedLayout.visibleColumns.length}/${resolvedLayout.orderedColumns.length})`
                    : ""}
                </button>
                {resolvedLayout.customized && (
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() => state.setResultLayout(tab.id, result.id, null)}
                  >
                    <RotateCcw size={12} />
                    Reset column layout
                  </button>
                )}
              </>
            )}
            <span className="result-caption">Detached result snapshot</span>
          </div>
        </div>
      ) : tab.kind === "table" ? (
        <div className="results-header table-views">
          <div className="result-tabs">
            <button
              className={tab.view === "data" ? "active" : ""}
              aria-pressed={tab.view === "data"}
              onClick={() => state.updateTab(tab.id, { view: "data" })}
            >
              <Table2 size={12} />
              Data
            </button>
            <button
              className={tab.view === "structure" ? "active" : ""}
              aria-pressed={tab.view === "structure"}
              onClick={() => state.updateTab(tab.id, { view: "structure" })}
            >
              <Code2 size={12} />
              Structure
            </button>
            {objectExplorerSupported && (
              <button
                className={tab.view === "ddl" ? "active" : ""}
                aria-pressed={tab.view === "ddl"}
                onClick={() => state.updateTab(tab.id, { view: "ddl" })}
              >
                <FileCode2 size={12} />
                DDL
              </button>
            )}
          </div>
          <div className="table-view-actions">
            {tableEditable && (
              <>
                <button
                  type="button"
                  className="button ghost small"
                  disabled={mutatingDisabled}
                  onClick={stageInsertRow}
                >
                  <Plus size={12} />
                  Insert row
                </button>
                {tab.schema?.columns.some((col) => col.insertable) && (
                  <button
                    type="button"
                    className="button ghost small"
                    disabled={mutatingDisabled || cloneCandidateVisualRow === null}
                    onClick={() => void cloneSelectedRow()}
                  >
                    <Copy size={12} />
                    Clone row
                  </button>
                )}
                <button
                  type="button"
                  className="button ghost small"
                  disabled={
                    mutatingDisabled ||
                    (!onlyDraftsSelected && !allExistingDeleted && deleteTargetCount === 0)
                  }
                  onClick={handleDeleteSelection}
                >
                  {allExistingDeleted ? <RotateCcw size={12} /> : <Trash2 size={12} />}
                  {onlyDraftsSelected
                    ? selectedDraftIds.length === 1
                      ? "Remove insert"
                      : `Remove ${selectedDraftIds.length} inserts`
                    : allExistingDeleted
                      ? selectedExisting.length === 1
                        ? "Undo delete"
                        : `Undo ${selectedExisting.length} deletes`
                      : deleteTargetCount > 1
                        ? `Delete ${deleteTargetCount} rows`
                        : "Delete row"}
                </button>
              </>
            )}
            {tab.view === "data" && tableColumns.length > 0 && (
              <>
                {renderGridModeSwitch()}
                <CopyAsMenu
                  gridRef={gridRef}
                  disabled={running || tab.loadingPage || gridMode !== "grid"}
                />
                {tableImportable ? (
                  <button
                    type="button"
                    className="button ghost small"
                    disabled={mutatingDisabled}
                    onClick={onOpenImport}
                  >
                    <Upload size={12} />
                    Import CSV…
                  </button>
                ) : null}
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running || tab.loadingPage}
                  onClick={onOpenCompare}
                >
                  <ArrowLeftRight size={12} />
                  Compare…
                </button>
                <button
                  type="button"
                  className="button ghost small"
                  disabled={running || tab.loadingPage}
                  onClick={onOpenExport}
                >
                  <Download size={12} />
                  Export…
                </button>
                <button type="button" className="button ghost small" onClick={onOpenColumns}>
                  <Columns3 size={12} />
                  Columns
                  {resolvedLayout.hiddenSet.size > 0
                    ? ` (${resolvedLayout.visibleColumns.length}/${resolvedLayout.orderedColumns.length})`
                    : ""}
                </button>
                {resolvedLayout.customized && tab.connectionId && tab.table && (
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() => state.resetTableLayout(tab.connectionId!, tab.table!)}
                  >
                    <RotateCcw size={12} />
                    Reset column layout
                  </button>
                )}
              </>
            )}
            <span className="result-caption">
              {session && tab.dataSessionId !== session.id
                ? "Refresh after reconnecting"
                : profile?.engine === "snowflake"
                  ? "Read-only preview · up to 500 rows / 2 MiB"
                  : tab.schema?.editable
                    ? "Double-click a cell to edit"
                    : "Read-only result"}
            </span>
          </div>
        </div>
      ) : tab.kind === "object" ? (
        <div className="results-header table-views">
          <div className="result-tabs">
            <button
              className={tab.view !== "ddl" ? "active" : ""}
              aria-pressed={tab.view !== "ddl"}
              onClick={() => state.updateTab(tab.id, { view: "structure" })}
            >
              <Code2 size={12} />
              Structure
            </button>
            <button
              className={tab.view === "ddl" ? "active" : ""}
              aria-pressed={tab.view === "ddl"}
              onClick={() => state.updateTab(tab.id, { view: "ddl" })}
            >
              <FileCode2 size={12} />
              DDL
            </button>
          </div>
          <div className="table-view-actions">
            <span className="result-caption">Read-only metadata</span>
          </div>
        </div>
      ) : (
        <div className="benchmark-note">
          <Gauge size={13} />
          <span>Generated on demand · no database connection · constant row storage</span>
          <div style={{ marginLeft: "auto" }}>{renderGridModeSwitch()}</div>
        </div>
      )}
      {tab.kind === "query" && tab.clientStartedAt !== undefined && (
        <ExecutionTiming metrics={tab.runMetrics} clientElapsedMs={tab.clientElapsedMs} />
      )}
      {tab.kind === "query" &&
        tab.clientStartedAt !== undefined &&
        !["idle", "queued", "running"].includes(tab.status) &&
        !tab.snowflakeDmlResult &&
        !tab.snowflakeDdlResult && (
          <ExecutionSummary summary={tab.executionSummary} status={tab.status} />
        )}
      {filtering && tab.view === "data" && (
        <TableFilterBar
          query={tab.query}
          columns={tab.schema?.columns ?? []}
          disabledReason={queryLock}
          searchNote={
            profile?.engine === "postgres" ? "case-insensitive text" : "follows column collation"
          }
          onChange={applyQuery}
        />
      )}
      {tab.kind === "query" && tab.snowflakeDmlResult && (
        <SnowflakeDmlNotice
          result={tab.snowflakeDmlResult}
          hasPriorResults={tab.results.length > 0}
        />
      )}
      {tab.kind === "query" && tab.snowflakeDdlResult && (
        <SnowflakeDdlNotice
          result={tab.snowflakeDdlResult}
          hasPriorResults={tab.results.length > 0}
        />
      )}
      {tab.error &&
        (() => {
          const resolvedError =
            tab.kind === "query" && tab.error.location && profile?.engine !== "redis"
              ? resolveExecutionError(tab.sql, tab.error, tab.errorOrigin)
              : null;
          const loc = tab.error.location;
          return (
            <div
              className={`query-error ${tab.error.code === "outcomeUnknown" ? "uncertain" : ""}`}
              role="alert"
            >
              <span className="error-symbol">!</span>
              <div className="query-error-body">
                <div className="query-error-main">
                  <span>{tab.error.message}</span>
                  {resolvedError && (
                    <span className="query-error-badge">
                      Line {resolvedError.line}, Col {resolvedError.column}
                    </span>
                  )}
                  {loc?.statementIndex !== undefined && (
                    <span className="query-error-badge">Statement {loc.statementIndex + 1}</span>
                  )}
                  {loc?.sqlState && (
                    <span className="query-error-badge">SQLSTATE {loc.sqlState}</span>
                  )}
                </div>
                {loc?.detail && !tab.error.message.includes(loc.detail) && (
                  <div className="query-error-detail">Detail: {loc.detail}</div>
                )}
                {loc?.hint && !tab.error.message.includes(loc.hint) && (
                  <div className="query-error-detail">Hint: {loc.hint}</div>
                )}
              </div>
              {resolvedError && (
                <button
                  type="button"
                  className="button ghost small query-error-jump"
                  onClick={() =>
                    editorRef.current?.revealRange(resolvedError.from, resolvedError.to)
                  }
                >
                  Jump to error
                </button>
              )}
              <button
                className="icon-button"
                aria-label="Dismiss query error"
                onClick={() => state.updateTab(tab.id, { error: null })}
              >
                <X size={12} />
              </button>
            </div>
          );
        })()}
      <div
        className="result-panel"
        role={tab.kind === "query" && result ? "tabpanel" : undefined}
        id={`result-panel-${tab.id}`}
        aria-labelledby={tab.kind === "query" && result ? `result-tab-${result.id}` : undefined}
      >
        {tab.kind === "object" ? (
          tab.metadataOrigin ? (
            <SessionStructureView key={tab.id} tab={tab} />
          ) : (
            <ObjectInspectorView key={tab.id} tab={tab} profile={profile} session={session} />
          )
        ) : tab.kind === "table" && tab.view === "structure" ? (
          <StructureView
            schema={tab.schema}
            details={activeTableDetails}
            detailsError={activeTableDetailsError}
            connectionId={tab.connectionId}
            highlightedColumn={tab.highlightedColumn}
            onRefresh={() => void state.refreshTable(tab.id)}
            onDropped={() => state.closeTab(tab.id)}
          />
        ) : tab.kind === "table" && tab.view === "ddl" ? (
          <div className="object-inspector-scroll">
            <DdlPanel
              connectionId={tab.connectionId}
              ddlStatus={activeTableDetails?.ddl ?? null}
              loading={activeTableDetailsLoading}
              error={activeTableDetailsError}
            />
          </div>
        ) : tab.kind === "query" &&
          result &&
          isQueryPlanResult(result, tab.planSource) &&
          planResultId === result.id ? (
          <QueryPlanView
            key={result.id}
            result={result}
            ownerTabId={tab.id}
            source={tab.planSource}
            canCapture={tab.status === "completed" && session?.id === tab.planSource?.sessionId}
          />
        ) : (tab.kind === "query" || tab.kind === "result") && !result ? (
          <div className="results-empty">
            <Table2 size={27} />
            <h3>{running ? "Your query is running" : "A fresh perspective awaits"}</h3>
            <p>
              {running
                ? "Results will appear as they arrive."
                : "Run a query to explore its results here."}
            </p>
            <span>
              <kbd>⌘ ↵</kbd>Run current query
            </span>
          </div>
        ) : gridMode === "transpose" ? (
          <TransposeGridView
            tab={tab}
            result={result}
            visibleColumns={resolvedLayout.visibleColumns}
            onSelect={handleSelectCell}
            onOpenValue={onOpenValue}
          />
        ) : gridMode === "record" ? (
          <RecordCardView
            tab={tab}
            result={result}
            visibleColumns={resolvedLayout.visibleColumns}
            onSelect={handleSelectCell}
            onOpenValue={onOpenValue}
          />
        ) : (
          <Suspense fallback={<div className="grid-empty">Opening data grid…</div>}>
            <DatabaseGrid
              ref={gridRef}
              key={tab.id}
              tab={tab}
              result={result}
              theme={state.theme}
              onSelect={handleSelectCell}
              onOpenValue={onOpenValue}
              onSelectedRowsChange={setSelectedRows}
              focusRowRequest={focusRowRequest}
              onSort={
                filtering
                  ? (column, additive) =>
                      queryLock
                        ? state.notify(queryLock)
                        : applyQuery(cycleSort(tab.query, column.name, additive))
                  : undefined
              }
            />
          </Suspense>
        )}
      </div>
      <div className="result-footer">
        <div>
          {tab.kind === "benchmark" ? (
            <>
              <span className="status-dot completed" />
              1,000,000 rows · virtualized
            </>
          ) : tab.kind === "object" && tab.object ? (
            <>
              <ObjectKindGlyph kind={tab.object.kind} size={12} />
              {`${tab.object.schema}.${objectTabTitle(tab.object)} · read-only metadata`}
            </>
          ) : tab.kind === "table" ? (
            <>
              <Table2 size={12} />
              {tab.loadedRows.toLocaleString()} rows loaded{tab.hasMore ? " · scroll for more" : ""}
              {profile?.engine === "snowflake"
                ? " · unordered preview · use SQL for more"
                : tab.schema && !tab.schema.primaryKey.length
                  ? " · first page only, no primary key"
                  : ""}
              {tab.pages.some((page) => page.result.status === "truncated") && (
                <span className="truncated-label">Preview limit reached</span>
              )}
            </>
          ) : (
            <>
              <span className={`status-dot ${tab.status}`} />
              {result?.retainedRows.toLocaleString() ?? "0"} rows
              {result?.status === "truncated" && (
                <span className="truncated-label">Retention limit reached</span>
              )}
              {tab.status === "completed" && result?.columns.length === 0 && (
                <span>{tab.metrics[result.statementIndex]?.affectedRows ?? "0"} affected</span>
              )}
            </>
          )}
        </div>
        <div>
          {pendingCount > 0 && (
            <span className="dirty-count">
              <i className="dirty-dot" />
              {pendingCount} pending
            </span>
          )}
          <span>{tab.kind === "benchmark" ? "Synthetic data" : "Results stay in memory"}</span>
        </div>
      </div>
    </>
  );
}

export function Welcome({
  onConnect,
  onQuery,
  onBenchmark,
}: {
  onConnect(): void;
  onQuery(): void;
  onBenchmark(): void;
}) {
  return (
    <div className="welcome">
      <div className="welcome-mark">
        <img src="/joinery.svg" alt="" width={52} height={52} />
      </div>
      <span className="eyebrow">A LITTLE CLARITY FOR YOUR DATA</span>
      <h1>Welcome to Joinery.</h1>
      <p>
        Your databases. A clear workspace.
        <br />
        Everything you need to get into the flow.
      </p>
      <button className="button primary welcome-connect" onClick={onConnect}>
        <Plus size={15} />
        Connect a database
        <ArrowRight size={15} />
      </button>
      <div className="welcome-cards">
        <button onClick={onQuery}>
          <div className="welcome-card-icon">
            <Code2 size={20} />
          </div>
          <strong>Start with SQL</strong>
          <span>A scratchpad for your next idea.</span>
          <kbd>⌘ T</kbd>
        </button>
        <button onClick={onBenchmark}>
          <div className="welcome-card-icon">
            <Gauge size={20} />
          </div>
          <strong>Take the grid for a spin</strong>
          <span>A million rows, generated as you scroll.</span>
          <ArrowRight size={15} />
        </button>
      </div>
      <div className="welcome-shortcuts">
        <span>
          <kbd>⌘ K</kbd>Command palette
        </span>
        <span>
          <kbd>⌘ B</kbd>Toggle sidebar
        </span>
        <span>
          <ShieldCheck size={12} />
          Built for considered changes
        </span>
      </div>
    </div>
  );
}
