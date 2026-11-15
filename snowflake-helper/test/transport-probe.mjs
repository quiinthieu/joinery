// Actual SDK HTTP transport fixture; only its loopback server is simulated.
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { writeFile, readdir, stat, lstat, chown } from "node:fs/promises";
import { join, isAbsolute } from "node:path";
import { gunzipSync } from "node:zlib";
import net from "node:net";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";

const scenario = process.argv[2] ?? "lost-success-deduped";
const allowed = [
  "missing-config",
  "lost-success-deduped",
  "lost-success-no-dedupe",
  "documented-options",
  "unsupported-options",
  "persistent-loss",
];
assert(allowed.includes(scenario));
assert.equal(process.argv.length, 3);
const root = process.env.JOINERY_TRANSPORT_ROOT;
assert(root && isAbsolute(root));
const rootStat = await lstat(root);
assert(rootStat.isDirectory() && !rootStat.isSymbolicLink());
assert.equal(rootStat.mode & 0o777, 0o700);
assert.equal(rootStat.uid, process.getuid());
// Pinned SDK's missing explicit config path currently fails connect (405505).
// Supply an owned OFF config, never discover a real user's configuration.
if (scenario !== "missing-config")
  await writeFile(
    join(root, "disabled-config.json"),
    JSON.stringify({ common: { log_level: "OFF", log_path: root } }),
    { mode: 0o600, flag: "wx" },
  );
if (scenario !== "missing-config")
  await chown(join(root, "disabled-config.json"), process.getuid(), process.getgid());
const output = process.stdout.write.bind(process.stdout);
const requests = [];
const sockets = new Set();
const cache = new Map();
let effects = 0,
  connection,
  reported = false,
  watchdog;
