import test from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";
import { SQL_CONTRACT } from "../src/sql-contract.mjs";
import { MUTATION_CONTEXT_SQL, MUTATION_SETTINGS_SQL } from "../src/mutation-batch.mjs";
import {
  MUTATION_READ_CONTEXT_SQL,
  MUTATION_READ_POLICY_SQL,
  MUTATION_READ_TRANSACTION_SQL,
  validateMutationReadRequest,
  mutationReadOwnerAdmits,
  mutationReadSpec,
} from "../src/mutation-read.mjs";

const generation = "b512e129-c689-48bd-b5c2-848438b1231a";
const token = "c512e129-c689-48bd-b5c2-848438b1231a";
const expected = Object.freeze({
  user: "FIXTURE",
  role: "WRITER",
  database: "TEST",
  schema: "PUBLIC",
  warehouse: "TINY",
  session: "123456",
});
const target = Object.freeze({
  database: "TEST",
  schema: "PUBLIC",
  name: "T",
  tableType: "BASE TABLE",
});
const basic = new Set(["context", "quotedIdentifierPolicy", "currentTransaction"]);
const payload = (kind = "columns", overrides = {}) => ({
  owner: token,
  expected: { ...expected },
  kind,
  target: basic.has(kind) ? null : { ...target },
  credits: 32,
  ...overrides,
});
const request = (p) => ({ v: 1, generation, id: "1", op: "mutationRead", payload: p });
const identityHelper = () => ({ identityPin: { user: expected.user, session: expected.session } });
const tick = () => new Promise((resolve) => setImmediate(resolve));
async function wait(predicate) {
  for (let i = 0; i < 1000; i++) {
    if (predicate()) return;
    await tick();
  }
  assert.fail("missing synthetic owner-read event");
}

function guard() {
  let operation = null,
    statement = null,
    remaining = 65536,
    reserved = 0;
  const leases = new Map(),
    ids = new Set();
  return {
    setAuthority() {},
    beginOperation() {
      assert.equal(operation, null);
      return (operation = {});
    },
    endOperation(owner) {
      assert.equal(owner, operation);
      assert.equal(statement, null);
      operation = null;
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
      else assert(remaining > reserved);
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
      }
      remaining--;
    },
    endStatement(ticket) {
      assert.equal(statement.ticket, ticket);
      statement = null;
    },
    snapshot: () => ({ remaining, reserved, ids: ids.size, leases: leases.size }),
  };
}

