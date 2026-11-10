import { closeSync, fchownSync, lstatSync, openSync, writeFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { Decoder, writer } from "./framing.mjs";
import { Helper } from "./helper.mjs";

// Capture protocol writer before silencing all dependency console/stdio output.
// SDK exceptions are never rendered with messages/stacks containing SQL or tokens.
const rawWrite = process.stdout.write.bind(process.stdout);
process.stdout.write = (_chunk, ...args) => {
  args.find((x) => typeof x === "function")?.();
  return true;
};
process.stderr.write = (_chunk, ...args) => {
  args.find((x) => typeof x === "function")?.();
  return true;
};
for (const method of ["log", "info", "warn", "error", "debug", "trace"]) console[method] = () => {};
let exiting = false;
const exit = (code = 0) => {
  if (exiting) return;
  exiting = true;
  process.exit(code);
};
process.on("uncaughtException", () => exit(2));
process.on("unhandledRejection", () => exit(2));
process.on("SIGTERM", () => exit(0));
process.on("SIGINT", () => exit(0));
if (process.argv.length !== 2) exit(2);
// Rust owns this private root until it has actually reaped this child. In
// particular SIGKILL must not leave untracked certificate/cache directories.
const root = process.env.JOINERY_SNOWFLAKE_STATE_DIR;
try {
  if (!root || !isAbsolute(root)) exit(2);
  const state = lstatSync(root);
  if (
    !state.isDirectory() ||
    state.isSymbolicLink() ||
    (state.mode & 0o777) !== 0o700 ||
    (process.getuid && state.uid !== process.getuid())
  )
    exit(2);
} catch {
  exit(2);
}
// Pinned SDK opens an explicitly supplied configuration path during connect.
// Supply an owned configuration instead of falling back to user defaults.
const clientConfigFile = join(root, "client-config.json");
const configHandle = openSync(clientConfigFile, "wx", 0o600);
try {
  // SDK validates the primary gid as well as uid; macOS temporary directories
  // can otherwise inherit another group even though their permissions are0700.
  if (process.getuid && process.getgid)
    fchownSync(configHandle, process.getuid(), process.getgid());
  writeFileSync(configHandle, JSON.stringify({ common: { log_level: "OFF", log_path: root } }));
} finally {
  closeSync(configHandle);
}
// Pinned SDK otherwise probes cloud metadata endpoints at module import time.
// Child-local ephemeral home also prevents os.homedir() falling back to owner state.
process.env.HOME = root;
process.env.SNOWFLAKE_HOME = join(root, "disabled-snowflake-home");
process.env.SNOWFLAKE_DISABLE_PLATFORM_DETECTION = "true";
// Certificate-validation caches are isolated; never read/write the owner home.
process.env.SF_OCSP_RESPONSE_CACHE_DIR = join(root, "ocsp");
process.env.SNOWFLAKE_CRL_ON_DISK_CACHE_DIR = join(root, "crls");
const sdk = (await import("snowflake-sdk")).default;
// An explicit custom logger avoids file transports, even before a connection.
sdk.configure({
  logLevel: "OFF",
  additionalLogToConsole: false,
  customLogger: { error() {}, warn() {}, info() {}, debug() {}, trace() {} },
});
// Explicit owned config path prevents SDK default/env/home profile lookup.
const output = writer(rawWrite, () => exit(2));
const helper = new Helper(
  sdk,
  output,
  () => {
    // A final frame queued before retirement must flush before exit, bounded locally.
    const deadline = setTimeout(() => exit(0), 250);
    output.flush().then(() => {
      clearTimeout(deadline);
      exit(0);
    });
  },
  { clientConfigFile },
);
const decoder = new Decoder();
process.stdin.on("data", (chunk) => {
  try {
    for (const request of decoder.push(chunk)) helper.receive(request);
  } catch {
    helper.fatal();
  }
});
process.stdin.on("end", () => {
  try {
    decoder.end();
  } catch {}
  helper.stop();
});
