import test from "node:test";
import assert from "node:assert/strict";
import { Readable, PassThrough } from "node:stream";
import { Helper } from "../src/helper.mjs";

const generation = "11111111-1111-4111-8111-111111111111";
const requestId = "22222222-2222-4222-8222-222222222222";
const queryId = "33333333-3333-4333-8333-333333333333";
const expected = { user: "FIXTURE", session: "opaque-session" };
const tick = () => new Promise((resolve) => setImmediate(resolve));
async function until(predicate) {
  for (let i = 0; i < 100; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("reviewed SQL fixture did not settle");
}
function fixture({ onEvent, synchronous = false, veto = () => false } = {}) {
  const events = [],
    calls = [];
  let retired = 0,
    acknowledged = false;
  const connection = {
    stream: Readable.from([["original"]], { objectMode: true }),
    context: {
      getCurrentDatabase: () => "NEW_DB",
      getCurrentSchema: () => "NEW_SCHEMA",
      getCurrentRole: () => "NEW_ROLE",
      getCurrentWarehouse: () => "NEW_WH",
    },
    destroy(callback) {
      callback?.();
    },
    execute(options) {
      calls.push(options);
      const statement = {
        getRequestId: () => requestId,
        getQueryId: () => (acknowledged ? queryId : undefined),
        getSessionState: () => connection.context,
        getColumns: () => {
          if (connection.badColumns) throw new Error("private SDK metadata");
          return [{ getName: () => "VALUE", getType: () => "TEXT" }];
        },
        streamRows: () => connection.stream,
        cancel(callback) {
          callback?.();
        },
      };
      connection.complete = () => {
        acknowledged = !connection.failure;
        options.complete(connection.failure ?? null, connection.callbackStatement ?? statement);
      };
      if (synchronous) connection.complete();
      else if (!connection.hold) setImmediate(connection.complete);
      return statement;
    },
  };
  let helper;
  helper = new Helper(
    {},
    (event) => {
      events.push(event);
      onEvent?.(event, helper);
      return Promise.resolve();
    },
    () => retired++,
    { isFileTransfer: veto, closeTimeoutMs: 5 },
  );
  helper.connection = connection;
  helper.state = "connected";
  helper.generation = generation;
  helper.identityPin = { ...expected };
  const send = (op, payload, id = "1") => helper.receive({ v: 1, generation, id, op, payload });
  const terminal = (id = "1") => events.find((e) => e.id === id && e.type === "terminal");
  const payload = (extra = {}) => ({
    sql: "SELECT ?",
    binds: ["9007199254740993", true, null],
    expected: { ...expected },
    credits: 1,
    maxRows: 10,
    maxBytes: 4096,
    ...extra,
  });
  return { helper, connection, events, calls, send, terminal, payload, retired: () => retired };
}
const ack = (s) => s.events.find((e) => e.type === "statementAcknowledged");

test("reviewed SQL preserves exact SQL/scalars and forces one statement, with passive context before drain", async () => {
  for (const sql of [
    "SELECT ?",
    "SHOW TABLES",
    'USE SCHEMA "quoted schema"',
    "CREATE TEMP TABLE T(A NUMBER)",
    "BEGIN",
    "CALL X(?)",
  ]) {
    const s = fixture();
    s.send("reviewedSql", s.payload({ sql }));
    await until(() => s.terminal());
    assert.equal(s.terminal().ok, true);
    assert.equal(s.calls.length, 1);
    assert.equal(s.calls[0].sqlText, sql);
    assert.deepEqual(s.calls[0].binds, ["9007199254740993", true, null]);
    assert.deepEqual(s.calls[0].parameters, { MULTI_STATEMENT_COUNT: 1 });
    assert.deepEqual(ack(s).context, {
      database: "NEW_DB",
      schema: "NEW_SCHEMA",
      role: "NEW_ROLE",
      warehouse: "NEW_WH",
    });
    const types = s.events.map((e) => e.type);
    assert(types.indexOf("statement") < types.indexOf("statementAcknowledged"));
    assert(types.indexOf("statementAcknowledged") < types.indexOf("columns"));
    assert.equal(ack(s).sdkRequestId, requestId);
    assert.equal(ack(s).queryId, undefined);
    assert(s.events.some((e) => e.type === "statement" && e.queryId === queryId));
    assert.equal(s.terminal().result.queryId, queryId);
    assert.equal(s.terminal().result.rowsSeen, 1);
    assert.equal(s.retired(), 0);
    assert.equal(s.helper.setupBootstrap, -1);
  }
});

test("reviewed SQL validates closed bounds, scalar binds, session and required file veto before SDK admission", async () => {
  const bad = [
    { binds: [1] },
    { binds: [[]] },
    { binds: [{}] },
    { binds: Array(1001).fill(null) },
    { binds: ["\ud800"] },
    { sql: "" },
    { sql: "\ud800" },
    { sql: "SELECT\0" },
    { sql: "x".repeat(262145) },
    { parameters: {} },
    { credits: 0 },
    { credits: 33 },
    { maxRows: -1 },
    { maxRows: 1000001 },
    { maxBytes: 1099511627777 },
    { expected: { ...expected, role: "EXTRA" } },
    { expected: { ...expected, session: "another" } },
  ];
  for (const extra of bad) {
    const s = fixture();
    s.send("reviewedSql", s.payload(extra));
    await until(() => s.terminal());
    assert.equal(s.terminal().ok, false, JSON.stringify(extra));
    assert.equal(s.calls.length, 0);
  }
  for (const veto of [
    undefined,
    () => {
      throw new Error("private veto failure");
    },
  ]) {
    const s = fixture();
    s.helper.isFileTransfer = veto;
    s.send("reviewedSql", s.payload());
    await until(() => s.terminal());
    assert.equal(s.terminal().error.code, "invalidInput");
    assert.equal(s.calls.length, 0);
  }
  const s = fixture({ veto: () => true });
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.equal(s.terminal().error.code, "unsupported");
  assert.equal(s.calls.length, 0);
});

test("reviewed SQL context fields degrade independently and never claim transaction state", async () => {
  const s = fixture();
  s.connection.context = {
    getCurrentDatabase: () => "observed",
    getCurrentSchema: () => {
      throw new Error("private response");
    },
    getCurrentRole: () => "x".repeat(256),
    getCurrentWarehouse: () => "bad\0value",
  };
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.deepEqual(ack(s).context, {
    database: "observed",
    schema: null,
    role: null,
    warehouse: null,
  });
  assert.deepEqual(Object.keys(ack(s).context).sort(), ["database", "role", "schema", "warehouse"]);
});

test("reviewed SQL synchronous completion registers identity before original ACK", async () => {
  const s = fixture({ synchronous: true });
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.equal(s.terminal().ok, true);
  assert.equal(s.events.filter((e) => e.type === "statementAcknowledged").length, 1);
  assert(s.events.findIndex((e) => e.type === "statement") < s.events.indexOf(ack(s)));
});

test("reviewed SQL keeps original ACK through result limits, discard and drain failure", async () => {
  for (const mode of ["limit", "discard", "failure", "metadata"]) {
    const s = fixture({
      onEvent(event, helper) {
        if (mode === "discard" && event.type === "statementAcknowledged")
          helper.receive({
            v: 1,
            generation,
            id: "2",
            op: "credit",
            payload: { target: "1", batches: 0, discard: true },
          });
      },
    });
    if (mode === "failure") {
      const stream = new PassThrough({ objectMode: true });
      s.connection.stream = stream;
      s.send("reviewedSql", s.payload());
      await until(() => ack(s));
      stream.destroy(new Error("private chunk failure"));
    } else {
      if (mode === "metadata") s.connection.badColumns = true;
      s.send("reviewedSql", s.payload(mode === "limit" ? { maxRows: 0 } : {}));
    }
    await until(() => s.terminal());
    assert(ack(s));
    assert.equal(s.events.filter((e) => e.type === "statementAcknowledged").length, 1);
    assert.equal(s.calls.length, 1);
    if (["failure", "metadata"].includes(mode)) {
      assert.equal(s.terminal().error.code, "connectionClosed");
      assert.equal(s.retired(), 1);
    } else {
      assert.equal(s.terminal().ok, true);
      assert.equal(s.terminal().result.rowsSeen, 1);
      assert.equal(s.terminal().result.rowsRetained, 0);
      assert.equal(s.terminal().result.truncated, true);
      assert.equal(s.retired(), 0);
    }
  }
});

test("reviewed SQL cancellation retains original ACK and retires without replay", async () => {
  for (const afterAck of [false, true]) {
    const s = fixture();
    if (afterAck) s.connection.stream = new PassThrough({ objectMode: true });
    else s.connection.hold = true;
    s.send("reviewedSql", s.payload());
    await until(() => (afterAck ? ack(s) : s.calls.length));
    s.send("cancel", { target: "1" }, "2");
    await until(() => s.terminal());
    assert.equal(s.terminal().error.code, afterAck ? "connectionClosed" : "outcomeUnknown");
    assert.equal(Boolean(ack(s)), afterAck);
    assert.equal(s.retired(), 1);
    s.connection.complete();
    await tick();
    assert.equal(s.events.filter((e) => e.id === "1" && e.type === "terminal").length, 1);
    assert.equal(s.calls.length, 1);
    s.connection.stream.destroy();
  }
});

test("reviewed SQL publishes sticky success if passive getters reenter close", async () => {
  const s = fixture();
  s.connection.context.getCurrentDatabase = () => {
    s.send("close", {}, "2");
    return "late value";
  };
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.equal(s.terminal().error.code, "connectionClosed");
  assert(ack(s));
  assert(s.events.indexOf(ack(s)) < s.events.indexOf(s.terminal()));
  assert.deepEqual(ack(s).context, { database: null, schema: null, role: null, warehouse: null });
  assert.equal(s.calls.length, 1);
});

test("reviewed SQL rejects product mixing once disposable setup owns the helper", async () => {
  const s = fixture();
  s.helper.setupRun = "0123456789ABCDEF0123456789ABCDEF";
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.equal(s.terminal().ok, false);
  assert.equal(s.calls.length, 0);
});

test("reviewed SQL distinguishes definitive rejection from unknown completion without replay", async () => {
  for (const sqlState of ["42501", "08006", undefined]) {
    const s = fixture();
    s.connection.failure = { sqlState };
    s.send("reviewedSql", s.payload());
    await until(() => s.terminal());
    assert.equal(s.terminal().error.code, sqlState === "42501" ? "database" : "outcomeUnknown");
    assert.equal(Boolean(ack(s)), false);
    assert.equal(s.retired(), sqlState === "42501" ? 0 : 1);
    assert.equal(s.calls.length, 1);
  }
});

test("reviewed SQL never attributes an unrelated callback statement to the registered request", async () => {
  const s = fixture();
  s.connection.callbackStatement = {};
  s.send("reviewedSql", s.payload());
  await until(() => s.terminal());
  assert.equal(s.terminal().error.code, "outcomeUnknown");
  assert.equal(Boolean(ack(s)), false);
  assert.equal(s.retired(), 1);
});

test("reviewed SQL revalidates queued pins and permits cancellation before SDK dispatch", async () => {
  for (const cancel of [false, true]) {
    const s = fixture();
    s.connection.hold = true;
    s.send("reviewedSql", s.payload());
    await until(() => s.calls.length === 1);
    s.send("reviewedSql", s.payload(), "2");
    if (cancel) s.send("cancel", { target: "2" }, "3");
    else s.helper.identityPin = { ...expected, session: "changed-session" };
    s.connection.complete();
    await until(() => s.terminal("2"));
    assert.equal(s.terminal("2").error.code, cancel ? "cancelled" : "invalidInput");
    assert.equal(s.calls.length, 1);
    assert.equal(s.retired(), 0);
  }
});

test("reviewed SQL waits for stream credits and discards a suffix without cancelling the original", async () => {
  const s = fixture();
  s.connection.stream = Readable.from(
    Array.from({ length: 300 }, () => ["retained"]),
    { objectMode: true },
  );
  s.send("reviewedSql", s.payload({ maxRows: 1000, maxBytes: 65536 }));
  await until(() => s.events.some((e) => e.type === "rows"));
  assert.equal(s.terminal(), undefined);
  s.send("credit", { target: "1", batches: 1, discard: false }, "2");
  await until(() => s.events.filter((e) => e.type === "rows").length === 2);
  assert.equal(s.terminal(), undefined);
  s.send("credit", { target: "1", batches: 0, discard: true }, "3");
  await until(() => s.terminal());
  assert.equal(s.terminal().ok, true);
  assert.equal(s.terminal().result.rowsSeen, 300);
  assert.equal(s.terminal().result.rowsRetained, 256);
  assert.equal(s.terminal().result.truncated, true);
  assert.equal(s.retired(), 0);
  assert.equal(s.calls.length, 1);
});
