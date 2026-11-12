import { beforeEach, afterEach, expect, test, vi } from "vitest";
import { client } from "../ipc/client";
import type { QueryEvent, ResultInfo, SnowflakeConsoleRequestLookup } from "../ipc/generated";
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
  client: { call: vi.fn(), startSnowflakeConsole: vi.fn(), snowflakeConsoleRequest: vi.fn() },
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: receiptView(),
  });
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
  expect(client.snowflakeConsoleRequest).toHaveBeenCalledWith(clientId);
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: receiptView({ pendingRecording: true }),
  });
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({ kind: "started", view: view });
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
    const lookup = deferred<SnowflakeConsoleRequestLookup>();
    vi.mocked(client.snowflakeConsoleRequest).mockReturnValue(lookup.promise);
    const work = useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    await vi.waitFor(() => expect(client.snowflakeConsoleRequest).toHaveBeenCalledOnce());
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
    lookup.resolve({ kind: "started", view });
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({ kind: "started", view: stale });
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].view).toBe(original);
  expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(true);
  const foreign = receiptView();
  foreign.run.metadata.sessionId = "different";
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({ kind: "started", view: foreign });
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
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
      kind: "started",
      view: receiptView({ results: [result] }),
    });
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: receiptView({ results: [result] }),
  });
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  useStudio.getState().closeTab(tab);
  expect(client.call).not.toHaveBeenCalledWith("release_result", { resultId: result.id });
  await useStudio.getState().openSnowflakeConsoleResults(clientId);
  const reopened = useStudio.getState().activeTabId!;
  // A second reviewed start may clear tab publication, but the first receipt owns its rows.
  vi.spyOn(crypto, "randomUUID").mockReturnValue("33333333-3333-4333-8333-333333333333");
  vi.mocked(client.startSnowflakeConsole).mockRejectedValue(new Error("lost"));
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
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
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: receiptView({ results: [result] }),
  });
  await useStudio.getState().openSnowflakeConsoleResults(clientId);
  const tab = useStudio.getState().tabs[0];
  expect(tab).toMatchObject({
    connectionId: null,
    sql: "",
    results: [result],
    dataSessionId: session.id,
  });
  expect(client.snowflakeConsoleRequest).toHaveBeenCalledWith(clientId);
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
  expect(Object.keys(restoredConsoleReceipts())).toEqual([clientId]);
  expect(client.snowflakeConsoleRequest).not.toHaveBeenCalled();
  expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  expect(useStudio.getState().tabs[0].status).toBe("failed");
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "notAdmitted",
    clientRequestId: clientId,
    sessionId: session.id,
    planDigest: plan.digest,
    code: "approvalExpired",
  });
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  await useStudio.getState().cancelSnowflakeConsole(clientId);
  expect(client.call).not.toHaveBeenCalled();
  expect(client.snowflakeConsoleRequest).toHaveBeenCalledOnce();
});
test.each(["foreign", "afterEvent"])(
  "%s negative response cannot settle possible original admission",
  async (caseName) => {
    const tab = setup();
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
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

function rejection(): Extract<SnowflakeConsoleRequestLookup, { kind: "notAdmitted" }> {
  return {
    kind: "notAdmitted",
    clientRequestId: clientId,
    sessionId: session.id,
    planDigest: plan.digest,
    code: "approvalExpired",
  };
}
test("lost refusal response is recovered from the authoritative request decision without resending", async () => {
  const tab = setup();
  vi.mocked(client.startSnowflakeConsole).mockRejectedValue(new Error("lost refusal response"));
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(rejection());
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
  expect(receipt.notAdmitted).toBe("approvalExpired");
  expect(consoleReceiptHeld(receipt)).toBe(false);
  expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  expect(client.snowflakeConsoleRequest).toHaveBeenCalledWith(clientId);
  expect(client.call).not.toHaveBeenCalled();
  expect(Object.keys(restoredConsoleReceipts())).toEqual([clientId]);
});
test("restart restores only the opaque identity and recovers its negative proof without session or SQL", async () => {
  const tab = setup();
  vi.mocked(client.startSnowflakeConsole).mockResolvedValue(rejection());
  await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
  const restored = restoredConsoleReceipts();
  expect(restored[clientId]).toEqual({ clientRequestId: clientId, lookupPending: true });
  expect(consoleReceiptHeld(restored[clientId])).toBe(true);
  useStudio.setState({ tabs: [], sessions: {}, snowflakeConsoleReceipts: restored });
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(rejection());
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId]).toMatchObject({
    sessionId: session.id,
    planDigest: plan.digest,
    notAdmitted: "approvalExpired",
    lookupPending: false,
  });
  expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(false);
  expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  expect(client.call).not.toHaveBeenCalled();
  expect(localStorage.getItem("joinery.snowflakeConsoleReceipts.v1")).toBe(
    JSON.stringify([clientId]),
  );
});
test.each(["client", "session", "digest", "positiveRun"])(
  "%s contradiction cannot replace retained positive proof with a refusal",
  async (which) => {
    const tab = setup();
    await useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    const original = useStudio.getState().snowflakeConsoleReceipts[clientId].view;
    const decision = rejection();
    if (which === "client") decision.clientRequestId = "33333333-3333-4333-8333-333333333333";
    if (which === "session") decision.sessionId = "other-session";
    if (which === "digest") decision.planDigest = "b".repeat(64);
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(decision);
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
    expect(receipt.view).toBe(original);
    expect(receipt.notAdmitted).toBeUndefined();
    expect(consoleReceiptHeld(receipt)).toBe(true);
    expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
  },
);
test.each(["positive", "changedCode", "null", "error"])(
  "%s after negative proof keeps original proof but holds reconciliation",
  async (which) => {
    useStudio.setState({
      snowflakeConsoleReceipts: {
        [clientId]: {
          clientRequestId: clientId,
          sessionId: session.id,
          planDigest: plan.digest,
          notAdmitted: "approvalExpired",
          lookupPending: false,
        },
      },
    });
    if (which === "positive")
      vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
        kind: "started",
        view: receiptView(),
      });
    else if (which === "changedCode")
      vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
        ...rejection(),
        code: "policyBlocked",
      });
    else if (which === "null") vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
    else
      vi.mocked(client.snowflakeConsoleRequest).mockRejectedValue(new Error("lookup unavailable"));
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
    expect(receipt.notAdmitted).toBe("approvalExpired");
    expect(receipt.view).toBeUndefined();
    expect(consoleReceiptHeld(receipt)).toBe(true);
    expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
    expect(client.call).not.toHaveBeenCalled();
  },
);

