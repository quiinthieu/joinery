import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough, Writable, Readable } from "node:stream";
import { spawn } from "node:child_process";
import { exerciseQualificationChild, CHILD_LIMITS } from "./snowflake-qualification-child.mjs";
import { compileManifest, WORKFLOW, SOURCE_PATHS } from "./snowflake-qualification-manifest.mjs";
import { Helper, LIMITS } from "../snowflake-helper/src/helper.mjs";
import { Decoder, encode } from "../snowflake-helper/src/framing.mjs";

const generation = "12345678-1234-4234-8234-123456789abc";
const timingOverrides = {
  helloMs: 1000,
  authenticationMs: 1000,
  sqlMs: 1000,
  optionsOperationMs: 2000,
  stateOperationMs: 2000,
  admissionWindowMs: 10000,
  closeMs: 50,
  eofMs: 50,
  termMs: 50,
  killMs: 500,
};
function configuration(overrides = {}) {
  const now = Date.now();
  const currentInput = {
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
    sources: Object.fromEntries(
      SOURCE_PATHS.map((name, index) => [name, index.toString(16).padStart(64, "0")]),
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
  return {
    manifest: compileManifest(currentInput),
    currentInput,
    generation,
    timingOverrides,
    ...overrides,
  };
}
function answer(template, schemas = 2) {
  const columns = (names, types) =>
    names.map((name, index) => ({ name, type: types?.[index] ?? "TEXT" }));
  if (template === "startupIdentity")
    return {
      columns: columns(["a", "b", "c", "d", "e", "f"]),
      rows: [["SERVER_NAME", "ROLE", "DB", "PUBLIC", "WH", "opaque-session"]],
    };
  if (template === "context")
    return {
      columns: columns([
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
    };
  if (template === "version")
    return { columns: columns(["CURRENT_VERSION()"]), rows: [["synthetic-version"]] };
  if (template === "transaction") return { columns: columns(["TRANSACTION_ID"]), rows: [[null]] };
  if (template === "schemas")
    return {
      columns: columns(["CATALOG_NAME", "SCHEMA_NAME"]),
      rows: Array.from({ length: schemas }, (_, index) => [
        "DB",
        index ? `S${String(index).padStart(4, "0")}` : "PUBLIC",
      ]),
    };
  if (template === "scalar")
    return {
      columns: columns(
        ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"],
        ["FIXED", "BOOLEAN", "TEXT", "TEXT"],
      ),
      rows: [["9007199254740993", "TRUE", "joinery-qualification", null]],
    };
  const key = {
    quotedIdentifierPolicy: "QUOTED_IDENTIFIERS_IGNORE_CASE",
    autocommit: "AUTOCOMMIT",
    abortOnError: "TRANSACTION_ABORT_ON_ERROR",
  }[template];
  return {
    columns: columns(["key", "value", "default", "level"]),
    rows: [[key, template === "autocommit" ? "true" : "false", "unused", "SESSION"]],
  };
}
function fixture(
  config,
  {
    schemas = 2,
    holdTemplate,
    corruptAck = false,
    badScalar = false,
    onEmit,
    delay = 0,
    closeHang = false,
    backpressure = false,
  } = {},
) {
  const child = new EventEmitter();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  const requests = [],
    sql = [],
    binds = [],
    signals = [];
  let helper,
    retired = false,
    sequence = 0;
  const exit = (code = 0, signal = null) => {
    if (retired) return;
    retired = true;
    child.stdout.end();
    child.stderr.end();
    child.emit("exit", code, signal);
    setImmediate(() => child.emit("close", code, signal));
  };
  child.kill = (signal) => {
    signals.push(signal);
    exit(null, signal);
    return true;
  };
  const decoder = new Decoder();
  child.stdin = new Writable({
    highWaterMark: 1,
    write(chunk, _encoding, callback) {
      for (const request of decoder.push(chunk)) {
        requests.push(request);
        if (!(closeHang && request.op === "close")) helper.receive(request);
      }
      if (backpressure) setImmediate(callback);
      else callback();
    },
    final(callback) {
      if (!closeHang) {
        helper?.stop();
        exit();
      }
      callback();
    },
  });
  const sdk = {
    createConnection() {
      return {
        connectAsync(callback) {
          queueMicrotask(() => callback(null));
          return Promise.resolve();
        },
        destroy(callback) {
          queueMicrotask(() => callback(null));
        },
        execute(options) {
          sql.push(options.sqlText);
          binds.push(options.binds);
          const entry = config.manifest.payload.sqlLedger.find(
            (entry) => entry.sql === options.sqlText,
          );
          assert(entry);
          const data = answer(entry.templateId, schemas),
            id = ++sequence;
          if (badScalar && entry.templateId === "scalar") data.rows[0][0] = "42";
          const statement = {
            getRequestId: () => `00000000-0000-4000-8000-${String(id).padStart(12, "0")}`,
            getQueryId: () => `00000000-0000-4000-8000-${String(id + 10000).padStart(12, "0")}`,
            getColumns: () =>
              data.columns.map((column) => ({
                getName: () => column.name,
                getType: () => column.type,
              })),
            streamRows: () => Readable.from(data.rows),
            cancel(callback) {
              callback(null);
            },
          };
          if (entry.templateId !== holdTemplate) {
            if (delay) setTimeout(() => options.complete(null, statement), delay);
            else queueMicrotask(() => options.complete(null, statement));
          }
          return statement;
        },
      };
    },
  };
  helper = new Helper(
    sdk,
    (frame) => {
      if (
        corruptAck &&
        frame.type === "phase" &&
        frame.phase === "acknowledged" &&
        frame.id === "3"
      ) {
        const invalid = Buffer.from('{"invalid":');
        const header = Buffer.alloc(4);
        header.writeUInt32BE(invalid.length);
        child.stdout.write(Buffer.concat([encode(frame), header, invalid]));
      } else child.stdout.write(encode(frame));
      onEmit?.(frame);
    },
    () => {
      if (!closeHang) exit();
    },
    { clientConfigFile: "unused-synthetic" },
  );
  return { child, requests, sql, binds, signals, exit, helper };
}
function redacted(result) {
  const encoded = JSON.stringify(result);
  for (const forbidden of [
    "SERVER_NAME",
    "opaque-session",
    "login@example.test",
    "SELECT ",
    "PRIVATE_RAW_ERROR",
    "9007199254740993",
  ])
    assert.equal(encoded.includes(forbidden), false, forbidden);
}

test("production Helper synthetic SDK completes exact24 SQL with bounded credit and real Writable backpressure", async () => {
  const config = configuration(),
    owner = fixture(config, { schemas: 1000, backpressure: true });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.passed, true, JSON.stringify(result));
  assert.equal(result.authorizationGranted, false);
  assert.equal(result.fullRunnerImplemented, false);
  assert.deepEqual(
    owner.sql,
    config.manifest.payload.sqlLedger.map((entry) => entry.sql),
  );
  assert.deepEqual(owner.binds[13], ["9007199254740993", true, "joinery-qualification", null]);
  assert.equal(result.outcomes.length, 24);
  assert.equal(
    result.outcomes.every(
      (entry) => entry.remoteOutcome === "succeeded" && entry.validationStatus === "validated",
    ),
    true,
  );
  assert.equal(result.protocol.extraCredits, 7);
  assert.equal(result.protocol.resultBudget.rows, 1023);
  assert.equal(result.cleanup.logoutAcknowledged, true);
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  assert.equal(result.counts.drains > 0, true);
  assert.equal(owner.requests.filter((request) => request.op === "close").length, 1);
  assert.equal(
    owner.requests.some((request) => request.op === "cancel"),
    false,
  );
  redacted(result);
});

test("terminal followed by a late credit rejection is owned separately and never advances SQL twice", async () => {
  const config = configuration(),
    owner = fixture(config, { schemas: 1000 });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.passed, true);
  // Immediate writes race the final schema terminal and reserve one late
  // credit, answered by an unaccepted invalidInput. It owns its own wire ID.
  assert.equal(result.protocol.extraCredits, 8);
  assert.equal(owner.requests.filter((request) => request.op === "credit").length, 8);
  assert.deepEqual(
    owner.sql,
    config.manifest.payload.sqlLedger.map((entry) => entry.sql),
  );
  assert.equal(result.outcomes.length, 24);
});

test("malformed bytes after identified ACK preserve known success and stop all later SQL", async () => {
  const config = configuration(),
    owner = fixture(config, { corruptAck: true });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.passed, false);
  assert.equal(result.failureCode, "decoderJson");
  assert.equal(owner.sql.length, 2);
  assert.deepEqual(result.outcomes.at(-1), {
    commandId: "startup.version",
    op: "query",
    requestStatus: "acknowledged",
    remoteOutcome: "succeeded",
    validationStatus: "pending",
  });
  assert.equal(result.cleanup.logoutAcknowledged, false);
  assert.equal(result.cleanup.childReaped, true);
  redacted(result);
});

test("typed scalar failure preserves acknowledged outcome and stops before state SQL", async () => {
  const config = configuration(),
    owner = fixture(config, { badScalar: true });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "scalarValue");
  assert.equal(owner.sql.length, 14);
  assert.deepEqual(result.outcomes.at(-1), {
    commandId: "scalar",
    op: "query",
    requestStatus: "terminal",
    remoteOutcome: "succeeded",
    validationStatus: "failed",
  });
  assert.equal(result.cleanup.childReaped, true);
  redacted(result);
});

