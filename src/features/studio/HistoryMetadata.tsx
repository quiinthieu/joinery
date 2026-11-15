import type { HistorySummary } from "../../ipc/generated";
import { executionSummaryText } from "./ExecutionSummary";

export type HistoryMetadata = Pick<HistorySummary, "status" | "elapsedMs" | "executionSummary"> & {
  provenance?: HistorySummary["provenance"];
};

export function historyDurationText(entry: HistoryMetadata): string {
  return entry.elapsedMs == null ? "Duration unavailable" : `${entry.elapsedMs} ms`;
}

export function historyExecutionText(entry: HistoryMetadata): string {
  // Older local records did not carry provenance. This default never invents
  // a missing terminal status or duration.
  const provenance = entry.provenance ?? "recorded";
  if (provenance === "admitted") return "Admitted; completion pending";
  const completion =
    entry.status == null
      ? "Completion unknown"
      : executionSummaryText(entry.executionSummary, entry.status);
  if (provenance === "interrupted") return `Recovered after interruption · ${completion}`;
  if (provenance === "restoreRecovered") return `Recovered from metadata restore · ${completion}`;
  return completion;
}

export function historyEvidenceText(entry: HistoryMetadata): string {
  const provenance = entry.provenance ?? "recorded";
  if (provenance === "admitted")
    return "Admission does not prove dispatch or server submission. History does not resume execution.";
  if (provenance === "interrupted" || provenance === "restoreRecovered")
    return entry.status == null
      ? "Local recovery did not execute SQL. Completion was not recorded; no failure, cancellation, or rollback is inferred."
      : "Original recorded completion is preserved. Local recovery did not execute SQL and does not prove committed effects.";
  return entry.executionSummary
    ? "An attempted statement may not have reached the server. Completed statements may still be uncommitted, and results may be truncated."
    : "Statement counts were not recorded. Planned, attempted, completed, and not attempted counts are unknown.";
}

export function HistoryExecutionSummary({ entry }: { entry: HistoryMetadata }) {
  return (
    <div className="execution-summary" role="status" aria-label="History execution summary">
      <p>{historyExecutionText(entry)}</p>
      <small>{historyEvidenceText(entry)}</small>
    </div>
  );
}
