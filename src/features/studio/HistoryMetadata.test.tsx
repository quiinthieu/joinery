import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import {
  HistoryExecutionSummary,
  historyDurationText,
  historyEvidenceText,
  historyExecutionText,
  type HistoryMetadata,
} from "./HistoryMetadata";

afterEach(cleanup);

test("legacy missing provenance preserves a recorded outcome and measured zero", () => {
  const legacy: HistoryMetadata = { status: "completed", elapsedMs: 0 };
  render(<HistoryExecutionSummary entry={legacy} />);
  expect(screen.getByRole("status").textContent).toContain("Completed · Statement counts unknown");
  expect(historyDurationText(legacy)).toBe("0 ms");
  expect(historyExecutionText(legacy)).not.toMatch(/Admitted|Recovered|Completion unknown/);
});

test("admission explains uncertainty without inventing running or completed status", () => {
  const entry: HistoryMetadata = { provenance: "admitted", status: null, elapsedMs: null };
  render(<HistoryExecutionSummary entry={entry} />);
  const status = screen.getByRole("status");
  expect(status.querySelector("p")?.textContent).toBe("Admitted; completion pending");
  expect(status.textContent).toContain("Admission does not prove dispatch or server submission");
  expect(historyDurationText(entry)).toBe("Duration unavailable");
});

test.each([
  ["interrupted", "Recovered after interruption"],
  ["restoreRecovered", "Recovered from metadata restore"],
] as const)("%s with no original terminal stays unknown", (provenance, label) => {
  const entry: HistoryMetadata = { provenance, status: null, elapsedMs: null };
  render(<HistoryExecutionSummary entry={entry} />);
  expect(screen.getByRole("status").querySelector("p")?.textContent).toBe(
    `${label} · Completion unknown`,
  );
  expect(historyEvidenceText(entry)).toContain("Local recovery did not execute SQL");
  expect(historyDurationText(entry)).toBe("Duration unavailable");
});

test.each(["interrupted", "restoreRecovered"] as const)(
  "%s preserves an original terminal and exact duration",
  (provenance) => {
    const entry: HistoryMetadata = { provenance, status: "completed", elapsedMs: 17 };
    expect(historyExecutionText(entry)).toContain("Completed · Statement counts unknown");
    expect(historyDurationText(entry)).toBe("17 ms");
    expect(historyEvidenceText(entry)).toContain("Original recorded completion is preserved");
  },
);

test.each(["recorded", "interrupted"] as const)(
  "%s known failure with no measured duration retains failure without a false zero",
  (provenance) => {
    const entry: HistoryMetadata = { provenance, status: "failed", elapsedMs: null };
    expect(historyExecutionText(entry)).toContain("Failed · Statement counts unknown");
    expect(historyDurationText(entry)).toBe("Duration unavailable");
  },
);

test("a malformed legacy missing outcome is rendered as unknown instead of fabricated success", () => {
  const entry: HistoryMetadata = { status: null, elapsedMs: null };
  expect(historyExecutionText(entry)).toBe("Completion unknown");
  expect(historyDurationText(entry)).toBe("Duration unavailable");
});
