import { lazy, Suspense, useEffect, useRef, useState, type RefObject } from "react";
import {
  ArrowRight,
  Check,
  ChevronDown,
  Clock3,
  Code2,
  Columns3,
  Gauge,
  Loader2,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Square,
  Table2,
  X,
} from "lucide-react";
import type { ConnectionProfile, SessionInfo, TableInfo } from "../ipc/generated";
import type { StudioTab } from "../state/studio";
import { useStudio, querySessionForTab } from "../state/studio";
import { client, messageOf } from "../ipc/client";
import { dirtyStore } from "../data/dirty-store";
import type { CellTarget } from "../features/grid/DatabaseGrid";
import { TableFilterBar } from "../features/grid/TableFilterBar";
import { findTableLayout, resolveTableLayout } from "../features/grid/table-layout";
import { cycleSort, hasRawClauses } from "../features/grid/table-query";
import type { EditorHandle } from "../features/studio/SqlEditor";
import { StructureView } from "../features/studio/UtilityViews";
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
  onRun(sql?: string): void;
  onReview(): void;
  onConfirm(title: string, description: string, action: () => void): void;
  onSelect(target: CellTarget | null): void;
  onOpenValue(target: CellTarget): void;
  onOpenColumns(): void;
  onNewSession(): void;
  onQueryChange(query: StudioTab["query"], options?: { force?: boolean }): void;
}

export function Workbench({
  tab,
  profile,
  session,
  editorRef,
  onRun,
  onReview,
  onConfirm,
  onSelect,
  onOpenValue,
  onOpenColumns,
  onNewSession,
  onQueryChange,
}: Props) {
  const state = useStudio(),
    [editorSize, setEditorSize] = useState(290),
    [runningTime, setRunningTime] = useState(0),
    editorHeight = useRef(290);
  const running = tab.status === "running" || tab.status === "queued";
  const result = tab.results.find((r) => r.id === tab.activeResultId) ?? tab.results[0];
  const edits = tab.pages.flatMap((page) => dirtyStore.forResult(page.result.id));
  const tableColumns = tab.kind === "table" ? (tab.schema?.columns ?? []) : [];
  const tableLayout =
    tab.kind === "table" ? findTableLayout(state.tableLayouts, tab.connectionId, tab.table) : null;
  const resolvedLayout = resolveTableLayout(tableColumns, tableLayout);
  const filtering = tab.kind === "table" && !!session?.capabilities.tableFiltering;
  const queryLock = !session
    ? "Connect to change sort or filters"
    : tab.dataSessionId !== session.id
      ? "Refresh after reconnecting"
      : edits.length
        ? "Review or discard pending edits to change sort or filters"
        : null;
  function applyQuery(query: StudioTab["query"]) {
    onQueryChange(query);
  }
  function changeConnection(connectionId: string) {
    const apply = () => {
      if (tab.querySession && !tab.querySessionClosed)
        void client
          .call("disconnect", { sessionId: tab.querySession.id })
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
    if (tab.querySession && !tab.querySessionClosed)
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
          ) : (
            <Table2 size={15} />
          )}
          <span>
            {tab.kind === "benchmark"
              ? "1,000,000 virtual rows"
              : tab.kind === "table"
                ? `${tab.table?.schema}.${tab.table?.name}`
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
                    : "Isolated session"
                  : session
                    ? "Shared session"
                    : "Disconnected"}
              </span>
              {(!tab.querySession || tab.querySessionClosed) && (
                <button
                  className="button ghost small"
                  disabled={running || !tab.connectionId}
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
                    for (const page of tab.pages) dirtyStore.clear(page.result.id);
                    if (hasRawClauses(tab.query)) onQueryChange(tab.query, { force: true });
                    else void state.refreshTable(tab.id);
                  };
                  if (edits.length)
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
              {edits.length > 0 && (
                <>
                  <button
                    className="button ghost small"
                    disabled={running}
                    onClick={() => {
                      for (const page of tab.pages) dirtyStore.clear(page.result.id);
                    }}
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
                    Review {edits.length} edit{edits.length === 1 ? "" : "s"}
                  </button>
                </>
              )}
            </>
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
                contextEpoch={JSON.stringify(
                  state.tabs
                    .filter(
                      (t) => session && querySessionForTab(t, state.sessions)?.id === session.id,
                    )
                    .map((t) => [t.id, t.status, t.revision]),
                )}
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
          </div>
          <div className="table-view-actions">
            {tab.view === "data" && tableColumns.length > 0 && (
              <>
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
        {tab.kind === "table" && tab.view === "structure" ? (
          <StructureView schema={tab.schema} />
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
              key={tab.id}
              tab={tab}
              result={result}
              theme={state.theme}
              onSelect={onSelect}
              onOpenValue={onOpenValue}
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
          {edits.length > 0 && (
            <span className="dirty-count">
              <i className="dirty-dot" />
              {edits.length} pending
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
