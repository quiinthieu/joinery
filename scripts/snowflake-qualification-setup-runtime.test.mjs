import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";
import { randomUUID } from "node:crypto";
import {
  compileSetupManifest,
  SETUP_SOURCE_PATHS,
  SETUP_WORKFLOW,
} from "./snowflake-qualification-setup-manifest.mjs";
import { deriveSetupLedger } from "../snowflake-helper/src/disposable-setup-ledger.mjs";
import { createSetupProtocol } from "./snowflake-qualification-setup-protocol.mjs";
import { exerciseSetupChild } from "./snowflake-qualification-setup-child.mjs";
import { createSetupJournal, setupRunKey } from "./snowflake-qualification-setup-journal.mjs";
import {
  main,
  compileSetupExecutionPlan,
  verifySetupExecutionPlan,
  RUNNER_SOURCES,
  APPROVAL_SCOPE,
  APPROVAL_FORMAT,
  publicSetupResult,
} from "./snowflake-qualification-setup-runner.mjs";

const clone = (value) => JSON.parse(JSON.stringify(value));
function input() {
  return {
    workflow: SETUP_WORKFLOW,
    route: "helper-direct",
    runId: "0123456789ABCDEF0123456789ABCDEF",
    window: {
      notBefore: new Date(Date.now() - 1000).toISOString(),
      expiresAt: new Date(Date.now() + 300000).toISOString(),
    },
    profile: {
      account: "synthetic-account",
      host: "synthetic-account.snowflakecomputing.com",
      username: "fixture-login@example.test",
      database: "INITIAL_DB",
      schema: "PUBLIC",
      warehouse: "WH",
      role: "OWNER_ROLE",
    },
    sources: Object.fromEntries(SETUP_SOURCE_PATHS.map((name) => [name, "a".repeat(64)])),
    runtime: {
      format: "joinery-snowflake-runtime-v1",
      target: "darwin-arm64",
      nodeVersion: "24.22.2",
      nodeSha256: "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a",
      sdkVersion: "3.4.1",
      manifestSha256: "b".repeat(64),
    },
  };
}
function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "joinery-setup-runtime-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const output = path.join(root, ".cache/snowflake-qualification");
  fs.mkdirSync(path.join(root, ".cache"), { mode: 0o700 });
  fs.mkdirSync(output, { mode: 0o700 });
  const full = input(),
    { sources, runtime, ...request } = full;
  const runnerSources = Object.fromEntries(RUNNER_SOURCES.map((name) => [name, "c".repeat(64)]));
  const options = {
    repoRoot: root,
    platform: "darwin",
    arch: "arm64",
    loadEvidence: () => ({ sources: clone(sources), runtime: clone(runtime) }),
    loadRunnerSources: () => clone(runnerSources),
  };
  const inputFile = path.join(root, ".cache/input.json"),
    planFile = path.join(output, "plan.json"),
    approvalFile = path.join(output, "approval.json");
  const write = (filename, value) =>
    fs.writeFileSync(filename, JSON.stringify(value), { mode: 0o600 });
  const plan = compileSetupExecutionPlan(full, runnerSources);
  write(inputFile, request);
  write(planFile, plan);
  const approve = (value = plan) =>
    write(approvalFile, {
      format: APPROVAL_FORMAT,
      executionPlanSha256: value.executionPlanSha256,
      scope: APPROVAL_SCOPE,
      maximumAttempts: 1,
    });
  return {
    root,
    output,
    full,
    request,
    runnerSources,
    options,
    inputFile,
    planFile,
    approvalFile,
    plan,
    write,
    approve,
    args: ["--input", inputFile, "--plan", planFile, "--approval", approvalFile, "--run"],
  };
}
const code = (expected) => (error) => error.safeCode === expected;
const hello = {
  v: 1,
  type: "hello",
  protocol: "joinery-snowflake",
  sdkVersion: "3.4.1",
  limits: { frameBytes: 1048576, batchRows: 128, batchBytes: 262144, queued: 8, credits: 32 },
};
const contextNames = [
  "PRINCIPAL",
  "ROLE_NAME",
  "DATABASE_NAME",
  "SCHEMA_NAME",
  "WAREHOUSE_NAME",
  "SESSION_ID",
  "SEARCH_PATH",
  "SECONDARY_ROLES",
];
function answer(step, full) {
  const db = step.phase === "initial" ? full.profile.database : `JOINERY_ACCEPTANCE_${full.runId}`;
  const schema = step.phase === "schema" ? "SCRATCH" : full.profile.schema;
  let names = ["STATUS"],
    types = ["text"],
    values = [["created"]];
  const kind = step.key.split(".").at(-1);
  if (kind === "context") {
    names = contextNames;
    types = names.map(() => "text");
    values = [
      [
        "FIXTURE_USER",
        full.profile.role,
        db,
        schema,
        full.profile.warehouse,
        "opaque-session",
        JSON.stringify([`${db}.${schema}`]),
        '{"roles":"","value":"NONE"}',
      ],
    ];
  } else if (["quotedIdentifierPolicy", "autocommit", "abortOnError"].includes(kind)) {
    names = ["key", "value"];
    types = ["text", "text"];
    values = [
      [
        {
          quotedIdentifierPolicy: "QUOTED_IDENTIFIERS_IGNORE_CASE",
          autocommit: "AUTOCOMMIT",
          abortOnError: "TRANSACTION_ABORT_ON_ERROR",
        }[kind],
        kind === "autocommit" ? "true" : "false",
      ],
    ];
  } else if (kind === "transaction") {
    names = ["TRANSACTION_ID"];
    values = [[null]];
  } else if (step.key === "verify.database") {
    names = ["DATABASE_NAME", "DATABASE_OWNER", "IS_TRANSIENT", "COMMENT", "CREATED"];
    types = ["text", "text", "text", "text", "timestamp_ltz"];
    values = [
      [
        db,
        full.profile.role,
        "YES",
        `joinery-disposable-${full.runId}`,
        "2026-11-11T00:00:00.000Z",
      ],
    ];
  } else if (step.key === "verify.schema") {
    names = ["CATALOG_NAME", "SCHEMA_NAME", "SCHEMA_OWNER", "IS_TRANSIENT", "COMMENT"];
    types = names.map(() => "text");
    values = [[db, "SCRATCH", full.profile.role, "YES", `joinery-disposable-${full.runId}`]];
  } else if (step.key === "verify.empty") {
    names = ["ROW_COUNT"];
    types = ["fixed"];
    values = [["0"]];
  }
  return { columns: names.map((name, i) => ({ name, type: types[i] ?? "text" })), values };
}
function protocolFixture() {
  const full = input(),
    manifest = compileSetupManifest(full),
    generation = randomUUID();
  const protocol = createSetupProtocol({ manifest, currentInput: full, generation });
  protocol.receive(hello);
  let sequence = 0;
  function send(request, type, value = {}) {
    protocol.receive({ v: 1, generation, id: request.id, type, ...value });
  }
  function complete(request, mutate = (value) => value) {
    send(request, "accepted");
    if (request.op === "connect")
      return send(request, "terminal", { ok: true, result: { connected: true } });
    const step = manifest.payload.sqlLedger[protocol.snapshot().nextStep - 1];
    send(request, "phase", { phase: "dispatching" });
    const sdkRequestId = `11111111-1111-4111-8111-${String(++sequence).padStart(12, "0")}`;
    const queryId = `22222222-2222-4222-8222-${String(sequence).padStart(12, "0")}`;
    send(request, "statement", { sdkRequestId });
    if (step.mutation)
      send(request, "setupAcknowledged", {
        runId: full.runId,
        step: step.index,
        key: step.key,
        sdkRequestId,
      });
    send(request, "statement", { sdkRequestId, queryId });
    send(request, "phase", { phase: "acknowledged", sdkRequestId, queryId });
    let result;
    if (step.index === 1)
      result = {
        user: "FIXTURE_USER",
        role: full.profile.role,
        database: full.profile.database,
        schema: full.profile.schema,
        warehouse: full.profile.warehouse,
        session: "opaque-session",
        sdkRequestId,
        queryId,
      };
    else if (step.index === 2) {
      send(request, "columns", { columns: [{ name: "VERSION", type: "text" }] });
      send(request, "rows", { rows: [["fixture-version"]] });
      result = { rowsSeen: 1, rowsRetained: 1, truncated: false, sdkRequestId, queryId };
    } else
      result = {
        runId: full.runId,
        step: step.index,
        key: step.key,
        phase: step.phase,
        mutation: step.mutation,
        rowsSeen: 1,
        rowsRetained: 0,
        truncated: false,
        ...answer(step, full),
        sdkRequestId,
        queryId,
      };
    send(request, "terminal", { ok: true, result: mutate(result), sdkRequestId, queryId });
  }
  function through(step) {
    while (protocol.snapshot().nextStep <= step) complete(protocol.reserveNext());
  }
  return { full, manifest, protocol, generation, complete, send, through };
}

