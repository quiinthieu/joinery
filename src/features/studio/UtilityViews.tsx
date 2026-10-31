import { useEffect, useState } from "react";
import { Activity, History, RefreshCw, Search, ArrowUpRight } from "lucide-react";
import type {
  AuditEntry,
  ConstraintKind,
  HistoryEntry,
  ObjectDetails,
  ProcessInfo,
  TableSchema,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import { Modal, InlineError } from "../../components/ui/Modal";
import { DependencySection } from "./ObjectInspectorView";

export function HistoryView() {
  const [entries, setEntries] = useState<HistoryEntry[]>([]),
    [search, setSearch] = useState(""),
    [error, setError] = useState<string | null>(null);
  const [audit, setAudit] = useState(false),
    [clearing, setClearing] = useState(false),
    [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    const timer = setTimeout(() => {
      void client
        .call<HistoryEntry[]>("history", { search })
        .then((entries) => {
          if (active) setEntries(entries);
        })
        .catch((error) => {
          if (active) setError(messageOf(error));
        });
    }, 120);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [search]);
  return (
    <div className="utility-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">YOUR WORK, REMEMBERED</span>
          <h1>Query history</h1>
          <p>Recent executions across your connections.</p>
        </div>
        <div className="utility-actions">
          <button className="button ghost" onClick={() => setClearing(true)}>
            Clear history
          </button>
          <button
            className="button ghost"
            onClick={() => {
              setSearch("");
              void client
                .call<HistoryEntry[]>("history", { search: "" })
                .then(setEntries)
                .catch((error) => setError(messageOf(error)));
            }}
          >
            <RefreshCw size={14} />
            Refresh
          </button>
        </div>
      </header>
      <div className="history-switch">
        <button
          className={!audit ? "selected" : ""}
          aria-pressed={!audit}
          onClick={() => setAudit(false)}
        >
          Executions
        </button>
        <button
          className={audit ? "selected" : ""}
          aria-pressed={audit}
          onClick={() => setAudit(true)}
        >
          Audit log
        </button>
      </div>
      {audit ? (
        <AuditLog />
      ) : (
        <>
          <div className="utility-search">
            <Search size={15} />
            <input
              aria-label="Search history"
              placeholder="Search SQL or connection names…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
          {error && <div className="inline-error">{error}</div>}
          {!entries.length ? (
            <div className="utility-empty">
              <History size={30} />
              <h3>No queries here yet</h3>
              <p>
                Your executed queries will appear here. SQL text is saved only when enabled for a
                connection.
              </p>
            </div>
          ) : (
            <div className="history-list">
              {entries.map((entry) => (
                <button
                  className="history-entry"
                  key={entry.id}
                  onClick={() => {
                    if (entry.sql) useStudio.getState().openQuery(entry.connectionId, entry.sql);
                    else
                      useStudio
                        .getState()
                        .notify("Only execution metadata was saved for this query.");
                  }}
                >
                  <div className="history-entry-top">
                    <span className={`status-dot ${entry.status}`} />
                    <strong>{entry.connectionName}</strong>
                    <span>{new Date(entry.startedAt).toLocaleString()}</span>
                    <em>{entry.elapsedMs} ms</em>
                  </div>
                  <code>
                    {entry.sql ??
                      `${entry.statementCount} statement${entry.statementCount === 1 ? "" : "s"} · SQL text not stored`}
                  </code>
                  {entry.sql && <ArrowUpRight size={15} className="history-open" />}
                </button>
              ))}
            </div>
          )}
        </>
      )}
      {clearing && (
        <Modal title="Clear local query history?" onClose={() => setClearing(false)} busy={busy}>
          <div className="modal-body">
            <p className="confirmation-description">
              This removes saved execution history from this device. Audit entries are retained.
            </p>
            <InlineError message={error} />
          </div>
          <footer className="modal-footer">
            <button className="button ghost" disabled={busy} onClick={() => setClearing(false)}>
              Cancel
            </button>
            <button
              className="button danger"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                void client
                  .call("clear_history")
                  .then(() => {
                    setEntries([]);
                    setClearing(false);
                  })
                  .catch((error) => setError(messageOf(error)))
                  .finally(() => setBusy(false));
              }}
            >
              Clear local history
            </button>
          </footer>
        </Modal>
      )}
    </div>
  );
}