// The SDK is synthetic; the production Helper owns queueing, execution, framing,
// credit, cancellation, and private-owner admission. No network/account is used.
async function fixture(options = {}) {
  const events = [],
    calls = [],
    submitGuard = guard();
  let sequence = 0,
    serial = 0,
    cancelled = 0,
    destroyed = 0,
    transaction = "1004",
    disposition = "committed";
  const state = { events, calls, submitGuard, holds: [] };
  let helper;
  const connection = {
    connectAsync(callback) {
      queueMicrotask(() => callback(null, connection));
      return Promise.resolve(connection);
    },
    destroy(callback) {
      destroyed++;
      callback?.();
    },
    execute(opts) {
      const n = ++serial;
      calls.push(opts);
      let columns = ["VALUE"],
        rows = [["read"]];
      if (opts.sqlText === SQL_CONTRACT.bootstrapContext || opts.sqlText === MUTATION_CONTEXT_SQL) {
        columns = ["user", "role", "database", "schema", "warehouse", "session"];
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
          columns.push("transaction");
          rows[0].push(transaction);
        }
      } else if (
        opts.sqlText === MUTATION_SETTINGS_SQL ||
        opts.sqlText === MUTATION_READ_POLICY_SQL
      ) {
        columns = ["key", "value"];
        rows = [["QUOTED_IDENTIFIERS_IGNORE_CASE", "false"]];
      } else if (opts.sqlText === "COMMIT" || opts.sqlText === "ROLLBACK") {
        transaction = null;
        disposition = opts.sqlText === "COMMIT" ? "committed" : "rolled back";
        columns = [];
        rows = [];
      } else if (opts.sqlText === "DESCRIBE TRANSACTION 1004") {
        columns = ["id", "session", "state"];
        rows = [["1004", expected.session, disposition]];
      } else if (opts.sqlText === MUTATION_READ_TRANSACTION_SQL) {
        columns = ["TRANSACTION_ID"];
        rows = [[transaction]];
      } else if (opts.sqlText === MUTATION_READ_CONTEXT_SQL) {
        columns = [
          "PRINCIPAL",
          "ROLE_NAME",
          "DATABASE_NAME",
          "SCHEMA_NAME",
          "WAREHOUSE_NAME",
          "SESSION_ID",
          "SEARCH_PATH",
          "SECONDARY_ROLES",
        ];
        rows = [
          [
            expected.user,
            expected.role,
            expected.database,
            expected.schema,
            expected.warehouse,
            expected.session,
            '["TEST.PUBLIC"]',
            '{"roles":"WRITER","value":"ALL"}',
          ],
        ];
      } else if (opts.sqlText.endsWith(" LIMIT 0")) {
        columns = ["N"];
        rows = [];
      }
      const override = options.result?.(opts.sqlText, n) ?? {};
      columns = override.columns ?? columns;
      rows = override.rows ?? rows;
      let acknowledged = false;
      const statement = {
        getRequestId: () => `11111111-1111-4111-8111-${String(n).padStart(12, "0")}`,
        getQueryId: () =>
          acknowledged ? `22222222-2222-4222-8222-${String(n).padStart(12, "0")}` : undefined,
        getColumns: () => columns.map((name) => ({ getName: () => name, getType: () => "text" })),
        streamRows: () => Readable.from(rows),
        cancel(callback) {
          cancelled++;
          callback?.();
        },
      };
      const complete = () => {
        acknowledged = !override.error;
        opts.complete(override.error ?? null, statement);
      };
      if (options.hold?.(opts.sqlText, n)) state.holds.push(complete);
      else queueMicrotask(complete);
      return statement;
    },
  };
  helper = new Helper(
    { createConnection: () => connection },
    (event) => {
      events.push(event);
      return options.onEvent?.(event, helper, state) ?? Promise.resolve();
    },
    () => {},
    { submitGuard, closeTimeoutMs: 1, isFileTransfer: () => false },
  );
  Object.assign(state, {
    helper,
    cancelled: () => cancelled,
    destroyed: () => destroyed,
    send(op, p = {}) {
      const id = String(++sequence);
      helper.receive({ v: 1, generation, id, op, payload: p });
      return id;
    },
    terminal: (id) => events.find((e) => e.id === id && e.type === "terminal"),
    frames: (id) => events.filter((e) => e.id === id),
  });
  let id = state.send("connect", {
    account: "fixture",
    host: "fixture.snowflakecomputing.com",
    username: expected.user,
    role: expected.role,
    database: expected.database,
    schema: expected.schema,
    warehouse: expected.warehouse,
  });
  await wait(() => state.terminal(id));
  assert.equal(state.terminal(id).ok, true);
  id = state.send("context");
  await wait(() => state.terminal(id));
  assert.equal(state.terminal(id).ok, true);
  // Seed only the already-established private pending authority being tested;
  // this is not evidence of a real remote BEGIN or a committed SQL outcome.
  state.owner = helper.mutationOwner = {
    token,
    expected: { ...expected },
    transactionId: transaction,
    batches: 2,
    finishCapacity: submitGuard.reserveCapacity(5),
  };
  return state;
}
async function read(state, p) {
  const id = state.send("mutationRead", p);
  await wait(() => state.terminal(id));
  return { id, terminal: state.terminal(id), frames: state.frames(id) };
}

