import { beforeEach, expect, test, vi } from "vitest";
import { beginEventBatch, useStudio } from "./studio";
import type { AppError, ColumnInfo, QueryEvent, SessionInfo } from "../ipc/generated";

vi.mock("../ipc/client", () => ({
  client: { call: vi.fn() },
  messageOf: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

function makeCol(name: string): ColumnInfo {
  return {
    id: name,
    name,
    databaseType: "TEXT",
    kind: "text",
    nullable: true,
    primaryKey: false,
    foreignKey: false,
    editable: false,
    insertable: false,
    hasDefault: false,
  };
}

const session: SessionInfo = {
  id: "session",
  connectionId: "profile",
  engine: "postgres",
  serverVersion: "fixture",
  latencyMs: 0,
  encrypted: false,
  capabilities: {
    sql: true,
    transactions: true,
    cancellation: true,
    tableBrowsing: true,
    tableEditing: true,
    tableFiltering: true,
    processes: true,
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  useStudio.setState({
    bootstrap: null,
    tabs: [],
    activeTabId: null,
    closedTabs: [],
    tableLayouts: [],
    catalog: {},
    objects: {},
    sessions: { profile: session },
  });
});

test("adversarial: statementErrors persistence and clearing across consecutive runs", () => {
  const tabId = useStudio.getState().openQuery("profile");
  const tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  const err1: AppError = {
    code: "database",
    message: "Table does not exist",
  };

  // Run 1: has an error on statement index 1
  const run1Events: QueryEvent[] = [
    { type: "started", runId: "run-1" },
    { type: "error", error: err1, statementIndex: 1 },
    { type: "finished", runId: "run-1", status: "failed", elapsedMs: 12 },
  ];

  useStudio.getState().applyEvents(tabId, run1Events, tab.revision);

  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("failed");
  expect(currentTab.statementErrors?.[1]?.message).toBe("Table does not exist");
  expect(currentTab.error?.message).toBe("Table does not exist");

  // Run 2: starts clean and succeeds on a single statement
  const run2Events: QueryEvent[] = [
    { type: "started", runId: "run-2" },
    { type: "statementCompleted", runId: "run-2", index: 0, affectedRows: "1", elapsedMs: 5 },
    { type: "finished", runId: "run-2", status: "completed", elapsedMs: 5 },
  ];

  useStudio.getState().applyEvents(tabId, run2Events, currentTab.revision);

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  console.log("After Run 2 status:", currentTab.status);
  console.log("After Run 2 error:", currentTab.error);
  console.log("After Run 2 statementErrors:", currentTab.statementErrors);

  // An adversarial test: Run 2 succeeded, so it should not inherit statement errors from Run 1
  expect(currentTab.error).toBeNull();
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(0);
});

test("adversarial: activeQueryId must be cleared when query finishes", () => {
  const tabId = useStudio.getState().openQuery("profile");
  const tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Snowflake query starts and emits queryHandle
  const sfEvents: QueryEvent[] = [
    { type: "started", runId: "sf-run-1" },
    {
      type: "queryHandle",
      runId: "sf-run-1",
      statementIndex: 0,
      queryId: "01b89abc-snowflake-handle-1",
    },
    { type: "finished", runId: "sf-run-1", status: "completed", elapsedMs: 100 },
  ];

  useStudio.getState().applyEvents(tabId, sfEvents, tab.revision);

  const currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  console.log("After finish activeQueryId:", currentTab.activeQueryId);

  // When query is finished, activeQueryId must not linger as an active query handle
  expect(currentTab.activeQueryId).toBeNull();
});

test("adversarial: multi-statement failure followed by multi-statement success clears all statementErrors and resurrects zero error tabs/badges", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  const err0: AppError = { code: "database", message: "Syntax error in statement 0" };
  const err2: AppError = { code: "database", message: "Column not found in statement 2" };

  // Run 1: 3 statements. Stmt 0 fails, Stmt 1 succeeds, Stmt 2 fails.
  const run1Events: QueryEvent[] = [
    { type: "started", runId: "batch-run-1" },
    { type: "queryHandle", runId: "batch-run-1", statementIndex: 0, queryId: "sf-stmt-0" },
    { type: "error", error: err0, statementIndex: 0 },
    {
      type: "result",
      result: {
        id: "res-stmt-1",
        runId: "batch-run-1",
        statementIndex: 1,
        columns: [makeCol("col1")],
        status: "complete",
        retainedRows: 1,
        table: null,
      },
    },
    {
      type: "statementCompleted",
      runId: "batch-run-1",
      index: 1,
      affectedRows: "1",
      elapsedMs: 10,
    },
    { type: "error", error: err2, statementIndex: 2 },
    { type: "finished", runId: "batch-run-1", status: "failed", elapsedMs: 50 },
  ];

  useStudio.getState().applyEvents(tabId, run1Events, currentTab.revision);
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Assert Run 1 state
  expect(currentTab.status).toBe("failed");
  expect(currentTab.runId).toBeNull();
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.error?.message).toBe("Column not found in statement 2");
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(2);
  expect(currentTab.statementErrors?.[0]?.message).toBe("Syntax error in statement 0");
  expect(currentTab.statementErrors?.[2]?.message).toBe("Column not found in statement 2");

  // Verify simulated UI badge counts for Run 1
  const renderedErrorBadgesRun1 = Object.entries(currentTab.statementErrors ?? {});
  expect(renderedErrorBadgesRun1.length).toBe(2);
  expect(Boolean(currentTab.error)).toBe(true);

  // Run 2: starts clean and succeeds on 2 statements
  const run2Events: QueryEvent[] = [
    { type: "started", runId: "batch-run-2" },
    { type: "queryHandle", runId: "batch-run-2", statementIndex: 0, queryId: "sf-stmt-2-0" },
    {
      type: "result",
      result: {
        id: "res-2-0",
        runId: "batch-run-2",
        statementIndex: 0,
        columns: [makeCol("val")],
        status: "complete",
        retainedRows: 1,
        table: null,
      },
    },
    { type: "statementCompleted", runId: "batch-run-2", index: 0, affectedRows: "1", elapsedMs: 5 },
    {
      type: "result",
      result: {
        id: "res-2-1",
        runId: "batch-run-2",
        statementIndex: 1,
        columns: [makeCol("val2")],
        status: "complete",
        retainedRows: 1,
        table: null,
      },
    },
    { type: "statementCompleted", runId: "batch-run-2", index: 1, affectedRows: "1", elapsedMs: 8 },
    { type: "finished", runId: "batch-run-2", status: "completed", elapsedMs: 25 },
  ];

  useStudio.getState().applyEvents(tabId, run2Events, currentTab.revision);
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Assert Run 2 completely wiped out all statement errors, general error, and activeQueryId
  expect(currentTab.status).toBe("completed");
  expect(currentTab.runId).toBeNull();
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(0);

  // Assert simulated UI badge count is strictly 0 and cancel button is disabled
  const renderedErrorBadgesRun2 = Object.entries(currentTab.statementErrors ?? {});
  expect(renderedErrorBadgesRun2.length).toBe(0);
  expect(Boolean(currentTab.error)).toBe(false);
  expect(Boolean(currentTab.runId || currentTab.activeQueryId)).toBe(false);
});

test("adversarial: displaced statement failure does not retain stale statement errors from prior run", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  const err0: AppError = { code: "database", message: "Old statement 0 error" };
  const err2: AppError = { code: "database", message: "Old statement 2 error" };

  // Run 1: failures on index 0 and index 2
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "run-d-1" },
      { type: "error", error: err0, statementIndex: 0 },
      { type: "error", error: err2, statementIndex: 2 },
      { type: "finished", runId: "run-d-1", status: "failed", elapsedMs: 10 },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(2);

  // Run 2: ONLY statement 1 fails (statement 0 succeeds)
  const err1: AppError = { code: "database", message: "New statement 1 error" };
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "run-d-2" },
      { type: "statementCompleted", runId: "run-d-2", index: 0, affectedRows: "1", elapsedMs: 5 },
      { type: "error", error: err1, statementIndex: 1 },
      { type: "finished", runId: "run-d-2", status: "failed", elapsedMs: 15 },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("failed");
  expect(currentTab.error?.message).toBe("New statement 1 error");
  expect(currentTab.statementErrors?.[1]?.message).toBe("New statement 1 error");
  // Stale errors from Run 1 MUST be gone
  expect(currentTab.statementErrors?.[0]).toBeUndefined();
  expect(currentTab.statementErrors?.[2]).toBeUndefined();
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(1);
});