test("setup protocol admits exactly the closed28 SQL with expected CREATE context transitions", () => {
  const f = protocolFixture();
  f.through(28);
  assert.equal(f.protocol.snapshot().ledgerComplete, true);
  assert.equal(f.protocol.reserveNext(), null);
  assert.equal(f.protocol.outcomes().length, 28);
  assert(
    f.protocol
      .outcomes()
      .every(
        (entry) => entry.remoteOutcome === "succeeded" && entry.validationStatus === "validated",
      ),
  );
  assert.equal(
    f.protocol.takeEvents().filter((event) => event.kind === "createAcknowledged").length,
    3,
  );
});

test("fresh idle, principal, role, warehouse and secondary role checks stop before CREATE", () => {
  for (const [step, mutate, expected] of [
    [
      3,
      (r) => {
        r.values[0][0] = "OTHER";
        return r;
      },
      "setupContextMismatch",
    ],
    [
      4,
      (r) => {
        r.values[0][1] = "true";
        return r;
      },
      "unsupportedPolicy",
    ],
    [
      5,
      (r) => {
        r.values[0][1] = "false";
        return r;
      },
      "setupSessionNotIdle",
    ],
    [
      7,
      (r) => {
        r.values[0][0] = "active-transaction";
        return r;
      },
      "setupSessionNotIdle",
    ],
    [
      9,
      (r) => {
        r.values[0][2] = "INITIAL_DB";
        return r;
      },
      "setupContextMismatch",
    ],
    [
      9,
      (r) => {
        r.values[0][7] = '{"roles":"OTHER","value":"ALL"}';
        return r;
      },
      "setupSecondaryRolesChanged",
    ],
  ]) {
    const f = protocolFixture();
    f.through(step - 1);
    assert.throws(() => f.complete(f.protocol.reserveNext(), mutate), code(expected));
    assert.equal(f.protocol.reserveNext(), null);
    assert.equal(f.protocol.outcomes().at(-1).validationStatus, "failed");
    assert.equal(f.protocol.outcomes().filter((entry) => entry.mutation).length, step > 8 ? 1 : 0);
  }
});

