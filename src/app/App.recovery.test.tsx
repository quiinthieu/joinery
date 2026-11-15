import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { App } from "./App";
import { client } from "../ipc/client";
import { useStudio } from "../state/studio";
import {
  RECOVERY_OPERATION_KEY,
  resumeWorkspaceAfterRecovery,
  pauseWorkspaceForRecovery,
} from "../state/workspace";
import type { Bootstrap, RecoveryOperation } from "../ipc/generated";

vi.mock("../features/connections/Sidebar", () => ({ Sidebar: () => <p>Ordinary sidebar</p> }));
vi.mock("./Workbench", () => ({
  Welcome: () => <p>Ordinary welcome</p>,
  Workbench: () => <textarea aria-label="Ordinary SQL editor" />,
}));
vi.mock("../ipc/client", () => ({
  desktopAvailable: true,
  client: { call: vi.fn() },
  messageOf: (error: unknown) =>
    error && typeof error === "object" && "message" in error
      ? String(error.message)
      : String(error),
  appErrorOf: (error: unknown) => error,
}));

const original = { ...useStudio.getState() };
const id = "11111111-1111-4111-8111-111111111111";
const bootstrap = {
  profiles: [],
  groups: [],
  workspace: null,
  supportedEngines: [],
  passphraseConfigured: false,
} as unknown as Bootstrap;
let initialize: ReturnType<typeof vi.fn<() => Promise<void>>>;
let reconciled = false;
beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  resumeWorkspaceAfterRecovery(true);
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  reconciled = false;
  initialize = vi.fn(async () => {
    useStudio.setState({ bootstrap });
  });
  useStudio.setState(
    {
      ...original,
      bootstrap,
      tabs: [],
      sessions: {},
      sessionPool: {},
      snowflakeConsoleReceipts: {},
      initialize,
    },
    true,
  );
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "metadata_recovery_operation")
      throw { code: "notFound", message: "Unknown ID" };
    if (command === "reconcile_metadata_recovery") {
      reconciled = true;
      return {
        id,
        kind: null,
        state: "failed",
        progress: { phase: "staging", completedBytes: 0, totalBytes: null },
        cancellationRequested: false,
        backup: null,
        error: { code: "outcomeUnknown", message: "Original outcome unknown", detail: null },
      } as RecoveryOperation;
    }
    if (command === "recovery_backups") return [];
    if (command === "metadata_recovery_admission_closed") return false;
    if (command === "resource_usage")
      return { sessionCount: 0, resultCount: 0, activeRuns: 0, retainedBytes: 0 };
    return null;
  });
});
afterEach(() => {
  cleanup();
  sessionStorage.clear();
  resumeWorkspaceAfterRecovery(true);
  vi.restoreAllMocks();
  useStudio.setState(original, true);
});

test("retained recovery blocks ordinary editing and shortcuts until native reconciliation reloads metadata", async () => {
  render(<App />);
  expect(await screen.findByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  fireEvent.keyDown(window, { key: "t", metaKey: true });
  expect(useStudio.getState().tabs).toHaveLength(0);
  const unload = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(unload);
  expect(unload.defaultPrevented).toBe(true);
  await screen.findByText(/Recovery status is unconfirmed/);
  fireEvent.click(screen.getByRole("button", { name: /Reload native metadata/ }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "Resolve metadata recovery" })).toBeNull(),
  );
  expect(reconciled).toBe(true);
  expect(initialize).toHaveBeenCalledTimes(2);
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull();
  expect(screen.getByText("Ordinary sidebar")).toBeTruthy();
});

test("an already paused startup remains gated after storage becomes readable with no marker", async () => {
  sessionStorage.clear();
  await pauseWorkspaceForRecovery();
  const originalCall = vi.mocked(client.call).getMockImplementation()!;
  vi.mocked(client.call).mockImplementation(async (...args) => {
    if (args[0] === "reconcile_unknown_metadata_recovery")
      return {
        id,
        kind: null,
        state: "failed",
        progress: { phase: "staging", completedBytes: 0, totalBytes: null },
        cancellationRequested: false,
        backup: null,
        error: { code: "outcomeUnknown", message: "Original outcome unknown", detail: null },
      } as RecoveryOperation;
    return originalCall(...args);
  });
  let finishReload!: () => void;
  initialize.mockImplementationOnce(async () => {
    useStudio.setState({ bootstrap });
  });
  initialize.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finishReload = () => {
          useStudio.setState({ bootstrap });
          resolve();
        };
      }),
  );
  render(<App />);
  expect(await screen.findByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  await screen.findByText(/saved recovery identifier is unavailable or malformed/);
  fireEvent.click(screen.getByRole("button", { name: "Reload native metadata without retrying" }));
  await waitFor(() => expect(initialize).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  await act(async () => {
    finishReload();
  });
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "Resolve metadata recovery" })).toBeNull(),
  );
  expect(screen.getByText("Ordinary sidebar")).toBeTruthy();
  const calls = vi.mocked(client.call).mock.calls.map(([command]) => command);
  expect(calls.filter((command) => command === "reconcile_unknown_metadata_recovery")).toHaveLength(
    1,
  );
  expect(calls).not.toContain("reconcile_metadata_recovery");
  expect(calls).not.toContain("metadata_recovery_operation");
  expect(calls).not.toContain("start_metadata_recovery");
});

