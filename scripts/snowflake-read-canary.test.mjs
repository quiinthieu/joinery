import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { Decoder } from "../snowflake-helper/src/framing.mjs";
import { exerciseChild, main, STATEMENTS, validateContext } from "./snowflake-read-canary.mjs";

const context = () => ({
  profile: "synthetic",
  source: "synthetic fixture",
  scope: "read canary",
  connection: {
    account: "synthetic-account",
    user: "CANARY_USER",
    role: "CANARY_ROLE",
    database: "CANARY_DATABASE",
    schema: "CANARY_SCHEMA",
    warehouse: "CANARY_WAREHOUSE",
    authenticator: "externalbrowser",
  },
});
const runtime = { manifestSha256: "a".repeat(64) };
const bounds = {
  helloMs: 2000,
  authMs: 200,
  queryMs: 1000,
  closeMs: 1000,
  exitMs: 30,
  killMs: 1000,
};
const framing = new URL("../snowflake-helper/src/framing.mjs", import.meta.url).href;

function childFixture(mode = "success") {
  const source = `
import { Decoder, encode } from ${JSON.stringify(framing)};
const mode = ${JSON.stringify(mode)};
const decoder = new Decoder(); let profile, query=0;
const send = value => process.stdout.write(encode(value));
const reply = (r,type,rest={}) => send({v:1,generation:r.generation,id:r.id,type,...rest});
const terminal=(r,result) => reply(r,"terminal",{ok:true,result});
process.stderr.write("private-browser-url https://secret.invalid/?token=SECRET_TOKEN\\n");
send({v:1,type:"hello",protocol:"joinery-snowflake",sdkVersion:"3.4.1"});
process.stdin.on("data",chunk=>{for(const r of decoder.push(chunk)) {
  reply(r,"accepted");
  if(r.op==="connect") {
    profile=r.payload;
    if(mode==="auth-stall" || mode==="ignore-termination") continue;
    if(mode==="helper-error") { reply(r,"terminal",{ok:false,error:{message:"SECRET_TOKEN private-browser-url"}}); continue; }
    terminal(r,{connected:true});
  } else if(r.op==="query") {
    query++;
    const names=query===1?["USER","ROLE","DATABASE","SCHEMA","WAREHOUSE","SESSION"]:["USER","SESSION","INTEGER","TEXT","BOOLEAN","NULL"];
    const types=query===1?["text","text","text","text","text","fixed"]:["text","fixed","fixed","text","boolean","text"];
    const row=query===1?["DIFFERENT_PRINCIPAL",profile.role,profile.database,profile.schema,profile.warehouse,"9001"]:["DIFFERENT_PRINCIPAL","9001","42","joinery-read-canary","TRUE",null];
    if(mode==="wrong-session"&&query===2)row[1]="9002";
    if(mode==="wrong-principal"&&query===2)row[0]="CHANGED_PRINCIPAL";
    if(mode==="empty-principal")row[0]="";
    if(mode==="oversized-principal")row[0]="x".repeat(257);
    if(mode==="wrong-type"&&query===2)types[2]="text";
    if(mode==="wrong-scalar"&&query===2)row[2]="43";
    reply(r,"columns",{columns:names.map((name,i)=>({name:"CANARY_"+name,type:types[i]}))});
    reply(r,"rows",{rows:mode==="extra-row"?[row,row]:[row]});
    terminal(r,{rowsSeen:1,rowsRetained:1,truncated:false});
  } else if(r.op==="close") {
    terminal(r,{closed:true,logoutAcknowledged:true});
    if(mode==="trailing-frame")process.stdout.write(Buffer.from([0,0]));
    if(mode!=="no-exit")process.stdout.write("",()=>process.exit(mode==="bad-exit"?2:0));
  }
}});
if(mode==="ignore-termination")process.on("SIGTERM",()=>{});
if(mode!=="ignore-termination"&&mode!=="no-exit")process.stdin.on("end",()=>process.exit(0));
setInterval(()=>{},1000);
`;
  const child = spawn(process.execPath, ["--input-type=module", "-e", source], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const requests = [];
  const decode = new Decoder();
  const write = child.stdin.write.bind(child.stdin);
  child.stdin.write = (chunk, ...args) => {
    requests.push(...decode.push(chunk));
    return write(chunk, ...args);
  };
  return { child, requests };
}

function repositoryFixture(t) {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "snowflake-canary-test-")));
  mkdirSync(path.join(root, ".cache"), { mode: 0o700 });
  writeFileSync(path.join(root, ".cache/owner-snowflake-context.json"), JSON.stringify(context()), {
    mode: 0o600,
  });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

