import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { syncBuiltinESMExports } from "node:module";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import {
  main,
  compileExecutionPlan,
  verifyExecutionPlan,
  RUNNER_SOURCES,
  APPROVAL_SCOPE,
} from "./snowflake-qualification-runner.mjs";
import { WORKFLOW, SOURCE_PATHS } from "./snowflake-qualification-manifest.mjs";

const clone = (value) => JSON.parse(JSON.stringify(value));
function fixture(t) {
  const root = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "joinery-qualification-runner-")),
  );
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const cache = path.join(root, ".cache");
  const output = path.join(cache, "snowflake-qualification");
  fs.mkdirSync(cache, { mode: 0o700 });
  fs.mkdirSync(output, { mode: 0o700 });
  const now = Date.now();
  const request = {
    workflow: WORKFLOW,
    route: "helper-direct",
    runId: "A1".repeat(16),
    window: {
      notBefore: new Date(now - 1000).toISOString(),
      expiresAt: new Date(now + 300000).toISOString(),
    },
    profile: {
      account: "synthetic-account",
      host: "synthetic-account.snowflakecomputing.com",
      username: "login@example.test",
      database: "DB",
      schema: "PUBLIC",
      warehouse: "WH",
      role: "ROLE",
    },
  };
  const evidence = {
    sources: Object.fromEntries(
      SOURCE_PATHS.map((file, index) => [file, index.toString(16).padStart(64, "0")]),
    ),
    runtime: {
      format: "joinery-snowflake-runtime-v1",
      target: "darwin-arm64",
      nodeVersion: "24.22.2",
      nodeSha256: "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a",
      sdkVersion: "3.4.1",
      manifestSha256: "a".repeat(64),
    },
  };
  const sources = Object.fromEntries(RUNNER_SOURCES.map((file) => [file, "b".repeat(64)]));
  const full = { ...request, ...evidence };
  const plan = compileExecutionPlan(full, sources);
  const inputFile = path.join(cache, "request.json"),
    planFile = path.join(output, "plan.json"),
    approvalFile = path.join(output, "approval.json");
  const write = (file, value) => fs.writeFileSync(file, JSON.stringify(value), { mode: 0o600 });
  write(inputFile, request);
  write(planFile, plan);
  const approve = (changes = {}) =>
    write(approvalFile, {
      format: "joinery-snowflake-qualification-approval-v1",
      executionPlanSha256: plan.executionPlanSha256,
      scope: APPROVAL_SCOPE,
      maximumAttempts: 1,
      ...changes,
    });
  let spawns = 0;
  const options = {
    repoRoot: root,
    loadEvidence: () => clone(evidence),
    loadRunnerSources: () => clone(sources),
    platform: "darwin",
    arch: "arm64",
    spawnChild: () => {
      spawns++;
      throw Error("PRIVATE_SPAWN_ERROR");
    },
  };
  return {
    root,
    cache,
    output,
    request,
    evidence,
    sources,
    full,
    plan,
    inputFile,
    planFile,
    approvalFile,
    options,
    write,
    approve,
    args: ["--input", inputFile, "--plan", planFile, "--approval", approvalFile, "--run"],
    spawns: () => spawns,
    consumed: () => fs.existsSync(path.join(output, `consumed-${plan.executionPlanSha256}.json`)),
  };
}
const code = (expected) => (error) => error.safeCode === expected;
const redacted = (value) => {
  const text = JSON.stringify(value);
  for (const secret of [
    "login@example.test",
    "synthetic-account",
    "SERVER_NAME",
    "opaque-session",
    "PRIVATE_SPAWN_ERROR",
    "PRIVATE_STDERR",
  ])
    assert.equal(text.includes(secret), false, secret);
};

