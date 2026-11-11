import { cleanup, render, screen } from "@testing-library/react";
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
  expect(screen.getByText(/1 statement · SQL text not stored/)).toBeTruthy();
  expect(screen.queryByText(/UPDATE .* SET/i)).toBeNull();
});
