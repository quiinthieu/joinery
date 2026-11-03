import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Copy, ExternalLink, ShieldCheck, Wrench } from "lucide-react";
import { Modal } from "../../components/ui/Modal";
import { client } from "../../ipc/client";
import type {
  PreparedPlan,
  TableMaintenanceAction,
  TableMaintenancePreview,
  TableRef,
  TableStorageStats,
} from "../../ipc/generated";
import { useStudio } from "../../state/studio";
import { ReviewDialog } from "../safety/ReviewDialog";

export interface TableMaintenanceDialogProps {
  connectionId: string;
  table: TableRef;
  columns?: readonly string[];
  storage?: TableStorageStats | null;
  onClose: () => void;
  onCompleted?: (action: TableMaintenanceAction) => void;
}

type PostgresMaintenanceKind = "analyze" | "vacuum" | "reindexTable";
type MysqlMaintenanceKind = "analyzeTable" | "optimizeTable" | "checkTable";

export function TableMaintenanceDialog({
  connectionId,
  table,
  columns = [],
  storage = null,
  onClose,
  onCompleted,
}: TableMaintenanceDialogProps) {
  const profile = useStudio(
    (s) => s.bootstrap?.profiles.find((p) => p.id === connectionId) ?? null,
  );
  const session = useStudio((s) => s.sessions[connectionId] ?? null);
  const sessionState = useStudio((s) => (session ? s.sessionStates[session.id] : undefined));
  const notify = useStudio((s) => s.notify);
  const openQuery = useStudio((s) => s.openQuery);
  const refreshCatalog = useStudio((s) => s.refreshCatalog);

  const engine = profile?.engine ?? session?.engine ?? "postgres";
  const isPostgres = engine === "postgres";
  const isReadOnly = profile?.safeMode === "readOnly";
  const hasOpenTransaction = sessionState !== undefined && sessionState.transactionState !== "idle";

  const initialPgKind: PostgresMaintenanceKind =
    !storage?.statsStale && (storage?.deadTuples ?? 0) > 0 ? "vacuum" : "analyze";
  const [pgKind, setPgKind] = useState<PostgresMaintenanceKind>(initialPgKind);
  const [mysqlKind, setMysqlKind] = useState<MysqlMaintenanceKind>("analyzeTable");

  // PostgreSQL ANALYZE options
  const [analyzeVerbose, setAnalyzeVerbose] = useState(false);
  const [selectedColumns, setSelectedColumns] = useState<string[]>([]);

  // PostgreSQL VACUUM options
  const [vacuumAnalyze, setVacuumAnalyze] = useState(true);
  const [vacuumVerbose, setVacuumVerbose] = useState(false);
  const [vacuumFull, setVacuumFull] = useState(false);

  const [preview, setPreview] = useState<TableMaintenancePreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [preparedPlan, setPreparedPlan] = useState<PreparedPlan | null>(null);

  const action = useMemo<TableMaintenanceAction>(() => {
    if (isPostgres) {
      if (pgKind === "vacuum") {
        return {
          type: "vacuum",
          table,
          analyze: vacuumAnalyze,
          verbose: vacuumVerbose,
          full: vacuumFull,
        };
      }
      if (pgKind === "reindexTable") {
        return {
          type: "reindexTable",
          table,
        };
      }
      return {
        type: "analyze",
        table,
        verbose: analyzeVerbose,
        columns: selectedColumns,
      };
    }
    if (mysqlKind === "optimizeTable") {
      return { type: "optimizeTable", table };
    }
    if (mysqlKind === "checkTable") {
      return { type: "checkTable", table };
    }
    return { type: "analyzeTable", table };
  }, [
    analyzeVerbose,
    isPostgres,
    mysqlKind,
    pgKind,
    selectedColumns,
    table,
    vacuumAnalyze,
    vacuumFull,
    vacuumVerbose,
  ]);

  useEffect(() => {
    let cancelled = false;
    void client
      .call("preview_table_maintenance", {
        engine,
        action,
      })
      .then((res) => {
        if (!cancelled) {
          setPreview(res);
          setPreviewError(null);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setPreview(null);
          setPreviewError(String((err as { message?: string })?.message ?? err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [engine, action]);

  const toggleAnalyzeColumn = (colName: string) => {
    setSelectedColumns((prev) =>
      prev.includes(colName) ? prev.filter((c) => c !== colName) : [...prev, colName],
    );
  };

  const handleCopySql = async () => {
    if (!preview) return;
    try {
      await navigator.clipboard.writeText(preview.statement.sql);
      notify("Copied maintenance SQL to clipboard.");
    } catch {
      notify("Clipboard copy failed.");
    }
  };

  const handleOpenInEditor = () => {
    if (!preview) return;
    openQuery(connectionId, preview.statement.sql);
    onClose();
  };

  const handlePrepareAndReview = async () => {
    if (!session) {
      notify("Connect this database profile before running table maintenance.");
      return;
    }
    setPreparing(true);
    try {
      const plan = await client.call("prepare_table_maintenance", {
        sessionId: session.id,
        action,
      });
      setPreparedPlan(plan);
    } catch (err) {
      notify(String((err as { message?: string })?.message ?? err));
    } finally {
      setPreparing(false);
    }
  };

  const handleExecuteApproved = async (approval: string | null) => {
    if (!preparedPlan) return;
    const outcome = await new Promise<{ status: string; errorMessage: string | null }>(
      (resolve, reject) => {
        let errorMessage: string | null = null;
        client
          .execute(preparedPlan.id, approval ?? "", (event) => {
            if (event.type === "result") {
              void client.call("release_result", { resultId: event.result.id }).catch(() => {});
            } else if (event.type === "error") {
              errorMessage = event.error.message;
            } else if (event.type === "finished") {
              resolve({ status: event.status, errorMessage });
            }
          })
          .catch(reject);
      },
    );
    setPreparedPlan(null);
    if (outcome.errorMessage) {
      notify(outcome.errorMessage);
      return;
    }
    if (outcome.status === "completed") {
      await refreshCatalog(connectionId);
      onCompleted?.(action);
      notify(`${preview?.statement.description ?? "Table maintenance"} completed.`);
      onClose();
    }
  };

  if (preparedPlan && profile) {
    return (
      <ReviewDialog
        key={preparedPlan.id}
        plan={preparedPlan}
        profile={profile}
        onExecute={handleExecuteApproved}
        onClose={() => setPreparedPlan(null)}
      />
    );
  }

  const nonTransactionalBlocked = Boolean(preview?.nonTransactional && hasOpenTransaction);

  return (
    <Modal
      title={`Table Maintenance — ${table.schema}.${table.name}`}
      onClose={onClose}
      wide
      busy={preparing}
    >
      <div className="modal-form">
        <p className="muted">
          Prepare a reviewed {isPostgres ? "PostgreSQL" : "MySQL / MariaDB"} maintenance operation
          on{" "}
          <code>
            {table.schema}.{table.name}
          </code>
          . Nothing runs until you review and authorize the generated statement.
        </p>

        {storage && (storage.statsStale || (storage.deadTuples ?? 0) > 0) && (
          <div className="warning-list" role="note" aria-label="Current storage health">
            {storage.statsStale && (
              <div className="warning-item">
                <Wrench size={14} />
                <span>
                  Planner statistics are currently marked stale or unanalyzed (last analyze:{" "}
                  {storage.lastAnalyze ?? "never"}).
                </span>
              </div>
            )}
            {(storage.deadTuples ?? 0) > 0 && (
              <div className="warning-item">
                <Wrench size={14} />
                <span>
                  {storage.deadTuples?.toLocaleString()} dead tuples reported (last vacuum:{" "}
                  {storage.lastVacuum ?? "never"}).
                </span>
              </div>
            )}
          </div>
        )}

        {isPostgres ? (
          <>
            <div className="segmented" role="radiogroup" aria-label="PostgreSQL maintenance action">
              <button
                type="button"
                role="radio"
                aria-checked={pgKind === "analyze"}
                className={pgKind === "analyze" ? "active" : ""}
                onClick={() => setPgKind("analyze")}
              >
                ANALYZE
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={pgKind === "vacuum"}
                className={pgKind === "vacuum" ? "active" : ""}
                onClick={() => setPgKind("vacuum")}
              >
                VACUUM
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={pgKind === "reindexTable"}
                className={pgKind === "reindexTable" ? "active" : ""}
                onClick={() => setPgKind("reindexTable")}
              >
                REINDEX TABLE
              </button>
            </div>

            {pgKind === "analyze" && (
              <>
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    aria-label="Include VERBOSE output"
                    checked={analyzeVerbose}
                    onChange={(e) => setAnalyzeVerbose(e.target.checked)}
                  />
                  <span>Include VERBOSE progress output</span>
                </label>

                {columns.length > 0 && (
                  <div className="maintenance-columns-box">
                    <div className="structure-section-header">
                      <strong>
                        Target columns (
                        {selectedColumns.length === 0
                          ? "all columns"
                          : `${selectedColumns.length} selected`}
                        )
                      </strong>
                      {selectedColumns.length > 0 && (
                        <button
                          type="button"
                          className="ghost-button small"
                          onClick={() => setSelectedColumns([])}
                        >
                          All columns
                        </button>
                      )}
                    </div>
                    <div className="ddl-column-chips" role="group" aria-label="ANALYZE columns">
                      {columns.map((col) => {
                        const checked = selectedColumns.includes(col);
                        return (
                          <label key={col} className="checkbox-row">
                            <input
                              type="checkbox"
                              aria-label={`Analyze column ${col}`}
                              checked={checked}
                              onChange={() => toggleAnalyzeColumn(col)}
                            />
                            <span className="mono">{col}</span>
                          </label>
                        );
                      })}
                    </div>
                  </div>
                )}
              </>
            )}

            {pgKind === "vacuum" && (
              <>
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    aria-label="Update planner statistics (ANALYZE)"
                    checked={vacuumAnalyze}
                    onChange={(e) => setVacuumAnalyze(e.target.checked)}
                  />
                  <span>Update planner statistics (ANALYZE)</span>
                </label>
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    aria-label="Include VERBOSE output"
                    checked={vacuumVerbose}
                    onChange={(e) => setVacuumVerbose(e.target.checked)}
                  />
                  <span>Include VERBOSE progress output</span>
                </label>
                <label className="checkbox-row">
                  <input
                    type="checkbox"
                    aria-label="VACUUM FULL (rewrite table and lock exclusively)"
                    checked={vacuumFull}
                    onChange={(e) => setVacuumFull(e.target.checked)}
                  />
                  <span>
                    VACUUM FULL (rewrite table onto new disk pages; holds ACCESS EXCLUSIVE lock)
                  </span>
                </label>
              </>
            )}
          </>
        ) : (
          <div
            className="segmented"
            role="radiogroup"
            aria-label="MySQL and MariaDB maintenance action"
          >
            <button
              type="button"
              role="radio"
              aria-checked={mysqlKind === "analyzeTable"}
              className={mysqlKind === "analyzeTable" ? "active" : ""}
              onClick={() => setMysqlKind("analyzeTable")}
            >
              ANALYZE TABLE
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={mysqlKind === "optimizeTable"}
              className={mysqlKind === "optimizeTable" ? "active" : ""}
              onClick={() => setMysqlKind("optimizeTable")}
            >
              OPTIMIZE TABLE
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={mysqlKind === "checkTable"}
              className={mysqlKind === "checkTable" ? "active" : ""}
              onClick={() => setMysqlKind("checkTable")}
            >
              CHECK TABLE
            </button>
          </div>
        )}

        {nonTransactionalBlocked && (
          <div className="error-banner" role="alert">
            <AlertTriangle size={14} />
            <span>
              This connection currently has an open manual transaction. Commit or roll back the
              transaction before running VACUUM.
            </span>
          </div>
        )}

        {preview && preview.warnings.length > 0 && (
          <div className="warning-list" role="status">
            {preview.warnings.map((w) => (
              <div className="warning-item" key={w}>
                <AlertTriangle size={14} />
                <span>{w}</span>
              </div>
            ))}
          </div>
        )}

        {previewError && (
          <div className="error-banner" role="alert">
            <AlertTriangle size={14} />
            <span>{previewError}</span>
          </div>
        )}

        {preview && (
          <div className="ddl-preview-panel">
            <div className="structure-section-header">
              <strong>Generated maintenance SQL</strong>
              {preview.statement.destructive && <span className="pill danger">Exclusive lock</span>}
              {preview.nonTransactional && <span className="pill">Non-transactional</span>}
            </div>
            <pre className="sql-preview" aria-label="Generated maintenance SQL preview">
              {preview.statement.sql}
            </pre>
          </div>
        )}

        <div className="modal-actions">
          <button type="button" className="ghost-button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="ghost-button"
            disabled={!preview}
            onClick={() => void handleCopySql()}
          >
            <Copy size={13} />
            Copy SQL
          </button>
          <button
            type="button"
            className="ghost-button"
            disabled={!preview}
            onClick={handleOpenInEditor}
          >
            <ExternalLink size={13} />
            Open in SQL editor
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={!preview || !session || isReadOnly || preparing || nonTransactionalBlocked}
            onClick={() => void handlePrepareAndReview()}
          >
            <ShieldCheck size={14} />
            {preparing ? "Preparing…" : "Review & authorize…"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
