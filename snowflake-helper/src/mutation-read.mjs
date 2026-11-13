import { sameMutationContext } from "./mutation-batch.mjs";

// An owner-scoped metadata lane, not permission to submit SQL. Native callers
// bracket these single-statement reads and validate complete results before
// treating an inspection as fresh. Reads never establish transaction disposition.
export const MUTATION_READ_CONTEXT_SQL =
  "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES";
export const MUTATION_READ_POLICY_SQL =
  "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION";
export const MUTATION_READ_TRANSACTION_SQL =
  "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID";
const CONTEXT_KEYS = ["user", "role", "database", "schema", "warehouse", "session"];
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const exact = (p, keys) =>
  Boolean(p) &&
  typeof p === "object" &&
  !Array.isArray(p) &&
  Object.keys(p).length === keys.length &&
  Object.keys(p).every((k) => keys.includes(k));
const text = (s, n) =>
  typeof s === "string" &&
  s.length > 0 &&
  Buffer.byteLength(s) <= n &&
  s.isWellFormed() &&
  !s.includes("\0");
const identifier = (s) => text(s, 1020) && [...s].length <= 255;
const expectedValid = (p) =>
  exact(p, CONTEXT_KEYS) &&
  identifier(p.user) &&
  text(p.session, 1024) &&
  ["role", "database", "schema", "warehouse"].every((k) => p[k] === null || identifier(p[k]));
const quote = (s) => `"${s.replaceAll('"', '""')}"`;
const literal = (s) => `'${s.replaceAll("\\", "\\\\").replaceAll("'", "''")}'`;
const BASIC = new Set(["context", "quotedIdentifierPolicy", "currentTransaction"]);
const TARGETED = new Set(["exactRelation", "columns", "describe", "zeroRowResolution"]);
const RELATION_COLUMNS = [
  "TABLE_CATALOG",
  "TABLE_SCHEMA",
  "TABLE_NAME",
  "TABLE_TYPE",
  "IS_TRANSIENT",
  "IS_TEMPORARY",
  "IS_ICEBERG",
  "IS_DYNAMIC",
  "IS_HYBRID",
  "IS_IMMUTABLE",
];
const COLUMN_COLUMNS = [
  "TABLE_CATALOG",
  "TABLE_SCHEMA",
  "TABLE_NAME",
  "COLUMN_NAME",
  "ORDINAL_POSITION",
  "COLUMN_DEFAULT",
  "IS_NULLABLE",
  "DATA_TYPE",
  "DATA_TYPE_ALIAS",
  "CHARACTER_MAXIMUM_LENGTH",
  "CHARACTER_OCTET_LENGTH",
  "NUMERIC_PRECISION",
  "NUMERIC_PRECISION_RADIX",
  "NUMERIC_SCALE",
  "IS_IDENTITY",
  "IDENTITY_GENERATION",
  "IDENTITY_START",
  "IDENTITY_INCREMENT",
  "IDENTITY_ORDERED",
  "COMMENT",
  "EXPRESSION",
  "KIND",
];
const TEXT_DIMENSIONS = new Set([
  "ORDINAL_POSITION",
  "CHARACTER_MAXIMUM_LENGTH",
  "CHARACTER_OCTET_LENGTH",
  "NUMERIC_PRECISION",
  "NUMERIC_PRECISION_RADIX",
  "NUMERIC_SCALE",
]);

export function validateMutationReadRequest(r, helper) {
  try {
    const p = r?.payload;
    if (
      r?.op !== "mutationRead" ||
      !exact(p, ["owner", "expected", "kind", "target", "credits"]) ||
      typeof p.owner !== "string" ||
      !UUID.test(p.owner) ||
      !expectedValid(p.expected) ||
      !Number.isInteger(p.credits) ||
      p.credits < 1 ||
      p.credits > 32 ||
      Buffer.byteLength(JSON.stringify(r)) > 1048576 ||
      !helper.identityPin ||
      helper.identityPin.user !== p.expected.user ||
      helper.identityPin.session !== p.expected.session
    )
      return false;
    if (BASIC.has(p.kind)) return p.target === null;
    const t = p.target;
    return (
      TARGETED.has(p.kind) &&
      exact(t, ["database", "schema", "name", "tableType"]) &&
      [t.database, t.schema, t.name].every(identifier) &&
      t.database === p.expected.database &&
      ["BASE TABLE", "TEMPORARY TABLE"].includes(t.tableType)
    );
  } catch {
    return false;
  }
}

