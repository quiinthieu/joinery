// Pure validators for the fixed qualification ledger. No transport or approval
// authority lives here. Returned values are private observations, not log data.
export const RESULT_LIMITS = Object.freeze({
  totalBytes: 4 * 1024 * 1024,
  totalRows: 1024,
  smallBytes: 65536,
  schemaBytes: 2 * 1024 * 1024,
  schemaNames: 1000,
  batchRows: 128,
  batchBytes: 262144,
});
const CONTEXT_NAMES = Object.freeze([
  "PRINCIPAL",
  "ROLE_NAME",
  "DATABASE_NAME",
  "SCHEMA_NAME",
  "WAREHOUSE_NAME",
  "SESSION_ID",
  "SEARCH_PATH",
  "SECONDARY_ROLES",
]);
const PARAMETERS = Object.freeze({
  quotedIdentifierPolicy: "QUOTED_IDENTIFIERS_IGNORE_CASE",
  autocommit: "AUTOCOMMIT",
  abortOnError: "TRANSACTION_ABORT_ON_ERROR",
});
const SDK_ID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const budgets = new WeakMap();
const contexts = new WeakSet();
const schemaResults = new WeakSet();
const samples = new WeakSet();
const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
const asciiLower = (value) => value.replace(/[A-Z]/g, (character) => character.toLowerCase());

