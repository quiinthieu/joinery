// Actual pinned SDK; synthetic login and gzip result chunks on owned loopback only.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import net from "node:net";
import { lstat, writeFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { gzipSync, gunzipSync } from "node:zlib";
import { setTimeout as delay, setImmediate as yieldTurn } from "node:timers/promises";
import { connectionOptions } from "../src/helper.mjs";

const scenario = process.argv[2];
assert(["million", "cancel"].includes(scenario));
assert.equal(process.argv.length, 3);
const root = process.env.JOINERY_SDK_LARGE_RESULT_ROOT;
assert(root && isAbsolute(root));
const rootStat = await lstat(root);
assert(rootStat.isDirectory() && !rootStat.isSymbolicLink());
assert.equal(rootStat.mode & 0o777, 0o700);
assert.equal(rootStat.uid, process.getuid());
const config = join(root, "disabled-config.json");
await writeFile(config, JSON.stringify({ common: { log_level: "OFF", log_path: root } }), {
  mode: 0o600,
  flag: "wx",
});

const TOTAL = 1_000_000;
const CHUNK_ROWS = 10_000;
const BASE = 90071992547409930000000000000000000000n;
const DECIMAL = "-12345678901234567890.123456789012345678";
const rowAt = (i) => [
  String(BASE + BigInt(i)),
  DECIMAL,
  `row:${i},"quoted"\r\n雪🪵`,
  i % 17 === 0 ? null : "",
];
const report = {
  scenario,
  sdkVersion: null,
  scope: "actual SDK local HTTP/gzip/chunk/stream behavior; synthetic server and OAuth only",
  optionDifferences: [
    "synthetic OAuth replaces external-browser authentication",
    "owned loopback HTTP endpoint",
  ],
  totalRows: TOTAL,
  chunkRows: CHUNK_ROWS,
  expectedChunks: TOTAL / CHUNK_ROWS,
  queryRequests: 0,
  chunkRequests: 0,
  chunkIndexes: [],
  abortRequests: 0,
  actualNetworkDenied: 0,
  consumedRows: 0,
  maxReadableLength: 0,
  compressedBytesServed: 0,
  uncompressedBytesServed: 0,
  memoryScope:
    "SDK plus lazy in-process loopback simulator; no total-RSS assertion or remote bound",
  nodeOldSpaceLimitMiB: 256,
  memorySamples: 0,
  peakSampled: { rss: 0, heapUsed: 0, external: 0, arrayBuffers: 0 },
};
const output = process.stdout.write.bind(process.stdout);
const sockets = new Set();
const serverErrors = [];
let connection;
let sampler;
report.phase = "listener";
const watchdog = setTimeout(() => {
  report.failure = { name: "Deadline", phase: report.phase };
  for (const socket of sockets) socket.destroy();
  server.close(() => {
    report.listenerClosed = true;
    output(JSON.stringify(report) + "\n", () => process.exit(1));
  });
}, 60_000);
const start = performance.now();
function sample() {
  const memory = process.memoryUsage();
  report.memorySamples++;
  for (const key of Object.keys(report.peakSampled))
    report.peakSampled[key] = Math.max(report.peakSampled[key], memory[key]);
  return memory;
}
function json(response, value) {
  response.writeHead(200, { "content-type": "application/json" });
  response.end(JSON.stringify(value));
}
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://127.0.0.1");
    if (url.pathname.startsWith("/chunks/")) {
      assert.equal(request.method, "GET");
      const index = Number(url.pathname.slice("/chunks/".length));
      assert(Number.isInteger(index) && index >= 0 && index < TOTAL / CHUNK_ROWS);
      report.chunkRequests++;
      report.chunkIndexes.push(index);
      // Build only the requested 10k-row chunk; never retain a million-row fixture.
      const rows = Array.from({ length: CHUNK_ROWS }, (_, offset) =>
        JSON.stringify(rowAt(index * CHUNK_ROWS + offset)),
      ).join(",");
      const compressed = gzipSync(rows);
      report.uncompressedBytesServed += Buffer.byteLength(rows);
      report.compressedBytesServed += compressed.length;
      sample();
      response.writeHead(200, {
        // Storage chunks are comma-separated row fragments, not standalone JSON.
        // The SDK adds outer brackets after the HTTP layer returns the text.
        "content-type": "application/octet-stream",
        "content-encoding": "gzip",
        "content-length": compressed.length,
      });
      response.end(compressed);
      return;
    }
    let size = 0;
    const buffers = [];
    for await (const buffer of request) {
      size += buffer.length;
      assert(size <= 1_048_576);
      buffers.push(buffer);
    }
    const bytes = Buffer.concat(buffers);
    const body = bytes.length
      ? JSON.parse(
          (request.headers["content-encoding"] === "gzip"
            ? gunzipSync(bytes, { maxOutputLength: 1_048_576 })
            : bytes
          ).toString(),
        )
      : {};
    if (url.pathname === "/session/v1/login-request") {
      json(response, {
        success: true,
        data: {
          token: "SYNTHETIC_SESSION",
          masterToken: "SYNTHETIC_MASTER",
          validityInSeconds: 3600,
          masterValidityInSeconds: 3600,
          sessionId: 1,
          parameters: [],
          sessionInfo: {
            databaseName: "SYNTHETIC",
            schemaName: "PUBLIC",
            roleName: "SYNTHETIC",
            warehouseName: "SYNTHETIC",
          },
        },
      });
    } else if (url.pathname === "/queries/v1/query-request") {
      report.queryRequests++;
      assert.equal(body.sqlText, "SELECT SYNTHETIC_LOCAL_LARGE_RESULT");
      json(response, {
        success: true,
        data: {
          queryId: "11111111-1111-4111-8111-111111111111",
          statementTypeId: 4096,
          version: 1,
          rowtype: [
            { name: "EXACT_INTEGER", type: "fixed", precision: 38, scale: 0, nullable: false },
            { name: "EXACT_DECIMAL", type: "fixed", precision: 38, scale: 18, nullable: false },
            { name: "TEXT", type: "text", length: 1024, nullable: false },
            { name: "NULL_OR_EMPTY", type: "text", length: 8, nullable: true },
          ],
          rowset: [],
          total: TOTAL,
          returned: TOTAL,
          parameters: [],
          chunks: Array.from({ length: TOTAL / CHUNK_ROWS }, (_, i) => ({
            url: `http://127.0.0.1:${server.address().port}/chunks/${i}`,
            rowCount: CHUNK_ROWS,
          })),
        },
      });
    } else if (url.pathname.endsWith("/abort-request")) {
      report.abortRequests++;
      json(response, { success: true, data: {} });
    } else if (["/session", "/telemetry/send"].includes(url.pathname)) {
      json(response, { success: true, data: {} });
    } else {
      throw new Error("Unexpected synthetic endpoint");
    }
  } catch (error) {
    serverErrors.push(error.name);
    response.destroy();
  }
});
server.on("connection", (socket) => {
  sockets.add(socket);
  socket.once("close", () => sockets.delete(socket));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;
const realConnect = net.Socket.prototype.connect;
net.Socket.prototype.connect = function (...args) {
  let first = args[0];
  if (Array.isArray(first)) first = first[0];
  const host = typeof first === "object" ? first.host : args[1];
  const targetPort = typeof first === "object" ? first.port : first;
  if (host !== "127.0.0.1" || Number(targetPort) !== port) {
    report.actualNetworkDenied++;
    throw new Error("Non-owned transport denied");
  }
  return realConnect.apply(this, args);
};
for (const method of ["log", "info", "warn", "error", "debug", "trace"]) console[method] = () => {};
const require = createRequire(new URL("../package.json", import.meta.url));
try {
  report.phase = "sdk-import";
  const sdk = require("snowflake-sdk");
  report.sdkVersion = require("snowflake-sdk/package.json").version;
  assert.equal(report.sdkVersion, "3.4.1");
  sdk.configure({ logLevel: "OFF", additionalLogToConsole: false });
  connection = sdk.createConnection({
    ...connectionOptions(
      {
        account: "synthetic",
        username: "SYNTHETIC",
        host: "127.0.0.1",
        port,
        protocol: "http",
        accessUrl: `http://127.0.0.1:${port}`,
      },
      config,
    ),
    authenticator: "OAUTH",
    token: "SYNTHETIC_OAUTH",
  });
  report.phase = "connect";
  await new Promise((resolve, reject) =>
    connection.connect((error) => (error ? reject(error) : resolve())),
  );
  global.gc?.();
  report.beforeQuery = sample();
  sampler = setInterval(sample, 20);
  report.phase = "query";
  const statement = await new Promise((resolve, reject) =>
    connection.execute({
      sqlText: "SELECT SYNTHETIC_LOCAL_LARGE_RESULT",
      rowMode: "array",
      fetchAsString: ["Number", "Date", "Boolean", "Buffer", "JSON"],
      streamResult: true,
      complete(error, completed) {
        error ? reject(error) : resolve(completed);
      },
    }),
  );
  report.chunksBeforeStream = report.chunkRequests;
  assert.equal(report.chunksBeforeStream, 0);
  const stream = statement.streamRows();
  report.phase = "consume";
  report.rowHighWaterMark = stream.readableHighWaterMark;
  for await (const row of stream) {
    assert.deepEqual(row, rowAt(report.consumedRows));
    report.consumedRows++;
    report.maxReadableLength = Math.max(report.maxReadableLength, stream.readableLength);
    if (report.consumedRows === 128) {
      await delay(150);
      report.pausedChunkRequests = report.chunkRequests;
      report.pausedReadableLength = stream.readableLength;
      // Pinned default prefetch is two chunks ahead. Do not override it.
      assert.equal(report.pausedChunkRequests, 3);
      assert(report.pausedReadableLength <= stream.readableHighWaterMark);
      if (scenario === "cancel") {
        stream.destroy();
        report.localStreamDestroyed = stream.destroyed;
        await new Promise((resolve, reject) =>
          statement.cancel((error) => (error ? reject(error) : resolve())),
        );
        report.cancelAcknowledged = true;
        report.cancelScope = "local stream stopped; abort acknowledged by synthetic server only";
        const before = report.chunkRequests;
        await delay(150);
        report.chunksAfterCancel = report.chunkRequests;
        assert.equal(report.chunksAfterCancel, before);
        break;
      }
    }
    if (report.consumedRows % 128 === 0) await yieldTurn();
  }
  assert.equal(report.consumedRows, scenario === "million" ? TOTAL : 128);
  report.streamReadableEnded = stream.readableEnded;
  report.resultFullyConsumed = report.consumedRows === TOTAL;
  assert.equal(report.queryRequests, 1);
  if (scenario === "million") {
    assert.equal(report.chunkRequests, TOTAL / CHUNK_ROWS);
    assert.deepEqual(
      report.chunkIndexes,
      Array.from({ length: TOTAL / CHUNK_ROWS }, (_, i) => i),
    );
  } else assert.equal(report.abortRequests, 1);
  report.afterConsumption = sample();
} catch (error) {
  // Never emit row contents, request bodies, configuration, or SDK diagnostics.
  report.failure = {
    name: error.name,
    code: error.code ?? null,
    phase: report.consumedRows ? "consume" : "setup",
  };
  process.exitCode = 1;
} finally {
  report.phase = "cleanup";
  clearTimeout(watchdog);
  clearInterval(sampler);
  if (connection) {
    await Promise.race([
      new Promise((resolve) => connection.destroy(() => resolve())),
      delay(1000),
    ]);
  }
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => server.close(resolve));
  net.Socket.prototype.connect = realConnect;
  report.listenerClosed = true;
  report.serverErrors = serverErrors;
  global.gc?.();
  report.afterCleanup = sample();
  report.resourceUsageMaxRssKiB = process.resourceUsage().maxRSS;
  report.elapsedMs = Math.round(performance.now() - start);
  // SDK-wide background handles may outlive connection.destroy. The fixture
  // owns this child; emit after listener cleanup, flush, then explicitly exit.
  // This does not qualify natural SDK process shutdown.
  report.exitPolicy = "explicit owned-child exit after report flush and loopback listener close";
  output(JSON.stringify(report) + "\n", () => process.exit(process.exitCode ?? 0));
}
