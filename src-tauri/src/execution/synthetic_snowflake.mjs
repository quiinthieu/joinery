// Synthetic SDK for Rust application lifecycle tests. Never packaged.
import { Readable } from 'node:stream';
import { appendFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
const controls = new URL('../../', import.meta.url);
const marker = name => new URL(name, controls);
let sequence = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
const contextColumns = ['PRINCIPAL', 'ROLE_NAME', 'DATABASE_NAME', 'SCHEMA_NAME', 'WAREHOUSE_NAME', 'SESSION_ID', 'SEARCH_PATH', 'SECONDARY_ROLES'];
const relationColumns = ['TABLE_CATALOG', 'TABLE_SCHEMA', 'TABLE_NAME', 'TABLE_TYPE', 'IS_TRANSIENT', 'IS_TEMPORARY', 'IS_ICEBERG', 'IS_DYNAMIC', 'IS_HYBRID', 'IS_IMMUTABLE'];
const event = name => appendFileSync(marker('sdk-events'), name + '\n');
export default {
  configure() {},
  createConnection(profile) {
    if (profile.authenticator !== 'EXTERNALBROWSER' && profile.authenticator !== 'externalbrowser') throw Error('incorrect route');
    event('create');
    let changeAfterSchemaCheck = false;
    let shiftedSchema;
    let policyAfterSchemaCheck = false;
    let unsupportedPolicy = false;
    const context = () => {
      if (!existsSync(marker('change-after-schema-recheck')) && !existsSync(marker('drift-during-schema'))) shiftedSchema = undefined;
      if (!existsSync(marker('policy-error-after-schema-recheck'))) {
        policyAfterSchemaCheck = false;
        unsupportedPolicy = false;
      }
      return {
        schema: profile.schema, warehouse: profile.warehouse,
        secondaryRoles: { roles: '', value: 'NONE' }, session: 'SESSION',
        ...(existsSync(marker('metadata-state.json')) ? JSON.parse(readFileSync(marker('metadata-state.json'), 'utf8')) : {}),
        ...(shiftedSchema ? { schema: shiftedSchema } : {}),
      };
    };
    return {
      connectAsync(callback) {
        event('connect');
        writeFileSync(marker('auth-started'), JSON.stringify({ pid: process.pid, root: process.env.JOINERY_SNOWFLAKE_STATE_DIR }));
        if (existsSync(marker('stall-auth'))) return new Promise(() => {});
        queueMicrotask(() => callback(null));
        return Promise.resolve();
      },
      destroy(callback) { event('destroy'); setTimeout(() => callback(null), 40); },
      execute(options) {
        appendFileSync(marker('queries'), options.sqlText + '\n');
        const id = ++sequence;
        const sql = options.sqlText;
        const current = context();
        let names = ['C0'];
        let types = ['TEXT'];
        let values = [['1']];
        let failure = null;
        if (sql.startsWith('SELECT CURRENT_USER() AS PRINCIPAL')) {
          names = contextColumns;
          values = [['SERVER_USER', profile.role, profile.database, current.schema, current.warehouse,
            current.session, JSON.stringify([`${profile.database}.PUBLIC`, `${profile.database}.OTHER`]),
            JSON.stringify(current.secondaryRoles)]];
          if (changeAfterSchemaCheck) {
            // Return the old context to the driver's post-column recheck, then
            // expose a new one to Studio's independent post-schema read. This
            // deterministic boundary makes the outer recheck essential.
            shiftedSchema = 'HIDDEN_AFTER_SCHEMA';
            changeAfterSchemaCheck = false;
            writeFileSync(marker('schema-context-shifted'), 'after-driver-recheck');
          }
        } else if (sql.startsWith('SELECT CURRENT_USER')) {
          values = [['SERVER_USER', profile.role, profile.database, profile.schema, profile.warehouse, 'SESSION']];
          names = values[0].map((_, index) => `C${index}`);
        } else if (sql === 'SELECT CURRENT_VERSION()') {
          values = [['synthetic-version']];
        } else if (sql === "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION") {
          names = ['key', 'value'];
          values = [['QUOTED_IDENTIFIERS_IGNORE_CASE', unsupportedPolicy ? 'true' : 'false']];
          if (policyAfterSchemaCheck) {
            // The driver sees the supported policy in its final recheck. Only
            // Studio's later identity read observes the unsupported policy.
            policyAfterSchemaCheck = false;
            unsupportedPolicy = true;
            writeFileSync(marker('schema-policy-shifted'), 'after-driver-policy-recheck');
          }
        } else if (sql.includes('.INFORMATION_SCHEMA.SCHEMATA ')) {
          names = ['CATALOG_NAME', 'SCHEMA_NAME'];
          values = [[profile.database, 'OTHER'], [profile.database, 'PUBLIC']];
          if (existsSync(marker('deny-catalog'))) {
            // Model a definitive SQL rejection, not a transport failure with an
            // unknown dispatch outcome. The real helper deliberately retires
            // unclassified SDK errors rather than pretending they are denied SQL.
            failure = Object.assign(Error('synthetic metadata denied'), { sqlState: '42501' });
          }
        } else if (sql.includes('.INFORMATION_SCHEMA.TABLES ')) {
          const schema = sql.includes("TABLE_SCHEMA = 'PUBLIC'") ? 'PUBLIC' : 'OTHER';
          names = relationColumns;
          values = [[profile.database, schema, 'SAME_NAME', 'BASE TABLE', 'NO', 'NO', 'NO', 'NO', 'NO', 'NO']];
          if (existsSync(marker('malformed-catalog'))) names = ['WRONG_CATALOG', ...relationColumns.slice(1)];
        } else if (sql.startsWith('SELECT * FROM ') && sql.endsWith(' LIMIT 0')) {
          names = ['ID', 'NAME'];
          types = ['FIXED', 'TEXT'];
          values = [];
          if (existsSync(marker('change-after-schema-recheck'))) changeAfterSchemaCheck = true;
          if (existsSync(marker('drift-during-schema'))) {
            shiftedSchema = 'HIDDEN_DURING_SCHEMA';
            writeFileSync(marker('schema-context-shifted'), 'during-column-read');
          }
          if (existsSync(marker('policy-error-after-schema-recheck'))) policyAfterSchemaCheck = true;
          if (existsSync(marker('deny-public-columns')) && sql.includes('."PUBLIC".')) {
            failure = Object.assign(Error('synthetic column read denied'), { sqlState: '42501' });
          }
        }
        const statement = {
          getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
          getColumns: () => names.map((name, index) => ({ getName: () => name, getType: () => types[index] ?? 'TEXT' })),
          streamRows: () => Readable.from(values),
          cancel: callback => callback(null),
        };
        setImmediate(() => options.complete(failure, statement));
        return statement;
      },
    };
  },
};
