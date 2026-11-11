// Offline compiler only. No helper, SDK import, network, authentication or runner.
import { createHash } from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  fsyncSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateSnowflakeResourceTree } from "./native-smoke-resources.mjs";
import { SQL_CONTRACT, validateSqlContract } from "../snowflake-helper/src/sql-contract.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
export const WORKFLOW = "session-options-scalar-state-v1";
const FORMAT = "joinery-snowflake-qualification-manifest-v1";
const HASH = /^[a-f0-9]{64}$/;
const IDENTIFIER = /^[A-Z_][A-Z0-9_]{0,254}$/;
const MAX_JSON_BYTES = 256 * 1024;
const NODE_SHA256 = "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a";
export const SOURCE_PATHS = Object.freeze([
  "scripts/snowflake-qualification-manifest.mjs",
  "scripts/native-smoke-resources.mjs",
  "scripts/prepare-snowflake-runtime.mjs",
  "snowflake-helper/src/main.mjs",
  "snowflake-helper/src/helper.mjs",
  "snowflake-helper/src/framing.mjs",
  "snowflake-helper/src/query-submit-guard.mjs",
  "snowflake-helper/src/transport-source-pins.mjs",
  "snowflake-helper/src/sql-contract.json",
  "snowflake-helper/src/sql-contract.mjs",
  "snowflake-helper/package-lock.json",
  "src-tauri/build.rs",
  "src-tauri/src/drivers/snowflake_sql_contract.rs",
  "src-tauri/src/drivers/snowflake_external.rs",
  "src-tauri/src/drivers/snowflake_metadata.rs",
  "src-tauri/src/drivers/snowflake_session.rs",
]);
const SQL = Object.freeze({
  startupIdentity: SQL_CONTRACT.bootstrapContext,
  version: SQL_CONTRACT.version,
  context:
    "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES",
  quotedIdentifierPolicy: "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
  autocommit: "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
  abortOnError: "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION",
  transaction: "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID",
  schemas:
    "SELECT CATALOG_NAME, SCHEMA_NAME FROM {}.INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = {} AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT {}",
  scalar:
    "SELECT CAST(? AS NUMBER(38,0)) AS EXACT_INTEGER, CAST(? AS BOOLEAN) AS EXACT_BOOLEAN, CAST(? AS VARCHAR) AS EXACT_TEXT, CAST(? AS VARCHAR) AS EXACT_NULL",
});
const SAMPLE = Object.freeze([
  "context",
  "quotedIdentifierPolicy",
  "autocommit",
  "abortOnError",
  "transaction",
]);
const CAPS = Object.freeze({
  sessions: 1,
  sqlStatements: 24,
  helperQueryRequests: 23,
  contextRequests: 1,
  connectRequests: 1,
  closeRequests: 1,
  cancelRequests: 1,
  additionalCreditRequests: 1000,
  maximumOutstandingCredits: 1,
  maximumSuccessfulSchemaNames: 1000,
  schemaSentinelRows: 1001,
  maximumSuccessfulRows: 1023,
  maximumAttemptedRows: 1024,
  maximumResultContentBytes: 4 * 1024 * 1024,
  maximumFrameBytes: 1024 * 1024,
  helloMs: 2000,
  authenticationMs: 120000,
  sqlMs: 30000,
  optionsOperationMs: 65000,
  stateOperationMs: 65000,
  admissionWindowMs: 600000,
  closeMs: 2000,
  eofMs: 2000,
  termMs: 2000,
  killMs: 3000,
});
const fail = (code) => Object.assign(new Error(code), { safeCode: code });
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const text = (value, maximum) =>
  typeof value === "string" &&
  value.length > 0 &&
  Buffer.byteLength(value) <= maximum &&
  !/[\x00-\x1f\x7f]/.test(value);

