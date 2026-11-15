import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Bootstrap, RecoveryBackup, RecoveryOperation } from "../../ipc/generated";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import {
  RECOVERY_OPERATION_KEY,
  discardRecoveryWorkspaceSaves,
  flushWorkspace,
  pauseWorkspaceForRecovery,
  resumeWorkspaceAfterRecovery,
  workspaceRecoveryStartupHeld,
} from "../../state/workspace";
import { MetadataRecovery, validBackupPassphrase } from "./MetadataRecovery";

vi.mock("../../ipc/client", () => ({
  desktopAvailable: true,
  client: { call: vi.fn() },
  messageOf: (value: unknown) =>
    value && typeof value === "object" && "message" in value
      ? String(value.message)
      : String(value),
}));
vi.mock("../../state/workspace", () => ({
  RECOVERY_OPERATION_KEY: "joinery.metadataRecoveryOperation.v1",
  flushWorkspace: vi.fn().mockResolvedValue(undefined),
  pauseWorkspaceForRecovery: vi.fn().mockResolvedValue(undefined),
  discardRecoveryWorkspaceSaves: vi.fn(),
  resumeWorkspaceAfterRecovery: vi.fn(),
  workspaceRecoveryStartupHeld: vi.fn(() => false),
}));

const id = "11111111-1111-4111-8111-111111111111";
const backupId = "22222222-2222-4222-8222-222222222222";
const initial = { ...useStudio.getState() };
const bootstrap: Bootstrap = {
  profiles: [],
  groups: [],
  workspace: null,
  passphraseConfigured: false,
  supportedEngines: ["postgres", "mysql", "redis", "snowflake"],
  resultBudgetBytes: 64 * 1024 * 1024,
  rowLimit: 500,
  savedQueries: [],
  bookmarks: [],
  sqlSnippets: [],
  sqlFormatSettings: { keywordCase: "upper", indentWidth: 2, linesBetweenQueries: 1 },
};
const backup: RecoveryBackup = {
  format: "encryptedV1",
  metadata: { id: backupId, createdAt: 1, sizeBytes: 4096, digest: "a".repeat(64) },
};
function operation(overrides: Partial<RecoveryOperation> = {}): RecoveryOperation {
  return {
    id,
    kind: "create",
    state: "running",
    cancellationRequested: false,
    progress: { phase: "encrypting", completedBytes: 0, totalBytes: null },
    backup: null,
    error: null,
    ...overrides,
  };
}
let inventory: RecoveryBackup[];
let current: RecoveryOperation;
let submitted: unknown;

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(workspaceRecoveryStartupHeld).mockReturnValue(false);
  sessionStorage.clear();
  inventory = [backup];
  current = operation();
  submitted = null;
  vi.spyOn(crypto, "randomUUID").mockReturnValue(id);
  useStudio.setState(
    {
      ...initial,
      bootstrap,
      tabs: [],
      sessions: {},
      initialize: vi.fn(async () => {
        useStudio.setState({ bootstrap });
      }),
    },
    true,
  );
  vi.mocked(client.call).mockImplementation((async (
    command: string,
    args?: Record<string, unknown>,
  ) => {
    if (command === "recovery_backups") return inventory;
    if (command === "metadata_recovery_admission_closed") return false;
    if (command === "delete_recovery_backup") return null;
    if (command === "start_metadata_recovery") {
      submitted = structuredClone(args);
      return current;
    }
    if (command === "metadata_recovery_operation" || command === "cancel_metadata_recovery")
      return current;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
});
afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.restoreAllMocks();
  useStudio.setState(initial, true);
});

async function fillKey() {
  fireEvent.change(screen.getByLabelText("Backup passphrase"), {
    target: { value: "separate backup key" },
  });
  fireEvent.change(screen.getByLabelText("Confirm backup passphrase"), {
    target: { value: "separate backup key" },
  });
}
async function chooseBackup() {
  await waitFor(() =>
    expect(screen.getByLabelText("Metadata backup").querySelectorAll("option")).toHaveLength(2),
  );
  fireEvent.change(screen.getByLabelText("Metadata backup"), { target: { value: backupId } });
}

async function startReadyRecovery(name: string) {
  const button = screen.getByRole("button", { name });
  await waitFor(() => expect(button).toHaveProperty("disabled", false));
  expect(client.call).toHaveBeenCalledWith("metadata_recovery_admission_closed");
  fireEvent.click(button);
}

