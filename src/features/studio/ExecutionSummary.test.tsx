import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { ExecutionSummary as Summary } from "../../ipc/generated";
import { ExecutionSummary, executionSummaryText } from "./ExecutionSummary";

afterEach(cleanup);
const limited: Summary = {
  planned: 3,
  attempted: 1,
  completed: 1,
  notAttempted: 2,
  stopReason: "resultLimit",
};

test("a stopped script exposes the untouched suffix and distinguishes calls from acknowledgements", () => {
  render(<ExecutionSummary summary={limited} status="stopped" />);
  const status = screen.getByRole("status", { name: "Statement execution summary" });
  expect(status.textContent).toContain(
    "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
  );
  expect(status.textContent).toContain("Result retention limit reached");
  expect(status.textContent).toContain("Statements 2–3 were not attempted");
  expect(status.textContent).toContain("An attempted statement may not have reached the server");
  expect(status.textContent).toContain(
    "Completed statements may still be uncommitted, and results may be truncated",
  );
});

test("a completed final limited read still states the retention stop reason", () => {
  const text = executionSummaryText({ ...limited, planned: 1, notAttempted: 0 }, "completed");
  expect(text).toBe(
    "Completed · Planned: 1 · Attempted: 1 · Completed: 1 · Not attempted: 0 · Result retention limit reached",
  );
  expect(text).not.toContain("were not attempted");
});

test.each([
  { status: "cancelled", reason: "cancelled", text: "Execution cancelled" },
  { status: "failed", reason: "failed", text: "Stopped after an error" },
  { status: "outcomeUnknown", reason: "outcomeUnknown", text: "Statement outcome unknown" },
] as const)(
  "$status preserves an attempted statement without inventing completion",
  ({ status, reason, text }) => {
    const summary: Summary = {
      planned: 2,
      attempted: 1,
      completed: 0,
      notAttempted: 1,
      stopReason: reason,
    };
    render(<ExecutionSummary summary={summary} status={status} />);
    expect(screen.getByRole("status").textContent).toContain(
      "Attempted: 1 · Completed: 0 · Not attempted: 1",
    );
    expect(screen.getByRole("status").textContent).toContain(text);
    expect(screen.getByRole("status").textContent).toContain("Statement 2 was not attempted");
  },
);

test("known zeros remain measured and missing legacy accounting remains unknown", () => {
  const { rerender } = render(
    <ExecutionSummary
      summary={{ planned: 3, attempted: 0, completed: 0, notAttempted: 3, stopReason: "cancelled" }}
      status="cancelled"
    />,
  );
  expect(screen.getByRole("status").textContent).toContain(
    "Planned: 3 · Attempted: 0 · Completed: 0 · Not attempted: 3",
  );
  rerender(<ExecutionSummary status="completed" />);
  expect(screen.getByRole("status").textContent).toContain("Completed · Statement counts unknown");
  expect(screen.getByRole("status").textContent).not.toMatch(/Planned:|Attempted:|Completed:/);
});
