// Pure closed SQL templates. Importing this file never authenticates or executes.
export const SETUP_WORKFLOW = "disposable-setup-v1";
const fail = () => Object.assign(new Error("invalidSetupRunId"), { safeCode: "invalidSetupRunId" });

export function setupTarget(runId) {
  if (typeof runId !== "string" || !/^[A-F0-9]{32}$/.test(runId) || /^0+$/.test(runId))
    throw fail();
  return Object.freeze({
    database: `JOINERY_ACCEPTANCE_${runId}`,
    schema: "SCRATCH",
    table: "ROW_CHECKS",
    label: `joinery-disposable-${runId}`,
  });
}

export function deriveSetupLedger(runId) {
  const target = setupTarget(runId);
  const db = `"${target.database}"`;
  const sample = [
    [
      "context",
      "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES",
      8,
    ],
    [
      "quotedIdentifierPolicy",
      "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
      32,
    ],
    ["autocommit", "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION", 32],
    ["abortOnError", "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION", 32],
    ["transaction", "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID", 1],
  ];
  const ledger = [];
  function add(key, sql, phase, { mutation = false, maximumColumns = 1 } = {}) {
    ledger.push(
      Object.freeze({
        index: ledger.length + 1,
        key,
        sql,
        phase,
        mutation,
        maximumRows: 1,
        maximumBytes: 65536,
        maximumColumns,
      }),
    );
  }
  function observe(prefix, phase) {
    for (const [name, sql, maximumColumns] of sample)
      add(`${prefix}.${name}`, sql, phase, { maximumColumns });
  }
  add(
    "startup.identity",
    "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()",
    "initial",
    { maximumColumns: 6 },
  );
  add("startup.version", "SELECT CURRENT_VERSION()", "initial");
  observe("before", "initial");
  add(
    "create.database",
    `CREATE TRANSIENT DATABASE ${db} DATA_RETENTION_TIME_IN_DAYS = 0 COMMENT = '${target.label}'`,
    "initial",
    { mutation: true },
  );
  observe("database", "database");
  add(
    "create.schema",
    `CREATE SCHEMA ${db}."SCRATCH" DATA_RETENTION_TIME_IN_DAYS = 0 COMMENT = '${target.label}'`,
    "database",
    { mutation: true },
  );
  observe("schema", "schema");
  add(
    "create.table",
    `CREATE TABLE ${db}."SCRATCH"."ROW_CHECKS" ("ID" NUMBER(38,0), "NOTE" VARCHAR(128), "ACTIVE" BOOLEAN) DATA_RETENTION_TIME_IN_DAYS = 0`,
    "schema",
    { mutation: true },
  );
  add(
    "verify.database",
    `SELECT DATABASE_NAME, DATABASE_OWNER, IS_TRANSIENT, COMMENT, CREATED FROM ${db}.INFORMATION_SCHEMA.DATABASES WHERE DATABASE_NAME = '${target.database}'`,
    "schema",
    { maximumColumns: 5 },
  );
  add(
    "verify.schema",
    `SELECT CATALOG_NAME, SCHEMA_NAME, SCHEMA_OWNER, IS_TRANSIENT, COMMENT FROM ${db}.INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = '${target.database}' AND SCHEMA_NAME = 'SCRATCH'`,
    "schema",
    { maximumColumns: 5 },
  );
  add("verify.empty", `SELECT COUNT(*) AS "ROW_COUNT" FROM ${db}."SCRATCH"."ROW_CHECKS"`, "schema");
  observe("after", "schema");
  return Object.freeze(ledger);
}
