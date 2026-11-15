import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import type { SnowflakeDmlResult } from "../../ipc/generated";
import { SnowflakeDmlNotice, SnowflakeDmlPendingControl } from "./SnowflakeDmlNotice";

afterEach(cleanup);

test("offers an explicit disconnect action without promising cancellation certainty", () => {
  const onDisconnect = vi.fn();
  render(<SnowflakeDmlPendingControl onDisconnect={onDisconnect} />);

  screen.getByRole("button", { name: "Disconnect to stop write" }).click();
  expect(onDisconnect).toHaveBeenCalledOnce();
  expect(screen.getByRole("status").textContent).toContain(
    "Disconnecting may not stop a statement already submitted.",
  );
  expect(screen.getByRole("status").textContent).toContain("Joinery will not retry it.");
});

function result(overrides: Partial<SnowflakeDmlResult> = {}): SnowflakeDmlResult {
  return {
    kind: "insert",
    statementOutcome: "succeeded",
    summary: {
      status: "validated",
      affectedRows: "1",
      multiJoinedRows: null,
    },
    state: {
      sessionId: "snowflake-session",
      transactionMode: "autoCommit",
      transactionState: "idle",
      contextKind: "database",
      activeContext: null,
      database: "ANALYTICS",
      role: "WRITER",
      searchPath: [],
    },
    retired: false,
    error: null,
    ...overrides,
  };
}

test("shows exact validated affected-row counts without rounding and keeps multi-joined separate", () => {
  render(
    <SnowflakeDmlNotice
      result={result({
        kind: "update",
        summary: {
          status: "validated",
          affectedRows: "900719925474099312345",
          multiJoinedRows: "900719925474099399999",
        },
      })}
    />,
  );

  expect(screen.getByRole("status", { name: "Snowflake UPDATE outcome" })).toBeTruthy();
  expect(screen.getByText("Affected rows: 900719925474099312345")).toBeTruthy();
  expect(screen.getByText("Multi-joined rows updated: 900719925474099399999")).toBeTruthy();
  expect(
    screen.getByText("Statement acknowledgement does not prove committed effects."),
  ).toBeTruthy();
});

test("labels an unavailable summary explicitly and never displays unvalidated count data", () => {
  render(
    <SnowflakeDmlNotice
      result={result({
        summary: {
          status: "unavailable",
          affectedRows: "999",
          multiJoinedRows: "88",
        },
      })}
    />,
  );

  expect(screen.getByText(/Affected-row summary is unavailable/)).toBeTruthy();
  expect(screen.queryByText(/999|88/)).toBeNull();
});

test("announces unknown and rejected outcomes without claiming statement effects", () => {
  const { rerender } = render(
    <SnowflakeDmlNotice
      result={result({
        statementOutcome: "unknown",
        summary: { status: "unavailable", affectedRows: null, multiJoinedRows: null },
      })}
    />,
  );

  expect(screen.getByRole("alert", { name: "Snowflake INSERT outcome" })).toBeTruthy();
  expect(screen.getByText("Statement outcome is unknown")).toBeTruthy();
  expect(
    screen.getByText(
      "The statement may have been submitted. Joinery will not retry it automatically.",
    ),
  ).toBeTruthy();

  rerender(<SnowflakeDmlNotice result={result({ statementOutcome: "rejected" })} />);
  expect(screen.getByRole("alert", { name: "Snowflake INSERT outcome" })).toBeTruthy();
  expect(screen.getByText("Statement was rejected")).toBeTruthy();
  expect(
    screen.getByText("Statement acknowledgement does not prove committed effects."),
  ).toBeTruthy();
});

test("keeps retirement, rollback recovery, prior results, and error notices accessible", () => {
  render(
    <SnowflakeDmlNotice
      hasPriorResults
      result={result({
        retired: true,
        error: { code: "internal", message: "Observer unavailable" },
        state: {
          sessionId: "snowflake-session",
          transactionMode: "manual",
          transactionState: "unknown",
          contextKind: "database",
          activeContext: null,
          database: "ANALYTICS",
          role: "WRITER",
          searchPath: [],
          transactionControlAuthority: { status: "rollbackOnly", revision: "rev-1" },
        },
      })}
    />,
  );

  expect(screen.getByText(/results shown below are from before this statement/i)).toBeTruthy();
  expect(
    screen.getByText("Only explicit rollback recovery is available for this session."),
  ).toBeTruthy();
  expect(
    screen.getByText("The SDK session was retired. Reconnect before running another statement."),
  ).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("Statement detail: Observer unavailable");
});