test("explicit owner abort reserves only one identified cancel; cancel ACK never settles original", async () => {
  const controller = new AbortController();
  const config = configuration({ signal: controller.signal });
  const owner = fixture(config, {
    holdTemplate: "version",
    onEmit(frame) {
      if (frame.id === "3" && frame.type === "statement") queueMicrotask(() => controller.abort());
    },
  });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childOwnerCancelled");
  assert.equal(owner.sql.length, 2);
  assert.equal(owner.requests.filter((request) => request.op === "cancel").length, 1);
  assert.equal(result.cleanup.cancelAcknowledged, true);
  assert.equal(result.outcomes.at(-1).remoteOutcome, "unknown");
  assert.equal(result.cleanup.childReaped, true);
  redacted(result);
});

test("SQL timeout never invents owner cancellation and bounded cleanup reaps", async () => {
  const config = configuration({ timingOverrides: { ...timingOverrides, sqlMs: 15 } });
  const owner = fixture(config, { holdTemplate: "version" });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childSqlTimeout");
  assert.equal(
    owner.requests.some((request) => request.op === "cancel"),
    false,
  );
  assert.equal(owner.sql.length, 2);
  assert.equal(result.outcomes.at(-1).remoteOutcome, "unknown");
  assert.equal(result.cleanup.childReaped, true);
});