test("catalog ownership/label/transience and exact FIXED zero are required independently from CREATE ACKs", () => {
  for (const [step, mutate, expected] of [
    [
      21,
      (r) => {
        r.values[0][1] = "OTHER_OWNER";
        return r;
      },
      "setupDatabaseMismatch",
    ],
    [
      22,
      (r) => {
        r.values[0][4] = "other-label";
        return r;
      },
      "setupSchemaMismatch",
    ],
    [
      23,
      (r) => {
        r.values[0][0] = "0.0";
        return r;
      },
      "setupTableNotEmpty",
    ],
    [
      23,
      (r) => {
        r.columns[0].type = "real";
        return r;
      },
      "setupTableNotEmpty",
    ],
  ]) {
    const f = protocolFixture();
    f.through(step - 1);
    assert.throws(() => f.complete(f.protocol.reserveNext(), mutate), code(expected));
    assert.deepEqual(
      f.protocol
        .outcomes()
        .filter((e) => e.mutation)
        .map((e) => e.remoteOutcome),
      ["succeeded", "succeeded", "succeeded"],
    );
  }
});

test("private journal is exclusive, hash chained, mode0600 and cannot reopen/resume", (t) => {
  const f = fixture(t),
    filename = path.join(f.output, "journal.jsonl"),
    runKey = setupRunKey(f.full.profile, f.full.runId);
  const journal = createSetupJournal({
    root: f.root,
    filename,
    executionPlanSha256: f.plan.executionPlanSha256,
    runKey,
  });
  journal.append({ kind: "intent", step: 8 });
  journal.append({
    kind: "createAcknowledged",
    step: 8,
    sdkRequestId: "11111111-1111-4111-8111-000000000008",
  });
  journal.close();
  assert.equal(fs.statSync(filename).mode & 0o7777, 0o600);
  const [header, first, second] = fs
    .readFileSync(filename, "utf8")
    .trim()
    .split("\n")
    .map(JSON.parse);
  assert.equal(first.sequence, 1);
  assert.equal(second.sequence, 2);
  assert.equal(second.previous, first.sha256);
  assert.equal(header.runKey, runKey);
  assert.throws(
    () =>
      createSetupJournal({
        root: f.root,
        filename,
        executionPlanSha256: f.plan.executionPlanSha256,
        runKey,
      }),
    { code: "EEXIST" },
  );
  assert.throws(() => journal.append({ kind: "intent", step: 14 }), code("journalUnavailable"));
});

