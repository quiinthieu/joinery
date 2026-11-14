import test from "node:test";
import assert from "node:assert/strict";
import { Readable, PassThrough } from "node:stream";
import { createRequire } from "node:module";
import { CANONICAL_QUERY_PARAMETERS, Helper, connectionOptions } from "../src/helper.mjs";
import { Decoder, encode, FRAME_BYTES } from "../src/framing.mjs";
const generation = "b512e129-c689-48bd-b5c2-848438b1231a";
const tick = () => new Promise((resolve) => setImmediate(resolve));
const profile = {
  account: "fixture-account",
  host: "fixture-account.snowflakecomputing.com",
  username: "FIXTURE",
  role: "READER",
  warehouse: "TINY",
  database: "TEST",
  schema: "PUBLIC",
};
function setup({ pendingConnect = false, submitGuard, synchronousComplete = false } = {}) {
  const events = [],
    calls = [];
  let retired = 0,
    options;
  const streams = [],
    completes = [],
    columnSets = [];
  const connection = {
    connectAsync(callback) {
      if (!pendingConnect) setImmediate(() => callback(null, connection));
      return Promise.resolve(connection);
    },
    destroy(callback) {
      calls.push("destroy");
      if (!connection.holdDestroy) callback();
    },
    execute(opts) {
      calls.push(opts);
      if (connection.nextThrow) throw new Error("private synchronous dispatch failure");
      if (connection.nextError) {
        const err = connection.nextError;
        connection.nextError = null;
        setImmediate(() => opts.complete(err, null));
        return {
          cancel(cb) {
            cb();
          },
        };
      }
      const stream =
        streams.shift() ??
        Readable.from([["9007199254740993", "duplicate"]], {
          objectMode: true,
        });
      const requestId = `11111111-1111-4111-8111-${String(calls.length).padStart(12, "0")}`;
      let acknowledged = false;
      const statement = {
        getRequestId: () => requestId,
        getQueryId: () =>
          acknowledged
            ? "22222222-2222-4222-8222-" + String(calls.length).padStart(12, "0")
            : undefined,
        getColumns: () =>
          (statement.columns ?? ["same", "same"]).map((name) => ({
            getName: () => name,
            getType: () => "text",
          })),
        streamRows: () => stream,
        cancel(callback) {
          calls.push("cancel");
          callback();
        },
      };
      statement.columns = columnSets.shift();
      completes.push(() => {
        acknowledged = true;
        opts.complete(null, statement);
      });
      if (synchronousComplete) completes.shift()?.();
      else if (!connection.holdComplete) setImmediate(() => completes.shift()?.());
      return statement;
    },
  };
  const sdk = {
    createConnection(o) {
      options = o;
      return connection;
    },
  };
  const helper = new Helper(
    sdk,
    (e) => {
      events.push(e);
      return Promise.resolve();
    },
    () => retired++,
    { clientConfigFile: "/owned/nonexistent", closeTimeoutMs: 10, submitGuard },
  );
  let sequence = 0;
  const send = (op, payload = {}, id = String(++sequence)) => {
    const r = { v: 1, generation, id, op, payload };
    helper.receive(r);
    return id;
  };
  const terminal = (id) => events.find((e) => e.id === id && e.type === "terminal");
  const query = (payload = {}) =>
    send("query", {
      sql: "SELECT synthetic",
      binds: [],
      classification: "read",
      credits: 2,
      maxRows: 500,
      maxBytes: 2097152,
      ...payload,
    });
  return {
    helper,
    events,
    calls,
    streams,
    columnSets,
    connection,
    send,
    query,
    terminal,
    options: () => options,
    retired: () => retired,
  };
}
async function connected(opts) {
  const s = setup(opts);
  s.connectId = s.send("connect", profile);
  await tick();
  return s;
}
const switchPayload = (overrides = {}) => ({
  database: profile.database,
  schema: "OTHER",
  expected: {
    user: "FIXTURE",
    role: profile.role,
    warehouse: profile.warehouse,
    session: "opaque-session",
  },
  ...overrides,
});
const transactionPayload = (action, overrides = {}) => ({
  action,
  expected: { user: "FIXTURE", session: "opaque-session" },
  ...overrides,
});
async function pinned(opts) {
  const s = await connected(opts);
  s.streams.push(Readable.from([["FIXTURE", null, "TEST", "PUBLIC", null, "opaque-session"]]));
  s.columnSets.push(["user", "role", "database", "schema", "warehouse", "session"]);
  const id = s.send("context");
  for (let turn = 0; turn < 8 && !s.terminal(id); turn++) await tick();
  assert.equal(s.terminal(id)?.ok, true);
  return s;
}
test("typed transaction executes exactly one fixed boundary on the pinned session", async () => {
  for (const [action, sql] of [
    ["begin", "BEGIN TRANSACTION"],
    ["commit", "COMMIT"],
    ["rollback", "ROLLBACK"],
  ]) {
    const s = await pinned({ synchronousComplete: true });
    s.streams.push(Readable.from([]));
    s.columnSets.push([]);
    const before = s.calls.length;
    const id = s.send("transaction", transactionPayload(action));
    for (let turn = 0; turn < 8 && !s.terminal(id); turn++) await tick();
    const calls = s.calls.slice(before).filter((call) => typeof call === "object");
    assert.equal(calls.length, 1);
    assert.equal(calls[0].sqlText, sql);
    assert.deepEqual(calls[0].binds, []);
    const events = s.events.filter((event) => event.id === id);
    const ack = events.find((event) => event.type === "boundaryAcknowledged");
    assert.equal(ack?.action, action);
    assert.equal(
      ack?.sdkRequestId,
      events.find((event) => event.type === "statement").sdkRequestId,
    );
    assert.equal(s.terminal(id)?.ok, true);
    assert.deepEqual(s.terminal(id).result, {
      action,
      ...Object.fromEntries(["sdkRequestId", "queryId"].map((key) => [key, s.terminal(id)[key]])),
    });
    assert.equal(
      events.some((event) => ["columns", "rows"].includes(event.type)),
      false,
    );
    assert.equal(s.retired(), 0);
    s.helper.stop();
  }
});
test("ordinary context strictly rejects schema payload before SQL", async () => {
  const s = await connected();
  const id = s.send("context", { schema: "OTHER" });
  assert.equal(s.terminal(id)?.error.code, "invalidInput");
  assert.equal(s.calls.length, 0);
});
test("switchSchema retains USE acknowledgement and accepts a nullable live role", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", null, "TEST", "OTHER", null, "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const id = s.send(
    "switchSchema",
    switchPayload({
      expected: { user: "FIXTURE", role: null, warehouse: null, session: "opaque-session" },
    }),
  );
  for (let i = 0; i < 5; i++) await tick();
  assert.equal(s.terminal(id)?.ok, true);
  assert.equal(s.terminal(id).result.role, null);
  assert.equal(s.events.filter((e) => e.id === id && e.type === "useAcknowledged").length, 1);
  assert.equal(s.calls.length, 2);
});
test("length-prefixed decoder handles fragmentation and coalescing", () => {
  const bytes = Buffer.concat([encode({ a: "é" }), encode({ b: 2 })]),
    d = new Decoder();
  const frames = [];
  for (const b of bytes) frames.push(...d.push(Buffer.from([b])));
  d.end();
  assert.deepEqual(frames, [{ a: "é" }, { b: 2 }]);
});
test("frame bounds, malformed UTF-8 and truncated EOF fail closed", () => {
  assert.throws(() => new Decoder().push(Buffer.from([0, 16, 0, 1])));
  assert.throws(() => new Decoder().push(Buffer.from([0, 0, 0, 2, 0xc0, 0xaf])));
  const d = new Decoder();
  d.push(Buffer.from([0, 0]));
  assert.throws(() => d.end());
  assert.throws(() => encode({ x: "x".repeat(FRAME_BYTES) }));
});
test("externalbrowser options explicitly disable caches and keepalive", async () => {
  const s = await connected();
  assert.equal(s.terminal(s.connectId).ok, true);
  const o = s.options();
  assert.equal(o.authenticator, "EXTERNALBROWSER");
  for (const k of [
    "clientStoreTemporaryCredential",
    "clientRequestMFAToken",
    "clientSessionKeepAlive",
    "serverSessionKeepAlive",
  ])
    assert.equal(o[k], false);
  assert.equal(o.rowMode, "array");
  assert.deepEqual(o.fetchAsString, ["Number", "Date", "Boolean", "Buffer", "JSON"]);
  assert.equal(o.clientConfigFile, "/owned/nonexistent");
  assert.equal(s.calls.length, 0);
});
test("credentials fields, mutation classification and numeric binds are rejected", async () => {
  const bad = setup();
  const id = bad.send("connect", { ...profile, password: "DO-NOT-LOG" });
  assert.equal(bad.terminal(id).ok, false);
  assert.equal(bad.options(), undefined);
  const s = await connected();
  const write = s.query({ classification: "write", sql: "INSERT secret" });
  assert.equal(s.terminal(write).error.code, "unsupported");
  const numeric = s.query({ binds: [9007199254740992] });
  assert.equal(s.terminal(numeric).error.code, "invalidInput");
  assert.equal(s.calls.length, 0);
});
test("ordered array rows preserve exact strings, duplicate columns and terminal ordering", async () => {
  const s = await connected(),
    id = s.query();
  await tick();
  await tick();
  assert.deepEqual(s.events.find((e) => e.id === id && e.type === "rows").rows, [
    ["9007199254740993", "duplicate"],
  ]);
  assert.deepEqual(
    s.events.find((e) => e.type === "columns").columns.map((c) => c.name),
    ["same", "same"],
  );
  assert.deepEqual(
    s.events
      .filter((e) => e.id === id)
      .filter((e) => e.type !== "statement")
      .map((e) => e.phase ?? e.type),
    ["accepted", "dispatching", "acknowledged", "columns", "rows", "terminal"],
  );
  assert.equal(s.terminal(id).ok, true);
});
test("zero credits pauses row publication, credit resumes; discard drains cutoff", async () => {
  const s = await connected(),
    id = s.query({ credits: 0 });
  await tick();
  await tick();
  assert.equal(s.terminal(id), undefined);
  assert.equal(
    s.events.some((e) => e.type === "rows"),
    false,
  );
  const c = s.send("credit", { target: id, batches: 1, discard: false });
  await tick();
  assert.equal(s.terminal(c).ok, true);
  assert.equal(s.terminal(id).ok, true);
  s.streams.push(
    Readable.from(
      [
        ["1", "a"],
        ["2", "b"],
        ["3", "c"],
      ],
      { objectMode: true },
    ),
  );
  const cutoff = s.query({ credits: 1, maxRows: 1 });
  await tick();
  await tick();
  assert.deepEqual(
    Object.fromEntries(
      ["rowsSeen", "rowsRetained", "truncated"].map((k) => [k, s.terminal(cutoff).result[k]]),
    ),
    { rowsSeen: 3, rowsRetained: 1, truncated: true },
  );
});
test("successful SDK callback is not terminal before row stream end; late error fails", async () => {
  const s = await connected(),
    stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  const id = s.query();
  await tick();
  assert(s.events.some((e) => e.id === id && e.phase === "acknowledged"));
  assert.equal(s.terminal(id), undefined);
  stream.write(["1", "a"]);
  stream.destroy(new Error("secret SQL token"));
  await tick();
  await tick();
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.retired(), 1);
  assert.equal(JSON.stringify(s.events).includes("secret SQL token"), false);
});
test("cancel bypasses blocked row credits and acknowledgement does not finish query", async () => {
  const s = await connected(),
    stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  const id = s.query({ credits: 0 });
  await tick();
  stream.write(["1", "a"]);
  await tick();
  const cancel = s.send("cancel", { target: id });
  assert.equal(s.terminal(cancel).ok, true);
  assert(s.calls.includes("cancel"));
  assert.equal(s.terminal(id), undefined);
  stream.end();
  await tick();
  await tick();
  assert.equal(s.terminal(id).ok, true);
  assert.equal(s.terminal(id).result.rowsRetained, 0);
});
test("close bypasses active stream and bounded queue, terminal is emitted once", async () => {
  const s = await connected(),
    stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  const active = s.query({ credits: 0 });
  await tick();
  const queued = Array.from({ length: 8 }, () => s.query());
  const busy = s.query();
  assert.equal(s.terminal(busy).error.code, "busy");
  const close = s.send("close");
  assert.equal(s.terminal(close).ok, true);
  assert.equal(s.terminal(active).error.code, "connectionClosed");
  for (const id of queued) assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.retired(), 1);
  stream.end();
  await tick();
  assert.equal(s.events.filter((e) => e.id === active && e.type === "terminal").length, 1);
});
test("pending connect cancel retires its only child without waiting for SDK callback", async () => {
  const s = await connected({ pendingConnect: true });
  assert.equal(s.terminal(s.connectId), undefined);
  const id = s.send("cancel", { target: s.connectId });
  assert.equal(s.terminal(id).ok, true);
  assert.equal(s.terminal(s.connectId).error.code, "cancelled");
  assert.equal(s.retired(), 1);
  assert(s.calls.includes("destroy"));
});
test("duplicate ID and wrong generation retire rather than execute or accept success", async () => {
  const s = await connected();
  s.send("query", {}, s.connectId);
  assert.equal(s.retired(), 1);
  assert.equal(s.calls.length, 0);
  const other = await connected();
  other.helper.receive({
    v: 1,
    generation: "85bc899c-c689-48bd-b5c2-848438b1231a",
    id: "2",
    op: "query",
    payload: {},
  });
  assert.equal(other.retired(), 1);
  assert.equal(other.calls.length, 0);
});

