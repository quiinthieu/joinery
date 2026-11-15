import test from "node:test";
import assert from "node:assert/strict";
import { PassThrough, Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";
import { deriveSetupLedger } from "../src/disposable-setup-ledger.mjs";

const generation = "b512e129-c689-48bd-b5c2-848438b1231a";
const pin = { user: "FIXTURE", session: "opaque-session" };
const profile = {
  account: "fixture-account",
  host: "fixture-account.snowflakecomputing.com",
  username: "FIXTURE",
  role: "READER",
  warehouse: "TINY",
  database: "TEST",
  schema: "PUBLIC",
};
const tick = () => new Promise((resolve) => setImmediate(resolve));

function fixture({ synchronousComplete = false, onEvent, submitGuard } = {}) {
  const events = [];
  const calls = [];
  const streams = [];
  const columnSets = [];
  let retired = 0;
  let callNumber = 0;
  let options;
  const connection = {
    connectAsync(callback) {
      setImmediate(() => callback(null, connection));
      return Promise.resolve(connection);
    },
    destroy(callback) {
      calls.push({ kind: "destroy" });
      callback?.();
    },
    execute(opts) {
      calls.push(opts);
      callNumber++;
      const stream = streams.shift() ?? Readable.from([], { objectMode: true });
      const requestId = `11111111-1111-4111-8111-${String(callNumber).padStart(12, "0")}`;
      let acknowledged = false;
      const rawColumns = columnSets.shift() ?? [];
      const columns = rawColumns.map((column) =>
        typeof column === "string" ? { name: column, type: "TEXT" } : column,
      );
      const statement = {
        getRequestId: () => requestId,
        getQueryId: () =>
          acknowledged
            ? `22222222-2222-4222-8222-${String(callNumber).padStart(12, "0")}`
            : undefined,
        getColumns: () =>
          columns.map((column) => ({
            getName: () => column.name,
            getType: () => column.type,
          })),
        streamRows: () => stream,
        cancel(callback) {
          calls.push({ kind: "cancel" });
          callback?.();
        },
      };
      const error = connection.nextError;
      connection.nextError = null;
      const complete = () => {
        acknowledged = !error;
        opts.complete(error ?? null, statement);
      };
      if (synchronousComplete) complete();
      else if (connection.holdComplete) connection.pendingComplete = complete;
      else setImmediate(complete);
      return statement;
    },
  };
  const sdk = {
    createConnection(value) {
      options = value;
      return connection;
    },
  };
  const helper = new Helper(
    sdk,
    (event) => {
      events.push(event);
      onEvent?.(event, helper);
      return Promise.resolve();
    },
    () => retired++,
    { clientConfigFile: "/owned/nonexistent", closeTimeoutMs: 10, submitGuard },
  );
  let sequence = 0;
  const send = (op, payload = {}, id = String(++sequence)) => {
    helper.receive({ v: 1, generation, id, op, payload });
    return id;
  };
  const terminal = (id) => events.find((event) => event.id === id && event.type === "terminal");
  return {
    helper,
    events,
    calls,
    streams,
    columnSets,
    connection,
    send,
    terminal,
    options: () => options,
    retired: () => retired,
  };
}

async function waitFor(predicate, message = "expected helper event did not arrive") {
  for (let attempt = 0; attempt < 30; attempt++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail(message);
}

async function connected(options) {
  const state = fixture(options);
  const connect = state.send("connect", profile);
  await waitFor(() => state.terminal(connect));
  assert.equal(state.terminal(connect).ok, true);
  return state;
}

async function pinned(options) {
  const state = await connected(options);
  state.streams.push(
    Readable.from([["FIXTURE", "READER", "TEST", "PUBLIC", "TINY", "opaque-session"]], {
      objectMode: true,
    }),
  );
  state.columnSets.push(["user", "role", "database", "schema", "warehouse", "session"]);
  const context = state.send("context");
  await waitFor(() => state.terminal(context));
  assert.equal(state.terminal(context).ok, true);
  return state;
}

const runId = "0123456789ABCDEF0123456789ABCDEF";
const ledger = deriveSetupLedger(runId);
const payload = (step, changes = {}) => ({ runId, step, expected: pin, ...changes });
const sqlCalls = (s) =>
  s.calls.filter((call) => call && typeof call === "object" && "sqlText" in call);

async function ready(options) {
  const state = await pinned(options);
  state.streams.push(Readable.from([["fixture-version"]], { objectMode: true }));
  state.columnSets.push(["VERSION"]);
  const id = state.send("query", {
    sql: "SELECT CURRENT_VERSION()",
    binds: [],
    classification: "read",
    credits: 1,
    maxRows: 1,
    maxBytes: 65536,
  });
  await waitFor(() => state.terminal(id) && !state.helper.active);
  assert.equal(state.terminal(id).ok, true);
  return state;
}

async function step(state, index, { rows = [["fixture"]], columns = ["STATUS"] } = {}) {
  state.streams.push(Readable.from(rows, { objectMode: true }));
  state.columnSets.push(columns);
  const id = state.send("setup", payload(index));
  await waitFor(() => state.terminal(id) && !state.helper.active);
  return id;
}

async function beforeCreate(options) {
  const state = await ready(options);
  for (let index = 3; index < 8; index++) {
    const id = await step(state, index);
    assert.equal(state.terminal(id).ok, true);
  }
  return state;
}

test("fixed setup executes the exact 28-statement ledger, with three early CREATE ACKs and private results", async () => {
  const state = await ready({ synchronousComplete: true });
  for (let index = 3; index <= 28; index++) {
    const entry = ledger[index - 1];
    const id = await step(state, index);
    const terminal = state.terminal(id);
    assert.equal(terminal.ok, true, entry.key);
    assert.deepEqual(
      Object.fromEntries(
        ["runId", "step", "key", "phase", "mutation"].map((k) => [k, terminal.result[k]]),
      ),
      { runId, step: index, key: entry.key, phase: entry.phase, mutation: entry.mutation },
    );
    assert.equal(terminal.result.rowsSeen, 1);
    assert.equal(terminal.result.rowsRetained, 0);
    assert.equal(terminal.result.truncated, false);
    assert.deepEqual(terminal.result.values, [["fixture"]]);
    const events = state.events.filter((event) => event.id === id);
    const ack = events.find((event) => event.type === "setupAcknowledged");
    assert.equal(Boolean(ack), entry.mutation, entry.key);
    if (ack) {
      assert.equal(ack.runId, runId);
      assert.equal(ack.step, index);
      assert.equal(ack.key, entry.key);
      assert(ack.sdkRequestId);
      assert(events.indexOf(ack) < events.indexOf(terminal));
    }
    assert.equal(
      events.some((event) => ["rows", "columns"].includes(event.type)),
      false,
    );
  }
  assert.deepEqual(
    sqlCalls(state).map((call) => call.sqlText),
    ledger.map((entry) => entry.sql),
  );
  assert(sqlCalls(state).every((call) => JSON.stringify(call.binds) === "[]"));
  assert.equal(state.events.filter((event) => event.type === "setupAcknowledged").length, 3);
  state.helper.stop();
});

test("setup requires the exact successful context/version bootstrap without adding SQL", async () => {
  for (const phase of ["connected", "pinned", "wrong-version"]) {
    const state = phase === "connected" ? await connected() : await pinned();
    if (phase === "wrong-version") {
      const query = state.send("query", {
        sql: "SELECT CURRENT_VERSION();",
        binds: [],
        classification: "read",
        credits: 1,
        maxRows: 1,
        maxBytes: 65536,
      });
      await waitFor(() => state.terminal(query) && !state.helper.active);
    }
    const before = sqlCalls(state).length;
    const id = state.send("setup", payload(3));
    assert.equal(state.terminal(id).error.code, "invalidInput", phase);
    assert.equal(sqlCalls(state).length, before);
    assert.equal(state.retired(), 1);
  }
});

test("setup attempted before connection completes permanently retires without SQL", async () => {
  const state = fixture();
  state.send("connect", profile);
  const id = state.send("setup", payload(3));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  await tick();
  assert.equal(state.retired(), 1);
  assert.equal(sqlCalls(state).length, 0);
  state.send("context");
  assert.equal(sqlCalls(state).length, 0);
});

test("setup rejects wrong shape, run, identity, ordinal, or caller-supplied SQL and permanently stops", async () => {
  for (const bad of [
    payload(2),
    payload(4),
    payload(29),
    payload(3, { step: "3" }),
    payload(3, { runId: "0".repeat(32) }),
    payload(3, { runId: runId.toLowerCase() }),
    payload(3, { expected: { ...pin, session: "OTHER" } }),
    payload(3, { expected: { ...pin, user: "OTHER" } }),
    payload(3, { expected: { ...pin, role: "R" } }),
    payload(3, { sql: "DROP DATABASE X" }),
    payload(3, { binds: [] }),
    payload(3, { credits: 1 }),
    payload(3, { maxRows: 1 }),
  ]) {
    const state = await ready();
    const before = sqlCalls(state).length;
    const id = state.send("setup", bad);
    assert.equal(state.terminal(id)?.error.code, "invalidInput", JSON.stringify(bad));
    state.send("setup", payload(3));
    assert.equal(sqlCalls(state).length, before);
    assert.equal(state.retired(), 1);
  }
});

test("setup cannot repeat, change RUN, pipeline, or interleave another SQL route", async () => {
  for (const bad of [
    "repeat",
    "changed-run",
    "pipeline",
    "query",
    "context",
    "transaction",
    "dml",
    "switchSchema",
    "credit",
  ]) {
    const state = await ready();
    if (bad === "pipeline") {
      state.connection.holdComplete = true;
      state.send("setup", payload(3));
    } else assert.equal(state.terminal(await step(state, 3)).ok, true);
    const before = sqlCalls(state).length;
    const id =
      bad === "repeat"
        ? state.send("setup", payload(3))
        : bad === "changed-run"
          ? state.send("setup", payload(4, { runId: "F".repeat(32) }))
          : bad === "pipeline"
            ? state.send("setup", payload(4))
            : state.send(bad, {});
    assert.equal(state.terminal(id)?.error.code, "invalidInput", bad);
    assert.equal(sqlCalls(state).length, before, bad);
    assert.equal(state.retired(), 1, bad);
  }
});

test("every rejected or uncertain setup step retires permanently without a CREATE ACK", async () => {
  for (const failure of [{ sqlState: "42501" }, new Error("private transport failure")]) {
    const state = await beforeCreate();
    state.connection.nextError = failure;
    const id = state.send("setup", payload(8));
    await waitFor(() => state.terminal(id));
    assert.equal(state.terminal(id).error.code, failure.sqlState ? "database" : "outcomeUnknown");
    assert.equal(
      state.events.some((event) => event.id === id && event.type === "setupAcknowledged"),
      false,
    );
    const before = sqlCalls(state).length;
    state.send("setup", payload(8));
    assert.equal(sqlCalls(state).length, before);
    assert.equal(state.retired(), 1);
  }
});

test("setup private result budgets honor each entry and share the 64KiB cap", async () => {
  for (const response of [
    { rows: [["x"], ["y"]], columns: ["A"] },
    { rows: [["x", "y"]], columns: ["A"] },
    { rows: [[1]], columns: ["A"] },
    { rows: [["x".repeat(65530)]], columns: ["A"] },
    { rows: [Array(9).fill("x")], columns: Array(9).fill("C") },
  ]) {
    const state = await ready();
    const id = await step(state, 3, response);
    assert.equal(state.terminal(id).ok, false);
    assert.equal(state.retired(), 1);
    assert.equal(
      state.events.some((event) => event.id === id && ["columns", "rows"].includes(event.type)),
      false,
    );
  }
  const state = await ready();
  assert.equal(state.terminal(await step(state, 3)).ok, true);
  const id = await step(state, 4, {
    rows: [Array(32).fill("x")],
    columns: Array(32).fill("PARAMETER"),
  });
  assert.equal(state.terminal(id).ok, true);
  state.helper.stop();
});

test("known CREATE acknowledgement survives drain failure, cancellation, guard violation and local EOF", async () => {
  for (const fault of ["drain", "cancel", "guard", "eof"]) {
    const state = await beforeCreate();
    const stream = new PassThrough({ objectMode: true });
    state.streams.push(stream);
    state.columnSets.push(["STATUS"]);
    const id = state.send("setup", payload(8));
    await waitFor(() =>
      state.events.some((event) => event.id === id && event.type === "setupAcknowledged"),
    );
    const before = sqlCalls(state).length;
    if (fault === "drain") stream.destroy(new Error("private drain failure"));
    if (fault === "cancel") state.send("cancel", { target: id });
    if (fault === "guard") state.helper.submissionViolation();
    if (fault === "eof") state.helper.stop();
    if (fault !== "eof") await waitFor(() => state.terminal(id));
    assert.equal(
      state.events.filter((event) => event.id === id && event.type === "setupAcknowledged").length,
      1,
    );
    if (fault !== "eof") assert.equal(state.terminal(id).error.code, "connectionClosed");
    assert.equal(sqlCalls(state).length, before);
    assert.equal(state.retired(), 1);
    assert.equal(JSON.stringify(state.events).includes("private drain failure"), false);
    stream.end();
  }
});

test("cancel before CREATE acknowledgement is unknown and consumes the step", async () => {
  const state = await beforeCreate();
  state.connection.holdComplete = true;
  const id = state.send("setup", payload(8));
  const cancel = state.send("cancel", { target: id });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "setupAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 1);
});

test("explicit close preserves unknown before CREATE ACK and known ACK during drain", async () => {
  for (const acknowledged of [false, true]) {
    const state = await beforeCreate();
    const stream = new PassThrough({ objectMode: true });
    state.connection.holdComplete = !acknowledged;
    state.streams.push(stream);
    state.columnSets.push(["STATUS"]);
    const id = state.send("setup", payload(8));
    if (acknowledged)
      await waitFor(() =>
        state.events.some((event) => event.id === id && event.type === "setupAcknowledged"),
      );
    const before = sqlCalls(state).length;
    state.send("close", {});
    assert.equal(
      state.terminal(id).error.code,
      acknowledged ? "connectionClosed" : "outcomeUnknown",
    );
    assert.equal(
      state.events.filter((event) => event.id === id && event.type === "setupAcknowledged").length,
      acknowledged ? 1 : 0,
    );
    assert.equal(state.retired(), 1);
    state.send("setup", payload(8));
    assert.equal(sqlCalls(state).length, before);
    state.connection.pendingComplete?.();
    await tick();
    assert.equal(
      state.terminal(id).error.code,
      acknowledged ? "connectionClosed" : "outcomeUnknown",
    );
    assert.equal(
      state.events.filter((event) => event.id === id && event.type === "setupAcknowledged").length,
      acknowledged ? 1 : 0,
    );
    stream.end();
  }
});

test("explicit close cannot replace a definitive setup rejection with unknown", async () => {
  const state = await beforeCreate();
  state.connection.nextError = { sqlState: "42501" };
  const record = state.helper.record.bind(state.helper);
  state.helper.record = (active, statement) => {
    record(active, statement);
    if (active.knownFailure) state.send("close", {});
  };
  const id = state.send("setup", payload(8));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "database");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "setupAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 1);
});
