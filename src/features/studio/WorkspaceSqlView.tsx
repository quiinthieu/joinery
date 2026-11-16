import { lazy, Suspense, useMemo, useRef, useState } from "react";
import {
  Clock3,
  Columns3,
  Database,
  ExternalLink,
  Layers,
  Play,
  Sparkles,
  Table2,
} from "lucide-react";
import type {
  ColumnInfo,
  ResultInfo,
  WorkspaceResultMount,
  WorkspaceSqlResponse,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import {
  isConsoleOwnedResult,
  isSnowflakeConsoleSession,
  querySessionForTab,
  useStudio,
  type StudioTab,
} from "../../state/studio";
import type { CellTarget, DatabaseGridHandle } from "../grid/DatabaseGrid";
import { CopyAsMenu } from "../grid/CopyAsMenu";
import { RecordCardView, TransposeGridView } from "../grid/GridAltViews";
import { ValueDialog } from "../grid/ValueDialog";
import { resolveTableLayout } from "../grid/table-layout";

const DatabaseGrid = lazy(() => import("../grid/DatabaseGrid"));

export interface MountableDataset {
  key: string;
  defaultAlias: string;
  title: string;
  connectionLabel: string;
  resultIds: string[];
  columns: ColumnInfo[];
  rowCount: number;
}

function sanitizeAlias(raw: string, index: number): string {
  const cleaned = raw
    .trim()
    .replace(/[^A-Za-z0-9_]+/g, "_")
    .replace(/^_+/, "")
    .replace(/_+$/, "")
    .slice(0, 48);
  const base = cleaned && /^[A-Za-z_]/.test(cleaned) ? cleaned : `r${index + 1}`;
  return base.toLowerCase().startsWith("sqlite_") ? `ds_${base}` : base;
}

export function discoverMountableDatasets(
  tabs: readonly StudioTab[],
  profileNames: ReadonlyMap<string, string>,
): MountableDataset[] {
  const out: MountableDataset[] = [];
  const usedAliases = new Set<string>();

  const claimAlias = (preferred: string) => {
    const base = sanitizeAlias(preferred, out.length);
    let candidate = base;
    let suffix = 2;
    while (usedAliases.has(candidate.toLowerCase())) {
      candidate = `${base}_${suffix++}`;
    }
    usedAliases.add(candidate.toLowerCase());
    return candidate;
  };

  const studioState = useStudio.getState();
  for (const t of tabs) {
    const tabSession = t.connectionId
      ? t.kind === "query"
        ? querySessionForTab(t, studioState.sessions)
        : studioState.sessions[t.connectionId]
      : null;
    if (tabSession && isSnowflakeConsoleSession(studioState, tabSession.id)) {
      continue;
    }
    const connName = t.connectionId ? (profileNames.get(t.connectionId) ?? "Connection") : "Local";
    if (t.kind === "table" && t.pages.length > 0 && t.schema && t.schema.columns.length > 0) {
      const validPages = t.pages.filter((p) => !isConsoleOwnedResult(p.result));
      if (validPages.length > 0) {
        const rows = validPages.reduce((sum, p) => sum + p.result.retainedRows, 0);
        out.push({
          key: `table:${t.id}`,
          defaultAlias: claimAlias(t.table?.name ?? t.title),
          title: t.table ? `${t.table.schema}.${t.table.name}` : t.title,
          connectionLabel: connName,
          resultIds: validPages.map((p) => p.result.id),
          columns: t.schema.columns,
          rowCount: rows,
        });
      }
    } else if (t.kind === "query" || t.kind === "result") {
      t.results.forEach((res, idx) => {
        if (isConsoleOwnedResult(res) || res.columns.length === 0) return;
        const suffix = t.results.length > 1 ? ` #${idx + 1}` : "";
        out.push({
          key: `res:${t.id}:${res.id}`,
          defaultAlias: claimAlias(res.table?.name ?? `${t.title}${idx > 0 ? `_${idx + 1}` : ""}`),
          title: `${t.title}${suffix}`,
          connectionLabel: connName,
          resultIds: [res.id],
          columns: res.columns,
          rowCount: res.retainedRows,
        });
      });
    }
  }
  return out;
}

export function WorkspaceSqlView({ tab }: { tab: StudioTab }) {
  const state = useStudio();
  const gridRef = useRef<DatabaseGridHandle>(null);

  const profileNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of state.bootstrap?.profiles ?? []) map.set(p.id, p.name);
    return map;
  }, [state.bootstrap?.profiles]);

  const available = useMemo(
    () => discoverMountableDatasets(state.tabs, profileNames),
    [profileNames, state.tabs],
  );

  const [selectedKeys, setSelectedKeys] = useState<Record<string, boolean>>({});
  const [aliases, setAliases] = useState<Record<string, string>>({});
  const [hasEditedSql, setHasEditedSql] = useState(() => Boolean(tab.sql));
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastMeta, setLastMeta] = useState<{ elapsedMs: number; mountedRows: number } | null>(null);
  const [gridMode, setGridMode] = useState<"grid" | "transpose" | "record">("grid");
  const [valueTarget, setValueTarget] = useState<CellTarget | null>(null);

  const effectiveSelectedKeys = useMemo(() => {
    const anyExplicit = Object.keys(selectedKeys).length > 0;
    const out = new Set<string>();
    for (const item of available) {
      const chosen = anyExplicit ? Boolean(selectedKeys[item.key]) : out.size < 4;
      if (chosen) out.add(item.key);
    }
    return out;
  }, [available, selectedKeys]);

  const activeMounts = useMemo(
    () =>
      available
        .filter((item) => effectiveSelectedKeys.has(item.key))
        .map((item) => ({
          dataset: item,
          alias: (aliases[item.key] ?? item.defaultAlias).trim(),
        })),
    [aliases, available, effectiveSelectedKeys],
  );

  const defaultSql = useMemo(() => {
    if (activeMounts.length === 0) return "SELECT 1 AS ready;";
    if (activeMounts.length === 1) {
      return `SELECT *\nFROM ${activeMounts[0].alias}\nLIMIT 100;`;
    }
    const left = activeMounts[0];
    const right = activeMounts[1];
    const leftCol = left.dataset.columns[0]?.name ?? "id";
    const rightMatch =
      right.dataset.columns.find((c) => c.name.toLowerCase() === leftCol.toLowerCase())?.name ??
      right.dataset.columns[0]?.name ??
      "id";
    return `SELECT a.*, b.*\nFROM ${left.alias} a\nJOIN ${right.alias} b ON a."${leftCol.replaceAll('"', '""')}" = b."${rightMatch.replaceAll('"', '""')}"\nLIMIT 100;`;
  }, [activeMounts]);

  const sqlText = hasEditedSql || Boolean(tab.sql) ? tab.sql : defaultSql;
  const activeResult: ResultInfo | undefined = tab.results[0];
  const resolvedLayout = useMemo(
    () =>
      resolveTableLayout(
        activeResult?.columns ?? [],
        activeResult ? (tab.resultLayouts?.[activeResult.id] ?? null) : null,
      ),
    [activeResult, tab.resultLayouts],
  );

  async function runQuery() {
    if (activeMounts.length === 0) {
      setError("Select at least one retained result set or table page to mount.");
      return;
    }
    const mounts: WorkspaceResultMount[] = activeMounts.map((m) => ({
      alias: m.alias,
      resultIds: m.dataset.resultIds,
      label: `${m.dataset.connectionLabel}: ${m.dataset.title}`,
    }));
    setRunning(true);
    setError(null);
    try {
      const resp = await client.call<WorkspaceSqlResponse>("run_workspace_sql", {
        request: {
          mounts,
          sql: sqlText,
        },
      });
      state.setWorkspaceSqlResult(tab.id, sqlText, resp);
      setLastMeta({ elapsedMs: resp.elapsedMs, mountedRows: resp.mountedRows });
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setRunning(false);
    }
  }

  return (
    <div
      className="utility-view workspace-sql-view"
      style={{ display: "flex", flexDirection: "column", height: "100%" }}
    >
      <header className="utility-header">
        <div>
          <span className="eyebrow">CROSS-SOURCE WORKSPACE SQL</span>
          <h1>Query across retained results</h1>
          <p>
            Join, filter, and aggregate snapshots across PostgreSQL, MySQL/MariaDB, and Snowflake in
            a sandboxed read-only in-memory SQLite engine — zero remote database load.
          </p>
        </div>
        <div className="diagram-toolbar">
          {activeMounts.length >= 2 && (
            <button
              type="button"
              className="button ghost"
              disabled={running}
              onClick={() => {
                setHasEditedSql(true);
                state.updateTab(tab.id, { sql: defaultSql });
              }}
            >
              <Sparkles size={14} />
              JOIN template
            </button>
          )}
          <button
            type="button"
            className="button primary"
            disabled={running || activeMounts.length === 0}
            onClick={() => void runQuery()}
          >
            <Play size={14} />
            {running ? "Running…" : "Run workspace SQL"}
          </button>
        </div>
      </header>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(260px, 340px) 1fr",
          gap: 12,
          padding: "0 16px 12px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <section
          aria-label="Mounted result sets"
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 6,
            maxHeight: 190,
            overflow: "auto",
            paddingRight: 4,
          }}
        >
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <strong style={{ fontSize: 12 }}>
              Mounted datasets ({activeMounts.length}/{available.length})
            </strong>
          </div>
          {available.length === 0 ? (
            <div className="field-hint">
              Run a SQL query or browse a table on any connection first — its retained rows will
              appear here ready to mount and join.
            </div>
          ) : (
            available.map((item) => {
              const checked = effectiveSelectedKeys.has(item.key);
              const alias = aliases[item.key] ?? item.defaultAlias;
              return (
                <div
                  key={item.key}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                    padding: "6px 8px",
                    borderRadius: 6,
                    border: "1px solid var(--border)",
                    background: checked ? "var(--surface-hover)" : "var(--surface)",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <input
                      type="checkbox"
                      aria-label={`Mount ${item.title}`}
                      checked={checked}
                      onChange={(e) => {
                        const nextChecked = e.target.checked;
                        setSelectedKeys((prev) => {
                          const base: Record<string, boolean> = {};
                          for (const d of available) {
                            base[d.key] = effectiveSelectedKeys.has(d.key);
                          }
                          base[item.key] = nextChecked;
                          return base;
                        });
                      }}
                    />
                    <Database size={12} />
                    <span style={{ fontSize: 12, fontWeight: 500, flex: 1 }} className="truncate">
                      {item.connectionLabel} · {item.title}
                    </span>
                    <small className="muted">{item.rowCount.toLocaleString()} rows</small>
                  </div>
                  {checked && (
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <small className="muted">AS</small>
                      <input
                        aria-label={`Alias for ${item.title}`}
                        className="mono"
                        style={{ flex: 1, fontSize: 12, padding: "2px 6px" }}
                        value={alias}
                        onChange={(e) =>
                          setAliases((prev) => ({ ...prev, [item.key]: e.target.value }))
                        }
                      />
                      <small
                        className="muted truncate"
                        title={item.columns.map((c) => c.name).join(", ")}
                      >
                        {item.columns.length} cols
                      </small>
                    </div>
                  )}
                </div>
              );
            })
          )}
        </section>

        <section style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <strong style={{ fontSize: 12 }}>SQLite query (SELECT / WITH / VALUES)</strong>
            <small className="muted">⌘Enter / Ctrl+Enter to run</small>
          </div>
          <textarea
            aria-label="Workspace SQL query"
            className="value-editor mono"
            style={{ minHeight: 145, resize: "vertical" }}
            value={sqlText}
            spellCheck={false}
            onChange={(e) => {
              setHasEditedSql(true);
              state.updateTab(tab.id, { sql: e.target.value });
            }}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                void runQuery();
              }
            }}
          />
        </section>
      </div>

      {error && (
        <div className="inline-error" role="alert" style={{ margin: "10px 16px 0" }}>
          {error}
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        {activeResult ? (
          <>
            <div className="results-header table-views">
              <div className="result-tabs">
                <button className="active" type="button" aria-pressed="true">
                  <Layers size={12} />
                  Workspace result
                  <span>{activeResult.retainedRows.toLocaleString()}</span>
                  {activeResult.status === "truncated" && (
                    <span className="query-error-badge" style={{ marginLeft: 4 }}>
                      Truncated
                    </span>
                  )}
                </button>
              </div>
              <div className="table-view-actions">
                <div className="segmented-inline" role="group" aria-label="Result layout view">
                  <button
                    type="button"
                    className={`button ghost small ${gridMode === "grid" ? "active" : ""}`}
                    aria-pressed={gridMode === "grid"}
                    onClick={() => setGridMode("grid")}
                  >
                    Grid
                  </button>
                  <button
                    type="button"
                    className={`button ghost small ${gridMode === "transpose" ? "active" : ""}`}
                    aria-pressed={gridMode === "transpose"}
                    onClick={() => setGridMode("transpose")}
                  >
                    <Columns3 size={12} />
                    Transpose
                  </button>
                  <button
                    type="button"
                    className={`button ghost small ${gridMode === "record" ? "active" : ""}`}
                    aria-pressed={gridMode === "record"}
                    onClick={() => setGridMode("record")}
                  >
                    Record
                  </button>
                </div>
                <CopyAsMenu gridRef={gridRef} disabled={gridMode !== "grid"} />
                <button
                  type="button"
                  className="button ghost small"
                  onClick={() => state.detachResultToTab(tab.id, activeResult.id)}
                >
                  <ExternalLink size={12} />
                  Open in tab
                </button>
                {lastMeta && (
                  <span className="result-caption">
                    <Clock3 size={12} />
                    {lastMeta.elapsedMs} ms · {lastMeta.mountedRows.toLocaleString()} mounted rows
                  </span>
                )}
              </div>
            </div>
            <div className="result-panel" style={{ flex: 1, minHeight: 0 }}>
              {gridMode === "transpose" ? (
                <TransposeGridView
                  tab={tab}
                  result={activeResult}
                  visibleColumns={resolvedLayout.visibleColumns}
                  onSelect={() => {}}
                  onOpenValue={(target) => setValueTarget(target)}
                />
              ) : gridMode === "record" ? (
                <RecordCardView
                  tab={tab}
                  result={activeResult}
                  visibleColumns={resolvedLayout.visibleColumns}
                  onSelect={() => {}}
                  onOpenValue={(target) => setValueTarget(target)}
                />
              ) : (
                <Suspense fallback={<div className="grid-empty">Loading result grid…</div>}>
                  <DatabaseGrid
                    ref={gridRef}
                    tab={tab}
                    result={activeResult}
                    theme={state.theme}
                    onSelect={() => {}}
                    onOpenValue={(target) => setValueTarget(target)}
                  />
                </Suspense>
              )}
            </div>
          </>
        ) : (
          <div className="results-empty" style={{ flex: 1 }}>
            <Table2 size={27} />
            <h3>Ready to join across sources</h3>
            <p>
              Mount one or more retained result snapshots above and run standard SQL (JOINs, CTEs,
              window functions, GROUP BY, JSON functions) locally in memory.
            </p>
          </div>
        )}
      </div>
      {valueTarget && <ValueDialog target={valueTarget} onClose={() => setValueTarget(null)} />}
    </div>
  );
}
