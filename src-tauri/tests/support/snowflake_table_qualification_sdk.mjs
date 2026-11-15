// Offline SDK fixture only. The production Helper owns framing, credits,
// acknowledgement publication and result collection. No account SDK is loaded.
import { Readable } from 'node:stream';

const DATABASE = 'SANDBOX';
const SCHEMA = 'BILLIE_TESTING';
const BOOTSTRAP = 'SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()';
const VERSION = 'SELECT CURRENT_VERSION()';
const CONTEXT = 'SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES';
const CURRENT_TRANSACTION = 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID';
const CONTEXT_COLUMNS = ['PRINCIPAL', 'ROLE_NAME', 'DATABASE_NAME', 'SCHEMA_NAME', 'WAREHOUSE_NAME', 'SESSION_ID', 'SEARCH_PATH', 'SECONDARY_ROLES'];
const RELATION_COLUMNS = ['TABLE_CATALOG', 'TABLE_SCHEMA', 'TABLE_NAME', 'TABLE_TYPE', 'IS_TRANSIENT', 'IS_TEMPORARY', 'IS_ICEBERG', 'IS_DYNAMIC', 'IS_HYBRID', 'IS_IMMUTABLE', 'CREATED', 'LAST_DDL'];
const COLUMN_COLUMNS = ['TABLE_CATALOG', 'TABLE_SCHEMA', 'TABLE_NAME', 'COLUMN_NAME', 'ORDINAL_POSITION', 'COLUMN_DEFAULT', 'IS_NULLABLE', 'DATA_TYPE', 'DATA_TYPE_ALIAS', 'CHARACTER_MAXIMUM_LENGTH', 'CHARACTER_OCTET_LENGTH', 'NUMERIC_PRECISION', 'NUMERIC_PRECISION_RADIX', 'NUMERIC_SCALE', 'IS_IDENTITY', 'IDENTITY_GENERATION', 'IDENTITY_START', 'IDENTITY_INCREMENT', 'IDENTITY_ORDERED', 'COMMENT', 'EXPRESSION', 'KIND'];
const STATEMENT_KEYS = ['Create', 'Add', 'Drop', 'RollbackInsert', 'CommittedInsert', 'Update', 'Delete', 'OwnershipRead', 'DataRead'];
const MUTATION_KEYS = STATEMENT_KEYS.slice(0, 7);
const CREATED = '2026-01-01 00:00:00.000 +0000';
const ALTERED = '2026-01-01 00:00:01.000 +0000';

const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const databaseError = message => Object.assign(new Error(message), { sqlState: '42501' });
const result = (names, rows, types = names.map(() => 'TEXT')) => ({ names, rows, types });
const requireState = (condition, message) => {
  if (!condition) throw databaseError(`Synthetic qualification state: ${message}`);
};

/**
 * statements: the nine exact PascalCase StatementId -> source SQL strings from
 * the internally created frozen Plan. Independently spelled fixture expectations
 * below reject altered key-to-effect mappings, targets, grammar, SQL or binds.
 * log(sql): optional synchronous observer for every SDK execute, including a
 * rejected call. The caller owns any private file logging.
 * fault: optional {kind, statement}, with kind rejectMutation, loseMutationAck,
 * or observerMismatch and statement one of the seven mutation keys.
 * {kind: 'stallRead', statement: 'DataRead'} instead stalls the first matching
 * row stream after its columns callback; cancel/destroy releases it to EOF.
 * stallReadError uses the same exact DataRead target but releases to a synthetic
 * SQLSTATE 57014 error; the production Helper still owns error classification.
 */