test("execution envelope binds offline ledger, all runner sources and physical caps without mutating v1", (t) => {
  const f = fixture(t);
  assert.equal(f.plan.payload.ledger.payload.executionStatus, "offlineOnly");
  assert.equal(f.plan.payload.ledger.payload.runner, null);
  assert.equal(f.plan.payload.maximumAttempts, 1);
  assert.equal(f.plan.payload.physicalLimits.stderrBytes, 65536);
  assert.equal(f.plan.payload.physicalLimits.inboundBytes, 8 * 1024 * 1024);
  assert.equal(
    verifyExecutionPlan(f.plan, f.full, f.sources).executionPlanSha256,
    f.plan.executionPlanSha256,
  );
  for (const name of RUNNER_SOURCES) {
    const changed = clone(f.sources);
    changed[name] = "c".repeat(64);
    assert.throws(() => verifyExecutionPlan(f.plan, f.full, changed), code("executionPlanDrift"));
  }
  const changed = clone(f.plan);
  changed.payload.physicalLimits.stderrBytes++;
  assert.throws(() => verifyExecutionPlan(changed, f.full, f.sources), code("executionPlanDrift"));
});

test("dry-run, emit and verify never spawn, consume approval or allocate authentication state", async (t) => {
  const f = fixture(t);
  const before = fs.readdirSync(f.output);
  const dry = await main(["--input", f.inputFile], f.options);
  assert.equal(dry.mode, "dryRun");
  redacted(dry);
  assert.deepEqual(fs.readdirSync(f.output), before);
  const emitted = path.join(f.output, "emitted.json");
  await main(["--input", f.inputFile, "--emit", emitted], f.options);
  assert.equal(fs.statSync(emitted).mode & 0o777, 0o600);
  assert.deepEqual(JSON.parse(fs.readFileSync(emitted)), f.plan);
  const verified = await main(["--input", f.inputFile, "--plan", emitted], f.options);
  assert.equal(verified.mode, "verifiedExecutionPlan");
  assert.equal(verified.approvalConsumed, false);
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
  assert.equal(
    fs.readdirSync(f.output).some((name) => name.startsWith("auth-")),
    false,
  );
  await assert.rejects(main(["--input", f.inputFile, "--emit", emitted], f.options), {
    code: "EEXIST",
  });
});

test("CLI admits no arbitrary executable, SQL, timeout or authority override", async (t) => {
  const f = fixture(t);
  for (const args of [
    [],
    ["--input", f.inputFile, "--run"],
    ["--input", f.inputFile, "--approval", f.approvalFile],
    ["--input", f.inputFile, "--node", "/bin/false"],
    ["--input", f.inputFile, "--sql", "SELECT 1"],
    ["--input", f.inputFile, "--timeout", "999999"],
    ["--input", f.inputFile, "--input", f.inputFile],
    ["--input", f.inputFile, "--emit", f.planFile, "--run", "--approval", f.approvalFile],
  ])
    await assert.rejects(main(args, f.options), code("invalidArguments"));
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
});

test("approval must name this exact execution plan, scope and one-attempt allowance", async (t) => {
  const f = fixture(t);
  for (const changes of [
    { format: "joinery-snowflake-read-canary-v1" },
    { executionPlanSha256: "c".repeat(64) },
    { executionPlanSha256: f.plan.payload.ledger.manifestSha256 },
    { scope: "read-canary" },
    { maximumAttempts: 2 },
    { extra: true },
  ]) {
    f.approve(changes);
    await assert.rejects(main(f.args, f.options), code("invalidApproval"));
  }
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
});

test("expired/future windows, pre-abort and unsupported platform fail before consumption", async (t) => {
  const f = fixture(t);
  f.approve();
  await assert.rejects(
    main(f.args, { ...f.options, platform: "linux" }),
    code("unsupportedPlatform"),
  );
  const owner = new AbortController();
  owner.abort();
  await assert.rejects(
    main(f.args, { ...f.options, signal: owner.signal }),
    code("ownerCancelledBeforeAttempt"),
  );
  for (const offset of [-10000, 10000]) {
    const request = clone(f.request);
    request.window = {
      notBefore: new Date(Date.now() + offset).toISOString(),
      expiresAt: new Date(Date.now() + offset + 1000).toISOString(),
    };
    f.write(f.inputFile, request);
    const plan = compileExecutionPlan({ ...request, ...f.evidence }, f.sources);
    f.write(f.planFile, plan);
    await assert.rejects(main(f.args, f.options), code("executionWindowUnavailable"));
  }
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
});

