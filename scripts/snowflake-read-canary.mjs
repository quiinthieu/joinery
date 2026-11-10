import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  chmodSync,
  closeSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Decoder, encode } from "../snowflake-helper/src/framing.mjs";
import { validateSnowflakeResourceTree } from "./native-smoke-resources.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const IDENTIFIER = /^[A-Z_][A-Z0-9_]{0,254}$/;
export const STATEMENTS = Object.freeze([
  "SELECT CURRENT_USER() AS CANARY_USER, CURRENT_ROLE() AS CANARY_ROLE, CURRENT_DATABASE() AS CANARY_DATABASE, CURRENT_SCHEMA() AS CANARY_SCHEMA, CURRENT_WAREHOUSE() AS CANARY_WAREHOUSE, CURRENT_SESSION() AS CANARY_SESSION",
  "SELECT CURRENT_USER() AS CANARY_USER, CURRENT_SESSION() AS CANARY_SESSION, CAST(42 AS NUMBER(10,0)) AS CANARY_INTEGER, CAST('joinery-read-canary' AS VARCHAR) AS CANARY_TEXT, CAST(TRUE AS BOOLEAN) AS CANARY_BOOLEAN, CAST(NULL AS VARCHAR) AS CANARY_NULL",
]);
const BOUNDS = Object.freeze({
  helloMs: 2_000,
  authMs: 120_000,
  queryMs: 30_000,
  closeMs: 2_000,
  exitMs: 2_000,
  killMs: 3_000,
});
const fail = (code) => Object.assign(new Error(code), { safeCode: code });
const exactKeys = (value, keys) =>
  value &&
  typeof value === "object" &&
  !Array.isArray(value) &&
  Object.keys(value).length === keys.length &&
  keys.every((key) => Object.hasOwn(value, key));
const text = (value, limit = 256) =>
  typeof value === "string" &&
  value.length > 0 &&
  Buffer.byteLength(value) <= limit &&
  !/[\x00-\x1f\x7f]/.test(value);

export function validateContext(context) {
  if (
    !exactKeys(context, ["profile", "source", "connection", "scope"]) ||
    ![context.profile, context.source, context.scope].every((value) => text(value, 2048)) ||
    !exactKeys(context.connection, [
      "account",
      "user",
      "database",
      "schema",
      "warehouse",
      "role",
      "authenticator",
    ])
  )
    throw fail("invalidContext");
  const c = context.connection;
  if (
    !text(c.account, 180) ||
    !/^[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]$/.test(c.account) ||
    c.account.includes("..") ||
    c.account.endsWith(".snowflakecomputing.com") ||
    !text(c.user) ||
    !/^[A-Za-z0-9_@.+-]+$/.test(c.user) ||
    c.authenticator !== "externalbrowser" ||
    !["database", "schema", "warehouse", "role"].every(
      (key) => text(c[key]) && IDENTIFIER.test(c[key]),
    )
  )
    throw fail("invalidContext");
  return Object.freeze({
    account: c.account,
    host: `${c.account}.snowflakecomputing.com`,
    username: c.user,
    database: c.database,
    schema: c.schema,
    warehouse: c.warehouse,
    role: c.role,
  });
}

export function makePlan(profile, runtime) {
  return {
    mode: "plan",
    network: false,
    authentication: false,
    sqlSubmitted: false,
    runtime: ".cache/snowflake-runtime/macos-arm64",
    runtimeManifestSha256: runtime.manifestSha256,
    contextSha256: createHash("sha256").update(JSON.stringify(profile)).digest("hex"),
    statements: [...STATEMENTS],
    maximumHelperQueryRequests: 2,
    maximumRowsPerStatement: 1,
    maximumColumnsPerStatement: 6,
    maximumResultBytesPerStatement: 8192,
    timeoutsMs: BOUNDS,
    browser:
      "--run can open the system browser for EXTERNALBROWSER SSO; do not share its URL or tokens.",
    billing:
      "SELECT may use or automatically resume the configured warehouse and incur Snowflake charges. No suspension, zero-cost or billing-cap guarantee is established. Obtain explicit approval before --run, including possible warehouse resume/billing.",
    exclusions: ["object reads", "writes", "DDL", "USE", "runner retries", "warehouse management"],
    sdkCaveat:
      "The original SDK can retransmit requests after a lost response; this runner does not change that behavior or prove server deduplication.",
    connectionSetup:
      "The official SDK performs authentication and session setup separately from these two helper query requests. Two is a runner query count, not a bound on physical SQL/HTTP submissions or SDK setup work.",
    identity:
      "The SDK authenticates using the configured login. This canary checks bounded nonempty CURRENT_USER() and its continuity, not an independent binding between that user NAME and the configured LOGIN_NAME.",
    execution: "node scripts/snowflake-read-canary.mjs --run (only after explicit approval)",
  };
}

