import test from "node:test";
import assert from "node:assert/strict";
import { Readable, PassThrough } from "node:stream";
import { Helper } from "../src/helper.mjs";
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
function setup({ pendingConnect = false } = {}) {
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
      setImmediate(() => completes.shift()?.());
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
    { clientConfigFile: "/owned/nonexistent", closeTimeoutMs: 10 },
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

test("context changes and identity queries use the same connection queue", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "READER", "TEST", "OTHER", "TINY", "123456789012345678"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["user", "role", "database", "schema", "warehouse", "session"]);
  const id = s.send("context", { schema: "OTHER" });
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
  assert.equal(s.calls[0].sqlText, 'USE SCHEMA "OTHER"');
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
test("context emits one operation phase pair and mismatched identity retires", async () => {
  const s = await connected();
  s.streams.push(
    Readable.from([], { objectMode: true }),
    Readable.from([["FIXTURE", "WRONG", "TEST", "OTHER", "TINY", "123"]], {
      objectMode: true,
    }),
  );
  s.columnSets.push([], ["u", "r", "d", "s", "w", "i"]);
  const id = s.send("context", { schema: "OTHER" });
  await tick();
  await tick();
  await tick();
  assert.deepEqual(
    s.events.filter((e) => e.id === id && e.type === "phase").map((e) => e.phase),
    ["dispatching"],
  );
  assert.equal(s.terminal(id).error.code, "database");
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
