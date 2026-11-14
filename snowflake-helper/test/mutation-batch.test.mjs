import test from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";
import {
  MUTATION_CONTEXT_SQL,
  MUTATION_SETTINGS_SQL,
  MUTATION_LIMITS,
  validateMutationRequest,
} from "../src/mutation-batch.mjs";
import { SQL_CONTRACT } from "../src/sql-contract.mjs";

const generation = "b512e129-c689-48bd-b5c2-848438b1231a";
const expected = Object.freeze({
  user: "FIXTURE",
  role: "WRITER",
  database: "TEST",
  schema: "PUBLIC",
  warehouse: "TINY",
  session: "123456",
});
const profile = {
  account: "fixture",
  host: "fixture.snowflakecomputing.com",
  username: expected.user,
  role: expected.role,
  database: expected.database,
  schema: expected.schema,
  warehouse: expected.warehouse,
};
const unit = (kind = "insert", value = "9007199254740993") => ({
  kind,
  sql:
    kind === "insert"
      ? 'INSERT INTO "T" VALUES (CAST(? AS NUMBER(38,0)))'
      : kind === "update"
        ? 'UPDATE "T" SET "N" = CAST(? AS NUMBER(38,0))'
        : 'DELETE FROM "T" WHERE "N" = CAST(? AS NUMBER(38,0))',
  binds: [value],
});
const payload = (units = [unit()], extra = {}) => ({
  mode: "automatic",
  owner: null,
  rollbackPrior: false,
  expected: { ...expected },
  units,
  ...extra,
});
const tick = () => new Promise((resolve) => setImmediate(resolve));
async function wait(predicate) {
  for (let n = 0; n < 100; n++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("missing synthetic helper event");
}

function guard(capacity = 65536) {
  let remaining = capacity,
    reserved = 0,
    operation = null,
    statement = null;
  const leases = new Map();
  const ids = new Set();
  return {
    setAuthority() {},
    beginOperation() {
      assert.equal(operation, null);
      operation = {};
      return operation;
    },
    reserveCapacity(count) {
      if (remaining - reserved < count) return null;
      const lease = {};
      leases.set(lease, count);
      reserved += count;
      return lease;
    },
    releaseCapacity(lease) {
      assert.notEqual(statement?.lease, lease);
      assert(leases.has(lease));
      reserved -= leases.get(lease);
      leases.delete(lease);
    },
    beginStatement(owner, lease) {
      assert.equal(owner, operation);
      assert.equal(statement, null);
      if (lease) assert(leases.get(lease) > 0);
      const ticket = {};
      statement = { ticket, lease };
      return ticket;
    },
    registerRequestId(ticket, id) {
      assert.equal(statement.ticket, ticket);
      assert(!ids.has(id));
      ids.add(id);
      if (statement.lease) {
        leases.set(statement.lease, leases.get(statement.lease) - 1);
        reserved--;
      } else assert(remaining > reserved);
      remaining--;
    },
    endStatement(ticket) {
      assert.equal(statement.ticket, ticket);
      statement = null;
    },
    endOperation(owner) {
      assert.equal(owner, operation);
      assert.equal(statement, null);
      operation = null;
    },
    snapshot() {
      return { remaining, reserved, ids: ids.size, leases: leases.size };
    },
  };
}

function fixture(options = {}) {
  const events = [],
    calls = [],
    records = new Map();
  const submitGuard = options.guard ?? guard();
  let transaction = options.foreign ?? null,
    serial = 0,
    sequence = 0,
    retired = 0;
  let helper;
  const connection = {
    connectAsync(callback) {
      queueMicrotask(() => callback(null, connection));
      return Promise.resolve(connection);
    },
    destroy(callback) {
      callback?.();
    },
    execute(opts) {
      const n = ++serial;
      calls.push(opts);
      const sdkRequestId =
        options.requestId?.({ n, sql: opts.sqlText }) ??
        `11111111-1111-4111-8111-${String(n).padStart(12, "0")}`;
      const queryId = `22222222-2222-4222-8222-${String(n).padStart(12, "0")}`;
      let acknowledged = false;
      let names = [],
        rows = [];
      if (opts.sqlText === SQL_CONTRACT.bootstrapContext || opts.sqlText === MUTATION_CONTEXT_SQL) {
        names = ["user", "role", "database", "schema", "warehouse", "session"];
        rows = [
          [
            expected.user,
            expected.role,
            expected.database,
            expected.schema,
            expected.warehouse,
            expected.session,
          ],
        ];
        if (opts.sqlText === MUTATION_CONTEXT_SQL) {
          names.push("transaction");
          rows[0].push(transaction);
        }
      } else if (opts.sqlText === SQL_CONTRACT.version) {
        names = ["CURRENT_VERSION()"];
        rows = [["fixture-version"]];
      } else if (opts.sqlText === MUTATION_SETTINGS_SQL) {
        names = ["key", "value"];
        rows = [["QUOTED_IDENTIFIERS_IGNORE_CASE", "false"]];
      } else if (opts.sqlText === "BEGIN TRANSACTION") {
        transaction = String(1000 + n);
        records.set(transaction, "running");
      } else if (opts.sqlText === "COMMIT" || opts.sqlText === "ROLLBACK") {
        if (transaction !== null)
          records.set(transaction, opts.sqlText === "COMMIT" ? "committed" : "rolled back");
        transaction = null;
      } else if (opts.sqlText.startsWith("DESCRIBE TRANSACTION ")) {
        names = ["id", "session", "state"];
        const id = opts.sqlText.slice("DESCRIBE TRANSACTION ".length);
        rows = [[id, expected.session, records.get(id) ?? "running"]];
      } else {
        const kind = opts.sqlText.split(" ")[0].toLowerCase();
        names =
          kind === "update"
            ? ["number of rows updated", "number of multi-joined rows updated"]
            : [`number of rows ${kind === "insert" ? "inserted" : "deleted"}`];
        rows = [kind === "update" ? ["1", "0"] : ["1"]];
      }
      const override =
        options.reply?.({ sql: opts.sqlText, n, names, rows, transaction, helper, calls }) ?? {};
      names = override.names ?? names;
      rows = override.rows ?? rows;
      const statement = {
        getRequestId() {
          options.onRequestId?.({ helper, acknowledged, sql: opts.sqlText });
          return sdkRequestId;
        },
        getQueryId() {
          options.onQueryId?.({ helper, acknowledged, sql: opts.sqlText });
          return acknowledged ? queryId : undefined;
        },
        getColumns: () =>
          names.map((name, i) => ({
            getName: () => name,
            getType: () =>
              override.types?.[i] ??
              (name.startsWith("number of ") ||
              (opts.sqlText.startsWith("DESCRIBE TRANSACTION") && ["id", "session"].includes(name))
                ? "FIXED"
                : "TEXT"),
          })),
        streamRows: () =>
          override.drainError
            ? new Readable({
                objectMode: true,
                read() {
                  this.destroy(new Error("synthetic drain failure"));
                },
              })
            : Readable.from(rows, { objectMode: true }),
        cancel(callback) {
          callback?.();
        },
      };
      const complete = () => {
        acknowledged = !override.error;
        opts.complete(override.error ?? null, statement);
      };
      if (override.hold) fixtureState.resume = complete;
      else if (options.synchronous) complete();
      else queueMicrotask(complete);
      return statement;
    },
  };
  helper = new Helper(
    { createConnection: () => connection },
    (event) => {
      events.push(event);
      return options.onEvent?.(event, helper) ?? Promise.resolve();
    },
    () => retired++,
    { submitGuard, isFileTransfer: (sql) => /^\s*(PUT|GET)\b/i.test(sql), closeTimeoutMs: 1 },
  );
  const send = (op, value = {}) => {
    const id = String(++sequence);
    helper.receive({ v: 1, generation, id, op, payload: value });
    return id;
  };
  const terminal = (id) => events.find((e) => e.id === id && e.type === "terminal");
  const fixtureState = {
    helper,
    events,
    calls,
    submitGuard,
    send,
    terminal,
    retired: () => retired,
    transaction: () => transaction,
  };
  return fixtureState;
}
async function pinned(options) {
  const state = fixture(options);
  let id = state.send("connect", profile);
  await wait(() => state.terminal(id));
  id = state.send("context");
  await wait(() => state.terminal(id));
  assert.equal(state.terminal(id).ok, true);
  return state;
}
async function batch(state, value) {
  const id = state.send("mutationBatch", value);
  await wait(() => state.terminal(id));
  return { id, terminal: state.terminal(id), frames: state.events.filter((e) => e.id === id) };
}
const sqls = (state) => state.calls.map((c) => c.sqlText);

test("automatic generated batch owns one guarded lane with exact per-unit ACK/count evidence", async () => {
  const state = await pinned({ synchronous: true });
  const { frames, terminal } = await batch(
    state,
    payload([unit(), unit("update"), unit("delete")]),
  );
  assert.equal(terminal.ok, true);
  assert.deepEqual(terminal.result, {
    outcome: "committed",
    owner: null,
    transactionId: "1004",
    failure: null,
    priorBatches: 0,
  });
  assert.equal(frames.filter((e) => e.type === "batchCount").length, 3);
  assert(frames.every((e) => !["statement", "phase", "dmlAcknowledged"].includes(e.type)));
  for (const count of frames.filter((e) => e.type === "batchCount")) {
    const at = frames.indexOf(count);
    assert(
      frames
        .slice(0, at)
        .some((e) => e.type === "batchStep" && e.step === count.step && e.phase === "acknowledged"),
    );
    assert(
      frames
        .slice(at + 1)
        .some((e) => e.type === "batchStep" && e.step === count.step && e.phase === "drained"),
    );
  }
  assert(state.calls.slice(1).every((c) => c.parameters.MULTI_STATEMENT_COUNT === 1));
  assert.equal(state.submitGuard.snapshot().reserved, 0);
  assert.equal(state.retired(), 0);
  state.helper.stop();
});

test("late duplicate and multi-join counts stop suffix and prove whole rollback", async () => {
  for (const bad of [
    ["2", "0"],
    ["1", "1"],
  ]) {
    let units = 0;
    const state = await pinned({
      reply: ({ sql }) => (sql.startsWith("UPDATE") && ++units === 2 ? { rows: [bad] } : null),
    });
    const { terminal, frames } = await batch(
      state,
      payload([unit("update"), unit("update"), unit("delete")]),
    );
    assert.equal(terminal.result.outcome, "rolledBack");
    assert.equal(terminal.result.failure, "cardinality");
    assert.equal(units, 2);
    assert(!sqls(state).some((s) => s.startsWith("DELETE")));
    assert(!sqls(state).includes("COMMIT"));
    assert.equal(frames.filter((e) => e.type === "batchCount").at(-1).affectedRows, bad[0]);
    assert(frames.some((e) => e.role === "disposition" && e.phase === "drained"));
    state.helper.stop();
  }
});

test("FIXED statuses preserve exact counts and optional generated UPDATE multi-join metadata", async () => {
  const state = await pinned({
    reply: ({ sql }) =>
      sql.startsWith("UPDATE")
        ? { names: ["number of rows updated"], types: ["FIXED"], rows: [["1"]] }
        : null,
  });
  const result = await batch(state, payload([unit("update")]));
  assert.equal(result.terminal.result.outcome, "committed");
  assert.equal(result.frames.find((e) => e.type === "batchCount").multiJoinedRows, null);
  assert(MUTATION_CONTEXT_SQL.includes("CAST(CURRENT_SESSION() AS VARCHAR)"));
  assert(MUTATION_CONTEXT_SQL.includes("CAST(CURRENT_TRANSACTION() AS VARCHAR)"));
  state.helper.stop();
  const huge = "9007199254740993";
  const failed = await pinned({
    reply: ({ sql }) => (sql.startsWith("INSERT") ? { rows: [[huge]] } : null),
  });
  const rejected = await batch(failed, payload());
  assert.equal(rejected.terminal.result.outcome, "rolledBack");
  assert.equal(rejected.frames.find((e) => e.type === "batchCount").affectedRows, huge);
  failed.helper.stop();
});

test("500 generated units use independent bounded IDs without relaxing ordinary two-ID tracking", async () => {
  const state = await pinned();
  const result = await batch(state, payload(Array.from({ length: 500 }, () => unit())));
  assert.equal(result.terminal.result.outcome, "committed");
  assert.equal(result.frames.filter((e) => e.type === "batchCount").length, 500);
  assert.equal(
    new Set(result.frames.filter((e) => e.type === "batchStatement").map((e) => e.sdkRequestId))
      .size,
    508,
  );
  assert.equal(state.submitGuard.snapshot().reserved, 0);
  state.helper.stop();
});

test("numeric context metadata and unavailable fully drained counts cannot report success", async () => {
  for (const variant of ["context", "count"]) {
    const state = await pinned({
      reply: ({ sql }) =>
        variant === "context" && sql === MUTATION_CONTEXT_SQL
          ? { types: ["TEXT", "TEXT", "TEXT", "TEXT", "TEXT", "FIXED", "FIXED"] }
          : variant === "count" && sql.startsWith("INSERT")
            ? { types: ["TEXT"] }
            : null,
    });
    const result = await batch(state, payload());
    assert.equal(result.terminal.result.outcome, variant === "context" ? "unknown" : "rolledBack");
    assert.equal(state.retired(), variant === "context" ? 1 : 0);
    if (variant === "count") {
      assert.equal(result.terminal.result.failure, "summaryUnavailable");
      assert(result.frames.some((e) => e.type === "batchCountUnavailable"));
    }
    assert(!sqls(state).includes("COMMIT"));
    state.helper.stop();
  }
});

test("unavailable drained count attempts rollback, but missing rollback proof remains unknown", async () => {
  const state = await pinned({
    reply: ({ sql }) =>
      sql.startsWith("INSERT")
        ? { rows: [["unknown"]] }
        : sql.startsWith("DESCRIBE TRANSACTION")
          ? { rows: [] }
          : null,
  });
  const result = await batch(state, payload([unit(), unit("delete")]));
  assert.equal(result.terminal.result.outcome, "unknown");
  assert(result.frames.some((e) => e.type === "batchCountUnavailable"));
  assert(sqls(state).includes("ROLLBACK"));
  assert(!sqls(state).some((s) => s.startsWith("DELETE")));
  assert.equal(state.retired(), 1);
});

test("lost ACK and acknowledged drain failure retire unknown without replay or rollback", async () => {
  for (const response of [{ error: { sqlState: "08006" } }, { drainError: true }]) {
    const state = await pinned({
      reply: ({ sql }) => (sql.startsWith("INSERT") ? response : null),
    });
    const { terminal, frames } = await batch(state, payload([unit(), unit("delete")]));
    assert.equal(terminal.result.outcome, "unknown");
    assert.equal(state.retired(), 1);
    assert(!sqls(state).includes("ROLLBACK"));
    assert(!sqls(state).includes("COMMIT"));
    assert(!sqls(state).some((s) => s.startsWith("DELETE")));
    if (response.drainError)
      assert(frames.some((e) => e.role === "unit" && e.phase === "acknowledged"));
  }
});

test("definitive database rejection rolls back only its owned transaction", async () => {
  const state = await pinned({
    reply: ({ sql }) => (sql.startsWith("INSERT") ? { error: { sqlState: "22000" } } : null),
  });
  const { terminal } = await batch(state, payload());
  assert.equal(terminal.result.outcome, "rolledBack");
  assert.equal(terminal.result.failure, "database");
  assert.equal(state.retired(), 0);
  state.helper.stop();
});

test("manual continuation requires explicit prior rollback consent and reports all previous batches", async () => {
  let units = 0;
  const state = await pinned({
    reply: ({ sql }) => (sql.startsWith("INSERT") && ++units === 2 ? { rows: [["2"]] } : null),
  });
  const first = await batch(state, payload([unit()], { mode: "manual" }));
  assert.equal(first.terminal.result.outcome, "pending");
  const owner = first.terminal.result.owner;
  assert.match(owner, /^[a-f0-9-]{36}$/);
  const before = state.calls.length;
  const denied = await batch(state, payload([unit()], { mode: "manual", owner }));
  assert.equal(denied.terminal.ok, false);
  assert.equal(state.calls.length, before);
  assert.equal(state.helper.mutationOwner.token, owner);
  const next = await batch(
    state,
    payload([unit()], { mode: "manual", owner, rollbackPrior: true }),
  );
  assert.equal(next.terminal.result.outcome, "rolledBack");
  assert.equal(next.terminal.result.priorBatches, 1);
  assert.equal(next.terminal.result.owner, owner);
  assert.equal(first.terminal.result.outcome, "pending");
  assert.equal(state.helper.mutationOwner, null);
  assert.equal(state.submitGuard.snapshot().reserved, 0);
  state.helper.stop();
});

test("manual finish commits with proof or rolls back without needing a fresh observation", async () => {
  for (const action of ["commit", "rollback"]) {
    const state = await pinned();
    const pending = await batch(state, payload([unit()], { mode: "manual" }));
    const before = state.calls.length;
    const id = state.send("mutationFinish", {
      owner: pending.terminal.result.owner,
      action,
      expected: { ...expected },
    });
    await wait(() => state.terminal(id));
    assert.equal(
      state.terminal(id).result.outcome,
      action === "commit" ? "committed" : "rolledBack",
    );
    assert.equal(state.terminal(id).result.priorBatches, 1);
    if (action === "rollback") assert.equal(state.calls[before].sqlText, "ROLLBACK");
    assert.equal(state.submitGuard.snapshot().reserved, 0);
    state.helper.stop();
  }
});

test("foreign transaction, context mismatch, and unsafe quoted setting never BEGIN or rollback", async () => {
  for (const options of [
    { foreign: "777" },
    {
      reply: ({ sql, rows }) =>
        sql === MUTATION_CONTEXT_SQL ? { rows: [["OTHER", ...rows[0].slice(1)]] } : null,
    },
    {
      reply: ({ sql }) =>
        sql === MUTATION_SETTINGS_SQL
          ? { rows: [["QUOTED_IDENTIFIERS_IGNORE_CASE", "true"]] }
          : null,
    },
  ]) {
    const state = await pinned(options);
    const { terminal } = await batch(state, payload());
    assert.equal(terminal.result.outcome, "rejected");
    assert(!sqls(state).includes("BEGIN TRANSACTION"));
    assert(!sqls(state).includes("ROLLBACK"));
    state.helper.stop();
  }
});

test("boundary ACK plus idle cannot substitute for owned transaction disposition", async () => {
  for (const bad of ["running", "rolled back", "mystery"]) {
    const state = await pinned({
      reply: ({ sql, rows }) =>
        sql.startsWith("DESCRIBE TRANSACTION")
          ? { rows: [[rows[0][0], expected.session, bad]] }
          : null,
    });
    const { terminal } = await batch(state, payload());
    assert.equal(terminal.result.outcome, "unknown");
    assert.equal(state.retired(), 1);
    assert.equal(sqls(state).filter((s) => s === "COMMIT").length, 1);
  }
});

test("quota rejection happens before any SQL and protects manual finish allowance", async () => {
  const state = await pinned({ guard: guard(19) });
  const first = await batch(state, payload([unit()], { mode: "manual" }));
  assert.equal(first.terminal.result.outcome, "pending");
  const before = state.calls.length;
  const refused = await batch(
    state,
    payload([unit()], { mode: "manual", owner: first.terminal.result.owner, rollbackPrior: true }),
  );
  assert.equal(refused.terminal.result.outcome, "rejected");
  assert.equal(state.calls.length, before);
  const id = state.send("mutationFinish", {
    owner: first.terminal.result.owner,
    action: "rollback",
    expected: { ...expected },
  });
  await wait(() => state.terminal(id));
  assert.equal(state.terminal(id).result.outcome, "rolledBack");
  state.helper.stop();
});

test("malformed, oversized escaped frame, file transfer, and non-scalar payloads reject before BEGIN", async () => {
  const state = await pinned();
  const bads = [
    payload([]),
    payload(Array(MUTATION_LIMITS.units + 1).fill(unit())),
    payload([{ ...unit(), binds: [1] }]),
    payload([{ ...unit(), binds: [["x"]] }]),
    payload([{ ...unit(), sql: "PUT file://x @stage" }]),
    payload([{ ...unit(), sql: "SELECT\0 1" }]),
    payload([{ ...unit(), binds: ["\u0001".repeat(200000)] }]),
    { ...payload(), extra: true },
  ];
  for (const value of bads) {
    const before = state.calls.length;
    const result = await batch(state, value);
    assert.equal(result.terminal.ok, false);
    assert.equal(state.calls.length, before);
  }
  state.helper.stop();
});

test("queued outsiders and locally cancelled batches cannot enter a newly owned session", async () => {
  let hold = true;
  const state = await pinned({
    reply: ({ sql }) =>
      sql === MUTATION_CONTEXT_SQL && hold ? ((hold = false), { hold: true }) : null,
  });
  const first = state.send("mutationBatch", payload([unit()], { mode: "manual" }));
  await wait(() => state.resume);
  const outsider = state.send("query", {
    sql: "SELECT 999",
    binds: [],
    classification: "read",
    credits: 1,
    maxRows: 1,
    maxBytes: 100,
  });
  const queued = state.send("mutationBatch", payload());
  const cancel = state.send("cancel", { target: queued });
  assert.equal(state.terminal(queued).error.code, "cancelled");
  assert.equal(state.terminal(cancel).result.cancelledLocally, true);
  state.resume();
  await wait(() => state.terminal(first) && state.terminal(outsider));
  assert.equal(state.terminal(first).result.outcome, "pending");
  assert.equal(state.terminal(outsider).error.code, "invalidInput");
  assert(!sqls(state).includes("SELECT 999"));
  state.helper.stop();
});

test("active cancel does not race a cleanup boundary against an undrained statement", async () => {
  const state = await pinned({
    reply: ({ sql }) => (sql.startsWith("INSERT") ? { hold: true } : null),
  });
  const id = state.send("mutationBatch", payload());
  await wait(() => state.resume);
  state.send("cancel", { target: id });
  assert.equal(state.terminal(id).result.outcome, "unknown");
  assert.equal(state.terminal(id).result.failure, "cancelled");
  assert(!sqls(state).includes("ROLLBACK"));
  assert.equal(state.retired(), 1);
  state.resume();
  await tick();
});

test("reentrant close and rejected event promises preserve ACK evidence and stop suffix dispatch", async () => {
  for (const action of ["close", "throw", "reject"]) {
    let fired = false;
    const state = await pinned({
      onEvent: (event, helper) => {
        if (
          !fired &&
          event.type === "batchStep" &&
          event.role === "unit" &&
          event.phase === "acknowledged"
        ) {
          fired = true;
          if (action === "close")
            helper.receive({ v: 1, generation, id: "999", op: "close", payload: {} });
          else if (action === "throw") throw new Error("synthetic observer");
          else return Promise.reject(new Error("synthetic observer"));
        }
      },
    });
    const result = await batch(state, payload([unit(), unit("delete")]));
    assert.equal(result.terminal.result.outcome, "unknown");
    assert(result.frames.some((e) => e.role === "unit" && e.phase === "acknowledged"));
    assert(!sqls(state).some((s) => s.startsWith("DELETE")));
    assert.equal(state.retired(), 1);
  }
});

test("query-ID passive getters never run before the dedicated ACK publication", async () => {
  let state;
  state = await pinned({
    onQueryId: ({ helper, sql }) => {
      if (!helper.active?.batch) return;
      assert(
        state.events.some(
          (e) =>
            e.type === "batchStep" &&
            e.step === helper.active.batch.current.step &&
            e.phase === "acknowledged",
        ),
        sql,
      );
    },
  });
  const result = await batch(state, payload());
  assert.equal(result.terminal.result.outcome, "committed");
  state.helper.stop();
});

test("successful synchronous and asynchronous callbacks retain ACK when request-ID getter throws", async () => {
  for (const synchronous of [true, false]) {
    const state = await pinned({
      synchronous,
      onRequestId: ({ acknowledged, sql }) => {
        if (acknowledged && sql.startsWith("INSERT")) throw new Error("synthetic identity getter");
      },
    });
    const result = await batch(state, payload([unit(), unit("delete")]));
    assert.equal(result.terminal.result.outcome, "unknown");
    const ack = result.frames.find(
      (e) => e.type === "batchStep" && e.role === "unit" && e.phase === "acknowledged",
    );
    assert(ack);
    if (synchronous)
      assert(!result.frames.some((e) => e.type === "batchStatement" && e.step === ack.step));
    assert(!result.frames.some((e) => e.type === "batchCount"));
    assert(!sqls(state).some((s) => s.startsWith("DELETE")));
    assert.equal(state.retired(), 1);
  }
});

test("identity publication failure before asynchronous completion does not discard its later ACK", async () => {
  let failed = false;
  const state = await pinned({
    onEvent: (event, helper) => {
      if (
        !failed &&
        event.type === "batchStatement" &&
        helper.active?.batch?.current.role === "unit"
      ) {
        failed = true;
        return Promise.reject(new Error("synthetic identity publication"));
      }
    },
  });
  const result = await batch(state, payload([unit(), unit("delete")]));
  assert.equal(result.terminal.result.outcome, "unknown");
  assert(
    result.frames.some(
      (e) => e.type === "batchStep" && e.role === "unit" && e.phase === "acknowledged",
    ),
  );
  assert(!sqls(state).some((s) => s.startsWith("DELETE")));
  assert.equal(state.retired(), 1);
});

test("mutation batch invalidates disposable setup bootstrap before further setup SQL", async () => {
  const state = await pinned();
  const version = state.send("query", {
    sql: SQL_CONTRACT.version,
    binds: [],
    classification: "read",
    credits: 1,
    maxRows: 1,
    maxBytes: 65536,
  });
  await wait(() => state.terminal(version));
  assert.equal(state.terminal(version).ok, true);
  const setupPayload = {
    runId: "0123456789ABCDEF0123456789ABCDEF",
    step: 3,
    expected: { user: expected.user, session: expected.session },
  };
  // Establish that the exact admission used below succeeds before the batch;
  // an unrelated malformed setup payload must not make this regression pass.
  assert.equal(state.helper.validateWork({ op: "setup", payload: setupPayload }), true);
  const generated = await batch(state, payload());
  assert.equal(generated.terminal.result.outcome, "committed");
  const before = state.calls.length;
  const setup = state.send("setup", setupPayload);
  await wait(() => state.terminal(setup));
  assert.equal(state.terminal(setup).ok, false);
  assert.equal(state.terminal(setup).error.code, "invalidInput");
  assert.equal(state.calls.length, before);
  state.helper.stop();
});

test("uppercase SDK batch identity preserves ACK but cannot become canonical successful evidence", async () => {
  const state = await pinned({
    synchronous: true,
    requestId: ({ n, sql }) =>
      sql.startsWith("INSERT")
        ? `ABCDEFAB-CDEF-4ABC-8ABC-${String(n).padStart(12, "0")}`
        : undefined,
  });
  const result = await batch(state, payload([unit(), unit("delete")]));
  assert.equal(result.terminal.result.outcome, "unknown");
  const ack = result.frames.find(
    (e) => e.type === "batchStep" && e.role === "unit" && e.phase === "acknowledged",
  );
  assert(ack);
  assert(!result.frames.some((e) => e.type === "batchStatement" && e.step === ack.step));
  assert(!result.frames.some((e) => e.type === "batchCount"));
  assert(!sqls(state).some((s) => s.startsWith("DELETE")));
  assert.equal(state.retired(), 1);
});

test("wide-table UPDATE unit accepts up to 2000 scalar binds and 768 KiB SQL while rejecting overflow", async () => {
  const state = await pinned();
  assert.equal(
    validateMutationRequest(
      {
        op: "mutationBatch",
        payload: payload([
          {
            kind: "update",
            sql: `UPDATE "DB"."PUBLIC"."WIDE" SET ${'"C" = CAST(? AS VARCHAR), '.repeat(11000)}"C0" = CAST(? AS VARCHAR)`,
            binds: new Array(2000).fill("v"),
          },
        ]),
      },
      state.helper,
    ),
    true,
  );
  assert.equal(
    validateMutationRequest(
      {
        op: "mutationBatch",
        payload: payload([
          {
            kind: "update",
            sql: "x".repeat(786433),
            binds: ["v"],
          },
        ]),
      },
      state.helper,
    ),
    false,
  );
  assert.equal(
    validateMutationRequest(
      {
        op: "mutationBatch",
        payload: payload([
          {
            kind: "update",
            sql: 'UPDATE "DB"."PUBLIC"."WIDE" SET "C0" = CAST(? AS VARCHAR)',
            binds: new Array(2001).fill("v"),
          },
        ]),
      },
      state.helper,
    ),
    false,
  );
  state.helper.stop();
});