test("backup passphrase limits use Unicode scalars and reject lone surrogates", () => {
  expect(validBackupPassphrase("x".repeat(11))).toBe(false);
  expect(validBackupPassphrase("x".repeat(12))).toBe(true);
  expect(validBackupPassphrase("😀".repeat(1024))).toBe(true);
  expect(validBackupPassphrase("😀".repeat(1025))).toBe(false);
  expect(validBackupPassphrase("x".repeat(12) + "\uD800")).toBe(false);
});

test("one create retains only its opaque ID, clears keys and waits for native completion", async () => {
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Create metadata backup");
  await waitFor(() =>
    expect(submitted).toEqual({
      id,
      request: { kind: "create", backupId: null, digest: null, confirmation: "" },
      backupPassphrase: "separate backup key",
      backupConfirmation: "separate backup key",
      writePassphrase: null,
    }),
  );
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(Object.keys(sessionStorage)).toEqual([RECOVERY_OPERATION_KEY]);
  expect(screen.getByLabelText("Backup passphrase")).toHaveProperty("value", "");
  expect(screen.getByLabelText("Confirm backup passphrase")).toHaveProperty("value", "");
  expect(pauseWorkspaceForRecovery).toHaveBeenCalled();
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  current = operation({ state: "succeeded", backup });
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(false);
  expect(
    vi.mocked(client.call).mock.calls.filter(([command]) => command === "start_metadata_recovery"),
  ).toHaveLength(1);
});

test("lost start response reads the same ID without resending any secret", async () => {
  vi.mocked(client.call).mockImplementation((async (
    command: string,
    args?: Record<string, unknown>,
  ) => {
    if (command === "recovery_backups") return inventory;
    if (command === "metadata_recovery_admission_closed") return false;
    if (command === "start_metadata_recovery") throw new Error("transport lost");
    if (command === "metadata_recovery_operation") return current;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Create metadata backup");
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("metadata_recovery_operation", { id }),
  );
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  current = operation({ state: "succeeded", backup });
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  expect(
    vi.mocked(client.call).mock.calls.filter(([command]) => command === "start_metadata_recovery"),
  ).toHaveLength(1);
});

test("a remounted completed restore discards old saves and reloads metadata without reconnecting", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  current = operation({ kind: "restore", state: "succeeded", backup });
  const initialize = vi.mocked(useStudio.getState().initialize);
  initialize.mockImplementation(async () => {
    expect(useStudio.getState().bootstrap).toBeNull();
    expect(useStudio.getState().tabs).toEqual([]);
    expect(discardRecoveryWorkspaceSaves).toHaveBeenCalled();
    expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
    useStudio.setState({ bootstrap });
  });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  expect(initialize).toHaveBeenCalledOnce();
  expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true);
  expect(client.call).not.toHaveBeenCalledWith("connect", expect.anything());
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
  expect(screen.getByText(/Reconnect manually/)).toBeTruthy();
});

test("unknown operation reconciliation fences delayed admission before metadata reload", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  vi.mocked(client.call).mockImplementation((async (
    command: string,
    args?: Record<string, unknown>,
  ) => {
    if (command === "metadata_recovery_operation")
      throw { code: "notFound", message: "No retained status" };
    if (command === "reconcile_metadata_recovery")
      return operation({
        kind: null,
        state: "failed",
        error: {
          code: "outcomeUnknown",
          message: "Prior outcome unknown; future admission fenced",
        },
      });
    if (command === "recovery_backups") return inventory;
    if (command === "metadata_recovery_admission_closed") return false;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
  render(<MetadataRecovery onBusy={vi.fn()} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Reload native metadata without retrying" }),
  );
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  const calls = vi.mocked(client.call).mock.calls.map(([command]) => command);
  expect(calls.indexOf("reconcile_metadata_recovery")).toBeLessThan(
    calls.indexOf("recovery_backups"),
  );
  expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true);
  expect(calls).not.toContain("start_metadata_recovery");
});

test("transport errors and mismatched native identities keep workspace saving paused", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  current = operation({ id: backupId, state: "succeeded" });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await screen.findByText(/identity or status changed unexpectedly/);
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(
    screen.queryByRole("button", { name: "Reload native metadata without retrying" }),
  ).toBeNull();
});

test("a retained unknown-outcome tombstone reloads metadata without inventing an operation kind", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  current = operation({
    kind: null,
    state: "failed",
    error: { code: "outcomeUnknown", message: "Prior outcome unknown; future admission fenced" },
  });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  expect(useStudio.getState().initialize).toHaveBeenCalledOnce();
  expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true);
  expect(screen.getByText("Prior outcome unknown; future admission fenced")).toBeTruthy();
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
});

