import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Startup qualification only. Empty stdin never submits an authentication or SQL request.
const repository = fileURLToPath(new URL("../", import.meta.url));
const runtime = path.join(repository, ".cache/snowflake-runtime/macos-arm64");
const manifest = JSON.parse(readFileSync(path.join(runtime, "manifest.json"), "utf8"));
if (process.platform !== "darwin" || process.arch !== "arm64" || manifest.target !== "darwin-arm64")
  throw new Error("The staged Snowflake runtime check requires macOS arm64");
const executable = path.join(runtime, "bin/node");
const sha256 = createHash("sha256").update(readFileSync(executable)).digest("hex");
if (sha256 !== manifest.runtime.sha256) throw new Error("The staged Node hash changed");
const expectedFiles = ["framing.mjs", "helper.mjs", "main.mjs"];
if (
  JSON.stringify(Object.keys(manifest.helper.sourceSha256 ?? {}).sort()) !==
  JSON.stringify(expectedFiles)
)
  throw new Error("The helper manifest must contain exactly all three source hashes");
const sdk = JSON.parse(
  readFileSync(path.join(runtime, "helper/node_modules/snowflake-sdk/package.json"), "utf8"),
);
if (
  sdk.name !== "snowflake-sdk" ||
  sdk.version !== "3.4.1" ||
  manifest.helper.sdkVersion !== sdk.version
)
  throw new Error("The installed SDK package version did not match");
for (const [filename, expected] of Object.entries(manifest.helper.sourceSha256)) {
  const actual = createHash("sha256")
    .update(readFileSync(path.join(runtime, "helper", filename)))
    .digest("hex");
  if (actual !== expected) throw new Error("The staged helper hash changed");
  const source = createHash("sha256")
    .update(readFileSync(path.join(repository, "snowflake-helper/src", filename)))
    .digest("hex");
  if (actual !== source) throw new Error("The staged helper does not match this checkout");
}

const root = mkdtempSync(path.join(tmpdir(), "joinery-snowflake-startup-"));
chmodSync(root, 0o700);
try {
  const networkAttempt = path.join(root, "network-attempted");
  const guard = path.join(root, "deny-network.cjs");
  writeFileSync(
    guard,
    `
const fs = require('node:fs');
function deny() {
  fs.writeFileSync(${JSON.stringify(networkAttempt)}, 'blocked');
  throw new Error('Network access is disabled during staged startup verification');
}
require('node:net').Socket.prototype.connect = deny;
require('node:dgram').Socket.prototype.send = deny;
const dns = require('node:dns');
for (const api of [dns, dns.promises, dns.Resolver.prototype, dns.promises.Resolver.prototype]) {
  for (const key of Object.getOwnPropertyNames(api)) {
    if (typeof api[key] === 'function' &&
        (key === 'lookup' || key === 'lookupService' || key === 'reverse' || key.startsWith('resolve')))
      api[key] = deny;
  }
}
`,
    { mode: 0o600 },
  );
  // spawnSync returns after the child has closed, including timeout/SIGKILL.
  // Only then can this parent remove its exact owned temporary root.
  const child = spawnSync(executable, ["--require", guard, path.join(runtime, "helper/main.mjs")], {
    cwd: runtime,
    env: { JOINERY_SNOWFLAKE_STATE_DIR: root },
    input: Buffer.alloc(0),
    timeout: 15_000,
    killSignal: "SIGKILL",
    maxBuffer: 1_048_580,
  });
  if (child.error || child.signal || child.status !== 0)
    throw new Error("The staged helper did not start and exit cleanly");
  if (existsSync(networkAttempt)) throw new Error("The staged helper attempted network access");
  const bytes = child.stdout;
  if (!Buffer.isBuffer(bytes) || bytes.length < 4 || bytes.readUInt32BE(0) !== bytes.length - 4)
    throw new Error("The staged helper did not emit exactly one bounded hello frame");
  const hello = JSON.parse(bytes.subarray(4).toString("utf8"));
  if (
    hello.v !== 1 ||
    hello.type !== "hello" ||
    hello.protocol !== "joinery-snowflake" ||
    hello.sdkVersion !== "3.4.1"
  )
    throw new Error("The staged helper protocol or SDK version did not match");
  process.stdout.write(
    JSON.stringify({
      passed: true,
      check: "staged-helper-startup-with-node-network-guard",
      sdkVersion: sdk.version,
      nodeSha256: sha256,
      authenticationRequests: 0,
      sqlRequests: 0,
    }) + "\n",
  );
} finally {
  rmSync(root, { recursive: true, force: false });
}