function record(value, required, optional = []) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    ![Object.prototype, null].includes(Object.getPrototypeOf(value))
  )
    throw fail("resultShape");
  const keys = Reflect.ownKeys(value);
  if (
    keys.length > required.length + optional.length ||
    required.some((key) => !Object.hasOwn(value, key)) ||
    keys.some((key) => {
      const d = Object.getOwnPropertyDescriptor(value, key);
      return (
        typeof key !== "string" ||
        ![...required, ...optional].includes(key) ||
        !d.enumerable ||
        !Object.hasOwn(d, "value")
      );
    })
  )
    throw fail("resultShape");
  return value;
}
function array(value, maximum) {
  if (
    !Array.isArray(value) ||
    Object.getPrototypeOf(value) !== Array.prototype ||
    value.length > maximum ||
    Reflect.ownKeys(value).length !== value.length + 1
  )
    throw fail("resultShape");
  for (let i = 0; i < value.length; i++) {
    const d = Object.getOwnPropertyDescriptor(value, String(i));
    if (!d?.enumerable || !Object.hasOwn(d, "value")) throw fail("resultShape");
  }
  return value;
}
function text(value, maximum, empty = false) {
  if (
    typeof value !== "string" ||
    (!empty && value.length === 0) ||
    value.length > maximum ||
    !value.isWellFormed() ||
    value.includes("\0")
  )
    throw fail("resultValue");
  if (Buffer.byteLength(value) > maximum) throw fail("resultBytes");
  return value;
}
function identifier(value) {
  text(value, 1020);
  if ([...value].length > 255) throw fail("resultValue");
  return value;
}
const nullableIdentifier = (value) => (value === null ? null : identifier(value));
function freeze(value) {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
function identities(value) {
  for (const key of ["sdkRequestId", "queryId"])
    if (Object.hasOwn(value, key) && (typeof value[key] !== "string" || !SDK_ID.test(value[key])))
      throw fail("resultIdentity");
  if (Object.hasOwn(value, "queryId") && !Object.hasOwn(value, "sdkRequestId"))
    throw fail("resultIdentity");
}

// Smaller limits are a pure unit-test seam; no caller can enlarge fixed bounds.
export function createResultBudget({
  maximumBytes = RESULT_LIMITS.totalBytes,
  maximumRows = RESULT_LIMITS.totalRows,
} = {}) {
  if (
    !Number.isSafeInteger(maximumBytes) ||
    maximumBytes < 1 ||
    maximumBytes > RESULT_LIMITS.totalBytes ||
    !Number.isSafeInteger(maximumRows) ||
    maximumRows < 1 ||
    maximumRows > RESULT_LIMITS.totalRows
  )
    throw fail("invalidBudget");
  const state = { bytes: 0, rows: 0, maximumBytes, maximumRows, failed: false };
  const token = Object.freeze({
    snapshot: () =>
      Object.freeze({
        bytes: state.bytes,
        rows: state.rows,
        failed: state.failed,
      }),
  });
  budgets.set(token, state);
  return token;
}
function reserve(budget, bytes = 0, rows = 0) {
  const state = budgets.get(budget);
  if (!state) throw fail("invalidBudget");
  if (
    state.failed ||
    bytes > state.maximumBytes - state.bytes ||
    rows > state.maximumRows - state.rows
  ) {
    state.failed = true;
    throw fail("resultBudgetExceeded");
  }
  state.bytes += bytes;
  state.rows += rows;
}

function spec(templateId) {
  if (typeof templateId !== "string") throw fail("unknownTemplate");
  if (Object.hasOwn(PARAMETERS, templateId)) return { maximumColumns: 32 };
  switch (templateId) {
    case "context":
      return { names: CONTEXT_NAMES, types: CONTEXT_NAMES.map(() => "text") };
    case "version":
      return { types: ["text"] };
    case "transaction":
      return { names: ["TRANSACTION_ID"], types: ["text"] };
    case "schemas":
      return { names: ["CATALOG_NAME", "SCHEMA_NAME"], types: ["text", "text"] };
    case "scalar":
      return {
        names: ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"],
        types: ["fixed", "boolean", "text", "text"],
      };
    default:
      throw fail("unknownTemplate");
  }
}

function jsonTextSize(value) {
  // Count escaped UTF-8 without allocating a possibly expanded JSON string.
  let size = 2;
  for (const character of value) {
    const point = character.codePointAt(0);
    size +=
      point < 32
        ? [8, 9, 10, 12, 13].includes(point)
          ? 2
          : 6
        : point === 34 || point === 92
          ? 2
          : point < 128
            ? 1
            : point < 2048
              ? 2
              : point < 65536
                ? 3
                : 4;
    if (size > RESULT_LIMITS.batchBytes) throw fail("resultBytes");
  }
  return size;
}

/** Incremental content representation is compact JSON of
 * {columns,rows,rowsSeen,rowsRetained,truncated}; identities are protocol-owned.
 * All descriptors (including uninterpreted SHOW columns), cells and summary
 * content count once. Frame envelopes/control traffic need separate wire caps.
 */
export function createQueryCollector(templateId, { budget = createResultBudget() } = {}) {
  const expected = spec(templateId);
  const maximumBytes =
    templateId === "schemas" ? RESULT_LIMITS.schemaBytes : RESULT_LIMITS.smallBytes;
  let columns,
    rows = [],
    bytes = 0,
    closed = false;
  function charge(size) {
    if (size > maximumBytes - bytes) throw fail("resultBytes");
    reserve(budget, size);
    bytes += size;
  }
  function guarded(action) {
    if (closed) throw fail("collectorClosed");
    try {
      return action();
    } catch (error) {
      closed = true;
      throw error;
    }
  }
  charge(Buffer.byteLength('{"columns":['));
  return Object.freeze({
    addColumns(value) {
      return guarded(() => {
        if (columns) throw fail("resultOrder");
        array(value, expected.maximumColumns ?? expected.types.length);
        if (!value.length || (expected.types && value.length !== expected.types.length))
          throw fail("resultColumns");
        columns = [];
        const names = new Set();
        for (const [index, descriptor] of value.entries()) {
          record(descriptor, ["name", "type"]);
          const name = text(descriptor.name, 4096),
            type = text(descriptor.type, 128);
          // Only ASCII case varies; do not alias VARCHAR/NUMBER to wire types.
          const normalizedType = asciiLower(type);
          const folded = asciiLower(name);
          if (names.has(folded)) throw fail("resultColumns");
          names.add(folded);
          if (expected.names && name !== expected.names[index]) throw fail("resultColumns");
          if (expected.types && normalizedType !== expected.types[index]) throw fail("resultTypes");
          const copy = { name, type: normalizedType };
          // Charge the actual descriptor spelling, before storing its normalized copy.
          charge(Buffer.byteLength(JSON.stringify({ name, type })) + (index ? 1 : 0));
          columns.push(copy);
        }
        charge(Buffer.byteLength('],"rows":['));
      });
    },
    addRows(batch) {
      return guarded(() => {
        if (!columns) throw fail("resultOrder");
        array(batch, RESULT_LIMITS.batchRows);
        if (!batch.length) throw fail("resultShape");
        let batchBytes = 2;
        for (const row of batch) {
          array(row, columns.length);
          if (row.length !== columns.length) throw fail("resultShape");
          reserve(budget, 0, 1);
          if (templateId === "schemas" && rows.length === RESULT_LIMITS.schemaNames)
            throw fail("schemaSentinel");
          if (templateId !== "schemas" && rows.length === 1) throw fail("resultRows");
          // Bounded string checks precede JSON encoding; never encode an unbounded row.
          const copy = [];
          let rowBytes = 2;
          for (const cell of row) {
            if (cell !== null) text(cell, Math.min(maximumBytes, RESULT_LIMITS.batchBytes), true);
            const size = cell === null ? 4 : jsonTextSize(cell);
            rowBytes += size + (copy.length ? 1 : 0);
            if (
              rowBytes > RESULT_LIMITS.batchBytes ||
              batchBytes + rowBytes > RESULT_LIMITS.batchBytes
            )
              throw fail("resultBytes");
            charge(size + (copy.length ? 1 : 0));
            copy.push(cell);
          }
          batchBytes += rowBytes + (batchBytes > 2 ? 1 : 0);
          if (batchBytes > RESULT_LIMITS.batchBytes) throw fail("resultBytes");
          charge(2 + (rows.length ? 1 : 0));
          rows.push(copy);
        }
      });
    },
    finish(summary) {
      return guarded(() => {
        if (!columns) throw fail("resultOrder");
        record(summary, ["rowsSeen", "rowsRetained", "truncated"], ["sdkRequestId", "queryId"]);
        identities(summary);
        if (
          !Number.isSafeInteger(summary.rowsSeen) ||
          summary.rowsSeen < 0 ||
          !Number.isSafeInteger(summary.rowsRetained) ||
          summary.rowsRetained < 0 ||
          summary.truncated !== false ||
          summary.rowsSeen !== rows.length ||
          summary.rowsRetained !== rows.length
        )
          throw fail("resultCounts");
        if (templateId !== "schemas" && rows.length !== 1) throw fail("resultRows");
        charge(
          Buffer.byteLength(
            `],"rowsSeen":${summary.rowsSeen},"rowsRetained":${summary.rowsRetained},"truncated":false}`,
          ),
        );
        const result = typedResult(templateId, columns, rows);
        closed = true;
        return result;
      });
    },
  });
}

// Convenience for bounded synthetic/previously collected answers. A transport
// should feed addRows incrementally rather than collect an unbounded answer.
export function parseQueryResult(templateId, answer, options) {
  record(answer, ["columns", "rows", "result"]);
  const collector = createQueryCollector(templateId, options);
  collector.addColumns(answer.columns);
  array(answer.rows, templateId === "schemas" ? 1001 : 2);
  for (let start = 0; start < answer.rows.length; start += RESULT_LIMITS.batchRows)
    collector.addRows(answer.rows.slice(start, start + RESULT_LIMITS.batchRows));
  return collector.finish(answer.result);
}

function json(value) {
  try {
    return JSON.parse(value);
  } catch {
    throw fail("resultJson");
  }
}
function requireUniqueStringMembers(raw, expected) {
  // JSON.parse alone silently overwrites duplicate keys. These objects are
  // already validated as string-only records; tokenizing their string members
  // also catches repeated keys with escaped spellings (e.g. \u0072oles).
  const seen = new Set();
  let count = 0;
  for (const match of raw.matchAll(
    /"(?:[^"\\\u0000-\u001f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"/g,
  )) {
    if (count >= expected.length * 2) throw fail("resultShape");
    if (count % 2 === 0) {
      const key = json(match[0]);
      if (!expected.includes(key) || seen.has(key)) throw fail("resultShape");
      seen.add(key);
    }
    count++;
  }
  if (count !== expected.length * 2) throw fail("resultShape");
}
function resolvedPath(raw) {
  // Preserve unresolved spelling, never guess case/qualification or split dots
  // inside quoted components. Exactly two validated components can resolve.
  const component = (start) => {
    if (raw[start] === '"') {
      let value = "",
        index = start + 1;
      while (index < raw.length) {
        if (raw[index] !== '"') {
          value += raw[index++];
          continue;
        }
        if (raw[index + 1] === '"') {
          value += '"';
          index += 2;
          continue;
        }
        try {
          identifier(value);
        } catch {
          return null;
        }
        return [value, index + 1];
      }
      return null;
    }
    const end = raw.indexOf(".", start),
      index = end < 0 ? raw.length : end;
    const value = raw.slice(start, index);
    return /^[A-Z_][A-Z0-9_$]{0,254}$/.test(value) ? [value, index] : null;
  };
  const first = component(0);
  const second = first && raw[first[1]] === "." ? component(first[1] + 1) : null;
  return second && second[1] === raw.length
    ? { raw, database: first[0], schema: second[0] }
    : { raw, database: null, schema: null };
}
function typedResult(templateId, columns, rows) {
  const row = rows[0];
  if (Object.hasOwn(PARAMETERS, templateId)) {
    const find = (name) => columns.findIndex((column) => asciiLower(column.name) === name);
    const key = find("key"),
      value = find("value");
    if (key < 0 || value < 0 || columns[key].type !== "text" || columns[value].type !== "text")
      throw fail("resultTypes");
    if (row[key] !== PARAMETERS[templateId]) throw fail("parameterKey");
    if (typeof row[value] !== "string" || !/^(true|false)$/i.test(row[value]))
      throw fail("parameterValue");
    const parsed = row[value].toLowerCase() === "true";
    if (templateId === "quotedIdentifierPolicy" && parsed) throw fail("unsupportedPolicy");
    return parsed;
  }
  if (templateId === "context") {
    const path = array(json(text(row[6], RESULT_LIMITS.smallBytes)), 64).map((entry) =>
      resolvedPath(text(entry, 1024)),
    );
    const secondary = record(json(text(row[7], RESULT_LIMITS.smallBytes)), ["roles", "value"]);
    const secondaryRoles = {
      roles: text(secondary.roles, 16384, true),
      value: text(secondary.value, 16384, true),
    };
    requireUniqueStringMembers(row[7], ["roles", "value"]);
    const context = freeze({
      principal: identifier(row[0]),
      role: nullableIdentifier(row[1]),
      database: nullableIdentifier(row[2]),
      schema: nullableIdentifier(row[3]),
      warehouse: nullableIdentifier(row[4]),
      sessionId: text(row[5], 1024),
      searchPath: path,
      secondaryRoles,
    });
    contexts.add(context);
    return context;
  }
  if (templateId === "version") return text(row[0], RESULT_LIMITS.smallBytes);
  if (templateId === "transaction") return row[0] === null ? null : text(row[0], 1024);
  if (templateId === "scalar") {
    if (
      row[0] !== "9007199254740993" ||
      row[1] !== "TRUE" ||
      row[2] !== "joinery-qualification" ||
      row[3] !== null
    )
      throw fail("scalarValue");
    return freeze({ integer: row[0], boolean: true, text: row[2], null: null });
  }
  const seen = new Set();
  const result = rows.map(([database, schema]) => {
    identifier(database);
    identifier(schema);
    if (schema === "INFORMATION_SCHEMA" || seen.has(schema)) throw fail("schemaValue");
    seen.add(schema);
    return { database, schema };
  });
  freeze(result);
  schemaResults.add(result);
  return result;
}

export function parseStartupContext(result, expectedSetup, { budget = createResultBudget() } = {}) {
  record(
    result,
    ["user", "role", "database", "schema", "warehouse", "session"],
    ["sdkRequestId", "queryId"],
  );
  record(expectedSetup, ["role", "database", "schema", "warehouse"]);
  identities(result);
  const parsed = {
    principal: identifier(result.user),
    role: nullableIdentifier(result.role),
    database: nullableIdentifier(result.database),
    schema: nullableIdentifier(result.schema),
    warehouse: nullableIdentifier(result.warehouse),
    sessionId: text(result.session, 1024),
  };
  // The terminal-only representation counts all six context fields, once.
  const content = {
    user: parsed.principal,
    role: parsed.role,
    database: parsed.database,
    schema: parsed.schema,
    warehouse: parsed.warehouse,
    session: parsed.sessionId,
  };
  const size = Buffer.byteLength(JSON.stringify(content));
  if (size > RESULT_LIMITS.smallBytes) throw fail("resultBytes");
  reserve(budget, size, 1);
  for (const key of ["role", "database", "schema", "warehouse"])
    if (parsed[key] !== identifier(expectedSetup[key])) throw fail("startupContextMismatch");
  return freeze(parsed);
}

export function requireContinuity(context, initial) {
  if (!contexts.has(context)) throw fail("invalidObservation");
  record(initial, ["principal", "role", "database", "schema", "warehouse", "sessionId"]);
  if (
    context.principal !== identifier(initial.principal) ||
    context.sessionId !== text(initial.sessionId, 1024) ||
    context.database !== identifier(initial.database)
  )
    throw fail("contextIdentityMismatch");
  return context;
}
export function requireSchemasInContext(schemas, context) {
  if (!schemaResults.has(schemas) || !contexts.has(context)) throw fail("invalidObservation");
  const database = identifier(context.database);
  if (schemas.some((entry) => entry.database !== database)) throw fail("schemaContextMismatch");
  return freeze(schemas.map((entry) => entry.schema).sort());
}
export function makeObservationSample(value) {
  record(value, [
    "context",
    "quotedIdentifierPolicy",
    "autocommit",
    "abortOnError",
    "transactionId",
  ]);
  if (
    !contexts.has(value.context) ||
    value.quotedIdentifierPolicy !== false ||
    typeof value.autocommit !== "boolean" ||
    typeof value.abortOnError !== "boolean"
  )
    throw fail("invalidObservation");
  const sample = freeze({
    context: value.context,
    quotedIdentifierPolicy: false,
    autocommit: value.autocommit,
    abortOnError: value.abortOnError,
    transactionId: value.transactionId === null ? null : text(value.transactionId, 1024),
  });
  samples.add(sample);
  return sample;
}
export function compareSamples(before, after) {
  if ((before !== null && !samples.has(before)) || (after !== null && !samples.has(after)))
    throw fail("invalidObservation");
  if (before === null || after === null) return "unavailable";
  return JSON.stringify(before) === JSON.stringify(after) ? "stable" : "changed";
}
