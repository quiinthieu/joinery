import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import childProcess from "node:child_process";
import net from "node:net";
import tls from "node:tls";
import dns from "node:dns";
import { syncBuiltinESMExports } from "node:module";
import os from "node:os";
import path from "node:path";
import {
  WORKFLOW,
  SOURCE_PATHS,
  canonicalJson,
  compileManifest,
  verifyManifest,
  summarizeManifest,
  validateSourceTemplates,
  main,
} from "./snowflake-qualification-manifest.mjs";

const clone = (value) => JSON.parse(JSON.stringify(value));
const input = () => ({
  workflow: WORKFLOW,
  route: "helper-direct",
  runId: "A1".repeat(16),
  window: { notBefore: "2026-11-10T10:00:00.000Z", expiresAt: "2026-11-10T10:10:00.000Z" },
  profile: {
    account: "synthetic-org-account",
    host: "synthetic-org-account.snowflakecomputing.com",
    username: "synthetic-login@example.test",
    database: "PRIVATE_DB_123",
    schema: "PRIVATE_SCHEMA_456",
    warehouse: "PRIVATE_WH_789",
    role: "PRIVATE_ROLE_012",
  },
  sources: Object.fromEntries(
    SOURCE_PATHS.map((name, index) => [
      name,
      createHash("sha256").update(String(index)).digest("hex"),
    ]),
  ),
  runtime: {
    format: "joinery-snowflake-runtime-v1",
    target: "darwin-arm64",
    nodeVersion: "24.22.2",
    nodeSha256: "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a",
    sdkVersion: "3.4.1",
    manifestSha256: "a".repeat(64),
  },
});
const rejects = (action, code) => assert.throws(action, (error) => error.safeCode === code);
function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "joinery-offline-manifest-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, ".cache"), { mode: 0o700 });
  const full = input();
  const { sources, runtime, ...request } = full;
  const filename = path.join(root, ".cache/request.json");
  fs.writeFileSync(filename, JSON.stringify(request), { mode: 0o600 });
  return {
    root,
    filename,
    full,
    loadEvidence: () => ({ sources: clone(sources), runtime: clone(runtime) }),
  };
}
function rehash(manifest) {
  manifest.manifestSha256 = createHash("sha256")
    .update(canonicalJson(manifest.payload))
    .digest("hex");
  return manifest;
}