test("group deadline is shared across SQL reads, not reset on each terminal", async () => {
  const config = configuration({ timingOverrides: { ...timingOverrides, optionsOperationMs: 20 } });
  const owner = fixture(config, { delay: 8 });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childGroupTimeout");
  assert.equal(owner.sql.length < 13, true);
  assert.equal(
    owner.requests.some((request) => request.op === "cancel"),
    false,
  );
});

test("reduced overall admission deadline stops work independently of SQL deadline", async () => {
  const config = configuration({ timingOverrides: { ...timingOverrides, admissionWindowMs: 20 } });
  const owner = fixture(config, { holdTemplate: "version" });
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childWindowExpired");
  assert.equal(owner.sql.length, 2);
  assert.equal(
    owner.requests.some((request) => request.op === "cancel"),
    false,
  );
  assert.equal(result.cleanup.childReaped, true);
});

test("backward wall-clock crossing notBefore stops every later admission", async () => {
  const config = configuration();
  const realNow = Date.now;
  let stepped = false;
  const owner = fixture(config, {
    onEmit(frame) {
      if (frame.id === "2" && frame.type === "terminal" && frame.ok) {
        stepped = true;
        Date.now = () => Date.parse(config.currentInput.window.notBefore) - 1;
      }
    },
  });
  try {
    const result = await exerciseQualificationChild(owner.child, config);
    assert.equal(stepped, true);
    assert.equal(result.failureCode, "childWindowUnavailable");
    assert.equal(owner.sql.length, 1);
    assert.equal(
      owner.requests.some((request) => request.op === "cancel"),
      false,
    );
    assert.equal(result.cleanup.childReaped, true);
    redacted(result);
  } finally {
    Date.now = realNow;
  }
});

