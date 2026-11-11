import assert from 'node:assert/strict';
import test from 'node:test';
import { makeSdk } from './snowflake_table_qualification_sdk.mjs';

const tableName = 'JOINERY_QUAL_0123456789ABCDEF0123456789ABCDEF';
const target = `"SANDBOX"."BILLIE_TESTING"."${tableName}"`;
const statements = {
  Create: `CREATE TABLE ${target} ("ID" NUMBER(38, 0), "NOTE" VARCHAR(128), "ACTIVE" BOOLEAN)`,
  Add: `ALTER TABLE ${target} ADD COLUMN "EXTRA" VARCHAR(32)`,
  Drop: `DROP TABLE ${target} RESTRICT`,
  RollbackInsert: `INSERT INTO ${target} ("ID", "NOTE", "ACTIVE") VALUES (1, 'rollback-only', TRUE)`,
  CommittedInsert: `INSERT INTO ${target} ("ID", "NOTE", "ACTIVE") VALUES (2, 'committed', TRUE)`,
  Update: `UPDATE ${target} SET "NOTE" = 'updated', "ACTIVE" = FALSE WHERE "ID" = 2`,
  Delete: `DELETE FROM ${target} WHERE "ID" = 2`,
  OwnershipRead: `SELECT TABLE_OWNER, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY, CREATED::VARCHAR AS CREATED FROM "SANDBOX".INFORMATION_SCHEMA.TABLES WHERE TABLE_CATALOG = 'SANDBOX' AND TABLE_SCHEMA = 'BILLIE_TESTING' AND TABLE_NAME = '${tableName}' ORDER BY TABLE_NAME LIMIT 2`,
  DataRead: `SELECT "ID", "NOTE", "ACTIVE", "EXTRA" FROM ${target} ORDER BY "ID" LIMIT 2`,
};
const options = { schemaCount: 1, tableName, statements };

async function connect(fault = null) {
  const connection = makeSdk({ ...options, fault }).createConnection({
    database: 'SANDBOX', schema: 'BILLIE_TESTING',
    authenticator: 'EXTERNALBROWSER', role: 'ROLE', warehouse: 'WAREHOUSE',
  });
  await new Promise((resolve, reject) => connection.connectAsync(error => error ? reject(error) : resolve()));
  return connection;
}

function execute(connection, sqlText, binds = []) {
  return new Promise((resolve, reject) => connection.execute({
    sqlText, binds,
    complete: (error, statement) => error ? reject(error) : resolve(statement),
  }));
}

test('accepts the independently spelled nine-statement contract', () => {
  assert.equal(typeof makeSdk(options).createConnection, 'function');
});

test('rejects every statement when its generated target differs', () => {
  for (const [key, sql] of Object.entries(statements)) {
    assert.throws(() => makeSdk({ ...options, statements: {
      ...statements, [key]: sql.replaceAll(tableName, 'EXISTING_TABLE'),
    } }), /Unexpected synthetic qualification statement/, key);
  }
});

test('rejects swapped statement meanings', () => {
  assert.throws(() => makeSdk({ ...options, statements: {
    ...statements, Create: statements.Drop, Drop: statements.Create,
  } }), /Unexpected synthetic qualification statement/);
  assert.throws(() => makeSdk({ ...options, statements: {
    ...statements, RollbackInsert: statements.CommittedInsert,
    CommittedInsert: statements.RollbackInsert,
  } }), /Unexpected synthetic qualification statement/);
});

test('rejects appended SQL, changed grammar and additional statement keys', () => {
  for (const [key, sql] of Object.entries(statements)) {
    assert.throws(() => makeSdk({ ...options, statements: {
      ...statements, [key]: `${sql}; DROP TABLE "OTHER"`,
    } }), /Unexpected synthetic qualification statement/, key);
  }
  assert.throws(() => makeSdk({ ...options, statements: {
    ...statements, Drop: `DROP TABLE ${target} CASCADE`,
  } }), /Unexpected synthetic qualification statement/);
  assert.throws(() => makeSdk({ ...options, statements: {
    ...statements, Extra: 'SELECT 1',
  } }), /exactly the nine frozen statements/);
});

test('rejects unknown execute SQL and nonempty binds before changing state', async () => {
  const connection = await connect();
  try {
    await assert.rejects(execute(connection, 'SELECT 1'), /Unrecognized synthetic qualification SQL/);
    await assert.rejects(execute(connection, `${statements.Create}; DROP TABLE "OTHER"`), /Unrecognized synthetic qualification SQL/);
    await assert.rejects(execute(connection, statements.Create, ['unexpected']), /Unrecognized synthetic qualification SQL or binds/);
    const created = await execute(connection, statements.Create);
    assert.deepEqual(await Array.fromAsync(created.streamRows()), [['Statement executed successfully.']]);
  } finally {
    await new Promise(resolve => connection.destroy(resolve));
  }
});

for (const release of ['cancel', 'destroy']) {
  test(`one-shot stalled read exposes columns and settles through ${release}`, async () => {
    const connection = await connect({ kind: 'stallRead', statement: 'DataRead' });
    try {
      await execute(connection, statements.Create);
      await execute(connection, statements.Add);
      const statement = await execute(connection, statements.DataRead);
      assert.deepEqual(statement.getColumns().map(column => column.getName()), ['ID', 'NOTE', 'ACTIVE', 'EXTRA']);
      const iterator = statement.streamRows()[Symbol.asyncIterator]();
      let settled = false;
      const first = iterator.next().then(value => { settled = true; return value; });
      await Promise.resolve();
      assert.equal(settled, false);
      await new Promise(resolve => (release === 'cancel' ? statement : connection)[release](resolve));
      assert.deepEqual(await first, { value: undefined, done: true });
      if (release === 'cancel') {
        const next = await execute(connection, statements.DataRead);
        assert.deepEqual(await Array.fromAsync(next.streamRows()), []);
      }
    } finally {
      await new Promise(resolve => connection.destroy(resolve));
    }
  });

  test(`one-shot stalled read error retains SQLSTATE through ${release}`, async () => {
    const connection = await connect({ kind: 'stallReadError', statement: 'DataRead' });
    try {
      await execute(connection, statements.Create);
      await execute(connection, statements.Add);
      const statement = await execute(connection, statements.DataRead);
      assert.deepEqual(statement.getColumns().map(column => column.getName()), ['ID', 'NOTE', 'ACTIVE', 'EXTRA']);
      const iterator = statement.streamRows()[Symbol.asyncIterator]();
      let settled = false;
      const first = iterator.next();
      const rejected = assert.rejects(first, error => {
        settled = true;
        assert.equal(error.message, 'Synthetic qualification stalled read interrupted');
        assert.equal(error.sqlState, '57014');
        assert.equal(Object.hasOwn(error, 'code'), false);
        return true;
      });
      await Promise.resolve();
      assert.equal(settled, false);
      await new Promise(resolve => (release === 'cancel' ? statement : connection)[release](resolve));
      await rejected;
      if (release === 'cancel') {
        const next = await execute(connection, statements.DataRead);
        assert.deepEqual(await Array.fromAsync(next.streamRows()), []);
      }
    } finally {
      await new Promise(resolve => connection.destroy(resolve));
    }
  });
}

test('stalled read faults cannot target mutation or ownership statements', () => {
  for (const kind of ['stallRead', 'stallReadError']) {
    for (const statement of Object.keys(statements).filter(key => key !== 'DataRead')) {
      assert.throws(() => makeSdk({ ...options, fault: { kind, statement } }), /Invalid synthetic qualification fault/);
    }
  }
});