test("legacy restore has a new undo key, separate current write authorization and explicit confirmation", async () => {
  inventory = [{ ...backup, format: "legacySqlite" }];
  current = operation({ kind: "restore" });
  useStudio.setState({ bootstrap: { ...bootstrap, passphraseConfigured: true } });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await chooseBackup();
  await fillKey();
  fireEvent.change(screen.getByLabelText(/Type RESTORE/), { target: { value: "RESTORE" } });
  fireEvent.change(screen.getByLabelText("Current write passphrase"), {
    target: { value: "current write key" },
  });
  expect(screen.getByText(/Legacy unencrypted backup: its original/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Export encrypted backup…" })).toHaveProperty(
    "disabled",
    true,
  );
  await startReadyRecovery("Restore metadata");
  await waitFor(() =>
    expect(submitted).toEqual({
      id,
      request: { kind: "restore", backupId, digest: "a".repeat(64), confirmation: "RESTORE" },
      backupPassphrase: "separate backup key",
      backupConfirmation: "separate backup key",
      writePassphrase: "current write key",
    }),
  );
  expect(screen.getByLabelText("Current write passphrase")).toHaveProperty("value", "");
});

test("native encrypted export receives an opaque backup identity and no key or path", async () => {
  current = operation({ kind: "export" });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await chooseBackup();
  await fillKey();
  await startReadyRecovery("Export encrypted backup…");
  await waitFor(() =>
    expect(submitted).toEqual({
      id,
      request: { kind: "export", backupId, digest: "a".repeat(64), confirmation: "" },
      backupPassphrase: "",
      backupConfirmation: "",
      writePassphrase: null,
    }),
  );
});

test("replacement cannot be cancelled and cancellation acknowledgement alone does not settle the job", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  current = operation({ progress: { phase: "replacing", completedBytes: 0, totalBytes: null } });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  expect(await screen.findByRole("button", { name: "Cancel recovery operation" })).toHaveProperty(
    "disabled",
    true,
  );
  current = operation({ cancellationRequested: true });
  await screen.findByRole("button", { name: "Cancellation requested" });
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
});

test("a workspace flush failure prevents admission and does not retain an operation marker", async () => {
  vi.mocked(flushWorkspace).mockRejectedValueOnce(new Error("save failed"));
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Create metadata backup");
  await screen.findByText("save failed");
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull();
});

test("native import has no browser path or passphrase and is never resubmitted", async () => {
  current = operation({ kind: "import" });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Import encrypted backup…");
  await waitFor(() =>
    expect(submitted).toEqual({
      id,
      request: { kind: "import", backupId: null, digest: null, confirmation: "" },
      backupPassphrase: "",
      backupConfirmation: "",
      writePassphrase: null,
    }),
  );
  expect(screen.getByLabelText("Backup passphrase")).toHaveProperty("value", "");
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
});

test("deleting a snapshot requires DELETE and sends its exact native digest", async () => {
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await chooseBackup();
  const remove = screen.getByRole("button", { name: "Delete snapshot" });
  expect(remove).toHaveProperty("disabled", true);
  fireEvent.change(screen.getByLabelText("Type RESTORE to replace metadata"), {
    target: { value: "DELETE" },
  });
  fireEvent.click(remove);
  await waitFor(() =>
    expect(client.call).toHaveBeenCalledWith("delete_recovery_backup", {
      id: backupId,
      digest: "a".repeat(64),
      confirmation: "DELETE",
    }),
  );
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
});

test("a cancellation request keeps the original ID until the native terminal is observed", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  render(<MetadataRecovery onBusy={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "Cancel recovery operation" }));
  await waitFor(() => expect(client.call).toHaveBeenCalledWith("cancel_metadata_recovery", { id }));
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  current = operation({ state: "cancelled" });
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
});

test("a failed restore finalization still reloads actual metadata before resuming saves", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, id);
  current = operation({
    kind: "restore",
    state: "failed",
    error: { code: "internal", message: "Finalization failed" },
  });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await waitFor(() => expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull());
  expect(useStudio.getState().initialize).toHaveBeenCalledOnce();
  expect(discardRecoveryWorkspaceSaves).toHaveBeenCalled();
  expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true);
  expect(screen.getByText("Finalization failed")).toBeTruthy();
});

test("a foreign operation kind cannot settle the original start", async () => {
  current = operation({ kind: "export", state: "succeeded" });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Create metadata backup");
  await screen.findByText(/identity or status changed unexpectedly/);
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe(id);
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
});