test("runner dry-run/emit/verify never create auth state or consume authority; source drift fails", async (t) => {
  const f = fixture(t);
  let spawns = 0;
  const options = {
    ...f.options,
    spawnChild: () => {
      spawns++;
      throw Error("synthetic");
    },
  };
  assert.equal((await main(["--input", f.inputFile], options)).mode, "dryRun");
  const emitted = path.join(f.output, "emitted.json");
  await main(["--input", f.inputFile, "--emit", emitted], options);
  assert.equal(
    (await main(["--input", f.inputFile, "--plan", emitted], options)).mode,
    "verifiedExecutionPlan",
  );
  assert.equal(spawns, 0);
  assert.deepEqual(fs.readdirSync(f.output).sort(), ["emitted.json", "plan.json"]);
  const changed = clone(f.runnerSources);
  changed[RUNNER_SOURCES[0]] = "d".repeat(64);
  assert.throws(
    () => verifySetupExecutionPlan(f.plan, f.full, changed),
    code("setupExecutionPlanDrift"),
  );
  for (const args of [
    ["--input", f.inputFile, "--sql", "CREATE TABLE X(I INT)"],
    ["--input", f.inputFile, "--run"],
    ["--input", f.inputFile, "--node", "/bin/false"],
  ])
    await assert.rejects(main(args, options), code("invalidSetupArguments"));
});

test("failed spawn consumes account/RUN permanently even under newly approved plan/window", async (t) => {
  const f = fixture(t);
  f.approve();
  let spawns = 0;
  const options = {
    ...f.options,
    spawnChild: () => {
      spawns++;
      throw Error("PRIVATE_SYNTHETIC");
    },
  };
  const failed = await main(f.args, options);
  assert.equal(failed.qualificationPassed, false);
  assert.equal(failed.approvalConsumed, true);
  assert.equal(spawns, 1);
  assert.equal(
    fs.readdirSync(f.output).filter((name) => name.startsWith("setup-consumed-")).length,
    2,
  );
  const changed = clone(f.full);
  changed.window.expiresAt = new Date(Date.now() + 200000).toISOString();
  const plan = compileSetupExecutionPlan(changed, f.runnerSources);
  f.write(f.inputFile, { ...f.request, window: changed.window });
  f.write(f.planFile, plan);
  f.approve(plan);
  await assert.rejects(main(f.args, options), { code: "EEXIST" });
  assert.equal(spawns, 1);
});

