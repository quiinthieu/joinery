import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";
import { canonicalJson, readLocal, emitExclusive } from "./snowflake-qualification-manifest.mjs";
import {
  compileSetupManifest,
  verifySetupManifest,
  currentSetupEvidence,
} from "./snowflake-qualification-setup-manifest.mjs";
import { exerciseSetupChild } from "./snowflake-qualification-setup-child.mjs";
import {
  createSetupJournal,
  setupRunKey,
  syncSetupDirectory,
} from "./snowflake-qualification-setup-journal.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const APPROVAL_SCOPE =
  "one-helper-session-externalbrowser-disposable-setup-28-sql-3-create-possible-billing-no-cleanup";
export const APPROVAL_FORMAT = "joinery-snowflake-setup-approval-v1";
export const RUNNER_SOURCES = Object.freeze([
  "scripts/snowflake-qualification-setup-runner.mjs",
  "scripts/snowflake-qualification-setup-protocol.mjs",
  "scripts/snowflake-qualification-setup-child.mjs",
  "scripts/snowflake-qualification-setup-journal.mjs",
  "scripts/snowflake-qualification-framing.mjs",
  "scripts/snowflake-qualification-results.mjs",
]);
function exact(value, keys, code = "invalidSetupInput") {
  canonicalJson(value);
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).length !== keys.length ||
    Object.keys(value)
      .sort()
      .some((key, index) => key !== [...keys].sort()[index])
  )
    throw fail(code);
}
function freeze(value) {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
export function compileSetupExecutionPlan(input, runnerSources) {
  const ledger = compileSetupManifest(input);
  exact(runnerSources, RUNNER_SOURCES);
  if (
    !Object.values(runnerSources).every(
      (value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value),
    )
  )
    throw fail("invalidSetupRunnerSources");
  const payload = {
    format: "joinery-snowflake-setup-execution-plan-v1",
    ledger,
    runnerSources: JSON.parse(canonicalJson(runnerSources)),
    approvalScope: APPROVAL_SCOPE,
    maximumAttempts: 1,
    physicalLimits: ledger.payload.caps,
  };
  return freeze(
    JSON.parse(canonicalJson({ executionPlanSha256: hash(canonicalJson(payload)), payload })),
  );
}
export function verifySetupExecutionPlan(plan, input, runnerSources) {
  const expected = compileSetupExecutionPlan(input, runnerSources);
  if (canonicalJson(plan) !== canonicalJson(expected)) throw fail("setupExecutionPlanDrift");
  verifySetupManifest(plan.payload.ledger, input);
  return expected;
}
function runnerEvidence(root) {
  return Object.fromEntries(
    RUNNER_SOURCES.map((name) => [name, hash(readLocal(root, name, { maximum: 1048576 }))]),
  );
}
function readJson(root, filename) {
  return JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(
      readLocal(root, filename, { privateFile: true }),
    ),
  );
}
function windowOpen(input) {
  if (
    Date.now() < Date.parse(input.window.notBefore) ||
    Date.now() >= Date.parse(input.window.expiresAt)
  )
    throw fail("setupExecutionWindowUnavailable");
}
function argumentsFor(args) {
  const options = {};
  for (let i = 0; i < args.length; i++) {
    const key = args[i];
    if (
      !["--input", "--emit", "--plan", "--approval", "--run"].includes(key) ||
      Object.hasOwn(options, key)
    )
      throw fail("invalidSetupArguments");
    if (key === "--run") options[key] = true;
    else {
      const value = args[++i];
      if (!value || value.startsWith("--")) throw fail("invalidSetupArguments");
      options[key] = value;
    }
  }
  if (
    !options["--input"] ||
    (options["--emit"] && (options["--plan"] || options["--run"] || options["--approval"])) ||
    Boolean(options["--run"]) !== Boolean(options["--approval"]) ||
    (options["--run"] && !options["--plan"])
  )
    throw fail("invalidSetupArguments");
  return options;
}
export function summarizeSetupExecution(plan, mode) {
  return {
    mode,
    executionPlanSha256: plan.executionPlanSha256,
    ledgerSha256: plan.payload.ledger.manifestSha256,
    runtimeManifestSha256: plan.payload.ledger.payload.runtime.manifestSha256,
    window: plan.payload.ledger.payload.window,
    route: "helper-direct",
    plannedSqlStatements: 28,
    plannedCreateStatements: 3,
    cleanupSqlStatements: 0,
    plannedSessions: 1,
    maximumAttempts: 1,
    approvalScope: APPROVAL_SCOPE,
    approvalConsumed: false,
    qualificationPassed: false,
  };
}
async function emergencyReap(child, reaped) {
  for (const stream of [child.stdin, child.stdout, child.stderr])
    try {
      stream?.destroy();
    } catch {}
  if (reaped()) return true;
  const wait = (ms) =>
    new Promise((resolve) => {
      let handle;
      const done = () => {
        clearTimeout(handle);
        child.removeListener("exit", done);
        resolve(reaped());
      };
      child.once("exit", done);
      handle = setTimeout(done, ms);
      if (reaped()) done();
    });
  try {
    child.kill("SIGTERM");
  } catch {}
  if (await wait(2000)) return true;
  try {
    child.kill("SIGKILL");
  } catch {}
  return wait(3000);
}

