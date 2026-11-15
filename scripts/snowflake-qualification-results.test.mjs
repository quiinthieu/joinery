import test from "node:test";
import assert from "node:assert/strict";
import {
  RESULT_LIMITS,
  createResultBudget,
  createQueryCollector,
  parseQueryResult,
  parseStartupContext,
  requireContinuity,
  requireSchemasInContext,
  makeObservationSample,
  compareSamples,
} from "./snowflake-qualification-results.mjs";

const names = [
  "PRINCIPAL",
  "ROLE_NAME",
  "DATABASE_NAME",
  "SCHEMA_NAME",
  "WAREHOUSE_NAME",
  "SESSION_ID",
  "SEARCH_PATH",
  "SECONDARY_ROLES",
];
const setup = { role: "ANALYST", database: "DB", schema: "PUBLIC", warehouse: "WH" };
const initial = () => ({ user: "SERVER_NAME", ...setup, session: "opaque-session-A7/x" });
const contextRow = () => [
  "SERVER_NAME",
  "ANALYST",
  "DB",
  "PUBLIC",
  "WH",
  "opaque-session-A7/x",
  '["DB.PUBLIC","\\\"D.B\\\".\\\"S\\\"\\\"Q\\\"","unresolved"]',
  '{"roles":"A,B","value":"ALL"}',
];
const columns = (labels, types = labels.map(() => "text")) =>
  labels.map((name, i) => ({ name, type: types[i] }));
const answer = (labels, rows, types) => ({
  columns: columns(labels, types),
  rows,
  result: { rowsSeen: rows.length, rowsRetained: rows.length, truncated: false },
});
const contextAnswer = () => answer(names, [contextRow()]);
const rejects = (action, code) =>
  assert.throws(action, (error) => {
    assert.equal(error.message, error.safeCode);
    assert.match(error.safeCode, /^[a-zA-Z]+$/);
    return error.safeCode === code;
  });
const clone = (value) => JSON.parse(JSON.stringify(value));
const parseContext = (mutate = () => {}) => {
  const a = contextAnswer();
  mutate(a);
  return parseQueryResult("context", a);
};
const sample = (context = parseContext(), overrides = {}) =>
  makeObservationSample({
    context,
    quotedIdentifierPolicy: false,
    autocommit: true,
    abortOnError: false,
    transactionId: null,
    ...overrides,
  });
function parameter(template, key, value, mutate = () => {}) {
  const a = answer(
    ["key", "value", "default", "level"],
    [[key, value, "uninterpreted", "SESSION"]],
  );
  mutate(a);
  return parseQueryResult(template, a);
}
const schemaAnswer = (rows) => answer(["CATALOG_NAME", "SCHEMA_NAME"], rows);
const scalar = () =>
  answer(
    ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"],
    [["9007199254740993", "TRUE", "joinery-qualification", null]],
    ["fixed", "boolean", "text", "text"],
  );

test("context preserves structured path/roles, opaque session and nullable observed fields", () => {
  const context = parseContext((a) => {
    a.rows[0][1] = null;
    a.rows[0][4] = null;
  });
  assert.equal(context.role, null);
  assert.equal(context.warehouse, null);
  assert.equal(context.sessionId, "opaque-session-A7/x");
  assert.deepEqual(context.searchPath, [
    { raw: "DB.PUBLIC", database: "DB", schema: "PUBLIC" },
    { raw: '"D.B"."S""Q"', database: "D.B", schema: 'S"Q' },
    { raw: "unresolved", database: null, schema: null },
  ]);
  assert.deepEqual(context.secondaryRoles, { roles: "A,B", value: "ALL" });
  assert(Object.isFrozen(context.secondaryRoles));
  assert(Object.isFrozen(context.searchPath[0]));
});

test("wire type casing is accepted while SQL aliases and non-authoritative column names fail", () => {
  assert.equal(
    parseContext((a) => {
      a.columns.forEach((c) => {
        c.type = "TeXt";
      });
    }).principal,
    "SERVER_NAME",
  );
  rejects(
    () =>
      parseContext((a) => {
        a.columns[0].type = "VARCHAR";
      }),
    "resultTypes",
  );
  rejects(
    () =>
      parseContext((a) => {
        a.columns[0].name = "principal";
      }),
    "resultColumns",
  );
  rejects(
    () =>
      parseQueryResult("scalar", {
        ...scalar(),
        columns: columns(
          ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"],
          ["NUMBER", "BOOLEAN", "TEXT", "TEXT"],
        ),
      }),
    "resultTypes",
  );
});

