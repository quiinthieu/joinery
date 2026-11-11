import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  SETUP_WORKFLOW,
  SETUP_SOURCE_PATHS,
  compileSetupManifest,
  verifySetupManifest,
  summarizeSetupManifest,
  main,
} from "./snowflake-qualification-setup-manifest.mjs";
import {
  canonicalJson,
  compileManifest as compileReadManifest,
  WORKFLOW as READ_WORKFLOW,
  SOURCE_PATHS as READ_SOURCE_PATHS,
} from "./snowflake-qualification-manifest.mjs";
import {
  deriveSetupLedger,
  setupTarget,
} from "../snowflake-helper/src/disposable-setup-ledger.mjs";

const clone = (value) => JSON.parse(JSON.stringify(value));
const input = () => ({
  workflow: SETUP_WORKFLOW,
  route: "helper-direct",
  runId: "A1".repeat(16),
  window: { notBefore: "2026-11-11T03:00:00.000Z", expiresAt: "2026-11-11T03:10:00.000Z" },
  profile: {
    account: "synthetic-account",
    host: "synthetic-account.snowflakecomputing.com",
    username: "synthetic@example.test",
    database: "EXISTING_DB",
    schema: "EXISTING_SCHEMA",
    warehouse: "SYNTHETIC_WH",
    role: "SYNTHETIC_ROLE",
  },
  sources: Object.fromEntries(
    SETUP_SOURCE_PATHS.map((name, i) => [
      name,
      createHash("sha256").update(String(i)).digest("hex"),
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

test("closed setup has exactly28 statements,3 CREATEs and no cleanup or existing-table reads", () => {
  const { payload } = compileSetupManifest(input());
  const { sqlLedger, commands } = payload;
  assert.equal(sqlLedger.length, 28);
  assert.deepEqual(
    sqlLedger.map((step) => step.index),
    Array.from({ length: 28 }, (_, i) => i + 1),
  );
  assert.deepEqual(
    sqlLedger.filter((step) => step.mutation).map((step) => step.index),
    [8, 14, 20],
  );
  assert.deepEqual(
    sqlLedger.filter((step) => step.mutation).map((step) => step.key),
    ["create.database", "create.schema", "create.table"],
  );
  assert.equal(commands.length, 30);
  assert.equal(commands.filter((command) => command.op === "setup").length, 26);
  assert.equal(commands[1].op, "context");
  assert.equal(commands[2].op, "query");
  assert.equal(payload.caps.sqlStatements, 28);
  assert.equal(payload.cleanup.sqlStatements, 0);
  assert.equal(payload.cleanup.databaseDropAuthorized, false);
  assert.equal(payload.executionStatus, "offlineOnly");
  assert.equal(payload.runner, null);
  const sql = sqlLedger.map((step) => step.sql).join("\n");
  assert.doesNotMatch(
    sql,
    /DROP|ROLLBACK|COMMIT;|INSERT|DELETE|UPDATE|OR REPLACE|IF NOT EXISTS|USE DATABASE|USE SCHEMA/,
  );
  assert.doesNotMatch(sql, /EXISTING_DB|EXISTING_SCHEMA/);
  assert.match(
    sqlLedger[22].sql,
    /FROM "JOINERY_ACCEPTANCE_[A-F0-9]{32}"\."SCRATCH"\."ROW_CHECKS"$/,
  );
  for (const step of sqlLedger) {
    assert.equal(step.maximumRows, 1);
    assert.equal(step.maximumBytes, 65536);
    assert.ok(step.maximumColumns <= 32);
    assert.equal(Object.isFrozen(step), true);
  }
});

test("CREATE context transitions and labels derive only from exact run token", () => {
  const ledger = deriveSetupLedger(input().runId);
  const target = setupTarget(input().runId);
  assert.equal(target.database, `JOINERY_ACCEPTANCE_${input().runId}`);
  assert.equal(target.label, `joinery-disposable-${input().runId}`);
  assert.deepEqual(
    [ledger[2].phase, ledger[8].phase, ledger[14].phase, ledger[23].phase],
    ["initial", "database", "schema", "schema"],
  );
  assert.match(ledger[7].sql, /^CREATE TRANSIENT DATABASE/);
  assert.match(ledger[13].sql, /^CREATE SCHEMA/);
  assert.match(ledger[19].sql, /^CREATE TABLE/);
  for (const i of [7, 13]) assert.ok(ledger[i].sql.includes(`COMMENT = '${target.label}'`));
  assert.equal(Object.isFrozen(ledger), true);
  for (const run of [
    "",
    "0".repeat(32),
    "a1".repeat(16),
    "A".repeat(31),
    "A".repeat(33),
    "A".repeat(31) + "'",
    {},
    null,
  ])
    rejects(() => deriveSetupLedger(run), "invalidSetupRunId");
});

test("tampered rehashed SQL, limits, context and cleanup cannot become an admitted manifest", () => {
  for (const change of [
    (p) => (p.sqlLedger[7].sql += "; DROP DATABASE OTHER"),
    (p) => (p.sqlLedger[7].mutation = false),
    (p) => (p.sqlLedger[8].phase = "initial"),
    (p) => (p.sqlLedger[22].maximumRows = 1000),
    (p) => (p.caps.sqlStatements = 29),
    (p) => (p.cleanup.databaseDropAuthorized = true),
    (p) => (p.approval.maximumAttempts = 2),
    (p) => (p.commands[3].payload.sql = "SELECT SECRET"),
    (p) => (p.target.database = "EXISTING_DB"),
  ]) {
    const candidate = clone(compileSetupManifest(input()));
    change(candidate.payload);
    candidate.manifestSha256 = createHash("sha256")
      .update(canonicalJson(candidate.payload))
      .digest("hex");
    rejects(() => verifySetupManifest(candidate, input()), "setupManifestMismatch");
  }
});

test("every source/runtime/profile/window binding and the closed input shape are enforced", () => {
  const base = input();
  const manifest = compileSetupManifest(base);
  for (const change of [
    (v) => (v.sources[SETUP_SOURCE_PATHS.at(-1)] = "b".repeat(64)),
    (v) => (v.runtime.manifestSha256 = "b".repeat(64)),
    (v) => (v.profile.database = "OTHER_DB"),
    (v) => (v.window.notBefore = "2026-11-11T03:01:00.000Z"),
    (v) => (v.runId = "B1".repeat(16)),
  ]) {
    const changed = clone(base);
    change(changed);
    rejects(() => verifySetupManifest(manifest, changed), "setupManifestMismatch");
  }
  for (const change of [
    (v) => (v.sql = "SELECT SECRET"),
    (v) => (v.profile.password = "synthetic-secret"),
    (v) => (v.sources.extra = "a".repeat(64)),
    (v) => delete v.sources[SETUP_SOURCE_PATHS[0]],
    (v) => (v.runtime.sdkVersion = "unreviewed"),
    (v) => (v.window.expiresAt = "2026-11-12T03:00:00.001Z"),
  ]) {
    const changed = clone(base);
    change(changed);
    assert.throws(() => compileSetupManifest(changed));
  }
  let getterCalls = 0;
  const accessor = input();
  Object.defineProperty(accessor, "profile", {
    enumerable: true,
    get() {
      getterCalls++;
      return base.profile;
    },
  });
  assert.throws(() => compileSetupManifest(accessor));
  assert.equal(getterCalls, 0);
  base.profile.database = "CHANGED_AFTER_COMPILE";
  assert.equal(manifest.payload.profile.database, "EXISTING_DB");
  assert.equal(Object.isFrozen(manifest.payload.profile), true);
});

test("new setup admits at most24h once while original read manifest keeps its10minute cap", () => {
  const candidate = input();
  candidate.window.expiresAt = "2026-11-12T03:00:00.000Z";
  const setup = compileSetupManifest(candidate);
  assert.equal(setup.payload.window.expiresAt, candidate.window.expiresAt);
  assert.equal(setup.payload.caps.admissionWindowMs, 86400000);
  assert.equal(setup.payload.caps.workflowMs, 360000);
  rejects(
    () =>
      compileReadManifest({
        ...candidate,
        workflow: READ_WORKFLOW,
        sources: Object.fromEntries(READ_SOURCE_PATHS.map((key) => [key, candidate.sources[key]])),
      }),
    "invalidWindow",
  );
  candidate.window.expiresAt = "2026-11-12T03:00:00.001Z";
  rejects(() => compileSetupManifest(candidate), "invalidWindow");
});

test("private offline CLI emits and verifies redacted plans but has no run or approval mode", (t) => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "joinery-setup-manifest-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, ".cache/snowflake-qualification"), { recursive: true, mode: 0o700 });
  const full = input();
  const { sources, runtime, ...request } = full;
  const filename = path.join(root, ".cache/request.json");
  fs.writeFileSync(filename, JSON.stringify(request), { mode: 0o600 });
  const options = { repoRoot: root, loadEvidence: () => ({ sources, runtime }) };
  const summary = main(["--input", filename], options);
  assert.equal(summary.accountActivityStarted, false);
  assert.equal(summary.offlineOnly, true);
  const rendered = JSON.stringify(summary);
  for (const secret of Object.values(full.profile)) assert.equal(rendered.includes(secret), false);
  assert.equal(rendered.includes(full.runId), false);
  const output = path.join(root, ".cache/snowflake-qualification/setup.json");
  assert.equal(main(["--input", filename, "--emit", output], options).mode, "emittedManifest");
  assert.equal(fs.statSync(output).mode & 0o777, 0o600);
  assert.equal(main(["--input", filename, "--verify", output], options).mode, "verifiedManifest");
  for (const action of ["--run", "--approve", "--sql", "--runtime"])
    rejects(() => main(["--input", filename, action, output], options), "invalidSetupArguments");
  assert.throws(() => main(["--input", filename, "--emit", output], options));
  fs.chmodSync(filename, 0o644);
  assert.throws(() => main(["--input", filename], options));
});
