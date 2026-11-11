import type { SnowflakeDdlEvidence, SnowflakeDdlResult } from "../../ipc/generated";

export function SnowflakeDdlHistorySummary({ evidence }: { evidence: SnowflakeDdlEvidence }) {
  return (
    <span className="snowflake-dml-history-summary">
      <strong>
        Snowflake{" "}
        {evidence.kind === "createTable"
          ? "CREATE TABLE"
          : evidence.kind === "addColumn"
            ? "ADD COLUMN"
            : "DROP TABLE"}{" "}
        · {evidence.qualifiedTarget}
      </strong>
      <br />
      {evidence.statementOutcome === "succeeded"
        ? "Statement acknowledged"
        : evidence.statementOutcome === "rejected"
          ? "Statement rejected"
          : "Statement outcome unknown"}
      .<br />
      DDL cannot be rolled back.{" "}
      {evidence.summaryStatus === "unavailable" && "Status summary unavailable. "}
      {evidence.catalogReconciliationRequired && "Explicit catalog reconciliation required. "}
      {evidence.retired && "Session retired; reconnect before further work. "}
      {evidence.statementOutcome === "unknown" &&
        "The statement may have executed; inspect current state before any new review. No automatic retry."}
    </span>
  );
}

export function SnowflakeDdlNotice({
  result,
  hasPriorResults,
}: {
  result: SnowflakeDdlResult;
  hasPriorResults?: boolean;
}) {
  return (
    <section
      className="snowflake-dml-notice"
      aria-label="Snowflake schema change outcome"
      role={result.statementOutcome === "succeeded" ? "status" : "alert"}
    >
      <h2>Snowflake schema change outcome</h2>
      <SnowflakeDdlHistorySummary evidence={result} />
      {result.error && <p>Statement detail: {result.error.message}</p>}
      {hasPriorResults && (
        <p>
          Results shown below are from before this schema change and do not reflect its effects.
        </p>
      )}
    </section>
  );
}