test("terminal-only startup context is distinct from streamed query descriptors and NAME need not be LOGIN", () => {
  const budget = createResultBudget();
  const result = { ...initial(), sdkRequestId: "00000000-0000-0000-0000-000000000001" };
  const parsed = parseStartupContext(result, setup, { budget });
  assert.equal(parsed.principal, "SERVER_NAME");
  assert(!Object.hasOwn(parsed, "sdkRequestId"));
  assert.equal(budget.snapshot().rows, 1);
  assert.equal(budget.snapshot().bytes, Buffer.byteLength(JSON.stringify(initial())));
  rejects(() => parseStartupContext({ ...result, columns: [] }, setup), "resultShape");
  rejects(() => parseStartupContext({ ...result, role: null }, setup), "startupContextMismatch");
  rejects(
    () => parseStartupContext({ ...result, database: "OTHER" }, setup),
    "startupContextMismatch",
  );
  rejects(
    () => parseQueryResult("context", { ...contextAnswer(), columns: undefined }),
    "resultShape",
  );
  assert.equal(requireContinuity(parseContext(), parsed).principal, "SERVER_NAME");
  for (const index of [0, 2, 5])
    rejects(
      () =>
        requireContinuity(
          parseContext((a) => {
            a.rows[0][index] = "DIFFERENT";
          }),
          parsed,
        ),
      "contextIdentityMismatch",
    );
  // Nullable role/schema/warehouse may change in a later truthful observation.
  assert.equal(
    requireContinuity(
      parseContext((a) => {
        a.rows[0][1] = null;
        a.rows[0][3] = null;
        a.rows[0][4] = null;
      }),
      parsed,
    ).role,
    null,
  );
});

test("malformed, oversized, ill-formed and coerced context values fail without exposing input", () => {
  const cases = [
    [0, null, "resultValue"],
    [1, [], "resultValue"],
    [5, "", "resultValue"],
    [5, "x".repeat(1025), "resultValue"],
    [5, "\ud800", "resultValue"],
    [6, "not-json-secret", "resultJson"],
    [6, "null", "resultShape"],
    [6, JSON.stringify(Array(65).fill("DB.PUBLIC")), "resultShape"],
    [7, '{"roles":[],"value":"ALL"}', "resultValue"],
    [7, '{"roles":"A","value":"ALL","extra":"secret"}', "resultShape"],
  ];
  for (const [index, value, code] of cases)
    rejects(
      () =>
        parseContext((a) => {
          a.rows[0][index] = value;
        }),
      code,
    );
  rejects(
    () =>
      parseContext((a) => {
        a.rows[0][0] = "雪".repeat(256);
      }),
    "resultValue",
  );
  assert.equal(
    parseContext((a) => {
      a.rows[0][0] = "雪".repeat(255);
    }).principal.length,
    255,
  );
});

test("SHOW reads exact key/value rather than default and rejects duplicate names and loose booleans", () => {
  assert.equal(parameter("autocommit", "AUTOCOMMIT", "TrUe"), true);
  assert.equal(parameter("abortOnError", "TRANSACTION_ABORT_ON_ERROR", "FALSE"), false);
  assert.equal(
    parameter("quotedIdentifierPolicy", "QUOTED_IDENTIFIERS_IGNORE_CASE", "false"),
    false,
  );
  rejects(
    () => parameter("quotedIdentifierPolicy", "QUOTED_IDENTIFIERS_IGNORE_CASE", "TRUE"),
    "unsupportedPolicy",
  );
  rejects(() => parameter("autocommit", "AUTOCOMMIT_EXTRA", "true"), "parameterKey");
  for (const value of [null, "1", " true", "false ", "TRUE\n"])
    rejects(() => parameter("autocommit", "AUTOCOMMIT", value), "parameterValue");
  rejects(
    () =>
      parameter("autocommit", "AUTOCOMMIT", "true", (a) => {
        a.columns[2].name = "VaLuE";
      }),
    "resultColumns",
  );
  rejects(
    () =>
      parameter("autocommit", "AUTOCOMMIT", "true", (a) => {
        a.columns[1].type = "BOOLEAN";
      }),
    "resultTypes",
  );
  rejects(
    () =>
      parameter("autocommit", "AUTOCOMMIT", "true", (a) => {
        a.columns[0].name = "Key";
      }),
    "resultTypes",
  );
  assert.equal(
    parameter("autocommit", "AUTOCOMMIT", "false", (a) => {
      a.columns[2].type = "FIXED";
      a.rows[0][2] = "1";
    }),
    false,
  );
});

