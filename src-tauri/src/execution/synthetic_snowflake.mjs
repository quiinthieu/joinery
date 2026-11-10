// Synthetic SDK for Rust application lifecycle tests. Never packaged.
import { Readable } from 'node:stream';
import { appendFileSync, existsSync, writeFileSync } from 'node:fs';
const controls = new URL('../../', import.meta.url);
const marker = name => new URL(name, controls);
let sequence = 0;
const uuid = n => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`;
export default {
  configure() {},
  createConnection(profile) {
    if (profile.authenticator !== 'EXTERNALBROWSER' && profile.authenticator !== 'externalbrowser') throw Error('incorrect route');
    return {
      connectAsync(callback) {
        writeFileSync(marker('auth-started'), JSON.stringify({ pid: process.pid, root: process.env.JOINERY_SNOWFLAKE_STATE_DIR }));
        if (existsSync(marker('stall-auth'))) return new Promise(() => {});
        queueMicrotask(() => callback(null));
        return Promise.resolve();
      },
      destroy(callback) { setTimeout(() => callback(null), 40); },
      execute(options) {
        appendFileSync(marker('queries'), options.sqlText + '\n');
        const id = ++sequence;
        const identity = options.sqlText.startsWith('SELECT CURRENT_USER');
        const values = identity
          ? [['SERVER_USER', profile.role, profile.database, profile.schema, profile.warehouse, 'SESSION']]
          : options.sqlText === 'SELECT CURRENT_VERSION()' ? [['synthetic-version']] : [['1']];
        const statement = {
          getRequestId: () => uuid(id), getQueryId: () => uuid(id + 1000),
          getColumns: () => values[0].map((_, i) => ({ getName: () => `C${i}`, getType: () => 'TEXT' })),
          streamRows: () => Readable.from(values),
          cancel: callback => callback(null),
        };
        setImmediate(() => options.complete(null, statement));
        return statement;
      },
    };
  },
};