test("fixed workflow expands exactly24 SQL,23 query commands and one nonmutating context command", () => {
  const manifest = compileManifest(input());
  const { commands, sqlLedger, caps } = manifest.payload;
  assert.equal(sqlLedger.length, 24);
  assert.equal(commands.length, 26);
  assert.equal(commands.filter((command) => command.op === "query").length, 23);
  assert.deepEqual(
    commands.filter((command) => command.op === "context"),
    [
      {
        id: "startup.identity",
        op: "context",
        payload: {},
        sqlOrdinals: [1],
        maximumInvocations: 1,
      },
    ],
  );
  assert.equal(
    sqlLedger[0].sql,
    "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()",
  );
  assert.equal(sqlLedger[1].sql, "SELECT CURRENT_VERSION()");
  const sample = [
    "SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME, CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME, CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID, TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH, CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES",
    "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION",
    "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
    "SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION",
    "SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID",
  ];
  assert.deepEqual(
    sqlLedger.slice(2, 7).map((step) => step.sql),
    sample,
  );
  assert.equal(
    sqlLedger[7].sql,
    "SELECT CATALOG_NAME, SCHEMA_NAME FROM \"PRIVATE_DB_123\".INFORMATION_SCHEMA.SCHEMATA WHERE CATALOG_NAME = 'PRIVATE_DB_123' AND SCHEMA_NAME <> 'INFORMATION_SCHEMA' ORDER BY SCHEMA_NAME LIMIT 1001",
  );
  assert.deepEqual(
    sqlLedger.slice(8, 13).map((step) => step.sql),
    sample,
  );
  assert.deepEqual(
    sqlLedger.slice(14).map((step) => step.sql),
    [...sample, ...sample],
  );
  assert.equal(
    sqlLedger[13].sql,
    "SELECT CAST(? AS NUMBER(38,0)) AS EXACT_INTEGER, CAST(? AS BOOLEAN) AS EXACT_BOOLEAN, CAST(? AS VARCHAR) AS EXACT_TEXT, CAST(? AS VARCHAR) AS EXACT_NULL",
  );
  assert.deepEqual(sqlLedger[13].binds, [
    { type: "integer", value: "9007199254740993" },
    { type: "boolean", value: true },
    { type: "text", value: "joinery-qualification" },
    { type: "text", value: null },
  ]);
  assert.deepEqual(commands.find((command) => command.id === "scalar").payload.binds, [
    "9007199254740993",
    true,
    "joinery-qualification",
    null,
  ]);
  assert.deepEqual(manifest.payload.operationGroups, {
    startup: [1, 2],
    options: [3, 13],
    scalar: [14, 14],
    state: [15, 24],
  });
  assert.deepEqual(
    sqlLedger.map((step) => step.ordinal),
    Array.from({ length: 24 }, (_, index) => index + 1),
  );
  assert(sqlLedger.every((step) => step.maximumInvocations === 1));
  assert(sqlLedger.every((step) => step.sql.startsWith("SELECT ") || sample.includes(step.sql)));
  assert.equal(caps.sqlStatements, 24);
  assert.equal(caps.sessions, 1);
  assert.equal(manifest.payload.executionStatus, "offlineOnly");
  assert.equal(manifest.payload.runner, null);
});

test("resource,deadline,credit and cleanup contracts remain bounded without execution promises", () => {
  const { payload } = compileManifest(input());
  assert.equal(payload.sqlLedger.filter((step) => step.templateId === "schemas").length, 1);
  assert.deepEqual(payload.sqlLedger[7].limits, {
    maximumRows: 1001,
    maximumBytes: 2097152,
    maximumColumns: 2,
  });
  assert(
    payload.sqlLedger
      .filter((_, index) => index !== 7)
      .every((step) => step.limits.maximumRows === 1 && step.limits.maximumBytes === 65536),
  );
  assert.equal(payload.caps.maximumAttemptedRows, 1001 + 23);
  assert.equal(payload.caps.maximumSuccessfulRows, 1000 + 23);
  assert(
    payload.sqlLedger.reduce((sum, step) => sum + step.limits.maximumBytes, 0) <=
      payload.caps.maximumResultContentBytes,
  );
  assert.equal(payload.caps.authenticationMs, 120000);
  assert.equal(payload.caps.sqlMs, 30000);
  assert.equal(payload.caps.optionsOperationMs, 65000);
  assert.equal(payload.caps.stateOperationMs, 65000);
  assert.equal(payload.caps.admissionWindowMs, 600000);
  assert.equal(payload.controls.credit.maximumAdditionalRequests, 1000);
  assert.equal(payload.controls.credit.replenishAfterConsumedNonemptyBatch, 1);
  assert.equal(payload.controls.cancel.acknowledgementProvesOriginalTerminal, false);
  assert.equal(payload.cleanup.sqlStatements, 0);
  assert.equal(payload.cleanup.requireActualReapBeforeIdentityCheckedRootRemoval, true);
  assert.equal(payload.cleanup.remoteRollbackGuaranteed, false);
  assert.equal(payload.approval.priorCanaryApprovalReusable, false);
});