test("adversarial: cancelled query lifecycle strictly nulls activeQueryId and preserves no error badges", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Run 1 has an error
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "run-c-1" },
      { type: "error", error: { code: "database", message: "E1" }, statementIndex: 0 },
      { type: "finished", runId: "run-c-1", status: "failed", elapsedMs: 10 },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.statementErrors?.[0]).toBeDefined();

  // Run 2 starts, receives query handle, but is cancelled mid-flight
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "run-c-2" },
      { type: "queryHandle", runId: "run-c-2", statementIndex: 0, queryId: "sf-cancel-handle-99" },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("running");
  expect(currentTab.activeQueryId).toBe("sf-cancel-handle-99");
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});

  // Cancel arrives
  useStudio
    .getState()
    .applyEvents(
      tabId,
      [{ type: "finished", runId: "run-c-2", status: "cancelled", elapsedMs: 40 }],
      currentTab.revision,
    );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("cancelled");
  expect(currentTab.runId).toBeNull();
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
  expect(Boolean(currentTab.runId || currentTab.activeQueryId)).toBe(false);
});

test("adversarial: real-time chunked streaming of events across multiple applyEvents invocations", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Chunk 1: started + queryHandle
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "stream-run-1" },
      {
        type: "queryHandle",
        runId: "stream-run-1",
        statementIndex: 0,
        queryId: "sf-stream-handle",
      },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("running");
  expect(currentTab.activeQueryId).toBe("sf-stream-handle");

  // Chunk 2: progress + statementCompleted
  useStudio.getState().applyEvents(
    tabId,
    [
      {
        type: "statementCompleted",
        runId: "stream-run-1",
        index: 0,
        affectedRows: "10",
        elapsedMs: 15,
      },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.activeQueryId).toBe("sf-stream-handle");

  // Chunk 3: finished
  useStudio
    .getState()
    .applyEvents(
      tabId,
      [{ type: "finished", runId: "stream-run-1", status: "completed", elapsedMs: 25 }],
      currentTab.revision,
    );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("completed");
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.runId).toBeNull();
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
});