test.each(["client", "session", "digest"])(
  "a %s-mismatched negative cannot settle a request with no positive evidence",
  async (which) => {
    useStudio.setState({
      snowflakeConsoleReceipts: {
        [clientId]: {
          clientRequestId: clientId,
          sessionId: session.id,
          planDigest: plan.digest,
          lookupPending: true,
        },
      },
    });
    const decision = rejection();
    if (which === "client") decision.clientRequestId = "33333333-3333-4333-8333-333333333333";
    if (which === "session") decision.sessionId = "other-session";
    if (which === "digest") decision.planDigest = "b".repeat(64);
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(decision);
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
    expect(receipt.notAdmitted).toBeUndefined();
    expect(receipt.sessionId).toBe(session.id);
    expect(receipt.planDigest).toBe(plan.digest);
    expect(consoleReceiptHeld(receipt)).toBe(true);
    expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
    expect(client.call).not.toHaveBeenCalled();
  },
);

test("negative then positive then original negative preserves a sticky contradiction hold", async () => {
  useStudio.setState({
    snowflakeConsoleReceipts: {
      [clientId]: {
        clientRequestId: clientId,
        sessionId: session.id,
        planDigest: plan.digest,
        notAdmitted: "approvalExpired",
        lookupPending: false,
      },
    },
  });
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue({
    kind: "started",
    view: receiptView(),
  });
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  expect(useStudio.getState().snowflakeConsoleReceipts[clientId].decisionConflict).toBe(true);
  vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(rejection());
  await useStudio.getState().refreshSnowflakeConsole(clientId);
  const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
  expect(receipt.notAdmitted).toBe("approvalExpired");
  expect(receipt.view).toBeUndefined();
  expect(receipt.decisionConflict).toBe(true);
  expect(consoleReceiptHeld(receipt)).toBe(true);
  expect(client.startSnowflakeConsole).not.toHaveBeenCalled();
  expect(client.call).not.toHaveBeenCalled();
});
test.each(["null", "error"])(
  "a %s lookup can recover a previous negative only through a matching decision",
  async (which) => {
    useStudio.setState({
      snowflakeConsoleReceipts: {
        [clientId]: {
          clientRequestId: clientId,
          sessionId: session.id,
          planDigest: plan.digest,
          notAdmitted: "approvalExpired",
          lookupPending: false,
        },
      },
    });
    if (which === "null") vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(null);
    else
      vi.mocked(client.snowflakeConsoleRequest).mockRejectedValue(new Error("temporary failure"));
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(true);
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(rejection());
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(false);
    expect(
      useStudio.getState().snowflakeConsoleReceipts[clientId].decisionConflict,
    ).toBeUndefined();
  },
);

test.each(["response", "event"])(
  "an in-flight positive %s after a recovered negative latches the contradiction",
  async (which) => {
    const tab = setup();
    const start = deferred<{ kind: "started"; runId: string }>();
    let emit!: (event: QueryEvent) => void;
    vi.mocked(client.startSnowflakeConsole).mockImplementation((_request, onEvent) => {
      emit = onEvent;
      return start.promise;
    });
    const work = useStudio.getState().startSnowflakeConsole(tab, plan, "approval");
    vi.mocked(client.snowflakeConsoleRequest).mockResolvedValue(rejection());
    await useStudio.getState().refreshSnowflakeConsole(clientId);
    expect(consoleReceiptHeld(useStudio.getState().snowflakeConsoleReceipts[clientId])).toBe(false);
    if (which === "event") emit({ type: "result", result: retainedResult() });
    start.resolve({ kind: "started", runId });
    await work;
    const receipt = useStudio.getState().snowflakeConsoleReceipts[clientId];
    expect(receipt.notAdmitted).toBe("approvalExpired");
    expect(receipt.decisionConflict).toBe(true);
    expect(consoleReceiptHeld(receipt)).toBe(true);
    if (which === "event") expect(isConsoleOwnedResult(retainedResult())).toBe(true);
    expect(client.startSnowflakeConsole).toHaveBeenCalledOnce();
    expect(client.call).not.toHaveBeenCalledWith("release_result", {
      resultId: retainedResult().id,
    });
  },
);
