import { useCallback, useEffect, useRef, useState } from "react";
import { Activity, AlertTriangle, Gauge, RefreshCw, Search, X } from "lucide-react";
import type { SnowflakeOperatorProfile, SnowflakeQueryHistoryEntry } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { revokeSchemaSelectionBeforeWork } from "../../state/studio";
import { formatStatCount, formatStorageBytes } from "./storage-stats";

function formatDurationMs(value: number | null): string {
  if (value === null || value === undefined) return "—";
  if (value < 1000) return `${value} ms`;
  return `${(value / 1000).toFixed(2)} s`;
}

function formatRatioPct(ratio: number | null): string {
  if (ratio === null || ratio === undefined || !Number.isFinite(ratio)) return "—";
  const normalized = ratio <= 1 && ratio >= 0 ? ratio * 100 : ratio;
  return `${normalized.toFixed(1)}%`;
}

function formatPruning(scanned: number | null, total: number | null): string {
  if (scanned === null && total === null) return "—";
  return `${formatStatCount(scanned)} / ${formatStatCount(total)}`;
}

export function SnowflakeQueryProfileModal({
  sessionId,
  initialQueryId = null,
  onClose,
}: {
  sessionId: string;
  initialQueryId?: string | null;
  onClose: () => void;
}) {
  const [limit, setLimit] = useState<number>(25);
  const [history, setHistory] = useState<SnowflakeQueryHistoryEntry[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);

  const [selectedQueryId, setSelectedQueryId] = useState<string>(initialQueryId ?? "");
  const [queryIdInput, setQueryIdInput] = useState<string>(initialQueryId ?? "");
  const [profile, setProfile] = useState<SnowflakeOperatorProfile | null>(null);
  const [profileLoading, setProfileLoading] = useState(false);
  const [profileError, setProfileError] = useState<string | null>(null);

  const historySeqRef = useRef(0);
  const profileSeqRef = useRef(0);
  const selectedQueryIdRef = useRef(selectedQueryId);
  selectedQueryIdRef.current = selectedQueryId;

  const loadOperatorProfile = useCallback(
    async (queryId: string) => {
      const trimmed = queryId.trim();
      if (!trimmed) return;
      const reqId = ++profileSeqRef.current;
      selectedQueryIdRef.current = trimmed;
      setSelectedQueryId(trimmed);
      setQueryIdInput(trimmed);
      setProfileLoading(true);
      setProfileError(null);
      try {
        revokeSchemaSelectionBeforeWork(sessionId);
        const result = await client.call("snowflake_query_operator_profile", {
          request: { sessionId, queryId: trimmed },
        });
        if (profileSeqRef.current !== reqId) return;
        setProfile(result);
      } catch (err) {
        if (profileSeqRef.current !== reqId) return;
        setProfile(null);
        setProfileError(messageOf(err));
      } finally {
        if (profileSeqRef.current === reqId) {
          setProfileLoading(false);
        }
      }
    },
    [sessionId],
  );

  const loadHistory = useCallback(
    async (nextLimit: number) => {
      const reqId = ++historySeqRef.current;
      setHistoryLoading(true);
      setHistoryError(null);
      try {
        revokeSchemaSelectionBeforeWork(sessionId);
        const entries = await client.call("snowflake_query_history", {
          request: { sessionId, limit: nextLimit },
        });
        if (historySeqRef.current !== reqId) return;
        setHistory(entries);
        if (!selectedQueryIdRef.current && entries.length > 0) {
          void loadOperatorProfile(entries[0].queryId);
        }
      } catch (err) {
        if (historySeqRef.current !== reqId) return;
        setHistory([]);
        setHistoryError(messageOf(err));
      } finally {
        if (historySeqRef.current === reqId) {
          setHistoryLoading(false);
        }
      }
    },
    [loadOperatorProfile, sessionId],
  );

  useEffect(() => {
    void loadHistory(limit);
  }, [loadHistory, limit]);

  useEffect(() => {
    if (initialQueryId) {
      void loadOperatorProfile(initialQueryId);
    }
  }, [initialQueryId, loadOperatorProfile]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      historySeqRef.current += 1;
      profileSeqRef.current += 1;
    };
  }, [onClose]);

  const selectedEntry = history.find((item) => item.queryId === selectedQueryId) ?? null;
  const isBottleneckOp = (op: SnowflakeOperatorProfile["operators"][number]) =>
    profile !== null &&
    profile.bottleneckOperatorId !== null &&
    op.operatorId === profile.bottleneckOperatorId &&
    (profile.bottleneckStepId === undefined ||
      profile.bottleneckStepId === null ||
      op.stepId === profile.bottleneckStepId);
  const bottleneckOp = profile ? (profile.operators.find(isBottleneckOp) ?? null) : null;

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="modal-card wide"
        role="dialog"
        aria-modal="true"
        aria-label="Snowflake query history and operator profile"
        style={{
          maxWidth: 1080,
          width: "94vw",
          maxHeight: "88vh",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div className="modal-header">
          <div>
            <span className="eyebrow">SNOWFLAKE TELEMETRY · ORDINARY SESSION</span>
            <h2>
              <Gauge size={16} style={{ marginRight: 6, verticalAlign: "text-bottom" }} />
              Query History &amp; Operator Profile
            </h2>
          </div>
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
              <span>Limit</span>
              <select
                aria-label="History limit"
                value={limit}
                onChange={(e) => setLimit(Number(e.target.value))}
              >
                <option value={25}>25</option>
                <option value={50}>50</option>
                <option value={100}>100</option>
              </select>
            </label>
            <button
              type="button"
              className="button ghost small"
              disabled={historyLoading}
              onClick={() => void loadHistory(limit)}
            >
              <RefreshCw size={12} />
              Refresh
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label="Close query profile"
              onClick={onClose}
            >
              <X size={15} />
            </button>
          </div>
        </div>

        <div className="modal-body" style={{ overflowY: "auto", display: "grid", gap: 16 }}>
          <form
            style={{ display: "flex", gap: 8, alignItems: "center" }}
            onSubmit={(e) => {
              e.preventDefault();
              void loadOperatorProfile(queryIdInput);
            }}
          >
            <input
              type="text"
              className="mono"
              aria-label="Inspect Snowflake query ID"
              placeholder="Paste Snowflake query ID (e.g. 01b4a9c0-0001-9f12-0000-00420001a111)"
              value={queryIdInput}
              onChange={(e) => setQueryIdInput(e.target.value)}
              style={{ flex: 1 }}
            />
            <button
              type="submit"
              className="button primary small"
              disabled={profileLoading || !queryIdInput.trim()}
            >
              <Search size={12} />
              Inspect profile
            </button>
          </form>

          <section aria-label="Session query history">
            <h3 className="object-section-title">Session QUERY_HISTORY ({history.length})</h3>
            {historyError && (
              <div className="inline-error" role="alert">
                {historyError}
              </div>
            )}
            {historyLoading && history.length === 0 ? (
              <div className="grid-empty">Loading session query history…</div>
            ) : history.length === 0 && !historyError ? (
              <div className="grid-empty">
                No recent Snowflake queries recorded on this session.
              </div>
            ) : (
              <div style={{ maxHeight: 220, overflowY: "auto" }}>
                <table className="structure-table" aria-label="Snowflake query history">
                  <thead>
                    <tr>
                      <th>Query ID</th>
                      <th>Status</th>
                      <th>Elapsed</th>
                      <th>Warehouse</th>
                      <th>Partitions</th>
                      <th>Cache</th>
                      <th>Scanned</th>
                      <th>Spill</th>
                      <th>SQL</th>
                    </tr>
                  </thead>
                  <tbody>
                    {history.map((entry) => {
                      const hasSpill =
                        (entry.bytesSpilledLocal ?? 0) > 0 || (entry.bytesSpilledRemote ?? 0) > 0;
                      const isSelected = entry.queryId === selectedQueryId;
                      return (
                        <tr
                          key={entry.queryId}
                          aria-selected={isSelected}
                          style={{
                            cursor: "pointer",
                            background: isSelected
                              ? "var(--surface-selected, rgba(59,130,246,0.12))"
                              : undefined,
                          }}
                          onClick={() => void loadOperatorProfile(entry.queryId)}
                        >
                          <td className="mono">
                            <button
                              type="button"
                              className="button ghost small mono"
                              onClick={(e) => {
                                e.stopPropagation();
                                void loadOperatorProfile(entry.queryId);
                              }}
                            >
                              {entry.queryId}
                            </button>
                          </td>
                          <td>
                            <span
                              className={`state-pill ${
                                entry.executionStatus.toUpperCase() === "SUCCESS" ? "active" : ""
                              }`}
                            >
                              {entry.executionStatus}
                            </span>
                          </td>
                          <td
                            className="mono"
                            title={`Compile ${formatDurationMs(entry.compilationMs)} · Execute ${formatDurationMs(entry.executionMs)} · Queue ${formatDurationMs(entry.queuedMs)}`}
                          >
                            {formatDurationMs(entry.totalElapsedMs)}
                          </td>
                          <td className="mono">
                            {entry.warehouseName
                              ? `${entry.warehouseName}${entry.warehouseSize ? ` (${entry.warehouseSize})` : ""}`
                              : "—"}
                          </td>
                          <td className="mono">
                            {formatPruning(entry.partitionsScanned, entry.partitionsTotal)}
                          </td>
                          <td className="mono">{formatRatioPct(entry.cacheHitRatio)}</td>
                          <td className="mono">{formatStorageBytes(entry.bytesScanned)}</td>
                          <td className="mono">
                            {hasSpill ? (
                              <span className="lock-badge waiting">
                                <AlertTriangle size={11} />{" "}
                                {formatStorageBytes(
                                  (entry.bytesSpilledLocal ?? 0) + (entry.bytesSpilledRemote ?? 0),
                                )}
                              </span>
                            ) : (
                              "—"
                            )}
                          </td>
                          <td
                            className="mono"
                            style={{
                              maxWidth: 260,
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                            }}
                            title={entry.queryText}
                          >
                            {entry.queryText}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          {selectedEntry && (
            <dl className="object-meta-grid" aria-label="Selected query telemetry">
              <div>
                <dt>Total / Compile / Execute</dt>
                <dd className="mono">
                  {`${formatDurationMs(selectedEntry.totalElapsedMs)} · ${formatDurationMs(selectedEntry.compilationMs)} · ${formatDurationMs(selectedEntry.executionMs)}`}
                </dd>
              </div>
              <div>
                <dt>Queue wait</dt>
                <dd className="mono">{formatDurationMs(selectedEntry.queuedMs)}</dd>
              </div>
              <div>
                <dt>Rows produced</dt>
                <dd className="mono">{formatStatCount(selectedEntry.rowsProduced)}</dd>
              </div>
              <div>
                <dt>Context</dt>
                <dd className="mono">
                  {[selectedEntry.databaseName, selectedEntry.schemaName]
                    .filter(Boolean)
                    .join(".") || "—"}
                </dd>
              </div>
              {selectedEntry.errorMessage && (
                <div>
                  <dt>Error</dt>
                  <dd className="mono">{selectedEntry.errorMessage}</dd>
                </div>
              )}
            </dl>
          )}

          <section aria-label="Operator execution profile">
            <h3 className="object-section-title">
              <Activity size={13} style={{ marginRight: 6, verticalAlign: "text-bottom" }} />
              Operator Profile {selectedQueryId ? `(${selectedQueryId})` : ""}
            </h3>
            {profileError && (
              <div className="inline-error" role="alert">
                {profileError}
              </div>
            )}
            {profileLoading ? (
              <div className="grid-empty">Loading GET_QUERY_OPERATOR_STATS…</div>
            ) : !profile ? (
              <div className="grid-empty">
                Select a query above or enter a Snowflake query ID to inspect per-operator execution
                statistics.
              </div>
            ) : (
              <>
                <dl className="object-meta-grid" aria-label="Operator profile summary">
                  <div>
                    <dt>Operators</dt>
                    <dd className="mono">{profile.operators.length}</dd>
                  </div>
                  <div>
                    <dt>Bottleneck operator</dt>
                    <dd className="mono">
                      {bottleneckOp
                        ? `#${bottleneckOp.operatorId} ${bottleneckOp.operatorType} (${formatRatioPct(bottleneckOp.overallPercentage)})`
                        : "—"}
                    </dd>
                  </div>
                  <div>
                    <dt>Local disk spill</dt>
                    <dd className="mono">
                      {profile.totalSpilledLocalBytes > 0
                        ? formatStorageBytes(profile.totalSpilledLocalBytes)
                        : "0 B"}
                    </dd>
                  </div>
                  <div>
                    <dt>Remote storage spill</dt>
                    <dd className="mono">
                      {profile.totalSpilledRemoteBytes > 0
                        ? formatStorageBytes(profile.totalSpilledRemoteBytes)
                        : "0 B"}
                    </dd>
                  </div>
                </dl>

                {profile.operators.length === 0 ? (
                  <div className="grid-empty">
                    No operator stats returned for this query (cached metadata result or DDL
                    statement).
                  </div>
                ) : (
                  <table className="structure-table" aria-label="Operator statistics">
                    <thead>
                      <tr>
                        <th>Step / Op</th>
                        <th>Operator</th>
                        <th>Time %</th>
                        <th>Rows (in → out)</th>
                        <th>Partitions</th>
                        <th>Cache</th>
                        <th>Spill</th>
                        <th>Detail</th>
                      </tr>
                    </thead>
                    <tbody>
                      {profile.operators.map((op) => {
                        const isBottleneck = isBottleneckOp(op);
                        const spillTotal =
                          (op.bytesSpilledLocal ?? 0) + (op.bytesSpilledRemote ?? 0);
                        const pctValue =
                          op.overallPercentage !== null && op.overallPercentage !== undefined
                            ? op.overallPercentage <= 1 && op.overallPercentage >= 0
                              ? op.overallPercentage * 100
                              : op.overallPercentage
                            : 0;
                        return (
                          <tr
                            key={`${op.stepId}:${op.operatorId}`}
                            data-testid={
                              op.stepId === 1
                                ? `snowflake-operator-${op.operatorId}`
                                : `snowflake-operator-${op.stepId}-${op.operatorId}`
                            }
                          >
                            <td className="mono">{`${op.stepId}.${op.operatorId}`}</td>
                            <td className="mono">
                              {op.operatorType}
                              {isBottleneck && (
                                <>
                                  {" "}
                                  <span className="lock-badge blocker">Bottleneck</span>
                                </>
                              )}
                              {op.parentOperators.length > 0 && (
                                <small style={{ display: "block", opacity: 0.75 }}>
                                  {`← op ${op.parentOperators.join(", ")}`}
                                </small>
                              )}
                            </td>
                            <td className="mono">
                              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                                <span>{formatRatioPct(op.overallPercentage)}</span>
                                <div
                                  aria-hidden="true"
                                  style={{
                                    width: 56,
                                    height: 6,
                                    borderRadius: 3,
                                    background: "rgba(148,163,184,0.22)",
                                    overflow: "hidden",
                                  }}
                                >
                                  <div
                                    style={{
                                      width: `${Math.min(100, Math.max(0, pctValue))}%`,
                                      height: "100%",
                                      background: isBottleneck ? "#ef4444" : "#3b82f6",
                                    }}
                                  />
                                </div>
                              </div>
                            </td>
                            <td className="mono">
                              {`${formatStatCount(op.inputRows)} → ${formatStatCount(op.outputRows)}`}
                            </td>
                            <td className="mono">
                              {formatPruning(op.partitionsScanned, op.partitionsTotal)}
                            </td>
                            <td className="mono">{formatRatioPct(op.cacheHitRatio)}</td>
                            <td className="mono">
                              {spillTotal > 0 ? (
                                <span className="lock-badge waiting">
                                  {formatStorageBytes(spillTotal)}
                                </span>
                              ) : (
                                "—"
                              )}
                            </td>
                            <td className="mono">{op.detail ?? "—"}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
