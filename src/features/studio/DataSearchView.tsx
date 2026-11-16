import { useMemo, useState } from "react";
import { ArrowUpRight, Database, FileCode2, Play, Search } from "lucide-react";
import type {
  DataSearchMatchMode,
  DataSearchReport,
  DataSearchTableMatch,
  FilterOp,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import {
  isSnowflakeConsoleSession,
  revokeSchemaSelectionBeforeWork,
  useStudio,
  type StudioTab,
} from "../../state/studio";

function quoteDouble(ident: string): string {
  return `"${ident.replaceAll('"', '""')}"`;
}

function quoteBacktick(ident: string): string {
  return `\`${ident.replaceAll("`", "``")}\``;
}

function escapeLikePattern(value: string): string {
  return value.replace(/[!%_]/g, (ch) => `!${ch}`);
}

function buildSqlWhereForColumns(
  engine: "postgres" | "mysql" | "sqlite",
  columns: readonly string[],
  query: string,
  matchMode: DataSearchMatchMode,
  caseSensitive: boolean,
): string | null {
  if (columns.length === 0) return null;
  const escapeLiteral = (raw: string) =>
    engine === "mysql"
      ? raw.replaceAll("\\", "\\\\").replaceAll("'", "''")
      : raw.replaceAll("'", "''");
  const lit = escapeLiteral(query);
  const likeBody = escapeLiteral(escapeLikePattern(query));
  const likePattern = matchMode === "startsWith" ? `${likeBody}%` : `%${likeBody}%`;

  const predicates = columns.map((col) => {
    const castExpr =
      engine === "mysql"
        ? `CAST(${quoteBacktick(col)} AS CHAR)`
        : `CAST(${quoteDouble(col)} AS TEXT)`;
    if (matchMode === "exact") {
      if (!caseSensitive) {
        return `LOWER(${castExpr}) = LOWER('${lit}')`;
      }
      return engine === "mysql"
        ? `${castExpr} COLLATE utf8mb4_bin = '${lit}'`
        : `${castExpr} = '${lit}'`;
    }
    if (engine === "postgres") {
      return caseSensitive
        ? `${castExpr} LIKE '${likePattern}' ESCAPE '!'`
        : `${castExpr} ILIKE '${likePattern}' ESCAPE '!'`;
    }
    if (engine === "sqlite") {
      if (!caseSensitive) {
        return `LOWER(${castExpr}) LIKE LOWER('${likePattern}') ESCAPE '!'`;
      }
      return matchMode === "startsWith"
        ? `SUBSTR(${castExpr}, 1, LENGTH('${lit}')) = '${lit}'`
        : `INSTR(${castExpr}, '${lit}') > 0`;
    }
    return caseSensitive
      ? `${castExpr} COLLATE utf8mb4_bin LIKE '${likePattern}' ESCAPE '!'`
      : `LOWER(${castExpr}) LIKE LOWER('${likePattern}') ESCAPE '!'`;
  });

  return predicates.length === 1 ? predicates[0]! : `(${predicates.join(" OR ")})`;
}

export function DataSearchView({ tab }: { tab: StudioTab }) {
  const state = useStudio();
  const connectedProfiles = useMemo(
    () =>
      (state.bootstrap?.profiles ?? []).filter((p) => {
        const sess = state.sessions[p.id];
        if (!sess || p.engine === "redis") return false;
        if (p.engine === "snowflake" && !sess.capabilities.relationBrowsing) return false;
        if (isSnowflakeConsoleSession(state, sess.id)) return false;
        return true;
      }),
    [state],
  );

  const effectiveConnectionId =
    tab.connectionId && connectedProfiles.some((p) => p.id === tab.connectionId)
      ? tab.connectionId
      : (connectedProfiles[0]?.id ?? tab.connectionId ?? "");
  const session = effectiveConnectionId ? (state.sessions[effectiveConnectionId] ?? null) : null;
  const profile = state.bootstrap?.profiles.find((p) => p.id === effectiveConnectionId) ?? null;

  const knownSchemas = useMemo(() => {
    if (!effectiveConnectionId) return [];
    const set = new Set<string>([
      ...(state.catalog[effectiveConnectionId] ?? []).map((t) => t.table.schema),
      ...(state.objects[effectiveConnectionId] ?? []).map((o) => o.object.schema),
    ]);
    if (set.size === 0 && profile?.snowflake?.schema) {
      set.add(profile.snowflake.schema);
    }
    return [...set].sort();
  }, [effectiveConnectionId, profile?.snowflake?.schema, state.catalog, state.objects]);

  const [query, setQuery] = useState("");
  const [matchMode, setMatchMode] = useState<DataSearchMatchMode>("contains");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [maxTables, setMaxTables] = useState<number>(25);
  const [selectedSchema, setSelectedSchema] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<DataSearchReport | null>(null);

  async function runSearch(event?: React.FormEvent) {
    event?.preventDefault();
    if (!session) {
      setError("Connect to a PostgreSQL, MySQL/MariaDB, SQLite, or Snowflake SDK database first.");
      return;
    }
    if (!query) {
      setError("Enter a value to search for across table columns.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      revokeSchemaSelectionBeforeWork(session.id);
      const next = await client.call<DataSearchReport>("search_database_data", {
        request: {
          sessionId: session.id,
          query,
          schemas: selectedSchema ? [selectedSchema] : [],
          tables: [],
          matchMode,
          caseSensitive,
          maxTables,
        },
      });
      setReport(next);
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(false);
    }
  }

  function openMatch(tableMatch: DataSearchTableMatch) {
    if (!effectiveConnectionId || !report) return;
    const distinctMatchedCols = [...new Set(tableMatch.rows.flatMap((r) => r.matchedColumns))];
    const effectiveCols =
      distinctMatchedCols.length > 0 ? distinctMatchedCols : tableMatch.searchableColumns;

    if (
      session?.engine === "postgres" ||
      session?.engine === "mysql" ||
      session?.engine === "sqlite"
    ) {
      const op: FilterOp =
        report.matchMode === "exact"
          ? "eq"
          : report.matchMode === "startsWith"
            ? "startsWith"
            : "contains";
      const singleCol = effectiveCols.length === 1 ? effectiveCols[0]! : null;
      const singleColInfo = singleCol
        ? tableMatch.columns.find((c) => c.name === singleCol)
        : undefined;
      const supportedTypedKind =
        singleColInfo !== undefined &&
        singleColInfo.kind !== "json" &&
        singleColInfo.kind !== "binary" &&
        singleColInfo.kind !== "unknown";
      const canUseTypedSingleFilter =
        singleCol !== null &&
        supportedTypedKind &&
        ((!report.caseSensitive && report.matchMode !== "exact" && session.engine !== "mysql") ||
          (report.caseSensitive && report.matchMode === "exact" && session.engine === "postgres"));

      const filters =
        canUseTypedSingleFilter && singleCol
          ? [{ column: singleCol, op, value: report.query }]
          : [];
      const whereClause = canUseTypedSingleFilter
        ? null
        : buildSqlWhereForColumns(
            session.engine,
            effectiveCols,
            report.query,
            report.matchMode,
            report.caseSensitive,
          );
      void state
        .openTable(effectiveConnectionId, tableMatch.table, {
          sort: [],
          filters,
          whereClause,
          orderClause: null,
        })
        .catch((err) => state.notify(messageOf(err)));
      return;
    }

    const escapedLiteral = report.query.replaceAll("\\", "\\\\").replaceAll("'", "''");
    const colPredicates = effectiveCols.map((colName) => {
      const matchedColInfo = tableMatch.columns.find((c) => c.name === colName);
      const upperType = (matchedColInfo?.databaseType ?? "").trim().toUpperCase();
      const qCol = quoteDouble(colName);
      const colExpr =
        upperType.startsWith("OBJECT") || upperType.startsWith("ARRAY")
          ? `TO_JSON(${qCol})`
          : upperType.startsWith("VARIANT")
            ? `IFF(IS_OBJECT(${qCol}) OR IS_ARRAY(${qCol}), TO_JSON(${qCol}), TO_VARCHAR(${qCol}))`
            : upperType.startsWith("GEOGRAPHY") || upperType.startsWith("GEOMETRY")
              ? `ST_ASEWKT(${qCol})`
              : `TO_VARCHAR(${qCol})`;
      if (report.matchMode === "exact") {
        return report.caseSensitive
          ? `${colExpr} = '${escapedLiteral}'`
          : `LOWER(${colExpr}) = LOWER('${escapedLiteral}')`;
      }
      if (report.matchMode === "startsWith") {
        return report.caseSensitive
          ? `STARTSWITH(${colExpr}, '${escapedLiteral}')`
          : `STARTSWITH(LOWER(${colExpr}), LOWER('${escapedLiteral}'))`;
      }
      return report.caseSensitive
        ? `CONTAINS(${colExpr}, '${escapedLiteral}')`
        : `CONTAINS(LOWER(${colExpr}), LOWER('${escapedLiteral}'))`;
    });
    const whereExpr =
      colPredicates.length === 0
        ? "1 = 1"
        : colPredicates.length === 1
          ? colPredicates[0]!
          : `(${colPredicates.join("\n  OR ")})`;
    const activeDatabase = (
      report.database ||
      (session ? state.sessionStates[session.id]?.database : undefined) ||
      (session ? state.snowflakeContextAuthorities[session.id]?.state.database : undefined) ||
      profile?.database ||
      ""
    ).trim();
    const dbPrefix = activeDatabase ? `${quoteDouble(activeDatabase)}.` : "";
    const sql = `SELECT *\nFROM ${dbPrefix}${quoteDouble(tableMatch.table.schema)}.${quoteDouble(tableMatch.table.name)}\nWHERE ${whereExpr}\nLIMIT 100;`;
    state.openQuery(effectiveConnectionId, sql);
  }

  return (
    <div
      className="utility-view data-search-view"
      style={{ display: "flex", flexDirection: "column", height: "100%" }}
    >
      <header className="utility-header">
        <div>
          <span className="eyebrow">FULL-TEXT DATA SEARCH</span>
          <h1>Search values across tables</h1>
          <p>
            Bounded, parameterized scan across text and scalar columns (up to 50 tables, 10 sample
            hits per table, zero SQL interpolation).
          </p>
        </div>
      </header>

      <form
        onSubmit={(e) => void runSearch(e)}
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: 10,
          alignItems: "flex-end",
          padding: "0 16px 12px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <label className="field" style={{ minWidth: 180 }}>
          <span>Connection</span>
          <select
            aria-label="Search connection"
            value={effectiveConnectionId}
            disabled={busy || connectedProfiles.length === 0}
            onChange={(e) => {
              state.updateTab(tab.id, { connectionId: e.target.value });
              setSelectedSchema("");
              setReport(null);
            }}
          >
            {connectedProfiles.length === 0 ? (
              <option value="">No active SQL connection</option>
            ) : (
              connectedProfiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} ({p.engine})
                </option>
              ))
            )}
          </select>
        </label>

        {knownSchemas.length > 0 && (
          <label className="field" style={{ minWidth: 140 }}>
            <span>Schema scope</span>
            <select
              aria-label="Schema scope"
              value={selectedSchema}
              disabled={busy}
              onChange={(e) => setSelectedSchema(e.target.value)}
            >
              <option value="">All schemas</option>
              {knownSchemas.map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
          </label>
        )}

        <label className="field" style={{ flex: "1 1 240px" }}>
          <span>Search value</span>
          <input
            aria-label="Search database values"
            placeholder="e.g. ada@example.com, ORD-1042, error_code…"
            maxLength={512}
            value={query}
            disabled={busy}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>

        <label className="field" style={{ width: 135 }}>
          <span>Match mode</span>
          <select
            aria-label="Match mode"
            value={matchMode}
            disabled={busy}
            onChange={(e) => setMatchMode(e.target.value as DataSearchMatchMode)}
          >
            <option value="contains">Contains</option>
            <option value="startsWith">Starts with</option>
            <option value="exact">Exact match</option>
          </select>
        </label>

        <label className="field" style={{ width: 115 }}>
          <span>Table cap</span>
          <select
            aria-label="Table cap"
            value={maxTables}
            disabled={busy}
            onChange={(e) => setMaxTables(Number(e.target.value))}
          >
            <option value={10}>10 tables</option>
            <option value={25}>25 tables</option>
            <option value={50}>50 tables</option>
          </select>
        </label>

        <label className="checkbox-field" style={{ paddingBottom: 6 }}>
          <input
            type="checkbox"
            checked={caseSensitive}
            disabled={busy}
            onChange={(e) => setCaseSensitive(e.target.checked)}
          />
          <span>Case sensitive</span>
        </label>

        <button type="submit" className="button primary" disabled={busy || !session || !query}>
          <Play size={14} />
          {busy ? "Scanning…" : "Search tables"}
        </button>
      </form>

      {error && (
        <div className="inline-error" role="alert" style={{ margin: "10px 16px 0" }}>
          {error}
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 16 }}>
        {!session ? (
          <div className="utility-empty">
            <Database size={28} />
            <h3>Connect to search table data</h3>
            <p>
              Open a PostgreSQL, MySQL/MariaDB, SQLite, or ordinary Snowflake SDK connection to scan
              column values across schemas.
            </p>
          </div>
        ) : !report ? (
          <div className="utility-empty">
            <Search size={28} />
            <h3>Find any value across your schema</h3>
            <p>
              Enter an ID, email, UUID, or substring above to locate every table and column where it
              appears.
            </p>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            <div className="diagram-notice" role="status">
              Found <strong>{report.totalRowHits}</strong> sample row hit
              {report.totalRowHits === 1 ? "" : "s"} in <strong>{report.matches.length}</strong>{" "}
              table{report.matches.length === 1 ? "" : "s"} (scanned {report.scannedTables} table
              {report.scannedTables === 1 ? "" : "s"}
              {report.skippedTables > 0 ? `, skipped ${report.skippedTables}` : ""} in{" "}
              {report.elapsedMs} ms).
              {report.truncated
                ? " Results were capped by the search safety limits; narrow the schema scope or search term for exhaustive coverage."
                : ""}
            </div>

            {report.matches.length === 0 ? (
              <div className="utility-empty">
                <Search size={24} />
                <h3>No matching rows</h3>
                <p>
                  No rows matched &ldquo;{report.query}&rdquo; across {report.scannedTables} scanned
                  table{report.scannedTables === 1 ? "" : "s"}.
                </p>
              </div>
            ) : (
              report.matches.map((m) => {
                const matchedColSet = new Set(m.rows.flatMap((r) => r.matchedColumns));
                return (
                  <section
                    key={`${m.table.schema}.${m.table.name}`}
                    style={{
                      border: "1px solid var(--border)",
                      borderRadius: 8,
                      overflow: "hidden",
                      background: "var(--surface)",
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        padding: "8px 12px",
                        background: "var(--surface-hover)",
                        borderBottom: "1px solid var(--border)",
                      }}
                    >
                      <div
                        style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}
                      >
                        <strong className="mono">
                          {m.table.schema}.{m.table.name}
                        </strong>
                        <span className="schema-count">
                          {m.rows.length}
                          {m.hasMoreInTable ? "+" : ""} hits
                        </span>
                        {[...matchedColSet].map((col) => (
                          <span key={col} className="key-badge foreign">
                            {col}
                          </span>
                        ))}
                      </div>
                      <button
                        type="button"
                        className="button ghost small"
                        onClick={() => openMatch(m)}
                      >
                        {session.engine === "snowflake" ? (
                          <>
                            <FileCode2 size={12} />
                            Open SQL query
                          </>
                        ) : (
                          <>
                            <ArrowUpRight size={12} />
                            Open table
                          </>
                        )}
                      </button>
                    </div>
                    <div style={{ overflowX: "auto" }}>
                      <table className="structure-table">
                        <thead>
                          <tr>
                            {m.columns.map((col) => (
                              <th key={col.id} className="mono">
                                {col.name}
                              </th>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {m.rows.map((hit, rIdx) => (
                            <tr key={rIdx}>
                              {m.columns.map((col, cIdx) => {
                                const val = hit.row[cIdx] ?? null;
                                const isHit = hit.matchedColumns.includes(col.name);
                                return (
                                  <td
                                    key={col.id}
                                    className="mono"
                                    style={
                                      isHit
                                        ? {
                                            background:
                                              "color-mix(in srgb, var(--accent) 16%, transparent)",
                                            fontWeight: 600,
                                          }
                                        : undefined
                                    }
                                  >
                                    {val === null ? (
                                      <span className="grid-token-null">NULL</span>
                                    ) : (
                                      val.slice(0, 160)
                                    )}
                                  </td>
                                );
                              })}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </section>
                );
              })
            )}
          </div>
        )}
      </div>
    </div>
  );
}
