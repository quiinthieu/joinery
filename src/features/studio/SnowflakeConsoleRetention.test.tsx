import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { client } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import type { SnowflakeConsoleReceipt } from "./SnowflakeConsole";
import {
  compactConsoleReceipts,
  consoleReceiptHeld,
  rememberConsoleReceipts,
  restoredConsoleReceipts,
  validateConsoleHandoff,
  validateConsoleStorage,
  validateConsoleSummary,
} from "./SnowflakeConsole";
import { SnowflakeConsoleReceipts } from "./SnowflakeConsoleReceipts";
import {
  clientId,
  runId,
  inventoryEpoch,
  receiptSummary,
  receiptView,
  session,
  plan,
} from "./SnowflakeConsole.test-fixtures";

vi.mock("../../ipc/client", () => ({
  client: { call: vi.fn(), snowflakeConsoleRequest: vi.fn(), startSnowflakeConsole: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));
const initial = { ...useStudio.getState() };
const key = "joinery.snowflakeConsoleReceipts.v1";
const id = (n: number) => `00000000-0000-4000-8000-${n.toString().padStart(12, "0")}`;
const storage = {
  inventoryEpoch,
  positiveCount: "1",
  refusalCount: "0",
  chargedBytes: "512",
  reservedBytes: null,
  admissionBudgetBytes: null,
  freeBytes: null,
  retentionEnabled: false,
};
function settled(n = 1): SnowflakeConsoleReceipt {
  const view = receiptView();
  view.run.metadata.clientRequestId = id(n);
  view.run.metadata.runId = id(100 + n);
  return {
    clientRequestId: id(n),
    sessionId: "console-session",
    connectionId: "profile",
    planDigest: "a".repeat(64),
    runId: id(100 + n),
    lookupPending: false,
    view,
    lastAccessed: n,
  };
}
beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  useStudio.setState(
    {
      ...initial,
      bootstrap: null,
      tabs: [],
      sessions: {},
      snowflakeConsoleReceipts: {},
      snowflakeConsoleResultOwners: {},
    },
    true,
  );
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "snowflake_console_storage") return storage;
    if (command === "snowflake_console_pending")
      return { clientRequestIds: [], recordingHold: false };
    if (command === "snowflake_console_receipts")
      return { inventoryEpoch, receipts: [receiptSummary()], nextCursor: null };
    return null;
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
  useStudio.setState(initial, true);
});

test("native inventory remains discoverable without browser correlations or connection profiles", async () => {
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  expect(await screen.findByRole("button", { name: `Receipt ${clientId}` })).toBeTruthy();
  expect(screen.getByText(/Archive keeps original evidence/)).toBeTruthy();
  expect(screen.getByText(/expanded retention is not enabled/)).toBeTruthy();
  for (const name of ["Receipt visibility", "Receipt decision", "Receipt connection"])
    expect(screen.getByRole("combobox", { name })).toBeTruthy();
  expect(
    vi
      .mocked(client.call)
      .mock.calls.every(([command]) => command.startsWith("snowflake_console_")),
  ).toBe(true);
});

test("malformed native storage accounting is unavailable instead of crashing or showing zero", async () => {
  vi.mocked(client.call).mockResolvedValue({ positiveCount: "unsafe" } as never);
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  expect(await screen.findByText(/Native receipt storage accounting is unavailable/)).toBeTruthy();
  expect(screen.queryByText(/Journal:/)).toBeNull();
});