test("canonical ordering is stable,outputs are immutable,and invalid JSON representations fail", () => {
  const original = input();
  const reordered = Object.fromEntries(Object.entries(original).reverse());
  reordered.profile = Object.fromEntries(Object.entries(original.profile).reverse());
  reordered.sources = Object.fromEntries(Object.entries(original.sources).reverse());
  assert.equal(compileManifest(original).manifestSha256, compileManifest(reordered).manifestSha256);
  const manifest = compileManifest(original);
  original.profile.database = "LATER_DATABASE";
  assert.equal(manifest.payload.commands[0].payload.database, "PRIVATE_DB_123");
  assert.throws(() => {
    manifest.payload.sqlLedger[0].sql = "DELETE FROM PRIVATE";
  }, TypeError);
  for (const value of [
    undefined,
    NaN,
    Infinity,
    -0,
    1.5,
    1n,
    new Date(),
    [,],
    [undefined],
    { value: undefined },
  ])
    rejects(() => canonicalJson(value), "invalidInput");
  let read = false;
  const accessor = Object.defineProperty({}, "sql", {
    enumerable: true,
    get() {
      read = true;
      return "SQL";
    },
  });
  rejects(() => canonicalJson(accessor), "invalidInput");
  assert.equal(read, false);
  rejects(() => canonicalJson("x".repeat(256 * 1024 + 1)), "inputTooLarge");
});

test("profile values require actual bounded strings and cannot carry SQL,credentials or route overrides", () => {
  for (const key of Object.keys(input().profile)) {
    for (const value of [null, true, 12, [], [input().profile[key]], {}, "", "x\n"]) {
      const candidate = input();
      candidate.profile[key] = value;
      assert.throws(() => compileManifest(candidate), undefined, `${key}:${JSON.stringify(value)}`);
    }
  }
  for (const value of ['DB"; DROP SCHEMA X;--', "DB.SCHEMA", "DB'", "lowercase", "A".repeat(256)]) {
    const candidate = input();
    candidate.profile.database = value;
    rejects(() => compileManifest(candidate), "invalidProfile");
  }
  const boundary = input();
  boundary.profile.database = "A".repeat(255);
  assert.equal(
    compileManifest(boundary).payload.sqlLedger[7].sql.includes(`"${boundary.profile.database}"`),
    true,
  );
  for (const key of [
    "password",
    "token",
    "authenticator",
    "sql",
    "executable",
    "scope",
    "approval",
    "binds",
  ]) {
    const candidate = input();
    candidate.profile[key] = "untrusted";
    rejects(() => compileManifest(candidate), "invalidInput");
  }
  const host = input();
  host.profile.host = "attacker.example.test";
  rejects(() => compileManifest(host), "invalidProfile");
});

test("unsupported app and account stages are explicit; windows,IDs,source and runtime schemas are strict", () => {
  for (const workflow of ["A1", "A2", "A3", "B1", "B2", "session-options-scalar-state-v2"]) {
    const candidate = input();
    candidate.workflow = workflow;
    rejects(() => compileManifest(candidate), "unsupportedWorkflow");
  }
  for (const route of ["app", "packaged-app", "oauth", null]) {
    const candidate = input();
    candidate.route = route;
    rejects(() => compileManifest(candidate), "unsupportedRoute");
  }
  for (const runId of ["a".repeat(32), "0".repeat(32), "A".repeat(31), 123]) {
    const candidate = input();
    candidate.runId = runId;
    rejects(() => compileManifest(candidate), "invalidRunId");
  }
  for (const expiresAt of [
    "2026-11-10T10:10:00.001Z",
    "2026-11-10T10:00:00.000Z",
    "tomorrow",
    "2026-11-10T10:10:00Z",
  ]) {
    const candidate = input();
    candidate.window.expiresAt = expiresAt;
    rejects(() => compileManifest(candidate), "invalidWindow");
  }
  for (const key of [
    "nodeVersion",
    "nodeSha256",
    "sdkVersion",
    "manifestSha256",
    "target",
    "format",
  ]) {
    const candidate = input();
    candidate.runtime[key] = "incorrect";
    rejects(() => compileManifest(candidate), "invalidRuntime");
  }
  const missing = input();
  delete missing.sources[SOURCE_PATHS[0]];
  rejects(() => compileManifest(missing), "invalidInput");
  const source = input();
  source.sources[SOURCE_PATHS[0]] = "A".repeat(64);
  rejects(() => compileManifest(source), "invalidSources");
  const extra = input();
  extra.sources["unreviewed.mjs"] = "a".repeat(64);
  rejects(() => compileManifest(extra), "invalidInput");
  const authority = input();
  authority.approved = true;
  rejects(() => compileManifest(authority), "invalidInput");
});

