// Local interoperability fixture: real helper code, injected fake SDK, no SDK import/network.
const { Helper } = await import(
  new URL("../../../snowflake-helper/src/helper.mjs", import.meta.url)
);
const { Decoder, writer } = await import(
  new URL("../../../snowflake-helper/src/framing.mjs", import.meta.url)
);
import { Readable } from "node:stream";
let next = 0,
  schema = "PUBLIC";
const uuid = (n) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
const sdk = {
  createConnection: () => ({
    connectAsync(callback) {
      queueMicrotask(() => callback(null));
      return Promise.resolve();
    },
    destroy(callback) {
      setTimeout(() => callback(null), 80);
    },
    execute(options) {
      const n = ++next,
        sql = options.sqlText;
      if (sql.startsWith("USE SCHEMA")) schema = sql.split('"')[1];
      const identity = sql.startsWith("SELECT CURRENT_USER");
      const rows = identity
        ? [["SERVER_USER", "ROLE", "DB", schema, "WH", "SESSION"]]
        : sql.startsWith("USE SCHEMA")
          ? []
          : [["9007199254740993", "exact"]];
      const statement = {
        getRequestId: () => uuid(n),
        getQueryId: () => uuid(n + 1000),
        getColumns: () =>
          Array.from({ length: identity ? 6 : sql.startsWith("USE SCHEMA") ? 0 : 2 }, (_, i) => ({
            getName: () => `c${i}`,
            getType: () => "TEXT",
          })),
        streamRows: () => Readable.from(rows),
        cancel(callback) {
          callback(null);
        },
      };
      if (sql === 'USE SCHEMA "WAIT"') setTimeout(() => options.complete(null, statement), 250);
      else setImmediate(() => options.complete(null, statement));
      return statement;
    },
  }),
};
let stopping = false;
const send = writer(process.stdout.write.bind(process.stdout), () => process.exit(2));
const retire = () => {
  if (stopping) return;
  stopping = true;
  send.flush().then(() => process.exit(0));
};
const helper = new Helper(sdk, send, retire, {
  clientConfigFile: "/nonexistent/synthetic-config",
  closeTimeoutMs: 200,
});
const decoder = new Decoder();
process.stdin.on("data", (chunk) => {
  try {
    for (const frame of decoder.push(chunk)) helper.receive(frame);
  } catch {
    retire();
  }
});
process.stdin.on("end", retire);