test("fixed overall ceiling is independent of approval window and cannot be raised", async () => {
  assert.equal(CHILD_LIMITS.overallMs, 360000);
  const config = configuration({ timingOverrides: { ...timingOverrides, overallMs: 360001 } });
  const owner = fixture(config);
  await assert.rejects(exerciseQualificationChild(owner.child, config), {
    message: "childConfiguration",
  });
  owner.exit();
});

test("monotonic overall deadline stops active auth, SQL and options group without owner cancel", async () => {
  for (const phase of ["authentication", "sql", "options"]) {
    const config = configuration({ timingOverrides: { ...timingOverrides, overallMs: 100 } });
    const owner = fixture(config, {
      holdTemplate: phase === "sql" ? "version" : phase === "options" ? "autocommit" : undefined,
    });
    if (phase === "authentication")
      owner.helper.sdk.createConnection = () => ({
        connectAsync() {
          return new Promise(() => {});
        },
        destroy(callback) {
          callback(null);
        },
      });
    const result = await exerciseQualificationChild(owner.child, config);
    assert.equal(result.failureCode, "childOverallTimeout", phase);
    assert.equal(
      owner.requests.some((request) => request.op === "cancel"),
      false,
      phase,
    );
    assert.equal(result.cleanup.cancelReserved, false, phase);
    assert.equal(result.cleanup.childReaped, true, phase);
    if (phase === "authentication") assert.equal(owner.sql.length, 0);
    else {
      assert.equal(owner.sql.length, phase === "sql" ? 2 : 5);
      assert.equal(result.outcomes.at(-1).remoteOutcome, "unknown");
    }
    redacted(result);
  }
});

test("sync write throw and early process error remain safe with handlers installed", async () => {
  for (const mode of ["write", "error"]) {
    const config = configuration(),
      owner = fixture(config);
    if (mode === "write")
      owner.child.stdin.write = () => {
        throw new Error("PRIVATE_RAW_ERROR");
      };
    else queueMicrotask(() => owner.child.emit("error", new Error("PRIVATE_RAW_ERROR")));
    const result = await exerciseQualificationChild(owner.child, config);
    assert.equal(result.failureCode, mode === "write" ? "childWriteFailed" : "childProcessError");
    assert.equal(owner.sql.length, 0);
    assert.equal(result.cleanup.childReaped, true);
    redacted(result);
  }
});

test("never-draining write cannot admit next request even after its terminal arrived", async () => {
  const config = configuration({ timingOverrides: { ...timingOverrides, authenticationMs: 15 } }),
    owner = fixture(config);
  const original = owner.child.stdin.write.bind(owner.child.stdin);
  owner.child.stdin.write = (chunk, callback) => {
    original(chunk, callback);
    // Swallow drain from the physical stream so the exercise cannot proceed.
    owner.child.stdin.removeAllListeners("drain");
    return false;
  };
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childWriteTimeout");
  assert.equal(owner.requests.filter((request) => request.op === "connect").length, 1);
  assert.equal(owner.sql.length, 0);
  assert.equal(result.cleanup.childReaped, true);
});

test("late write callback failure after connect terminal cannot publish the next SQL", async () => {
  const config = configuration(),
    owner = fixture(config);
  const original = owner.child.stdin.write.bind(owner.child.stdin);
  owner.child.stdin.write = (chunk, callback) =>
    original(chunk, () => setTimeout(() => callback(new Error("PRIVATE_RAW_ERROR")), 5));
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childWriteFailed");
  assert.equal(owner.sql.length, 0);
  assert.equal(result.outcomes.length, 0);
  redacted(result);
});