test("source/runtime drift at either preflight blocks execution and preserves approval", async (t) => {
  const f = fixture(t);
  f.approve();
  const changed = clone(f.evidence);
  changed.runtime.manifestSha256 = "c".repeat(64);
  await assert.rejects(
    main(f.args, { ...f.options, loadEvidence: () => changed }),
    code("executionPlanDrift"),
  );
  let reads = 0;
  await assert.rejects(
    main(f.args, {
      ...f.options,
      loadRunnerSources: () => {
        const sources = clone(f.sources);
        if (++reads > 1) sources[RUNNER_SOURCES[0]] = "c".repeat(64);
        return sources;
      },
    }),
    code("executionPlanDrift"),
  );
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
});

test("unsafe approval permissions, symlink inputs and output roots fail before spawn", async (t) => {
  const f = fixture(t);
  f.approve();
  fs.chmodSync(f.approvalFile, 0o644);
  await assert.rejects(main(f.args, f.options), code("unsafeLocalFile"));
  fs.chmodSync(f.approvalFile, 0o600);
  fs.renameSync(f.inputFile, `${f.inputFile}.real`);
  fs.symlinkSync(`${f.inputFile}.real`, f.inputFile);
  await assert.rejects(main(f.args, f.options), code("unsafeLocalFile"));
  fs.unlinkSync(f.inputFile);
  fs.renameSync(`${f.inputFile}.real`, f.inputFile);
  fs.chmodSync(f.output, 0o755);
  await assert.rejects(main(f.args, f.options), code("unsafeOutputDirectory"));
  fs.chmodSync(f.output, 0o700);
  fs.renameSync(f.output, `${f.output}.real`);
  fs.symlinkSync(`${f.output}.real`, f.output);
  await assert.rejects(main(f.args, f.options), code("unsafeLocalPath"));
  assert.equal(f.spawns(), 0);
  assert.equal(f.consumed(), false);
});

test("spawn failure consumes one attempt permanently and removes only its private root", async (t) => {
  const f = fixture(t);
  f.approve();
  const result = await main(f.args, f.options);
  assert.equal(result.approvalConsumed, true);
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "localRunnerFailure");
  assert.equal(result.localCleanup.stateRemoved, true);
  assert.equal(result.localCleanup.childSpawned, false);
  assert.equal(f.spawns(), 1);
  assert.equal(f.consumed(), true);
  redacted(result);
  await assert.rejects(main(f.args, f.options), code("evidenceAlreadyExists"));
  // Removing the result does not restore the one-use grant, even from a copy.
  fs.unlinkSync(path.join(f.output, `result-${f.plan.executionPlanSha256}.json`));
  await assert.rejects(main(f.args, f.options), { code: "EEXIST" });
  assert.equal(f.spawns(), 1);
  for (const name of fs.readdirSync(f.output))
    assert.equal(fs.statSync(path.join(f.output, name)).mode & 0o777, 0o600);
});

test("directory-sync failure retains consumption and prevents auth-root allocation or spawn", async (t) => {
  const f = fixture(t);
  f.approve();
  const original = fs.fsyncSync;
  fs.fsyncSync = (fd) => {
    if (fs.fstatSync(fd).isDirectory()) throw Error("PRIVATE_DIRECTORY_SYNC_FAILURE");
    return original(fd);
  };
  syncBuiltinESMExports();
  try {
    await assert.rejects(main(f.args, f.options), code("consumptionDurabilityUnconfirmed"));
  } finally {
    fs.fsyncSync = original;
    syncBuiltinESMExports();
  }
  assert.equal(f.consumed(), true);
  assert.equal(f.spawns(), 0);
  assert.equal(
    fs.readdirSync(f.output).some((name) => name.startsWith("auth-")),
    false,
  );
  await assert.rejects(main(f.args, f.options), { code: "EEXIST" });
});