// Real child process + production Helper, synthetic SDK only; no network/account.
function syntheticChild(t, f, mode = "success") {
  const filename = path.join(f.root, `child-${randomUUID()}.mjs`),
    sqlLog = path.join(f.root, `sql-${randomUUID()}.jsonl`);
  const helperUrl = pathToFileURL(path.resolve("snowflake-helper/src/helper.mjs")).href;
  const framingUrl = pathToFileURL(path.resolve("snowflake-helper/src/framing.mjs")).href;
  const source = `
import fs from 'node:fs'; import { Readable } from 'node:stream';
import { Helper } from ${JSON.stringify(helperUrl)}; import { Decoder, encode } from ${JSON.stringify(framingUrl)};
const full=${JSON.stringify(f.full)}, ledger=${JSON.stringify(deriveSetupLedger(f.full.runId))}, mode=${JSON.stringify(mode)}, sqlLog=${JSON.stringify(sqlLog)};
const answer=${answer.toString()}, contextNames=${JSON.stringify(contextNames)};
let count=0, exiting=false;
const retire=()=>{ if(exiting)return; exiting=true; process.stdin.destroy(); setTimeout(()=>process.exit(0),10); };
const connection={connectAsync(callback){setImmediate(()=>callback(null,connection));return Promise.resolve(connection)},destroy(callback){callback?.()},
execute(options){ const index=++count, step=ledger[index-1]; fs.appendFileSync(sqlLog,JSON.stringify({sql:options.sqlText,binds:options.binds})+'\\n');
 if(options.sqlText!==step.sql) throw Error('unexpectedSQL');
 const sdkRequestId='11111111-1111-4111-8111-'+String(index).padStart(12,'0'); let acknowledged=false;
 let result=index===1?{columns:['u','r','d','s','w','session'].map(name=>({name,type:'text'})),values:[['FIXTURE_USER',full.profile.role,full.profile.database,full.profile.schema,full.profile.warehouse,'opaque-session']]}:index===2?{columns:[{name:'VERSION',type:'text'}],values:[['fixture-version']]}:answer(step,full);
 if(mode==='badOwner'&&index===21)result.values[0][1]='OTHER';
 const faultAt=Number(mode.match(/[0-9]+$/)?.[0]??8);
 const statement={getRequestId:()=>sdkRequestId,getQueryId:()=>acknowledged?'22222222-2222-4222-8222-'+String(index).padStart(12,'0'):undefined,
 getColumns:()=>result.columns.map(c=>({getName:()=>c.name,getType:()=>c.type})),streamRows(){if(mode.startsWith('holdDrain')&&index===faultAt)return new Readable({objectMode:true,read(){}});if(mode.startsWith('drainLost')&&index===faultAt)return new Readable({objectMode:true,read(){this.destroy(Error('syntheticDrainLoss'))}});return Readable.from(result.values,{objectMode:true})},cancel(callback){callback?.()}};
 setImmediate(()=>{if(mode.startsWith('lostAck')&&index===faultAt){retire();return}if(mode.startsWith('rejected')&&index===faultAt){options.complete({sqlState:'42501'},statement);return} acknowledged=true; options.complete(null,statement)}); return statement;
}};
const helper=new Helper({createConnection:()=>connection},frame=>new Promise((resolve,reject)=>process.stdout.write(encode(frame),error=>error?reject(error):resolve())),retire,{clientConfigFile:'/owned/synthetic',closeTimeoutMs:50});
const decoder=new Decoder();process.stdin.on('data',chunk=>{for(const frame of decoder.push(chunk))helper.receive(frame)});process.stdin.on('end',()=>helper.stop());
`;
  fs.writeFileSync(filename, source, { mode: 0o600 });
  const child = spawn(process.execPath, [filename], { env: {}, stdio: ["pipe", "pipe", "pipe"] });
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  });
  return { child, sqlLog };
}

test("real Helper child completes exact28 once, persisting each intent and three ACKs before the next SQL", async (t) => {
  const f = fixture(t),
    { child, sqlLog } = syntheticChild(t, f);
  const events = [];
  const result = await exerciseSetupChild(child, {
    manifest: f.plan.payload.ledger,
    currentInput: f.full,
    generation: randomUUID(),
    persist: async (event) => {
      if (event.kind === "createAcknowledged") {
        await new Promise((resolve) => setTimeout(resolve, 15));
        const calls = fs.readFileSync(sqlLog, "utf8").trim().split("\n");
        assert.equal(calls.length, event.step, "nextSQL escaped durable ACK gate");
      }
      events.push(event);
    },
  });
  assert.equal(result.passed, true, JSON.stringify(result));
  const calls = fs.readFileSync(sqlLog, "utf8").trim().split("\n").map(JSON.parse);
  assert.deepEqual(
    calls.map((call) => call.sql),
    deriveSetupLedger(f.full.runId).map((step) => step.sql),
  );
  assert(calls.every((call) => JSON.stringify(call.binds) === "[]"));
  assert.equal(events.filter((event) => event.kind === "createAcknowledged").length, 3);
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.logoutAcknowledged, true);
});

test("real Helper partial CREATE distinguishes lost ACK, retained ACK/drain failure, and later catalog failure", async (t) => {
  for (const [mode, expectedCalls, outcomes] of [
    ["lostAck8", 8, ["unknown"]],
    ["drainLost8", 8, ["succeeded"]],
    ["lostAck14", 14, ["succeeded", "unknown"]],
    ["drainLost14", 14, ["succeeded", "succeeded"]],
    ["rejected14", 14, ["succeeded", "rejected"]],
    ["lostAck20", 20, ["succeeded", "succeeded", "unknown"]],
    ["drainLost20", 20, ["succeeded", "succeeded", "succeeded"]],
    ["badOwner", 21, ["succeeded", "succeeded", "succeeded"]],
  ]) {
    const f = fixture(t),
      { child, sqlLog } = syntheticChild(t, f, mode),
      events = [];
    const result = await exerciseSetupChild(child, {
      manifest: f.plan.payload.ledger,
      currentInput: f.full,
      generation: randomUUID(),
      persist: (event) => events.push(event),
      timingOverrides: { closeMs: 100, eofMs: 100, termMs: 100, killMs: 100 },
    });
    assert.equal(result.passed, false, mode);
    assert.equal(fs.readFileSync(sqlLog, "utf8").trim().split("\n").length, expectedCalls, mode);
    assert.deepEqual(
      result.outcomes.filter((event) => event.mutation).map((event) => event.remoteOutcome),
      outcomes,
      mode,
    );
    assert.equal(
      events.filter((event) => event.kind === "createAcknowledged").length,
      outcomes.filter((outcome) => outcome === "succeeded").length,
    );
  }
});

