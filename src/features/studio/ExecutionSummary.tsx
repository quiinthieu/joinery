import type { ExecutionSummary as Summary, RunStatus } from "../../ipc/generated";

type SummaryStatus = RunStatus | "idle" | "queued" | "running";

function statusLabel(status: SummaryStatus) {
  switch (status) {
    case "completed":
      return "Completed";
    case "stopped":
      return "Stopped";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "outcomeUnknown":
      return "Outcome unknown";
    case "queued":
      return "Queued";
    case "running":
      return "Running";
    case "idle":
      return "Not run";
  }
}

export function executionSummaryText(summary: Summary | null | undefined, status: SummaryStatus) {
  const outcome = statusLabel(status);
  if (!summary) return `${outcome} · Statement counts unknown`;
  const counts = `Planned: ${summary.planned} · Attempted: ${summary.attempted} · Completed: ${summary.completed} · Not attempted: ${summary.notAttempted}`;
  const reason =
    summary.stopReason === "resultLimit"
      ? "Result retention limit reached"
      : summary.stopReason === "cancelled"
        ? "Execution cancelled"
        : summary.stopReason === "failed"
          ? "Stopped after an error"
          : summary.stopReason === "outcomeUnknown"
            ? "Statement outcome unknown"
            : null;
  const suffix =
    summary.notAttempted === 0
      ? null
      : summary.notAttempted === 1
        ? `Statement ${summary.planned} was not attempted`
        : `Statements ${summary.attempted + 1}–${summary.planned} were not attempted`;
  return [outcome, counts, reason, suffix].filter(Boolean).join(" · ");
}

export function ExecutionSummary({
  summary,
  status,
}: {
  summary?: Summary | null;
  status: SummaryStatus;
}) {
  return (
    <div className="execution-summary" role="status" aria-label="Statement execution summary">
      <p>{executionSummaryText(summary, status)}</p>
      <small>
        {summary
          ? "An attempted statement may not have reached the server. Completed statements may still be uncommitted, and results may be truncated."
          : "Statement counts were not recorded. Planned, attempted, completed, and not attempted counts are unknown."}
      </small>
    </div>
  );
}
