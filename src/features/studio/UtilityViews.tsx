import { useEffect, useState } from "react";
import { Activity, History, RefreshCw, Search, ArrowUpRight } from "lucide-react";
import type { HistoryEntry, ProcessInfo, TableSchema } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";

export function HistoryView() {
  const [entries, setEntries] = useState<HistoryEntry[]>([]),
    [search, setSearch] = useState(""),
    [error, setError] = useState<string | null>(null);
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
        <button
          className="button ghost"
          onClick={() => {
            setSearch("");
            void client.call<HistoryEntry[]>("history", { search: "" }).then(setEntries);
          }}
        >
          <RefreshCw size={14} />
          Refresh
        </button>
      </header>
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
                  useStudio.getState().notify("Only execution metadata was saved for this query.");
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
    </div>
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
  return (
    <div className="utility-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">INSTANCE ACTIVITY</span>
          <h1>Processes</h1>
          <p>Inspect active sessions and cancel long-running queries.</p>
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
                    {process.state === "active" && (
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

export function StructureView({ schema }: { schema: TableSchema | null }) {
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
            const fk = schema.foreignKeys.find((key) => key.column === column.name);
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
                <td className="mono">
                  {fk ? `${fk.target.schema}.${fk.target.name}.${fk.targetColumn}` : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