test("owner read factories select all seven fixed metadata shapes and native bounds", () => {
  const cases = [
    ["context", MUTATION_READ_CONTEXT_SQL, 1, 65536],
    ["quotedIdentifierPolicy", MUTATION_READ_POLICY_SQL, 1, 65536],
    ["currentTransaction", MUTATION_READ_TRANSACTION_SQL, 1, 65536],
    [
      "exactRelation",
      "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE, CREATED::VARCHAR AS CREATED, LAST_DDL::VARCHAR AS LAST_DDL FROM \"TEST\".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'TEST' AND TABLE_SCHEMA = 'PUBLIC' AND TABLE_NAME = 'T' ORDER BY TABLE_TYPE, IS_TEMPORARY LIMIT 3",
      3,
      2097152,
    ],
    ["describe", 'DESCRIBE TABLE "TEST"."PUBLIC"."T" TYPE = COLUMNS', 1001, 1048576],
    ["zeroRowResolution", 'SELECT * FROM "TEST"."PUBLIC"."T" LIMIT 0', 0, 1048576],
  ];
  for (const [kind, sql, maxRows, maxBytes] of cases) {
    assert.deepEqual(mutationReadSpec(request(payload(kind))), {
      sql,
      binds: [],
      maxRows,
      maxBytes,
      maxColumns: 1000,
    });
  }
  const columns = mutationReadSpec(request(payload("columns")));
  assert.equal(columns.maxRows, 1001);
  assert.equal(columns.maxBytes, 1048576);
  assert.equal(
    columns.sql,
    "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, ORDINAL_POSITION::VARCHAR AS ORDINAL_POSITION, COLUMN_DEFAULT, IS_NULLABLE, DATA_TYPE, DATA_TYPE_ALIAS, CHARACTER_MAXIMUM_LENGTH::VARCHAR AS CHARACTER_MAXIMUM_LENGTH, CHARACTER_OCTET_LENGTH::VARCHAR AS CHARACTER_OCTET_LENGTH, NUMERIC_PRECISION::VARCHAR AS NUMERIC_PRECISION, NUMERIC_PRECISION_RADIX::VARCHAR AS NUMERIC_PRECISION_RADIX, NUMERIC_SCALE::VARCHAR AS NUMERIC_SCALE, IS_IDENTITY, IDENTITY_GENERATION, IDENTITY_START, IDENTITY_INCREMENT, IDENTITY_ORDERED, COMMENT, EXPRESSION, KIND FROM \"TEST\".INFORMATION_SCHEMA.COLUMNS AS C WHERE TABLE_CATALOG = 'TEST' AND TABLE_SCHEMA = 'PUBLIC' AND TABLE_NAME = 'T' ORDER BY C.ORDINAL_POSITION LIMIT 1001",
  );
});

test("owner read identifiers are quoted as components and backslashes escaped in literals", () => {
  const names = {
    database: 'D"B\\',
    schema: "S'\\",
    name: 'T"; COMMIT;--\\',
    tableType: "TEMPORARY TABLE",
  };
  const p = payload("exactRelation", {
    target: names,
    expected: { ...expected, database: names.database },
  });
  const sql = mutationReadSpec(request(p)).sql;
  assert(sql.includes('FROM "D""B\\".INFORMATION_SCHEMA.TABLES'));
  assert(
    sql.includes(
      "TABLE_CATALOG = 'D\"B\\\\' AND TABLE_SCHEMA = 'S''\\\\' AND TABLE_NAME = 'T\"; COMMIT;--\\\\'",
    ),
  );
  assert.equal(
    mutationReadSpec(request({ ...p, kind: "describe" })).sql,
    'DESCRIBE TABLE "D""B\\"."S\'\\"."T""; COMMIT;--\\" TYPE = COLUMNS',
  );
});

test("owner read rejects extra SQL/binds/limits, unsupported descriptors, malformed names and authority", () => {
  const helper = identityHelper();
  const hostile = [
    null,
    {},
    payload("other"),
    payload("context", { target }),
    payload("columns", { target: null }),
    ...["sql", "binds", "maxRows", "maxBytes"].map((key) => ({
      ...payload(),
      [key]: key === "binds" ? [] : "SELECT 1",
    })),
    ...["VIEW", "DYNAMIC TABLE", "ICEBERG TABLE", "HYBRID TABLE"].map((tableType) =>
      payload("describe", { target: { ...target, tableType } }),
    ),
    ...["", "a\0b", "\ud800", "x".repeat(256), "😀".repeat(256)].map((name) =>
      payload("columns", { target: { ...target, name } }),
    ),
    ...[0, 33, 1.5, "1"].map((credits) => payload("context", { credits })),
    payload("columns", { target: { ...target, database: "OTHER" } }),
    payload("context", { owner: null }),
    payload("context", { owner: "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA" }),
    payload("context", { expected: { ...expected, session: "other" } }),
    payload("context", { expected: { ...expected, extra: "x" } }),
  ];
  for (const p of hostile) assert.equal(validateMutationReadRequest(request(p), helper), false);
  assert.equal(
    validateMutationReadRequest(
      request(payload("columns", { target: { ...target, name: "😀".repeat(255) } })),
      helper,
    ),
    true,
  );
  assert.throws(
    () => mutationReadSpec(request(payload("unknown"))),
    (e) => e.code === "invalidInput",
  );
  const owner = { token, expected };
  Object.assign(helper, { mutationOwner: owner });
  assert.equal(mutationReadOwnerAdmits(helper, request(payload())), true);
  assert.equal(mutationReadOwnerAdmits(helper, request(payload()), { ...owner }), false);
  assert.equal(
    mutationReadOwnerAdmits({ ...helper, mutationOwner: null }, request(payload())),
    false,
  );
  assert.equal(
    mutationReadOwnerAdmits(
      helper,
      request(payload("context", { expected: { ...expected, role: "OTHER" } })),
    ),
    false,
  );
});

