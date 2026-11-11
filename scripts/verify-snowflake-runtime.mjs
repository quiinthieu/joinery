import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { NODE_BINARY_SHA256, NODE_VERSION } from "./prepare-snowflake-runtime.mjs";
import { verifyTransportSources } from "../snowflake-helper/src/transport-source-pins.mjs";

// Startup qualification only. Empty stdin never submits an authentication or SQL request.
const repository = fileURLToPath(new URL("../", import.meta.url));
const scriptPath = fileURLToPath(import.meta.url);

export function selectRuntime(args, repoRoot = repository) {
  if (args.length === 0)
    return { kind: "stage", runtime: path.join(repoRoot, ".cache/snowflake-runtime/macos-arm64") };
  if (
    args.length !== 2 ||
    args[0] !== "--bundle" ||
    typeof args[1] !== "string" ||
    !args[1].trim() ||
    args[1].includes("\0") ||
    args[1].startsWith("--") ||
    path.extname(path.resolve(args[1])) !== ".app"
  )
    throw new Error("Usage: verify-snowflake-runtime.mjs [--bundle <path.app>]");
  return {
    kind: "bundle",
    runtime: path.join(path.resolve(args[1]), "Contents/Resources/snowflake-runtime"),
  };
}

// Programmatic seams support private fixtures; the CLI accepts only selectRuntime's arguments.
export function verifySnowflakeRuntime({
  args = [],
  repoRoot = repository,
  platform = process.platform,
  arch = process.arch,
  runChild = spawnSync,
  verifyTransport = verifyTransportSources,
} = {}) {
  const { kind, runtime } = selectRuntime(args, repoRoot);
  if (platform !== "darwin" || arch !== "arm64")
    throw new Error("The Snowflake runtime check requires macOS arm64");
  const manifestBytes = readFileSync(path.join(runtime, "manifest.json"));
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  if (
    manifest.format !== "joinery-snowflake-runtime-v1" ||
    manifest.target !== "darwin-arm64" ||
    manifest.runtime?.version !== NODE_VERSION ||
    manifest.runtime.platform !== "darwin" ||
    manifest.runtime.arch !== "arm64" ||
    manifest.runtime.sha256 !== NODE_BINARY_SHA256
  )
    throw new Error("The runtime manifest did not match the pinned Node release");
  const executable = path.join(runtime, "bin/node");
  const sha256 = createHash("sha256").update(readFileSync(executable)).digest("hex");
  if (sha256 !== NODE_BINARY_SHA256)
    throw new Error("The Node hash did not match the pinned binary");
  const expectedFiles = [
    "disposable-setup-ledger.mjs",
    "framing.mjs",
    "helper.mjs",
    "main.mjs",
    "query-submit-guard.mjs",
    "transport-source-pins.mjs",
  ];
  if (
    JSON.stringify(Object.keys(manifest.helper.sourceSha256 ?? {}).sort()) !==
    JSON.stringify(expectedFiles)
  )
    throw new Error("The helper manifest must contain exactly the reviewed source hashes");
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
    if (actual !== expected) throw new Error("The selected helper hash changed");
    const source = createHash("sha256")
      .update(readFileSync(path.join(repoRoot, "snowflake-helper/src", filename)))
      .digest("hex");
    if (actual !== source) throw new Error("The selected helper does not match this checkout");
  }

  verifyTransport(pathToFileURL(path.join(runtime, "helper/main.mjs")));
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
  throw new Error('Network access is disabled during startup verification');
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
    const child = runChild(
      executable,
      ["--require", guard, path.join(runtime, "helper/main.mjs")],
      {
        cwd: runtime,
        env: { JOINERY_SNOWFLAKE_STATE_DIR: root },
        input: Buffer.alloc(0),
        timeout: 15_000,
        killSignal: "SIGKILL",
        maxBuffer: 1_048_580,
      },
    );
    if (child.error || child.signal || child.status !== 0)
      throw new Error("The selected helper did not start and exit cleanly");
    if (existsSync(networkAttempt)) throw new Error("The selected helper attempted network access");
    const bytes = child.stdout;
    if (!Buffer.isBuffer(bytes) || bytes.length < 4 || bytes.readUInt32BE(0) !== bytes.length - 4)
      throw new Error("The selected helper did not emit exactly one bounded hello frame");
    const hello = JSON.parse(bytes.subarray(4).toString("utf8"));
    if (
      hello.v !== 1 ||
      hello.type !== "hello" ||
      hello.protocol !== "joinery-snowflake" ||
      hello.sdkVersion !== "3.4.1"
    )
      throw new Error("The selected helper protocol or SDK version did not match");
    return {
      passed: true,
      check: `${kind === "bundle" ? "bundle" : "staged"}-helper-startup-with-node-network-guard`,
      resourceKind: kind,
      sdkVersion: sdk.version,
      nodeSha256: sha256,
      manifestSha256: createHash("sha256").update(manifestBytes).digest("hex"),
      authenticationRequests: 0,
      sqlRequests: 0,
    };
  } finally {
    rmSync(root, { recursive: true, force: false });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    process.stdout.write(
      `${JSON.stringify(verifySnowflakeRuntime({ args: process.argv.slice(2) }))}\n`,
    );
  } catch (error) {
    process.stderr.write(`Snowflake runtime startup verification failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