test("bounded unresponsive cleanup never claims reap, logout or remote cancellation", async () => {
  const config = configuration({
    timingOverrides: {
      ...timingOverrides,
      sqlMs: 10,
      closeMs: 10,
      eofMs: 10,
      termMs: 10,
      killMs: 10,
    },
  });
  const owner = fixture(config, { holdTemplate: "version", closeHang: true });
  owner.child.kill = () => false;
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childSqlTimeout");
  assert.equal(result.cleanup.childReaped, false);
  assert.equal(result.cleanup.closeObserved, false);
  assert.equal(result.cleanup.logoutAcknowledged, false);
  assert.equal(result.cleanup.cancelAcknowledged, false);
  assert.equal(result.cleanup.stdinEndRequested, true);
  owner.exit();
});

test("pre-aborted signal, expired window and invalid timing cannot broaden scope", async () => {
  const controller = new AbortController();
  controller.abort();
  const config = configuration({ signal: controller.signal }),
    owner = fixture(config);
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childOwnerCancelled");
  assert.equal(owner.requests.length, 0);
  assert.equal(owner.sql.length, 0);
  const invalid = configuration({ timingOverrides: { sqlMs: 30001 } }),
    unused = fixture(invalid);
  await assert.rejects(exerciseQualificationChild(unused.child, invalid), {
    message: "childConfiguration",
  });
  for (const badOptions of [undefined, null, { ...invalid, arbitrarySql: "SELECT 99" }])
    await assert.rejects(exerciseQualificationChild(unused.child, badOptions), {
      message: "childConfiguration",
    });
  // Invalid configuration explicitly leaves lifecycle ownership with caller.
  unused.exit();
  const expired = configuration();
  expired.currentInput.window = {
    notBefore: new Date(Date.now() - 5000).toISOString(),
    expiresAt: new Date(Date.now() - 1000).toISOString(),
  };
  expired.manifest = compileManifest(expired.currentInput);
  const expiredOwner = fixture(expired);
  const expiredResult = await exerciseQualificationChild(expiredOwner.child, expired);
  assert.equal(expiredResult.failureCode, "childWindowUnavailable");
  assert.equal(expiredOwner.sql.length, 0);
});

test("stderr is count-only bounded and never included in diagnostics", async () => {
  const config = configuration(),
    owner = fixture(config, { holdTemplate: "version" });
  queueMicrotask(() => owner.child.stderr.write(Buffer.alloc(CHILD_LIMITS.stderrBytes + 1, 65)));
  const result = await exerciseQualificationChild(owner.child, config);
  assert.equal(result.failureCode, "childStderrLimit");
  assert.equal(result.counts.stderrBytes, CHILD_LIMITS.stderrBytes + 1);
  assert.equal(JSON.stringify(result).includes("AAAAAAAAAA"), false);
});

// These processes are synthetic protocol fixtures, not the SDK, runtime factory,
// external-browser auth or account qualification. Only the test file spawns.
const start = (source) =>
  spawn(process.execPath, ["--input-type=module", "-e", source], {
    stdio: ["pipe", "pipe", "pipe"],
  });
