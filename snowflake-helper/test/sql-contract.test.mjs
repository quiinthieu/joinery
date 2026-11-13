import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, copyFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { Readable } from "node:stream";
import { Helper } from "../src/helper.mjs";
import { SQL_CONTRACT, validateSqlContract } from "../src/sql-contract.mjs";
import { deriveSetupLedger } from "../src/disposable-setup-ledger.mjs";

test("the bundled closed contract preserves the existing statement semantics", () => {
  assert.deepEqual(SQL_CONTRACT, {
    contractVersion: 1,
    bootstrapContext:
      "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()",
    version: "SELECT CURRENT_VERSION()",
    begin: "BEGIN TRANSACTION",
    commit: "COMMIT",
    rollback: "ROLLBACK",
  });
  assert.equal(Object.isFrozen(SQL_CONTRACT), true);
  const setup = deriveSetupLedger("A".repeat(32));
  assert.equal(setup[0].sql, SQL_CONTRACT.bootstrapContext);
  assert.equal(setup[1].sql, SQL_CONTRACT.version);
});

test("closed contract rejects missing, additional, wrong-version and malformed SQL fields", () => {
  for (const invalid of [
    null,
    [],
    "contract",
    { ...SQL_CONTRACT, additional: true },
    { ...SQL_CONTRACT, contractVersion: 2 },
    { ...SQL_CONTRACT, contractVersion: "1" },
  ]) {
    assert.throws(() => validateSqlContract(invalid), /Invalid bundled/);
  }
  for (const key of Object.keys(SQL_CONTRACT)) {
    const missing = { ...SQL_CONTRACT };
    delete missing[key];
    assert.throws(() => validateSqlContract(missing), /Invalid bundled/, key);
  }
  for (const key of ["bootstrapContext", "version", "begin", "commit", "rollback"])
    for (const invalid of [
      null,
      1,
      [],
      "",
      " ",
      " COMMIT",
      "COMMIT ",
      "COMMIT;ROLLBACK",
      "A\nB",
      "A\0B",
      "é",
      "X".repeat(1025),
    ])
      assert.throws(
        () => validateSqlContract({ ...SQL_CONTRACT, [key]: invalid }),
        /Invalid bundled/,
        key,
      );
  const mutable = { ...SQL_CONTRACT };
  const validated = validateSqlContract(mutable);
  mutable.begin = "changed";
  assert.equal(validated.begin, SQL_CONTRACT.begin);
});

test("invalid bundled JSON or schema prevents helper module loading", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "joinery-sql-contract-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [index, contents] of [
    "{invalid",
    JSON.stringify({ ...SQL_CONTRACT, extra: true }),
  ].entries()) {
    const directory = join(root, String(index));
    await mkdir(directory);
    for (const name of [
      "helper.mjs",
      "mutation-batch.mjs",
      "disposable-setup-ledger.mjs",
      "sql-contract.mjs",
    ])
      await copyFile(new URL(`../src/${name}`, import.meta.url), join(directory, name));
    await writeFile(join(directory, "sql-contract.json"), contents);
    await assert.rejects(import(pathToFileURL(join(directory, "helper.mjs")).href));
  }
});

test("actual helper dispatches contract SQL and recognizes the contract version bootstrap", async (t) => {
  await exerciseHelperContract(t, Helper, SQL_CONTRACT);
});

test("disposable source copy proves helper SQL and version recognition derive from contract bytes", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "joinery-sql-contract-source-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const name of [
    "helper.mjs",
    "mutation-batch.mjs",
    "disposable-setup-ledger.mjs",
    "sql-contract.mjs",
  ])
    await copyFile(new URL(`../src/${name}`, import.meta.url), join(root, name));
  // Change only a disposable source artifact, never a runtime override or packaged file.
  const contract = Object.fromEntries(
    Object.entries(SQL_CONTRACT).map(([key, value]) => [
      key,
      key === "contractVersion" ? value : `${value} /* source fixture */`,
    ]),
  );
  await writeFile(join(root, "sql-contract.json"), JSON.stringify(contract));
  const { Helper: CopiedHelper } = await import(pathToFileURL(join(root, "helper.mjs")).href);
  await exerciseHelperContract(t, CopiedHelper, contract);
  const { deriveSetupLedger: copiedLedger } = await import(
    pathToFileURL(join(root, "disposable-setup-ledger.mjs")).href
  );
  assert.equal(copiedLedger("A".repeat(32))[0].sql, contract.bootstrapContext);
  assert.equal(copiedLedger("A".repeat(32))[1].sql, contract.version);
});

async function exerciseHelperContract(t, HelperClass, contract) {
  const calls = [],
    events = [];
  let sequence = 0;
  const uuid = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
  const identity = ["FIXTURE", null, "TEST", "PUBLIC", null, "opaque-session"];
  const connection = {
    connectAsync(callback) {
      setImmediate(() => callback(null));
      return Promise.resolve();
    },
    destroy(callback) {
      callback();
    },
    execute(options) {
      calls.push(options);
      const n = calls.length;
      const rows =
        options.sqlText === contract.bootstrapContext
          ? [identity]
          : options.sqlText === contract.version
            ? [["fixture-version"]]
            : [];
      const statement = {
        getRequestId: () => uuid(n),
        getQueryId: () => uuid(n + 100),
        getColumns: () =>
          Array.from({ length: rows[0]?.length ?? 0 }, (_, i) => ({
            getName: () => `C${i}`,
            getType: () => "text",
          })),
        streamRows: () => Readable.from(rows),
        cancel(callback) {
          callback();
        },
      };
      setImmediate(() => options.complete(null, statement));
      return statement;
    },
  };
  const helper = new HelperClass(
    { createConnection: () => connection },
    (event) => {
      events.push(event);
      return Promise.resolve();
    },
    () => {},
    {
      clientConfigFile: "/owned/nonexistent",
      closeTimeoutMs: 10,
      // An unrecognized constructor property cannot replace the bundled contract.
      sqlContract: { ...contract, begin: "arbitrary SQL" },
    },
  );
  t.after(() => helper.stop());
  async function send(op, payload = {}) {
    const id = String(++sequence);
    helper.receive({ v: 1, generation: uuid(900), id, op, payload });
    for (let turn = 0; turn < 32; turn++) {
      const terminal = events.find((event) => event.id === id && event.type === "terminal");
      if (terminal) {
        assert.equal(terminal.ok, true, JSON.stringify(terminal));
        return;
      }
      await new Promise((resolve) => setImmediate(resolve));
    }
    assert.fail(`No terminal for ${op}`);
  }
  await send("connect", {
    account: "fixture-account",
    host: "fixture-account.snowflakecomputing.com",
    username: "FIXTURE",
    role: "READER",
    warehouse: "TINY",
    database: "TEST",
    schema: "PUBLIC",
  });
  await send("context");
  assert.equal(helper.setupBootstrap, 1);
  await send("query", {
    sql: contract.version,
    binds: [],
    classification: "read",
    credits: 1,
    maxRows: 1,
    maxBytes: 65536,
  });
  assert.equal(helper.setupBootstrap, 2);
  for (const action of ["begin", "commit", "rollback"])
    await send("transaction", { action, expected: { user: "FIXTURE", session: "opaque-session" } });
  assert.deepEqual(
    calls.map((call) => call.sqlText),
    [
      contract.bootstrapContext,
      contract.version,
      contract.begin,
      contract.commit,
      contract.rollback,
    ],
  );
  assert(calls.every((call) => call.binds.length === 0));
  assert.equal(events.filter((event) => event.type === "boundaryAcknowledged").length, 3);
}