test("explicit owner cancellation after CREATE ACK preserves success and admits no later SQL", async (t) => {
  const f = fixture(t),
    { child, sqlLog } = syntheticChild(t, f, "holdDrain8"),
    owner = new AbortController();
  const result = await exerciseSetupChild(child, {
    manifest: f.plan.payload.ledger,
    currentInput: f.full,
    generation: randomUUID(),
    signal: owner.signal,
    persist(event) {
      if (event.kind === "createAcknowledged") owner.abort();
    },
  });
  assert.equal(result.passed, false);
  assert.equal(result.failureCode, "setupOwnerCancelled");
  assert.equal(result.cleanup.cancelRequested, true);
  assert.equal(result.outcomes.find((entry) => entry.step === 8).remoteOutcome, "succeeded");
  assert.equal(fs.readFileSync(sqlLog, "utf8").trim().split("\n").length, 8);
});

test("stalled asynchronous journal callback has a bounded failure and cannot dispatch more SQL", async (t) => {
  const f = fixture(t),
    { child, sqlLog } = syntheticChild(t, f);
  const result = await exerciseSetupChild(child, {
    manifest: f.plan.payload.ledger,
    currentInput: f.full,
    generation: randomUUID(),
    persist(event) {
      if (event.kind === "createAcknowledged") return new Promise(() => {});
    },
    timingOverrides: { sqlMs: 150 },
  });
  assert.equal(result.passed, false);
  assert.equal(result.journalFailed, true);
  assert.equal(fs.readFileSync(sqlLog, "utf8").trim().split("\n").length, 8);
  assert.equal(result.outcomes.find((entry) => entry.step === 8).remoteOutcome, "succeeded");
});

test("approval mismatch, pre-abort, platform and window rejection precede all consumption", async (t) => {
  const f = fixture(t);
  f.approve();
  let spawns = 0;
  const options = {
    ...f.options,
    spawnChild() {
      spawns++;
      throw Error("unexpected");
    },
  };
  const owner = new AbortController();
  owner.abort();
  await assert.rejects(
    main(f.args, { ...options, signal: owner.signal }),
    code("setupOwnerCancelledBeforeAttempt"),
  );
  await assert.rejects(
    main(f.args, { ...options, platform: "linux" }),
    code("unsupportedSetupPlatform"),
  );
  f.write(f.approvalFile, {
    format: "joinery-snowflake-qualification-approval-v1",
    executionPlanSha256: f.plan.executionPlanSha256,
    scope: APPROVAL_SCOPE,
    maximumAttempts: 1,
  });
  await assert.rejects(main(f.args, options), code("invalidSetupApproval"));
  assert.equal(spawns, 0);
  assert.equal(
    fs.readdirSync(f.output).some((name) => name.startsWith("setup-consumed")),
    false,
  );
});

test("partial CREATE journal survives a runner restart and prevents replay under changed provenance", async (t) => {
  const f = fixture(t);
  f.approve();
  let spawns = 0;
  const options = {
    ...f.options,
    spawnChild() {
      spawns++;
      return syntheticChild(t, f, "drainLost14").child;
    },
  };
  const result = await main(f.args, options);
  assert.equal(result.qualificationPassed, false);
  assert.deepEqual(
    result.exercise.outcomes.filter((entry) => entry.mutation).map((entry) => entry.remoteOutcome),
    ["succeeded", "succeeded"],
  );
  const events = fs
    .readFileSync(path.join(f.output, `setup-journal-${f.plan.executionPlanSha256}.jsonl`), "utf8")
    .trim()
    .split("\n")
    .slice(1)
    .map((line) => JSON.parse(line).event);
  assert.deepEqual(
    events.filter((entry) => entry.kind === "createAcknowledged").map((entry) => entry.step),
    [8, 14],
  );
  const changed = clone(f.runnerSources);
  changed[RUNNER_SOURCES[0]] = "d".repeat(64);
  const plan = compileSetupExecutionPlan(f.full, changed);
  f.write(f.planFile, plan);
  f.approve(plan);
  await assert.rejects(main(f.args, { ...options, loadRunnerSources: () => changed }), {
    code: "EEXIST",
  });
  assert.equal(spawns, 1);
});