test("default plan never spawns, authenticates, writes evidence, or exposes context values", async (t) => {
  const root = repositoryFixture(t);
  let validations = 0;
  const plan = await main([], {
    repoRoot: root,
    validateRuntime: () => {
      validations++;
      return runtime;
    },
    spawnChild: () => assert.fail("plan spawned a child"),
  });
  assert.equal(validations, 1);
  assert.equal(plan.mode, "plan");
  assert.equal(plan.network, false);
  assert.equal(plan.maximumHelperQueryRequests, 2);
  assert.equal(plan.maximumColumnsPerStatement, 6);
  assert.deepEqual(plan.statements, STATEMENTS);
  assert.match(plan.billing, /automatically resume/);
  assert.match(plan.connectionSetup, /not a bound on physical SQL/);
  assert.match(plan.identity, /not an independent binding/);
  assert.deepEqual(readdirSync(path.join(root, ".cache")), ["owner-snowflake-context.json"]);
  assert.doesNotMatch(JSON.stringify(plan), /synthetic-account/);
});

test("context identifiers reject coercible non-string values", () => {
  for (const key of ["database", "schema", "warehouse", "role"]) {
    for (const value of [["VALID"], [["VALID"]], null, {}, 42, true, false]) {
      const candidate = context();
      candidate.connection[key] = value;
      assert.throws(
        () => validateContext(candidate),
        /invalidContext/,
        `${key}: ${JSON.stringify(value)}`,
      );
    }
  }
});

test("arguments and fixed context schema cannot inject SQL, credentials, host or commands", async () => {
  for (const args of [["--sql", "SELECT 1"], ["--run", "--run"], ["--runtime=/tmp/other"]])
    await assert.rejects(main(args), /invalidArguments/);
  for (const mutate of [
    (c) => {
      c.connection.password = "secret";
    },
    (c) => {
      c.connection.account = "attacker.invalid/;command";
    },
    (c) => {
      c.connection.role = "ROLE; USE OTHER";
    },
    (c) => {
      c.connection.authenticator = "oauth";
    },
    (c) => {
      c.sql = "SELECT 2";
    },
  ]) {
    const candidate = context();
    mutate(candidate);
    assert.throws(() => validateContext(candidate), /invalidContext/);
  }
});

test("a principal distinct from the configured login passes with exactly two fixed SELECTs", async () => {
  const { child, requests } = childFixture();
  const result = await exerciseChild(child, validateContext(context()), { bounds });
  assert.equal(result.passed, true);
  assert.equal(result.helperQueryRequestsSent, 2);
  assert.equal(requests[0].payload.username, "CANARY_USER");
  assert.notEqual(requests[0].payload.username, "DIFFERENT_PRINCIPAL");
  assert.deepEqual(result.checks, {
    connected: true,
    context: true,
    sessionContinuity: true,
    principalContinuity: true,
    scalarTypesAndValues: true,
    logoutAcknowledged: true,
    childReaped: true,
    childExitClean: true,
  });
  assert.deepEqual(
    requests.map((request) => request.op),
    ["connect", "query", "query", "close"],
  );
  assert.deepEqual(
    requests.filter((r) => r.op === "query").map((r) => r.payload.sql),
    STATEMENTS,
  );
  assert(
    requests
      .filter((r) => r.op === "query")
      .every(
        (r) => r.payload.maxRows === 1 && r.payload.maxBytes === 8192 && r.payload.credits === 1,
      ),
  );
  assert.doesNotMatch(JSON.stringify(result), /SECRET_TOKEN|private-browser-url|9001|CANARY_USER/);
});