function record(value) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    ![Object.prototype, null].includes(Object.getPrototypeOf(value))
  )
    throw fail("invalidInput");
  if (
    Reflect.ownKeys(value).some(
      (key) =>
        typeof key !== "string" ||
        !Object.getOwnPropertyDescriptor(value, key).enumerable ||
        !Object.hasOwn(Object.getOwnPropertyDescriptor(value, key), "value"),
    )
  )
    throw fail("invalidInput");
  return value;
}
function exact(value, keys) {
  record(value);
  if (Object.keys(value).length !== keys.length || !keys.every((key) => Object.hasOwn(value, key)))
    throw fail("invalidInput");
}

export function canonicalJson(value) {
  function encode(current, depth) {
    if (depth > 32) throw fail("invalidInput");
    if (current === null || typeof current === "string" || typeof current === "boolean")
      return JSON.stringify(current);
    if (typeof current === "number" && Number.isSafeInteger(current) && !Object.is(current, -0))
      return String(current);
    if (Array.isArray(current)) {
      if (
        current.length > 10000 ||
        Reflect.ownKeys(current).length !== current.length + 1 ||
        Array.from({ length: current.length }, (_, index) => String(index)).some((key) => {
          const descriptor = Object.getOwnPropertyDescriptor(current, key);
          return !descriptor?.enumerable || !Object.hasOwn(descriptor, "value");
        })
      )
        throw fail("invalidInput");
      return `[${current.map((item) => encode(item, depth + 1)).join(",")}]`;
    }
    record(current);
    return `{${Object.keys(current)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${encode(current[key], depth + 1)}`)
      .join(",")}}`;
  }
  const result = encode(value, 0);
  if (Buffer.byteLength(result) > MAX_JSON_BYTES) throw fail("inputTooLarge");
  return result;
}
function frozenCopy(value) {
  const copy = JSON.parse(canonicalJson(value));
  function freeze(current) {
    if (current && typeof current === "object") {
      Object.values(current).forEach(freeze);
      Object.freeze(current);
    }
    return current;
  }
  return freeze(copy);
}
function validateProfile(profile) {
  exact(profile, ["account", "host", "username", "database", "schema", "warehouse", "role"]);
  if (
    !text(profile.account, 180) ||
    !/^[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]$/.test(profile.account) ||
    profile.account.includes("..") ||
    profile.account.endsWith(".snowflakecomputing.com") ||
    profile.host !== `${profile.account}.snowflakecomputing.com` ||
    !text(profile.username, 256) ||
    !/^[A-Za-z0-9_@.+-]+$/.test(profile.username) ||
    !["database", "schema", "warehouse", "role"].every(
      (key) => text(profile[key], 255) && IDENTIFIER.test(profile[key]),
    )
  )
    throw fail("invalidProfile");
}
export function validateQualificationInput(input, admissionWindowMs = CAPS.admissionWindowMs) {
  if (
    !Number.isSafeInteger(admissionWindowMs) ||
    admissionWindowMs < 1 ||
    admissionWindowMs > 86400000
  )
    throw fail("invalidWindow");
  exact(input, ["workflow", "route", "runId", "window", "profile", "sources", "runtime"]);
  if (input.workflow !== WORKFLOW) throw fail("unsupportedWorkflow");
  if (input.route !== "helper-direct") throw fail("unsupportedRoute");
  if (
    typeof input.runId !== "string" ||
    !/^[A-F0-9]{32}$/.test(input.runId) ||
    /^0+$/.test(input.runId)
  )
    throw fail("invalidRunId");
  exact(input.window, ["notBefore", "expiresAt"]);
  const times = [input.window.notBefore, input.window.expiresAt].map((value) => {
    if (
      !text(value, 24) ||
      !Number.isFinite(Date.parse(value)) ||
      new Date(value).toISOString() !== value
    )
      throw fail("invalidWindow");
    return Date.parse(value);
  });
  if (times[1] <= times[0] || times[1] - times[0] > admissionWindowMs) throw fail("invalidWindow");
  validateProfile(input.profile);
  exact(input.sources, SOURCE_PATHS);
  if (!Object.values(input.sources).every((value) => typeof value === "string" && HASH.test(value)))
    throw fail("invalidSources");
  exact(input.runtime, [
    "format",
    "target",
    "nodeVersion",
    "nodeSha256",
    "sdkVersion",
    "manifestSha256",
  ]);
  if (
    input.runtime.format !== "joinery-snowflake-runtime-v1" ||
    input.runtime.target !== "darwin-arm64" ||
    input.runtime.nodeVersion !== "24.22.2" ||
    input.runtime.nodeSha256 !== NODE_SHA256 ||
    input.runtime.sdkVersion !== "3.4.1" ||
    typeof input.runtime.manifestSha256 !== "string" ||
    !HASH.test(input.runtime.manifestSha256)
  )
    throw fail("invalidRuntime");
}

export function compileManifest(input) {
  validateQualificationInput(input);
  // Snapshot the caller's data before generating commands; neither mutation nor
  // JSON key order may alter a compiled manifest after its hash is returned.
  input = frozenCopy(input);
  const commands = [],
    sqlLedger = [];
  const small = (template) => ({
    maximumRows: 1,
    maximumBytes: 65536,
    maximumColumns:
      template === "context"
        ? 8
        : ["quotedIdentifierPolicy", "autocommit", "abortOnError"].includes(template)
          ? 32
          : template === "scalar"
            ? 4
            : template === "startupIdentity"
              ? 6
              : 1,
  });
  function sqlCommand(id, templateId, group, { op = "query", binds = [] } = {}) {
    const isSchemas = templateId === "schemas";
    const sql = isSchemas
      ? SQL.schemas
          .replace("{}", `"${input.profile.database.replaceAll('"', '""')}"`)
          .replace("{}", `'${input.profile.database.replaceAll("'", "''")}'`)
          .replace("{}", "1001")
      : SQL[templateId];
    const limits = isSchemas
      ? { maximumRows: 1001, maximumBytes: 2 * 1024 * 1024, maximumColumns: 2 }
      : small(templateId);
    const ordinal = sqlLedger.length + 1;
    sqlLedger.push({
      ordinal,
      commandId: id,
      templateId,
      sql,
      binds,
      group,
      maximumInvocations: 1,
      limits,
    });
    commands.push({
      id,
      op,
      sqlOrdinals: [ordinal],
      maximumInvocations: 1,
      payload:
        op === "context"
          ? {}
          : {
              sql,
              binds: binds.map((bind) => bind.value),
              classification: "read",
              credits: 1,
              maxRows: limits.maximumRows,
              maxBytes: limits.maximumBytes,
            },
    });
  }
  commands.push({
    id: "connect",
    op: "connect",
    payload: input.profile,
    sqlOrdinals: [],
    maximumInvocations: 1,
  });
  sqlCommand("startup.identity", "startupIdentity", "startup", { op: "context" });
  sqlCommand("startup.version", "version", "startup");
  const sample = (prefix, group) =>
    SAMPLE.forEach((template) => sqlCommand(`${prefix}.${template}`, template, group));
  sample("options.before", "options");
  sqlCommand("options.schemas", "schemas", "options");
  sample("options.after", "options");
  sqlCommand("scalar", "scalar", "scalar", {
    binds: [
      { type: "integer", value: "9007199254740993" },
      { type: "boolean", value: true },
      { type: "text", value: "joinery-qualification" },
      { type: "text", value: null },
    ],
  });
  sample("state.before", "state");
  sample("state.after", "state");
  commands.push({ id: "close", op: "close", payload: {}, sqlOrdinals: [], maximumInvocations: 1 });
  const payload = {
    format: FORMAT,
    workflow: WORKFLOW,
    route: "helper-direct",
    executionStatus: "offlineOnly",
    runner: null,
    runId: input.runId,
    window: input.window,
    contextSha256: hash(canonicalJson(input.profile)),
    sourceSnapshotSha256: hash(canonicalJson(input.sources)),
    sources: input.sources,
    runtime: input.runtime,
    templateSha256: hash(canonicalJson(SQL)),
    commands,
    sqlLedger,
    caps: CAPS,
    operationGroups: { startup: [1, 2], options: [3, 13], scalar: [14, 14], state: [15, 24] },
    approval: {
      required: true,
      binding: "manifestSha256",
      priorCanaryApprovalReusable: false,
      requiredDisclosures: [
        "externalBrowserSso",
        "warehouseAutoResumeAndBilling",
        "currentDatabaseSchemaNames",
      ],
    },
    assertions: [
      "initialPrincipalAndSessionPinned",
      "opaqueSessionAndTransactionIds",
      "nullableObservedRoleWithoutProfileFallback",
      "sameFixedDatabase",
      "exactShowParameterKeyAndBooleanValue",
      "stableFullBeforeAfterSamples",
      "successfulNullTransactionDistinctFromUnavailable",
      "transactionUsabilityUnknown",
      "currentSchemaNotAddedToChoices",
      "schemaSentinelOrTruncationFails",
      "exactTypedScalarValues",
      "noAdditionalSqlOnFailure",
    ],
    exclusions: [
      "applicationRoute",
      "tableMetadata",
      "existingTableData",
      "writes",
      "ddl",
      "transactionControls",
      "sessionSwitching",
      "exports",
      "runnerRetries",
      "automaticReconnect",
      "warehouseManagement",
    ],
    controls: {
      credit: {
        commandId: "options.schemas",
        initialCredits: 1,
        replenishAfterConsumedNonemptyBatch: 1,
        maximumAdditionalRequests: 1000,
      },
      cancel: {
        maximumRequests: 1,
        reason: "ownerCancellationOnly",
        acknowledgementProvesOriginalTerminal: false,
      },
    },
    cleanup: {
      sqlStatements: 0,
      closeOnNormalCompletion: true,
      eofThenTermThenKill: true,
      requireActualReapBeforeIdentityCheckedRootRemoval: true,
      failedReapOrRemovalRetainsState: true,
      remoteRollbackGuaranteed: false,
    },
    limitations: [
      "No runner is implemented and this manifest is not authorization.",
      "Pure API fingerprints are supplied bindings; only CLI evidence loading validates current local files.",
      "Run ID validity is syntactic; freshness and one-shot approval consumption require a future runner.",
      "SQL counts exclude authentication, renewal, cancellation and result-fetch HTTP work; no physical HTTP or currency ceiling is established.",
      "The guard bounds reviewed query submissions, not exactly-once server effects.",
      "No packaged application or account behavior is qualified; application automatic cadence is not admitted.",
      "No independent principal NAME to login binding is established.",
      "Two samples detect observed drift; they do not establish atomicity or exclude external ABA changes.",
    ],
  };
  return frozenCopy({ manifestSha256: hash(canonicalJson(payload)), payload });
}