test("archive is exact CAS; lost response reconciles browsing once without retrying mutation", async () => {
  const archived = receiptSummary({ archived: true, browsingRevision: "1" });
  vi.mocked(client.call).mockImplementation(async (command) => {
    if (command === "snowflake_console_storage") return storage;
    if (command === "snowflake_console_pending")
      return { clientRequestIds: [], recordingHold: false };
    if (command === "snowflake_console_receipts")
      return { inventoryEpoch, receipts: [receiptSummary()], nextCursor: null };
    if (command === "set_snowflake_console_receipt_archive") throw new Error("response lost");
    if (command === "snowflake_console_receipt_summary") return archived;
    throw new Error(`Unexpected ${command}`);
  });
  render(<SnowflakeConsoleReceipts />);
  fireEvent.click(screen.getByRole("button", { name: /Console receipts/ }));
  fireEvent.click(await screen.findByRole("button", { name: "Archive receipt" }));
  expect(await screen.findByText(/Current local preference is archived/)).toBeTruthy();
  const mutations = vi
    .mocked(client.call)
    .mock.calls.filter(([command]) => command === "set_snowflake_console_receipt_archive");
  expect(mutations).toEqual([
    [
      "set_snowflake_console_receipt_archive",
      {
        request: {
          inventoryEpoch,
          archived: true,
          items: [
            {
              clientRequestId: clientId,
              sessionId: "console-session",
              planDigest: "a".repeat(64),
              runId,
              kind: "positive",
              recordedAt: "1",
              originalRevision: "4",
              browsingRevision: "0",
              refusalCode: null,
            },
          ],
        },
      },
    ],
  ]);
  await waitFor(() =>
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Refresh receipts" })),
  );
  expect(
    vi
      .mocked(client.call)
      .mock.calls.every(
        ([command]) =>
          command.startsWith("snowflake_console_") ||
          command === "set_snowflake_console_receipt_archive",
      ),
  ).toBe(true);
});

test("summary ordering stays exact above JS safe integers and malformed fields are rejected", () => {
  expect(() =>
    validateConsoleSummary(receiptSummary({ sequence: "9007199254740993" })),
  ).not.toThrow();
  for (const change of [
    { sequence: "01" },
    { sequence: "9223372036854775808" },
    { originalRevision: 4 },
    { state: "invented" },
    { stopReason: "invented" },
    { connectionId: null },
    { archiveEligible: "true" },
  ])
    expect(() => validateConsoleSummary({ ...receiptSummary(), ...change } as never)).toThrow();
  expect(() => validateConsoleStorage({ ...storage, freeBytes: undefined } as never)).toThrow();
});

test("only exact settled native proof hands off a browser correlation", () => {
  const receipt: SnowflakeConsoleReceipt = {
    clientRequestId: clientId,
    sessionId: "console-session",
    connectionId: "profile",
    planDigest: "a".repeat(64),
    runId,
    lookupPending: false,
    view: receiptView(),
  };
  expect(() => validateConsoleHandoff(receipt, receiptSummary())).not.toThrow();
  for (const change of [
    { originalRevision: "5" },
    { recordedAt: "2" },
    { archiveEligible: false },
    { runId: id(8) },
    { sessionId: "other" },
  ])
    expect(() => validateConsoleHandoff(receipt, receiptSummary(change))).toThrow();
  expect(() =>
    validateConsoleHandoff({ ...receipt, lookupPending: true }, receiptSummary()),
  ).toThrow();
  localStorage.setItem(key, JSON.stringify([clientId]));
  rememberConsoleReceipts({ [clientId]: { ...receipt, inventoried: true, nativeSettled: true } });
  expect(restoredConsoleReceipts()).toEqual({});
});

test("detail cache bounds count and payload bytes while preserving unresolved identities and recording holds", () => {
  const receipts = Object.fromEntries(
    Array.from({ length: 100 }, (_, i) => [id(i + 1), settled(i + 1)]),
  );
  const held = settled(200);
  held.view!.pendingRecording = true;
  receipts[held.clientRequestId] = held;
  const compacted = compactConsoleReceipts(receipts);
  expect(Object.keys(compacted)).toHaveLength(101);
  expect(
    Object.values(compacted).filter((receipt) => receipt.view && !receipt.view.pendingRecording),
  ).toHaveLength(16);
  expect(compacted[held.clientRequestId].view).toBe(held.view);
  expect(compacted[id(1)].view).toBeUndefined();
  expect(compacted[id(1)].settledProof).toEqual({ revision: "4", recordedAt: "1" });
  const huge = settled(300);
  const outcome = huge.view!.run.steps[0].outcome;
  if (outcome?.kind !== "acknowledged") throw new Error("Missing fixture acknowledgement");
  outcome.evidence.context.database = "x".repeat(8 * 1024 * 1024);
  expect(
    compactConsoleReceipts({ [huge.clientRequestId]: huge })[huge.clientRequestId].view,
  ).toBeUndefined();
  expect(() =>
    validateConsoleHandoff(
      compactConsoleReceipts({ [huge.clientRequestId]: huge })[huge.clientRequestId],
      receiptSummary({
        clientRequestId: huge.clientRequestId,
        runId: huge.runId!,
        sequence: "300",
      }),
    ),
  ).not.toThrow();
  const publishing = compactConsoleReceipts({
    [huge.clientRequestId]: { ...huge, startPending: true, inventoried: true, nativeSettled: true },
  })[huge.clientRequestId];
  expect(publishing.view).toBeUndefined();
  expect(publishing.settledProof).toBeTruthy();
  expect(consoleReceiptHeld(publishing)).toBe(false);
  const native = Object.fromEntries(
    Array.from({ length: 20 }, (_, i) => [
      id(i + 1),
      { ...settled(i + 1), inventoried: true, nativeSettled: true },
    ]),
  );
  expect(Object.keys(compactConsoleReceipts(native))).toHaveLength(16);
});