function regularBytes(filename, maximum) {
  const stat = lstatSync(filename);
  if (
    !stat.isFile() ||
    stat.isSymbolicLink() ||
    stat.size > maximum ||
    realpathSync(filename) !== filename
  )
    throw fail("unsafeLocalFile");
  return readFileSync(filename);
}

function privateDirectory(directory) {
  try {
    mkdirSync(directory, { mode: 0o700 });
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
  }
  const stat = lstatSync(directory);
  if (
    !stat.isDirectory() ||
    stat.isSymbolicLink() ||
    realpathSync(directory) !== directory ||
    (stat.mode & 0o7777) !== 0o700 ||
    (process.getuid && stat.uid !== process.getuid())
  )
    throw fail("unsafeStateDirectory");
}

function deadline(promise, milliseconds, code) {
  let timer;
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(fail(code)), milliseconds);
    }),
  ]).finally(() => clearTimeout(timer));
}

// Programmatic seams are for synthetic tests only; the CLI exposes no path, SQL,
// executable, environment, timeout or connection override.
export async function exerciseChild(child, profile, { signal, bounds = BOUNDS } = {}) {
  const generation = randomUUID();
  const decoder = new Decoder();
  let id = 0,
    pending,
    helloResolve,
    helloReject,
    closed = false,
    bytes = 0,
    helloSeen = false,
    fatalCode;
  const hello = new Promise((resolve, reject) => {
    helloResolve = resolve;
    helloReject = reject;
  });
  // A child can fail before the caller starts waiting for hello.
  hello.catch(() => {});
  const exited = new Promise((resolve) =>
    child.once("close", (code, terminationSignal) => {
      closed = true;
      try {
        decoder.end();
      } catch {
        rejectAll("invalidProtocol");
      }
      if (pending || !helloSeen) rejectAll("childClosed");
      resolve({ code, signal: terminationSignal });
    }),
  );
  function rejectAll(code) {
    fatalCode ??= code;
    helloReject(fail(code));
    pending?.reject(fail(code));
  }
  const abort = () => rejectAll("cancelled");
  signal?.addEventListener("abort", abort, { once: true });
  child.once("error", () => rejectAll("childStartFailed"));
  child.stdin.on("error", () => rejectAll("childWriteFailed"));
  // Dependency output is neither retained nor rendered.
  child.stderr.on("data", () => {});
  child.stdout.on("data", (chunk) => {
    try {
      bytes += chunk.length;
      if (bytes > 2 * 1024 * 1024) throw fail("invalidProtocol");
      for (const frame of decoder.push(chunk)) {
        if (frame?.type === "hello") {
          if (
            helloSeen ||
            id !== 0 ||
            frame.v !== 1 ||
            frame.protocol !== "joinery-snowflake" ||
            frame.sdkVersion !== "3.4.1"
          )
            throw fail("invalidProtocol");
          helloSeen = true;
          helloResolve();
          continue;
        }
        if (
          !pending ||
          frame?.v !== 1 ||
          frame.generation !== generation ||
          frame.id !== pending.id
        )
          throw fail("invalidProtocol");
        if (frame.type === "columns") {
          if (pending.columns || !Array.isArray(frame.columns) || frame.columns.length > 6)
            throw fail("invalidProtocol");
          pending.columns = frame.columns;
        } else if (frame.type === "rows") {
          if (!pending.columns || !Array.isArray(frame.rows)) throw fail("invalidProtocol");
          pending.rows.push(...frame.rows);
          if (pending.rows.length > 1 || Buffer.byteLength(JSON.stringify(pending.rows)) > 8192)
            throw fail("invalidProtocol");
        } else if (frame.type === "terminal") {
          if (frame.ok !== true) pending.reject(fail("helperRejected"));
          else
            pending.resolve({ result: frame.result, columns: pending.columns, rows: pending.rows });
          pending = undefined;
        } else if (!["accepted", "phase", "statement"].includes(frame.type))
          throw fail("invalidProtocol");
      }
    } catch {
      rejectAll("invalidProtocol");
    }
  });
  async function request(op, payload, timeout) {
    if (signal?.aborted) throw fail("cancelled");
    if (fatalCode) throw fail(fatalCode);
    const currentId = String(++id);
    if (op === "query") outcome.helperQueryRequestsSent += 1;
    const response = new Promise((resolve, reject) => {
      pending = { id: currentId, resolve, reject, rows: [] };
    });
    const bounded = deadline(
      response,
      timeout,
      op === "connect" ? "authenticationTimeout" : "operationTimeout",
    );
    try {
      child.stdin.write(encode({ v: 1, generation, id: currentId, op, payload }));
    } catch {
      rejectAll("childWriteFailed");
    }
    return bounded;
  }
  let outcome = {
    passed: false,
    failure: "qualificationFailed",
    helperQueryRequestsSent: 0,
    checks: {},
  };
  try {
    if (signal?.aborted) throw fail("cancelled");
    await deadline(hello, bounds.helloMs, "helloTimeout");
    const connection = await request("connect", profile, bounds.authMs);
    if (connection.result?.connected !== true) throw fail("invalidProtocol");
    outcome.checks.connected = true;
    const answers = [];
    for (const sql of STATEMENTS) {
      const answer = await request(
        "query",
        { sql, binds: [], classification: "read", credits: 1, maxRows: 1, maxBytes: 8192 },
        bounds.queryMs,
      );
      if (
        answer.result?.rowsSeen !== 1 ||
        answer.result?.rowsRetained !== 1 ||
        answer.result?.truncated !== false ||
        answer.rows.length !== 1
      )
        throw fail("resultMismatch");
      answers.push(answer);
    }
    validateAnswers(answers, profile);
    outcome.checks.context = true;
    outcome.checks.sessionContinuity = true;
    outcome.checks.principalContinuity = true;
    outcome.checks.scalarTypesAndValues = true;
    const close = await request("close", {}, bounds.closeMs);
    if (fatalCode) throw fail(fatalCode);
    outcome.checks.logoutAcknowledged = close.result?.logoutAcknowledged === true;
    if (close.result?.closed !== true || !outcome.checks.logoutAcknowledged)
      throw fail("logoutUnconfirmed");
    outcome.passed = true;
    delete outcome.failure;
  } catch (error) {
    outcome.failure = error.safeCode ?? "qualificationFailed";
  } finally {
    signal?.removeEventListener("abort", abort);
    try {
      child.stdin.end();
    } catch {}
    let exit;
    const kill = (terminationSignal) => {
      try {
        child.kill(terminationSignal);
      } catch {}
    };
    try {
      exit = await deadline(exited, bounds.exitMs, "exitTimeout");
    } catch {
      kill("SIGTERM");
      try {
        exit = await deadline(exited, bounds.closeMs, "exitTimeout");
      } catch {
        kill("SIGKILL");
        try {
          exit = await deadline(exited, bounds.killMs, "reapTimeout");
        } catch {}
      }
    }
    outcome.checks.childReaped = closed;
    outcome.checks.childExitClean = closed && exit?.code === 0 && exit?.signal === null;
    if (outcome.passed && (!outcome.checks.childExitClean || fatalCode)) {
      outcome.passed = false;
      outcome.failure = fatalCode ?? "childExitUnclean";
    }
    if (!closed) {
      outcome.passed = false;
      outcome.failure = "reapUnconfirmed";
      child.unref();
      child.stdin.destroy();
      child.stdout.destroy();
      child.stderr.destroy();
    }
  }
  return outcome;
}

