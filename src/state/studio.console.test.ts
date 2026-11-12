import { beforeEach, afterEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type { QueryEvent, ResultInfo } from "../ipc/generated";
import {
  useStudio,
  revokeSchemaSelectionBeforeWork,
  observeTemplateSessionContext,
  isConsoleOwnedResult,
} from "./studio";
import { consoleReceiptHeld, restoredConsoleReceipts } from "../features/studio/SnowflakeConsole";
import {
  clientId,
  runId,
  session,
  plan,
  receiptView,
} from "../features/studio/SnowflakeConsole.test-fixtures";
vi.mock("../ipc/client", () => ({
  client: { call: vi.fn(), startSnowflakeConsole: vi.fn(), snowflakeConsoleRun: vi.fn() },
  messageOf: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
const initial = { ...useStudio.getState() };
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { resolve, promise };
}
function setup() {
  useStudio.getState().setSession(session);
  return useStudio.getState().openQuery("profile", "SELECT :value; COMMIT");
}
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.spyOn(crypto, "randomUUID").mockReturnValue(clientId);
  useStudio.setState(
    {
      ...initial,
      tabs: [],
      sessions: {},
      sessionPool: {},
      consoleSessions: {},
      snowflakeConsoleReceipts: {},
    },
    true,
  );
  vi.mocked(client.call).mockResolvedValue(null);
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(receiptView());
  vi.mocked(client.startSnowflakeConsole).mockResolvedValue({ kind: "started", runId });
});
afterEach(() => {
  vi.restoreAllMocks();
  useStudio.setState(initial, true);
  localStorage.clear();
});
test("lost start response reconciles the same durable request once without resending SQL", async () => {
  const tab = setup();
  vi.mocked(client.startSnowflakeConsole).mockRejectedValue(new Error("response lost"));
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  expect(client.startSnowflakeConsole).toHaveBeenCalledTimes(1);
  expect(client.startSnowflakeConsole).toHaveBeenCalledWith(
    expect.objectContaining({
      clientRequestId: clientId,
      planDigest: plan.digest,
      approval: "approval",
    }),
    expect.any(Function),
  );
  expect(client.snowflakeConsoleRun).toHaveBeenCalledWith(clientId);
  expect(useStudio.getState().tabs[0].status).toBe("stopped");
  expect(
    useStudio.getState().snowflakeConsoleReceipts[clientId].view?.run.steps[0].outcome?.kind,
  ).toBe("acknowledged");
  expect(JSON.stringify(restoredConsoleReceipts())).not.toContain("immutable");
  expect(Object.keys(restoredConsoleReceipts())).toEqual([clientId]);
  expect(client.call).not.toHaveBeenCalled();
});
test("null lookup retains the correlation and blocks a second start", async () => {
  const tab = setup();
  vi.mocked(client.startSnowflakeConsole).mockRejectedValue(new Error("lost"));
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(null);
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  await expect(useStudio.getState().startSnowflakeConsole(tab, plan, "again")).rejects.toThrow(
    /Reconcile/,
  );
  expect(client.startSnowflakeConsole).toHaveBeenCalledTimes(1);
  expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(true);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].error).toContain(
    "Submission is not disproved",
  );
});
test("terminal pending recording blocks another run while preserving the original ACK", async () => {
  const tab = setup();
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(receiptView({ pendingRecording: true }));
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  await expect(useStudio.getState().startSnowflakeConsole(tab, plan, "again")).rejects.toThrow(
    /Reconcile/,
  );
  expect(useStudio.getState().tabs[0].status).toBe("running");
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view?.run.terminal?.reason).toBe(
    "resultLimit",
  );
});
test("cancellation before start response recovers the original run ID without a replay", async () => {
  const tab = setup();
  const start = deferred<{ kind: "started"; runId: string }>();
  vi.mocked(client.startSnowflakeConsole).mockReturnValue(start.promise);
  const view = receiptView();
  view.run.state = "running";
  view.run.terminal = null;
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(view);
  const work = useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  await useStudio.getState().cancelSnowflakeConsole(clientId);
  expect(client.call).toHaveBeenCalledWith("cancel_run", { runId });
  start.resolve({ kind: "started", runId });
  await work;
  expect(client.startSnowflakeConsole).toHaveBeenCalledTimes(1);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].cancellationRequested).toBe(true);
});
test.each(["edit", "rebind", "close"])(
  "%s keeps the receipt and prevents late result publication",
  async (action) => {
    const tab = setup();
    const lookup = deferred<ReturnType<typeof receiptView>>();
    vi.mocked(client.snowflakeConsoleRun).mockReturnValue(lookup.promise);
    const work = useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    await vi.waitFor(() => expect(client.snowflakeConsoleRun).toHaveBeenCalledOnce());
    if (action === "edit")
      useStudio.getState().updateTab(tab, { revision: 99, sql: "SELECT 'new'", status: "idle" });
    else if (action === "rebind")
      useStudio.getState().updateTab(tab, { connectionId: null, status: "idle" });
    else useStudio.getState().closeTab(tab);
    const view = receiptView();
    view.results = [
      {
        id: "result",
        runId,
        statementIndex: 0,
        columns: [],
        retainedRows: 1,
        status: "complete",
        table: null,
      },
    ];
    lookup.resolve(view);
    await work;
    expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view?.run.metadata.runId).toBe(
      runId,
    );
    expect(useStudio.getState().tabs.flatMap((t) => t.results)).toEqual([]);
    expect(client.startSnowflakeConsole).toHaveBeenCalledTimes(1);
  },
);
test("foreign or stale lookup cannot replace retained original evidence", async () => {
  const tab = setup();
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  const original = useStudio.getState().snowflakeConsoleReceipts[clientId].view;
  const stale = receiptView();
  stale.run.revision = 1;
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(stale);
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view).toBe(original);
  expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(true);
  const foreign = receiptView();
  foreign.run.metadata.sessionId = "different";
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(foreign);
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view).toBe(original);
});
test("console sessions suppress metadata, context and transaction probes including stale callers after retirement", async () => {
  setup();
  await useStudio.getState().refreshCatalog("profile");
  expect(await useStudio.getState().refreshSessionContexts(session.id)).toBe(false);
  expect(() => revokeSchemaSelectionBeforeWork(session.id)).toThrow(/reviewed SQL console/);
  await expect(observeTemplateSessionContext(session.id)).rejects.toThrow(/reviewed SQL console/);
  await expect(useStudio.getState().commitTransaction(session.id)).rejects.toThrow(
    /reviewed SQL console/,
  );
  await expect(useStudio.getState().rollbackTransaction(session.id)).rejects.toThrow(
    /reviewed SQL console/,
  );
  useStudio.getState().retireSession(session.id);
  expect(() => revokeSchemaSelectionBeforeWork(session.id)).toThrow(/reviewed SQL console/);
  expect(client.call).not.toHaveBeenCalled();
});