test("failed localStorage handoffs cannot accumulate full terminal views or discard opaque IDs", async () => {
  const receipts = Object.fromEntries(
    Array.from({ length: 40 }, (_, i) => [
      id(i + 1),
      { clientRequestId: id(i + 1), lookupPending: true },
    ]),
  );
  localStorage.setItem(key, JSON.stringify(Object.keys(receipts)));
  useStudio.setState({ snowflakeConsoleReceipts: receipts });
  const raw = localStorage.getItem(key);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("quota exhausted");
  });
  for (let n = 1; n <= 40; n++) {
    const receipt = settled(n);
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
      kind: "started",
      view: receipt.view!,
    });
    vi.mocked(client.call).mockResolvedValue(
      receiptSummary({ clientRequestId: id(n), runId: id(100 + n), sequence: String(n) }),
    );
    await useStudio.getState().refreshSnowflakeConsole(id(n));
    await expect(useStudio.getState().handoffSnowflakeConsoleReceipt(id(n))).rejects.toThrow(
      /quota/,
    );
  }
  const current = useStudio.getState().snowflakeConsoleReceipts;
  expect(Object.keys(current)).toHaveLength(40);
  expect(Object.values(current).filter((receipt) => receipt.view)).toHaveLength(16);
  expect(localStorage.getItem(key)).toBe(raw);
  expect(
    Object.values(current)
      .filter((receipt) => !receipt.view)
      .every((receipt) => receipt.settledProof !== undefined),
  ).toBe(true);
  expect(
    vi
      .mocked(client.call)
      .mock.calls.every(([command]) => command === "snowflake_console_receipt_summary"),
  ).toBe(true);
  vi.restoreAllMocks();
  for (let n = 1; n <= 40; n++) {
    vi.mocked(client.call).mockResolvedValue(
      receiptSummary({ clientRequestId: id(n), runId: id(100 + n), sequence: String(n) }),
    );
    await useStudio.getState().handoffSnowflakeConsoleReceipt(id(n));
  }
  expect(restoredConsoleReceipts()).toEqual({});
  expect(Object.keys(useStudio.getState().snowflakeConsoleReceipts)).toHaveLength(16);
});

test("a later missing or contradictory lookup keeps derived original proof held", async () => {
  const receipt = settled();
  const { view: _view, ...identity } = receipt;
  useStudio.setState({
    snowflakeConsoleReceipts: {
      [receipt.clientRequestId]: { ...identity, settledProof: { revision: "4", recordedAt: "1" } },
    },
  });
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
  await useStudio.getState().refreshSnowflakeConsole(receipt.clientRequestId);
  expect(
    consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[receipt.clientRequestId]),
  ).toBe(true);
  expect(
    useStudio.getState().snowflakeConsoleReceipts[receipt.clientRequestId].settledProof,
  ).toEqual({ revision: "4", recordedAt: "1" });
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "notAdmitted",
    clientRequestId: receipt.clientRequestId,
    sessionId: receipt.sessionId!,
    planDigest: receipt.planDigest!,
    code: "approvalRequired",
  });
  await useStudio.getState().refreshSnowflakeConsole(receipt.clientRequestId);
  expect(
    useStudio.getState().snowflakeConsoleReceipts[receipt.clientRequestId].decisionConflict,
  ).toBe(true);
  expect(
    consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[receipt.clientRequestId]),
  ).toBe(true);
});

