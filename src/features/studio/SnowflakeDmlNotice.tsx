import { useId } from "react";
import type { SnowflakeDmlEvidence, SnowflakeDmlResult } from "../../ipc/generated";

export function formatSnowflakeDmlEvidence(evidence: SnowflakeDmlEvidence): string[] {
  const summaryAvailable =
    evidence.summary.status === "validated" && evidence.summary.affectedRows !== null;
  const affectedRows = summaryAvailable ? evidence.summary.affectedRows : "unavailable";
  const multiJoinedRows =
    evidence.kind === "update" && evidence.summary.status === "validated"
      ? evidence.summary.multiJoinedRows
      : null;
  return [
    `Affected rows: ${affectedRows}${summaryAvailable ? "" : ". Affected-row summary is unavailable."}`,
    ...(multiJoinedRows === null ? [] : [`Multi-joined rows updated: ${multiJoinedRows}`]),
    "Statement acknowledgement does not prove committed effects.",
    ...(evidence.retired
      ? ["The SDK session was retired. Reconnect before running another statement."]
      : []),
  ];
}

export function SnowflakeDmlHistorySummary({ evidence }: { evidence: SnowflakeDmlEvidence }) {
  const outcomeLabel =
    evidence.statementOutcome === "succeeded"
      ? "statement succeeded"
      : evidence.statementOutcome === "rejected"
        ? "statement was rejected"
        : "statement outcome is unknown";

  return (
    <span className="snowflake-dml-history-summary">
      <strong>
        Snowflake {evidence.kind.toUpperCase()} · {outcomeLabel}
      </strong>
      <br />
      {formatSnowflakeDmlEvidence(evidence).map((line) => (
        <span key={line}>
          {line}
          <br />
        </span>
      ))}
      {evidence.statementOutcome === "unknown" && (
        <span>The statement may have been submitted. Joinery will not retry it automatically.</span>
      )}
    </span>
  );
}

export function SnowflakeDmlPendingControl({ onDisconnect }: { onDisconnect(): void }) {
  return (
    <>
      <button type="button" className="button cancel-query" onClick={onDisconnect}>
        Disconnect to stop write
      </button>
      <span role="status">
        Disconnecting may not stop a statement already submitted. Its outcome may be unknown;
        Joinery will not retry it.
      </span>
    </>
  );
}

export function SnowflakeDmlNotice({
  result,
  hasPriorResults,
}: {
  result: SnowflakeDmlResult;
  hasPriorResults?: boolean;
}) {
  const titleId = useId();
  const outcomeLabel =
    result.statementOutcome === "succeeded"
      ? "Statement succeeded"
      : result.statementOutcome === "rejected"
        ? "Statement was rejected"
        : "Statement outcome is unknown";
  const evidenceLines = formatSnowflakeDmlEvidence(result);

  return (
    <section
      className="snowflake-dml-notice"
      aria-labelledby={titleId}
      role={result.statementOutcome === "succeeded" ? "status" : "alert"}
    >
      <h2 id={titleId}>Snowflake {result.kind.toUpperCase()} outcome</h2>
      <p>{outcomeLabel}</p>
      {result.statementOutcome === "unknown" && (
        <p>The statement may have been submitted. Joinery will not retry it automatically.</p>
      )}
      {evidenceLines.map((line) => (
        <p key={line}>{line}</p>
      ))}
      {hasPriorResults && (
        <p>Results shown below are from before this statement and do not reflect its effects.</p>
      )}
      {result.state.transactionControlAuthority?.status === "rollbackOnly" && (
        <p>Only explicit rollback recovery is available for this session.</p>
      )}
      {result.error && <p role="alert">Statement detail: {result.error.message}</p>}
    </section>
  );
}