test("browser marker failure prevents native admission", async () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementationOnce(() => {
    throw new Error("Storage unavailable");
  });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await fillKey();
  await startReadyRecovery("Create metadata backup");
  await screen.findByText("Storage unavailable");
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
});

function globalFence(overrides: Partial<RecoveryOperation> = {}): RecoveryOperation {
  return operation({
    id: backupId,
    kind: null,
    state: "failed",
    progress: { phase: "finalizing", completedBytes: 0, totalBytes: null },
    error: {
      code: "outcomeUnknown",
      message: "Prior outcome unknown; all recovery admission is closed for this process",
    },
    ...overrides,
  });
}

test("startup hold with a now-absent marker uses global fencing before reload and resume", async () => {
  vi.mocked(workspaceRecoveryStartupHeld).mockReturnValue(true);
  const originalCall = vi.mocked(client.call).getMockImplementation()!;
  vi.mocked(client.call).mockImplementation(async (...args) => {
    if (args[0] === "reconcile_unknown_metadata_recovery") return globalFence();
    return originalCall(...args);
  });
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await screen.findByText(/saved recovery identifier is unavailable or malformed/);
  fireEvent.click(screen.getByRole("button", { name: "Reload native metadata without retrying" }));
  await waitFor(() => expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true));
  expect(client.call).toHaveBeenCalledWith("reconcile_unknown_metadata_recovery");
  expect(useStudio.getState().initialize).toHaveBeenCalledOnce();
  const calls = vi.mocked(client.call).mock.calls.map(([command]) => command);
  expect(calls).not.toContain("reconcile_metadata_recovery");
  expect(calls).not.toContain("metadata_recovery_operation");
  expect(calls).not.toContain("start_metadata_recovery");
  expect(
    screen.getByText(/Further recovery requires quitting and restarting the full Joinery app/),
  ).toBeTruthy();
});

test.each(["malformed", "unavailable"])(
  "%s original identifier uses only the global native fence before resuming",
  async (mode) => {
    if (mode === "malformed")
      sessionStorage.setItem(RECOVERY_OPERATION_KEY, "not-an-original-uuid");
    else
      vi.spyOn(Storage.prototype, "getItem").mockImplementationOnce(() => {
        throw new Error("Storage unavailable");
      });
    vi.mocked(client.call).mockImplementation((async (command: string) => {
      if (command === "reconcile_unknown_metadata_recovery") return globalFence();
      if (command === "recovery_backups") return inventory;
      if (command === "metadata_recovery_admission_closed") return true;
      throw new Error(`Unexpected command ${command}`);
    }) as typeof client.call);
    render(<MetadataRecovery onBusy={vi.fn()} />);
    expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
    fireEvent.click(
      await screen.findByRole("button", { name: "Reload native metadata without retrying" }),
    );
    await waitFor(() => expect(resumeWorkspaceAfterRecovery).toHaveBeenCalledWith(true));
    expect(client.call).toHaveBeenCalledWith("reconcile_unknown_metadata_recovery");
    const calls = vi.mocked(client.call).mock.calls.map(([command]) => command);
    expect(
      calls.filter((command) => command === "reconcile_unknown_metadata_recovery"),
    ).toHaveLength(1);
    expect(calls.indexOf("reconcile_unknown_metadata_recovery")).toBeLessThan(
      calls.indexOf("recovery_backups"),
    );
    expect(calls).not.toContain("reconcile_metadata_recovery");
    expect(calls).not.toContain("metadata_recovery_operation");
    expect(calls).not.toContain("start_metadata_recovery");
    expect(crypto.randomUUID).not.toHaveBeenCalled();
    expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull();
    expect(screen.getByText(/The prior operation outcome remains unknown/)).toBeTruthy();
    expect(screen.getByText(/Further recovery requires quitting and restarting/)).toBeTruthy();
    expect(screen.queryByText(/Metadata restored/)).toBeNull();
    expect(screen.queryByText(/Encrypted backup created/)).toBeNull();
    await fillKey();
    expect(screen.getByRole("button", { name: "Create metadata backup" })).toHaveProperty(
      "disabled",
      true,
    );
    expect(screen.getByRole("button", { name: "Import encrypted backup…" })).toHaveProperty(
      "disabled",
      true,
    );
  },
);

