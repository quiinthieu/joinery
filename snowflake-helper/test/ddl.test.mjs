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

const ddlPayload = (kind, sql, expected = pin, extra = {}) => ({
  kind,
  sql,
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

function ddlCalls(state) {
  return state.calls.filter((call) => call && typeof call === "object" && "sqlText" in call);
}

function resultRow(values = ["Table T successfully created."], columns = ["status"]) {
  return {
    stream: Readable.from([values], { objectMode: true }),
    columns,
  };
}

test("createTable, addColumn and dropTable each dispatch one exact SQL statement without binds", async () => {
  for (const [kind, sql] of [
    ["createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" NUMBER(38,0) NOT NULL)'],
    ["addColumn", 'ALTER TABLE "TEST"."PUBLIC"."T" ADD COLUMN "NOTE" VARCHAR(255)'],
    ["dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'],
  ]) {
    const state = await pinned({ synchronousComplete: true });
    const row = resultRow();
    state.streams.push(row.stream);
    state.columnSets.push(row.columns);
    const before = ddlCalls(state).length;
    const id = state.send("ddl", ddlPayload(kind, sql));
    await waitFor(() => state.terminal(id));

    const [call] = ddlCalls(state).slice(before);
    assert.equal(ddlCalls(state).length - before, 1);
    assert.equal(call.sqlText, sql);
    assert.deepEqual(call.binds, []);
    const terminal = state.terminal(id);
    assert.equal(terminal.ok, true);
    assert.deepEqual(terminal.result, {
      kind,
      rowsSeen: 1,
      rowsRetained: 0,
      truncated: false,
      columns: [{ name: "status", type: "TEXT" }],
      values: [["Table T successfully created."]],
      sdkRequestId: "11111111-1111-4111-8111-000000000002",
      queryId: "22222222-2222-4222-8222-000000000002",
    });
    const events = state.events.filter((event) => event.id === id);
    assert(events.findIndex((event) => event.type === "ddlAcknowledged") >= 0);
    assert(
      events.findIndex((event) => event.type === "ddlAcknowledged") <
        events.findIndex((event) => event.type === "terminal"),
    );
    assert.equal(
      events.some((event) => ["rows", "columns"].includes(event.type)),
      false,
    );
    state.helper.stop();
  }
});

test("DDL admission requires exact shape, no binds, and a matching identity pin", async () => {
  const unpinned = await connected();
  const unpinnedId = unpinned.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  assert.equal(unpinned.terminal(unpinnedId).error.code, "invalidInput");
  assert.equal(ddlCalls(unpinned).length, 0);

  const cases = [
    ["invalid kind", ddlPayload("alterColumn", 'ALTER TABLE "T" DROP COLUMN "C"')],
    ["empty SQL", ddlPayload("createTable", "")],
    ["SQL NUL", ddlPayload("addColumn", 'ALTER TABLE "T" ADD COLUMN "C" BOOLEAN\0')],
    ["oversized SQL", ddlPayload("dropTable", "x".repeat(262145))],
    ["ill-formed SQL", ddlPayload("createTable", `CREATE TABLE ${String.fromCharCode(0xd800)}`)],
    [
      "binds field present",
      ddlPayload("createTable", 'CREATE TABLE "T" ("C" BOOLEAN)', pin, { binds: [] }),
    ],
    [
      "wrong user pin",
      ddlPayload("createTable", 'CREATE TABLE "T" ("C" BOOLEAN)', { ...pin, user: "OTHER" }),
    ],
    [
      "wrong session pin",
      ddlPayload("dropTable", 'DROP TABLE "T" RESTRICT', { ...pin, session: "other" }),
    ],
    [
      "extra expected field",
      ddlPayload("addColumn", 'ALTER TABLE "T" ADD COLUMN "C" BOOLEAN', { ...pin, role: "R" }),
    ],
    [
      "extra classification",
      ddlPayload("createTable", 'CREATE TABLE "T" ("C" BOOLEAN)', pin, { classification: "ddl" }),
    ],
    ["extra credits", ddlPayload("dropTable", 'DROP TABLE "T" RESTRICT', pin, { credits: 1 })],
    [
      "extra limits",
      ddlPayload("addColumn", 'ALTER TABLE "T" ADD COLUMN "C" BOOLEAN', pin, { maxRows: 100 }),
    ],
  ];

  for (const [label, payload] of cases) {
    const state = await pinned();
    const before = ddlCalls(state).length;
    const id = state.send("ddl", payload);
    assert.equal(state.terminal(id)?.error?.code, "invalidInput", label);
    assert.equal(ddlCalls(state).length, before, label);
    assert.equal(state.retired(), 0, label);
    state.helper.stop();
  }
});

test("queued DDL revalidates the identity pin before dispatch", async () => {
  const state = await pinned();
  const drain = new PassThrough({ objectMode: true });
  state.streams.push(drain, Readable.from([], { objectMode: true }));
  state.columnSets.push(["status"]);
  const active = state.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  await waitFor(() =>
    state.events.some((event) => event.id === active && event.type === "ddlAcknowledged"),
  );
  const before = ddlCalls(state).length;
  const queued = state.send(
    "ddl",
    ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'),
  );
  assert(state.events.some((event) => event.id === queued && event.type === "accepted"));
  state.helper.identityPin = { user: "OTHER", session: "changed-session" };
  drain.end();
  await waitFor(() => state.terminal(active) && state.terminal(queued));
  assert.equal(state.terminal(active).ok, true);
  assert.equal(state.terminal(queued).error.code, "invalidInput");
  assert.equal(ddlCalls(state).length, before);
});

test("queued DDL cancellation finishes locally without dispatch", async () => {
  const state = await pinned();
  const drain = new PassThrough({ objectMode: true });
  state.streams.push(drain, Readable.from([], { objectMode: true }));
  state.columnSets.push(["status"]);
  const active = state.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  await waitFor(() =>
    state.events.some((event) => event.id === active && event.type === "ddlAcknowledged"),
  );
  const before = ddlCalls(state).length;
  const queued = state.send(
    "ddl",
    ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'),
  );
  const cancel = state.send("cancel", { target: queued });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(queued).error.code, "cancelled");
  assert.equal(ddlCalls(state).length, before);
  drain.end();
  await waitFor(() => state.terminal(active));
  state.helper.stop();
});

test("definitive database failure has no acknowledgement and does not retire", async () => {
  const state = await pinned();
  state.connection.nextError = { sqlState: "42000", message: "private SQL detail" };
  const id = state.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "database");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 0);
  assert.equal(JSON.stringify(state.events).includes("private SQL detail"), false);
  const next = state.send(
    "ddl",
    ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'),
  );
  await waitFor(() => state.terminal(next));
  assert.equal(state.terminal(next).ok, true);
});