test("all uninterpreted SHOW cells and descriptors count toward fixed budgets", () => {
  rejects(
    () =>
      parameter("autocommit", "AUTOCOMMIT", "true", (a) => {
        a.rows[0][2] = "x".repeat(65536);
      }),
    "resultBytes",
  );
  const c = createQueryCollector("autocommit");
  rejects(
    () => c.addColumns(columns(Array.from({ length: 33 }, (_, i) => `c${i}`))),
    "resultShape",
  );
  const c2 = createQueryCollector("autocommit");
  rejects(
    () => c2.addColumns(columns(Array.from({ length: 32 }, (_, i) => `c${i}${"x".repeat(3000)}`))),
    "resultBytes",
  );
});

test("nullable opaque transaction ID is observed separately from unavailable samples", () => {
  assert.equal(
    parseQueryResult("transaction", answer(["TRANSACTION_ID"], [["TX-A/7:opaque"]])),
    "TX-A/7:opaque",
  );
  assert.equal(parseQueryResult("transaction", answer(["TRANSACTION_ID"], [[null]])), null);
  rejects(
    () => parseQueryResult("transaction", answer(["TRANSACTION_ID"], [[123]])),
    "resultValue",
  );
  rejects(() => parseQueryResult("transaction", answer(["TRANSACTION_ID"], [[""]])), "resultValue");
  assert.equal(compareSamples(null, null), "unavailable");
  assert.equal(compareSamples(null, sample()), "unavailable");
  assert.equal(compareSamples(sample(), sample()), "stable");
  rejects(() => compareSamples({}, sample()), "invalidObservation");
});

test("sample comparison covers every context/mode/activity field and does not retain mutable input", () => {
  const baseline = sample();
  for (const index of [0, 1, 2, 3, 4, 5]) {
    assert.equal(
      compareSamples(
        baseline,
        sample(
          parseContext((a) => {
            a.rows[0][index] = index === 1 ? null : "CHANGED";
          }),
        ),
      ),
      "changed",
    );
  }
  for (const [index, value] of [
    [6, '["DB.OTHER"]'],
    [7, '{"roles":"A","value":"ALL"}'],
    [7, '{"roles":"A,B","value":"NONE"}'],
  ])
    assert.equal(
      compareSamples(
        baseline,
        sample(
          parseContext((a) => {
            a.rows[0][index] = value;
          }),
        ),
      ),
      "changed",
    );
  for (const overrides of [
    { autocommit: false },
    { abortOnError: true },
    { transactionId: "TX-A" },
  ])
    assert.equal(compareSamples(baseline, sample(undefined, overrides)), "changed");
  rejects(() => sample(undefined, { quotedIdentifierPolicy: true }), "invalidObservation");
  rejects(() => sample(undefined, { autocommit: "true" }), "invalidObservation");
  const a = contextAnswer();
  const parsed = parseQueryResult("context", a);
  a.rows[0][0] = "MUTATED";
  assert.equal(parsed.principal, "SERVER_NAME");
});

test("secondary-role duplicate keys, including escaped spelling, fail rather than overwrite evidence", () => {
  for (const raw of [
    '{"roles":"A","roles":"B","value":"ALL"}',
    '{"roles":"A","\\u0072oles":"B","value":"ALL"}',
    '{"value":"ALL","roles":"A","value":"NONE"}',
  ])
    rejects(
      () =>
        parseContext((a) => {
          a.rows[0][7] = raw;
        }),
      "resultShape",
    );
  assert.deepEqual(
    parseContext((a) => {
      a.rows[0][7] = '{"value":"A\\\"B","\\u0072oles":"A,B"}';
    }).secondaryRoles,
    { roles: "A,B", value: 'A"B' },
  );
});