test("failed global fencing preserves the malformed marker and paused workspace", async () => {
  sessionStorage.setItem(RECOVERY_OPERATION_KEY, "lost-original-id");
  vi.mocked(client.call).mockRejectedValue(new Error("A native recovery is still running"));
  render(<MetadataRecovery onBusy={vi.fn()} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Reload native metadata without retrying" }),
  );
  await screen.findByText("A native recovery is still running");
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe("lost-original-id");
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  expect(useStudio.getState().initialize).not.toHaveBeenCalled();
  expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
    "reconcile_unknown_metadata_recovery",
  ]);
});

test.each([
  operation({ id: backupId, state: "succeeded" }),
  globalFence({ id: "not-a-canonical-uuid" }),
  globalFence({ error: { code: "internal", message: "No global proof" } }),
])(
  "invalid global proof %# cannot invent an original kind or release the workspace",
  async (proof) => {
    sessionStorage.setItem(RECOVERY_OPERATION_KEY, "lost-original-id");
    vi.mocked(client.call).mockResolvedValue(proof as never);
    render(<MetadataRecovery onBusy={vi.fn()} />);
    fireEvent.click(
      await screen.findByRole("button", { name: "Reload native metadata without retrying" }),
    );
    await screen.findByText(/Native recovery reconciliation is incomplete/);
    expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
    expect(useStudio.getState().initialize).not.toHaveBeenCalled();
    expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBe("lost-original-id");
    expect(vi.mocked(client.call).mock.calls.map(([command]) => command)).toEqual([
      "reconcile_unknown_metadata_recovery",
    ]);
  },
);

test("persistent browser-storage failure keeps saves paused even after a valid global fence", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("Storage remains unavailable");
  });
  vi.mocked(client.call).mockImplementation((async (command: string) => {
    if (command === "reconcile_unknown_metadata_recovery") return globalFence();
    if (command === "recovery_backups") return inventory;
    if (command === "metadata_recovery_admission_closed") return true;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
  render(<MetadataRecovery onBusy={vi.fn()} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Reload native metadata without retrying" }),
  );
  await screen.findByText("Storage remains unavailable");
  expect(resumeWorkspaceAfterRecovery).not.toHaveBeenCalled();
  expect(client.call).not.toHaveBeenCalledWith("reconcile_metadata_recovery", expect.anything());
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
  expect(screen.getByText(/Recovery admission is closed for this app process/)).toBeTruthy();
});

test("a remounted window reads native closure without inventing an operation marker", async () => {
  vi.mocked(client.call).mockImplementation((async (command: string) => {
    if (command === "metadata_recovery_admission_closed") return true;
    if (command === "recovery_backups") return inventory;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
  const first = render(<MetadataRecovery onBusy={vi.fn()} />);
  await screen.findByText(/Recovery admission is closed for this app process/);
  first.unmount();
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await screen.findByText(/Recovery admission is closed for this app process/);
  await chooseBackup();
  await fillKey();
  fireEvent.change(screen.getByLabelText("Type RESTORE to replace metadata"), {
    target: { value: "RESTORE" },
  });
  for (const name of [
    "Create metadata backup",
    "Import encrypted backup…",
    "Restore metadata",
    "Export encrypted backup…",
  ])
    expect(screen.getByRole("button", { name })).toHaveProperty("disabled", true);
  expect(screen.getByRole("button", { name: "Refresh backup inventory" })).toHaveProperty(
    "disabled",
    false,
  );
  fireEvent.change(screen.getByLabelText("Type RESTORE to replace metadata"), {
    target: { value: "DELETE" },
  });
  expect(screen.getByRole("button", { name: "Delete snapshot" })).toHaveProperty("disabled", false);
  expect(
    vi
      .mocked(client.call)
      .mock.calls.filter(([command]) => command === "metadata_recovery_admission_closed"),
  ).toHaveLength(2);
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
  expect(client.call).not.toHaveBeenCalledWith("reconcile_unknown_metadata_recovery");
  expect(sessionStorage.getItem(RECOVERY_OPERATION_KEY)).toBeNull();
});

test("an invalid native admission read keeps new operations disabled", async () => {
  vi.mocked(client.call).mockImplementation((async (command: string) => {
    if (command === "metadata_recovery_admission_closed") return "false";
    if (command === "recovery_backups") return inventory;
    throw new Error(`Unexpected command ${command}`);
  }) as typeof client.call);
  render(<MetadataRecovery onBusy={vi.fn()} />);
  await screen.findByText(/Native recovery admission is unknown/);
  await fillKey();
  expect(screen.getByRole("button", { name: "Create metadata backup" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(screen.getByRole("button", { name: "Import encrypted backup…" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(client.call).not.toHaveBeenCalledWith("start_metadata_recovery", expect.anything());
});
