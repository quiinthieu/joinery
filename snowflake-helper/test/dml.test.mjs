import test from "node:test";
import assert from "node:assert/strict";
import { PassThrough, Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";

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

function setup({ synchronousComplete = false, onEvent, submitGuard } = {}) {
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

const dmlPayload = (kind, sql, binds = [], expected = pin, extra = {}) => ({
  kind,
  sql,
  binds,
  expected,
  ...extra,
});

async function waitFor(predicate, message = "expected helper event did not arrive") {
  for (let attempt = 0; attempt < 30; attempt++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail(message);
}

async function connected(options) {
  const state = setup(options);
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

function dmlCalls(state) {
  return state.calls.filter((call) => call && typeof call === "object" && "sqlText" in call);
}

function resultRow(values = ["9007199254740993", null], columns = ["VALUE", "OPTIONAL"]) {
  return {
    stream: Readable.from([values], { objectMode: true }),
    columns,
  };
}

test("insert, update and delete each dispatch one exact SQL and scalar bind list", async () => {
  for (const [kind, sql, binds] of [
    ["insert", 'INSERT INTO "T" VALUES (?, ?)', ["9007199254740993", null]],
    ["update", 'UPDATE "T" SET "N" = ? WHERE "ID" = ?', ["9007199254740993", "7"]],
    ["delete", 'DELETE FROM "T" WHERE "ID" = ?', [false]],
  ]) {
    const state = await pinned({ synchronousComplete: true });
    const row = resultRow();
    state.streams.push(row.stream);
    state.columnSets.push(row.columns);
    const before = dmlCalls(state).length;
    const id = state.send("dml", dmlPayload(kind, sql, binds));
    await waitFor(() => state.terminal(id));

    const [call] = dmlCalls(state).slice(before);
    assert.equal(dmlCalls(state).length - before, 1);
    assert.equal(call.sqlText, sql);
    assert.deepEqual(call.binds, binds);
    const terminal = state.terminal(id);
    assert.equal(terminal.ok, true);
    assert.deepEqual(terminal.result, {
      kind,
      rowsSeen: 1,
      rowsRetained: 0,
      truncated: false,
      columns: [
        { name: "VALUE", type: "TEXT" },
        { name: "OPTIONAL", type: "TEXT" },
      ],
      values: [["9007199254740993", null]],
      sdkRequestId: "11111111-1111-4111-8111-000000000002",
      queryId: "22222222-2222-4222-8222-000000000002",
    });
    const events = state.events.filter((event) => event.id === id);
    assert(events.findIndex((event) => event.type === "dmlAcknowledged") >= 0);
    assert(
      events.findIndex((event) => event.type === "dmlAcknowledged") <
        events.findIndex((event) => event.type === "terminal"),
    );
    assert.equal(
      events.some((event) => ["rows", "columns"].includes(event.type)),
      false,
    );
    state.helper.stop();
  }
});

test("DML admission requires exact shape, scalar binds, and a matching identity pin", async () => {
  const unpinned = await connected();
  const unpinnedId = unpinned.send("dml", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x"]));
  assert.equal(unpinned.terminal(unpinnedId).error.code, "invalidInput");
  assert.equal(dmlCalls(unpinned).length, 0);

  const cases = [
    ["invalid kind", dmlPayload("merge", "MERGE INTO T", [])],
    ["empty SQL", dmlPayload("insert", "", [])],
    ["SQL NUL", dmlPayload("update", "UPDATE T SET A = ?\0", ["x"])],
    ["oversized SQL", dmlPayload("delete", "x".repeat(262145), [])],
    ["ill-formed SQL", dmlPayload("insert", `INSERT ${String.fromCharCode(0xd800)}`, [])],
    ["numeric bind", dmlPayload("insert", "INSERT INTO T VALUES (?)", [9007199254740993n])],
    ["object bind", dmlPayload("update", "UPDATE T SET A = ?", [{}])],
    ["oversized bind", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x".repeat(262145)])],
    ["ill-formed bind", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["\ud800"])],
    ["too many binds", dmlPayload("delete", "DELETE FROM T WHERE A = ?", Array(1001).fill("x"))],
    [
      "wrong user pin",
      dmlPayload("insert", "INSERT INTO T VALUES (?)", [], { ...pin, user: "OTHER" }),
    ],
    ["wrong session pin", dmlPayload("delete", "DELETE FROM T", [], { ...pin, session: "other" })],
    ["extra expected field", dmlPayload("update", "UPDATE T SET A = 1", [], { ...pin, role: "R" })],
    [
      "extra classification",
      dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x"], pin, { classification: "write" }),
    ],
    ["extra credits", dmlPayload("delete", "DELETE FROM T", [], pin, { credits: 4 })],
    ["extra limits", dmlPayload("update", "UPDATE T SET A = ?", ["x"], pin, { maxRows: 100 })],
  ];

  for (const [label, payload] of cases) {
    const state = await pinned();
    const before = dmlCalls(state).length;
    const id = state.send("dml", payload);
    assert.equal(state.terminal(id)?.error?.code, "invalidInput", label);
    assert.equal(dmlCalls(state).length, before, label);
    assert.equal(state.retired(), 0, label);
    state.helper.stop();
  }
});

test("queued DML revalidates the identity pin before dispatch", async () => {
  const state = await pinned();
  const drain = new PassThrough({ objectMode: true });
  state.streams.push(drain, Readable.from([], { objectMode: true }));
  state.columnSets.push(["COUNT"]);
  const active = state.send("dml", dmlPayload("update", "UPDATE T SET A = ?", ["1"]));
  await waitFor(() =>
    state.events.some((event) => event.id === active && event.type === "dmlAcknowledged"),
  );
  const before = dmlCalls(state).length;
  const queued = state.send("dml", dmlPayload("delete", "DELETE FROM T WHERE A = ?", ["never"]));
  assert(state.events.some((event) => event.id === queued && event.type === "accepted"));
  state.helper.identityPin = { user: "OTHER", session: "changed-session" };
  drain.end();
  await waitFor(() => state.terminal(active) && state.terminal(queued));
  assert.equal(state.terminal(active).ok, true);
  assert.equal(state.terminal(queued).error.code, "invalidInput");
  assert.equal(dmlCalls(state).length, before);
});

test("queued DML cancellation finishes locally without dispatch", async () => {
  const state = await pinned();
  const drain = new PassThrough({ objectMode: true });
  state.streams.push(drain, Readable.from([], { objectMode: true }));
  state.columnSets.push(["COUNT"]);
  const active = state.send("dml", dmlPayload("update", "UPDATE T SET A = ?", ["1"]));
  await waitFor(() =>
    state.events.some((event) => event.id === active && event.type === "dmlAcknowledged"),
  );
  const before = dmlCalls(state).length;
  const queued = state.send("dml", dmlPayload("delete", "DELETE FROM T WHERE A = ?", ["never"]));
  const cancel = state.send("cancel", { target: queued });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(queued).error.code, "cancelled");
  assert.equal(dmlCalls(state).length, before);
  drain.end();
  await waitFor(() => state.terminal(active));
  state.helper.stop();
});

test("definitive database failure has no acknowledgement and does not retire", async () => {
  const state = await pinned();
  state.connection.nextError = { sqlState: "42000", message: "private SQL detail" };
  const id = state.send("dml", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x"]));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "database");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 0);
  assert.equal(JSON.stringify(state.events).includes("private SQL detail"), false);
  const next = state.send("dml", dmlPayload("delete", "DELETE FROM T WHERE ID = ?", ["7"]));
  await waitFor(() => state.terminal(next));
  assert.equal(state.terminal(next).ok, true);
});

test("lost pre-ack SDK response is outcome-unknown and retires without replay", async () => {
  const state = await pinned();
  state.connection.nextError = new Error("private transport error");
  const before = dmlCalls(state).length;
  const id = state.send("dml", dmlPayload("update", "UPDATE T SET A = ?", ["x"]));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
    false,
  );
  assert.equal(dmlCalls(state).length, before + 1);
  assert.equal(state.retired(), 1);
});

test("ACK is emitted before draining and privately retains bounded DML results", async () => {
  const registeredRequestIds = [];
  const submitGuard = {
    setAuthority: () => {},
    beginOperation: () => ({}),
    beginStatement: () => ({}),
    registerRequestId: (_ticket, id) => registeredRequestIds.push(id),
    endStatement: () => {},
    endOperation: () => {},
  };
  const state = await pinned({ synchronousComplete: true, submitGuard });
  state.streams.push(Readable.from([["affected", "9007199254740993"]], { objectMode: true }));
  state.columnSets.push([
    { name: "AFFECTED", type: "NUMBER(38,0)" },
    { name: "EXACT", type: "NUMBER(38,0)" },
  ]);
  const id = state.send("dml", dmlPayload("insert", "INSERT INTO T VALUES (?, ?)", ["1", "2"]));
  await waitFor(() => state.terminal(id));
  const events = state.events.filter((event) => event.id === id);
  const acknowledgement = events.find((event) => event.type === "dmlAcknowledged");
  assert.equal(acknowledgement.sdkRequestId, registeredRequestIds.at(-1));
  assert.deepEqual(Object.keys(acknowledgement).sort(), [
    "generation",
    "id",
    "kind",
    "queryId",
    "sdkRequestId",
    "type",
    "v",
  ]);
  assert(
    events.findIndex((event) => event.type === "dmlAcknowledged") <
      events.findIndex((event) => event.type === "terminal"),
  );
  assert.equal(
    events.some((event) => ["rows", "columns"].includes(event.type)),
    false,
  );
  assert.deepEqual(state.terminal(id).result.values, [["affected", "9007199254740993"]]);
  assert.deepEqual(state.terminal(id).result.columns, [
    { name: "AFFECTED", type: "NUMBER(38,0)" },
    { name: "EXACT", type: "NUMBER(38,0)" },
  ]);
});

test("malformed or oversized DML result after ACK retires with connection-closed", async () => {
  const cases = [
    {
      label: "more than two columns",
      columns: ["A", "B", "C"],
      rows: [["1", "2", "3"]],
    },
    {
      label: "more than one row",
      columns: ["A"],
      rows: [["1"], ["2"]],
    },
    {
      label: "row width mismatch",
      columns: ["A", "B"],
      rows: [["1"]],
    },
    {
      label: "non-string result value",
      columns: ["A"],
      rows: [[1]],
    },
    {
      label: "private byte budget exceeded",
      columns: ["A"],
      rows: [["x".repeat(65536)]],
    },
  ];
  for (const item of cases) {
    const state = await pinned();
    state.streams.push(Readable.from(item.rows, { objectMode: true }));
    state.columnSets.push(item.columns);
    const id = state.send("dml", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x"]));
    await waitFor(() => state.terminal(id));
    assert.equal(
      state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
      true,
      item.label,
    );
    assert.equal(state.terminal(id).error.code, "connectionClosed", item.label);
    assert.equal(state.retired(), 1, item.label);
  }
});

test("cancel before DML ACK preserves unknown outcome and never retries", async () => {
  const state = await pinned();
  state.connection.holdComplete = true;
  const id = state.send("dml", dmlPayload("delete", "DELETE FROM T", []));
  await waitFor(() => dmlCalls(state).length === 2);
  const cancel = state.send("cancel", { target: id });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
    false,
  );
  assert.equal(dmlCalls(state).length, 2);
  assert.equal(state.retired(), 1);
});

test("cancel after DML ACK preserves ACK but retires uncertain stream outcome", async () => {
  const state = await pinned();
  const stream = new PassThrough({ objectMode: true });
  state.streams.push(stream);
  state.columnSets.push(["A"]);
  const id = state.send("dml", dmlPayload("update", "UPDATE T SET A = ?", ["x"]));
  await waitFor(() =>
    state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
  );
  const cancel = state.send("cancel", { target: id });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "dmlAcknowledged").length,
    1,
  );
  assert.equal(state.calls.filter((call) => call.kind === "cancel").length, 1);
  assert.equal(state.retired(), 1);
});

test("drain failure after DML ACK preserves ACK and retires", async () => {
  const state = await pinned();
  const stream = new PassThrough({ objectMode: true });
  state.streams.push(stream);
  state.columnSets.push(["A"]);
  const id = state.send("dml", dmlPayload("insert", "INSERT INTO T VALUES (?)", ["x"]));
  await waitFor(() =>
    state.events.some((event) => event.id === id && event.type === "dmlAcknowledged"),
  );
  stream.write(["private"]);
  stream.destroy(new Error("private stream failure"));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "dmlAcknowledged").length,
    1,
  );
  assert.equal(JSON.stringify(state.events).includes("private stream failure"), false);
  assert.equal(state.retired(), 1);
});

test("reentrant protocol violation after DML ACK preserves acknowledgement and retires", async () => {
  const state = await pinned();
  const originalSend = state.helper.send.bind(state.helper);
  let violated = false;
  state.helper.send = (request, type, rest) => {
    const emitted = originalSend(request, type, rest);
    if (type === "dmlAcknowledged" && !violated) {
      violated = true;
      state.helper.submissionViolation();
    }
    return emitted;
  };
  state.streams.push(Readable.from([], { objectMode: true }));
  state.columnSets.push(["A"]);
  const id = state.send("dml", dmlPayload("delete", "DELETE FROM T WHERE A = ?", ["x"]));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  const events = state.events.filter((event) => event.id === id);
  assert.equal(events.filter((event) => event.type === "dmlAcknowledged").length, 1);
  assert.equal(events.filter((event) => event.type === "terminal").length, 1);
  assert(
    events.findIndex((event) => event.type === "dmlAcknowledged") <
      events.findIndex((event) => event.type === "terminal"),
  );
  assert.equal(
    events.some((event) => ["rows", "columns"].includes(event.type)),
    false,
  );
  assert.equal(state.retired(), 1);
});