test("schemas bind database, preserve quoted names, reject duplicate/system names, and never invent current schema", () => {
  const parsed = parseQueryResult(
    "schemas",
    schemaAnswer([
      ["DB", "Z"],
      ["DB", 'Dotted.雪"name'],
    ]),
  );
  assert.deepEqual(requireSchemasInContext(parsed, parseContext()), ['Dotted.雪"name', "Z"]);
  assert(!requireSchemasInContext(parsed, parseContext()).includes("PUBLIC"));
  assert.deepEqual(
    requireSchemasInContext(parseQueryResult("schemas", schemaAnswer([])), parseContext()),
    [],
  );
  rejects(
    () =>
      parseQueryResult(
        "schemas",
        schemaAnswer([
          ["DB", "A"],
          ["DB", "A"],
        ]),
      ),
    "schemaValue",
  );
  rejects(
    () => parseQueryResult("schemas", schemaAnswer([["DB", "INFORMATION_SCHEMA"]])),
    "schemaValue",
  );
  rejects(
    () =>
      requireSchemasInContext(
        parseQueryResult("schemas", schemaAnswer([["OTHER", "A"]])),
        parseContext(),
      ),
    "schemaContextMismatch",
  );
  const rows = Array.from({ length: 1000 }, (_, i) => ["DB", `SCHEMA_${i}`]);
  assert.equal(parseQueryResult("schemas", schemaAnswer(rows)).length, 1000);
  rejects(
    () => parseQueryResult("schemas", schemaAnswer([...rows, ["DB", "SENTINEL"]])),
    "schemaSentinel",
  );
});

test("schema content is bounded incrementally before semantic parsing", () => {
  const c = createQueryCollector("schemas");
  c.addColumns(columns(["CATALOG_NAME", "SCHEMA_NAME"]));
  // Cells are wire-valid strings, but deliberately too large to be identifiers.
  // Accumulated bytes must fail before finish can attempt semantic interpretation.
  rejects(() => {
    for (let i = 0; i < 40; i++) c.addRows([["DB", "x".repeat(60000)]]);
  }, "resultBytes");
  rejects(() => c.finish({ rowsSeen: 40, rowsRetained: 40, truncated: false }), "collectorClosed");
});

test("scalar preserves integer precision, explicit server Boolean string, and SQL NULL", () => {
  assert.deepEqual(parseQueryResult("scalar", scalar()), {
    integer: "9007199254740993",
    boolean: true,
    text: "joinery-qualification",
    null: null,
  });
  for (const [index, value] of [
    [0, "9007199254740992"],
    [1, "true"],
    [2, "other"],
    [3, "NULL"],
  ]) {
    const a = scalar();
    a.rows[0][index] = value;
    rejects(() => parseQueryResult("scalar", a), "scalarValue");
  }
  const a = scalar();
  a.rows[0][0] = 9007199254740993;
  rejects(() => parseQueryResult("scalar", a), "resultValue");
});

test("summary counts/truncation and query ordering fail closed", () => {
  for (const result of [
    { rowsSeen: 2, rowsRetained: 1, truncated: false },
    { rowsSeen: 1, rowsRetained: 0, truncated: false },
    { rowsSeen: 1, rowsRetained: 1, truncated: true },
    { rowsSeen: 1.5, rowsRetained: 1, truncated: false },
    { rowsSeen: "1", rowsRetained: 1, truncated: false },
  ])
    rejects(
      () => parseQueryResult("version", { ...answer(["ANY_LABEL"], [["1.0"]]), result }),
      "resultCounts",
    );
  rejects(() => createQueryCollector("version").addRows([["1.0"]]), "resultOrder");
  rejects(() => createQueryCollector("version").finish({}), "resultOrder");
  const c = createQueryCollector("version");
  c.addColumns(columns(["ANY_LABEL"]));
  rejects(() => c.addColumns(columns(["ANY_LABEL"])), "resultOrder");
  rejects(() => parseQueryResult("version", answer(["V"], [])), "resultRows");
  rejects(() => parseQueryResult("version", answer(["V"], [["1"], ["2"]])), "resultRows");
  rejects(() => parseQueryResult("version", answer(["V"], [["1", "2"]])), "resultShape");
});

