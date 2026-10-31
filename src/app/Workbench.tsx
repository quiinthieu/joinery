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
  Check,
  ChevronDown,
  Clock3,
  Code2,
  Columns3,
  Download,
  FileCode2,
  Gauge,
  Loader2,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Square,
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
  SessionInfo,
  TableInfo,
  TransactionMode,
} from "../ipc/generated";
import type { StudioTab } from "../state/studio";
import {
  useStudio,
  querySessionForTab,
  isLastOwnerOfSecondarySession,
  sessionHasOpenTransaction,
} from "../state/studio";
import { client, messageOf } from "../ipc/client";
import { dirtyStore } from "../data/dirty-store";
import type { CellTarget, DatabaseGridHandle } from "../features/grid/DatabaseGrid";
import { CopyAsMenu } from "../features/grid/CopyAsMenu";
import { TableFilterBar } from "../features/grid/TableFilterBar";
import { findTableLayout, resolveTableLayout } from "../features/grid/table-layout";
import { cycleSort, hasRawClauses } from "../features/grid/table-query";
import type { EditorHandle } from "../features/studio/SqlEditor";
import {
  DdlPanel,
  ObjectInspectorView,
  ObjectKindGlyph,
} from "../features/studio/ObjectInspectorView";
import { StructureView } from "../features/studio/UtilityViews";
import { objectTabTitle, tableInfoKindToObjectKind } from "../features/connections/explorer-tree";
import { RedisTools } from "../features/redis/RedisTools";
import { resultCache } from "../data/result-cache";
import { tabNavigation } from "../components/ui/tabs";

const SqlEditor = lazy(() =>
  import("../features/studio/SqlEditor").then((module) => ({ default: module.SqlEditor })),
);
const DatabaseGrid = lazy(() => import("../features/grid/DatabaseGrid"));
const EMPTY_TABLES: TableInfo[] = [];
interface Props {
  tab: StudioTab;
  profile?: ConnectionProfile;
  session?: SessionInfo;
  editorRef: RefObject<EditorHandle>;
  gridRef: RefObject<DatabaseGridHandle>;
  onRun(sql?: string): void;
  onReview(): void;
  onConfirm(title: string, description: string, action: () => void): void;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
  onOpenColumns(): void;
  onOpenExport(): void;
  onOpenImport(): void;
  onOpenCompare(): void;
  onNewSession(): void;
  onQueryChange(query: StudioTab["query"], options?: { force?: boolean }): void;
}