test("schema switching and identity queries use the same connection queue", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "READER", "TEST", "OTHER", "TINY", "123456789012345678"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const id = s.send(
    "switchSchema",
    switchPayload({
      expected: {
        user: "FIXTURE",
        role: profile.role,
        warehouse: profile.warehouse,
        session: "123456789012345678",
      },
    }),
  );
  await tick();
  await tick();
  await tick();
  assert.deepEqual(
    Object.fromEntries(
      ["user", "role", "database", "schema", "warehouse", "session"].map((k) => [
        k,
        s.terminal(id).result[k],
      ]),
    ),
    {
      user: "FIXTURE",
      role: "READER",
      database: "TEST",
      schema: "OTHER",
      warehouse: "TINY",
      session: "123456789012345678",
    },
  );
  assert.deepEqual(
    s.events.filter((e) => e.id === id && e.type === "phase").map((e) => e.phase),
    ["dispatching", "acknowledged"],
  );
  assert.equal(s.calls[0].sqlText, 'USE SCHEMA "TEST"."OTHER"');
  assert(s.calls[1].sqlText.startsWith("SELECT CURRENT_USER()"));
  assert.equal(
    s.events.some((e) => e.id === id && e.type === "rows"),
    false,
  );
});
test("retention byte cutoff drains all rows without claiming failure", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from(
      [
        ["1", "x".repeat(262144)],
        ["2", "ok"],
      ],
      { objectMode: true },
    ),
  );
  const id = s.query({ credits: 0 });
  await tick();
  await tick();
  assert.equal(s.terminal(id).result.rowsSeen, 2);
  assert.equal(s.terminal(id).result.rowsRetained, 0);
  assert.equal(s.terminal(id).result.truncated, true);
});