test("lost row-release response preserves state until exact lookup removes released rows from every tab", async () => {
  const receipt = settled();
  const result = {
    id: "retained-result",
    runId: receipt.runId!,
    statementIndex: 0,
    columns: [],
    retainedRows: 1,
    status: "complete" as const,
    table: null,
  };
  receipt.view!.results = [result];
  const first = useStudio.getState().openQuery();
  const second = useStudio.getState().openQuery();
  for (const tabId of [first, second])
    useStudio.getState().updateTab(tabId, { results: [result], activeResultId: result.id });
  useStudio.setState({
    snowflakeConsoleReceipts: { [receipt.clientRequestId]: receipt },
    snowflakeConsoleResultOwners: { [result.id]: receipt.runId! },
  });
  vi.mocked(client.call).mockRejectedValue(new Error("release response lost"));
  await expect(
    useStudio.getState().releaseSnowflakeConsoleResults(receipt.clientRequestId),
  ).rejects.toThrow(/lost/);
  expect(useStudio.getState().tabs.every((tab) => tab.results.length === 1)).toBe(true);
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: { ...receipt.view!, results: [] },
  });
  await useStudio.getState().refreshSnowflakeConsole(receipt.clientRequestId);
  expect(
    useStudio
      .getState()
      .tabs.every((tab) => tab.results.length === 0 && tab.activeResultId === null),
  ).toBe(true);
  expect(useStudio.getState().snowflakeConsoleResultOwners).toEqual({});
  expect(vi.mocked(client.call).mock.calls).toEqual([
    ["release_snowflake_console_results", { clientRequestId: receipt.clientRequestId }],
  ]);
});

test("a delayed running lookup cannot erase a newer result event or its independent owner", async () => {
  vi.spyOn(crypto, "randomUUID").mockReturnValue(clientId);
  useStudio.getState().setSession(session);
  const tabId = useStudio.getState().openQuery("profile", "SELECT 1; SELECT 2");
  const first = {
    id: "result-first",
    runId,
    statementIndex: 0,
    columns: [],
    retainedRows: 1,
    status: "complete" as const,
    table: null,
  };
  const second = { ...first, id: "result-second", statementIndex: 1 };
  let emit!: Parameters<typeof client.startSnowflakeConsole>[1];
  vi.mocked(client.startSnowflakeConsole).mockImplementation(async (_, callback) => {
    emit = callback;
    return { kind: "started", runId };
  });
  let resolve!: (value: Awaited<ReturnType<typeof client.snowflakeConsoleRequest>>) => void;
  vi.mocked(client.snowflakeConsoleRequest).mockReturnValue(
    new Promise((done) => {
      resolve = done;
    }),
  );
  const starting = useStudio.getState().startSnowflakeConsole(tabId, plan, "approval");
  await waitFor(() => expect(client.snowflakeConsoleRequest).toHaveBeenCalledOnce());
  emit({ type: "result", result: second });
  await waitFor(() => expect(useStudio.getState().tabs[0].results).toContainEqual(second));
  const stale = receiptView({ results: [first] });
  stale.run.state = "running";
  stale.run.terminal = null;
  resolve({ kind: "started", view: stale });
  await starting;
  expect(useStudio.getState().snowflakeConsoleResultOwners[second.id]).toBe(runId);
  expect(useStudio.getState().tabs[0].results).toContainEqual(second);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view!.results).toContainEqual(
    second,
  );
});

test("row release reserves the lookup lane until native release and local cleanup finish", async () => {
  const receipt = settled();
  const result = {
    id: "release-race-result",
    runId: receipt.runId!,
    statementIndex: 0,
    columns: [],
    retainedRows: 1,
    status: "complete" as const,
    table: null,
  };
  receipt.view!.results = [result];
  const tabId = useStudio.getState().openQuery();
  useStudio.getState().updateTab(tabId, { results: [result], activeResultId: result.id });
  useStudio.setState({
    snowflakeConsoleReceipts: { [receipt.clientRequestId]: receipt },
    snowflakeConsoleResultOwners: { [result.id]: receipt.runId! },
  });
  let finishRelease!: () => void;
  vi.mocked(client.call).mockReturnValue(
    new Promise<void>((resolve) => {
      finishRelease = resolve;
    }),
  );
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: { ...receipt.view!, results: [] },
  });
  const releasing = useStudio.getState().releaseSnowflakeConsoleResults(receipt.clientRequestId);
  await useStudio.getState().refreshSnowflakeConsole(receipt.clientRequestId);
  const lookupsDuringRelease = vi.mocked(client.snowflakeConsoleRequest).mock.calls.length;
  finishRelease();
  await releasing;
  expect(lookupsDuringRelease).toBe(0);
  expect(client.snowflakeConsoleRequest).toHaveBeenCalledOnce();
  expect(useStudio.getState().tabs[0].results).toEqual([]);
  expect(useStudio.getState().snowflakeConsoleResultOwners).toEqual({});
});

