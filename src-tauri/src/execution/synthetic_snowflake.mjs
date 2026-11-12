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
    let transactionId = null;
    let ddlCreated = false;
    let ddlExecuted = false;
    let consoleValue = null;
    let tempRows = [];
    let transactionRows = null;
    let lastReviewed = { names: [], types: [], values: [] };
    const context = () => {
      if (!existsSync(marker('change-after-schema-recheck')) && !existsSync(marker('drift-during-schema'))) shiftedSchema = undefined;
      if (!existsSync(marker('policy-error-after-schema-recheck'))) {
        policyAfterSchemaCheck = false;
        unsupportedPolicy = false;
      }
      return {
        schema: profile.schema, warehouse: profile.warehouse, role: profile.role,
        autocommit: true, abortOnError: false, transactionId,
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
        const reviewed = options.parameters?.MULTI_STATEMENT_COUNT === 1;
        let cancelled = false;
        const boundary = ['BEGIN TRANSACTION', 'COMMIT', 'ROLLBACK'].includes(sql);
        const dml = /^(INSERT INTO|UPDATE|DELETE FROM) /.test(sql);
        const ddl = /^(CREATE TABLE|ALTER TABLE|DROP TABLE) /.test(sql);
        const ddlFinalObserver = ddlExecuted && sql.startsWith('SELECT CURRENT_USER() AS PRINCIPAL');
        if (ddlFinalObserver) writeFileSync(marker('ddl-final-observer-entered'), String(id));
        let names = ['C0'];
        let types = ['TEXT'];
        let values = [['1']];
        let failure = null;
        if (reviewed) {
          appendFileSync(marker('reviewed-specs'), JSON.stringify({sql,binds:options.binds,parameters:options.parameters}) + '\n');
          names = ['VALUE']; types = ['TEXT'];
          if (sql.startsWith('SET CONSOLE_VALUE = ')) {
            consoleValue = sql.endsWith('?') ? options.binds[0] : sql.slice(20).replace(/^'|'$/g, '');
            names = ['status']; values = [['Statement executed successfully.']];
          } else if (sql === 'SELECT $CONSOLE_VALUE') {
            values = [[consoleValue]];
          } else if (sql === 'SHOW TABLES') {
            names = ['name']; values = [['CONSOLE_TEMP']];
          } else if (sql === 'SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()))') {
            ({ names, types, values } = structuredClone(lastReviewed));
          } else if (sql === 'CREATE TEMPORARY TABLE CONSOLE_TEMP (V NUMBER)') {
            tempRows = []; names = ['status']; values = [['Statement executed successfully.']];
          } else if (sql === 'INSERT INTO CONSOLE_TEMP VALUES (7)') {
            tempRows.push(['7']); names = ['number of rows inserted']; types = ['FIXED']; values = [['1']];
          } else if (sql === 'SELECT * FROM CONSOLE_TEMP') {
            names = ['V']; types = ['FIXED']; values = structuredClone(tempRows);
          } else if (sql === 'BEGIN' || sql === 'BEGIN TRANSACTION') {
            transactionRows = structuredClone(tempRows); names = []; types = []; values = [];
          } else if (sql === 'ROLLBACK' || sql === 'COMMIT') {
            if (sql === 'ROLLBACK' && transactionRows) tempRows = transactionRows;
            transactionRows = null; names = []; types = []; values = [];
          } else if (sql === 'SELECT CONSOLE_REJECT' || existsSync(marker('reject-reviewed'))) {
            failure = Object.assign(Error('synthetic reviewed SQL rejected'), { sqlState: '42000' });
          } else {
            values = [[sql === 'SELECT ?' ? options.binds[0] : /^SELECT [0-9]+$/.test(sql) ? sql.slice(7) : '1']];
          }
          if (!failure) lastReviewed = structuredClone({ names, types, values });
        } else if (ddl) {
          ddlExecuted = true;
          names = ['status']; types = ['TEXT']; values = [['Statement executed successfully.']];
          appendFileSync(marker('ddl-effects'), JSON.stringify({ sql }) + '\n');
          writeFileSync(marker('ddl-entered'), String(id));
          if (existsSync(marker('reject-ddl'))) {
            failure = Object.assign(Error('synthetic DDL rejected'), { sqlState: '42501' });
          } else if (sql.startsWith('CREATE TABLE "DB"."PUBLIC"."DDL_TEST" ')) {
            ddlCreated = true;
          }
          if (existsSync(marker('deny-observer-after-ddl'))) {
            writeFileSync(marker('deny-observer'), 'synthetic DDL final-observer denial');
          }
        } else if (dml) {
          const kind = sql.startsWith('INSERT') ? 'insert' : sql.startsWith('UPDATE') ? 'update' : 'delete';
          names = [`number of rows ${kind === 'insert' ? 'inserted' : kind === 'update' ? 'updated' : 'deleted'}`];
          values = [['9007199254740993']];
          if (kind === 'update') {
            names.push('number of multi-joined rows updated');
            values[0].push('7');
          }
          types = names.map(() => 'FIXED');
          appendFileSync(marker('dml-effects'), JSON.stringify({ sql, binds: options.binds }) + '\n');
          writeFileSync(marker('dml-entered'), String(id));
          if (existsSync(marker('deny-observer-after-dml'))) {
            writeFileSync(marker('deny-observer'), 'synthetic DML final-observer denial');
          }
        } else if (boundary) {
          names = []; values = [];
          transactionId = sql === 'BEGIN TRANSACTION' ? 'opaque-studio-tx:90071992547409931234567890' : null;
          appendFileSync(marker('transaction-effects'), sql + '\n');
          if (sql === 'BEGIN TRANSACTION' && existsSync(marker('deny-observer-after-boundary'))) {
            writeFileSync(marker('deny-observer'), 'synthetic aborted-observer recovery');
          }
        } else if (sql.startsWith('SELECT CURRENT_USER() AS PRINCIPAL')) {
          names = contextColumns;
          values = [['SERVER_USER', current.role, profile.database, current.schema, current.warehouse,
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
        } else if (sql === "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION") {
          names = ['key', 'value'];
          values = [['AUTOCOMMIT', current.autocommit ? 'true' : 'false']];
          if (existsSync(marker('deny-observer'))) {
            failure = Object.assign(Error('synthetic observer denied'), { sqlState: '42501' });
          }
        } else if (sql === "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION") {
          names = ['key', 'value'];
          values = [['TRANSACTION_ABORT_ON_ERROR', current.abortOnError ? 'true' : 'false']];
        } else if (sql === 'SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID') {
          names = ['TRANSACTION_ID'];
          values = [[current.transactionId]];
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
          if (ddlCreated && schema === 'PUBLIC') values.push([profile.database, schema, 'DDL_TEST', 'BASE TABLE', 'NO', 'NO', 'NO', 'NO', 'NO', 'NO']);
          if (sql.includes('CREATED::VARCHAR AS CREATED')) {
            names = [...names, 'CREATED', 'LAST_DDL'];
            values = values.map(row => [...row, 'created', 'last-ddl']);
          }
          if (existsSync(marker('malformed-catalog'))) names = ['WRONG_CATALOG', ...relationColumns.slice(1)];
        } else if (sql.includes('.INFORMATION_SCHEMA.COLUMNS AS C ')) {
          names = ['TABLE_CATALOG','TABLE_SCHEMA','TABLE_NAME','COLUMN_NAME','ORDINAL_POSITION','COLUMN_DEFAULT','IS_NULLABLE','DATA_TYPE','DATA_TYPE_ALIAS','CHARACTER_MAXIMUM_LENGTH','CHARACTER_OCTET_LENGTH','NUMERIC_PRECISION','NUMERIC_PRECISION_RADIX','NUMERIC_SCALE','IS_IDENTITY','IDENTITY_GENERATION','IDENTITY_START','IDENTITY_INCREMENT','IDENTITY_ORDERED','COMMENT','EXPRESSION','KIND'];
          const schema = sql.includes("TABLE_SCHEMA = 'PUBLIC'") ? 'PUBLIC' : 'OTHER';
          values = [[profile.database,schema,'SAME_NAME','ID','1',null,'NO','NUMBER',null,null,null,'38','10','0','NO',null,null,null,null,null,null,'COLUMN'],[profile.database,schema,'SAME_NAME','NAME','3',null,'YES','TEXT',null,'100','400',null,null,null,'NO',null,null,null,null,'synthetic comment',null,'COLUMN']];
        } else if (sql.startsWith('DESCRIBE TABLE ')) {
          names = ['name','type','kind','null?','default','expression','comment'];
          values = [['ID','NUMBER(38,0)','COLUMN','N',null,null,null],['NAME','VARCHAR(100)','COLUMN','Y',null,null,'synthetic comment']];
        } else if (sql.startsWith('SELECT "ID", "NAME" FROM ')) {
          names = ['ID','NAME']; types = ['FIXED','TEXT'];
          values = [['9007199254740993',null],['2','']];
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
          getSessionState: () => existsSync(marker('null-reviewed-context')) ? null : ({
            getCurrentDatabase: () => profile.database,
            getCurrentSchema: () => context().schema,
            getCurrentRole: () => context().role,
            getCurrentWarehouse: () => context().warehouse,
          }),
          streamRows() {
            if (reviewed && existsSync(marker('fail-reviewed-drain'))) {
              return Readable.from((async function*() { throw {sqlState:'58000'}; })());
            }
            if (reviewed && existsSync(marker('hold-reviewed-drain'))) {
              writeFileSync(marker('reviewed-drain-entered'), '');
              return Readable.from((async function*() {
                while (!existsSync(marker('release-reviewed-drain')) && !cancelled) await new Promise(resolve => setTimeout(resolve,5));
                if (cancelled) throw {sqlState:'57014'};
                yield* values;
              })());
            }
            return Readable.from(values);
          },
          cancel: callback => { cancelled = true; callback(null); },
        };
        setImmediate(() => {
          // Test-only loss after synthetic execution and before its response.
          // This never claims logout or real Snowflake transaction effects.
          if (reviewed && existsSync(marker('lose-reviewed-ack'))) process.exit(0);
          if (boundary && existsSync(marker('lose-transaction-ack'))) process.exit(0);
          if (dml && existsSync(marker('lose-dml-ack'))) process.exit(0);
          if (ddl && existsSync(marker('lose-ddl-ack'))) process.exit(0);
          const complete = () => {
            if (reviewed && existsSync(marker('hold-reviewed-ack')) && !existsSync(marker('release-reviewed-ack')) && !cancelled) {
              writeFileSync(marker('reviewed-ack-entered'), '');
              setTimeout(complete, 10);
              return;
            }
            if (boundary && existsSync(marker('hold-transaction-ack')) && !existsSync(marker('release-transaction-ack'))) {
              setTimeout(complete, 10);
              return;
            }
            if (dml && existsSync(marker('hold-dml-ack')) && !existsSync(marker('release-dml-ack'))) {
              setTimeout(complete, 10);
              return;
            }
            if (ddl && existsSync(marker('hold-ddl-ack')) && !existsSync(marker('release-ddl-ack'))) {
              setTimeout(complete, 10);
              return;
            }
            if (ddlFinalObserver && existsSync(marker('hold-ddl-final-observer')) && !existsSync(marker('release-ddl-final-observer'))) {
              setTimeout(complete, 10);
              return;
            }
            options.complete(failure, statement);
          };
          complete();
        });
        return statement;
      },
    };
  },
};