test("verification rejects unhashed tampering,rehashed edits and independent current-evidence drift", () => {
  const original = compileManifest(input());
  assert.deepEqual(verifyManifest(clone(original), input()), original);
  for (const mutate of [
    (value) => {
      value.payload.sqlLedger[0].sql = "SELECT 1";
    },
    (value) => {
      value.payload.commands[0].payload.host = "changed.example.test";
    },
    (value) => {
      value.payload.caps.sqlStatements = 25;
    },
    (value) => {
      value.payload.sqlLedger[13].binds[0].value = 42;
    },
    (value) => {
      value.payload.templateSha256 = "b".repeat(64);
    },
    (value) => {
      value.payload.executionStatus = "approved";
    },
  ]) {
    const changed = clone(original);
    mutate(changed);
    rejects(() => verifyManifest(changed, input()), "manifestTampered");
    rejects(() => verifyManifest(rehash(changed), input()), "manifestDrift");
  }
  for (const mutate of [
    (value) => {
      value.sources[SOURCE_PATHS[0]] = "f".repeat(64);
    },
    (value) => {
      value.runtime.manifestSha256 = "b".repeat(64);
    },
    (value) => {
      value.profile.schema = "CHANGED_SCHEMA";
    },
    (value) => {
      value.runId = "B2".repeat(16);
    },
  ]) {
    const current = input();
    mutate(current);
    rejects(() => verifyManifest(original, current), "manifestDrift");
  }
});

test("independent current-source check rejects observer,metadata,setup and schema-limit template drift", () => {
  const sources = Object.fromEntries(
    SOURCE_PATHS.map((filename) => [
      filename,
      fs.readFileSync(new URL(`../${filename}`, import.meta.url), "utf8"),
    ]),
  );
  validateSourceTemplates(sources);
  for (const [filename, before, after] of [
    [
      "src-tauri/src/drivers/snowflake_session.rs",
      "SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION",
      "SHOW PARAMETERS LIKE 'OTHER' IN SESSION",
    ],
    [
      "src-tauri/src/drivers/snowflake_session.rs",
      "CAST(CURRENT_TRANSACTION() AS VARCHAR)",
      "CURRENT_TRANSACTION()",
    ],
    [
      "src-tauri/src/drivers/snowflake_metadata.rs",
      "CURRENT_USER() AS PRINCIPAL",
      "CURRENT_USER() AS OTHER_PRINCIPAL",
    ],
    [
      "src-tauri/src/drivers/snowflake_metadata.rs",
      "TO_VARCHAR(CURRENT_SCHEMAS())",
      "TO_JSON(CURRENT_SCHEMAS())",
    ],
    [
      "src-tauri/src/drivers/snowflake_metadata.rs",
      "const SCHEMA_LIMIT: usize = 1000;",
      "const SCHEMA_LIMIT: usize = 999;",
    ],
    [
      "src-tauri/src/drivers/snowflake_metadata.rs",
      "SCHEMATA WHERE CATALOG_NAME",
      "SCHEMATA WHERE OTHER_NAME",
    ],
    [
      "snowflake-helper/src/helper.mjs",
      "SELECT CURRENT_USER(), CURRENT_ROLE(),",
      "SELECT CURRENT_USER(), CURRENT_DATE(),",
    ],
    [
      "src-tauri/src/drivers/snowflake_external.rs",
      '"SELECT CURRENT_VERSION()"',
      '"SELECT CURRENT_DATE()"',
    ],
  ]) {
    const changed = { ...sources, [filename]: sources[filename].replace(before, after) };
    assert.notEqual(changed[filename], sources[filename], "mutation must change actual source");
    rejects(() => validateSourceTemplates(changed), "sourceTemplateDrift");
  }
});