export function verifyManifest(manifest, currentInput) {
  exact(manifest, ["manifestSha256", "payload"]);
  if (
    typeof manifest.manifestSha256 !== "string" ||
    !HASH.test(manifest.manifestSha256) ||
    hash(canonicalJson(manifest.payload)) !== manifest.manifestSha256
  )
    throw fail("manifestTampered");
  const expected = compileManifest(currentInput);
  if (canonicalJson(manifest) !== canonicalJson(expected)) throw fail("manifestDrift");
  return expected;
}
export function summarizeManifest(manifest, mode = "dryRun") {
  return {
    mode,
    executionStatus: "offlineOnly",
    runnerImplemented: false,
    authorizationGranted: false,
    workflow: WORKFLOW,
    route: "helper-direct",
    manifestSha256: manifest.manifestSha256,
    sourceSnapshotSha256: manifest.payload.sourceSnapshotSha256,
    runtimeManifestSha256: manifest.payload.runtime.manifestSha256,
    templateSha256: manifest.payload.templateSha256,
    caps: { ...CAPS },
    requiredApproval:
      "Exact manifest hash, run window, external-browser SSO, warehouse resume/billing and current-database schema-name disclosure; no prior approval is inherited.",
  };
}

function inside(root, filename) {
  const relative = path.relative(root, filename);
  return (
    relative !== "" &&
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}
function localPath(root, value) {
  if (!text(value, 4096)) throw fail("unsafeLocalPath");
  const filename = path.resolve(root, value);
  if (!inside(root, filename)) throw fail("unsafeLocalPath");
  let directory = path.dirname(filename);
  while (directory !== root) {
    const stat = lstatSync(directory);
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(directory) !== directory)
      throw fail("unsafeLocalPath");
    directory = path.dirname(directory);
  }
  return filename;
}
export function readLocal(root, filename, { privateFile = false, maximum = MAX_JSON_BYTES } = {}) {
  filename = localPath(root, filename);
  if (privateFile && !inside(path.join(root, ".cache"), filename)) throw fail("unsafeLocalPath");
  const before = lstatSync(filename);
  const safe = (stat) =>
    stat.isFile() &&
    !stat.isSymbolicLink() &&
    stat.size <= maximum &&
    (!privateFile ||
      ((stat.mode & 0o7777) === 0o600 && (!process.getuid || stat.uid === process.getuid())));
  if (!safe(before)) throw fail("unsafeLocalFile");
  const fd = openSync(filename, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = fstatSync(fd);
    if (!safe(opened) || before.dev !== opened.dev || before.ino !== opened.ino)
      throw fail("unsafeLocalFile");
    const bytes = Buffer.alloc(maximum + 1);
    let length = 0,
      read;
    while (
      length < bytes.length &&
      (read = readSync(fd, bytes, length, bytes.length - length, null)) > 0
    )
      length += read;
    if (length > maximum) throw fail("inputTooLarge");
    return bytes.subarray(0, length);
  } finally {
    closeSync(fd);
  }
}
function rustConstant(source, name) {
  const match = source.match(new RegExp(`const ${name}: &str =\\s*("(?:[^"\\\\]|\\\\.)*");`));
  if (!match) throw fail("sourceTemplateDrift");
  try {
    return JSON.parse(match[1]);
  } catch {
    throw fail("sourceTemplateDrift");
  }
}
export function validateSourceTemplates(sources) {
  exact(sources, SOURCE_PATHS);
  if (
    !Object.values(sources).every(
      (value) => typeof value === "string" && Buffer.byteLength(value) <= 1024 * 1024,
    )
  )
    throw fail("invalidSources");
  const session = sources["src-tauri/src/drivers/snowflake_session.rs"];
  const metadata = sources["src-tauri/src/drivers/snowflake_metadata.rs"];
  let contract;
  try {
    contract = validateSqlContract(JSON.parse(sources["snowflake-helper/src/sql-contract.json"]));
  } catch {
    throw fail("sourceTemplateDrift");
  }
  for (const [name, template] of [
    ["AUTOCOMMIT_SQL", "autocommit"],
    ["ABORT_ON_ERROR_SQL", "abortOnError"],
    ["CURRENT_TRANSACTION_SQL", "transaction"],
  ]) {
    if (rustConstant(session, name) !== SQL[template]) throw fail("sourceTemplateDrift");
  }
  if (
    rustConstant(metadata, "CONTEXT_SQL") !== SQL.context ||
    rustConstant(metadata, "QUOTED_IDENTIFIER_POLICY_SQL") !== SQL.quotedIdentifierPolicy ||
    !metadata.includes(JSON.stringify(SQL.schemas)) ||
    !/const SCHEMA_LIMIT: usize = 1000;/.test(metadata) ||
    contract.bootstrapContext !== SQL.startupIdentity ||
    contract.version !== SQL.version ||
    !sources["snowflake-helper/src/helper.mjs"].includes(
      "const CONTEXT_SQL = SQL_CONTRACT.bootstrapContext;",
    ) ||
    !metadata.includes("super::snowflake_sql_contract::VERSION_SQL") ||
    !sources["src-tauri/src/drivers/snowflake_external.rs"].includes("metadata::version_read()") ||
    !sources["src-tauri/src/drivers/snowflake_sql_contract.rs"].includes(
      'include!(concat!(env!("OUT_DIR"), "/snowflake_sql_contract.rs"));',
    ) ||
    !sources["src-tauri/build.rs"].includes('join("../snowflake-helper/src/sql-contract.json")')
  )
    throw fail("sourceTemplateDrift");
}
export function currentEvidence(root) {
  const bytes = Object.fromEntries(
    SOURCE_PATHS.map((file) => [file, readLocal(root, file, { maximum: 1024 * 1024 })]),
  );
  validateSourceTemplates(
    Object.fromEntries(SOURCE_PATHS.map((file) => [file, bytes[file].toString("utf8")])),
  );
  const runtime = validateSnowflakeResourceTree({ repoRoot: root });
  return {
    sources: Object.fromEntries(SOURCE_PATHS.map((file) => [file, hash(bytes[file])])),
    runtime: {
      format: "joinery-snowflake-runtime-v1",
      target: "darwin-arm64",
      nodeVersion: "24.22.2",
      nodeSha256: NODE_SHA256,
      sdkVersion: "3.4.1",
      manifestSha256: runtime.manifestSha256,
    },
  };
}
export function emitExclusive(root, filename, manifest) {
  filename = localPath(root, filename);
  const outputRoot = path.join(root, ".cache/snowflake-qualification");
  if (!inside(outputRoot, filename)) throw fail("unsafeLocalPath");
  const output = lstatSync(outputRoot);
  if (
    !output.isDirectory() ||
    output.isSymbolicLink() ||
    (output.mode & 0o7777) !== 0o700 ||
    (process.getuid && output.uid !== process.getuid())
  )
    throw fail("unsafeOutputDirectory");
  const parent = lstatSync(path.dirname(filename));
  if ((parent.mode & 0o7777) !== 0o700 || (process.getuid && parent.uid !== process.getuid()))
    throw fail("unsafeOutputDirectory");
  const fd = openSync(
    filename,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
    0o600,
  );
  let complete = false;
  let identity;
  try {
    const stat = (identity = fstatSync(fd));
    if ((stat.mode & 0o7777) !== 0o600 || (process.getuid && stat.uid !== process.getuid()))
      throw fail("unsafeLocalFile");
    writeFileSync(fd, `${canonicalJson(manifest)}\n`);
    fsyncSync(fd);
    const current = lstatSync(filename);
    if (current.dev !== identity.dev || current.ino !== identity.ino)
      throw fail("outputIdentityChanged");
    complete = true;
  } finally {
    closeSync(fd);
    if (!complete && identity) {
      try {
        const current = lstatSync(filename);
        if (current.dev === identity.dev && current.ino === identity.ino) unlinkSync(filename);
      } catch {
        /* Preserve the primary failure; never remove a replacement. */
      }
    }
  }
}