test("compact content accounting is exact for escaped/wide values and shared totals", () => {
  const budget = createResultBudget();
  const a = answer(['ANY_"LABEL'], [['雪😀\n\b\t\u0001\\"']], ["TEXT"]);
  assert.equal(parseQueryResult("version", a, { budget }), a.rows[0][0]);
  assert.equal(
    budget.snapshot().bytes,
    Buffer.byteLength(JSON.stringify({ columns: a.columns, rows: a.rows, ...a.result })),
  );
  assert.equal(budget.snapshot().rows, 1);
  const small = createResultBudget({ maximumRows: 1 });
  parseQueryResult("version", answer(["V"], [["1"]]), { budget: small });
  rejects(
    () => parseQueryResult("version", answer(["V"], [["2"]]), { budget: small }),
    "resultBudgetExceeded",
  );
  assert.equal(small.snapshot().failed, true);
  const bytes = createResultBudget({ maximumBytes: 20 });
  rejects(
    () => parseQueryResult("version", answer(["V"], [["1"]]), { budget: bytes }),
    "resultBudgetExceeded",
  );
  assert(bytes.snapshot().bytes <= 20);
  rejects(
    () => createResultBudget({ maximumBytes: RESULT_LIMITS.totalBytes + 1 }),
    "invalidBudget",
  );
  rejects(() => createQueryCollector("version", { budget: {} }), "invalidBudget");
});

test("plain data-only shapes reject getters/sparse arrays and terminal IDs are checked but not returned", () => {
  let accessed = false;
  const c = createQueryCollector("version");
  rejects(
    () =>
      c.addColumns([
        {
          name: "V",
          get type() {
            accessed = true;
            return "TEXT";
          },
        },
      ]),
    "resultShape",
  );
  assert.equal(accessed, false);
  const sparse = [];
  sparse.length = 1;
  rejects(
    () => parseQueryResult("version", { ...answer(["V"], [["1"]]), rows: sparse }),
    "resultShape",
  );
  const a = answer(["V"], [["1"]]);
  a.result.sdkRequestId = "00000000-0000-0000-0000-000000000001";
  a.result.queryId = "00000000-0000-0000-0000-000000000002";
  assert.equal(parseQueryResult("version", a), "1");
  const bad = clone(a);
  bad.result.queryId = "secret-account-value";
  rejects(() => parseQueryResult("version", bad), "resultIdentity");
  const orphan = clone(a);
  delete orphan.result.sdkRequestId;
  rejects(() => parseQueryResult("version", orphan), "resultIdentity");
  const extra = clone(a);
  extra.columns[0].nullable = true;
  rejects(() => parseQueryResult("version", extra), "resultShape");
});

test("fixed24 result workflow shares bounds and admits exactly1023 successful rows at schema ceiling", () => {
  const budget = createResultBudget();
  const start = parseStartupContext(initial(), setup, { budget });
  assert.equal(
    parseQueryResult("version", answer(["CURRENT_VERSION()"], [["synthetic-version"]]), { budget }),
    "synthetic-version",
  );
  const collectSample = () => {
    const context = requireContinuity(
      parseQueryResult("context", contextAnswer(), { budget }),
      start,
    );
    const quotedIdentifierPolicy = parseQueryResult(
      "quotedIdentifierPolicy",
      answer(["key", "value"], [["QUOTED_IDENTIFIERS_IGNORE_CASE", "false"]]),
      { budget },
    );
    const autocommit = parseQueryResult(
      "autocommit",
      answer(["key", "value"], [["AUTOCOMMIT", "true"]]),
      { budget },
    );
    const abortOnError = parseQueryResult(
      "abortOnError",
      answer(["key", "value"], [["TRANSACTION_ABORT_ON_ERROR", "false"]]),
      { budget },
    );
    const transactionId = parseQueryResult("transaction", answer(["TRANSACTION_ID"], [[null]]), {
      budget,
    });
    return makeObservationSample({
      context,
      quotedIdentifierPolicy,
      autocommit,
      abortOnError,
      transactionId,
    });
  };
  const before = collectSample();
  const schemas = parseQueryResult(
    "schemas",
    schemaAnswer(Array.from({ length: 1000 }, (_, i) => ["DB", `S${i}`])),
    { budget },
  );
  assert.equal(requireSchemasInContext(schemas, before.context).length, 1000);
  assert.equal(compareSamples(before, collectSample()), "stable");
  assert.equal(parseQueryResult("scalar", scalar(), { budget }).integer, "9007199254740993");
  assert.equal(compareSamples(collectSample(), collectSample()), "stable");
  assert.equal(budget.snapshot().rows, 1023);
  assert(budget.snapshot().bytes < RESULT_LIMITS.totalBytes);
  assert.equal(budget.snapshot().failed, false);
});