export function Workbench({
  tab,
  profile,
  session,
  editorRef,
  gridRef,
  onRun,
  onReview,
  onConfirm,
  onSelect,
  onOpenValue,
  onOpenColumns,
  onOpenExport,
  onOpenImport,
  onOpenCompare,
  onNewSession,
  onQueryChange,
}: Props) {
  const state = useStudio(),
    [editorSize, setEditorSize] = useState(290),
    [runningTime, setRunningTime] = useState(0),
    [selectedRows, setSelectedRows] = useState<number[]>([]),
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
    editorHeight = useRef(290);
  useSyncExternalStore(dirtyStore.subscribe, dirtyStore.snapshot);
  const objectExplorerSupported = Boolean(session?.capabilities.objectExplorer);
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
  const running = tab.status === "running" || tab.status === "queued";
  const result = tab.results.find((r) => r.id === tab.activeResultId) ?? tab.results[0];
  const resultIds = tab.pages.map((page) => page.result.id);
  const pendingCount = tab.kind === "table" ? dirtyStore.pendingCountForTab(tab.id, resultIds) : 0;
  const draftInserts = tab.kind === "table" ? dirtyStore.insertsForTab(tab.id) : [];
  const tableColumns = tab.kind === "table" ? (tab.schema?.columns ?? []) : [];
  const tableLayout =
    tab.kind === "table" ? findTableLayout(state.tableLayouts, tab.connectionId, tab.table) : null;
  const resolvedLayout = resolveTableLayout(tableColumns, tableLayout);
  const filtering = tab.kind === "table" && !!session?.capabilities.tableFiltering;
  const tableEditable =
    tab.kind === "table" &&
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
  function stageInsertRow() {
    try {
      dirtyStore.addInsertRow(tab.id);
      const nextRow = tab.loadedRows + dirtyStore.insertsForTab(tab.id).length - 1;
      setFocusRowRequest((prev) => ({ seq: (prev?.seq ?? 0) + 1, row: nextRow }));
    } catch (error) {
      state.notify(error instanceof Error ? error.message : "Could not stage inserted row");
    }
  }
  function handleDeleteSelection() {
    try {
      if (onlyDraftsSelected) {
        dirtyStore.removeInsertRows(tab.id, selectedDraftIds);
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
      if (selectedDraftIds.length) dirtyStore.removeInsertRows(tab.id, selectedDraftIds);
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
  const txMode: TransactionMode = runtimeState?.transactionMode ?? "autoCommit";
  const txState = runtimeState?.transactionState ?? "idle";
  function changeConnection(connectionId: string) {
    const lastOwner = isLastOwnerOfSecondarySession(state.tabs, tab);
    const apply = () => {
      if (lastOwner && tab.querySession && !tab.querySessionClosed)
        void state
          .disconnectQuerySession(tab.querySession.id)
          .catch((error) => state.notify(messageOf(error)));
      for (const result of tab.results) {
        resultCache.release(result.id);
        void client.call("release_result", { resultId: result.id }).catch(() => {});
      }
      state.updateTab(tab.id, {
        connectionId: connectionId || null,
        results: [],
        activeResultId: null,
        status: "idle",
        error: null,
        metrics: {},
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
        "This closes the isolated SQL session and rolls back its open transactions.",
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
                            "Disconnecting this session rolls back its open transaction.",
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
          {tab.kind === "query" && session?.capabilities.transactions && (
            <>
              <select
                aria-label="Transaction mode"
                value={txMode}
                disabled={running || txBusy}
                onChange={(event) => {
                  const nextMode = event.target.value as TransactionMode;
                  setTxBusy(true);
                  void state
                    .setTransactionMode(session.id, nextMode)
                    .catch((error) => state.notify(messageOf(error)))
                    .finally(() => setTxBusy(false));
                }}
              >
                <option value="autoCommit">Auto-commit</option>
                <option value="manual">Manual commit</option>
              </select>
              {txState !== "idle" && (
                <span className={`tx-badge ${txState}`} aria-live="polite">
                  {txState === "failed" ? "Failed transaction" : "In transaction"}
                </span>
              )}
              {(txMode === "manual" || txState !== "idle") && (
                <>
                  <button
                    className="button ghost small"
                    aria-label="Commit transaction"
                    disabled={
                      running ||
                      txBusy ||
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
                    disabled={running || txBusy || txState === "idle"}
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
          {tab.kind === "query" && (
            <>
              <button
                className="button ghost small"
                title="Format SQL · ⇧⌘F"
                disabled={profile?.engine === "redis"}
                onClick={() => editorRef.current?.format()}
              >
                <Code2 size={13} />
                Format
              </button>
              {running ? (
                <button
                  className="button cancel-query"
                  onClick={() =>
                    tab.runId &&
                    void client
                      .call("cancel_run", { runId: tab.runId })
                      .catch((error) => state.notify(messageOf(error)))
                  }
                >
                  <Square size={11} />
                  Cancel<kbd>⌘ .</kbd>
                </button>
              ) : (
                <button
                  className="button primary run-button"
                  onClick={() => onRun()}
                  disabled={!session}
                >
                  <Play size={12} fill="currentColor" />
                  Run<kbd>⌘ ↵</kbd>
                </button>
              )}
              <button
                className="icon-button run-all"
                title="Run all statements · ⇧⌘↵"
                aria-label="Run all statements"
                disabled={running || !session}
                onClick={() => onRun(tab.sql)}
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
          {tab.kind === "object" && (
            <button
              className="button ghost small"
              disabled={!session}
              onClick={() => state.updateTab(tab.id, { revision: tab.revision + 1 })}
            >
              <RefreshCw size={13} />
              Refresh
            </button>
          )}
        </div>
      </div>
      {tab.kind === "query" && profile?.engine === "redis" && (
        <RedisTools
          sessionId={session?.id ?? null}
          disabled={running}
          readOnly={profile.safeMode === "readOnly"}
          onCommand={(command) => {
            state.updateTab(tab.id, { sql: command });
            onRun(command);
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
                engine={profile?.engine ?? "postgres"}
                sessionId={session?.id ?? null}
                tables={profile ? (state.catalog[profile.id] ?? EMPTY_TABLES) : EMPTY_TABLES}
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
            {result && result.columns.length > 0 && (
              <>
                <CopyAsMenu gridRef={gridRef} disabled={running} />
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
                  {tab.elapsedMs ? `${tab.elapsedMs} ms` : "Ready when you are"}
                </>
              )}
            </span>
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
                <CopyAsMenu gridRef={gridRef} disabled={running || tab.loadingPage} />
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
          Generated on demand · no database connection · constant row storage
        </div>
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
      {tab.error && (
        <div
          className={`query-error ${tab.error.code === "outcomeUnknown" ? "uncertain" : ""}`}
          role="alert"
        >
          <span className="error-symbol">!</span>
          <span>{tab.error.message}</span>
          <button
            className="icon-button"
            aria-label="Dismiss query error"
            onClick={() => state.updateTab(tab.id, { error: null })}
          >
            <X size={12} />
          </button>
        </div>
      )}
      <div
        className="result-panel"
        role={tab.kind === "query" && result ? "tabpanel" : undefined}
        id={`result-panel-${tab.id}`}
        aria-labelledby={tab.kind === "query" && result ? `result-tab-${result.id}` : undefined}
      >
        {tab.kind === "object" ? (
          <ObjectInspectorView key={tab.id} tab={tab} profile={profile} session={session} />
        ) : tab.kind === "table" && tab.view === "structure" ? (
          <StructureView
            schema={tab.schema}
            details={activeTableDetails}
            detailsError={activeTableDetailsError}
            connectionId={tab.connectionId}
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
        ) : tab.kind === "query" && !result ? (
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
        ) : (
          <Suspense fallback={<div className="grid-empty">Opening data grid…</div>}>
            <DatabaseGrid
              ref={gridRef}
              key={tab.id}
              tab={tab}
              result={result}
              theme={state.theme}
              onSelect={onSelect}
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