test("adversarial: resetConnectionResults purges statementErrors, activeQueryId, results, and resets status to idle", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Populate dirty running state with statement errors and query handle
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "disconnect-run" },
      {
        type: "queryHandle",
        runId: "disconnect-run",
        statementIndex: 0,
        queryId: "sf-disc-handle",
      },
      {
        type: "error",
        error: { code: "database", message: "Disconnection error" },
        statementIndex: 0,
      },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.activeQueryId).toBe("sf-disc-handle");
  expect(currentTab.statementErrors?.[0]).toBeDefined();

  // Trigger disconnect reset
  useStudio.getState().resetConnectionResults("profile");

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("idle");
  expect(currentTab.runId).toBeNull();
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
  expect(currentTab.results).toEqual([]);
});

test("adversarial: beginEventBatch flush preserves zero error leakage across consecutive failing then succeeding batches", () => {
  const tabId = useStudio.getState().openQuery("profile");

  // Batch 1: failing batch dispatched via beginEventBatch
  const pushBatch1 = beginEventBatch(tabId);
  pushBatch1({ type: "started", runId: "beb-run-1" });
  pushBatch1({
    type: "error",
    error: { code: "database", message: "Table missing in batch 1" },
    statementIndex: 0,
  });
  pushBatch1({ type: "finished", runId: "beb-run-1", status: "failed", elapsedMs: 15 });

  let tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(tab.status).toBe("failed");
  expect(tab.error?.message).toBe("Table missing in batch 1");
  expect(tab.statementErrors?.[0]?.message).toBe("Table missing in batch 1");
  expect(tab.runId).toBeNull();
  expect(tab.activeQueryId).toBeNull();

  // Batch 2: successful batch dispatched via beginEventBatch
  const pushBatch2 = beginEventBatch(tabId);
  pushBatch2({ type: "started", runId: "beb-run-2" });
  pushBatch2({
    type: "queryHandle",
    runId: "beb-run-2",
    statementIndex: 0,
    queryId: "sf-handle-batch2",
  });
  pushBatch2({
    type: "result",
    result: {
      id: "res-beb-2",
      runId: "beb-run-2",
      statementIndex: 0,
      columns: [makeCol("res")],
      status: "complete",
      retainedRows: 1,
      table: null,
    },
  });
  pushBatch2({
    type: "statementCompleted",
    runId: "beb-run-2",
    index: 0,
    affectedRows: "1",
    elapsedMs: 8,
  });
  pushBatch2({ type: "finished", runId: "beb-run-2", status: "completed", elapsedMs: 12 });

  tab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(tab.status).toBe("completed");
  expect(tab.error).toBeNull();
  expect(tab.statementErrors).toEqual({});
  expect(Object.keys(tab.statementErrors || {}).length).toBe(0);
  expect(tab.runId).toBeNull();
  expect(tab.activeQueryId).toBeNull();
});

