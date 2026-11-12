import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { HistoryEntry } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { HistoryView } from "./UtilityViews";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

test("shows DML acknowledgement and exact counts in durable history without SQL text", async () => {
  const entry: HistoryEntry = {
    id: "dml-history",
    connectionId: "snowflake-profile",
    connectionName: "Warehouse",
    startedAt: 1,
    sql: null,
    statementCount: 1,
    status: "completed",
    elapsedMs: 142,
    snowflakeDml: {
      kind: "update",
      statementOutcome: "succeeded",
      summary: {
        status: "validated",
        affectedRows: "900719925474099312345",
        multiJoinedRows: "900719925474099312346",
      },
      retired: false,
    },
  };
  vi.mocked(client.call).mockResolvedValue([entry]);
  render(<HistoryView />);

  expect(await screen.findByText("Snowflake UPDATE · statement succeeded")).toBeTruthy();
  expect(screen.getByText("Affected rows: 900719925474099312345")).toBeTruthy();
  expect(screen.getByText("Multi-joined rows updated: 900719925474099312346")).toBeTruthy();
  expect(
    screen.getByText("Statement acknowledgement does not prove committed effects."),
  ).toBeTruthy();
  expect(screen.getByText("SQL text not stored")).toBeTruthy();
  expect(screen.getByText(/Statement counts unknown/)).toBeTruthy();
  expect(screen.queryByText(/UPDATE .* SET/i)).toBeNull();
});

test("history distinguishes stopped suffixes, complete plans and legacy unknown counts with SQL storage unchanged", async () => {
  const base: HistoryEntry = {
    id: "stopped",
    connectionId: "fixture",
    connectionName: "Stopped fixture",
    startedAt: 1,
    sql: "SELECT limited; SELECT skipped; UPDATE skipped;",
    statementCount: 999,
    status: "stopped",
    elapsedMs: 12,
    executionSummary: {
      planned: 3,
      attempted: 1,
      completed: 1,
      notAttempted: 2,
      stopReason: "resultLimit",
    },
  };
  vi.mocked(client.call).mockResolvedValue([
    base,
    {
      ...base,
      id: "complete",
      connectionName: "Complete fixture",
      sql: null,
      status: "completed",
      executionSummary: {
        planned: 2,
        attempted: 2,
        completed: 2,
        notAttempted: 0,
        stopReason: null,
      },
    },
    {
      ...base,
      id: "legacy",
      connectionName: "Legacy fixture",
      sql: null,
      status: "completed",
      executionSummary: undefined,
    },
  ]);
  render(<HistoryView />);
  const stopped = await screen.findByRole("button", { name: /Stopped fixture/ });
  expect(within(stopped).getByRole("status").textContent).toContain(
    "Stopped · Planned: 3 · Attempted: 1 · Completed: 1 · Not attempted: 2",
  );
  expect(stopped.textContent).toContain("Statements 2–3 were not attempted");
  expect(stopped.textContent).toContain(base.sql);
  expect(stopped.querySelector(".status-dot.stopped")).toBeTruthy();
  const completed = screen.getByRole("button", { name: /Complete fixture/ });
  expect(completed.textContent).toContain(
    "Completed · Planned: 2 · Attempted: 2 · Completed: 2 · Not attempted: 0",
  );
  expect(completed.textContent).toContain("SQL text not stored");
  const legacy = screen.getByRole("button", { name: /Legacy fixture/ });
  expect(legacy.textContent).toContain("Statement counts unknown");
  expect(legacy.textContent).not.toMatch(/Planned:|Attempted:|Completed:/);
  expect(screen.queryByText(/999/)).toBeNull();
});