test("unacknowledged transport failure retires unknown outcome; known SQL error stays usable", async () => {
  const known = await connected();
  known.connection.nextError = { sqlState: "42000", message: "secret SQL" };
  const bad = known.query();
  await tick();
  await tick();
  assert.equal(known.terminal(bad).error.code, "database");
  assert.equal(known.retired(), 0);
  const good = known.query();
  await tick();
  await tick();
  assert.equal(known.terminal(good).ok, true);
  const unknown = await connected();
  unknown.connection.nextError = new Error("private credential");
  const id = unknown.query();
  await tick();
  await tick();
  assert.equal(unknown.terminal(id).error.code, "outcomeUnknown");
  assert.equal(unknown.retired(), 1);
  assert.equal(
    unknown.events.some((e) => e.id === id && e.phase === "acknowledged"),
    false,
  );
});

test("explicit close waits for SDK logout callback or bounded uncertain timeout", async () => {
  const s = await connected();
  s.connection.holdDestroy = true;
  const id = s.send("close");
  assert.equal(s.terminal(id), undefined);
  assert.equal(s.retired(), 0);
  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.retired(), 1);
  const good = await connected();
  const close = good.send("close");
  assert.deepEqual(good.terminal(close).result, {
    closed: true,
    logoutAcknowledged: true,
  });
});
test("synchronous SDK throw after dispatch is conservatively unknown and retires", async () => {
  const s = await connected();
  s.connection.nextThrow = true;
  const id = s.query();
  await tick();
  assert.equal(s.terminal(id).error.code, "outcomeUnknown");
  assert.equal(s.retired(), 1);
  assert.equal(s.calls.filter((x) => typeof x === "object").length, 1);
});

test("real row batches use one credit per <=128 rows and flush first row promptly", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from(
      Array.from({ length: 300 }, (_, i) => [String(i), "x"]),
      { objectMode: true },
    ),
  );
  const id = s.query({ credits: 3, maxRows: 1000 });
  await tick();
  await tick();
  assert.deepEqual(
    s.events.filter((e) => e.id === id && e.type === "rows").map((e) => e.rows.length),
    [128, 128, 44],
  );
  assert.equal(s.terminal(id).result.rowsRetained, 300);
  const slow = new PassThrough({ objectMode: true });
  s.streams.push(slow);
  const latency = s.query({ credits: 1 });
  await tick();
  slow.write(["1", "first"]);
  await new Promise((resolve) => setTimeout(resolve, 45));
  assert.equal(s.events.find((e) => e.id === latency && e.type === "rows").rows.length, 1);
  assert.equal(s.terminal(latency), undefined);
  slow.end();
  await tick();
});
test("switch emits one operation phase pair and mismatched identity retires", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "WRONG", "TEST", "OTHER", "TINY", "123"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["u", "r", "d", "s", "w", "i"]);
  const id = s.send("switchSchema", switchPayload());
  await tick();
  await tick();
  await tick();
  assert.deepEqual(
    s.events.filter((e) => e.id === id && e.type === "phase").map((e) => e.phase),
    ["dispatching"],
  );
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.retired(), 1);
});

for (const control of ["cancel", "discard"]) {
  test(`queued accepted query ${control} removes work before SDK dispatch`, async () => {
    const s = await connected();
    const firstStream = new PassThrough({ objectMode: true });
    s.streams.push(firstStream);
    const first = s.query({ credits: 0 });
    await tick();
    const queued = s.query({ sql: "SELECT never_dispatched" });
    assert(s.events.some((e) => e.id === queued && e.type === "accepted"));
    const request =
      control === "cancel"
        ? s.send("cancel", { target: queued })
        : s.send("credit", { target: queued, batches: 0, discard: true });
    assert.equal(s.terminal(request)?.ok, true);
    assert.equal(s.terminal(queued)?.error.code, "cancelled");
    firstStream.end();
    await tick();
    await tick();
    assert.equal(s.terminal(first)?.ok, true);
    assert.equal(
      s.calls.filter((c) => typeof c === "object" && c.sqlText === "SELECT never_dispatched")
        .length,
      0,
    );
    assert.equal(s.retired(), 0);
    s.send("close");
  });
}

// This seam checks helper ownership/ordering only. Native transport behavior is
// separately exercised by query-submit-guard and actual pinned-SDK fault tests.
function trackingGuard() {
  const calls = [];
  let owner, ticket;
  return {
    calls,
    setAuthority(value) {
      calls.push(["authority", value]);
    },
    beginOperation() {
      assert.equal(owner, undefined);
      owner = {};
      calls.push(["operation"]);
      return owner;
    },
    beginStatement(value) {
      assert.equal(value, owner);
      assert.equal(ticket, undefined);
      ticket = {};
      calls.push(["statement"]);
      return ticket;
    },
    registerRequestId(value, id) {
      assert.equal(value, ticket);
      assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
      calls.push(["register", id]);
    },
    endStatement(value) {
      assert.equal(value, ticket);
      ticket = undefined;
      calls.push(["endStatement"]);
    },
    endOperation(value) {
      assert.equal(value, owner);
      assert.equal(ticket, undefined);
      owner = undefined;
      calls.push(["endOperation"]);
    },
  };
}