test("production Helper streams complete metadata beyond batch's one-row collector and preserves pending owner", async () => {
  const rows = Array.from({ length: 1001 }, (_, i) => [String(i), null, "", "9".repeat(38)]);
  const state = await fixture({
    result: (sql) =>
      sql.includes("INFORMATION_SCHEMA.COLUMNS")
        ? { columns: ["ORDINAL", "DEFAULT", "EXPRESSION", "EXACT"], rows }
        : {},
  });
  try {
    const before = state.submitGuard.snapshot(),
      calls = state.calls.length;
    const result = await read(state, payload());
    assert.equal(result.terminal.ok, true);
    assert.deepEqual(
      result.frames.filter((e) => e.type === "rows").flatMap((e) => e.rows),
      rows,
    );
    assert.equal(result.terminal.result.rowsSeen, 1001);
    assert.equal(result.terminal.result.rowsRetained, 1001);
    assert.equal(result.terminal.result.truncated, false);
    assert.equal(state.calls.length, calls + 1);
    assert.equal(state.calls.at(-1).parameters.MULTI_STATEMENT_COUNT, 1);
    assert.deepEqual(state.calls.at(-1).binds, []);
    assert.equal(state.helper.mutationOwner, state.owner);
    assert.equal(state.owner.batches, 2);
    assert.deepEqual(state.submitGuard.snapshot(), {
      ...before,
      remaining: before.remaining - 1,
      ids: before.ids + 1,
    });
    assert.equal(state.helper.setupBootstrap, -1);
  } finally {
    state.helper.stop();
  }
});

test("production Helper refuses ordinary operations and owner reads without matching live authority", async () => {
  const state = await fixture();
  try {
    const before = state.calls.length;
    for (const [op, p] of [
      ["context", {}],
      [
        "query",
        {
          sql: "SELECT 1",
          binds: [],
          classification: "read",
          credits: 1,
          maxRows: 1,
          maxBytes: 65536,
        },
      ],
      ["mutationRead", payload("context", { owner: generation })],
    ]) {
      const id = state.send(op, p);
      await wait(() => state.terminal(id));
      assert.equal(state.terminal(id).ok, false);
    }
    state.helper.mutationOwner = null;
    assert.equal((await read(state, payload("context"))).terminal.ok, false);
    assert.equal(state.calls.length, before);
  } finally {
    state.helper.stop();
  }
});

test("queued owner read behind verified finish is rejected without dispatch", async () => {
  let hold = true;
  const state = await fixture({ hold: (sql) => sql === MUTATION_CONTEXT_SQL && hold });
  try {
    const finish = state.send("mutationFinish", {
      owner: token,
      action: "commit",
      expected: { ...expected },
    });
    await wait(() => state.holds.length === 1);
    const queued = state.send("mutationRead", payload("columns"));
    assert.equal(state.helper.queue.length, 1);
    hold = false;
    state.holds.shift()();
    await wait(() => state.terminal(finish) && state.terminal(queued));
    assert.equal(state.terminal(finish).result.outcome, "committed");
    assert.equal(state.terminal(queued).ok, false);
    assert.equal(state.helper.mutationOwner, null);
    assert(!state.calls.some((c) => c.sqlText.includes("INFORMATION_SCHEMA.COLUMNS")));
  } finally {
    state.helper.stop();
  }
});