test("adversarial: interleaving queryHandle with syntax errors and retry with multiple statements completely clears statementErrors", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Batch 1: 4 statements. Stmts 0, 2, 3 error out; Stmt 1 has handle.
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "interleave-1" },
      {
        type: "error",
        error: { code: "database", message: "Syntax error on 0" },
        statementIndex: 0,
      },
      { type: "queryHandle", runId: "interleave-1", statementIndex: 1, queryId: "handle-stmt-1" },
      {
        type: "error",
        error: { code: "database", message: "Type mismatch on 2" },
        statementIndex: 2,
      },
      {
        type: "error",
        error: { code: "database", message: "Division by zero on 3" },
        statementIndex: 3,
      },
      { type: "finished", runId: "interleave-1", status: "failed", elapsedMs: 30 },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("failed");
  expect(Object.keys(currentTab.statementErrors || {}).length).toBe(3);
  expect(currentTab.activeQueryId).toBeNull();

  // Now user starts Batch 2: observe state IMMEDIATELY after "started" event
  useStudio
    .getState()
    .applyEvents(tabId, [{ type: "started", runId: "interleave-2" }], currentTab.revision);

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  // Crucial invariant: as soon as new query starts, all prior errors MUST be wiped!
  expect(currentTab.status).toBe("running");
  expect(currentTab.runId).toBe("interleave-2");
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
  expect(currentTab.activeQueryId).toBeNull();

  // Batch 2 statements succeed
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "queryHandle", runId: "interleave-2", statementIndex: 0, queryId: "new-handle-0" },
      {
        type: "result",
        result: {
          id: "res-new-0",
          runId: "interleave-2",
          statementIndex: 0,
          columns: [makeCol("n0")],
          status: "complete",
          retainedRows: 5,
          table: null,
        },
      },
      {
        type: "statementCompleted",
        runId: "interleave-2",
        index: 0,
        affectedRows: "5",
        elapsedMs: 10,
      },
      { type: "finished", runId: "interleave-2", status: "completed", elapsedMs: 15 },
    ],
    currentTab.revision,
  );

  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("completed");
  expect(currentTab.error).toBeNull();
  expect(currentTab.statementErrors).toEqual({});
  expect(currentTab.runId).toBeNull();
  expect(currentTab.activeQueryId).toBeNull();
});

test("adversarial: activeQueryId lifecycle on query failure, success, and cancel with streaming results", () => {
  const tabId = useStudio.getState().openQuery("profile");
  let currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;

  // Case 1: Failure with queryHandle -> activeQueryId set during run, strictly null on finish
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "fail-handle-run" },
      {
        type: "queryHandle",
        runId: "fail-handle-run",
        statementIndex: 0,
        queryId: "sf-fail-handle-101",
      },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.activeQueryId).toBe("sf-fail-handle-101");

  useStudio.getState().applyEvents(
    tabId,
    [
      {
        type: "error",
        error: { code: "database", message: "Compilation failure" },
        statementIndex: 0,
      },
      { type: "finished", runId: "fail-handle-run", status: "failed", elapsedMs: 20 },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("failed");
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.runId).toBeNull();

  // Case 2: Success with queryHandle -> activeQueryId set during run, strictly null on finish
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "succ-handle-run" },
      {
        type: "queryHandle",
        runId: "succ-handle-run",
        statementIndex: 0,
        queryId: "sf-succ-handle-202",
      },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.activeQueryId).toBe("sf-succ-handle-202");

  useStudio.getState().applyEvents(
    tabId,
    [
      {
        type: "statementCompleted",
        runId: "succ-handle-run",
        index: 0,
        affectedRows: "1",
        elapsedMs: 5,
      },
      { type: "finished", runId: "succ-handle-run", status: "completed", elapsedMs: 10 },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("completed");
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.runId).toBeNull();

  // Case 3: Cancel with queryHandle -> activeQueryId set during run, strictly null on finish
  useStudio.getState().applyEvents(
    tabId,
    [
      { type: "started", runId: "canc-handle-run" },
      {
        type: "queryHandle",
        runId: "canc-handle-run",
        statementIndex: 0,
        queryId: "sf-canc-handle-303",
      },
      { type: "finished", runId: "canc-handle-run", status: "cancelled", elapsedMs: 15 },
    ],
    currentTab.revision,
  );
  currentTab = useStudio.getState().tabs.find((t) => t.id === tabId)!;
  expect(currentTab.status).toBe("cancelled");
  expect(currentTab.activeQueryId).toBeNull();
  expect(currentTab.runId).toBeNull();
});
