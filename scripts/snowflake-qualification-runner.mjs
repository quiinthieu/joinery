// Executable boundary for ONE separately approved helper-direct qualification.
// A v1 manifest remains an offline ledger. It grants no execution authority.
import { createHash, randomUUID } from "node:crypto";
import { spawn } from "node:child_process";
import {
  closeSync,
  constants,
  fstatSync,
  fsyncSync,
  lstatSync,
  mkdtempSync,
  openSync,
  realpathSync,
  rmSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  canonicalJson,
  compileManifest,
  currentEvidence,
  emitExclusive,
  readLocal,
  verifyManifest,
} from "./snowflake-qualification-manifest.mjs";
import { CHILD_LIMITS, exerciseQualificationChild } from "./snowflake-qualification-child.mjs";
import { FRAMING_LIMITS } from "./snowflake-qualification-framing.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const FORMAT = "joinery-snowflake-qualification-execution-plan-v1";
const APPROVAL = "joinery-snowflake-qualification-approval-v1";
export const APPROVAL_SCOPE =
  "one-helper-session-externalbrowser-24-read-statements-schema-names-possible-billing";
export const RUNNER_SOURCES = Object.freeze([
  "scripts/snowflake-qualification-runner.mjs",
  "scripts/snowflake-qualification-child.mjs",
  "scripts/snowflake-qualification-framing.mjs",
  "scripts/snowflake-qualification-protocol.mjs",
  "scripts/snowflake-qualification-results.mjs",
]);
const hash = (value) => createHash("sha256").update(value).digest("hex");
const fail = (code) => Object.assign(new Error(code), { safeCode: code });
function exact(value, keys, code = "invalidInput") {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    ![Object.prototype, null].includes(Object.getPrototypeOf(value)) ||
    Reflect.ownKeys(value).length !== keys.length ||
    !keys.every((key) => {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      return descriptor?.enumerable && Object.hasOwn(descriptor, "value");
    })
  )
    throw fail(code);
}
const freeze = (value) => {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
};
export function compileExecutionPlan(input, runnerSources) {
  const ledger = compileManifest(input);
  exact(runnerSources, RUNNER_SOURCES);
  if (
    !Object.values(runnerSources).every(
      (value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value),
    )
  )
    throw fail("invalidRunnerSources");
  const payload = JSON.parse(
    canonicalJson({
      format: FORMAT,
      ledger,
      runnerSources,
      physicalLimits: { ...FRAMING_LIMITS, ...CHILD_LIMITS },
      approvalScope: APPROVAL_SCOPE,
      maximumAttempts: 1,
    }),
  );
  return freeze({ executionPlanSha256: hash(canonicalJson(payload)), payload });
}
export function verifyExecutionPlan(plan, input, runnerSources) {
  exact(plan, ["executionPlanSha256", "payload"]);
  const expected = compileExecutionPlan(input, runnerSources);
  if (canonicalJson(plan) !== canonicalJson(expected)) throw fail("executionPlanDrift");
  verifyManifest(plan.payload.ledger, input);
  return expected;
}
function readJson(root, filename) {
  return JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(
      readLocal(root, filename, { privateFile: true }),
    ),
  );
}
function runnerEvidence(root) {
  return Object.fromEntries(
    RUNNER_SOURCES.map((file) => [file, hash(readLocal(root, file, { maximum: 1024 * 1024 }))]),
  );
}
function privateDirectory(directory) {
  const stat = lstatSync(directory);
  if (
    !stat.isDirectory() ||
    stat.isSymbolicLink() ||
    realpathSync(directory) !== directory ||
    (stat.mode & 0o7777) !== 0o700 ||
    (process.getuid && stat.uid !== process.getuid())
  )
    throw fail("unsafeOutputDirectory");
  return stat;
}
function syncPrivateDirectory(directory) {
  const before = privateDirectory(directory);
  const fd = openSync(directory, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = fstatSync(fd);
    if (opened.dev !== before.dev || opened.ino !== before.ino) throw fail("outputIdentityChanged");
    fsyncSync(fd);
    const after = privateDirectory(directory);
    if (after.dev !== opened.dev || after.ino !== opened.ino) throw fail("outputIdentityChanged");
  } catch {
    throw fail("consumptionDurabilityUnconfirmed");
  } finally {
    closeSync(fd);
  }
}
function windowOpen(input) {
  const now = Date.now();
  if (now < Date.parse(input.window.notBefore) || now >= Date.parse(input.window.expiresAt))
    throw fail("executionWindowUnavailable");
}
function summary(plan, mode) {
  return {
    mode,
    executionPlanSha256: plan.executionPlanSha256,
    ledgerSha256: plan.payload.ledger.manifestSha256,
    runtimeManifestSha256: plan.payload.ledger.payload.runtime.manifestSha256,
    window: plan.payload.ledger.payload.window,
    route: "helper-direct",
    plannedSqlStatements: 24,
    plannedSessions: 1,
    maximumAttempts: 1,
    physicalLimits: plan.payload.physicalLimits,
    approvalScope: APPROVAL_SCOPE,
    approvalConsumed: false,
    qualificationPassed: false,
  };
}
function argumentsFor(args) {
  const options = {};
  for (let index = 0; index < args.length; index++) {
    const key = args[index];
    if (
      !["--input", "--emit", "--plan", "--approval", "--run"].includes(key) ||
      Object.hasOwn(options, key)
    )
      throw fail("invalidArguments");
    if (key === "--run") options[key] = true;
    else {
      const value = args[++index];
      if (!value || value.startsWith("--")) throw fail("invalidArguments");
      options[key] = value;
    }
  }
  if (
    !options["--input"] ||
    (options["--emit"] && (options["--plan"] || options["--run"] || options["--approval"])) ||
    Boolean(options["--run"]) !== Boolean(options["--approval"]) ||
    (options["--run"] && !options["--plan"])
  )
    throw fail("invalidArguments");
  return options;
}
async function emergencyReap(child, hasExited) {
  child.stdin?.destroy();
  child.stdout?.destroy();
  child.stderr?.destroy();
  // A reaped process can leave inherited pipes open in another process.
  // Close our owned descriptors even when no further signal is appropriate.
  if (hasExited()) return true;
  let timer;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const wait = (milliseconds) =>
    new Promise((resolve) => {
      timer = setTimeout(resolve, milliseconds);
    });
  try {
    child.kill("SIGTERM");
    await Promise.race([exited, wait(2000)]);
    clearTimeout(timer);
    if (!hasExited()) {
      child.kill("SIGKILL");
      await Promise.race([exited, wait(3000)]);
    }
  } catch {
    // Failure to signal is evidence of uncertain local cleanup, never success.
  } finally {
    clearTimeout(timer);
  }
  return hasExited();
}