test("dry-run prints only redacted hashes/counts and creates no files,helpers or network work", (t) => {
  const f = fixture(t);
  const before = fs.readdirSync(f.root, { recursive: true });
  const deny = () => {
    throw Error("unexpected process or network attempt");
  };
  for (const method of [
    "spawn",
    "spawnSync",
    "exec",
    "execSync",
    "execFile",
    "execFileSync",
    "fork",
  ])
    t.mock.method(childProcess, method, deny);
  t.mock.method(net, "connect", deny);
  t.mock.method(net, "createConnection", deny);
  t.mock.method(tls, "connect", deny);
  t.mock.method(dns, "lookup", deny);
  t.mock.method(globalThis, "fetch", deny);
  syncBuiltinESMExports();
  t.after(() => {
    t.mock.restoreAll();
    syncBuiltinESMExports();
  });
  const summary = main(["--input", ".cache/request.json"], {
    repoRoot: f.root,
    loadEvidence: f.loadEvidence,
  });
  assert.equal(summary.mode, "dryRun");
  assert.equal(summary.executionStatus, "offlineOnly");
  assert.equal(summary.runnerImplemented, false);
  assert.equal(summary.authorizationGranted, false);
  const printed = JSON.stringify(summary);
  for (const value of [
    ...Object.values(f.full.profile),
    "SELECT ",
    "SHOW ",
    "9007199254740993",
    "joinery-qualification",
  ])
    assert.equal(printed.includes(value), false);
  assert.deepEqual(fs.readdirSync(f.root, { recursive: true }), before);
  assert.equal(summarizeManifest(compileManifest(f.full)).manifestSha256, summary.manifestSha256);
});

test("CLI args cannot enable execution or redirect runtime,SQL,approval or private-state inputs", (t) => {
  const f = fixture(t);
  let evidenceReads = 0;
  const options = {
    repoRoot: f.root,
    loadEvidence: () => {
      evidenceReads++;
      return f.loadEvidence();
    },
  };
  for (const args of [
    [],
    ["--run"],
    ["--input"],
    ["--input", "a", "--input", "b"],
    ["--input", "a", "--emit", "b", "--verify", "c"],
    ["--runtime", "elsewhere"],
    ["--sql", "SELECT 1"],
    ["--approval", "old-canary"],
    ["--input", "--run"],
  ])
    rejects(() => main(args, options), "invalidArguments");
  assert.equal(evidenceReads, 0);
});

test("input rejects escapes,links,nonregular files,permissions,owners and oversize before reading", (t) => {
  const f = fixture(t);
  const options = {
    repoRoot: f.root,
    loadEvidence: () => {
      throw Error("must not inspect evidence");
    },
  };
  rejects(() => main(["--input", "../outside.json"], options), "unsafeLocalPath");
  fs.writeFileSync(path.join(f.root, "tracked.json"), "{}", { mode: 0o600 });
  rejects(() => main(["--input", "tracked.json"], options), "unsafeLocalPath");
  fs.symlinkSync(f.filename, path.join(f.root, ".cache/link.json"));
  rejects(() => main(["--input", ".cache/link.json"], options), "unsafeLocalFile");
  fs.mkdirSync(path.join(f.root, ".cache/directory.json"));
  rejects(() => main(["--input", ".cache/directory.json"], options), "unsafeLocalFile");
  fs.chmodSync(f.filename, 0o644);
  rejects(() => main(["--input", ".cache/request.json"], options), "unsafeLocalFile");
  fs.chmodSync(f.filename, 0o600);
  fs.writeFileSync(path.join(f.root, ".cache/large.json"), "x".repeat(256 * 1024 + 1), {
    mode: 0o600,
  });
  rejects(() => main(["--input", ".cache/large.json"], options), "unsafeLocalFile");
  const originalStat = fs.lstatSync;
  const fake = t.mock.method(fs, "lstatSync", (filename, ...args) => {
    const stat = originalStat(filename, ...args);
    if (filename === f.filename) stat.uid += 1;
    return stat;
  });
  syncBuiltinESMExports();
  rejects(() => main(["--input", ".cache/request.json"], options), "unsafeLocalFile");
  fake.mock.restore();
  syncBuiltinESMExports();
});