function AuditLog() {
  const profiles = useStudio((state) => state.bootstrap?.profiles),
    [entries, setEntries] = useState<AuditEntry[]>([]),
    [search, setSearch] = useState(""),
    [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    void client
      .call<AuditEntry[]>("audit_log")
      .then((rows) => {
        if (active) setEntries(rows);
      })
      .catch((error) => {
        if (active) setError(messageOf(error));
      });
    return () => {
      active = false;
    };
  }, []);
  const name = (id: string) => profiles?.find((profile) => profile.id === id)?.name ?? id;
  const rows = entries.filter((row) =>
    `${name(row.connectionId)} ${row.action} ${row.outcome} ${row.digest}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  return (
    <>
      <p className="form-note">
        Latest 1,000 local audit entries. Digests identify reviewed operations; query text,
        bindings, and result rows are excluded.
      </p>
      <div className="utility-search">
        <Search size={15} />
        <input
          aria-label="Search audit log"
          placeholder="Search connection, outcome, or digest…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>
      <InlineError message={error} />
      <div className="structure-scroll">
        <table className="structure-table audit-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Connection</th>
              <th>Action</th>
              <th>Outcome</th>
              <th>Digest</th>
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 200).map((row) => (
              <tr key={row.id}>
                <td>{new Date(row.occurredAt).toLocaleString()}</td>
                <td>{name(row.connectionId)}</td>
                <td>{row.action}</td>
                <td>{row.outcome}</td>
                <td>
                  <code title={row.digest}>{row.digest}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!rows.length && <div className="grid-empty">No matching audit entries</div>}
        {rows.length > 200 && <p className="form-note">Search to narrow {rows.length} entries.</p>}
      </div>
    </>
  );
}

export function ProcessesView({
  tab,
  onCancel,
}: {
  tab: StudioTab;
  onCancel(process: ProcessInfo): void;
}) {
  const session = useStudio((s) => (tab.connectionId ? s.sessions[tab.connectionId] : null));
  const [processes, setProcesses] = useState<ProcessInfo[]>([]),
    [error, setError] = useState<string | null>(null),
    [loading, setLoading] = useState(false);
  async function refresh() {
    if (!session) return;
    setLoading(true);
    try {
      setProcesses(await client.call("processes", { sessionId: session.id }));
      setError(null);
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setLoading(false);
    }
  }
  useEffect(() => {
    void refresh();
  }, [session?.id]);
  useEffect(() => {
    if (tab.status === "completed") void refresh();
  }, [tab.status]);
  return (
    <div className="utility-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">INSTANCE ACTIVITY</span>
          <h1>Processes</h1>
          <p>
            {session?.engine === "mysql"
              ? "Inspect server sessions. Cancel your own queries from their SQL tabs."
              : "Inspect active sessions and cancel long-running queries."}
          </p>
        </div>
        <button
          className="button ghost"
          disabled={loading || !session}
          onClick={() => void refresh()}
        >
          <RefreshCw size={14} className={loading ? "spin" : ""} />
          Refresh
        </button>
      </header>
      {error && (
        <div className="inline-error" role="alert">
          {error}
        </div>
      )}
      {!session ? (
        <div className="utility-empty">
          <Activity size={28} />
          <h3>This connection is offline</h3>
          <p>Reconnect to inspect database sessions.</p>
        </div>
      ) : (
        <div className="structure-scroll">
          <table className="structure-table">
            <thead>
              <tr>
                <th>PID</th>
                <th>User</th>
                <th>Database</th>
                <th>State</th>
                <th>Duration</th>
                <th>Query</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {processes.map((process) => (
                <tr key={`${process.id}:${process.identity}`}>
                  <td className="mono">{process.id}</td>
                  <td>{process.user}</td>
                  <td>{process.database}</td>
                  <td>
                    <span className={`state-pill ${process.state === "active" ? "active" : ""}`}>
                      {process.state || "unknown"}
                    </span>
                  </td>
                  <td>{process.durationSeconds.toFixed(1)} s</td>
                  <td>
                    <code className="process-sql" title={process.query ?? ""}>
                      {process.query ?? "Unavailable"}
                    </code>
                  </td>
                  <td>
                    {process.state === "active" && session.engine !== "mysql" && (
                      <button
                        className="button small danger-text"
                        onClick={() => onCancel(process)}
                      >
                        Cancel query
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

const CONSTRAINT_KIND_LABELS: Record<ConstraintKind, string> = {
  primaryKey: "PRIMARY KEY",
  foreignKey: "FOREIGN KEY",
  unique: "UNIQUE",
  check: "CHECK",
  exclusion: "EXCLUDE",
};

export function StructureView({
  schema,
  details = null,
  detailsError = null,
  connectionId = null,
}: {
  schema: TableSchema | null;
  details?: ObjectDetails | null;
  detailsError?: string | null;
  connectionId?: string | null;
}) {
  if (!schema) return <div className="grid-empty">Loading table structure…</div>;
  return (
    <div className="structure-scroll">
      <table className="structure-table">
        <thead>
          <tr>
            <th>Column</th>
            <th>Type</th>
            <th>Nullable</th>
            <th>Key</th>
            <th>References</th>
          </tr>
        </thead>
        <tbody>
          {schema.columns.map((column) => {
            const references = schema.foreignKeys.flatMap((key) => {
              const index = key.columns.indexOf(column.name);
              return index < 0
                ? []
                : [
                    `${key.target.schema}.${key.target.name}.${key.targetColumns[index]}` +
                      (key.columns.length > 1 ? ` (${key.name})` : ""),
                  ];
            });
            return (
              <tr key={column.id}>
                <td className="mono">{column.name}</td>
                <td>
                  <code>{column.databaseType}</code>
                </td>
                <td>{column.nullable ? "Yes" : "No"}</td>
                <td>
                  {column.primaryKey ? (
                    <span className="key-badge">PRIMARY KEY</span>
                  ) : column.foreignKey ? (
                    <span className="key-badge foreign">FOREIGN KEY</span>
                  ) : (
                    "—"
                  )}
                </td>
                <td className="mono">{references.length ? references.join(", ") : "—"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {schema.foreignKeys.length > 0 && (
        <table className="structure-table" aria-label="Foreign keys">
          <thead>
            <tr>
              <th>Foreign key</th>
              <th>Columns</th>
              <th>References</th>
            </tr>
          </thead>
          <tbody>
            {schema.foreignKeys.map((key) => (
              <tr key={`${key.name}:${key.columns.join(",")}`}>
                <td className="mono">{key.name}</td>
                <td className="mono">{key.columns.join(", ")}</td>
                <td className="mono">
                  {`${key.target.schema}.${key.target.name} (${key.targetColumns.join(", ")})`}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {detailsError && (
        <div className="structure-details-error">
          <div className="inline-error" role="alert">
            {detailsError}
          </div>
        </div>
      )}
      {details && details.constraints.length > 0 && (
        <table className="structure-table" aria-label="Constraints">
          <thead>
            <tr>
              <th>Constraint</th>
              <th>Kind</th>
              <th>Columns</th>
              <th>References</th>
              <th>Definition</th>
            </tr>
          </thead>
          <tbody>
            {details.constraints.map((constraint) => (
              <tr key={`${constraint.kind}:${constraint.name}`}>
                <td className="mono">{constraint.name}</td>
                <td>
                  <span
                    className={`key-badge ${constraint.kind === "foreignKey" ? "foreign" : ""}`}
                  >
                    {CONSTRAINT_KIND_LABELS[constraint.kind]}
                  </span>
                </td>
                <td className="mono">
                  {constraint.columns.length ? constraint.columns.join(", ") : "—"}
                </td>
                <td className="mono">
                  {constraint.target ? (
                    connectionId ? (
                      <button
                        type="button"
                        className="structure-link-button"
                        onClick={() =>
                          void useStudio
                            .getState()
                            .openTable(connectionId, constraint.target!, undefined, "structure")
                        }
                      >
                        {`${constraint.target.schema}.${constraint.target.name}${
                          constraint.targetColumns.length
                            ? ` (${constraint.targetColumns.join(", ")})`
                            : ""
                        }`}
                      </button>
                    ) : (
                      `${constraint.target.schema}.${constraint.target.name}`
                    )
                  ) : (
                    "—"
                  )}
                </td>
                <td>
                  <code>{constraint.definition}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {details && details.indexes.length > 0 && (
        <table className="structure-table" aria-label="Indexes">
          <thead>
            <tr>
              <th>Index</th>
              <th>Method</th>
              <th>Uniqueness</th>
              <th>Keys / Expressions</th>
              <th>Partial predicate</th>
            </tr>
          </thead>
          <tbody>
            {details.indexes.map((index) => (
              <tr key={index.name}>
                <td className="mono">
                  {connectionId ? (
                    <button
                      type="button"
                      className="structure-link-button"
                      onClick={() =>
                        void useStudio.getState().openObject(
                          connectionId,
                          {
                            schema: details.object.schema,
                            name: index.name,
                            kind: "index",
                            identity: details.object.name,
                          },
                          "structure",
                        )
                      }
                    >
                      {index.name}
                    </button>
                  ) : (
                    index.name
                  )}
                </td>
                <td className="mono">{index.method}</td>
                <td>
                  {index.primary ? (
                    <span className="key-badge">PRIMARY KEY</span>
                  ) : index.unique ? (
                    <span className="key-badge foreign">UNIQUE</span>
                  ) : (
                    "Non-unique"
                  )}
                </td>
                <td className="mono">
                  {index.expressions.length ? index.expressions.join(", ") : "—"}
                </td>
                <td className="mono">{index.predicate ? `WHERE ${index.predicate}` : "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {details && details.triggers.length > 0 && (
        <table className="structure-table" aria-label="Triggers">
          <thead>
            <tr>
              <th>Trigger</th>
              <th>Timing</th>
              <th>Events</th>
              <th>Orientation</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {details.triggers.map((trigger) => (
              <tr key={trigger.name}>
                <td className="mono">
                  {connectionId ? (
                    <button
                      type="button"
                      className="structure-link-button"
                      onClick={() =>
                        void useStudio.getState().openObject(
                          connectionId,
                          {
                            schema: details.object.schema,
                            name: trigger.name,
                            kind: "trigger",
                            identity: trigger.tableName,
                          },
                          "structure",
                        )
                      }
                    >
                      {trigger.name}
                    </button>
                  ) : (
                    trigger.name
                  )}
                </td>
                <td>{trigger.timing}</td>
                <td>{trigger.events.join(" OR ")}</td>
                <td>{`FOR EACH ${trigger.orientation}`}</td>
                <td>
                  <span className={`state-pill ${trigger.enabled ? "active" : ""}`}>
                    {trigger.enabled ? "Enabled" : "Disabled"}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {details && (
        <div className="structure-dependencies">
          <DependencySection
            connectionId={connectionId}
            uses={details.uses}
            usedBy={details.usedBy}
          />
        </div>
      )}
    </div>
  );
}
