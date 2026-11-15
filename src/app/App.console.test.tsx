import { cleanup, fireEvent, render, screen, waitFor, act } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { App } from "./App";
import { useStudio } from "../state/studio";
import { client } from "../ipc/client";
import {
  clientId,
  plan,
  receiptView,
  session,
} from "../features/studio/SnowflakeConsole.test-fixtures";
import type { Bootstrap, ConnectionProfile } from "../ipc/generated";
vi.mock("../features/connections/Sidebar", () => ({ Sidebar: () => null }));
vi.mock("../app/Workbench", () => ({
  Welcome: () => <p>Welcome</p>,
  Workbench: ({
    onRun,
    onRunConsoleSelected,
    editorRef,
  }: {
    onRun: () => void;
    onRunConsoleSelected: () => void;
    editorRef: { current: unknown };
  }) => {
    editorRef.current = {
      executionSlice: () => ({ sql: "SELECT 1", fromUtf16: 0 }),
      selection: () => "BEGIN LET x := 1; RETURN x; END",
      document: () => "SELECT 1; COMMIT",
    };
    return (
      <>
        <button onClick={() => onRun()}>Review current</button>
        <button onClick={onRunConsoleSelected}>Review opaque selection</button>
      </>
    );
  },
}));
vi.mock("../ipc/client", () => ({
  desktopAvailable: false,
  client: {
    call: vi.fn(),
    prepareSnowflakeConsole: vi.fn(),
    startSnowflakeConsole: vi.fn(),
    snowflakeConsoleRun: vi.fn(),
    snowflakeConsoleRequest: vi.fn(),
  },
  messageOf: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  appErrorOf: (e: unknown) => ({
    code: "invalidInput",
    message: e instanceof Error ? e.message : String(e),
    detail: null,
  }),
}));
const initial = { ...useStudio.getState() };
const profile = {
  id: "profile",
  name: "Reviewed console",
  engine: "snowflake",
  database: "DB",
  environment: "local",
  safeMode: "confirm",
  storeDrafts: false,
  storeQueryText: false,
  snowflake: { authentication: "externalBrowser", reviewedConsole: true },
} as ConnectionProfile;
let tabId: string;
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useStudio.setState(
    {
      ...initial,
      bootstrap: {
        profiles: [profile],
        groups: [],
        supportedEngines: ["snowflake"],
      } as unknown as Bootstrap,
      tabs: [],
      sessions: { profile: session },
      consoleSessions: { [session.id]: true },
      snowflakeConsoleReceipts: {},
      sessionPool: {},
      initialize: vi.fn().mockResolvedValue(undefined),
    },
    true,
  );
  tabId = useStudio.getState().openQuery("profile", "SELECT 1; COMMIT");
  vi.mocked(client.call).mockImplementation(async (command) =>
    command === "extract_query_params" ? [] : command === "authorize_plan" ? "approval" : null,
  );
  vi.mocked(client.prepareSnowflakeConsole).mockResolvedValue(plan);
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  useStudio.setState(initial, true);
  localStorage.clear();
});
test("normal console run prepares dedicated endpoint and always reviews before start", async () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Review current" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  expect(client.prepareSnowflakeConsole).toHaveBeenCalledWith(session.id, {
    sql: "SELECT 1",
    mode: "parsedStatements",
    parameters: null,
  });
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
  expect(
    vi.mocked(client.call).mock.calls.some(([command]) => command === "prepare_execution"),
  ).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
});
test("selected server unit uses exact selected text and an explicit validation warning", async () => {
  const selected = {
    ...plan,
    mode: "selectedUnit" as const,
    validations: ["serverAtExecution" as const],
    review: {
      ...plan.review,
      statements: [{ ...plan.review.statements[0], sql: "BEGIN LET x := 1; RETURN x; END" }],
    },
  };
  vi.mocked(client.prepareSnowflakeConsole).mockResolvedValue(selected);
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Review opaque selection" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  expect(client.prepareSnowflakeConsole).toHaveBeenCalledWith(session.id, {
    sql: "BEGIN LET x := 1; RETURN x; END",
    mode: "selectedUnit",
    parameters: null,
  });
  expect(screen.getByText(/validated by Snowflake only at execution/)).toBeTruthy();
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
});
test("editing after review rejects the stale approval before dispatch", async () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Review current" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  act(() => useStudio.getState().updateTab(tabId, { sql: "DELETE FROM different_target" }));
  fireEvent.submit(screen.getByRole("dialog").querySelector("form")!);
  await screen.findByText(/editor or physical session changed/i);
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
});
test("cancel keyboard recovers a missing run ID using stable correlation, and app close remains guarded", async () => {
  const cancel = vi.fn().mockResolvedValue(undefined);
  useStudio.setState({
    cancelSnowflakeConsole: cancel,
    refreshSnowflakeConsole: vi.fn().mockResolvedValue(undefined),
    snowflakeConsoleReceipts: {
      [clientId]: { clientRequestId: clientId, tabId, sessionId: session.id, lookupPending: true },
    },
  });
  render(<App />);
  fireEvent.keyDown(window, { key: ".", metaKey: true });
  expect(cancel).toHaveBeenCalledWith(clientId);
  const event = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
});
test("health timer and mounted utility views schedule no session SQL for console sessions", async () => {
  vi.useFakeTimers();
  useStudio.getState().updateTab(tabId, { kind: "processes" });
  render(<App />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(31000);
  });
  expect(
    screen.getByText(/Metadata, monitoring and generated operations are disabled/),
  ).toBeTruthy();
  expect(
    vi
      .mocked(client.call)
      .mock.calls.some(([command]) =>
        ["health", "session_state", "session_contexts", "processes"].includes(command),
      ),
  ).toBe(false);
});

test("scalar parameter collection feeds one immutable console review without executing", async () => {
  vi.mocked(client.call).mockImplementation(async (command) =>
    command === "extract_query_params"
      ? [{ key: "value", positional: false, occurrences: 1 }]
      : null,
  );
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Review current" }));
  await screen.findByLabelText("Value for parameter value");
  fireEvent.change(screen.getByLabelText("Value for parameter value"), {
    target: { value: "bound exactly" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run query" }));
  await screen.findByRole("dialog", { name: "Review this operation" });
  expect(client.prepareSnowflakeConsole).toHaveBeenCalledWith(session.id, {
    sql: "SELECT 1",
    mode: "parsedStatements",
    parameters: [{ key: "value", kind: "text", value: "bound exactly" }],
  });
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
});