test("journal failure after real CREATE ACK stops later SQL while retaining known success in memory", async (t) => {
  const f = fixture(t),
    { child, sqlLog } = syntheticChild(t, f),
    journalEvents = [];
  const result = await exerciseSetupChild(child, {
    manifest: f.plan.payload.ledger,
    currentInput: f.full,
    generation: randomUUID(),
    persist(event) {
      if (event.kind === "createAcknowledged") throw Error("syntheticDiskFailure");
      journalEvents.push(event);
    },
  });
  assert.equal(result.passed, false);
  assert.equal(result.journalFailed, true);
  assert.equal(fs.readFileSync(sqlLog, "utf8").trim().split("\n").length, 8);
  assert.equal(result.outcomes.find((entry) => entry.step === 8).remoteOutcome, "succeeded");
  assert.equal(journalEvents.filter((event) => event.kind === "createAcknowledged").length, 0);
});

test("runner owns real Helper child, private durable receipt and identity-checked auth-root cleanup", async (t) => {
  const f = fixture(t);
  f.approve();
  let sqlLog;
  const result = await main(f.args, {
    ...f.options,
    spawnChild(_executable, _args, options) {
      assert.deepEqual(Object.keys(options.env), ["JOINERY_SNOWFLAKE_STATE_DIR"]);
      assert.equal(options.shell, false);
      assert.equal(fs.statSync(options.env.JOINERY_SNOWFLAKE_STATE_DIR).mode & 0o7777, 0o700);
      const supplied = syntheticChild(t, f);
      sqlLog = supplied.sqlLog;
      return supplied.child;
    },
  });
  assert.equal(result.qualificationPassed, true, JSON.stringify(result));
  assert.equal(result.evidencePersisted, true);
  assert.equal(result.qualificationCandidate, true);
  assert.equal(result.workflowChecksPassed, true);
  assert.equal(result.finalizationStatus, "runnerConfirmed");
  const provisional = JSON.parse(
    fs.readFileSync(path.join(f.output, `setup-result-${f.plan.executionPlanSha256}.json`), "utf8"),
  );
  assert.equal(provisional.qualificationPassed, false);
  assert.equal(provisional.evidencePersisted, false);
  assert.equal(provisional.qualificationCandidate, true);
  assert.equal(provisional.workflowChecksPassed, true);
  assert.equal(provisional.finalizationStatus, "requiresRunnerConfirmation");
  assert.equal(fs.readFileSync(sqlLog, "utf8").trim().split("\n").length, 28);
  const journalFile = path.join(f.output, `setup-journal-${f.plan.executionPlanSha256}.jsonl`);
  const events = fs
    .readFileSync(journalFile, "utf8")
    .trim()
    .split("\n")
    .slice(1)
    .map((line) => JSON.parse(line).event);
  assert.deepEqual(
    events.filter((entry) => entry.kind === "createAcknowledged").map((entry) => entry.step),
    [8, 14, 20],
  );
  assert(
    events
      .filter((entry) => entry.kind === "createAcknowledged")
      .every((entry) => entry.sdkRequestId),
  );
  assert.equal(
    fs.readdirSync(f.output).some((name) => name.startsWith("setup-auth-")),
    false,
  );
  const publicText = JSON.stringify(publicSetupResult(result));
  for (const privateValue of [
    "FIXTURE_USER",
    "opaque-session",
    f.full.profile.account,
    f.full.runId,
    "11111111-1111",
    "22222222-2222",
  ])
    assert.equal(publicText.includes(privateValue), false, privateValue);
});

test("pre-existing final receipt stops before consumption, authentication state or spawn", async (t) => {
  const f = fixture(t);
  f.approve();
  const receipt = path.join(f.output, `setup-result-${f.plan.executionPlanSha256}.json`);
  const original = '{"existingPrivateReceipt":true}\n';
  fs.writeFileSync(receipt, original, { mode: 0o600 });
  let spawns = 0;
  await assert.rejects(
    main(f.args, {
      ...f.options,
      spawnChild() {
        spawns++;
        throw Error("unexpectedSpawn");
      },
    }),
    code("setupEvidenceAlreadyExists"),
  );
  assert.equal(spawns, 0);
  assert.equal(fs.readFileSync(receipt, "utf8"), original);
  assert.equal(
    fs
      .readdirSync(f.output)
      .some(
        (name) =>
          name.startsWith("setup-auth-") ||
          name.startsWith("setup-consumed-") ||
          name.startsWith("setup-journal-"),
      ),
    false,
  );
});