test("lost pre-ack SDK response is outcome-unknown and retires without replay", async () => {
  const state = await pinned();
  state.connection.nextError = new Error("private transport error");
  const before = ddlCalls(state).length;
  const id = state.send(
    "ddl",
    ddlPayload("addColumn", 'ALTER TABLE "TEST"."PUBLIC"."T" ADD COLUMN "C" BOOLEAN'),
  );
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(ddlCalls(state).length, before + 1);
  assert.equal(state.retired(), 1);
});

test("ACK is emitted before draining and privately retains bounded DDL results", async () => {
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
  state.streams.push(Readable.from([["Table T successfully created."]], { objectMode: true }));
  state.columnSets.push([{ name: "status", type: "TEXT" }]);
  const id = state.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  await waitFor(() => state.terminal(id));
  const events = state.events.filter((event) => event.id === id);
  const acknowledgement = events.find((event) => event.type === "ddlAcknowledged");
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
    events.findIndex((event) => event.type === "ddlAcknowledged") <
      events.findIndex((event) => event.type === "terminal"),
  );
  assert.equal(
    events.some((event) => ["rows", "columns"].includes(event.type)),
    false,
  );
  assert.deepEqual(state.terminal(id).result.values, [["Table T successfully created."]]);
  assert.deepEqual(state.terminal(id).result.columns, [{ name: "status", type: "TEXT" }]);
});

test("malformed or oversized DDL result after ACK retires with connection-closed", async () => {
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
    const id = state.send(
      "ddl",
      ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
    );
    await waitFor(() => state.terminal(id));
    assert.equal(
      state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
      true,
      item.label,
    );
    assert.equal(state.terminal(id).error.code, "connectionClosed", item.label);
    assert.equal(state.retired(), 1, item.label);
  }
});

test("cancel before DDL ACK preserves unknown outcome and never retries", async () => {
  const state = await pinned();
  state.connection.holdComplete = true;
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  await waitFor(() => ddlCalls(state).length === 2);
  const cancel = state.send("cancel", { target: id });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(ddlCalls(state).length, 2);
  assert.equal(state.retired(), 1);
});

test("cancel after DDL ACK preserves ACK but retires uncertain stream outcome", async () => {
  const state = await pinned();
  const stream = new PassThrough({ objectMode: true });
  state.streams.push(stream);
  state.columnSets.push(["status"]);
  const id = state.send(
    "ddl",
    ddlPayload("addColumn", 'ALTER TABLE "TEST"."PUBLIC"."T" ADD COLUMN "C" BOOLEAN'),
  );
  await waitFor(() =>
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
  );
  const cancel = state.send("cancel", { target: id });
  assert.equal(state.terminal(cancel).ok, true);
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "ddlAcknowledged").length,
    1,
  );
  assert.equal(state.calls.filter((call) => call.kind === "cancel").length, 1);
  assert.equal(state.retired(), 1);
});