// Dependency injection is solely an offline-test seam. The CLI has no runtime,
// SQL, credentials, environment, policy or executable override flags.
export async function main(
  args = process.argv.slice(2),
  {
    repoRoot = repository,
    loadEvidence = currentSetupEvidence,
    loadRunnerSources = runnerEvidence,
    spawnChild = spawn,
    platform = process.platform,
    arch = process.arch,
    signal,
  } = {},
) {
  const options = argumentsFor(args),
    root = fs.realpathSync(repoRoot);
  const request = readJson(root, options["--input"]);
  exact(request, ["workflow", "route", "runId", "window", "profile"]);
  const input = { ...request, ...loadEvidence(root) },
    sources = loadRunnerSources(root);
  let plan = compileSetupExecutionPlan(input, sources);
  if (options["--plan"])
    plan = verifySetupExecutionPlan(readJson(root, options["--plan"]), input, sources);
  if (options["--emit"]) emitExclusive(root, options["--emit"], plan);
  if (!options["--run"])
    return summarizeSetupExecution(
      plan,
      options["--emit"]
        ? "emittedExecutionPlan"
        : options["--plan"]
          ? "verifiedExecutionPlan"
          : "dryRun",
    );
  if (platform !== "darwin" || arch !== "arm64") throw fail("unsupportedSetupPlatform");
  if (signal?.aborted) throw fail("setupOwnerCancelledBeforeAttempt");
  windowOpen(input);
  const approval = readJson(root, options["--approval"]);
  exact(
    approval,
    ["format", "executionPlanSha256", "scope", "maximumAttempts"],
    "invalidSetupApproval",
  );
  if (
    approval.format !== APPROVAL_FORMAT ||
    approval.executionPlanSha256 !== plan.executionPlanSha256 ||
    approval.scope !== APPROVAL_SCOPE ||
    approval.maximumAttempts !== 1
  )
    throw fail("invalidSetupApproval");
  const output = path.join(root, ".cache/snowflake-qualification");
  syncSetupDirectory(output);
  verifySetupExecutionPlan(plan, { ...request, ...loadEvidence(root) }, loadRunnerSources(root));
  windowOpen(input);
  const id = plan.executionPlanSha256,
    runKey = setupRunKey(input.profile, input.runId);
  const resultFile = path.join(output, `setup-result-${id}.json`);
  try {
    fs.lstatSync(resultFile);
    throw fail("setupEvidenceAlreadyExists");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const consumption = {
    format: "joinery-snowflake-setup-consumption-v1",
    executionPlanSha256: id,
    runKey,
    at: new Date().toISOString(),
    scope: APPROVAL_SCOPE,
    maximumAttempts: 1,
  };
  // Both permanent markers precede authentication. A changed window/source hash
  // cannot authorize a second attempt against the same generated object names.
  emitExclusive(root, path.join(output, `setup-consumed-run-${runKey}.json`), consumption);
  syncSetupDirectory(output);
  emitExclusive(root, path.join(output, `setup-consumed-plan-${id}.json`), consumption);
  syncSetupDirectory(output);
  const journal = createSetupJournal({
    root,
    filename: path.join(output, `setup-journal-${id}.jsonl`),
    executionPlanSha256: id,
    runKey,
  });
  let authRoot,
    identity,
    authority,
    child,
    engineReturned = false,
    childSpawned = false,
    launchFailed = false,
    childClosed = false,
    reaped = false;
  let outcome = null,
    failureCode = null,
    sourceRevalidated = false,
    stateRemoved = false,
    rootIdentityConfirmed = false,
    journalCloseFailed = false;
  try {
    authRoot = fs.mkdtempSync(path.join(output, "setup-auth-"));
    syncSetupDirectory(authRoot);
    authority = fs.openSync(authRoot, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    identity = fs.fstatSync(authority);
    windowOpen(input);
    if (signal?.aborted) throw fail("setupOwnerCancelledBeforeAttempt");
    const stage = path.join(root, ".cache/snowflake-runtime/macos-arm64");
    journal.append({ kind: "spawnIntent" });
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
    outcome = await exerciseSetupChild(child, {
      manifest: plan.payload.ledger,
      currentInput: input,
      generation: randomUUID(),
      persist: (event) => journal.append(event),
      signal,
    });
    engineReturned = true;
    failureCode = outcome.failureCode;
    try {
      verifySetupExecutionPlan(
        plan,
        { ...request, ...loadEvidence(root) },
        loadRunnerSources(root),
      );
      sourceRevalidated = true;
    } catch {
      failureCode ??= "setupPostRunSourceDrift";
    }
  } catch {
    failureCode ??= "setupLocalRunnerFailure";
  } finally {
    if (child && !engineReturned) reaped = await emergencyReap(child, () => reaped);
    const noChildConfirmed = !child || (launchFailed && childClosed && !childSpawned);
    if (authRoot && (noChildConfirmed || reaped)) {
      try {
        syncSetupDirectory(output);
        syncSetupDirectory(authRoot);
        const current = fs.lstatSync(authRoot);
        if (!identity || current.dev !== identity.dev || current.ino !== identity.ino)
          throw fail("setupStateIdentityChanged");
        rootIdentityConfirmed = true;
        fs.rmSync(authRoot, { recursive: true, force: false });
        stateRemoved = true;
      } catch {
        failureCode ??= "setupStateCleanupUnconfirmed";
      }
    }
    if (authority !== undefined)
      try {
        fs.closeSync(authority);
      } catch {
        failureCode ??= "setupStateHandleCloseFailed";
      }
    if (child && !reaped && !noChildConfirmed) {
      for (const stream of [child.stdin, child.stdout, child.stderr])
        try {
          stream?.destroy();
        } catch {}
      child.unref?.();
      failureCode ??= "setupChildNotReaped";
    }
    try {
      journal.close();
    } catch {
      journalCloseFailed = true;
      failureCode ??= "setupJournalCloseFailed";
    }
  }
  const qualificationCandidate =
    outcome?.passed === true && reaped && stateRemoved && sourceRevalidated && failureCode === null;
  const completeJournal = !journalCloseFailed && outcome?.journalFailed !== true;
  const result = {
    ...summarizeSetupExecution(plan, "attempted"),
    format: "joinery-snowflake-setup-result-v1",
    at: new Date().toISOString(),
    approvalConsumed: true,
    // A receipt cannot attest to the durability of the write that contains it.
    // Persist execution facts and a candidate verdict only. Final confirmation
    // is returned separately after both exclusive file emission and dir fsync.
    qualificationPassed: false,
    evidencePersisted: false,
    qualificationCandidate,
    workflowChecksPassed: outcome?.passed === true,
    finalizationStatus: "requiresRunnerConfirmation",
    failureCode,
    exercise: outcome,
    sourceRevalidated,
    journalCloseFailed,
    localCleanup: {
      processObjectCreated: Boolean(child),
      childSpawned,
      childReaped: reaped,
      noChildConfirmed: !child || (launchFailed && childClosed && !childSpawned),
      rootIdentityConfirmed,
      stateRemoved,
    },
  };
  try {
    emitExclusive(root, resultFile, result);
    syncSetupDirectory(output);
    result.evidencePersisted = completeJournal;
    result.qualificationPassed = qualificationCandidate && completeJournal;
    result.finalizationStatus = "runnerConfirmed";
  } catch {
    result.qualificationPassed = false;
    result.evidencePersisted = false;
    result.failureCode ??= "setupEvidencePersistenceFailed";
    result.finalizationStatus = "runnerConfirmationFailed";
  }
  return result;
}

// CLI output deliberately omits private principals, session/SDK IDs, object names
// and collected cells. Exact receipts remain in the mode-0600 private journal.
export function publicSetupResult(result) {
  const { exercise, localCleanup, ...summary } = result;
  return {
    ...summary,
    ...(exercise
      ? {
          createOutcomes: exercise.outcomes
            .filter((entry) => entry.mutation)
            .map(({ step, remoteOutcome, validationStatus }) => ({
              step,
              remoteOutcome,
              validationStatus,
            })),
          journalFailed: exercise.journalFailed || summary.journalCloseFailed === true,
        }
      : {}),
    ...(localCleanup ? { localCleanup } : {}),
  };
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController(),
    cancel = () => controller.abort();
  process.once("SIGINT", cancel);
  process.once("SIGTERM", cancel);
  try {
    const result = await main(undefined, { signal: controller.signal });
    process.stdout.write(`${JSON.stringify(publicSetupResult(result), null, 2)}\n`);
    if (result.mode === "attempted" && !result.qualificationPassed) process.exitCode = 1;
  } catch (error) {
    process.stderr.write(
      `${JSON.stringify({ qualificationPassed: false, failureCode: /^[A-Za-z]{1,80}$/.test(error.safeCode ?? "") ? error.safeCode : "setupLocalValidationFailed" })}\n`,
    );
    process.exitCode = 1;
  } finally {
    process.removeListener("SIGINT", cancel);
    process.removeListener("SIGTERM", cancel);
  }
}