const report = {
  scenario,
  sdkVersion: "3.4.1",
  transport: "loopback HTTP only; no TLS qualification",
  simulatedServerSuccesses: 0,
  statementCallbacks: 0,
  actualNetworkDenied: 0,
  httpEndpoints: [],
};
function response(res, value) {
  res.writeHead(200, { "content-type": "application/json" });
  res.end(JSON.stringify(value));
}
const server = createServer(async (req, res) => {
  try {
    const chunks = [];
    for await (const chunk of req) {
      chunks.push(chunk);
      assert(Buffer.concat(chunks).length <= 1_048_576);
    }
    const bytes = Buffer.concat(chunks);
    const body = bytes.length
      ? JSON.parse(
          (req.headers["content-encoding"] === "gzip"
            ? gunzipSync(bytes, { maxOutputLength: 1_048_576 })
            : bytes
          ).toString(),
        )
      : {};
    const url = new URL(req.url, "http://127.0.0.1");
    report.httpEndpoints.push(url.pathname);
    if (url.pathname === "/session/v1/login-request") {
      return response(res, {
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
    }
    if (url.pathname === "/queries/v1/query-request") {
      const requestId = url.searchParams.get("requestId");
      requests.push({
        method: req.method,
        requestId,
        retryCount: url.searchParams.get("retryCount"),
        retryReason: url.searchParams.get("retryReason"),
        sequenceIdInUrl: url.searchParams.get("sequenceId"),
        sequenceIdInBody: body.sequenceId ?? null,
        urlKeys: [...url.searchParams.keys()].sort(),
        bodyKeys: Object.keys(body).sort(),
        bodySHA256: createHash("sha256").update(bytes).digest("hex"),
        sqlText: body.sqlText,
        body: JSON.stringify(body),
        milliseconds: Date.now(),
      });
      let result = cache.get(requestId);
      if (!result || scenario === "lost-success-no-dedupe") {
        effects++;
        result = {
          success: true,
          data: {
            queryId: "11111111-1111-4111-8111-111111111111",
            statementTypeId: 4096,
            version: 1,
            rowtype: [{ name: "RESULT", type: "text", length: 32, nullable: false }],
            rowset: [["synthetic-success"]],
            total: 1,
            returned: 1,
            parameters: [],
          },
        };
        cache.set(requestId, result);
      }
      report.simulatedServerSuccesses = effects;
      if (requests.length === 1 || scenario === "persistent-loss") {
        req.socket.destroy();
        return;
      }
      return response(res, result);
    }
    if (url.pathname === "/session" || url.pathname === "/telemetry/send")
      return response(res, { success: true, data: {} });
    report.unexpectedEndpoint = url.pathname;
    res.writeHead(400);
    res.end("{}");
  } catch (error) {
    report.serverError = error.name;
    res.destroy();
  }
});
server.on("connection", (socket) => {
  sockets.add(socket);
  socket.on("close", () => sockets.delete(socket));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;
// Deny every actual outbound socket except the owned listener, even if a future
// SDK import adds an undocumented endpoint. This does not replace SDK methods.
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
for (const key of Object.keys(process.env)) if (/proxy/i.test(key)) delete process.env[key];
process.env.HOME = root;
process.env.SNOWFLAKE_HOME = join(root, "disabled-home");
process.env.SNOWFLAKE_DISABLE_PLATFORM_DETECTION = "true";
process.env.SF_OCSP_RESPONSE_CACHE_DIR = join(root, "ocsp");
process.env.SNOWFLAKE_CRL_ON_DISK_CACHE_DIR = join(root, "crls");
for (const method of ["log", "info", "warn", "error", "debug", "trace"]) console[method] = () => {};
const require = createRequire(new URL("../package.json", import.meta.url));
const sdk = require("snowflake-sdk");
assert.equal(require("snowflake-sdk/package.json").version, "3.4.1");
sdk.configure({
  logLevel: "OFF",
  additionalLogToConsole: false,
  customLogger: {
    error(message) {
      report.setupDiagnostic = String(message).slice(0, 1500);
    },
    warn() {},
    info() {},
    debug() {},
    trace() {},
  },
});
async function finish(reason) {
  if (reported) return;
  reported = true;
  clearTimeout(watchdog);
  report.finishedBecause = reason;
  report.requests = requests.map(({ body, ...request }) => request);
  report.statementRequestCount = requests.length;
  report.sameRequestId =
    requests.length >= 2 && new Set(requests.map((r) => r.requestId)).size === 1;
  report.sameBody = requests.length >= 2 && new Set(requests.map((r) => r.body)).size === 1;
  report.sequenceIdPresent = requests.some(
    (r) => r.sequenceIdInBody !== null || r.sequenceIdInUrl !== null,
  );
  report.applicationOutcome =
    reason === "success-callback"
      ? "SDK reported statement success; simulator effects are independently recorded"
      : "outcome unknown to client; simulator evidence is not a remote guarantee";
  if (reason === "harness-error" && requests.length === 0)
    report.applicationOutcome = "No statement HTTP submission observed; local setup failed";
  report.ownedArtifacts = await readdir(root, { recursive: true });
  if (scenario !== "missing-config")
    report.configMode = (await stat(join(root, "disabled-config.json"))).mode & 0o777;
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => server.close(resolve));
  // The parent retains this root until it observes actual child close.
  report.listenerClosed = true;
  output(JSON.stringify(report, null, 2) + "\n");
  process.exit(reason === "harness-error" ? 1 : 0);
}
watchdog = setTimeout(
  () => {
    void finish("bounded-no-terminal");
  },
  scenario === "persistent-loss" ? 6500 : 15000,
);
try {
  const options = {
    account: "synthetic",
    host: "127.0.0.1",
    port,
    protocol: "http",
    accessUrl: `http://127.0.0.1:${port}`,
    username: "SYNTHETIC",
    authenticator: "OAUTH",
    token: "SYNTHETIC_OAUTH",
    clientConfigFile: join(root, "disabled-config.json"),
    clientSessionKeepAlive: false,
    clientRequestMFAToken: false,
    clientStoreTemporaryCredential: false,
    keepAlive: false,
    timeout: 500,
  };
  if (scenario === "documented-options")
    Object.assign(options, { retryTimeout: 0, sfRetryMaxLoginRetries: 0 });
  if (scenario === "unsupported-options")
    Object.assign(options, {
      sfRetryMaxNumRetries: 0,
      sfRetryStartingSleepTime: 0,
      sfRetryMaxSleepTime: 0,
    });
  report.optionOverrides =
    scenario === "documented-options"
      ? { retryTimeout: 0, sfRetryMaxLoginRetries: 0 }
      : scenario === "unsupported-options"
        ? { sfRetryMaxNumRetries: 0, sfRetryStartingSleepTime: 0, sfRetryMaxSleepTime: 0 }
        : {};
  connection = sdk.createConnection(options);
  await new Promise((resolve, reject) =>
    connection.connect((error) => (error ? reject(error) : resolve())),
  );
  report.connectedToOwnedFakeServer = true;
  const statement = connection.execute({
    sqlText: "SELECT 'SYNTHETIC_LOCAL_ONLY'",
    streamResult: true,
    complete(error, completed) {
      report.statementCallbacks++;
      report.callback = error
        ? { ok: false, code: String(error.code), sqlState: error.sqlState ?? null }
        : { ok: true, queryId: completed.getQueryId(), requestId: completed.getRequestId() };
      if (error) return void finish("error-callback");
      const stream = completed.streamRows();
      let count = 0;
      stream.on("data", (row) => {
        assert.equal(row.RESULT, "synthetic-success");
        count++;
      });
      stream.on("error", () => {
        void finish("harness-error");
      });
      stream.on("end", () => {
        report.streamRows = count;
        void finish("success-callback");
      });
    },
  });
  report.sdkGeneratedRequestId = statement.getRequestId();
} catch (error) {
  report.harnessError = { name: error.name, code: String(error.code) };
  await finish("harness-error");
}