test("drain failure after DDL ACK preserves ACK and retires", async () => {
  const state = await pinned();
  const stream = new PassThrough({ objectMode: true });
  state.streams.push(stream);
  state.columnSets.push(["status"]);
  const id = state.send(
    "ddl",
    ddlPayload("createTable", 'CREATE TABLE "TEST"."PUBLIC"."T" ("ID" BOOLEAN)'),
  );
  await waitFor(() =>
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
  );
  stream.write(["private"]);
  stream.destroy(new Error("private stream failure"));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "ddlAcknowledged").length,
    1,
  );
  assert.equal(JSON.stringify(state.events).includes("private stream failure"), false);
  assert.equal(state.retired(), 1);
});

test("reentrant protocol violation after DDL ACK preserves acknowledgement and retires", async () => {
  const state = await pinned();
  const originalSend = state.helper.send.bind(state.helper);
  let violated = false;
  state.helper.send = (request, type, rest) => {
    const emitted = originalSend(request, type, rest);
    if (type === "ddlAcknowledged" && !violated) {
      violated = true;
      state.helper.submissionViolation();
    }
    return emitted;
  };
  state.streams.push(Readable.from([], { objectMode: true }));
  state.columnSets.push(["status"]);
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  await waitFor(() => state.terminal(id));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  const events = state.events.filter((event) => event.id === id);
  assert.equal(events.filter((event) => event.type === "ddlAcknowledged").length, 1);
  assert.equal(events.filter((event) => event.type === "terminal").length, 1);
  assert(
    events.findIndex((event) => event.type === "ddlAcknowledged") <
      events.findIndex((event) => event.type === "terminal"),
  );
  assert.equal(
    events.some((event) => ["rows", "columns"].includes(event.type)),
    false,
  );
  assert.equal(state.retired(), 1);
});

test("close before DDL ACK keeps the possible remote outcome unknown", async () => {
  const state = await pinned();
  state.connection.holdComplete = true;
  const before = ddlCalls(state).length;
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  const close = state.send("close");
  assert.equal(state.terminal(id).error.code, "outcomeUnknown");
  assert.equal(state.terminal(close).ok, true);
  state.connection.pendingComplete();
  await tick();
  assert.equal(ddlCalls(state).length, before + 1);
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "terminal").length,
    1,
  );
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 1);
});

test("close after DDL ACK preserves its separate acknowledgement", async () => {
  const state = await pinned();
  state.streams.push(new PassThrough({ objectMode: true }));
  state.columnSets.push(["status"]);
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  await waitFor(() =>
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
  );
  const close = state.send("close");
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(state.terminal(close).ok, true);
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "ddlAcknowledged").length,
    1,
  );
  assert.equal(state.retired(), 1);
});

test("reentrant close preserves a definitive DDL database rejection", async () => {
  const state = await pinned();
  state.connection.nextError = { sqlState: "42501", message: "private rejection" };
  const originalRecord = state.helper.record.bind(state.helper);
  let close;
  state.helper.record = (active, statement) => {
    originalRecord(active, statement);
    if (active.knownFailure === "database") close = state.send("close");
  };
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  await waitFor(() => state.terminal(id) && state.terminal(close));
  assert.equal(state.terminal(id).error.code, "database");
  assert.equal(state.terminal(close).ok, true);
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(
    state.events.filter((event) => event.id === id && event.type === "terminal").length,
    1,
  );
  assert.equal(state.retired(), 1);
});

test("reentrant close before DDL dispatch prevents execution while logout is pending", async () => {
  const state = await pinned();
  state.connection.destroy = (callback) => setImmediate(() => callback?.());
  const before = ddlCalls(state).length;
  const originalSend = state.helper.send.bind(state.helper);
  let close;
  state.helper.send = (request, type, rest) => {
    const emitted = originalSend(request, type, rest);
    if (request.op === "ddl" && type === "phase" && rest.phase === "dispatching")
      close = state.send("close");
    return emitted;
  };
  const id = state.send("ddl", ddlPayload("dropTable", 'DROP TABLE "TEST"."PUBLIC"."T" RESTRICT'));
  await waitFor(() => state.terminal(close));
  assert.equal(state.terminal(id).error.code, "connectionClosed");
  assert.equal(ddlCalls(state).length, before);
  assert.equal(
    state.events.some((event) => event.id === id && event.type === "ddlAcknowledged"),
    false,
  );
  assert.equal(state.retired(), 1);
});