// Dependency injection is ONLY a synthetic test seam. The CLI has no executable,
// runtime, evidence, timeout, platform or authentication-root override.
export async function main(
  args = process.argv.slice(2),
  {
    repoRoot = repository,
    loadEvidence = currentEvidence,
    loadRunnerSources = runnerEvidence,
    spawnChild = spawn,
    platform = process.platform,
    arch = process.arch,
    signal,
  } = {},
) {
  const options = argumentsFor(args);
  const root = realpathSync(repoRoot);
  const request = readJson(root, options["--input"]);
  exact(request, ["workflow", "route", "runId", "window", "profile"]);
  const input = { ...request, ...loadEvidence(root) };
  const sources = loadRunnerSources(root);
  let plan = compileExecutionPlan(input, sources);
  if (options["--plan"])
    plan = verifyExecutionPlan(readJson(root, options["--plan"]), input, sources);
  if (options["--emit"]) emitExclusive(root, options["--emit"], plan);
  if (!options["--run"])
    return summary(
      plan,
      options["--emit"]
        ? "emittedExecutionPlan"
        : options["--plan"]
          ? "verifiedExecutionPlan"
          : "dryRun",
    );

  if (platform !== "darwin" || arch !== "arm64") throw fail("unsupportedPlatform");
  if (signal?.aborted) throw fail("ownerCancelledBeforeAttempt");
  windowOpen(input);
  const approval = readJson(root, options["--approval"]);
  exact(approval, ["format", "executionPlanSha256", "scope", "maximumAttempts"], "invalidApproval");
  if (
    approval.format !== APPROVAL ||
    approval.executionPlanSha256 !== plan.executionPlanSha256 ||
    approval.scope !== APPROVAL_SCOPE ||
    approval.maximumAttempts !== 1
  )
    throw fail("invalidApproval");
  const output = path.join(root, ".cache/snowflake-qualification");
  privateDirectory(output);
  // Revalidate all current bytes immediately before consuming one-use authority.
  verifyExecutionPlan(plan, { ...request, ...loadEvidence(root) }, loadRunnerSources(root));
  windowOpen(input);
  const id = plan.executionPlanSha256;
  try {
    lstatSync(path.join(output, `result-${id}.json`));
    throw fail("evidenceAlreadyExists");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  // Exclusive, deterministic marker makes every copy of the same grant one-use.
  // Never remove this marker, including on spawn/authentication/local failures.
  emitExclusive(root, path.join(output, `consumed-${id}.json`), {
    format: "joinery-snowflake-qualification-consumption-v1",
    executionPlanSha256: id,
    at: new Date().toISOString(),
    scope: APPROVAL_SCOPE,
    maximumAttempts: 1,
  });
  // The new directory entry must be durable before starting any external work.
  // File fsync alone cannot establish persistence of a newly created marker.
  syncPrivateDirectory(output);

  let authRoot,
    authority,
    identity,
    child,
    reaped = false,
    engineReturned = false,
    spawnAttempted = false,
    childSpawned = false,
    launchFailed = false,
    childClosed = false;
  let stateRemoved = false,
    rootIdentityConfirmed = false,
    sourceRevalidated = false;
  let outcome = null,
    failureCode = null;
  try {
    authRoot = mkdtempSync(path.join(output, "auth-"));
    privateDirectory(authRoot);
    authority = openSync(authRoot, constants.O_RDONLY | constants.O_NOFOLLOW);
    identity = fstatSync(authority);
    // No auth can start after an expired window, including slow local preflight.
    windowOpen(input);
    if (signal?.aborted) throw fail("ownerCancelledBeforeAttempt");
    const stage = path.join(root, ".cache/snowflake-runtime/macos-arm64");
    spawnAttempted = true;
    child = spawnChild(path.join(stage, "bin/node"), [path.join(stage, "helper/main.mjs")], {
      cwd: stage,
      env: { JOINERY_SNOWFLAKE_STATE_DIR: authRoot },
      stdio: ["pipe", "pipe", "pipe"],
      shell: false,
    });
    child.on("error", () => {
      if (!childSpawned) launchFailed = true;
    });
    child.once("spawn", () => {
      childSpawned = true;
    });
    child.once("close", () => {
      childClosed = true;
    });
    child.once("exit", () => {
      reaped = true;
    });
    outcome = await exerciseQualificationChild(child, {
      manifest: plan.payload.ledger,
      currentInput: input,
      generation: randomUUID(),
      signal,
    });
    engineReturned = true;
    failureCode = outcome.failureCode;
    try {
      verifyExecutionPlan(plan, { ...request, ...loadEvidence(root) }, loadRunnerSources(root));
      sourceRevalidated = true;
    } catch {
      failureCode ??= "postRunSourceDrift";
    }
  } catch {
    failureCode = "localRunnerFailure";
  } finally {
    if (child && !engineReturned) reaped = await emergencyReap(child, () => reaped);
    const noChildConfirmed = !child || (launchFailed && childClosed && !childSpawned);
    if (authRoot && (noChildConfirmed || reaped)) {
      try {
        privateDirectory(output);
        const current = privateDirectory(authRoot);
        if (!identity || current.dev !== identity.dev || current.ino !== identity.ino)
          throw fail("stateIdentityChanged");
        rootIdentityConfirmed = true;
        rmSync(authRoot, { recursive: true, force: false });
        stateRemoved = true;
      } catch {
        failureCode ??= "stateCleanupUnconfirmed";
      }
    }
    if (authority !== undefined) closeSync(authority);
    if (child && !reaped && !noChildConfirmed) {
      // Local reaping is uncertain. Preserve the private root; no rollback or
      // remote cancellation is inferred. Do not hold the CLI open indefinitely.
      child.stdin?.destroy();
      child.stdout?.destroy();
      child.stderr?.destroy();
      child.unref?.();
      failureCode ??= "childNotReaped";
    }
  }
  const result = {
    ...summary(plan, "attempted"),
    format: "joinery-snowflake-qualification-result-v1",
    at: new Date().toISOString(),
    approvalConsumed: true,
    qualificationPassed:
      outcome?.passed === true &&
      reaped &&
      stateRemoved &&
      sourceRevalidated &&
      failureCode === null,
    failureCode,
    exercise: outcome,
    sourceRevalidated,
    evidencePersisted: true,
    localCleanup: {
      spawnAttempted,
      processObjectCreated: Boolean(child),
      childSpawned,
      childReaped: reaped,
      noChildConfirmed: !child || (launchFailed && childClosed && !childSpawned),
      rootIdentityConfirmed,
      stateRemoved,
    },
  };
  try {
    emitExclusive(root, path.join(output, `result-${id}.json`), result);
  } catch {
    result.qualificationPassed = false;
    result.evidencePersisted = false;
    result.failureCode ??= "evidencePersistenceFailed";
  }
  return result;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  const cancel = () => controller.abort();
  process.once("SIGINT", cancel);
  process.once("SIGTERM", cancel);
  try {
    const result = await main(undefined, { signal: controller.signal });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    if (result.mode === "attempted" && !result.qualificationPassed) process.exitCode = 1;
  } catch (error) {
    process.stderr.write(
      `${JSON.stringify({ qualificationPassed: false, failureCode: error.safeCode ?? "localValidationFailed" })}\n`,
    );
    process.exitCode = 1;
  } finally {
    process.removeListener("SIGINT", cancel);
    process.removeListener("SIGTERM", cancel);
  }
}