test("submit ownership keeps exact statement through drain and cancel acknowledgement", async () => {
  const guard = trackingGuard();
  const x = setup({ submitGuard: guard });
  const stream = new PassThrough({ objectMode: true });
  x.streams.push(stream);
  x.send("connect", profile);
  await tick();
  assert.deepEqual(guard.calls, [
    ["authority", { protocol: "https:", hostname: profile.host, port: 443 }],
  ]);
  assert.equal(x.options().arrayBindingThreshold, 1_000_000);
  const query = x.query();
  await tick();
  assert.deepEqual(
    guard.calls.map(([kind]) => kind),
    ["authority", "operation", "statement", "register"],
  );
  const cancel = x.send("cancel", { target: query });
  assert.equal(x.terminal(cancel).ok, true);
  assert.equal(x.terminal(query), undefined);
  assert.equal(
    guard.calls.some(([kind]) => kind === "endStatement"),
    false,
  );
  const end = guard.endOperation.bind(guard);
  guard.endOperation = (owner) => {
    assert.equal(x.terminal(query).ok, true);
    end(owner);
  };
  stream.end(["value", "value"]);
  await tick();
  await tick();
  assert.deepEqual(
    guard.calls.map(([kind]) => kind),
    ["authority", "operation", "statement", "register", "endStatement", "endOperation"],
  );
  assert.equal(x.retired(), 0);
});

test("synchronous completion is processed only after exact returned ID registration", async () => {
  const guard = trackingGuard();
  const x = setup({ submitGuard: guard, synchronousComplete: true });
  x.send("connect", profile);
  await tick();
  const execute = x.connection.execute.bind(x.connection);
  x.connection.execute = (options) => {
    const complete = options.complete;
    options.complete = (...args) => {
      assert.equal(
        guard.calls.some(([kind]) => kind === "register"),
        false,
      );
      complete(...args);
    };
    const statement = execute(options);
    const columns = statement.getColumns;
    statement.getColumns = () => {
      assert.equal(guard.calls.at(-1)[0], "register");
      return columns();
    };
    return statement;
  };
  const query = x.query();
  await tick();
  await tick();
  assert.equal(x.terminal(query).ok, true);
  assert.deepEqual(
    guard.calls.map(([kind]) => kind),
    ["authority", "operation", "statement", "register", "endStatement", "endOperation"],
  );
});

test("switch USE and readback each own a distinct ticket under one operation", async () => {
  const guard = trackingGuard();
  const x = setup({ submitGuard: guard });
  x.send("connect", profile);
  await tick();
  x.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from(
      [["FIXTURE", profile.role, profile.database, "OTHER", profile.warehouse, "opaque-session"]],
      { objectMode: true },
    ),
  );
  x.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const context = x.send("switchSchema", switchPayload());
  for (let i = 0; i < 5; i++) await tick();
  assert.equal(x.terminal(context).ok, true);
  assert.deepEqual(
    guard.calls.map(([kind]) => kind),
    [
      "authority",
      "operation",
      "statement",
      "register",
      "endStatement",
      "statement",
      "register",
      "endStatement",
      "endOperation",
    ],
  );
  const ids = guard.calls.filter(([kind]) => kind === "register").map(([, id]) => id);
  assert.equal(new Set(ids).size, 2);
});

test("submission violation during acknowledged drain preserves known outcome and retires", async () => {
  const x = setup();
  const stream = new PassThrough({ objectMode: true });
  x.streams.push(stream);
  x.send("connect", profile);
  await tick();
  const query = x.query();
  await tick();
  const queued = x.query();
  x.helper.submissionViolation();
  assert.equal(x.terminal(query).error.code, "connectionClosed");
  assert.equal(x.terminal(queued).error.code, "connectionClosed");
  assert.equal(x.retired(), 1);
  stream.end(["value", "value"]);
  await tick();
  x.helper.submissionViolation();
  assert.equal(
    x.events.filter((event) => event.id === query && event.type === "terminal").length,
    1,
  );
  assert.equal(x.retired(), 1);
});

test("late submission violation preserves established successful terminal", async () => {
  const x = setup();
  x.send("connect", profile);
  await tick();
  const query = x.query();
  await tick();
  await tick();
  const terminal = x.terminal(query);
  assert.equal(terminal.ok, true);
  x.helper.submissionViolation();
  assert.equal(x.terminal(query), terminal);
  assert.equal(
    x.events.filter((event) => event.id === query && event.type === "terminal").length,
    1,
  );
  assert.equal(x.retired(), 1);
});

test("submission violation before current statement acknowledgement remains unknown", async () => {
  const x = setup();
  x.send("connect", profile);
  await tick();
  x.connection.holdComplete = true;
  const query = x.query();
  x.helper.submissionViolation();
  assert.equal(x.terminal(query).error.code, "outcomeUnknown");
  assert.equal(x.retired(), 1);
});

test("USE acknowledgement survives pending readback violation without claiming verified context", async () => {
  const x = setup();
  x.send("connect", profile);
  await tick();
  x.streams.push(Readable.from([], { objectMode: true }));
  x.columnSets.push([]);
  const execute = x.connection.execute.bind(x.connection);
  let statements = 0;
  x.connection.execute = (options) => {
    statements++;
    if (statements === 2) x.connection.holdComplete = true;
    return execute(options);
  };
  const context = x.send("switchSchema", switchPayload());
  for (let i = 0; i < 3; i++) await tick();
  assert.equal(statements, 2);
  assert.equal(x.terminal(context), undefined);
  x.helper.submissionViolation();
  assert.equal(x.terminal(context).error.code, "connectionClosed");
  assert.equal(x.events.filter((e) => e.id === context && e.type === "useAcknowledged").length, 1);
  assert.equal(
    x.events.some((e) => e.id === context && e.phase === "acknowledged"),
    false,
  );
  assert.equal(x.retired(), 1);
});

test("definitive SQL failure survives violation before its pump terminal", async () => {
  const x = setup();
  x.send("connect", profile);
  await tick();
  x.connection.nextError = { sqlState: "42501" };
  const execute = x.connection.execute.bind(x.connection);
  x.connection.execute = (options) => {
    const complete = options.complete;
    options.complete = (...args) => {
      complete(...args);
      x.helper.submissionViolation();
    };
    return execute(options);
  };
  const query = x.query();
  await tick();
  assert.equal(x.terminal(query).error.code, "database");
  assert.equal(
    x.events.filter((event) => event.id === query && event.type === "terminal").length,
    1,
  );
  assert.equal(x.retired(), 1);
});

test("a prior definitive SQL failure cannot settle a later pending query", async () => {
  const x = setup();
  x.send("connect", profile);
  await tick();
  x.connection.nextError = { sqlState: "42501" };
  const first = x.query();
  await tick();
  assert.equal(x.terminal(first).error.code, "database");
  assert.equal(x.retired(), 0);
  x.connection.holdComplete = true;
  const next = x.query();
  x.helper.submissionViolation();
  assert.equal(x.terminal(next).error.code, "outcomeUnknown");
  assert.equal(x.terminal(first).error.code, "database");
  assert.equal(x.retired(), 1);
});