// Passing the captured object at the final SDK boundary also rejects replacing
// a private owner with a new object that happens to carry the same token.
export function mutationReadOwnerAdmits(helper, r, owner = helper.mutationOwner) {
  return Boolean(
    owner &&
    owner === helper.mutationOwner &&
    validateMutationReadRequest(r, helper) &&
    r.payload.owner === owner.token &&
    sameMutationContext(r.payload.expected, owner.expected),
  );
}

export function mutationReadSpec(r) {
  // This factory remains closed even if called independently of queue admission.
  if (!validateMutationReadRequest(r, { identityPin: r?.payload?.expected }))
    throw { code: "invalidInput" };
  const { kind, target: t } = r.payload;
  const spec = (sql, maxRows, maxBytes) =>
    Object.freeze({ sql, binds: Object.freeze([]), maxRows, maxBytes, maxColumns: 1000 });
  if (kind === "context") return spec(MUTATION_READ_CONTEXT_SQL, 1, 65536);
  if (kind === "quotedIdentifierPolicy") return spec(MUTATION_READ_POLICY_SQL, 1, 65536);
  if (kind === "currentTransaction") return spec(MUTATION_READ_TRANSACTION_SQL, 1, 65536);
  const name = [t.database, t.schema, t.name].map(quote).join(".");
  const filter = `TABLE_CATALOG = ${literal(t.database)} AND TABLE_SCHEMA = ${literal(t.schema)} AND TABLE_NAME = ${literal(t.name)}`;
  if (kind === "exactRelation")
    return spec(
      `SELECT ${RELATION_COLUMNS.join(", ")}, CREATED::VARCHAR AS CREATED, LAST_DDL::VARCHAR AS LAST_DDL FROM ${quote(t.database)}.INFORMATION_SCHEMA.TABLES WHERE ${filter} ORDER BY TABLE_TYPE, IS_TEMPORARY LIMIT 3`,
      3,
      2097152,
    );
  if (kind === "columns") {
    const projection = COLUMN_COLUMNS.map((column) =>
      TEXT_DIMENSIONS.has(column) ? `${column}::VARCHAR AS ${column}` : column,
    ).join(", ");
    return spec(
      `SELECT ${projection} FROM ${quote(t.database)}.INFORMATION_SCHEMA.COLUMNS AS C WHERE ${filter} ORDER BY C.ORDINAL_POSITION LIMIT 1001`,
      1001,
      1048576,
    );
  }
  if (kind === "describe") return spec(`DESCRIBE TABLE ${name} TYPE = COLUMNS`, 1001, 1048576);
  return spec(`SELECT * FROM ${name} LIMIT 0`, 0, 1048576);
}

export async function runMutationRead(helper, active) {
  const r = active.request;
  if (!mutationReadOwnerAdmits(helper, r, active.mutationReadOwner ?? helper.mutationOwner)) {
    helper.finish(r, "invalidInput");
    return;
  }
  active.mutationReadOwner ??= helper.mutationOwner;
  active.mutationRead = mutationReadSpec(r);
  helper.setupBootstrap = 0;
  // Ordinary guard capacity only. The pending owner's finish reservation and
  // immutable batch evidence are neither borrowed nor changed by metadata reads.
  const result = await helper.execute(active, active.mutationRead.sql, [], true);
  if (helper.retired || helper.done.has(r)) return;
  if (!mutationReadOwnerAdmits(helper, r, active.mutationReadOwner))
    throw { code: "outcomeUnknown", retire: true };
  helper.finish(r, null, { ...result, ...helper.idsFor(active) });
}