function helperProcessSource(config, corruptAck = false) {
  return `
    import {Helper} from ${JSON.stringify(new URL("../snowflake-helper/src/helper.mjs", import.meta.url).href)};
    import {Decoder,encode} from ${JSON.stringify(new URL("../snowflake-helper/src/framing.mjs", import.meta.url).href)};
    import {Readable} from 'node:stream';
    const answer=${answer.toString()};
    const ledger=${JSON.stringify(config.manifest.payload.sqlLedger)};
    let executed=0;
    const sdk={createConnection(){return {
      connectAsync(callback){queueMicrotask(()=>callback(null));return Promise.resolve();},
      destroy(callback){queueMicrotask(()=>callback(null));},
      execute(options){const entry=ledger[executed++];if(!entry||entry.sql!==options.sqlText)throw Error('unexpected synthetic SQL');
        const data=answer(entry.templateId,1000),id=executed;
        const stmt={getRequestId:()=> '00000000-0000-4000-8000-'+String(id).padStart(12,'0'),getQueryId:()=> '00000000-0000-4000-8000-'+String(id+10000).padStart(12,'0'),
          getColumns:()=>data.columns.map(c=>({getName:()=>c.name,getType:()=>c.type})),streamRows:()=>Readable.from(data.rows),cancel(callback){callback(null);}};
        queueMicrotask(()=>options.complete(null,stmt));return stmt;}
    };}};
    process.stdout.on('error',()=>process.exit(0));
    const helper=new Helper(sdk,frame=>{
      const encoded=encode(frame);
      if(${corruptAck}&&frame.id==='3'&&frame.type==='phase'&&frame.phase==='acknowledged'){
        const body=Buffer.from('{"invalid":'),header=Buffer.alloc(4);header.writeUInt32BE(body.length);
        return process.stdout.write(Buffer.concat([encoded,header,body]));
      }
      return process.stdout.write(encoded);
    },()=>process.stdout.end(()=>process.exit(executed===24?0:3)),{clientConfigFile:'unused-synthetic'});
    const decoder=new Decoder();process.stdin.on('data',chunk=>{for(const frame of decoder.push(chunk))helper.receive(frame);});
    process.stdin.on('end',()=>helper.stop());
  `;
}
test("actual owned child runs production Helper synthetic SDK through exactly24 SQL and clean logout/reap", async () => {
  const config = configuration();
  const result = await exerciseQualificationChild(start(helperProcessSource(config)), config);
  assert.equal(result.passed, true, JSON.stringify(result));
  assert.equal(result.outcomes.length, 24);
  assert.equal(result.cleanup.logoutAcknowledged, true);
  assert.equal(result.cleanup.exitCode, 0);
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  redacted(result);
});

test("actual owned helper fixture ACK plus malformed coalesced bytes preserves success then reaps", async () => {
  const config = configuration();
  const result = await exerciseQualificationChild(start(helperProcessSource(config, true)), config);
  assert.equal(result.failureCode, "decoderJson");
  assert.equal(result.outcomes.length, 2);
  assert.equal(result.outcomes.at(-1).remoteOutcome, "succeeded");
  assert.equal(result.outcomes.at(-1).validationStatus, "pending");
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  assert.equal(result.cleanup.logoutAcknowledged, false);
  redacted(result);
});

test("actual owned process silent/TERM-resistant cleanup reaches KILL and actual reap", async () => {
  const child = start(
    'process.on("SIGTERM",()=>{});process.stdin.resume();setInterval(()=>{},1000);',
  );
  const config = configuration({
    timingOverrides: { ...timingOverrides, helloMs: 100, closeMs: 10, eofMs: 10, termMs: 20 },
  });
  const result = await exerciseQualificationChild(child, config);
  assert.equal(result.failureCode, "childHelloTimeout");
  assert.equal(result.cleanup.termSent, true);
  assert.equal(result.cleanup.killSent, true);
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  assert.equal(result.cleanup.exitSignal, "SIGKILL");
});

test("actual early exit is reaped without claiming logout or remote rollback", async () => {
  const child = start('process.stderr.write("PRIVATE_RAW_ERROR");process.exit(7);');
  const result = await exerciseQualificationChild(child, configuration());
  assert.equal(result.passed, false);
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  assert.equal(result.cleanup.exitCode, 7);
  assert.equal(result.cleanup.logoutAcknowledged, false);
  redacted(result);
});

test("exit with inherited stdout still open closes only owned pipes and waits close", async () => {
  const child = start(
    'import {spawn} from "node:child_process";spawn(process.execPath,["-e","setTimeout(()=>{},200)"],{stdio:["ignore",process.stdout,process.stderr]});process.exit(0);',
  );
  const result = await exerciseQualificationChild(
    child,
    configuration({ timingOverrides: { ...timingOverrides, eofMs: 10 } }),
  );
  assert.equal(result.cleanup.childReaped, true);
  assert.equal(result.cleanup.closeObserved, true);
  assert.equal(result.cleanup.termSent, false);
  assert.equal(result.cleanup.killSent, false);
  assert.equal(result.cleanup.stdoutEnded, false);
  // The test-owned bounded descendant is allowed to complete before test return.
  await new Promise((resolve) => setTimeout(resolve, 250));
});