function identityRows(s, rows) {
  s.streams.push(Readable.from(rows, { objectMode: true }));
  s.columnSets.push(["user", "role", "database", "schema", "warehouse", "session"]);
}
test("empty context observes NULL live fields without profile fallback and pins only identity", async () => {
  const s = await connected();
  identityRows(s, [["SERVER_NAME", null, null, null, null, "session-alpha"]]);
  const id = s.send("context", {});
  for (let i = 0; i < 3; i++) await tick();
  assert.equal(s.terminal(id).ok, true);
  assert.equal(s.terminal(id).result.role, null);
  assert.equal(s.calls.length, 1);
  identityRows(s, [["OTHER_USER", null, null, null, null, "session-alpha"]]);
  const changed = s.send("context", {});
  for (let i = 0; i < 3; i++) await tick();
  assert.equal(s.terminal(changed).error.code, "database");
  assert.equal(s.retired(), 1);
});
test("switch quotes each Unicode identifier component without interpreting punctuation", async () => {
  const s = await connected({ synchronousComplete: true });
  const schema = 'Sales.\"雪; --';
  s.streams.push(Readable.from([], { objectMode: true }));
  s.columnSets.push([]);
  identityRows(s, [["FIXTURE", "READER", "TEST", schema, "TINY", "opaque-session"]]);
  const id = s.send("switchSchema", switchPayload({ schema }));
  for (let i = 0; i < 3; i++) await tick();
  assert.equal(s.terminal(id).ok, true);
  assert.equal(s.calls[0].sqlText, `USE SCHEMA "TEST"."${schema.replaceAll('"', '""')}"`);
  assert.equal(s.calls.length, 2);
  const events = s.events.filter((e) => e.id === id);
  assert(
    events.findIndex((e) => e.type === "useAcknowledged") <
      events.findIndex((e) => e.phase === "acknowledged"),
  );
});
test("switch payload rejects coercible, malformed, oversized and cross-database names before SQL", async () => {
  const invalid = [
    null,
    [],
    ["OTHER"],
    1,
    true,
    {},
    "",
    "bad\0name",
    "x".repeat(256),
    "\ud800",
    "😀".repeat(256),
  ];
  for (const key of ["database", "schema"])
    for (const value of invalid) {
      const s = await connected();
      const id = s.send("switchSchema", switchPayload({ [key]: value }));
      assert.equal(s.terminal(id)?.error.code, "invalidInput");
      assert.equal(s.calls.length, 0);
    }
  for (const payload of [
    switchPayload({ database: "OTHER" }),
    switchPayload({ extra: true }),
    { database: "TEST", schema: "OTHER" },
    switchPayload({ expected: { user: "FIXTURE", session: "opaque-session" } }),
    switchPayload({ expected: { ...switchPayload().expected, role: [] } }),
  ]) {
    const s = await connected();
    const id = s.send("switchSchema", payload);
    assert.equal(s.terminal(id)?.error.code, "invalidInput");
    assert.equal(s.calls.length, 0);
  }
});
test("255 Unicode codepoints remain valid and UTF-8 bounded", async () => {
  const s = await connected();
  const schema = "😀".repeat(255);
  s.streams.push(Readable.from([], { objectMode: true }));
  s.columnSets.push([]);
  identityRows(s, [["FIXTURE", "READER", "TEST", schema, "TINY", "opaque-session"]]);
  const id = s.send("switchSchema", switchPayload({ schema }));
  for (let i = 0; i < 4; i++) await tick();
  assert.equal(s.terminal(id).ok, true);
});
for (const failure of ["denied", "empty", "duplicate", "mismatch", "opaqueType", "nontext"]) {
  test(`acknowledged USE followed by ${failure} readback retires with retained evidence`, async () => {
    const s = await connected();
    s.streams.push(Readable.from([], { objectMode: true }));
    s.columnSets.push([]);
    const row = ["FIXTURE", "READER", "TEST", "OTHER", "TINY", "opaque-session"];
    if (failure === "mismatch") row[3] = "WRONG";
    if (failure === "opaqueType") row[5] = 123;
    identityRows(s, failure === "empty" ? [] : failure === "duplicate" ? [row, row] : [row]);
    const original = s.connection.execute.bind(s.connection);
    s.connection.execute = (options) => {
      if (s.calls.length === 1 && failure === "denied")
        s.connection.nextError = { sqlState: "42501", message: "private" };
      const statement = original(options);
      if (s.calls.length === 2 && failure === "nontext")
        statement.getColumns = () =>
          row.map((_, i) => ({ getName: () => `c${i}`, getType: () => "fixed" }));
      return statement;
    };
    const id = s.send("switchSchema", switchPayload());
    for (let i = 0; i < 5; i++) await tick();
    assert.equal(s.terminal(id).error.code, "connectionClosed");
    assert.equal(s.events.filter((e) => e.id === id && e.type === "useAcknowledged").length, 1);
    assert.equal(
      s.events.some((e) => e.id === id && e.phase === "acknowledged"),
      false,
    );
    assert.equal(s.retired(), 1);
    assert.equal(s.calls.filter((c) => typeof c === "object").length, 2);
  });
}
test("definitive USE rejection is distinguishable from a failed post-USE readback", async () => {
  const s = await connected();
  s.connection.nextError = { sqlState: "42501" };
  const id = s.send("switchSchema", switchPayload());
  for (let i = 0; i < 3; i++) await tick();
  assert.equal(s.terminal(id).error.code, "database");
  assert.equal(
    s.events.some((e) => e.id === id && e.type === "useAcknowledged"),
    false,
  );
  assert.equal(s.calls.filter((c) => typeof c === "object").length, 1);
});
for (const afterUse of [false, true]) {
  test(`switch cancellation ${afterUse ? "after" : "before"} USE acknowledgement retires without replay`, async () => {
    const s = await connected();
    s.streams.push(Readable.from([], { objectMode: true }));
    s.columnSets.push([]);
    const original = s.connection.execute.bind(s.connection);
    s.connection.execute = (options) => {
      if (!afterUse || s.calls.length === 1) s.connection.holdComplete = true;
      return original(options);
    };
    const id = s.send("switchSchema", switchPayload());
    for (let i = 0; i < 3; i++) await tick();
    const cancel = s.send("cancel", { target: id });
    assert.equal(s.terminal(cancel).ok, true);
    assert.equal(s.terminal(id).error.code, afterUse ? "connectionClosed" : "outcomeUnknown");
    assert.equal(
      s.events.some((e) => e.id === id && e.type === "useAcknowledged"),
      afterUse,
    );
    assert.equal(
      s.events.some((e) => e.id === id && e.phase === "acknowledged"),
      false,
    );
    assert.equal(s.calls.filter((c) => typeof c === "object").length, afterUse ? 2 : 1);
    assert.equal(s.retired(), 1);
  });
}
test("USE acknowledgement is emitted before its drain and survives a reentrant guard violation", async () => {
  const s = await connected();
  const stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  s.columnSets.push([]);
  const id = s.send("switchSchema", switchPayload());
  await tick();
  assert.equal(
    s.events.some((e) => e.id === id && e.type === "useAcknowledged"),
    true,
  );
  s.helper.submissionViolation();
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  stream.end();
  await tick();
  assert.equal(s.calls.filter((c) => typeof c === "object").length, 1);
  assert.equal(s.events.filter((e) => e.id === id && e.type === "terminal").length, 1);
});