test("explicit emit is private,exclusive,restricted to existing ignored root and independently verifiable", (t) => {
  const f = fixture(t);
  const options = { repoRoot: f.root, loadEvidence: f.loadEvidence };
  const root = path.join(f.root, ".cache/snowflake-qualification");
  fs.mkdirSync(root, { mode: 0o700 });
  const filename = ".cache/snowflake-qualification/review.json";
  const args = ["--input", ".cache/request.json", "--emit", filename];
  const summary = main(args, options);
  assert.equal(summary.mode, "emittedOfflineManifest");
  assert.equal(fs.statSync(path.join(f.root, filename)).mode & 0o7777, 0o600);
  const bytes = fs.readFileSync(path.join(f.root, filename));
  assert.equal(JSON.parse(bytes).manifestSha256, summary.manifestSha256);
  assert.throws(
    () => main(args, options),
    (error) => error.code === "EEXIST",
  );
  assert.deepEqual(fs.readFileSync(path.join(f.root, filename)), bytes);
  assert.equal(
    main(["--input", ".cache/request.json", "--verify", filename], options).mode,
    "verifiedOfflineManifest",
  );
  const changed = {
    repoRoot: f.root,
    loadEvidence: () => {
      const current = f.loadEvidence();
      current.runtime.manifestSha256 = "b".repeat(64);
      return current;
    },
  };
  rejects(
    () => main(["--input", ".cache/request.json", "--verify", filename], changed),
    "manifestDrift",
  );
  rejects(
    () =>
      main(["--input", ".cache/request.json", "--emit", ".cache/untracked-output.json"], options),
    "unsafeLocalPath",
  );
  fs.chmodSync(root, 0o755);
  rejects(
    () =>
      main(
        ["--input", ".cache/request.json", "--emit", ".cache/snowflake-qualification/new.json"],
        options,
      ),
    "unsafeOutputDirectory",
  );
  assert.deepEqual(fs.readFileSync(path.join(f.root, filename)), bytes);
});

test("symlinked parents and existing output links cannot redirect a private artifact", (t) => {
  const f = fixture(t);
  const options = { repoRoot: f.root, loadEvidence: f.loadEvidence };
  const privateRoot = path.join(f.root, ".cache/private");
  fs.mkdirSync(privateRoot, { mode: 0o700 });
  fs.symlinkSync(privateRoot, path.join(f.root, ".cache/snowflake-qualification"));
  rejects(
    () =>
      main(
        ["--input", ".cache/request.json", "--emit", ".cache/snowflake-qualification/result.json"],
        options,
      ),
    "unsafeLocalPath",
  );
  fs.unlinkSync(path.join(f.root, ".cache/snowflake-qualification"));
  fs.mkdirSync(path.join(f.root, ".cache/snowflake-qualification"), { mode: 0o700 });
  fs.symlinkSync(f.filename, path.join(f.root, ".cache/snowflake-qualification/result.json"));
  const before = fs.readFileSync(f.filename);
  assert.throws(() =>
    main(
      ["--input", ".cache/request.json", "--emit", ".cache/snowflake-qualification/result.json"],
      options,
    ),
  );
  assert.deepEqual(fs.readFileSync(f.filename), before);
});