test("initial bootstrap must settle before reconciliation can replace metadata", async () => {
  let finish!: () => void;
  initialize.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  render(<App />);
  expect(screen.getByText(/Reading local metadata before recovery reconciliation/)).toBeTruthy();
  expect(
    vi
      .mocked(client.call)
      .mock.calls.some(([command]) => command === "metadata_recovery_operation"),
  ).toBe(false);
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  await act(async () => {
    finish();
  });
  await screen.findByText(/Recovery status is unconfirmed/);
});

test("StrictMode shares one initial bootstrap and drains it before the reconciled reload", async () => {
  const releases: (() => void)[] = [];
  initialize.mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        releases.push(() => {
          useStudio.setState({ bootstrap });
          resolve();
        });
      }),
  );
  render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
  expect(initialize).toHaveBeenCalledTimes(1);
  expect(screen.getByText(/Reading local metadata before recovery reconciliation/)).toBeTruthy();
  expect(
    vi
      .mocked(client.call)
      .mock.calls.some(([command]) => command === "metadata_recovery_operation"),
  ).toBe(false);
  await act(async () => {
    releases[0]();
  });
  await screen.findByText(/Recovery status is unconfirmed/);
  fireEvent.click(screen.getByRole("button", { name: "Reload native metadata without retrying" }));
  await waitFor(() => expect(initialize).toHaveBeenCalledTimes(2));
  expect(reconciled).toBe(true);
  expect(screen.getByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  await act(async () => {
    releases[1]();
  });
  await waitFor(() =>
    expect(screen.queryByRole("dialog", { name: "Resolve metadata recovery" })).toBeNull(),
  );
  expect(screen.getByText("Ordinary sidebar")).toBeTruthy();
  expect(releases).toHaveLength(2);
  expect(initialize).toHaveBeenCalledTimes(2);
});

test("a failed native reconciliation keeps the gate and original retained identity", async () => {
  const draftId = useStudio.getState().openQuery(null, "SELECT 'held restored draft'");
  const call = vi.mocked(client.call).getMockImplementation()!;
  vi.mocked(client.call).mockImplementation(async (...args) => {
    if (args[0] === "reconcile_metadata_recovery") throw new Error("Native fence unavailable");
    return call(...args);
  });
  render(<App />);
  await screen.findByText(/Recovery status is unconfirmed/);
  fireEvent.click(screen.getByRole("button", { name: /Reload native metadata/ }));
  await screen.findByText("Native fence unavailable");
  expect(screen.getByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
  expect(screen.queryByText("Ordinary sidebar")).toBeNull();
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(initialize).toHaveBeenCalledTimes(1);
  expect(useStudio.getState().tabs.find((tab) => tab.id === draftId)?.sql).toBe(
    "SELECT 'held restored draft'",
  );
});

test("a normal mounted recovery pause does not replace the existing Preferences flow with a startup gate", async () => {
  sessionStorage.clear();
  render(<App />);
  await screen.findByText("Ordinary sidebar");
  await act(async () => {
    await pauseWorkspaceForRecovery();
  });
  expect(screen.queryByRole("dialog", { name: "Resolve metadata recovery" })).toBeNull();
  expect(screen.getByText("Ordinary sidebar")).toBeTruthy();
});

test.each(["malformed", "unreadable"])(
  "%s retained storage blocks startup without inventing an original UUID",
  async (mode) => {
    if (mode === "malformed") sessionStorage.setItem(RECOVERY_OPERATION_KEY, "malformed");
    else
      vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
        throw new Error("Storage unavailable");
      });
    render(<App />);
    await screen.findByText(/saved recovery identifier is unavailable or malformed/);
    expect(screen.getByRole("dialog", { name: "Resolve metadata recovery" })).toBeTruthy();
    expect(screen.queryByText("Ordinary sidebar")).toBeNull();
    expect(
      vi
        .mocked(client.call)
        .mock.calls.some(([command]) =>
          [
            "metadata_recovery_operation",
            "start_metadata_recovery",
            "reconcile_metadata_recovery",
          ].includes(command),
        ),
    ).toBe(false);
  },
);