test("registered USE acknowledgement precedes reentrant completion-record violation", async () => {
  const s = await connected();
  s.streams.push(Readable.from([], { objectMode: true }));
  s.columnSets.push([]);
  const emit = s.helper.emit;
  s.helper.emit = (event) => {
    const result = emit(event);
    if (event.type === "statement" && event.queryId) s.helper.submissionViolation();
    return result;
  };
  const id = s.send("switchSchema", switchPayload());
  for (let i = 0; i < 4; i++) await tick();
  const events = s.events.filter((e) => e.id === id);
  const ack = events.findIndex((e) => e.type === "useAcknowledged");
  const terminal = events.findIndex((e) => e.type === "terminal");
  assert(ack >= 0 && ack < terminal);
  assert.equal(events[ack].sdkRequestId, events.find((e) => e.type === "statement").sdkRequestId);
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.calls.filter((c) => typeof c === "object").length, 1);
  assert.equal(s.retired(), 1);
});
test("readback definitive failure plus immediate guard violation preserves USE evidence", async () => {
  const s = await connected();
  s.streams.push(Readable.from([], { objectMode: true }));
  s.columnSets.push([]);
  const original = s.connection.execute.bind(s.connection);
  s.connection.execute = (options) => {
    if (s.calls.length === 1) {
      s.connection.nextError = { sqlState: "42501" };
      const complete = options.complete;
      options.complete = (...args) => {
        complete(...args);
        s.helper.submissionViolation();
      };
    }
    return original(options);
  };
  const id = s.send("switchSchema", switchPayload());
  for (let i = 0; i < 4; i++) await tick();
  assert.equal(s.terminal(id).error.code, "connectionClosed");
  assert.equal(s.events.filter((e) => e.id === id && e.type === "useAcknowledged").length, 1);
  assert.equal(s.events.filter((e) => e.id === id && e.type === "terminal").length, 1);
});

test("transaction rejects arbitrary SQL, coerced actions and foreign or missing session pins before dispatch", async () => {
  const s = await pinned();
  const before = s.calls.length;
  for (const payload of [
    transactionPayload("COMMIT"),
    transactionPayload("commit; SELECT 1"),
    transactionPayload(["commit"]),
    transactionPayload("toString"),
    transactionPayload("commit", { sql: "COMMIT" }),
    { action: "commit" },
    transactionPayload("commit", { expected: { user: "FIXTURE", session: "other" } }),
    transactionPayload("commit", { expected: { user: "other", session: "opaque-session" } }),
    transactionPayload("commit", {
      expected: { user: "FIXTURE", session: "opaque-session", role: null },
    }),
    transactionPayload("commit", {
      expected: { user: "FIXTURE", session: String.fromCharCode(0xd800) },
    }),
    transactionPayload("commit", { expected: { user: "FIXTURE", session: "x".repeat(1025) } }),
    transactionPayload("commit", { expected: { user: "FIXTURE", session: null } }),
    transactionPayload("commit", { expected: { user: ["FIXTURE"], session: "opaque-session" } }),
  ]) {
    const id = s.send("transaction", payload);
    assert.equal(s.terminal(id)?.error.code, "invalidInput");
  }
  assert.equal(s.calls.length, before);
  const unpinned = await connected();
  const id = unpinned.send("transaction", transactionPayload("begin"));
  assert.equal(unpinned.terminal(id)?.error.code, "invalidInput");
  assert.equal(unpinned.calls.length, 0);
  s.helper.stop();
  unpinned.helper.stop();
});

test("queued transaction revalidates its pin before executing", async () => {
  const s = await pinned();
  const stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  s.columnSets.push([]);
  const first = s.send("transaction", transactionPayload("begin"));
  await tick();
  const queued = s.send("transaction", transactionPayload("rollback"));
  s.helper.identityPin = { user: "FIXTURE", session: "replacement" };
  stream.end();
  for (let turn = 0; turn < 8 && !s.terminal(queued); turn++) await tick();
  assert.equal(s.terminal(first)?.ok, true);
  assert.equal(s.terminal(queued)?.error.code, "invalidInput");
  assert.equal(s.calls.filter((c) => typeof c === "object").length, 2);
  s.helper.stop();
});

test("transaction ACK survives reentrant guard failure before record or row drain", async () => {
  for (const boundary of ["ack", "record"]) {
    const s = await pinned();
    s.streams.push(new PassThrough({ objectMode: true }));
    s.columnSets.push([]);
    const emit = s.helper.emit;
    s.helper.emit = (event) => {
      const result = emit(event);
      if (
        (boundary === "ack" && event.type === "boundaryAcknowledged") ||
        (boundary === "record" && event.type === "statement" && event.queryId)
      )
        s.helper.submissionViolation();
      return result;
    };
    const id = s.send("transaction", transactionPayload("commit"));
    for (let turn = 0; turn < 8 && !s.terminal(id); turn++) await tick();
    const events = s.events.filter((event) => event.id === id);
    assert.equal(events.filter((event) => event.type === "boundaryAcknowledged").length, 1);
    assert(
      events.findIndex((event) => event.type === "boundaryAcknowledged") <
        events.findIndex((event) => event.type === "terminal"),
    );
    assert.equal(s.terminal(id)?.error.code, "connectionClosed");
    assert.equal(s.retired(), 1);
  }
});

test("transaction cancellation preserves independent pre/post ACK evidence and settles exactly once", async () => {
  for (const acknowledged of [false, true]) {
    const s = await pinned();
    s.connection.holdComplete = !acknowledged;
    const stream = new PassThrough({ objectMode: true });
    s.streams.push(stream);
    s.columnSets.push([]);
    const id = s.send("transaction", transactionPayload("rollback"));
    await tick();
    const cancel = s.send("cancel", { target: id });
    assert.equal(s.terminal(cancel)?.ok, true);
    assert.equal(s.terminal(id)?.error.code, acknowledged ? "connectionClosed" : "outcomeUnknown");
    assert.equal(
      s.events.filter((event) => event.id === id && event.type === "boundaryAcknowledged").length,
      acknowledged ? 1 : 0,
    );
    stream.end();
    await tick();
    assert.equal(
      s.events.filter((event) => event.id === id && event.type === "terminal").length,
      1,
    );
    assert.equal(s.retired(), 1);
  }
});

test("queued transaction cancellation cannot dispatch and cannot discard an active boundary", async () => {
  const s = await pinned();
  const stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  s.columnSets.push([]);
  const active = s.send("transaction", transactionPayload("begin"));
  await tick();
  const queued = s.send("transaction", transactionPayload("commit"));
  const cancel = s.send("cancel", { target: queued });
  assert.equal(s.terminal(cancel)?.result.cancelledLocally, true);
  assert.equal(s.terminal(queued)?.error.code, "cancelled");
  const credit = s.send("credit", { target: active, discard: true, batches: 0 });
  assert.equal(s.terminal(credit)?.error.code, "invalidInput");
  assert.equal(s.calls.filter((call) => typeof call === "object").length, 2);
  stream.end();
  for (let turn = 0; turn < 5; turn++) await tick();
  assert.equal(s.terminal(active)?.ok, true);
  s.helper.stop();
});

