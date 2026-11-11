// Offline compiler only: no process launch, SDK import or account activity.
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  WORKFLOW as READ_WORKFLOW,
  SOURCE_PATHS as READ_SOURCE_PATHS,
  canonicalJson,
  validateQualificationInput,
  currentEvidence,
  readLocal,
  emitExclusive,
} from "./snowflake-qualification-manifest.mjs";
import {
  SETUP_WORKFLOW,
  setupTarget,
  deriveSetupLedger,
} from "../snowflake-helper/src/disposable-setup-ledger.mjs";

export { SETUP_WORKFLOW };
export const SETUP_SOURCE_PATHS = Object.freeze([
  ...READ_SOURCE_PATHS,
  "snowflake-helper/src/disposable-setup-ledger.mjs",
  "scripts/snowflake-qualification-setup-manifest.mjs",
]);
const repository = fileURLToPath(new URL("../", import.meta.url));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const fail = (code) => Object.assign(new Error(code), { safeCode: code });
const clone = (value) => JSON.parse(canonicalJson(value));
function freeze(value) {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
function exact(value, keys) {
  const actual = value && typeof value === "object" ? Object.keys(value).sort() : [];
  const expected = [...keys].sort();
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    actual.length !== expected.length ||
    actual.some((key, index) => key !== expected[index])
  )
    throw fail("invalidSetupInput");
}

export function compileSetupManifest(input) {
  // Canonical serialization rejects accessors/prototypes/unsafe numbers before
  // any caller-supplied field is read. The returned plan is deeply immutable.
  input = clone(input);
  exact(input, ["workflow", "route", "runId", "window", "profile", "sources", "runtime"]);
  if (input.workflow !== SETUP_WORKFLOW) throw fail("unsupportedSetupWorkflow");
  exact(input.sources, SETUP_SOURCE_PATHS);
  if (
    !Object.values(input.sources).every(
      (value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value),
    )
  )
    throw fail("invalidSetupSources");
  // Reuse only the established closed profile/window/runtime/source validation.
  // None of the read workflow's SQL, approval or execution authority is inherited.
  validateQualificationInput(
    {
      ...input,
      workflow: READ_WORKFLOW,
      sources: Object.fromEntries(READ_SOURCE_PATHS.map((name) => [name, input.sources[name]])),
    },
    86400000,
  );
  const target = setupTarget(input.runId);
  const sqlLedger = deriveSetupLedger(input.runId);
  const commands = [
    { id: "connect", op: "connect", payload: input.profile, sqlIndexes: [], maximumInvocations: 1 },
    ...sqlLedger.map((entry) => ({
      id: entry.key,
      op: entry.index === 1 ? "context" : entry.index === 2 ? "query" : "setup",
      sqlIndexes: [entry.index],
      maximumInvocations: 1,
      ...(entry.index > 2 ? { expectedIdentityFrom: "startup.identity" } : {}),
      payload:
        entry.index === 1
          ? {}
          : entry.index === 2
            ? {
                sql: entry.sql,
                binds: [],
                classification: "read",
                credits: 1,
                maxRows: entry.maximumRows,
                maxBytes: entry.maximumBytes,
              }
            : { runId: input.runId, step: entry.index },
    })),
    { id: "close", op: "close", payload: {}, sqlIndexes: [], maximumInvocations: 1 },
  ];
  const payload = {
    format: "joinery-snowflake-disposable-setup-manifest-v1",
    workflow: SETUP_WORKFLOW,
    route: input.route,
    executionStatus: "offlineOnly",
    runner: null,
    runId: input.runId,
    window: input.window,
    profile: input.profile,
    target,
    contextSha256: hash(canonicalJson(input.profile)),
    sourceSnapshotSha256: hash(canonicalJson(input.sources)),
    sources: input.sources,
    runtime: input.runtime,
    sqlLedger,
    commands,
    caps: {
      sessions: 1,
      sqlStatements: 28,
      createStatements: 3,
      setupRequests: 26,
      contextRequests: 1,
      queryRequests: 1,
      connectRequests: 1,
      closeRequests: 1,
      cancelRequests: 1,
      maximumResultRowsPerStatement: 1,
      maximumResultBytesPerStatement: 65536,
      maximumColumns: 32,
      maximumFrameBytes: 1048576,
      maximumInboundBytes: 8388608,
      maximumInboundFrames: 4096,
      maximumStderrBytes: 65536,
      maximumOutstandingSqlCommands: 1,
      cleanupControlMayOverlapActiveSql: true,
      helloMs: 2000,
      authenticationMs: 120000,
      sqlMs: 30000,
      workflowMs: 360000,
      admissionWindowMs: 86400000,
      closeMs: 2000,
      eofMs: 2000,
      termMs: 2000,
      killMs: 3000,
    },
    approval: {
      required: true,
      priorApprovalReusable: false,
      maximumAttempts: 1,
      durableConsumptionKeys: ["executionPlanSha256", "accountHostAndRunId"],
      requiredDisclosures: [
        "externalBrowserSso",
        "threeCreateStatements",
        "newDisposableObjectMetadata",
        "warehouseAutoResumeStorageAndBilling",
        "implicitDdlCommit",
        "noAutomaticCleanupOrReplay",
      ],
    },
    assertions: [
      "exactStartupPrincipalSessionRoleWarehouse",
      "singleRunSequentialSetupSteps",
      "idleAutocommitAndQuotedIdentifierPolicyBeforeEveryCreate",
      "onlyExpectedCreateInducedDatabaseSchemaTransitions",
      "stableSecondaryRoles",
      "exactDatabaseSchemaOwnerLabelsAndTransience",
      "exactEmptySyntheticTableCount",
      "durableIntentBeforeDispatch",
      "retainAndSyncCreateAcknowledgementBeforeNextSql",
      "firstFailureStopsFurtherSql",
      "knownAcknowledgementSurvivesLaterFailure",
    ],
    cleanup: {
      sqlStatements: 0,
      databaseDropAuthorized: false,
      closeOnNormalCompletion: true,
      eofThenTermThenKill: true,
      requireActualReapBeforeIdentityCheckedRootRemoval: true,
      failedReapOrRemovalRetainsState: true,
      remoteRollbackGuaranteed: false,
    },
    exclusions: [
      "existingTableData",
      "dmlAcceptance",
      "transactionCommands",
      "roleChanges",
      "grants",
      "arbitrarySql",
      "cleanupSql",
      "applicationRoute",
      "retries",
      "automaticReconnect",
      "warehouseManagement",
    ],
    limitations: [
      "This offline manifest is not execution or billing authorization.",
      "CREATE can commit pending work; the runner must establish fresh idle autocommit state.",
      "Acknowledgements, catalog readback and complete workflow qualification are separate evidence.",
      "Readback does not establish complete inventory, table shape, immutable identity or cleanup authority.",
      "No currency ceiling or physical authentication/result/logout HTTP limit is established.",
      "Source checks are pathname-based and do not defend against hostile same-user mutation.",
      "Context samples cannot exclude concurrent external changes or establish atomic identity.",
    ],
  };
  return freeze({ manifestSha256: hash(canonicalJson(payload)), payload });
}