export function makeSdk({
  schemaCount,
  tableName,
  statements,
  log = () => {},
  principal = 'SERVER_USER',
  sessionId = 'SESSION',
  fault = null,
}) {
  if (!Number.isInteger(schemaCount) || schemaCount < 1 || schemaCount > 32)
    throw new Error('Synthetic qualification schema count must be 1..32');
  if (!/^JOINERY_QUAL_[0-9A-F]{32}$/.test(tableName) || /^JOINERY_QUAL_0{32}$/.test(tableName))
    throw new Error('Synthetic qualification requires the generated table name');
  if (!statements || Object.keys(statements).length !== STATEMENT_KEYS.length
      || STATEMENT_KEYS.some(key => !Object.hasOwn(statements, key) || typeof statements[key] !== 'string' || statements[key].length === 0))
    throw new Error('Synthetic qualification requires exactly the nine frozen statements');
  if (typeof log !== 'function' || typeof principal !== 'string' || !principal
      || typeof sessionId !== 'string' || !sessionId)
    throw new Error('Invalid synthetic qualification observers or identity');
  if (fault !== null && (!fault || !(['stallRead', 'stallReadError'].includes(fault.kind)
      ? fault.statement === 'DataRead'
      : ['rejectMutation', 'loseMutationAck', 'observerMismatch'].includes(fault.kind)
        && MUTATION_KEYS.includes(fault.statement))))
    throw new Error('Invalid synthetic qualification fault');

  const schemas = [SCHEMA, ...Array.from({ length: schemaCount - 1 }, (_, i) => `S${String(i + 2).padStart(2, '0')}`)];
  const qualified = `"${DATABASE}"."${SCHEMA}"."${tableName}"`;
  // Independent of the supplied Plan dictionary: only this generated name may
  // vary, and each key must retain its exact SQL meaning before any SDK is made.
  const expectedStatements = {
    Create: `CREATE TABLE ${qualified} ("ID" NUMBER(38, 0), "NOTE" VARCHAR(128), "ACTIVE" BOOLEAN)`,
    Add: `ALTER TABLE ${qualified} ADD COLUMN "EXTRA" VARCHAR(32)`,
    Drop: `DROP TABLE ${qualified} RESTRICT`,
    RollbackInsert: `INSERT INTO ${qualified} ("ID", "NOTE", "ACTIVE") VALUES (1, 'rollback-only', TRUE)`,
    CommittedInsert: `INSERT INTO ${qualified} ("ID", "NOTE", "ACTIVE") VALUES (2, 'committed', TRUE)`,
    Update: `UPDATE ${qualified} SET "NOTE" = 'updated', "ACTIVE" = FALSE WHERE "ID" = 2`,
    Delete: `DELETE FROM ${qualified} WHERE "ID" = 2`,
    OwnershipRead: `SELECT TABLE_OWNER, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, CREATED::VARCHAR AS CREATED FROM "SANDBOX".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'SANDBOX' AND TABLE_SCHEMA = 'BILLIE_TESTING' AND TABLE_NAME = '${tableName}' ORDER BY TABLE_NAME LIMIT 2`,
    DataRead: `SELECT "ID", "NOTE", "ACTIVE", "EXTRA" FROM ${qualified} ORDER BY "ID" LIMIT 2`,
  };
  const statementKinds = new Map();
  for (const key of STATEMENT_KEYS) {
    const source = statements[key];
    if (source !== expectedStatements[key])
      throw new Error(`Unexpected synthetic qualification statement: ${key}`);
    statementKinds.set(source, key);
  }
  const relationProjection = `${RELATION_COLUMNS.slice(0, 10).join(', ')}, CREATED::VARCHAR AS CREATED, LAST_DDL::VARCHAR AS LAST_DDL`;
  const relationPrefix = `SELECT ${relationProjection} FROM "${DATABASE}".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = '${DATABASE}' AND TABLE_SCHEMA = `;
  const relationSql = new Map(schemas.map(schema => [
    `${relationPrefix}'${schema}' ORDER BY TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE, CREATED, LAST_DDL LIMIT 10001`, schema,
  ]));
  const exactRelation = `${relationPrefix}'${SCHEMA}' AND TABLE_NAME = '${tableName}' ORDER BY TABLE_TYPE, IS_TEMPORARY LIMIT 3`;
  const schemasSql = `SELECT CATALOG_NAME, SCHEMA_NAME FROM "${DATABASE}".INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = '${DATABASE}' AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT 1001`;
  const numericMetadata = new Set(['ORDINAL_POSITION', 'CHARACTER_MAXIMUM_LENGTH', 'CHARACTER_OCTET_LENGTH', 'NUMERIC_PRECISION', 'NUMERIC_PRECISION_RADIX', 'NUMERIC_SCALE']);
  const projection = COLUMN_COLUMNS.map(name => numericMetadata.has(name) ? `${name}::VARCHAR AS ${name}` : name).join(', ');
  const columnsSql = `SELECT ${projection} FROM "${DATABASE}".INFORMATION_SCHEMA.COLUMNS AS C WHERE TABLE_CATALOG = '${DATABASE}' AND TABLE_SCHEMA = '${SCHEMA}' AND TABLE_NAME = '${tableName}' ORDER BY C.ORDINAL_POSITION LIMIT 1001`;
  const describeSql = `DESCRIBE TABLE ${qualified} TYPE = COLUMNS`;
  const resolvedSql = `SELECT * FROM ${qualified} LIMIT 0`;
  const definitions = [
    { name: 'ID', type: 'NUMBER', full: 'NUMBER(38,0)', wire: 'FIXED', precision: '38', scale: '0' },
    { name: 'NOTE', type: 'TEXT', full: 'VARCHAR(128)', wire: 'TEXT', length: '128', octets: '512' },
    { name: 'ACTIVE', type: 'BOOLEAN', full: 'BOOLEAN', wire: 'BOOLEAN' },
    { name: 'EXTRA', type: 'TEXT', full: 'VARCHAR(32)', wire: 'TEXT', length: '32', octets: '128' },
  ];
  let connectionCreated = false;
  let sequence = 0;

  return {
    configure() {},
    createConnection(profile) {
      if (connectionCreated) throw new Error('Synthetic qualification cannot reconnect');
      if (!profile || profile.database !== DATABASE || profile.schema !== SCHEMA
          || !['EXTERNALBROWSER', 'externalbrowser'].includes(profile.authenticator)
          || typeof profile.role !== 'string' || !profile.role
          || typeof profile.warehouse !== 'string' || !profile.warehouse)
        throw new Error('Unexpected synthetic qualification connection');
      connectionCreated = true;
      let connected = false;
      let destroyed = false;
      let exists = false;
      let added = false;
      let createdOnce = false;
      let dropped = false;
      let committed = [];
      let working = null;
      let transactionId = null;
      let transactionCount = 0;
      let faultTriggered = false;
      let mismatched = false;
      const stalledReads = new Set();
      const currentRows = () => working ?? committed;
      const shape = () => definitions.slice(0, added ? 4 : 3);
      const relation = () => exists ? [[DATABASE, SCHEMA, tableName, 'BASE TABLE', 'NO', 'NO', 'NO', 'NO', 'NO', 'NO', CREATED, added ? ALTERED : CREATED]] : [];
      const status = () => result(['status'], [['Statement executed successfully.']]);
      const affected = (verb, count) => result([`number of rows ${verb}`], [[String(count)]], ['FIXED']);

      function executeSql(sql) {
        if (sql === BOOTSTRAP)
          return result(['U', 'R', 'D', 'S', 'W', 'ID'], [[principal, profile.role, DATABASE, SCHEMA, profile.warehouse, sessionId]]);
        if (sql === VERSION) return result(['VERSION'], [['synthetic-qualification-version']]);
        if (sql === CONTEXT)
          return result(CONTEXT_COLUMNS, [[principal, mismatched ? `${profile.role}_MISMATCH` : profile.role, DATABASE, SCHEMA, profile.warehouse, sessionId, JSON.stringify([`${DATABASE}.${SCHEMA}`]), JSON.stringify({ roles: '', value: 'NONE' })]]);
        for (const key of ['QUOTED_IDENTIFIERS_IGNORE_CASE', 'AUTOCOMMIT', 'TRANSACTION_ABORT_ON_ERROR']) {
          if (sql === `SHOW PARAMETERS LIKE '${key}' IN SESSION`) {
            const value = key === 'AUTOCOMMIT' ? 'true' : 'false';
            return result(['key', 'value', 'default', 'level'], [[key, value, value, 'SESSION']]);
          }
        }
        if (sql === CURRENT_TRANSACTION) return result(['TRANSACTION_ID'], [[transactionId]]);
        if (sql === schemasSql) return result(['CATALOG_NAME', 'SCHEMA_NAME'], schemas.map(schema => [DATABASE, schema]));
        if (relationSql.has(sql))
          return result(RELATION_COLUMNS, relationSql.get(sql) === SCHEMA ? relation() : []);
        if (sql === exactRelation) return result(RELATION_COLUMNS, relation());
        if (sql === columnsSql) {
          requireState(exists, 'columns of missing table');
          return result(COLUMN_COLUMNS, shape().map((column, index) => [
            DATABASE, SCHEMA, tableName, column.name, String(index + 1), null, 'YES', column.type, null,
            column.length ?? null, column.octets ?? null, column.precision ?? null,
            column.precision ? '10' : null, column.scale ?? null, 'NO', null, null, null, null, null, null, 'COLUMN',
          ]));
        }
        if (sql === describeSql) {
          requireState(exists, 'describe of missing table');
          return result(['name', 'type', 'kind', 'null?', 'default', 'expression', 'comment'], shape().map(column => [column.name, column.full, 'COLUMN', 'Y', null, null, null]));
        }
        if (sql === resolvedSql) {
          requireState(exists, 'resolution of missing table');
          return result(shape().map(column => column.name), [], shape().map(column => column.wire));
        }
        if (sql === 'BEGIN TRANSACTION') {
          requireState(exists && added && transactionId === null && transactionCount < 2, 'unexpected BEGIN');
          transactionId = `opaque-fixture-transaction-${++transactionCount === 1 ? 'A' : 'B'}`;
          working = committed.map(row => [...row]);
          return status();
        }
        if (sql === 'ROLLBACK') {
          requireState(transactionId !== null && transactionCount === 1, 'unexpected ROLLBACK');
          working = null;
          transactionId = null;
          return status();
        }
        if (sql === 'COMMIT') {
          requireState(transactionId !== null && transactionCount === 2, 'unexpected COMMIT');
          committed = working;
          working = null;
          transactionId = null;
          return status();
        }
        const kind = statementKinds.get(sql);
        if (kind === undefined) throw new Error('Unrecognized synthetic qualification SQL');
        switch (kind) {
          case 'Create':
            requireState(!createdOnce && !exists && !dropped && transactionId === null, 'unexpected CREATE');
            createdOnce = true;
            exists = true;
            return status();
          case 'Add':
            requireState(exists && !added && transactionId === null, 'unexpected ADD');
            added = true;
            return status();
          case 'Drop':
            requireState(exists && added && transactionId === null && committed.length === 0, 'unexpected DROP');
            exists = false;
            dropped = true;
            return status();
          case 'OwnershipRead':
            requireState(exists, 'ownership of missing table');
            return result(['TABLE_OWNER', 'TABLE_TYPE', 'IS_TRANSIENT', 'IS_TEMPORARY', 'CREATED'], [[profile.role, 'BASE TABLE', 'NO', 'NO', CREATED]]);
          case 'DataRead':
            requireState(exists && added, 'data read before four-column table');
            return result(definitions.map(column => column.name), currentRows().map(row => [...row]), definitions.map(column => column.wire));
          case 'RollbackInsert':
            requireState(exists && added && transactionId !== null && transactionCount === 1 && working.length === 0, 'unexpected rollback INSERT');
            working.push(['1', 'rollback-only', 'true', null]);
            return affected('inserted', 1);
          case 'CommittedInsert':
            requireState(exists && added && transactionId !== null && transactionCount === 2 && working.length === 0, 'unexpected committed INSERT');
            working.push(['2', 'committed', 'true', null]);
            return affected('inserted', 1);
          case 'Update':
            requireState(exists && added && transactionId === null && committed.length === 1 && committed[0][0] === '2' && committed[0][1] === 'committed', 'unexpected UPDATE');
            committed[0][1] = 'updated';
            committed[0][2] = 'false';
            return affected('updated', 1);
          case 'Delete':
            requireState(exists && added && transactionId === null && committed.length === 1 && committed[0][0] === '2' && committed[0][1] === 'updated', 'unexpected DELETE');
            committed = [];
            return affected('deleted', 1);
          default:
            throw new Error('Unrecognized synthetic qualification statement kind');
        }
      }

      return {
        connectAsync(callback) {
          if (connected || destroyed) throw new Error('Synthetic qualification connection already used');
          connected = true;
          queueMicrotask(() => callback(null));
          return Promise.resolve();
        },
        destroy(callback) {
          destroyed = true;
          connected = false;
          for (const release of stalledReads) release();
          queueMicrotask(() => callback(null));
        },
        execute(options) {
          if (!connected || destroyed) throw new Error('Synthetic qualification connection is closed');
          const sql = options.sqlText;
          log(sql);
          if (typeof sql !== 'string' || !Array.isArray(options.binds) || options.binds.length !== 0)
            throw new Error('Unrecognized synthetic qualification SQL or binds');
          const id = ++sequence;
          const kind = statementKinds.get(sql);
          const trigger = fault !== null && !faultTriggered && kind === fault.statement;
          let data = result([], []);
          let failure = null;
          if (trigger && fault.kind === 'rejectMutation') {
            faultTriggered = true;
            failure = databaseError('Synthetic qualification mutation rejected');
          } else {
            data = executeSql(sql);
            if (trigger) {
              faultTriggered = true;
              if (fault.kind === 'loseMutationAck')
                failure = new Error('Synthetic mutation applied without an acknowledged SDK callback');
              else if (fault.kind === 'observerMismatch') mismatched = true;
            }
          }
          let cancelledRead = false;
          let releaseRead = () => {};
          const stalledRead = trigger && ['stallRead', 'stallReadError'].includes(fault.kind)
            ? new Promise(resolve => {
              releaseRead = () => {
                cancelledRead = true;
                stalledReads.delete(releaseRead);
                resolve();
              };
              stalledReads.add(releaseRead);
            })
            : null;
          const statement = {
            getRequestId: () => uuid(id),
            getQueryId: () => uuid(id + 1000),
            getColumns: () => data.names.map((name, index) => ({ getName: () => name, getType: () => data.types[index] })),
            streamRows: () => stalledRead === null
              ? Readable.from(data.rows.map(row => [...row]))
              : Readable.from((async function* () {
                await stalledRead;
                if (fault.kind === 'stallReadError')
                  throw Object.assign(new Error('Synthetic qualification stalled read interrupted'), { sqlState: '57014' });
                if (!cancelledRead) yield* data.rows.map(row => [...row]);
              })()),
            cancel(callback) {
              releaseRead();
              queueMicrotask(() => callback(null));
            },
          };
          setImmediate(() => options.complete(failure, statement));
          return statement;
        },
      };
    },
  };
}