test("actual OS launch failure proves no child after error plus close, without claiming reaping", async (t) => {
  const f = fixture(t);
  f.approve();
  const result = await main(f.args, {
    ...f.options,
    spawnChild() {
      return spawn(path.join(f.root, "missing-executable"), [], {
        stdio: ["pipe", "pipe", "pipe"],
      });
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "childProcessError");
  assert.equal(result.localCleanup.spawnAttempted, true);
  assert.equal(result.localCleanup.processObjectCreated, true);
  assert.equal(result.localCleanup.childSpawned, false);
  assert.equal(result.localCleanup.childReaped, false);
  assert.equal(result.localCleanup.noChildConfirmed, true);
  assert.equal(result.localCleanup.stateRemoved, true);
});

test("pre-existing evidence cannot be overwritten or followed after an attempt", async (t) => {
  const f = fixture(t);
  f.approve();
  const filename = path.join(f.output, `result-${f.plan.executionPlanSha256}.json`);
  const sentinel = path.join(f.cache, "sentinel");
  fs.writeFileSync(sentinel, "retain");
  fs.symlinkSync(sentinel, filename);
  await assert.rejects(main(f.args, f.options), code("evidenceAlreadyExists"));
  assert.equal(f.consumed(), false);
  assert.equal(f.spawns(), 0);
  assert.equal(fs.readFileSync(sentinel, "utf8"), "retain");
});

const start = (source) =>
  spawn(process.execPath, ["--input-type=module", "-e", source], {
    stdio: ["pipe", "pipe", "pipe"],
  });

function successfulHelperSource(f) {
  const cols = (names, types) =>
    names.map((name, index) => ({ name, type: types?.[index] ?? "TEXT" }));
  const data = {
    startupIdentity: {
      columns: cols(["a", "b", "c", "d", "e", "f"]),
      rows: [["SERVER_NAME", "ROLE", "DB", "PUBLIC", "WH", "opaque-session"]],
    },
    version: { columns: cols(["CURRENT_VERSION()"]), rows: [["synthetic-version"]] },
    context: {
      columns: cols([
        "PRINCIPAL",
        "ROLE_NAME",
        "DATABASE_NAME",
        "SCHEMA_NAME",
        "WAREHOUSE_NAME",
        "SESSION_ID",
        "SEARCH_PATH",
        "SECONDARY_ROLES",
      ]),
      rows: [
        [
          "SERVER_NAME",
          "ROLE",
          "DB",
          "PUBLIC",
          "WH",
          "opaque-session",
          '["DB.PUBLIC"]',
          '{"roles":"","value":"ALL"}',
        ],
      ],
    },
    transaction: { columns: cols(["TRANSACTION_ID"]), rows: [[null]] },
    schemas: {
      columns: cols(["CATALOG_NAME", "SCHEMA_NAME"]),
      rows: [
        ["DB", "PUBLIC"],
        ["DB", "S1"],
      ],
    },
    scalar: {
      columns: cols(
        ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"],
        ["FIXED", "BOOLEAN", "TEXT", "TEXT"],
      ),
      rows: [["9007199254740993", "TRUE", "joinery-qualification", null]],
    },
  };
  for (const [key, name, value] of [
    ["autocommit", "AUTOCOMMIT", "true"],
    ["abortOnError", "TRANSACTION_ABORT_ON_ERROR", "false"],
    ["quotedIdentifierPolicy", "QUOTED_IDENTIFIERS_IGNORE_CASE", "false"],
  ])
    data[key] = {
      columns: cols(["key", "value", "default", "level"]),
      rows: [[name, value, "unused", "SESSION"]],
    };
  // Actual subprocess + production Helper, synthetic SDK only. This does not
  // execute the fixed prepared binary, authenticate, use network or qualify DB.
  return `
    import {Helper} from ${JSON.stringify(new URL("../snowflake-helper/src/helper.mjs", import.meta.url).href)};
    import {Decoder,encode} from ${JSON.stringify(new URL("../snowflake-helper/src/framing.mjs", import.meta.url).href)};
    import {Readable} from 'node:stream';
    const ledger=${JSON.stringify(f.plan.payload.ledger.payload.sqlLedger)},data=${JSON.stringify(data)};
    let executed=0;
    const sdk={createConnection(){return {
      connectAsync(callback){queueMicrotask(()=>callback(null));return Promise.resolve();},
      destroy(callback){queueMicrotask(()=>callback(null));},
      execute(options){const entry=ledger[executed++];if(!entry||entry.sql!==options.sqlText)throw Error('unexpected synthetic SQL');
        const result=data[entry.templateId],id=executed;
        const stmt={getRequestId:()=> '00000000-0000-4000-8000-'+String(id).padStart(12,'0'),getQueryId:()=> '00000000-0000-4000-8000-'+String(id+10000).padStart(12,'0'),
          getColumns:()=>result.columns.map(c=>({getName:()=>c.name,getType:()=>c.type})),streamRows:()=>Readable.from(result.rows),cancel(callback){callback(null);}};
        queueMicrotask(()=>options.complete(null,stmt));return stmt;}
    };}};
    const helper=new Helper(sdk,frame=>process.stdout.write(encode(frame)),()=>process.stdout.end(()=>process.exit(executed===24?0:4)),{clientConfigFile:'unused-synthetic'});
    const decoder=new Decoder();process.stdin.on('data',chunk=>{for(const frame of decoder.push(chunk))helper.receive(frame);});
    process.stdin.on('end',()=>helper.stop());
  `;
}

test("actual subprocess composes production Helper/full24 synthetic SDK ledger with one-use private execution boundary", async (t) => {
  const f = fixture(t);
  f.approve();
  const result = await main(f.args, {
    ...f.options,
    spawnChild() {
      return start(successfulHelperSource(f));
    },
  });
  assert.equal(result.qualificationPassed, true, JSON.stringify(result));
  assert.equal(result.exercise.outcomes.length, 24);
  assert.equal(result.exercise.cleanup.logoutAcknowledged, true);
  assert.equal(result.localCleanup.childReaped, true);
  assert.equal(result.localCleanup.stateRemoved, true);
  assert.equal(result.sourceRevalidated, true);
  assert.equal(result.evidencePersisted, true);
  assert.equal(f.consumed(), true);
  redacted(result);
  await assert.rejects(main(f.args, f.options), code("evidenceAlreadyExists"));
});

test("post-run source drift fails otherwise successful synthetic workflow while preserving known SQL outcomes", async (t) => {
  const f = fixture(t);
  f.approve();
  let reads = 0;
  const result = await main(f.args, {
    ...f.options,
    loadRunnerSources() {
      const sources = clone(f.sources);
      if (++reads === 3) sources[RUNNER_SOURCES[0]] = "c".repeat(64);
      return sources;
    },
    spawnChild() {
      return start(successfulHelperSource(f));
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "postRunSourceDrift");
  assert.equal(result.exercise.passed, true);
  assert.equal(result.exercise.outcomes.length, 24);
  assert.equal(result.sourceRevalidated, false);
  assert.equal(result.localCleanup.stateRemoved, true);
});

test("post-run evidence collision fails qualification without overwriting another local artifact", async (t) => {
  const f = fixture(t);
  f.approve();
  const file = path.join(f.output, `result-${f.plan.executionPlanSha256}.json`);
  const result = await main(f.args, {
    ...f.options,
    spawnChild() {
      fs.writeFileSync(file, "retain", { mode: 0o600 });
      return start(successfulHelperSource(f));
    },
  });
  assert.equal(result.exercise.passed, true);
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "evidencePersistenceFailed");
  assert.equal(result.evidencePersisted, false);
  assert.equal(fs.readFileSync(file, "utf8"), "retain");
  assert.equal(result.localCleanup.stateRemoved, true);
});
test("actual synthetic child early exit is reaped; private root is removed and evidence remains redacted", async (t) => {
  const f = fixture(t);
  f.approve();
  let spawnOptions;
  const result = await main(f.args, {
    ...f.options,
    spawnChild(executable, args, options) {
      assert.equal(executable, path.join(f.root, ".cache/snowflake-runtime/macos-arm64/bin/node"));
      assert.deepEqual(args, [
        path.join(f.root, ".cache/snowflake-runtime/macos-arm64/helper/main.mjs"),
      ]);
      assert.equal(options.shell, false);
      assert.deepEqual(Object.keys(options.env), ["JOINERY_SNOWFLAKE_STATE_DIR"]);
      spawnOptions = options;
      fs.writeFileSync(
        path.join(options.env.JOINERY_SNOWFLAKE_STATE_DIR, "private-cache"),
        "PRIVATE_STDERR",
        { mode: 0o600 },
      );
      return start('process.stderr.write("PRIVATE_STDERR");process.exit(7);');
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.ok(["childExitedEarly", "childStdoutEndedEarly"].includes(result.failureCode));
  assert.equal(result.localCleanup.childReaped, true);
  assert.equal(result.localCleanup.stateRemoved, true);
  assert.equal(fs.existsSync(spawnOptions.env.JOINERY_SNOWFLAKE_STATE_DIR), false);
  assert.equal(result.sourceRevalidated, true);
  redacted(result);
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(f.output, `result-${f.plan.executionPlanSha256}.json`))),
    result,
  );
});

test("replacement auth root is preserved after actual reap and fails qualification", async (t) => {
  const f = fixture(t);
  f.approve();
  let replacement;
  const result = await main(f.args, {
    ...f.options,
    spawnChild(_exe, _args, options) {
      replacement = options.env.JOINERY_SNOWFLAKE_STATE_DIR;
      fs.renameSync(replacement, `${replacement}.original`);
      fs.mkdirSync(replacement, { mode: 0o700 });
      fs.writeFileSync(path.join(replacement, "keep"), "retain");
      return start("process.exit(7)");
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.localCleanup.childReaped, true);
  assert.equal(result.localCleanup.stateRemoved, false);
  assert.equal(result.localCleanup.rootIdentityConfirmed, false);
  assert.equal(fs.readFileSync(path.join(replacement, "keep"), "utf8"), "retain");
  assert.equal(fs.existsSync(`${replacement}.original`), true);
});

test("pre-ownership child validation failure still triggers bounded actual process reaping", async (t) => {
  const f = fixture(t);
  f.approve();
  const result = await main(f.args, {
    ...f.options,
    spawnChild() {
      const child = start("setInterval(()=>{},1000)");
      child.stdin.write = undefined;
      return child;
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "localRunnerFailure");
  assert.equal(result.localCleanup.childReaped, true);
  assert.equal(result.localCleanup.stateRemoved, true);
});

test("emergency path closes owned pipes when exit preceded pre-ownership rejection", async (t) => {
  const f = fixture(t);
  f.approve();
  const child = new EventEmitter();
  child.stdin = new PassThrough();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.kill = () => {
    throw Error("must not signal an exited process");
  };
  const result = await main(f.args, {
    ...f.options,
    signal: {},
    spawnChild() {
      queueMicrotask(() => child.emit("exit", 0, null));
      return child;
    },
  });
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.failureCode, "localRunnerFailure");
  assert.equal(result.localCleanup.childReaped, true);
  assert.equal(result.localCleanup.stateRemoved, true);
  for (const stream of [child.stdin, child.stdout, child.stderr])
    assert.equal(stream.destroyed, true);
});
