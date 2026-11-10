// Real SDK construction only. Never authenticates, executes SQL, or opens a browser.
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import tls from "node:tls";
import http from "node:http";
import https from "node:https";
import childProcess from "node:child_process";
import { syncBuiltinESMExports } from "node:module";
import { join } from "node:path";
import { connectionOptions } from "../src/helper.mjs";
let networkAttempts = 0,
  browserAttempts = 0,
  privateReads = 0;
const denyNetwork = () => {
  networkAttempts++;
  throw Error("network forbidden in construction test");
};
net.Socket.prototype.connect = denyNetwork;
tls.connect = denyNetwork;
http.request = http.get = https.request = https.get = denyNetwork;
for (const name of ["spawn", "exec", "execFile", "spawnSync", "execSync", "execFileSync"])
  childProcess[name] = () => {
    browserAttempts++;
    throw Error("child process forbidden");
  };
for (const name of [
  "readFileSync",
  "readFile",
  "realpathSync",
  "existsSync",
  "statSync",
  "accessSync",
]) {
  const original = fs[name];
  fs[name] = function (path, ...args) {
    if (
      typeof path === "string" &&
      /(?:[/\\]\.snowflake(?:[/\\]|$)|connections\.toml|sf_client_config\.json|token_cache)/.test(
        path,
      )
    ) {
      privateReads++;
      throw Error("private profile lookup forbidden");
    }
    return original.call(this, path, ...args);
  };
}
syncBuiltinESMExports();
process.env.SNOWFLAKE_DISABLE_PLATFORM_DETECTION = "true";
process.env.SF_OCSP_RESPONSE_CACHE_DIR = join(process.cwd(), "ocsp");
process.env.SNOWFLAKE_CRL_ON_DISK_CACHE_DIR = join(process.cwd(), "crls");
const sdk = (await import("snowflake-sdk")).default;
sdk.configure({
  logLevel: "OFF",
  additionalLogToConsole: false,
  customLogger: { error() {}, warn() {}, info() {}, debug() {}, trace() {} },
});
const profile = {
  account: "synthetic-account",
  host: "synthetic-account.snowflakecomputing.com",
  username: "SYNTHETIC_LOGIN",
  role: "SYNTHETIC_ROLE",
  warehouse: "SYNTHETIC_WH",
  database: "SYNTHETIC_DB",
  schema: "PUBLIC",
};
const options = connectionOptions(profile, join(process.cwd(), "disabled-client-config.json"));
const connection = sdk.createConnection(options);
assert.equal(typeof connection.connectAsync, "function");
assert.equal(typeof connection.execute, "function");
// No connect/connectAsync/execute call: close a never-authenticated SDK object only.
const closed = await Promise.race([
  new Promise((resolve) =>
    connection.destroy((error) => resolve({ callback: true, error: Boolean(error) })),
  ),
  new Promise((_, reject) =>
    setTimeout(() => reject(Error("pre-auth destroy timeout")), 1500).unref(),
  ),
]);
assert.equal(closed.callback, true);
assert.equal(networkAttempts, 0);
assert.equal(browserAttempts, 0);
assert.equal(privateReads, 0);
process.stdout.write(
  JSON.stringify({
    constructed: true,
    destroyCallback: true,
    networkAttempts,
    browserAttempts,
    privateReads,
  }),
);