function validateAnswers(answers, profile) {
  const [context, scalar] = answers;
  const expectedNames = [
    [
      "CANARY_USER",
      "CANARY_ROLE",
      "CANARY_DATABASE",
      "CANARY_SCHEMA",
      "CANARY_WAREHOUSE",
      "CANARY_SESSION",
    ],
    [
      "CANARY_USER",
      "CANARY_SESSION",
      "CANARY_INTEGER",
      "CANARY_TEXT",
      "CANARY_BOOLEAN",
      "CANARY_NULL",
    ],
  ];
  const expectedTypes = [
    ["text", "text", "text", "text", "text", "fixed"],
    ["text", "fixed", "fixed", "text", "boolean", "text"],
  ];
  for (const [i, answer] of answers.entries()) {
    if (
      JSON.stringify(answer.columns?.map((c) => c.name)) !== JSON.stringify(expectedNames[i]) ||
      JSON.stringify(answer.columns?.map((c) => c.type)) !== JSON.stringify(expectedTypes[i]) ||
      answer.rows[0].length !== expectedNames[i].length
    )
      throw fail("resultMismatch");
  }
  const row = context.rows[0];
  if (
    !text(row[0]) ||
    JSON.stringify(row.slice(1, 5)) !==
      JSON.stringify([profile.role, profile.database, profile.schema, profile.warehouse]) ||
    typeof row[5] !== "string" ||
    !/^[0-9]{1,38}$/.test(row[5]) ||
    JSON.stringify(scalar.rows[0]) !==
      JSON.stringify([row[0], row[5], "42", "joinery-read-canary", "TRUE", null])
  )
    throw fail("resultMismatch");
}

