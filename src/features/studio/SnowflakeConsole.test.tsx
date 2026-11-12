import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { SnowflakeConsoleReview } from "./SnowflakeConsoleReview";
import { SnowflakeConsoleReceipts } from "./SnowflakeConsoleReceipts";
import {
  consoleReceiptHeld,
  consoleRunStatus,
  rememberConsoleReceipts,
  restoredConsoleReceipts,
  validateConsolePlan,
  validateConsoleReceipt,
} from "./SnowflakeConsole";
import { clientId, runId, plan, receiptView } from "./SnowflakeConsole.test-fixtures";
vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
const initial = { ...useStudio.getState() };
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  localStorage.clear();
  useStudio.setState(initial, true);
});
test("only opaque receipt IDs survive reload; malformed storage never restores SQL or a run", () => {
  localStorage.setItem(
    "joinery.snowflakeConsoleReceipts.v1",
    JSON.stringify([clientId, { sql: "secret", runId }, "SELECT secret"]),
  );
  expect(restoredConsoleReceipts()).toEqual({
    [clientId]: { clientRequestId: clientId, lookupPending: true },
  });
  rememberConsoleReceipts({
    [clientId]: {
      clientRequestId: clientId,
      lookupPending: false,
      error: "private error",
      view: receiptView(),
    },
  });
  expect(localStorage.getItem("joinery.snowflakeConsoleReceipts.v1")).toBe(
    JSON.stringify([clientId]),
  );
  localStorage.setItem("joinery.snowflakeConsoleReceipts.v1", "{");
  expect(restoredConsoleReceipts()).toEqual({});
});
test("whole-script review preserves exact SQL, bound values and selected-unit validation warning", async () => {
  vi.mocked(client.call).mockResolvedValue("approval");
  const execute = vi.fn().mockResolvedValue(undefined);
  render(
    <SnowflakeConsoleReview
      plan={{
        ...plan,
        mode: "selectedUnit",
        validations: ["serverAtExecution"],
        review: { ...plan.review, statements: plan.review.statements.slice(0, 1) },
      }}
      onExecute={execute}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText("SELECT :value")).toBeTruthy();
  expect(screen.getByText('"immutable"')).toBeTruthy();
  expect(screen.getByText(/fixed 65-second/)).toBeTruthy();
  expect(screen.getByText(/validated by Snowflake only at execution/)).toBeTruthy();
  fireEvent.submit(screen.getByRole("dialog").querySelector("form")!);
  await waitFor(() => expect(execute).toHaveBeenCalledWith("approval"));
  expect(client.call).toHaveBeenCalledWith("authorize_plan", {
    planId: "plan",
    passphrase: null,
    confirmation: null,
  });
});
test("review close sends no execution or authorization", () => {
  const close = vi.fn(),
    execute = vi.fn();
  render(<SnowflakeConsoleReview plan={plan} onExecute={execute} onClose={close} />);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(close).toHaveBeenCalledOnce();
  expect(execute).not.toHaveBeenCalled();
  expect(client.call).not.toHaveBeenCalled();
});
test("receipt separates original ACK, untouched COMMIT and unavailable rows without commit claims", () => {
  const view = receiptView();
  useStudio.setState({
    snowflakeConsoleReceipts: {
      [clientId]: { clientRequestId: clientId, runId, view, lookupPending: false },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  expect(screen.getByText(/Unit 1: Original statement acknowledged/)).toBeTruthy();
  expect(screen.getByText(/Unit 2: Not attempted/)).toBeTruthy();
  expect(screen.getByText(/does not mean the statements returned zero rows/)).toBeTruthy();
  expect(
    screen.getByText(/does not establish transaction state or committed effects/),
  ).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Cancel retained run" })).toBeNull();
});
test("recording hold remains cancellable/reconcilable after terminal ACK and exposes orthogonal flags", async () => {
  const view = receiptView({ pendingRecording: true });
  view.run.observations.recordingFailed = true;
  const reconcile = vi.fn().mockResolvedValue(undefined),
    cancel = vi.fn().mockResolvedValue(undefined);
  useStudio.setState({
    refreshSnowflakeConsole: reconcile,
    cancelSnowflakeConsole: cancel,
    snowflakeConsoleReceipts: {
      [clientId]: { clientRequestId: clientId, view, lookupPending: false },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  expect(screen.getByText(/Recording hold:/)).toBeTruthy();
  expect(screen.getByText(/Unit 1: Original statement acknowledged/)).toBeTruthy();
  expect(screen.getByText(/recording failure occurred/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cancel retained run" }));
  expect(cancel).toHaveBeenCalledWith(clientId);
  fireEvent.click(screen.getByRole("button", { name: /Reconcile receipt/ }));
  expect(reconcile).toHaveBeenCalledWith(clientId);
});
test("missing receipt keeps lookup/cancel available and does not offer retry execution", () => {
  const reconcile = vi.fn().mockResolvedValue(undefined),
    cancel = vi.fn().mockResolvedValue(undefined);
  useStudio.setState({
    refreshSnowflakeConsole: reconcile,
    cancelSnowflakeConsole: cancel,
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        lookupPending: true,
        error: "No durable receipt found yet",
      },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  expect(screen.getByText(/execution will not be resent/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Cancel retained run" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
  expect(reconcile).toHaveBeenCalledOnce();
});
test("closed plan and receipt validation rejects identity/count/result-index changes", () => {
  expect(() => validateConsolePlan({ ...plan, validations: [] }, plan.review.sessionId)).toThrow();
  expect(() =>
    validateConsolePlan({ ...plan, mode: "selectedUnit" }, plan.review.sessionId),
  ).toThrow();
  expect(() => validateConsolePlan(plan, "other")).toThrow();
  const receipt = {
    clientRequestId: clientId,
    sessionId: plan.review.sessionId,
    lookupPending: true,
  };
  const view = receiptView();
  view.results = [
    {
      id: "bad",
      runId,
      statementIndex: -1,
      columns: [],
      retainedRows: 0,
      status: "complete",
      table: null,
    },
  ];
  expect(() => validateConsoleReceipt(receipt, view)).toThrow();
});
test("unresolved and late recording holds dominate terminal display status without changing original reason", () => {
  const view = receiptView();
  expect(consoleRunStatus(view)).toBe("stopped");
  expect(consoleRunStatus({ ...view, pendingRecording: true })).toBe("running");
  expect(consoleReceiptHeld({ clientRequestId: clientId, view, lookupPending: true })).toBe(true);
  view.run.terminal!.reason = "interrupted";
  expect(consoleRunStatus(view)).toBe("outcomeUnknown");
});

test("receipt opens existing native rows explicitly without replaying SQL", () => {
  const open = vi.fn().mockResolvedValue(undefined);
  const view = receiptView({
    results: [
      {
        id: "retained",
        runId,
        statementIndex: 0,
        columns: [],
        retainedRows: 2,
        status: "complete",
        table: null,
      },
    ],
  });
  useStudio.setState({
    openSnowflakeConsoleResults: open,
    snowflakeConsoleReceipts: {
      [clientId]: { clientRequestId: clientId, runId, view, lookupPending: false },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  fireEvent.click(screen.getByRole("button", { name: "Open retained results" }));
  expect(open).toHaveBeenCalledWith(clientId);
  expect(client.call).not.toHaveBeenCalled();
});

test("received definite non-admission is settled and offers a fresh review, not cancel or retry", () => {
  useStudio.setState({
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        sessionId: plan.review.sessionId,
        lookupPending: false,
        notAdmitted: "approvalExpired",
      },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  expect(screen.getByText(/No SQL was admitted/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Cancel retained/ })).toBeNull();
  expect(screen.getByRole("button", { name: /Reconcile/ })).toBeTruthy();
  expect(screen.queryByText(/Original outcome unavailable/)).toBeNull();
});

test("a durable negative keeps its opaque ID after reload, without storing its proof payload", () => {
  rememberConsoleReceipts({
    [clientId]: {
      clientRequestId: clientId,
      sessionId: "private-session",
      planDigest: plan.digest,
      notAdmitted: "approvalExpired",
      lookupPending: false,
    },
  });
  expect(localStorage.getItem("joinery.snowflakeConsoleReceipts.v1")).toBe(
    JSON.stringify([clientId]),
  );
  expect(restoredConsoleReceipts()[clientId]).toEqual({
    clientRequestId: clientId,
    lookupPending: true,
  });
});
test("unresolved refresh of an earlier refusal visibly holds new runs without claiming submission", () => {
  useStudio.setState({
    refreshSnowflakeConsole: vi.fn().mockResolvedValue(undefined),
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        sessionId: plan.review.sessionId,
        notAdmitted: "approvalExpired",
        lookupPending: true,
        error: "Decision identity changed",
      },
    },
  });
  render(<SnowflakeConsoleReceipts />);
  expect(screen.getByText(/new console runs remain held/)).toBeTruthy();
  expect(screen.getByRole("button", { name: /Reconcile receipt/ })).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Cancel retained/ })).toBeNull();
});

test("pending receipts stay visible while settled history expands independently in a keyboard scroll region", () => {
  const receipts: ReturnType<typeof useStudio.getState>["snowflakeConsoleReceipts"] = {};
  for (let i = 0; i < 11; i++)
    receipts[`settled-${i}`] = {
      clientRequestId: `settled-${i}`,
      lookupPending: false,
      view: receiptView(),
    };
  receipts[clientId] = { clientRequestId: clientId, lookupPending: true };
  useStudio.setState({
    refreshSnowflakeConsole: vi.fn().mockResolvedValue(undefined),
    snowflakeConsoleReceipts: receipts,
  });
  render(<SnowflakeConsoleReceipts />);
  const region = screen.getByRole("region", { name: "Console receipt details" });
  expect(region.tabIndex).toBe(0);
  expect(screen.getAllByRole("article")).toHaveLength(1);
  expect(screen.getByRole("article", { name: `Console receipt ${clientId}` })).toBeTruthy();
  const toggle = screen.getByRole("button", { name: /Console receipts/ });
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(toggle);
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  expect(screen.getAllByRole("article")).toHaveLength(12);
  fireEvent.click(toggle);
  expect(screen.getAllByRole("article")).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Cancel retained run" })).toBeTruthy();
  expect(screen.getByText(/pending receipt remains visible/)).toBeTruthy();
});