test("definitive boundary rejection has no success ACK and leaves same-session rollback possible", async () => {
  const s = await pinned();
  s.connection.nextError = { sqlState: "42501", message: "PRIVATE_DATABASE_ERROR" };
  const denied = s.send("transaction", transactionPayload("commit"));
  for (let turn = 0; turn < 8 && !s.terminal(denied); turn++) await tick();
  assert.equal(s.terminal(denied)?.error.code, "database");
  assert.equal(
    s.events.some((event) => event.id === denied && event.type === "boundaryAcknowledged"),
    false,
  );
  assert.equal(s.retired(), 0);
  s.streams.push(Readable.from([]));
  s.columnSets.push([]);
  const rollback = s.send("transaction", transactionPayload("rollback"));
  for (let turn = 0; turn < 8 && !s.terminal(rollback); turn++) await tick();
  assert.equal(s.terminal(rollback)?.ok, true);
  assert.equal(
    s.events.some((event) => event.id === rollback && event.type === "boundaryAcknowledged"),
    true,
  );
  assert.equal(JSON.stringify(s.events).includes("PRIVATE_DATABASE_ERROR"), false);
  s.helper.stop();
});

test("boundary drain failure retains ACK without claiming transaction disposition", async () => {
  const s = await pinned();
  const stream = new PassThrough({ objectMode: true });
  s.streams.push(stream);
  s.columnSets.push([]);
  const id = s.send("transaction", transactionPayload("commit"));
  await tick();
  stream.destroy(new Error("PRIVATE_DRAIN_ERROR"));
  for (let turn = 0; turn < 8 && !s.terminal(id); turn++) await tick();
  assert.equal(s.terminal(id)?.error.code, "connectionClosed");
  assert.equal(
    s.events.filter((event) => event.id === id && event.type === "boundaryAcknowledged").length,
    1,
  );
  assert.equal(s.retired(), 1);
  assert.equal(JSON.stringify(s.events).includes("PRIVATE_DRAIN_ERROR"), false);
});

test("cancel reentry cannot erase an already classified definitive boundary failure", async () => {
  const s = await pinned();
  const record = s.helper.record.bind(s.helper);
  let cancel;
  s.helper.record = (active, statement) => {
    if (active.knownFailure === "database" && !cancel) {
      cancel = s.send("cancel", { target: active.request.id });
    }
    record(active, statement);
  };
  s.connection.nextError = { sqlState: "42501", message: "PRIVATE_DATABASE_ERROR" };
  const id = s.send("transaction", transactionPayload("commit"));
  for (let turn = 0; turn < 8 && !s.terminal(id); turn++) await tick();
  assert.equal(s.terminal(cancel)?.ok, true);
  assert.equal(s.terminal(id)?.error.code, "database");
  assert.equal(
    s.events.some((e) => e.id === id && e.type === "boundaryAcknowledged"),
    false,
  );
  assert.equal(s.events.filter((e) => e.id === id && e.type === "terminal").length, 1);
  assert.equal(s.retired(), 1);
  assert.equal(JSON.stringify(s.events).includes("PRIVATE_DATABASE_ERROR"), false);
});

test("console timeout connect policy is closed, bounded and preserves ordinary SDK options", async () => {
  const ordinary = connectionOptions(profile, "/owned/nonexistent");
  assert.equal(Object.hasOwn(ordinary, "timeout"), false);
  assert.equal(Object.hasOwn(ordinary, "reviewedConsoleTimeoutSeconds"), false);
  for (const [seconds, milliseconds] of [
    [1, 90000],
    [65, 90000],
    [120, 125000],
    [86400, 86405000],
  ]) {
    const s = setup();
    const id = s.send("connect", { ...profile, reviewedConsoleTimeoutSeconds: seconds });
    await tick();
    assert.equal(s.terminal(id)?.ok, true);
    assert.deepEqual(s.options(), { ...ordinary, timeout: milliseconds });
    assert.equal(Object.hasOwn(s.options(), "reviewedConsoleTimeoutSeconds"), false);
  }
  for (const value of [null, 0, -1, 1.5, 86401, 4294967296, "120", {}, [], true]) {
    const s = setup();
    const id = s.send("connect", { ...profile, reviewedConsoleTimeoutSeconds: value });
    await tick();
    assert.equal(s.terminal(id)?.error.code, "invalidInput");
    assert.equal(s.options(), undefined);
    assert.throws(
      () =>
        connectionOptions(
          { ...profile, reviewedConsoleTimeoutSeconds: value },
          "/owned/nonexistent",
        ),
      RangeError,
    );
  }
  for (const extra of [
    { timeout: 86405000 },
    { requestConfig: { timeout: 86405000 } },
    { parameters: { STATEMENT_TIMEOUT_IN_SECONDS: 86400 } },
  ]) {
    const s = setup();
    const id = s.send("connect", { ...profile, ...extra });
    await tick();
    assert.equal(s.terminal(id)?.error.code, "invalidInput");
    assert.equal(s.options(), undefined);
  }
});