// Injection is an offline synthetic-test seam; no CLI evidence/path-to-runtime override.
export function main(
  args = process.argv.slice(2),
  { repoRoot = repository, loadEvidence = currentEvidence } = {},
) {
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    const option = args[index];
    if (
      !["--input", "--emit", "--verify"].includes(option) ||
      Object.hasOwn(options, option) ||
      !args[index + 1] ||
      args[index + 1].startsWith("--")
    )
      throw fail("invalidArguments");
    options[option] = args[index + 1];
  }
  if (!options["--input"] || (options["--emit"] && options["--verify"]))
    throw fail("invalidArguments");
  const root = realpathSync(repoRoot);
  const request = JSON.parse(
    readLocal(root, options["--input"], { privateFile: true }).toString("utf8"),
  );
  exact(request, ["workflow", "route", "runId", "window", "profile"]);
  const input = { ...request, ...loadEvidence(root) };
  let manifest = compileManifest(input);
  if (options["--verify"])
    manifest = verifyManifest(
      JSON.parse(readLocal(root, options["--verify"], { privateFile: true }).toString("utf8")),
      input,
    );
  if (options["--emit"]) emitExclusive(root, options["--emit"], manifest);
  return summarizeManifest(
    manifest,
    options["--emit"]
      ? "emittedOfflineManifest"
      : options["--verify"]
        ? "verifiedOfflineManifest"
        : "dryRun",
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.stdout.write(`${JSON.stringify(main(), null, 2)}\n`);
  } catch (error) {
    process.stderr.write(
      `${JSON.stringify({ executionStatus: "offlineOnly", failure: error.safeCode ?? "localValidationFailed" })}\n`,
    );
    process.exitCode = 1;
  }
}