test("journal close failure retains acknowledged CREATE outcomes and attempts a failed-persistence final receipt", async (t) => {
  const f = fixture(t);
  f.approve();
  const journalFile = path.join(f.output, `setup-journal-${f.plan.executionPlanSha256}.jsonl`);
  const open = fs.openSync,
    close = fs.closeSync;
  let journalFd,
    injected = false;
  fs.openSync = function (...args) {
    const fd = Reflect.apply(open, fs, args);
    if (args[0] === journalFile && args[1] & fs.constants.O_APPEND) journalFd = fd;
    return fd;
  };
  fs.closeSync = function (fd) {
    if (fd === journalFd && !injected) {
      injected = true;
      Reflect.apply(close, fs, [fd]);
      throw Error("syntheticJournalCloseFailure");
    }
    return Reflect.apply(close, fs, [fd]);
  };
  let result;
  try {
    result = await main(f.args, {
      ...f.options,
      spawnChild() {
        return syntheticChild(t, f).child;
      },
    });
  } finally {
    fs.openSync = open;
    fs.closeSync = close;
  }
  assert.equal(injected, true);
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.evidencePersisted, false);
  assert.equal(result.failureCode, "setupJournalCloseFailed");
  assert.equal(result.exercise.passed, true);
  assert.equal(result.journalCloseFailed, true);
  assert.equal(publicSetupResult(result).journalFailed, true);
  assert.deepEqual(
    result.exercise.outcomes.filter((entry) => entry.mutation).map((entry) => entry.remoteOutcome),
    ["succeeded", "succeeded", "succeeded"],
  );
  const persisted = JSON.parse(
    fs.readFileSync(path.join(f.output, `setup-result-${f.plan.executionPlanSha256}.json`), "utf8"),
  );
  assert.equal(persisted.evidencePersisted, false);
  assert.equal(persisted.qualificationPassed, false);
  assert.deepEqual(
    persisted.exercise.outcomes
      .filter((entry) => entry.mutation)
      .map((entry) => entry.remoteOutcome),
    ["succeeded", "succeeded", "succeeded"],
  );
  assert.equal(
    fs
      .readFileSync(journalFile, "utf8")
      .trim()
      .split("\n")
      .map(JSON.parse)
      .filter((entry) => entry.event?.kind === "createAcknowledged").length,
    3,
  );
});

test("final directory sync failure leaves a provisional receipt with known CREATE facts", async (t) => {
  const f = fixture(t);
  f.approve();
  const receipt = path.join(f.output, `setup-result-${f.plan.executionPlanSha256}.json`),
    sync = fs.fsyncSync;
  let injected = false,
    result;
  fs.fsyncSync = function (fd) {
    if (!injected && fs.fstatSync(fd).isDirectory() && fs.existsSync(receipt)) {
      injected = true;
      throw Error("syntheticFinalDirectorySyncFailure");
    }
    return Reflect.apply(sync, fs, [fd]);
  };
  try {
    result = await main(f.args, {
      ...f.options,
      spawnChild() {
        return syntheticChild(t, f).child;
      },
    });
  } finally {
    fs.fsyncSync = sync;
  }
  assert.equal(injected, true);
  assert.equal(result.qualificationPassed, false);
  assert.equal(result.evidencePersisted, false);
  assert.equal(result.failureCode, "setupEvidencePersistenceFailed");
  assert.equal(result.finalizationStatus, "runnerConfirmationFailed");
  const persisted = JSON.parse(fs.readFileSync(receipt, "utf8"));
  assert.equal(persisted.qualificationPassed, false);
  assert.equal(persisted.evidencePersisted, false);
  assert.equal(persisted.qualificationCandidate, true);
  assert.equal(persisted.workflowChecksPassed, true);
  assert.equal(persisted.finalizationStatus, "requiresRunnerConfirmation");
  for (const value of [result, persisted])
    assert.deepEqual(
      value.exercise.outcomes.filter((entry) => entry.mutation).map((entry) => entry.remoteOutcome),
      ["succeeded", "succeeded", "succeeded"],
    );
});