test("real snowflake-sdk Column extraction preserves SQL null, boolean lowercase, temporal FF9, hex binary and structured JSON", async () => {
  const require = createRequire(import.meta.url);
  const sdk = require("snowflake-sdk");
  const Column = require("snowflake-sdk/dist/lib/connection/result/column");
  sdk.configure({ logLevel: "OFF", additionalLogToConsole: false });
  const opts = connectionOptions(profile, "/owned/nonexistent");
  assert.equal(opts.representNullAsStringNull, false);
  sdk.createConnection(opts);

  const statementParameters = {
    ...CANONICAL_QUERY_PARAMETERS,
    TIMEZONE: "UTC",
  };
  const metas = [
    { name: "b", type: "boolean", nullable: true },
    { name: "f", type: "real", nullable: true },
    { name: "bin", type: "binary", nullable: true, byteLength: 16 },
    { name: "d", type: "date", nullable: true },
    { name: "t", type: "time", scale: 9, nullable: true },
    { name: "ntz", type: "timestamp_ntz", scale: 9, nullable: true },
    { name: "ltz", type: "timestamp_ltz", scale: 9, nullable: true },
    { name: "tz", type: "timestamp_tz", scale: 9, nullable: true },
    { name: "v", type: "variant", nullable: true },
    { name: "arr", type: "array", nullable: true },
    { name: "obj", type: "object", nullable: true },
    { name: "txt", type: "text", nullable: true },
  ];
  const cols = metas.map(
    (columnMetadata, index) =>
      new Column({
        columnMetadata,
        index,
        statementParameters,
        resultVersion: "1",
        onPrecisionLoss: () => {},
        onNonJsonCompliantVariant: () => {},
      }),
  );
  const extract = (values) => {
    const row = {
      values: [...values],
      _arrayProcessedColumns: new Array(values.length).fill(false),
    };
    return cols.map((col) => col._getRowValueAsString(row));
  };

  assert.deepEqual(extract(new Array(metas.length).fill(null)), new Array(metas.length).fill(null));
  const populated = extract([
    "1",
    "NaN",
    "DEADBEEF",
    "20771",
    "45296.123456789",
    "1794656096.123456789",
    "1794656096.123456789",
    "1794656096.123456789 1860",
    "null",
    '[1,"x"]',
    '{"k":true}',
    "NULL",
  ]);
  assert.equal(populated[0], "TRUE");
  assert.equal(populated[1], "NaN");
  assert.equal(populated[2], "DEADBEEF");
  assert.equal(populated[3], "2026-11-14");
  assert.equal(populated[4], "12:34:56.123456789");
  assert.equal(populated[5], "2026-11-14 11:34:56.123456789");
  assert.equal(populated[6], "2026-11-14 11:34:56.123456789 +00:00");
  assert.equal(populated[7], "2026-11-14 18:34:56.123456789 +07:00");
  assert.equal(populated[8], "null");
  assert.equal(populated[9], '[1,"x"]');
  assert.equal(populated[10], '{"k":true}');
  assert.equal(populated[11], "NULL");

  const events = [];
  const execCalls = [];
  const helper = new Helper(
    {
      configure() {},
      createConnection: () => ({
        connectAsync(cb) {
          setImmediate(() => cb(null, this));
          return Promise.resolve(this);
        },
        destroy(cb) {
          cb();
        },
        execute(execOpts) {
          execCalls.push(execOpts);
          const stmt = {
            getRequestId: () => "11111111-1111-4111-8111-000000000001",
            getQueryId: () => "22222222-2222-4222-8222-000000000001",
            getColumns: () => [
              { getName: () => "flag", getType: () => "boolean" },
              { getName: () => "text_flag", getType: () => "text" },
            ],
            streamRows: () =>
              Readable.from(
                [
                  ["TRUE", "TRUE"],
                  ["FALSE", "FALSE"],
                  [null, null],
                ],
                { objectMode: true },
              ),
            cancel(cb) {
              cb();
            },
          };
          setImmediate(() => execOpts.complete(null, stmt));
          return stmt;
        },
      }),
    },
    (event) => {
      events.push(event);
      return Promise.resolve();
    },
    () => {},
    { clientConfigFile: "/owned/nonexistent" },
  );
  helper.receive({ v: 1, generation, id: "1", op: "connect", payload: profile });
  await tick();
  helper.receive({
    v: 1,
    generation,
    id: "2",
    op: "query",
    payload: {
      sql: "SELECT 1",
      binds: [],
      classification: "read",
      credits: 4,
      maxRows: 100,
      maxBytes: 65536,
    },
  });
  for (let i = 0; i < 4; i++) await tick();
  assert.equal(events.find((e) => e.id === "2" && e.type === "terminal")?.ok, true);
  assert.deepEqual(execCalls[0].parameters, CANONICAL_QUERY_PARAMETERS);
  assert.deepEqual(execCalls[0].fetchAsString, [
    "Number",
    "Date",
    "Boolean",
    "Buffer",
    "JSON",
    "Object",
    "Array",
    "Map",
  ]);
  assert.deepEqual(
    events.filter((e) => e.id === "2" && e.type === "rows").flatMap((e) => e.rows),
    [
      ["true", "TRUE"],
      ["false", "FALSE"],
      [null, null],
    ],
  );
});

test("switchContext switches role, warehouse, database and schema with readback verification and updates currentDatabase", async () => {
  const s = await pinned();
  const baseExpected = {
    user: "FIXTURE",
    role: "ANALYST",
    database: "TEST",
    schema: "PUBLIC",
    warehouse: "WH_XS",
    session: "opaque-session",
  };
  // 1) Switch role -> ANALYST
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "ANALYST", "TEST", "PUBLIC", "WH_XS", "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const roleId = s.send("switchContext", {
    kind: "role",
    role: "ANALYST",
    expected: { ...baseExpected, role: null, warehouse: null },
  });
  for (let i = 0; i < 6 && !s.terminal(roleId); i++) await tick();
  assert.equal(s.terminal(roleId)?.ok, true);
  assert.equal(s.terminal(roleId).result.role, "ANALYST");
  assert(s.calls.some((c) => c?.sqlText === 'USE ROLE "ANALYST"'));

  // 2) Switch warehouse -> WH_LG
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "ANALYST", "TEST", "PUBLIC", "WH_LG", "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const whId = s.send("switchContext", {
    kind: "warehouse",
    warehouse: "WH_LG",
    expected: baseExpected,
  });
  for (let i = 0; i < 6 && !s.terminal(whId); i++) await tick();
  assert.equal(s.terminal(whId)?.ok, true);
  assert.equal(s.terminal(whId).result.warehouse, "WH_LG");
  assert(s.calls.some((c) => c?.sqlText === 'USE WAREHOUSE "WH_LG"'));

  // 3) Switch database -> ANALYTICS (updates currentDatabase!)
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "ANALYST", "ANALYTICS", "PUBLIC", "WH_LG", "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const dbId = s.send("switchContext", {
    kind: "database",
    database: "ANALYTICS",
    schema: null,
    expected: { ...baseExpected, warehouse: "WH_LG" },
  });
  for (let i = 0; i < 6 && !s.terminal(dbId); i++) await tick();
  assert.equal(s.terminal(dbId)?.ok, true);
  assert.equal(s.terminal(dbId).result.database, "ANALYTICS");
  assert(s.calls.some((c) => c?.sqlText === 'USE DATABASE "ANALYTICS"'));

  // 4) Subsequent switchSchema on ANALYTICS succeeds because currentDatabase updated, while old TEST is rejected!
  const staleDbId = s.send("switchSchema", switchPayload({ database: "TEST", schema: "OTHER" }));
  assert.equal(s.terminal(staleDbId)?.error.code, "invalidInput");

  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "ANALYST", "ANALYTICS", "MART", "WH_LG", "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const schemaId = s.send("switchContext", {
    kind: "schema",
    database: "ANALYTICS",
    schema: "MART",
    expected: { ...baseExpected, database: "ANALYTICS", warehouse: "WH_LG" },
  });
  for (let i = 0; i < 6 && !s.terminal(schemaId); i++) await tick();
  assert.equal(s.terminal(schemaId)?.ok, true);
  assert.equal(s.terminal(schemaId).result.schema, "MART");

  // 5) Definitive pre-USE SQL rejection (e.g. ungranted role) preserves helper session
  s.connection.nextError = { sqlState: "42501", message: "Role not granted" };
  const deniedRoleId = s.send("switchContext", {
    kind: "role",
    role: "ACCOUNTADMIN",
    expected: { ...baseExpected, database: "ANALYTICS", schema: "MART", warehouse: "WH_LG" },
  });
  for (let i = 0; i < 6 && !s.terminal(deniedRoleId); i++) await tick();
  assert.equal(s.terminal(deniedRoleId)?.error.code, "database");
  assert.equal(s.retired(), 0);

  // 6) Post-USE readback mismatch retires helper with connectionClosed
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "WRONG_ROLE", "ANALYTICS", "MART", "WH_LG", "opaque-session"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const badId = s.send("switchContext", {
    kind: "role",
    role: "SYSADMIN",
    expected: { ...baseExpected, database: "ANALYTICS", schema: "MART", warehouse: "WH_LG" },
  });
  for (let i = 0; i < 6 && !s.terminal(badId); i++) await tick();
  assert.equal(s.terminal(badId)?.error.code, "connectionClosed");
  assert.equal(s.retired(), 1);
});