test("restored receipt with unresolved scope holds console starts until its original session is known", async () => {
  const tab = setup();
  useStudio.setState({
    snowflakeConsoleReceipts: { [clientId]: { clientRequestId: clientId, lookupPending: true } },
  });
  await expect(useStudio.getState().startSnowflakeConsole(tab, plan, "approval")).rejects.toThrow(
    /Reconcile/,
  );
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
  expect(client.call).not.toHaveBeenCalled();
});
test("stale table pagination and generated browsing callers reject before SQL on a console session", async () => {
  const tab = setup();
  useStudio.getState().updateTab(tab, {
    kind: "table",
    table: { schema: "PUBLIC", name: "OLD" },
    dataSessionId: session.id,
    hasMore: true,
    error: null,
  });
  await expect(useStudio.getState().refreshTable(tab)).rejects.toThrow(/reviewed SQL console/);
  await expect(useStudio.getState().loadNextPage(tab)).rejects.toThrow(/reviewed SQL console/);
  await expect(
    useStudio
      .getState()
      .setTableQuery(tab, { sort: [], filters: [], whereClause: "true", orderClause: null }),
  ).rejects.toThrow(/reviewed SQL console/);
  expect(client.call).not.toHaveBeenCalled();
});

function retainedResult(): ResultInfo {
  return {
    id: "retained-result",
    runId,
    statementIndex: 0,
    columns: [],
    retainedRows: 1,
    status: "complete",
    table: null,
  };
}
test.each(["edit", "close", "rebind"])(
  "late console Result after %s remains owned by the receipt",
  async (action) => {
    const tab = setup();
    const start = deferred<{ kind: "started"; runId: string }>();
    let emit!: (event: QueryEvent) => void;
    vi.mocked(client.startSnowflakeConsole).mockImplementation((_request, onEvent) => {
      emit = onEvent;
      return start.promise;
    });
    const work = useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    if (action === "close") useStudio.getState().closeTab(tab);
    else
      useStudio
        .getState()
        .updateTab(
          tab,
          action === "edit"
            ? { revision: 99, sql: "SELECT 'new'" }
            : { connectionId: null, revision: 99 },
        );
    const result = retainedResult();
    emit({ type: "result", result });
    emit({ type: "finished", runId, status: "completed", elapsedMs: 1 });
    expect(isConsoleOwnedResult(result)).toBe(true);
    expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
    vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(receiptView({ results: [result] }));
    start.resolve({ kind: "started", runId });
    await work;
    await useStudio.getState().openSnowflakeConsoleResults(clientId);
    expect(
      useStudio.getState().tabs.find((t) => t.title === "Console retained results")?.results,
    ).toEqual([result]);
    expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
    expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
  },
);
test("closing and clearing console tabs never releases receipt-owned native rows", async () => {
  const tab = setup();
  const result = retainedResult();
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(receiptView({ results: [result] }));
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  useStudio.getState().closeTab(tab);
  expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
  await useStudio.getState().openSnowflakeConsoleResults(clientId);
  const reopened = useStudio.getState().activeTabId!;
  // A second reviewed start may clear tab publication, but the first receipt owns its rows.
  vi.spyOn(crypto, "randomUUID").mockReturnValue("33333333-3333-4333-8333-333333333333");
  vi.mocked(client.startSnowflakeConsole).mockRejectedValue(new Error("lost"));
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(null);
  await useStudio.getState().startSnowflakeConsole(reopened, plan, "approval");
  expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
});
test("opening restored rows uses only receipt lookup and does not bind a replacement session", async () => {
  setup();
  useStudio.setState({
    snowflakeConsoleReceipts: { [clientId]: { clientRequestId: clientId, lookupPending: true } },
    sessions: { profile: { ...session, id: "replacement" } },
    tabs: [],
  });
  const result = retainedResult();
  vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(receiptView({ results: [result] }));
  await useStudio.getState().openSnowflakeConsoleResults(clientId);
  const tab = useStudio.getState().tabs[0];
  expect(tab).toMatchObject({
    connectionId: null,
    sql: "",
    results: [result],
    dataSessionId: session.id,
  });
  expect(client.snowflakeConsoleRun).toHaveBeenCalledWith(clientId);
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
  expect(client.call).not.toHaveBeenCalled();
});
test("receipt without native rows does not open a fabricated empty result", async () => {
  useStudio.setState({
    snowflakeConsoleReceipts: { [clientId]: { clientRequestId: clientId, lookupPending: true } },
  });
  await expect(useStudio.getState().openSnowflakeConsoleResults(clientId)).rejects.toThrow(
    /does not mean zero/,
  );
  expect(useStudio.getState().tabs).toEqual([]);
  expect(client.call).not.toHaveBeenCalled();
});

