import test from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";
import { Decoder, encode } from "../src/framing.mjs";
const generation = "b512e129-c689-48bd-b5c2-848438b1231a";
const requestId = "11111111-1111-4111-8111-111111111111",
  queryId = "22222222-2222-4222-8222-222222222222";
test(
  "two synthetic million-row streams cross 10k controls with exact bounded batches",
  { timeout: 30000 },
  async (t) => {
    let helper,
      wire = 0,
      rows = 0,
      batches = 0,
      peak = 0,
      current = null;
    const incoming = new Decoder(),
      outgoing = new Decoder();
    const send = (op, payload) => {
      const r = { v: 1, generation, id: String(++wire), op, payload };
      for (const framed of incoming.push(encode(r))) helper.receive(framed);
      return r.id;
    };
    const sdk = {
      createConnection() {
        return {
          connectAsync(cb) {
            setImmediate(() => cb(null));
            return Promise.resolve();
          },
          destroy(cb) {
            cb();
          },
          execute(opts) {
            let acknowledged = false;
            const statement = {
              getRequestId: () => requestId,
              getQueryId: () => (acknowledged ? queryId : undefined),
              getColumns: () => [
                { getName: () => "DUPLICATE", getType: () => "fixed" },
                { getName: () => "DUPLICATE", getType: () => "text" },
              ],
              cancel(cb) {
                cb();
              },
              streamRows() {
                return Readable.from(
                  (function* () {
                    for (let i = 0; i < 1000000; i++) yield ["9007199254740993", String(i)];
                  })(),
                  { objectMode: true, highWaterMark: 1 },
                );
              },
            };
            setImmediate(() => {
              acknowledged = true;
              opts.complete(null, statement);
            });
            return statement;
          },
        };
      },
    };
    helper = new Helper(
      sdk,
      (raw) => {
        const [event] = outgoing.push(encode(raw));
        if (event.type === "rows") {
          assert(event.rows.length <= 128);
          assert(Buffer.byteLength(JSON.stringify(event.rows)) <= 262144);
          for (const row of event.rows) {
            assert.equal(row[0], "9007199254740993");
            assert.equal(row[1], String(rows % 1000000));
            rows++;
          }
          batches++;
          if (batches % 128 === 0) peak = Math.max(peak, process.memoryUsage().heapUsed);
          send("credit", { target: event.id, batches: 1, discard: false });
        }
        if (event.type === "terminal" && current?.id === event.id) {
          const resolve = current.resolve;
          current = null;
          resolve(event);
        }
        return Promise.resolve();
      },
      () => assert.fail("unexpected retirement"),
      { clientConfigFile: "/owned/disabled" },
    );
    const connected = new Promise((resolve) => {
      current = { id: "1", resolve };
    });
    send("connect", {
      account: "synthetic",
      host: "synthetic.snowflakecomputing.com",
      username: "LOGIN_NAME",
      role: "READER",
      database: "TEST",
      schema: "PUBLIC",
      warehouse: "TINY",
    });
    await connected;
    global.gc?.();
    const baseline = process.memoryUsage().heapUsed,
      start = performance.now();
    for (let run = 0; run < 2; run++) {
      const done = new Promise((resolve) => {
        current = { id: String(wire + 1), resolve };
      });
      send("query", {
        sql: "SELECT synthetic_only",
        classification: "read",
        binds: [],
        credits: 1,
        maxRows: 1000000,
        maxBytes: 1073741824,
      });
      const terminal = await done;
      assert.equal(terminal.ok, true);
      assert.equal(terminal.result.rowsSeen, 1000000);
      assert.equal(terminal.result.rowsRetained, 1000000);
      assert.equal(terminal.result.truncated, false);
    }
    global.gc?.();
    const retained = process.memoryUsage().heapUsed - baseline;
    assert.equal(rows, 2000000);
    assert.equal(batches, 15626);
    assert(wire > 10000);
    // GC-enabled invocation qualifies retained helper/test memory after both reads;
    // it says nothing about the real SDK's remote chunk/decompression memory.
    if (global.gc) assert(retained < 8 * 1024 * 1024, `retained heap grew ${retained}`);
    t.diagnostic(
      JSON.stringify({
        syntheticOnly: true,
        rows,
        batches,
        wireRequests: wire,
        elapsedMs: Math.round(performance.now() - start),
        retainedHeapDeltaBytes: retained,
        sampledPeakHeapDeltaBytes: peak - baseline,
        gcEnabled: !!global.gc,
      }),
    );
  },
);