for (const control of ["cancel", "discard"])
  test(`queued owner read ${control} is local and preserves pending authority`, async () => {
    const state = await fixture({ hold: (sql) => sql === MUTATION_READ_CONTEXT_SQL });
    try {
      const active = state.send("mutationRead", payload("context"));
      await wait(() => state.holds.length === 1);
      const queued = state.send("mutationRead", payload("columns"));
      const id = state.send(
        control === "cancel" ? "cancel" : "credit",
        control === "cancel" ? { target: queued } : { target: queued, batches: 0, discard: true },
      );
      await wait(() => state.terminal(id) && state.terminal(queued));
      assert.equal(state.terminal(id).result.cancelledLocally, true);
      assert.equal(state.terminal(queued).error.code, "cancelled");
      assert.equal(state.helper.mutationOwner, state.owner);
      assert.equal(state.owner.batches, 2);
      assert.equal(state.helper.retired, false);
      state.holds.shift()();
      await wait(() => state.terminal(active));
      assert(!state.calls.some((c) => c.sqlText.includes("INFORMATION_SCHEMA.COLUMNS")));
    } finally {
      state.helper.stop();
    }
  });

test("owner read uses ordinary credits and paused drain remains exclusively owned", async () => {
  const rows = Array.from({ length: 257 }, (_, i) => [String(i)]);
  const state = await fixture({
    result: (sql) => (sql.includes("INFORMATION_SCHEMA.COLUMNS") ? { rows } : {}),
  });
  try {
    const id = state.send("mutationRead", payload("columns", { credits: 1 }));
    await wait(() => state.frames(id).filter((e) => e.type === "rows").length === 1);
    assert.equal(state.terminal(id), undefined);
    assert.equal(state.helper.mutationOwner, state.owner);
    const credit = state.send("credit", { target: id, batches: 2, discard: false });
    await wait(() => state.terminal(credit) && state.terminal(id));
    assert.equal(state.terminal(id).ok, true);
    assert.equal(state.terminal(id).result.truncated, false);
    assert.deepEqual(
      state
        .frames(id)
        .filter((e) => e.type === "rows")
        .flatMap((e) => e.rows),
      rows,
    );
  } finally {
    state.helper.stop();
  }
});

test("active owner-read cancellation retires without transaction boundary or suffix SQL", async () => {
  const state = await fixture({ hold: (sql) => sql === MUTATION_READ_CONTEXT_SQL });
  try {
    const id = state.send("mutationRead", payload("context"));
    await wait(() => state.holds.length === 1);
    const calls = state.calls.length,
      cancel = state.send("cancel", { target: id });
    await wait(() => state.terminal(cancel) && state.terminal(id));
    await wait(() => state.destroyed() > 0);
    assert.equal(state.terminal(id).ok, false);
    assert.equal(state.cancelled(), 1);
    assert.equal(state.calls.length, calls);
    assert(!state.calls.some((c) => /^(COMMIT|ROLLBACK)$/.test(c.sqlText)));
    state.holds.shift()();
    await tick();
    assert.equal(state.calls.length, calls);
    assert.equal(state.frames(id).filter((e) => e.type === "terminal").length, 1);
  } finally {
    state.helper.stop();
  }
});

test("reentrant private-owner replacement at dispatch phase prevents SDK execution", async () => {
  let armed = false;
  const state = await fixture({
    onEvent: (event, helper) => {
      if (armed && event.type === "phase" && event.phase === "dispatching")
        helper.mutationOwner = { ...helper.mutationOwner };
    },
  });
  try {
    armed = true;
    const before = state.calls.length;
    assert.equal((await read(state, payload("context"))).terminal.ok, false);
    assert.equal(state.calls.length, before);
  } finally {
    state.helper.stop();
  }
});

test("fixed zero-row and aggregate byte bounds expose incomplete metadata instead of fresh success", async () => {
  for (const [kind, rows] of [
    ["zeroRowResolution", [["unexpected"]]],
    ["columns", Array.from({ length: 10 }, () => ["x".repeat(120000)])],
  ]) {
    const state = await fixture({
      result: (sql) =>
        sql.endsWith(" LIMIT 0") || sql.includes("INFORMATION_SCHEMA.COLUMNS") ? { rows } : {},
    });
    try {
      const result = await read(state, payload(kind));
      assert.equal(result.terminal.ok, true);
      assert.equal(result.terminal.result.truncated, true);
      assert.equal(result.terminal.result.rowsSeen, rows.length);
      assert(result.terminal.result.rowsRetained < rows.length);
      assert.equal(state.helper.mutationOwner, state.owner);
    } finally {
      state.helper.stop();
    }
  }
});