test.each([false, true])(
  "confirmed row release suppresses queued and late results after lookup loss=%s",
  async (loseLookup) => {
    vi.spyOn(crypto, "randomUUID").mockReturnValue(clientId);
    let frame!: FrameRequestCallback;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frame = callback;
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
    useStudio.getState().setSession(session);
    const tabId = useStudio.getState().openQuery("profile", "SELECT 1");
    const result = {
      id: "queued-release-result",
      runId,
      statementIndex: 0,
      columns: [],
      retainedRows: 1,
      status: "complete" as const,
      table: null,
    };
    let emit!: Parameters<typeof client.startSnowflakeConsole>[1];
    vi.mocked(client.startSnowflakeConsole).mockImplementation(async (_, callback) => {
      emit = callback;
      callback({ type: "started", runId });
      callback({ type: "result", result });
      return { kind: "started", runId };
    });
    const view = receiptView({ results: [result] });
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({ kind: "started", view });
    await useStudio.getState().startSnowflakeConsole(tabId, plan, "approval");
    expect(useStudio.getState().tabs[0].results).toEqual([result]);
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
      kind: "started",
      view: { ...view, results: [] },
    });
    await useStudio.getState().releaseSnowflakeConsoleResults(clientId);
    if (loseLookup) {
      vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
      await useStudio.getState().refreshSnowflakeConsole(clientId);
      expect(useStudio.getState().snowflakeConsoleReceipts[clientId].lookupPending).toBe(true);
    }
    frame(0);
    emit({ type: "result", result });
    emit({ type: "progress", resultId: result.id, retainedRows: 2 });
    emit({ type: "finished", runId, status: "completed", elapsedMs: 10 });
    expect(useStudio.getState().tabs[0].results).toEqual([]);
    expect(useStudio.getState().snowflakeConsoleResultOwners).toEqual({});
    expect(useStudio.getState().tabs[0].status).toBe("completed");
    expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view!.run.terminal).toEqual(
      view.run.terminal,
    );
  },
);

test("settled native refusals remain inspectable within the shared detail cache", async () => {
  const receipts = Object.fromEntries(
    Array.from({ length: 24 }, (_, n) => {
      const receipt = settled(n + 1);
      const { view: _view, runId: _runId, ...identity } = receipt;
      return [
        receipt.clientRequestId,
        {
          ...identity,
          inventoried: true,
          nativeSettled: true,
          notAdmitted: "approvalRequired" as const,
        },
      ];
    }),
  );
  const compacted = compactConsoleReceipts(receipts);
  expect(Object.keys(compacted)).toHaveLength(16);
  expect(compacted[id(24)].notAdmitted).toBe("approvalRequired");
  expect(compacted[id(1)]).toBeUndefined();
  expect(compactConsoleReceipts(compacted)).toEqual(compacted);
});

test("looking up an old refusal keeps its detail ahead of older positive cache entries", async () => {
  const positives = Array.from({ length: 16 }, (_, n) => ({
    ...settled(n + 1),
    inventoried: true,
    nativeSettled: true,
  }));
  const old = settled(24);
  const { view: _view, runId: _runId, ...identity } = old;
  useStudio.setState({
    snowflakeConsoleReceipts: {
      ...Object.fromEntries(positives.map((receipt) => [receipt.clientRequestId, receipt])),
      [old.clientRequestId]: {
        ...identity,
        lastAccessed: 0,
        inventoried: true,
        nativeSettled: true,
        notAdmitted: "approvalRequired",
      },
    },
  });
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "notAdmitted",
    clientRequestId: old.clientRequestId,
    sessionId: old.sessionId!,
    planDigest: old.planDigest!,
    code: "approvalRequired",
  });
  await useStudio.getState().refreshSnowflakeConsole(old.clientRequestId);
  const compacted = useStudio.getState().snowflakeConsoleReceipts;
  expect(Object.keys(compacted)).toHaveLength(16);
  expect(compacted[old.clientRequestId].notAdmitted).toBe("approvalRequired");
  expect(compacted[id(1)]).toBeUndefined();
});