test("matching native NotAdmitted settles only this identity and permits a fresh review without replay", async () => {
  const tab = setup();
  vi.mocked(client.startSnowflakeConsole).mockResolvedValue({
    kind: "notAdmitted",
    clientRequestId: clientId,
    sessionId: session.id,
    planDigest: plan.digest,
    code: "approvalExpired",
  });
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
  expect(receipt.notAdmitted).toBe("approvalExpired");
  expect(consoleReceiptHeld(receipt)).toBe(false);
  expect(restoredConsoleReceipts()).toEqual({});
  expect(client.snowflakeConsoleRun).not.toHaveBeenCalled();
  expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  expect(useStudio.getState().tabs[0].status).toBe("failed");
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  await useStudio.getState().cancelSnowflakeConsole(clientId);
  expect(client.call).not.toHaveBeenCalled();
  expect(client.snowflakeConsoleRun).not.toHaveBeenCalled();
});
test.each(["foreign", "afterEvent"])(
  "%s negative response cannot settle possible original admission",
  async (caseName) => {
    const tab = setup();
    vi.mocked(client.snowflakeConsoleRun).mockResolvedValue(null);
    vi.mocked(client.startSnowflakeConsole).mockImplementation(async (_request, emit) => {
      if (caseName === "afterEvent") emit({ type: "started", runId });
      return {
        kind: "notAdmitted",
        clientRequestId: caseName === "foreign" ? "foreign" : clientId,
        sessionId: session.id,
        planDigest: plan.digest,
        code: "approvalExpired",
      };
    });
    await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
    expect(receipt.notAdmitted).toBeUndefined();
    expect(consoleReceiptHeld(receipt)).toBe(true);
    expect(Object.keys(restoredConsoleReceipts())).toEqual([clientId]);
    expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  },
);