test("session, principal, scalar type/value and extra-row failures cannot pass qualification", async () => {
  for (const mode of [
    "wrong-session",
    "wrong-principal",
    "empty-principal",
    "oversized-principal",
    "wrong-type",
    "wrong-scalar",
    "extra-row",
    "trailing-frame",
    "bad-exit",
  ]) {
    const { child } = childFixture(mode);
    const result = await exerciseChild(child, validateContext(context()), { bounds });
    assert.equal(result.passed, false, mode);
    assert.equal(result.checks.childReaped, true, mode);
  }
});

test("authentication timeout kills and reaps an unresponsive child without retries", async () => {
  const { child, requests } = childFixture("ignore-termination");
  const result = await exerciseChild(child, validateContext(context()), {
    bounds: { ...bounds, closeMs: 40 },
  });
  assert.equal(result.passed, false);
  assert.equal(result.failure, "authenticationTimeout");
  assert.equal(result.checks.childReaped, true);
  assert.deepEqual(
    requests.map((request) => request.op),
    ["connect"],
  );
  assert.equal(child.signalCode, "SIGKILL");
});

test("cancellation during browser-equivalent connect retires child before returning", async () => {
  const { child, requests } = childFixture("auth-stall");
  const controller = new AbortController();
  const decoder = new Decoder();
  child.stdout.on("data", (chunk) => {
    if (decoder.push(chunk).some((frame) => frame.type === "accepted" && frame.id === "1"))
      controller.abort();
  });
  const result = await exerciseChild(child, validateContext(context()), {
    bounds,
    signal: controller.signal,
  });
  assert.equal(result.failure, "cancelled");
  assert.equal(result.checks.childReaped, true);
  assert.deepEqual(
    requests.map((request) => request.op),
    ["connect"],
  );
});

test("helper errors stay sanitized and acknowledged close still requires actual clean exit", async () => {
  const errorChild = childFixture("helper-error");
  const error = await exerciseChild(errorChild.child, validateContext(context()), { bounds });
  assert.equal(error.failure, "helperRejected");
  assert.doesNotMatch(JSON.stringify(error), /SECRET_TOKEN|private-browser-url/);
  const hanging = childFixture("no-exit");
  const result = await exerciseChild(hanging.child, validateContext(context()), { bounds });
  assert.equal(result.checks.logoutAcknowledged, true);
  assert.equal(result.checks.childReaped, true);
  assert.equal(result.passed, false);
  assert.equal(result.failure, "childExitUnclean");
});

test(
  "opt-in synthetic runner removes private state and persists only safe repo-local evidence",
  {
    skip: process.platform !== "darwin" || process.arch !== "arm64",
  },
  async (t) => {
    const root = repositoryFixture(t);
    let authRoot;
    const result = await main(["--run"], {
      repoRoot: root,
      validateRuntime: () => runtime,
      spawnChild: (executable, args, options) => {
        assert.equal(executable, path.join(root, ".cache/snowflake-runtime/macos-arm64/bin/node"));
        assert.deepEqual(args, [
          path.join(root, ".cache/snowflake-runtime/macos-arm64/helper/main.mjs"),
        ]);
        assert.deepEqual(Object.keys(options.env), ["JOINERY_SNOWFLAKE_STATE_DIR"]);
        assert.equal(options.shell, false);
        authRoot = options.env.JOINERY_SNOWFLAKE_STATE_DIR;
        return childFixture().child;
      },
    });
    assert.equal(result.passed, true);
    assert.equal(result.checks.stateRemoved, true);
    assert.throws(() => readFileSync(authRoot), /ENOENT/);
    const saved = readFileSync(path.join(root, result.evidence), "utf8");
    assert.doesNotMatch(
      saved,
      /SECRET_TOKEN|private-browser-url|synthetic-account|CANARY_USER|9001/,
    );
    assert.deepEqual(JSON.parse(saved).checks, result.checks);
  },
);