export async function main(
  args = process.argv.slice(2),
  {
    repoRoot = repository,
    spawnChild = spawn,
    validateRuntime = validateSnowflakeResourceTree,
    signal,
  } = {},
) {
  if (args.length > 1 || (args.length === 1 && args[0] !== "--run")) throw fail("invalidArguments");
  const repo = realpathSync(repoRoot);
  const cache = path.join(repo, ".cache");
  if (realpathSync(cache) !== cache || !lstatSync(cache).isDirectory())
    throw fail("unsafeLocalFile");
  const profile = validateContext(
    JSON.parse(regularBytes(path.join(cache, "owner-snowflake-context.json"), 16_384)),
  );
  const runtime = validateRuntime({ repoRoot: repo });
  const plan = makePlan(profile, runtime);
  if (args.length === 0) return plan;
  if (process.platform !== "darwin" || process.arch !== "arm64") throw fail("unsupportedPlatform");
  const evidenceRoot = path.join(cache, "snowflake-read-canary");
  privateDirectory(evidenceRoot);
  const root = mkdtempSync(path.join(evidenceRoot, "auth-"));
  let authority, identity;
  let outcome = { passed: false, failure: "childStartFailed", checks: { childReaped: true } };
  let started = false;
  try {
    chmodSync(root, 0o700);
    authority = openSync(root, "r");
    identity = fstatSync(authority);
    const stage = path.join(repo, ".cache/snowflake-runtime/macos-arm64");
    const child = spawnChild(path.join(stage, "bin/node"), [path.join(stage, "helper/main.mjs")], {
      cwd: stage,
      env: { JOINERY_SNOWFLAKE_STATE_DIR: root },
      stdio: ["pipe", "pipe", "pipe"],
      shell: false,
    });
    started = true;
    outcome = await exerciseChild(child, profile, { signal });
  } catch {
    outcome = { passed: false, failure: "localRunnerFailure", checks: { childReaped: !started } };
  } finally {
    outcome.checks.stateRemoved = false;
    if (outcome.checks.childReaped) {
      try {
        const current = lstatSync(root);
        if (
          !identity ||
          current.isSymbolicLink() ||
          current.dev !== identity.dev ||
          current.ino !== identity.ino
        )
          throw fail("stateIdentityChanged");
        rmSync(root, { recursive: true, force: false });
        outcome.checks.stateRemoved = true;
      } catch {
        outcome.passed = false;
        outcome.failure = "stateCleanupUnconfirmed";
      }
    }
    if (authority !== undefined) closeSync(authority);
  }
  const evidence = {
    format: "joinery-snowflake-read-canary-v1",
    at: new Date().toISOString(),
    runtimeManifestSha256: plan.runtimeManifestSha256,
    contextSha256: plan.contextSha256,
    helperQueriesPlanned: 2,
    ...outcome,
  };
  const evidenceFile = path.join(evidenceRoot, `${randomUUID()}.json`);
  writeFileSync(evidenceFile, `${JSON.stringify(evidence, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  return { ...evidence, evidence: path.relative(repo, evidenceFile) };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  const cancel = () => controller.abort();
  process.once("SIGINT", cancel);
  process.once("SIGTERM", cancel);
  try {
    const result = await main(undefined, { signal: controller.signal });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    if (result.passed === false) process.exitCode = 1;
  } catch (error) {
    process.stderr.write(
      `${JSON.stringify({ passed: false, failure: error.safeCode ?? "localValidationFailed" })}\n`,
    );
    process.exitCode = 1;
  } finally {
    process.removeListener("SIGINT", cancel);
    process.removeListener("SIGTERM", cancel);
  }
}