export function verifySetupManifest(manifest, input) {
  manifest = clone(manifest);
  exact(manifest, ["manifestSha256", "payload"]);
  const expected = compileSetupManifest(input);
  if (canonicalJson(manifest) !== canonicalJson(expected)) throw fail("setupManifestMismatch");
  return expected;
}

export function currentSetupEvidence(root) {
  const evidence = currentEvidence(root);
  const extra = SETUP_SOURCE_PATHS.filter((name) => !READ_SOURCE_PATHS.includes(name));
  return {
    ...evidence,
    sources: {
      ...evidence.sources,
      ...Object.fromEntries(
        extra.map((name) => [name, hash(readLocal(root, name, { maximum: 1048576 }))]),
      ),
    },
  };
}

export function summarizeSetupManifest(manifest, mode = "dryRun") {
  return {
    mode,
    workflow: SETUP_WORKFLOW,
    manifestSha256: manifest.manifestSha256,
    sourceSnapshotSha256: manifest.payload.sourceSnapshotSha256,
    runtimeManifestSha256: manifest.payload.runtime.manifestSha256,
    sqlStatements: 28,
    createStatements: 3,
    cleanupSqlStatements: 0,
    offlineOnly: true,
    accountActivityStarted: false,
  };
}

export function main(
  args = process.argv.slice(2),
  { repoRoot = repository, loadEvidence = currentSetupEvidence } = {},
) {
  if (args.length !== 2 && args.length !== 4) throw fail("invalidSetupArguments");
  const [flag, filename, action, destination] = args;
  if (
    flag !== "--input" ||
    !filename ||
    (action !== undefined && !["--emit", "--verify"].includes(action)) ||
    (action && !destination)
  )
    throw fail("invalidSetupArguments");
  const request = JSON.parse(readLocal(repoRoot, filename, { privateFile: true }).toString("utf8"));
  exact(request, ["workflow", "route", "runId", "window", "profile"]);
  const input = { ...request, ...loadEvidence(repoRoot) };
  let manifest = compileSetupManifest(input);
  if (action === "--verify")
    manifest = verifySetupManifest(
      JSON.parse(readLocal(repoRoot, destination, { privateFile: true }).toString("utf8")),
      input,
    );
  if (action === "--emit") emitExclusive(repoRoot, destination, manifest);
  return summarizeSetupManifest(
    manifest,
    action === "--emit" ? "emittedManifest" : action === "--verify" ? "verifiedManifest" : "dryRun",
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.stdout.write(`${JSON.stringify(main())}\n`);
  } catch (error) {
    process.stderr.write(
      `${JSON.stringify({ passed: false, code: error.safeCode ?? "setupManifestFailed" })}\n`,
    );
    process.exitCode = 1;
  }
}
