// Actual pinned SDK + production admission guard; only owned servers/login are
// synthetic. helper-* scenarios drive production Helper terminals using an
// explicitly preconnected SDK fixture (browser authentication is skipped).
// Other scenarios label harness-only outcome mapping. This is not an OS sandbox.
import http from "node:http";
import https from "node:https";
import net from "node:net";
import dns from "node:dns";
import tls from "node:tls";
import dgram from "node:dgram";
import { createRequire, syncBuiltinESMExports } from "node:module";
import { readFile, writeFile, lstat } from "node:fs/promises";
import { join, isAbsolute } from "node:path";
import { once } from "node:events";
import { gunzipSync } from "node:zlib";
import assert from "node:assert/strict";
import { installQuerySubmitGuard } from "../src/query-submit-guard.mjs";
import { verifyTransportSources } from "../src/transport-source-pins.mjs";
import { Helper, connectionOptions } from "../src/helper.mjs";
import { deriveSetupLedger } from "../src/disposable-setup-ledger.mjs";
import { MUTATION_CONTEXT_SQL, MUTATION_SETTINGS_SQL } from "../src/mutation-batch.mjs";

const [scenario, transport] = process.argv.slice(2);
const allowed = new Set([
  "success",
  "lost-ack",
  "reset",
  "delayed-loss",
  "status-503",
  "status-429",
  "renewal",
  "cancel-before",
  "cancel-during",
  "cancel-after",
  "stale-owner",
  "mutation-no-guard",
  "redirect-301",
  "redirect-302",
  "redirect-303",
  "redirect-307",
  "redirect-308",
  "helper-success",
  "helper-batch-automatic",
  "helper-batch-cardinality",
  "helper-batch-manual-commit",
  "helper-batch-manual-rollback",
  "helper-batch-lost-ack",
  "helper-lost-ack",
  "helper-ack-drain",
  "helper-database",
  "reviewed-success",
  "reviewed-timeout-policy",
  "reviewed-context-partial",
  "reviewed-ack-drain",
  "reviewed-file-veto",
  "helper-transaction-begin",
  "helper-transaction-commit",
  "helper-transaction-rollback",
  "helper-transaction-lost-ack",
  "helper-transaction-ack-guard",
  "helper-transaction-database",
  "helper-dml-insert",
  "helper-dml-update",
  "helper-dml-delete",
  "helper-dml-lost-ack",
  "helper-dml-ack-guard",
  "helper-dml-database",
  "helper-dml-ack-drain",
  "helper-dml-ack-cancel",
  "helper-dml-ack-eof",
  "helper-ddl-create",
  "helper-ddl-add-column",
  "helper-ddl-drop",
  "helper-ddl-lost-ack",
  "helper-ddl-ack-guard",
  "helper-ddl-database",
  "helper-ddl-ack-drain",
  "helper-ddl-ack-cancel",
  "helper-ddl-ack-eof",
  "helper-ddl-close-before-ack",
  "helper-ddl-close-after-ack",
  "helper-switch-success",
  "helper-switch-lost-use",
  "helper-switch-readback-denied",
  "helper-switch-lost-readback",
  "helper-setup-database",
  "helper-setup-schema",
  "helper-setup-table",
  "helper-setup-read-context",
  "helper-setup-read-parameter",
  "helper-setup-lost-ack",
  "helper-setup-rejected",
  "helper-setup-ack-drain",
  "helper-setup-ack-guard",
  "helper-setup-ack-cancel",
  "helper-setup-ack-eof",
  "helper-setup-close-before-ack",
  "helper-setup-close-after-ack",
  "untrusted-tls",
  "fixture-boundary",
]);
const output = process.stdout.write.bind(process.stdout);
const report = {
  scenario,
  transport,
  sdkVersion: "3.4.1",
  sourcePins: false,
  guardEnabled: scenario !== "mutation-no-guard",
  connected: false,
  queryPosts: 0,
  effects: 0,
  intendedSQLOnly: true,
  scalarBindsExact: true,
  queryIdsSame: true,
  queryCallbacks: 0,
  rows: 0,
  violations: 0,
  outcome: "pending",
  cancelAcknowledged: false,
  renewalRequests: 0,
  destinationRequests: 0,
  destinationConnections: 0,
  networkDenied: 0,
  dnsDenied: 0,
  proxyConnects: 0,
  tlsVerified: false,
  knownSuccesses: 0,
  listenersClosed: false,
};
let phase = "arguments",
  finishing = false;
let deadline;
const sockets = new Set(),
  listeners = [];
let firstId, firstQueryTime, firstQueryProxyConnects, guard, owner, ticket, statement;
let helper;
let currentAcknowledged = false,
  currentCompleted = false;
let resolveViolation, resolveFirstQuery;
const violated = new Promise((resolve) => {
  resolveViolation = resolve;
});
const firstQuery = new Promise((resolve) => {
  resolveFirstQuery = resolve;
});
const SQL = "SELECT ? AS EXACT_VALUE, ? AS ENABLED, ? AS EMPTY_VALUE";
const switchScenario = scenario.startsWith("helper-switch-");
const transactionScenario = scenario.startsWith("helper-transaction-");
const dmlScenario = scenario.startsWith("helper-dml-");
const ddlScenario = scenario.startsWith("helper-ddl-");
const setupScenario = scenario.startsWith("helper-setup-");
const reviewedScenario = scenario.startsWith("reviewed-");
const batchScenario = scenario.startsWith("helper-batch-");
const setupRunId = "0123456789ABCDEF0123456789ABCDEF";
const setupLedger = deriveSetupLedger(setupRunId);
const setupStep =
  scenario === "helper-setup-read-context"
    ? 3
    : scenario === "helper-setup-read-parameter"
      ? 4
      : scenario === "helper-setup-schema"
        ? 14
        : scenario === "helper-setup-table"
          ? 20
          : 8;
const setupEntry = setupLedger[setupStep - 1];
const setupColumnCount = setupEntry.mutation ? 1 : setupEntry.maximumColumns;
const setupColumns = Array.from({ length: setupColumnCount }, (_, i) =>
  setupEntry.mutation ? "status" : `FIELD_${i}`,
);
const setupValues = setupColumns.map(() =>
  setupEntry.mutation ? "synthetic CREATE status" : "synthetic observation",
);
const dmlKind =
  scenario === "helper-dml-insert"
    ? "insert"
    : scenario === "helper-dml-delete"
      ? "delete"
      : "update";
const dmlSQL = {
  insert: "INSERT INTO SYNTHETIC.PUBLIC.T (A,B,C) VALUES (?,?,?)",
  update: "UPDATE SYNTHETIC.PUBLIC.T SET A=?, B=? WHERE C=?",
  delete: "DELETE FROM SYNTHETIC.PUBLIC.T WHERE A=? AND B=? AND C=?",
}[dmlKind];
const dmlColumn = {
  insert: "number of rows inserted",
  update: "number of rows updated",
  delete: "number of rows deleted",
}[dmlKind];
const ddlKind =
  scenario === "helper-ddl-create"
    ? "createTable"
    : scenario === "helper-ddl-add-column"
      ? "addColumn"
      : "dropTable";
const ddlSQL = {
  createTable: 'CREATE TABLE "SYNTHETIC"."PUBLIC"."T" ("ID" NUMBER(38,0) NOT NULL)',
  addColumn: 'ALTER TABLE "SYNTHETIC"."PUBLIC"."T" ADD COLUMN "NOTE" VARCHAR(255)',
  dropTable: 'DROP TABLE "SYNTHETIC"."PUBLIC"."T" RESTRICT',
}[ddlKind];
const transactionAction =
  scenario === "helper-transaction-begin"
    ? "begin"
    : scenario === "helper-transaction-rollback"
      ? "rollback"
      : "commit";
const transactionSQL = { begin: "BEGIN TRANSACTION", commit: "COMMIT", rollback: "ROLLBACK" }[
  transactionAction
];
const SWITCH_SCHEMA = 'Quoted."雪';
const SWITCH_SQL = `USE SCHEMA "SYNTHETIC"."${SWITCH_SCHEMA.replaceAll('"', '""')}"`;
const CONTEXT_SQL =
  "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()";
const switchIds = new Set();
const binds = ["9007199254740993", true, null];
const expectedBindings = {
  1: { type: "TEXT", value: binds[0] },
  2: { type: "BOOLEAN", value: "true" },
  3: { type: "TEXT", value: null },
};
const batchExpected = Object.freeze({
  user: "SERVER_USER",
  role: "SYNTHETIC",
  database: "SYNTHETIC",
  schema: "PUBLIC",
  warehouse: "SYNTHETIC",
  session: "1",
});
const batchTransactionId = "9007199254740993";
const batchHugeCount = "99999999999999999999999999999999999999";
const batchManual = scenario.startsWith("helper-batch-manual-");
const batchUnits = ["insert", "update", "delete"].map((kind) => ({
  kind,
  sql: {
    insert: "INSERT INTO SYNTHETIC.PUBLIC.T (A,B,C) VALUES (?,?,?)",
    update: "UPDATE SYNTHETIC.PUBLIC.T SET A=?, B=? WHERE C=?",
    delete: "DELETE FROM SYNTHETIC.PUBLIC.T WHERE A=? AND B=? AND C=?",
  }[kind],
  binds,
}));
const batchControl = (role, sql = MUTATION_CONTEXT_SQL) => ({ role, sql, unit: null });
const batchUnitPlan = batchUnits.map((unit, index) => ({
  role: "unit",
  sql: unit.sql,
  unit: index,
}));
const batchPlan = [
  batchControl("context"),
  batchControl("settings", MUTATION_SETTINGS_SQL),
  batchControl("begin", "BEGIN TRANSACTION"),
  batchControl("transaction"),
  ...batchUnitPlan.slice(
    0,
    batchManual || ["helper-batch-cardinality", "helper-batch-lost-ack"].includes(scenario) ? 2 : 3,
  ),
  ...(scenario === "helper-batch-lost-ack"
    ? []
    : scenario === "helper-batch-cardinality"
      ? [batchControl("rollback", "ROLLBACK"), batchControl("idle")]
      : batchManual
        ? [
            batchControl("verify"),
            ...(scenario === "helper-batch-manual-commit"
              ? [
                  batchControl("context"),
                  batchControl("settings", MUTATION_SETTINGS_SQL),
                  batchControl("commit", "COMMIT"),
                  batchControl("idle"),
                ]
              : [batchControl("rollback", "ROLLBACK"), batchControl("idle")]),
          ]
        : [batchControl("verify"), batchControl("commit", "COMMIT"), batchControl("idle")]),
  ...(scenario === "helper-batch-lost-ack"
    ? []
    : [batchControl("disposition", `DESCRIBE TRANSACTION ${batchTransactionId}`)]),
];
const batchPosts = [];
let batchTransaction = null,
  batchDisposition = null;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const bounded = (promise, ms = 12_000) =>
  Promise.race([
    promise,
    new Promise((_, reject) => {
      const timer = setTimeout(() => reject(new Error("bounded")), ms);
      timer.unref();
    }),
  ]);
function track(server) {
  listeners.push(server);
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });
  return server;
}
async function listen(server) {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return server.address().port;
}
function json(response, body) {
  response.writeHead(200, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}
function session() {
  return {
    token: "SYNTHETIC_SESSION",
    masterToken: "SYNTHETIC_MASTER",
    validityInSeconds: 3600,
    masterValidityInSeconds: 3600,
    sessionId: 1,
    parameters: [],
    sessionInfo: {
      databaseName: "SYNTHETIC",
      schemaName: "PUBLIC",
      roleName: "SYNTHETIC",
      warehouseName: "SYNTHETIC",
    },
  };
}
function success() {
  return {
    success: true,
    data: {
      queryId: "11111111-1111-4111-8111-111111111111",
      statementTypeId: 4096,
      version: 1,
      rowtype: [{ name: "RESULT", type: "text", length: 32, nullable: false }],
      rowset: [["synthetic-success"]],
      total: 1,
      returned: 1,
      parameters: [],
    },
  };
}
async function finish(failed = false) {
  if (finishing) return;
  finishing = true;
  clearTimeout(deadline);
  if (failed) report.harnessFailure = phase;
  for (const socket of sockets) socket.destroy();
  await Promise.all(listeners.map((server) => new Promise((resolve) => server.close(resolve))));
  report.listenersClosed = true;
  output(JSON.stringify(report) + "\n");
  // SDK timers are intentionally left alive in the quarantine scenarios; this
  // owned child is retired here and parent observes close before removing root.
  process.exit(failed ? 1 : 0);
}
process.on("uncaughtException", () => {
  void finish(true);
});
process.on("unhandledRejection", () => {
  void finish(true);
});
deadline = setTimeout(() => {
  phase = "deadline";
  void finish(true);
}, 18_000);

try {
  assert.equal(process.argv.length, 4);
  assert(allowed.has(scenario));
  assert(["http", "tls", "proxy"].includes(transport));
  const root = process.env.JOINERY_GUARD_ROOT;
  assert(root && isAbsolute(root));
  const state = await lstat(root);
  assert(state.isDirectory() && !state.isSymbolicLink());
  assert.equal(state.mode & 0o777, 0o700);
  assert.equal(state.uid, process.getuid());
  await writeFile(
    join(root, "disabled-config.json"),
    JSON.stringify({
      common: { log_level: "OFF", log_path: root },
    }),
    { mode: 0o600, flag: "wx" },
  );
  const tlsOptions =
    transport === "http"
      ? undefined
      : {
          key: await readFile(join(root, "tls-key.pem")),
          cert: await readFile(join(root, "tls-cert.pem")),
        };
  phase = "listeners";
  const destination = track(
    http.createServer((_request, response) => {
      report.destinationRequests++;
      response.end();
    }),
  );
  destination.on("connection", () => {
    report.destinationConnections++;
  });
  const destinationPort = await listen(destination);
  const handler = async (request, response) => {
    try {
      let size = 0;
      const chunks = [];
      for await (const chunk of request) {
        size += chunk.length;
        assert(size <= 1_048_576);
        chunks.push(chunk);
      }
      const bytes = Buffer.concat(chunks);
      const body = bytes.length
        ? JSON.parse(
            (request.headers["content-encoding"] === "gzip"
              ? gunzipSync(bytes, { maxOutputLength: 1_048_576 })
              : bytes
            ).toString(),
          )
        : {};
      const url = new URL(request.url, "http://127.0.0.1");
      if (url.pathname === "/dml-status-chunk") {
        assert(
          [
            "helper-dml-ack-drain",
            "helper-ddl-ack-drain",
            "helper-setup-ack-drain",
            "reviewed-ack-drain",
          ].includes(scenario),
        );
        assert.equal(request.method, "GET");
        report.resultChunkRequests = (report.resultChunkRequests ?? 0) + 1;
        response.writeHead(404);
        response.end();
        return;
      }
      if (url.pathname === "/session/v1/login-request")
        return json(response, { success: true, data: session() });
      if (url.pathname === "/session/token-request") {
        report.renewalRequests++;
        return json(response, { success: true, data: session() });
      }
      if (url.pathname.endsWith("/abort-request")) {
        json(response, { success: true, data: {} });
        return;
      }
      if (url.pathname === "/session" || url.pathname === "/telemetry/send")
        return json(response, { success: true, data: {} });
      assert.equal(url.pathname, "/queries/v1/query-request");
      assert.equal(request.method, "POST");
      report.queryPosts++;
      const id = url.searchParams.get("requestId");
      if (firstId === undefined) {
        firstId = id;
        firstQueryTime = Date.now();
        firstQueryProxyConnects = report.proxyConnects;
      } else if (!batchScenario) report.queryIdsSame &&= firstId === id;
      if (batchScenario) {
        const expected = batchPlan[batchPosts.length];
        assert(expected, "no suffix/replay POST after the closed batch plan");
        assert.equal(body.sqlText, expected.sql);
        assert.deepEqual(body.parameters, { MULTI_STATEMENT_COUNT: 1 });
        assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
        assert(!batchPosts.some((post) => post.id === id), "one admitted POST per SDK identity");
        if (expected.role === "unit") assert.deepEqual(body.bindings, expectedBindings);
        else assert(body.bindings === undefined || JSON.stringify(body.bindings) === "{}");
        const queryId = `11111111-1111-4111-8111-${String(report.queryPosts).padStart(12, "0")}`;
        batchPosts.push({ ...expected, id, queryId });
        resolveFirstQuery();
        const reply = success();
        reply.data.queryId = queryId;
        const textColumns = (names) =>
          names.map((name) => ({ name, type: "text", length: 1024, nullable: true }));
        if (["context", "transaction", "verify", "idle"].includes(expected.role)) {
          reply.data.rowtype = textColumns([
            "user",
            "role",
            "database",
            "schema",
            "warehouse",
            "session",
            "transaction",
          ]);
          reply.data.rowset = [
            [
              batchExpected.user,
              batchExpected.role,
              batchExpected.database,
              batchExpected.schema,
              batchExpected.warehouse,
              batchExpected.session,
              batchTransaction,
            ],
          ];
        } else if (expected.role === "settings") {
          reply.data.rowtype = textColumns(["key", "value"]);
          reply.data.rowset = [["QUOTED_IDENTIFIERS_IGNORE_CASE", "false"]];
        } else if (expected.role === "disposition") {
          reply.data.rowtype = [
            { name: "id", type: "fixed", precision: 38, scale: 0, nullable: false },
            { name: "session", type: "fixed", precision: 38, scale: 0, nullable: false },
            ...textColumns(["state"]),
          ];
          reply.data.rowset = [[batchTransactionId, batchExpected.session, batchDisposition]];
        } else if (expected.role === "unit") {
          report.effects++;
          if (scenario === "helper-batch-lost-ack" && expected.unit === 1)
            return request.socket.destroy();
          const kind = batchUnits[expected.unit].kind;
          // The manual cases exercise the SDK's actual one-column UPDATE
          // metadata. Absence remains null; no zero observation is invented.
          const multi = kind === "update" && !batchManual;
          reply.data.statementTypeId = { insert: 12288, update: 12289, delete: 12290 }[kind];
          reply.data.rowtype = [
            `number of rows ${kind === "insert" ? "inserted" : kind === "update" ? "updated" : "deleted"}`,
            ...(multi ? ["number of multi-joined rows updated"] : []),
          ].map((name) => ({ name, type: "fixed", precision: 38, scale: 0, nullable: false }));
          reply.data.rowset = [
            [
              scenario === "helper-batch-cardinality" && expected.unit === 1 ? batchHugeCount : "1",
              ...(multi ? ["0"] : []),
            ],
          ];
        } else {
          if (expected.role === "begin") batchTransaction = batchTransactionId;
          else {
            assert.equal(batchTransaction, batchTransactionId);
            batchDisposition = expected.role === "commit" ? "committed" : "rolled back";
            batchTransaction = null;
          }
          reply.data.rowtype = textColumns(["status"]);
          reply.data.rowset = [["Synthetic boundary acknowledged"]];
        }
        reply.data.total = reply.data.returned = reply.data.rowset.length;
        return json(response, reply);
      }
      report.intendedSQLOnly &&=
        body.sqlText ===
        (setupScenario
          ? setupEntry.sql
          : dmlScenario
            ? dmlSQL
            : ddlScenario
              ? ddlSQL
              : transactionScenario
                ? transactionSQL
                : switchScenario
                  ? report.queryPosts === 1
                    ? SWITCH_SQL
                    : CONTEXT_SQL
                  : SQL);
      report.scalarBindsExact &&=
        setupScenario || switchScenario || transactionScenario || ddlScenario
          ? body.bindings === undefined || JSON.stringify(body.bindings) === "{}"
          : JSON.stringify(body.bindings) === JSON.stringify(expectedBindings);
      resolveFirstQuery();
      if (reviewedScenario) {
        report.singleStatementParameter = body.parameters?.MULTI_STATEMENT_COUNT === 1;
        assert.deepEqual(body.parameters, { MULTI_STATEMENT_COUNT: 1 });
        const reply = success();
        reply.data.finalDatabaseName = "RESPONSE_DB";
        reply.data.finalSchemaName = "RESPONSE_SCHEMA";
        reply.data.finalRoleName = scenario === "reviewed-context-partial" ? null : "RESPONSE_ROLE";
        if (scenario !== "reviewed-context-partial") reply.data.finalWarehouseName = "RESPONSE_WH";
        if (scenario === "reviewed-ack-drain") {
          reply.data.rowset = [];
          reply.data.chunks = [
            {
              url: `${transport === "http" ? "http" : "https"}://127.0.0.1:${port}/dml-status-chunk`,
              rowCount: 1,
            },
          ];
        }
        if (scenario === "reviewed-timeout-policy") await pause(1500);
        return json(response, reply);
      }
      if (setupScenario) {
        if (scenario === "helper-setup-rejected")
          return json(response, {
            success: false,
            code: "100038",
            message: "synthetic CREATE rejection",
            data: { sqlState: "42501", queryId: "11111111-1111-4111-8111-111111111111" },
          });
        if (setupEntry.mutation) report.effects++;
        if (scenario === "helper-setup-close-before-ack") return;
        if (scenario === "helper-setup-lost-ack") return request.socket.destroy();
        const reply = success();
        reply.data.statementTypeId = 4096;
        reply.data.rowtype = setupColumns.map((name) => ({
          name,
          type: "text",
          length: 1024,
          nullable: false,
        }));
        reply.data.rowset = [setupValues];
        if (scenario === "helper-setup-ack-drain") {
          reply.data.rowset = [];
          reply.data.chunks = [
            {
              url: `${transport === "http" ? "http" : "https"}://127.0.0.1:${port}/dml-status-chunk`,
              rowCount: 1,
            },
          ];
        }
        return json(response, reply);
      }
      if (dmlScenario) {
        if (scenario === "helper-dml-database")
          return json(response, {
            success: false,
            code: "100038",
            message: "synthetic DML rejection",
            data: { sqlState: "42501", queryId: "11111111-1111-4111-8111-111111111111" },
          });
        report.effects++;
        if (scenario === "helper-dml-lost-ack") return request.socket.destroy();
        const reply = success();
        reply.data.statementTypeId = { insert: 12288, update: 12289, delete: 12290 }[dmlKind];
        reply.data.rowtype = [
          dmlColumn,
          ...(dmlKind === "update" ? ["number of multi-joined rows updated"] : []),
        ].map((name) => ({
          name,
          type: "fixed",
          precision: 38,
          scale: 0,
          nullable: false,
        }));
        reply.data.rowset = [["9007199254740993", ...(dmlKind === "update" ? ["7"] : [])]];
        if (scenario === "helper-dml-ack-drain") {
          reply.data.rowset = [];
          reply.data.chunks = [
            {
              url: `${transport === "http" ? "http" : "https"}://127.0.0.1:${port}/dml-status-chunk`,
              rowCount: 1,
            },
          ];
        }
        return json(response, reply);
      }
      if (ddlScenario) {
        if (scenario === "helper-ddl-database")
          return json(response, {
            success: false,
            code: "100038",
            message: "synthetic DDL rejection",
            data: { sqlState: "42501", queryId: "11111111-1111-4111-8111-111111111111" },
          });
        report.effects++;
        if (scenario === "helper-ddl-lost-ack") return request.socket.destroy();
        if (scenario === "helper-ddl-close-before-ack") return;
        const reply = success();
        reply.data.statementTypeId = 4096;
        reply.data.rowtype = [
          {
            name: "status",
            type: "text",
            length: 1024,
            nullable: false,
          },
        ];
        reply.data.rowset = [["Synthetic DDL acknowledged"]];
        if (scenario === "helper-ddl-ack-drain") {
          reply.data.rowset = [];
          reply.data.chunks = [
            {
              url: `${transport === "http" ? "http" : "https"}://127.0.0.1:${port}/dml-status-chunk`,
              rowCount: 1,
            },
          ];
        }
        return json(response, reply);
      }
      if (transactionScenario) {
        if (scenario === "helper-transaction-database")
          return json(response, {
            success: false,
            code: "100038",
            message: "synthetic boundary rejection",
            data: { sqlState: "42501", queryId: "11111111-1111-4111-8111-111111111111" },
          });
        report.effects++;
        if (scenario === "helper-transaction-lost-ack") return request.socket.destroy();
        const reply = success();
        reply.data.rowtype = [{ name: "status", type: "text", length: 1024, nullable: false }];
        reply.data.rowset = [["Synthetic boundary acknowledged"]];
        reply.data.total = reply.data.returned = 1;
        return json(response, reply);
      }
      if (switchScenario) {
        switchIds.add(id);
        report.switchIdsDistinct = switchIds.size === report.queryPosts;
        if (report.queryPosts === 1) report.effects++;
        if (
          (scenario === "helper-switch-lost-use" && report.queryPosts === 1) ||
          (scenario === "helper-switch-lost-readback" && report.queryPosts === 2)
        )
          return request.socket.destroy();
        if (scenario === "helper-switch-readback-denied" && report.queryPosts === 2)
          return json(response, {
            success: false,
            code: "100038",
            message: "synthetic readback rejection",
            data: { sqlState: "42501", queryId: "11111111-1111-4111-8111-111111111112" },
          });
        const reply = success();
        const use = report.queryPosts === 1;
        reply.data.queryId = use
          ? "11111111-1111-4111-8111-111111111111"
          : "11111111-1111-4111-8111-111111111112";
        reply.data.rowtype = use
          ? []
          : ["user", "role", "database", "schema", "warehouse", "session"].map((name) => ({
              name,
              type: "text",
              length: 1024,
              nullable: true,
            }));
        reply.data.rowset = use
          ? []
          : [["SERVER_USER", null, "SYNTHETIC", SWITCH_SCHEMA, null, "OPAQUE-SESSION"]];
        reply.data.total = reply.data.returned = use ? 0 : 1;
        return json(response, reply);
      }
      if (scenario.startsWith("redirect-")) {
        const status = Number(scenario.slice("redirect-".length));
        // Each tested redirect simultaneously changes path and authority. The
        // 307 variant also changes protocol, including TLS -> HTTP downgrades.
        const protocol = status === 307 ? (transport === "http" ? "https" : "http") : "http";
        response.writeHead(status, {
          Location: `${protocol}://localhost:${destinationPort}/changed`,
        });
        response.end();
        return;
      }
      if (scenario === "renewal")
        return json(response, {
          success: false,
          code: "390112",
          message: "synthetic expired",
          data: {},
        });
      if (scenario === "helper-database")
        return json(response, {
          success: false,
          code: "100038",
          message: "synthetic failure",
          data: { sqlState: "22018", queryId: "11111111-1111-4111-8111-111111111111" },
        });
      if (scenario === "status-503" || scenario === "status-429") {
        response.writeHead(Number(scenario.slice(7)), { "content-type": "application/json" });
        response.end("{}");
        return;
      }
      if (scenario === "reset") return request.socket.destroy();
      if (scenario === "cancel-during") {
        report.effects++;
        request.socket.destroy();
        return;
      }
      if (["lost-ack", "helper-lost-ack", "mutation-no-guard", "delayed-loss"].includes(scenario)) {
        report.effects++;
        if (scenario === "mutation-no-guard" && report.queryPosts === 2)
          return json(response, success());
        if (scenario === "delayed-loss") await pause(250);
        request.socket.destroy();
        return;
      }
      report.effects++;
      json(response, success());
    } catch {
      phase = "server-contract";
      response.destroy();
      void finish(true);
    }
  };
  const server = track(
    transport === "http" ? http.createServer(handler) : https.createServer(tlsOptions, handler),
  );
  const port = await listen(server);
  let proxyPort;
  if (transport === "proxy") {
    const proxy = track(
      http.createServer((_request, response) => {
        response.writeHead(405);
        response.end();
      }),
    );
    proxy.on("connect", (request, socket, head) => {
      if (request.url !== `127.0.0.1:${port}` || head.length !== 0) {
        phase = "proxy-contract";
        socket.destroy();
        void finish(true);
        return;
      }
      report.proxyConnects++;
      const target = net.connect({ host: "127.0.0.1", port });
      sockets.add(target);
      target.on("close", () => sockets.delete(target));
      target.on("error", () => socket.destroy());
      socket.on("error", () => target.destroy());
      target.once("connect", () => {
        socket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        target.pipe(socket);
        socket.pipe(target);
      });
    });
    proxyPort = await listen(proxy);
  }
  const allowedPorts = new Set([port, destinationPort, ...(proxyPort ? [proxyPort] : [])]);
  const originalConnect = net.Socket.prototype.connect;
  net.Socket.prototype.connect = function (...args) {
    let first = args[0];
    if (Array.isArray(first)) first = first[0];
    const host = typeof first === "object" ? first.host : args[1];
    const targetPort = typeof first === "object" ? first.port : first;
    if (!["127.0.0.1", "localhost"].includes(host) || !allowedPorts.has(Number(targetPort))) {
      report.networkDenied++;
      throw new Error("Non-owned socket denied");
    }
    return originalConnect.apply(this, args);
  };
  const originalTLS = tls.connect;
  tls.connect = function (...args) {
    const socket = originalTLS.apply(this, args);
    socket.once("secureConnect", () => {
      report.tlsVerified ||= socket.authorized && socket.authorizationError === null;
    });
    socket.once("error", (error) => {
      if (["DEPTH_ZERO_SELF_SIGNED_CERT", "SELF_SIGNED_CERT_IN_CHAIN"].includes(error.code))
        report.tlsUntrustedRejected = true;
    });
    return socket;
  };
  const dnsMethod = /^(lookup|resolve|reverse)/;
  const denyDNS = () => {
    report.dnsDenied++;
    throw new Error("DNS denied");
  };
  const denyDNSPromise = async () => denyDNS();
  for (const method of Object.getOwnPropertyNames(dns))
    if (dnsMethod.test(method) && typeof dns[method] === "function") dns[method] = denyDNS;
  for (const method of Object.getOwnPropertyNames(dns.promises))
    if (dnsMethod.test(method) && typeof dns.promises[method] === "function")
      dns.promises[method] = denyDNSPromise;
  for (const [Constructor, deny] of [
    [dns.Resolver, denyDNS],
    [dns.promises.Resolver, denyDNSPromise],
  ]) {
    for (
      let prototype = Constructor.prototype;
      prototype !== Object.prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      for (const method of Object.getOwnPropertyNames(prototype))
        if (dnsMethod.test(method) && typeof prototype[method] === "function")
          prototype[method] = deny;
    }
  }
  // Localhost is a constant mapping; this never consults an OS resolver.
  dns.lookup = (hostname, options, callback) => {
    if (typeof options === "function") {
      callback = options;
      options = {};
    }
    if (hostname !== "localhost") {
      return denyDNS();
    }
    process.nextTick(() =>
      options?.all
        ? callback(null, [{ address: "127.0.0.1", family: 4 }])
        : callback(null, "127.0.0.1", 4),
    );
  };
  const denyUDP = () => {
    report.networkDenied++;
    throw new Error("UDP denied");
  };
  const OriginalDatagramSocket = dgram.Socket;
  for (const method of ["bind", "connect", "send", "sendto"])
    OriginalDatagramSocket.prototype[method] = denyUDP;
  dgram.createSocket = denyUDP;
  dgram.Socket = function () {
    return denyUDP();
  };
  syncBuiltinESMExports();
  for (const method of ["log", "info", "warn", "error", "debug", "trace"])
    console[method] = () => {};
  phase = "source-pins";
  report.sourcePins = verifyTransportSources().files === 15;
  if (scenario === "fixture-boundary") {
    phase = "boundary-check";
    let dnsChecks = 0;
    for (const object of [dns, new dns.Resolver()]) {
      const methods = new Set();
      for (
        let prototype = object;
        prototype !== Object.prototype;
        prototype = Object.getPrototypeOf(prototype)
      ) {
        for (const method of Object.getOwnPropertyNames(prototype))
          if (dnsMethod.test(method) && typeof object[method] === "function") methods.add(method);
      }
      for (const method of methods) {
        assert.throws(() => object[method]("unowned.invalid"), /DNS denied/);
        dnsChecks++;
      }
    }
    for (const object of [dns.promises, new dns.promises.Resolver()]) {
      const methods = new Set();
      for (
        let prototype = object;
        prototype !== Object.prototype;
        prototype = Object.getPrototypeOf(prototype)
      ) {
        for (const method of Object.getOwnPropertyNames(prototype))
          if (dnsMethod.test(method) && typeof object[method] === "function") methods.add(method);
      }
      for (const method of methods) {
        await assert.rejects(() => object[method]("unowned.invalid"), /DNS denied/);
        dnsChecks++;
      }
    }
    assert.equal(report.dnsDenied, dnsChecks);
    assert(dnsChecks >= 60);
    assert.throws(() => dgram.createSocket("udp4"), /UDP denied/);
    assert.throws(() => new dgram.Socket("udp4"), /UDP denied/);
    for (const method of ["bind", "connect", "send", "sendto"])
      assert.throws(() => OriginalDatagramSocket.prototype[method](), /UDP denied/);
    const deniedSocket = new net.Socket();
    try {
      assert.throws(
        () => deniedSocket.connect({ host: "unowned.invalid", port }),
        /Non-owned socket denied/,
      );
    } finally {
      deniedSocket.destroy();
    }
    await new Promise((resolve, reject) =>
      dns.lookup("localhost", { all: true }, (error, addresses) => {
        if (error) return reject(error);
        assert.deepEqual(addresses, [{ address: "127.0.0.1", family: 4 }]);
        resolve();
      }),
    );
    report.boundaryDNSChecks = dnsChecks;
    report.boundaryVerified = true;
    report.outcome = "fixture-boundary-verified";
    await finish();
  }
  if (report.guardEnabled)
    guard = installQuerySubmitGuard({
      onViolation(event) {
        report.violations++;
        report.violationCategory = event.category;
        report.admittedAtViolation = event.admitted;
        report.violationLatencyMs = firstQueryTime === undefined ? 0 : Date.now() - firstQueryTime;
        report.proxyConnectsAtViolation = report.proxyConnects;
        if (helper) {
          helper.submissionViolation();
          report.helperSettledSynchronously = report.helperTerminals === 1 && helper.retired;
        } else if (!currentCompleted)
          report.outcome = currentAcknowledged
            ? "harness-ConnectionClosed"
            : "harness-OutcomeUnknown";
        resolveViolation();
      },
    });
  const protocol = transport === "http" ? "http" : "https";
  guard?.setAuthority({ protocol: `${protocol}:`, hostname: "127.0.0.1", port });
  phase = "sdk-import";
  const require = createRequire(new URL("../package.json", import.meta.url));
  // Test-only observation at the actual Axios request facade. Pinned Axios
  // calls RedirectableRequest.setTimeout; follow-redirects delegates that to
  // the socket and its own timer, bypassing ClientRequest.setTimeout. Keep the
  // real facade, SDK, admission guard, receiver, arguments and return values.
  if (scenario === "reviewed-timeout-policy") {
    report.observedQueryTimeouts = [];
    report.queryTimeoutObservationLayer = "follow-redirects-request";
    const redirects = require("follow-redirects");
    for (const transport of [redirects.http, redirects.https]) {
      const originalRequest = transport.request;
      transport.request = function (...args) {
        const queryRequest = args[0]?.path?.includes("/queries/v1/query-request");
        const request = Reflect.apply(originalRequest, this, args);
        if (queryRequest) {
          const originalSetTimeout = request.setTimeout;
          request.setTimeout = function (milliseconds, ...timeoutArgs) {
            report.observedQueryTimeouts.push(milliseconds);
            return Reflect.apply(originalSetTimeout, this, [milliseconds, ...timeoutArgs]);
          };
        }
        return request;
      };
    }
  }
  const sdk = require("snowflake-sdk");
  assert.equal(require("snowflake-sdk/package.json").version, "3.4.1");
  sdk.configure({
    logLevel: "OFF",
    additionalLogToConsole: false,
    customLogger: { error() {}, warn() {}, info() {}, debug() {}, trace() {} },
  });
  const connection = sdk.createConnection({
    account: "synthetic",
    host: "127.0.0.1",
    port,
    protocol,
    accessUrl: `${protocol}://127.0.0.1:${port}`,
    username: "SYNTHETIC",
    authenticator: "OAUTH",
    token: "SYNTHETIC_OAUTH",
    clientConfigFile: join(root, "disabled-config.json"),
    clientSessionKeepAlive: false,
    clientRequestMFAToken: false,
    clientStoreTemporaryCredential: false,
    keepAlive: false,
    timeout:
      scenario === "reviewed-timeout-policy"
        ? connectionOptions(
            { reviewedConsoleTimeoutSeconds: 120 },
            join(root, "disabled-config.json"),
          ).timeout
        : 1000,
    arrayBindingThreshold: 1_000_000,
    ...(scenario === "untrusted-tls" ? { retryTimeout: 0, sfRetryMaxLoginRetries: 0 } : {}),
    ...(proxyPort ? { proxyHost: "127.0.0.1", proxyPort, proxyProtocol: "http" } : {}),
  });
  phase = "sdk-connect";
  try {
    await bounded(
      new Promise((resolve, reject) =>
        connection.connect((error) => (error ? reject(error) : resolve())),
      ),
    );
  } catch (error) {
    if (scenario === "untrusted-tls" && report.tlsUntrustedRejected) {
      report.outcome = "tls-untrusted-rejected";
      assert.equal(report.queryPosts, 0);
      await finish();
    }
    throw error;
  }
  assert.notEqual(scenario, "untrusted-tls");
  report.connected = true;
  if (reviewedScenario) {
    // Only authentication/bootstrap is seeded. SDK execute, response getters,
    // transport registration, result streaming and source-pinned veto are real.
    phase = "preconnected-reviewed-helper";
    const { isPutCommand, isGetCommand } = require("snowflake-sdk/dist/lib/util.js");
    report.helperFixturePreconnected = true;
    report.externalBrowserConnectSkipped = true;
    report.helperTerminals = 0;
    report.helperRetired = 0;
    const events = [];
    let resolveTerminal;
    let terminal = new Promise((resolve) => {
      resolveTerminal = resolve;
    });
    helper = new Helper(
      sdk,
      (event) => {
        events.push(event);
        if (event.type === "rows") report.rows += event.rows.length;
        if (event.type === "statementAcknowledged") {
          report.originalAck = event.sdkRequestId === firstId;
          report.ackBeforeTerminal = report.helperTerminals === 0;
          report.passiveContext = event.context;
          report.ackIdentityRegistered = events.some(
            (e) =>
              e.type === "statement" &&
              e.sdkRequestId === event.sdkRequestId &&
              (!event.queryId || e.queryId === event.queryId),
          );
        }
        if (event.type === "terminal") {
          report.helperTerminals++;
          report.helperTerminal = event.ok ? "success" : event.error.code;
          resolveTerminal();
        }
      },
      () => report.helperRetired++,
      {
        submitGuard: guard,
        clientConfigFile: join(root, "disabled-config.json"),
        isFileTransfer: (sql) => isPutCommand(sql) || isGetCommand(sql),
      },
    );
    helper.connection = connection;
    helper.state = "connected";
    helper.generation = "11111111-1111-4111-8111-111111111111";
    helper.identityPin = { user: "SERVER_USER", session: "OPAQUE-SESSION" };
    const send = (id, sql) =>
      helper.receive({
        v: 1,
        generation: helper.generation,
        id,
        op: "reviewedSql",
        payload: {
          sql,
          binds,
          expected: { user: "SERVER_USER", session: "OPAQUE-SESSION" },
          credits: 1,
          maxRows: 4,
          maxBytes: 4096,
        },
      });
    if (scenario === "reviewed-file-veto") {
      const candidates = [
        "PUT file:///unreadable @stage",
        " get @stage file:///unwritten",
        "\ufeffpUt file:///none @stage",
        "GETTING_NOT_SQL",
      ];
      for (let i = 0; i < candidates.length; i++) {
        terminal = new Promise((resolve) => {
          resolveTerminal = resolve;
        });
        assert(isPutCommand(candidates[i]) || isGetCommand(candidates[i]));
        send(String(i + 1), candidates[i]);
        await bounded(terminal);
        assert.equal(report.helperTerminal, "unsupported");
      }
      // Comment-prefixed strings take the same non-file SDK branch. They are
      // not claimed safe SQL; the server remains the SQL validity boundary.
      for (const sql of [
        "/* comment */ PUT file:///none @stage",
        "-- comment\nGET @stage file:///none",
        "SELECT 'PUT'",
      ])
        assert.equal(isPutCommand(sql) || isGetCommand(sql), false);
      report.vetoCases = candidates.length;
      assert.equal(report.queryPosts, 0);
      assert.equal(guard.snapshot().admitted, 0);
      assert.equal(report.helperRetired, 0);
    } else {
      send("1", SQL);
      await bounded(terminal);
      assert.equal(report.queryPosts, 1);
      if (scenario === "reviewed-timeout-policy")
        assert.deepEqual(report.observedQueryTimeouts, [125000]);
      assert.equal(report.singleStatementParameter, true);
      assert.equal(report.originalAck, true);
      assert.equal(report.ackBeforeTerminal, true);
      assert.equal(report.ackIdentityRegistered, true);
      assert.deepEqual(report.passiveContext, {
        database: "RESPONSE_DB",
        schema: "RESPONSE_SCHEMA",
        role: scenario === "reviewed-context-partial" ? null : "RESPONSE_ROLE",
        warehouse: scenario === "reviewed-context-partial" ? null : "RESPONSE_WH",
      });
      assert.equal(
        report.helperTerminal,
        scenario === "reviewed-ack-drain" ? "connectionClosed" : "success",
      );
      assert.equal(report.helperRetired, scenario === "reviewed-ack-drain" ? 1 : 0);
      assert.equal(report.rows, scenario === "reviewed-ack-drain" ? 0 : 1);
    }
    report.guardAdmitted = guard.snapshot().admitted;
    report.outcome = `reviewed-${report.helperTerminal}`;
    if (!helper.retired) helper.stop();
    await finish();
  }
  if (setupScenario) {
    phase = "preconnected-setup-helper";
    report.helperFixturePreconnected = true;
    report.externalBrowserConnectSkipped = true;
    report.setupCursorFixtureSeeded = true;
    report.helperTerminals = 0;
    report.helperRetired = 0;
    report.helperSetupAcknowledged = false;
    report.rows = 0;
    report.publicColumns = 0;
    let resolveTerminal;
    const terminal = new Promise((resolve) => {
      resolveTerminal = resolve;
    });
    helper = new Helper(
      sdk,
      (event) => {
        if (event.id === "2" && event.type === "terminal")
          report.helperCancelRequested = event.ok && event.result.requested === true;
        if (event.id !== "1") return;
        if (event.type === "columns") report.publicColumns++;
        if (event.type === "rows") report.rows += event.rows.length;
        if (event.type === "setupAcknowledged") {
          report.helperSetupAcknowledged =
            event.runId === setupRunId &&
            event.step === setupStep &&
            event.key === setupEntry.key &&
            event.sdkRequestId === firstId;
          report.setupAckBeforeTerminal = report.helperTerminals === 0;
          report.setupAckRegistered = guard.snapshot().admitted === 1;
          if (scenario === "helper-setup-ack-guard") {
            const request = transport === "http" ? http.request : https.request;
            try {
              request({
                hostname: "127.0.0.1",
                port,
                method: "POST",
                path: `/queries/v1/query-request?requestId=${event.sdkRequestId}`,
              });
            } catch {}
          }
          if (scenario === "helper-setup-ack-cancel")
            helper.receive({
              v: 1,
              generation: helper.generation,
              id: "2",
              op: "cancel",
              payload: { target: "1" },
            });
          if (scenario === "helper-setup-ack-eof") {
            // Same local retirement seam as stdin EOF, not a framed-process EOF test.
            helper.stop();
            report.helperTerminal = "localEOF";
            resolveTerminal();
          }
          if (scenario === "helper-setup-close-after-ack") {
            report.helperExplicitCloseRequested = true;
            helper.receive({
              v: 1,
              generation: helper.generation,
              id: "2",
              op: "close",
              payload: {},
            });
          }
        }
        if (event.type === "terminal") {
          report.helperTerminals++;
          report.helperTerminal = event.ok ? "success" : event.error.code;
          if (event.ok) {
            const r = event.result;
            report.privateSetupResultExact =
              r.runId === setupRunId &&
              r.step === setupStep &&
              r.key === setupEntry.key &&
              r.phase === setupEntry.phase &&
              r.mutation === setupEntry.mutation &&
              r.rowsSeen === 1 &&
              r.rowsRetained === 0 &&
              r.truncated === false &&
              r.sdkRequestId === firstId &&
              JSON.stringify(r.columns) ===
                JSON.stringify(setupColumns.map((name) => ({ name, type: "text" }))) &&
              JSON.stringify(r.values) === JSON.stringify([setupValues]);
          }
          resolveTerminal();
        }
      },
      () => report.helperRetired++,
      { submitGuard: guard, clientConfigFile: join(root, "disabled-config.json") },
    );
    // Scope: real SDK/guard/Helper execution and evidence, with browser/bootstrap
    // and prior ledger cursors explicitly seeded. Unit tests execute all 28 SQL.
    helper.connection = connection;
    helper.state = "connected";
    helper.generation = "11111111-1111-4111-8111-111111111111";
    helper.identityPin = { user: "SERVER_USER", session: "OPAQUE-SESSION" };
    helper.setupBootstrap = 2;
    helper.setupRun = { runId: setupRunId, next: setupStep, ledger: setupLedger };
    helper.receive({
      v: 1,
      generation: helper.generation,
      id: "1",
      op: "setup",
      payload: {
        runId: setupRunId,
        step: setupStep,
        expected: { user: "SERVER_USER", session: "OPAQUE-SESSION" },
      },
    });
    if (scenario === "helper-setup-close-before-ack") {
      await bounded(firstQuery);
      report.helperExplicitCloseRequested = true;
      helper.receive({ v: 1, generation: helper.generation, id: "2", op: "close", payload: {} });
    }
    await bounded(terminal);
    if (["helper-setup-lost-ack", "helper-setup-ack-guard"].includes(scenario))
      await bounded(violated);
    await pause(350);
    assert.equal(report.queryPosts, 1);
    assert(report.intendedSQLOnly && report.scalarBindsExact);
    const expected =
      {
        "helper-setup-lost-ack": "outcomeUnknown",
        "helper-setup-rejected": "database",
        "helper-setup-ack-drain": "connectionClosed",
        "helper-setup-ack-guard": "connectionClosed",
        "helper-setup-ack-cancel": "connectionClosed",
        "helper-setup-ack-eof": "localEOF",
        "helper-setup-close-before-ack": "outcomeUnknown",
        "helper-setup-close-after-ack": "connectionClosed",
      }[scenario] ?? "success";
    assert.equal(report.helperTerminal, expected);
    assert.equal(
      report.helperSetupAcknowledged,
      setupEntry.mutation &&
        ![
          "helper-setup-lost-ack",
          "helper-setup-rejected",
          "helper-setup-close-before-ack",
        ].includes(scenario),
    );
    if (expected === "success") assert.equal(report.privateSetupResultExact, true);
    assert.equal(report.helperTerminals, scenario === "helper-setup-ack-eof" ? 0 : 1);
    assert.equal(report.rows, 0);
    assert.equal(report.publicColumns, 0);
    if (report.helperSetupAcknowledged)
      assert(report.setupAckBeforeTerminal && report.setupAckRegistered);
    helper.stop();
    assert.equal(report.helperRetired, 1);
    report.outcome = `helper-${expected}`;
    report.guardFailed = guard.snapshot().failed;
    report.guardAdmitted = guard.snapshot().admitted;
    report.firstQueryProxyConnects = firstQueryProxyConnects;
    await finish();
  }
  if (batchScenario) {
    phase = "preconnected-batch-helper";
    report.helperFixturePreconnected = true;
    report.externalBrowserConnectSkipped = true;
    report.helperTerminals = 0;
    report.helperRetired = 0;
    const { isPutCommand, isGetCommand } = require("snowflake-sdk/dist/lib/util.js");
    const events = [],
      terminals = new Map();
    let sequence = 0;
    helper = new Helper(
      sdk,
      (event) => {
        events.push(event);
        if (event.type === "terminal") {
          report.helperTerminals++;
          terminals.get(event.id)?.(event);
        }
      },
      () => {
        report.helperRetired++;
      },
      {
        submitGuard: guard,
        clientConfigFile: join(root, "disabled-config.json"),
        isFileTransfer: (sql) => isPutCommand(sql) || isGetCommand(sql),
      },
    );
    // Only browser authentication/bootstrap is seeded. Every batch preflight,
    // SDK execute/getter, streamed FIXED value, guarded HTTP submission and
    // disposition observation below crosses the real pinned SDK connection.
    helper.connection = connection;
    helper.state = "connected";
    helper.generation = "11111111-1111-4111-8111-111111111111";
    helper.identityPin = { user: batchExpected.user, session: batchExpected.session };
    const submit = (op, payload) => {
      const id = String(++sequence);
      const terminal = new Promise((resolve) => terminals.set(id, resolve));
      helper.receive({ v: 1, generation: helper.generation, id, op, payload });
      return bounded(terminal);
    };
    phase = "batch-original-terminal";
    const original = await submit("mutationBatch", {
      mode: batchManual ? "manual" : "automatic",
      owner: null,
      rollbackPrior: false,
      expected: batchExpected,
      units: batchManual ? batchUnits.slice(0, 2) : batchUnits,
    });
    report.batchTerminal = original.result?.outcome ?? original.error?.code;
    report.batchFailure = original.result?.failure ?? null;
    assert.equal(original.ok, true);
    const originalSnapshot = JSON.stringify(original);
    let final = original;
    if (batchManual) {
      assert.equal(original.result.outcome, "pending");
      assert.equal(original.result.transactionId, batchTransactionId);
      assert.equal(original.result.priorBatches, 0);
      assert.match(original.result.owner, /^[0-9a-f-]{36}$/);
      phase = "batch-finish-terminal";
      final = await submit("mutationFinish", {
        owner: original.result.owner,
        action: scenario.endsWith("-commit") ? "commit" : "rollback",
        expected: batchExpected,
      });
      report.batchTerminal = final.result?.outcome ?? final.error?.code;
      report.batchFailure = final.result?.failure ?? null;
      assert.equal(final.ok, true);
      assert.equal(final.result.owner, original.result.owner);
      assert.equal(final.result.priorBatches, 1);
      assert.equal(JSON.stringify(original), originalSnapshot);
      report.batchPendingImmutable = true;
      report.batchOwnerStable = true;
    }
    if (scenario === "helper-batch-lost-ack") {
      phase = "batch-lost-ack-guard";
      await bounded(violated);
    }
    // The loss fixture records receipt of the second unit then drops its ACK.
    // Wait through the guard's retry denial before checking the fixed prefix.
    await pause(350);
    phase = "batch-request-prefix";
    assert.equal(batchPosts.length, batchPlan.length);
    assert.equal(report.queryPosts, batchPosts.length);
    const dispatches = events.filter(
      (event) => event.type === "batchStep" && event.phase === "dispatching",
    );
    assert.equal(dispatches.length, batchPosts.length);
    assert.equal(new Set(batchPosts.map((post) => post.id)).size, batchPosts.length);
    const countEvents = [],
      seenIdentities = new Set();
    report.batchCounts = countEvents;
    let acknowledged = 0;
    for (const [index, dispatch] of dispatches.entries()) {
      phase = `batch-evidence-${index + 1}-${dispatch.role}`;
      const post = batchPosts[index];
      assert.equal(dispatch.role, post.role);
      assert.equal(dispatch.unit, post.unit);
      if (index > 0) {
        const previous = dispatches[index - 1];
        const previousDrain = events.find(
          (event) =>
            event.type === "batchStep" &&
            event.phase === "drained" &&
            event.id === previous.id &&
            event.step === previous.step,
        );
        assert(previousDrain);
        assert(events.indexOf(previousDrain) < events.indexOf(dispatch));
      }
      const same = (event) => event.id === dispatch.id && event.step === dispatch.step;
      const identities = events.filter((event) => event.type === "batchStatement" && same(event));
      assert(identities.length >= 1);
      assert(identities.length <= 2);
      assert(identities.every((event) => event.sdkRequestId === post.id));
      assert(
        identities.every((event) => event.queryId === undefined || event.queryId === post.queryId),
      );
      assert.equal(
        new Set(identities.map((event) => JSON.stringify([event.sdkRequestId, event.queryId])))
          .size,
        identities.length,
      );
      assert(!seenIdentities.has(post.id));
      seenIdentities.add(post.id);
      const ack = events.find(
        (event) => event.type === "batchStep" && event.phase === "acknowledged" && same(event),
      );
      const drained = events.find(
        (event) => event.type === "batchStep" && event.phase === "drained" && same(event),
      );
      if (scenario === "helper-batch-lost-ack" && index === dispatches.length - 1) {
        assert.equal(ack, undefined);
        assert.equal(drained, undefined);
        continue;
      }
      acknowledged++;
      assert(ack && drained);
      assert(events.indexOf(dispatch) < events.indexOf(ack));
      assert(events.indexOf(ack) < events.indexOf(drained));
      assert.equal(identities.at(-1).queryId, post.queryId);
      assert(events.indexOf(ack) < events.indexOf(identities.at(-1)));
      assert(events.indexOf(identities.at(-1)) < events.indexOf(drained));
      if (post.role === "unit") {
        const count = events.find((event) => event.type === "batchCount" && same(event));
        assert(count);
        assert.equal(count.unit, post.unit);
        assert(events.indexOf(ack) < events.indexOf(count));
        assert(events.indexOf(count) < events.indexOf(drained));
        assert.equal(typeof count.affectedRows, "string");
        assert.equal(
          count.affectedRows,
          scenario === "helper-batch-cardinality" && post.unit === 1 ? batchHugeCount : "1",
        );
        assert.equal(
          count.multiJoinedRows,
          batchUnits[post.unit].kind === "update" && !batchManual ? "0" : null,
        );
        countEvents.push({
          affectedRows: count.affectedRows,
          multiJoinedRows: count.multiJoinedRows,
        });
      }
    }
    assert(
      !events.some((event) =>
        ["statement", "phase", "columns", "rows", "batchCountUnavailable"].includes(event.type),
      ),
    );
    const snapshot = guard.snapshot();
    phase = "batch-disposition-and-admission";
    assert.equal(snapshot.registered, batchPosts.length);
    assert.equal(snapshot.admitted, batchPosts.length);
    const expectedOutcome =
      scenario === "helper-batch-lost-ack"
        ? "unknown"
        : scenario === "helper-batch-cardinality" || scenario.endsWith("-rollback")
          ? "rolledBack"
          : "committed";
    assert.equal(final.result.outcome, expectedOutcome);
    assert.equal(final.result.transactionId, batchTransactionId);
    assert.equal(
      final.result.failure,
      scenario === "helper-batch-cardinality"
        ? "cardinality"
        : scenario === "helper-batch-lost-ack"
          ? "transport"
          : null,
    );
    assert.equal(report.effects, scenario === "helper-batch-automatic" ? 3 : 2);
    if (scenario === "helper-batch-lost-ack") {
      assert.equal(report.violations, 1);
      assert.equal(report.violationCategory, "duplicate");
      assert.equal(snapshot.failed, true);
      assert.equal(helper.retired, true);
      assert(
        !batchPosts.some((post) =>
          ["verify", "commit", "rollback", "idle", "disposition"].includes(post.role),
        ),
      );
    } else {
      assert.equal(report.violations, 0);
      assert.equal(snapshot.failed, false);
      assert.equal(batchTransaction, null);
      assert.equal(batchDisposition, expectedOutcome === "committed" ? "committed" : "rolled back");
      assert.equal(dispatches.at(-1).role, "disposition");
    }
    report.batchSteps = dispatches.length;
    report.batchAcknowledgements = acknowledged;
    report.batchCounts = countEvents;
    report.batchPostIdentitiesExact = true;
    report.batchPostIdentitiesDistinct = true;
    report.batchPrefixExact = true;
    report.batchTerminal = final.result.outcome;
    report.batchFailure = final.result.failure;
    report.batchPriorBatches = final.result.priorBatches;
    report.guardAdmitted = snapshot.admitted;
    report.guardRegistered = snapshot.registered;
    report.guardFailed = snapshot.failed;
    report.outcome = `helper-batch-${final.result.outcome}`;
    if (!helper.retired) helper.stop();
    await finish();
  }
  if (scenario.startsWith("helper-")) {
    phase = "preconnected-helper";
    report.helperFixturePreconnected = true;
    report.externalBrowserConnectSkipped = true;
    report.helperTerminals = 0;
    report.helperRetired = 0;
    report.helperAcknowledged = false;
    report.publicColumns = 0;
    if (dmlScenario) report.helperDmlAcknowledged = false;
    if (ddlScenario) report.helperDdlAcknowledged = false;
    if (switchScenario) report.helperUseAcknowledged = false;
    let resolveTerminal, resolveAcknowledged;
    const terminal = new Promise((resolve) => {
      resolveTerminal = resolve;
    });
    const acknowledged = new Promise((resolve) => {
      resolveAcknowledged = resolve;
    });
    const nativeStale = (id) => {
      const request = transport === "http" ? http.request : https.request;
      try {
        request({
          hostname: "127.0.0.1",
          port,
          method: "POST",
          path: `/queries/v1/query-request?requestId=${id}`,
        });
      } catch {}
    };
    helper = new Helper(
      sdk,
      (event) => {
        if (
          event.id === "2" &&
          ["helper-dml-ack-cancel", "helper-ddl-ack-cancel"].includes(scenario) &&
          event.type === "terminal"
        )
          report.helperCancelRequested = event.ok && event.result.requested === true;
        if (event.id !== "1") return;
        if (event.type === "phase" && event.phase === "acknowledged") {
          report.helperAcknowledged = true;
          resolveAcknowledged();
        }
        if (event.type === "useAcknowledged") report.helperUseAcknowledged = true;
        if (event.type === "dmlAcknowledged") {
          report.helperDmlAcknowledged = event.kind === dmlKind && event.sdkRequestId === firstId;
          report.dmlAckBeforeTerminal = report.helperTerminals === 0;
          report.dmlAckRegistered = guard.snapshot().admitted === 1;
          if (scenario === "helper-dml-ack-guard") nativeStale(event.sdkRequestId);
          if (scenario === "helper-dml-ack-cancel")
            helper.receive({
              v: 1,
              generation: helper.generation,
              id: "2",
              op: "cancel",
              payload: { target: "1" },
            });
          if (scenario === "helper-dml-ack-eof") {
            // Same retirement entry point as main.mjs stdin end; this is a
            // helper seam, not a subprocess/protocol-EOF qualification.
            helper.stop();
            report.helperTerminal = "localEOF";
            report.outcome = "helper-localEOF";
            resolveTerminal();
          }
        }
        if (event.type === "ddlAcknowledged") {
          report.helperDdlAcknowledged = event.kind === ddlKind && event.sdkRequestId === firstId;
          report.ddlAckBeforeTerminal = report.helperTerminals === 0;
          report.ddlAckRegistered = guard.snapshot().admitted === 1;
          if (scenario === "helper-ddl-close-after-ack") {
            report.helperExplicitCloseRequested = true;
            helper.receive({
              v: 1,
              generation: helper.generation,
              id: "2",
              op: "close",
              payload: {},
            });
          }
          if (scenario === "helper-ddl-ack-guard") nativeStale(event.sdkRequestId);
          if (scenario === "helper-ddl-ack-cancel")
            helper.receive({
              v: 1,
              generation: helper.generation,
              id: "2",
              op: "cancel",
              payload: { target: "1" },
            });
          if (scenario === "helper-ddl-ack-eof") {
            helper.stop();
            report.helperTerminal = "localEOF";
            report.outcome = "helper-localEOF";
            resolveTerminal();
          }
        }
        if (event.type === "columns") report.publicColumns++;
        if (event.type === "boundaryAcknowledged") {
          report.helperBoundaryAcknowledged =
            event.action === transactionAction && event.sdkRequestId === firstId;
          if (scenario === "helper-transaction-ack-guard") nativeStale(event.sdkRequestId);
        }
        if (event.type === "rows") report.rows += event.rows.length;
        if (
          event.type === "statement" &&
          event.queryId &&
          ["helper-database", "helper-dml-database", "helper-ddl-database"].includes(scenario) &&
          !report.helperReentrantFailure
        ) {
          // Reenter after the actual SDK supplied definitive SQLSTATE, before the
          // pump publishes its terminal. No SDK/transport method is replaced.
          report.helperReentrantFailure = true;
          nativeStale(event.sdkRequestId);
        }
        if (event.type === "terminal") {
          report.helperTerminals++;
          report.helperTerminal = event.ok ? "success" : event.error.code;
          report.outcome = `helper-${report.helperTerminal}`;
          if (switchScenario && event.ok) {
            report.switchSchemaMatches = event.result.schema === SWITCH_SCHEMA;
            report.switchRoleNull = event.result.role === null && event.result.warehouse === null;
          }
          if (dmlScenario && event.ok) {
            const r = event.result;
            report.privateDmlResultExact =
              r.kind === dmlKind &&
              r.rowsSeen === 1 &&
              r.rowsRetained === 0 &&
              r.truncated === false &&
              r.sdkRequestId === firstId &&
              r.queryId === "11111111-1111-4111-8111-111111111111" &&
              JSON.stringify(r.columns) ===
                JSON.stringify(
                  [
                    dmlColumn,
                    ...(dmlKind === "update" ? ["number of multi-joined rows updated"] : []),
                  ].map((name) => ({ name, type: "fixed" })),
                ) &&
              JSON.stringify(r.values) ===
                JSON.stringify([["9007199254740993", ...(dmlKind === "update" ? ["7"] : [])]]);
          }
          if (ddlScenario && event.ok) {
            const r = event.result;
            report.privateDdlResultExact =
              r.kind === ddlKind &&
              r.rowsSeen === 1 &&
              r.rowsRetained === 0 &&
              r.truncated === false &&
              r.sdkRequestId === firstId &&
              r.queryId === "11111111-1111-4111-8111-111111111111" &&
              JSON.stringify(r.columns) === JSON.stringify([{ name: "status", type: "text" }]) &&
              JSON.stringify(r.values) === JSON.stringify([["Synthetic DDL acknowledged"]]);
          }
          resolveTerminal();
        }
      },
      () => {
        report.helperRetired++;
      },
      { submitGuard: guard, clientConfigFile: join(root, "disabled-config.json") },
    );
    // Explicit test seam: only browser authentication/profile connect is skipped.
    // This is the actual local SDK connection, with production query validation,
    // ticket ownership, execute/consume, terminal mapping and retirement intact.
    helper.connection = connection;
    helper.state = "connected";
    helper.generation = "11111111-1111-4111-8111-111111111111";
    if (switchScenario) helper.expectedProfile = { database: "SYNTHETIC" };
    if (transactionScenario || dmlScenario || ddlScenario)
      helper.identityPin = { user: "SERVER_USER", session: "OPAQUE-SESSION" };
    helper.receive({
      v: 1,
      generation: helper.generation,
      id: "1",
      op: dmlScenario
        ? "dml"
        : ddlScenario
          ? "ddl"
          : transactionScenario
            ? "transaction"
            : switchScenario
              ? "switchSchema"
              : "query",
      payload: dmlScenario
        ? {
            kind: dmlKind,
            sql: dmlSQL,
            binds,
            expected: { user: "SERVER_USER", session: "OPAQUE-SESSION" },
          }
        : ddlScenario
          ? {
              kind: ddlKind,
              sql: ddlSQL,
              expected: { user: "SERVER_USER", session: "OPAQUE-SESSION" },
            }
          : transactionScenario
            ? {
                action: transactionAction,
                expected: { user: "SERVER_USER", session: "OPAQUE-SESSION" },
              }
            : switchScenario
              ? {
                  database: "SYNTHETIC",
                  schema: SWITCH_SCHEMA,
                  expected: {
                    user: "SERVER_USER",
                    role: null,
                    warehouse: null,
                    session: "OPAQUE-SESSION",
                  },
                }
              : {
                  sql: SQL,
                  binds,
                  classification: "read",
                  credits: scenario === "helper-ack-drain" ? 0 : 1,
                  maxRows: 4,
                  maxBytes: 4096,
                },
    });
    if (scenario === "helper-ddl-close-before-ack") {
      await bounded(firstQuery);
      report.helperExplicitCloseRequested = true;
      helper.receive({ v: 1, generation: helper.generation, id: "2", op: "close", payload: {} });
    }
    if (scenario === "helper-ack-drain") {
      await bounded(acknowledged);
      assert.equal(report.helperTerminals, 0);
      nativeStale(firstId);
    }
    await bounded(terminal);
    if (
      dmlScenario
        ? ["helper-dml-lost-ack", "helper-dml-ack-guard", "helper-dml-database"].includes(scenario)
        : ddlScenario
          ? ["helper-ddl-lost-ack", "helper-ddl-ack-guard", "helper-ddl-database"].includes(
              scenario,
            )
          : transactionScenario
            ? ["helper-transaction-lost-ack", "helper-transaction-ack-guard"].includes(scenario)
            : scenario !== "helper-success" && (!switchScenario || scenario.includes("lost-"))
    )
      await bounded(violated);
    await pause(350);
    assert.equal(
      report.queryPosts,
      switchScenario && scenario !== "helper-switch-lost-use" ? 2 : 1,
    );
    assert(report.intendedSQLOnly && report.scalarBindsExact);
    assert.equal(
      report.helperTerminals,
      ["helper-dml-ack-eof", "helper-ddl-ack-eof"].includes(scenario) ? 0 : 1,
    );
    const expected = {
      "helper-success": "success",
      "helper-lost-ack": "outcomeUnknown",
      "helper-ack-drain": "connectionClosed",
      "helper-database": "database",
      "helper-transaction-begin": "success",
      "helper-transaction-commit": "success",
      "helper-transaction-rollback": "success",
      "helper-transaction-lost-ack": "outcomeUnknown",
      "helper-transaction-ack-guard": "connectionClosed",
      "helper-transaction-database": "database",
      "helper-dml-insert": "success",
      "helper-dml-update": "success",
      "helper-dml-delete": "success",
      "helper-dml-lost-ack": "outcomeUnknown",
      "helper-dml-ack-guard": "connectionClosed",
      "helper-dml-database": "database",
      "helper-dml-ack-drain": "connectionClosed",
      "helper-dml-ack-cancel": "connectionClosed",
      "helper-dml-ack-eof": "localEOF",
      "helper-ddl-create": "success",
      "helper-ddl-add-column": "success",
      "helper-ddl-drop": "success",
      "helper-ddl-lost-ack": "outcomeUnknown",
      "helper-ddl-ack-guard": "connectionClosed",
      "helper-ddl-database": "database",
      "helper-ddl-ack-drain": "connectionClosed",
      "helper-ddl-ack-cancel": "connectionClosed",
      "helper-ddl-ack-eof": "localEOF",
      "helper-ddl-close-before-ack": "outcomeUnknown",
      "helper-ddl-close-after-ack": "connectionClosed",
      "helper-switch-success": "success",
      "helper-switch-lost-use": "outcomeUnknown",
      "helper-switch-readback-denied": "connectionClosed",
      "helper-switch-lost-readback": "connectionClosed",
    }[scenario];
    assert.equal(report.helperTerminal, expected);
    assert.equal(
      report.helperRetired,
      expected === "success" || scenario === "helper-transaction-database" ? 0 : 1,
    );
    if (transactionScenario)
      assert.equal(
        report.helperBoundaryAcknowledged === true,
        !["helper-transaction-lost-ack", "helper-transaction-database"].includes(scenario),
      );
    if (dmlScenario) {
      assert.equal(
        report.helperDmlAcknowledged,
        !["helper-dml-lost-ack", "helper-dml-database"].includes(scenario),
      );
      assert.equal(report.rows, 0);
      assert.equal(report.publicColumns, 0);
      if (report.helperDmlAcknowledged) {
        assert.equal(report.dmlAckBeforeTerminal, true);
        assert.equal(report.dmlAckRegistered, true);
      }
      if (expected === "success") assert.equal(report.privateDmlResultExact, true);
      if (scenario === "helper-dml-ack-drain") assert.equal(report.resultChunkRequests, 1);
      if (scenario === "helper-dml-ack-cancel") assert.equal(report.helperCancelRequested, true);
    }
    if (ddlScenario) {
      assert.equal(
        report.helperDdlAcknowledged,
        !["helper-ddl-lost-ack", "helper-ddl-database", "helper-ddl-close-before-ack"].includes(
          scenario,
        ),
      );
      assert.equal(report.rows, 0);
      assert.equal(report.publicColumns, 0);
      if (report.helperDdlAcknowledged) {
        assert.equal(report.ddlAckBeforeTerminal, true);
        assert.equal(report.ddlAckRegistered, true);
      }
      if (expected === "success") assert.equal(report.privateDdlResultExact, true);
      if (scenario === "helper-ddl-ack-drain") assert.equal(report.resultChunkRequests, 1);
      if (scenario === "helper-ddl-ack-cancel") assert.equal(report.helperCancelRequested, true);
      if (scenario.startsWith("helper-ddl-close-"))
        assert.equal(report.helperExplicitCloseRequested, true);
    }
    if (switchScenario) {
      assert.equal(report.helperUseAcknowledged, scenario !== "helper-switch-lost-use");
      assert.equal(report.helperAcknowledged, expected === "success");
      assert.equal(report.switchIdsDistinct, true);
      assert.equal(report.effects, 1);
      if (expected === "success") assert(report.switchSchemaMatches && report.switchRoleNull);
    }
    if (expected === "success" || scenario === "helper-transaction-database") {
      assert.equal(
        report.rows,
        switchScenario || transactionScenario || dmlScenario || ddlScenario ? 0 : 1,
      );
      helper.stop();
    }
    report.guardFailed = guard.snapshot().failed;
    report.guardAdmitted = guard.snapshot().admitted;
    report.firstQueryProxyConnects = firstQueryProxyConnects;
    await finish();
  }
  async function execute() {
    currentAcknowledged = false;
    currentCompleted = false;
    report.outcome = "pending";
    owner ??= guard?.beginOperation();
    ticket = guard?.beginStatement(owner);
    let resolveComplete;
    const complete = new Promise((resolve) => {
      resolveComplete = resolve;
    });
    statement = connection.execute({
      sqlText: SQL,
      binds,
      streamResult: true,
      complete(error, completed) {
        report.queryCallbacks++;
        if (error) {
          resolveComplete(false);
          return;
        }
        currentAcknowledged = true;
        const stream = completed.streamRows();
        stream.on("data", (row) => {
          if (row.RESULT !== "synthetic-success") {
            phase = "row-contract";
            void finish(true);
          }
          report.rows++;
        });
        stream.once("error", () => {
          phase = "row-stream";
          void finish(true);
        });
        stream.once("end", () => {
          currentCompleted = true;
          report.knownSuccesses++;
          report.outcome = "success";
          resolveComplete(true);
        });
      },
    });
    guard?.registerRequestId(ticket, statement.getRequestId());
    return { complete };
  }
  const cancel = () =>
    bounded(
      new Promise((resolve, reject) =>
        statement.cancel((error) => {
          if (error) reject(error);
          else {
            report.cancelAcknowledged = true;
            resolve();
          }
        }),
      ),
    );
  phase = "execute";
  const { complete } = await execute();
  if (scenario === "cancel-before") {
    // Model owner retirement before deferred native dispatch, then ask the real
    // SDK to cancel. Cancel acknowledgement alone does NOT close the allowance
    // or establish the original outcome; the explicit ownership close does.
    guard.endStatement(ticket);
    guard.endOperation(owner);
    owner = undefined;
    await Promise.all([cancel(), bounded(violated)]);
  } else if (scenario === "cancel-during") {
    await bounded(firstQuery);
    await cancel();
    await bounded(violated);
  } else if (["success", "cancel-after", "stale-owner", "mutation-no-guard"].includes(scenario)) {
    assert.equal(await bounded(complete), true);
    if (scenario === "cancel-after") {
      await cancel();
      // A known terminal stays known even when a stale submission is denied.
      guard.endStatement(ticket);
      guard.endOperation(owner);
      owner = undefined;
      const request = transport === "http" ? http.request : https.request;
      try {
        request({
          hostname: "127.0.0.1",
          port,
          method: "POST",
          path: `/queries/v1/query-request?requestId=${firstId}`,
        });
      } catch {}
      assert.equal(report.outcome, "success");
    } else if (scenario === "stale-owner") {
      guard.endStatement(ticket);
      guard.endOperation(owner);
      owner = undefined;
      const next = await execute();
      // Synthetic stale native work uses actual previous SDK ID while a new
      // actual SDK execution owns the lane. This is not an SDK retry claim.
      const request = transport === "http" ? http.request : https.request;
      try {
        request({
          hostname: "127.0.0.1",
          port,
          method: "POST",
          path: `/queries/v1/query-request?requestId=${firstId}`,
        });
      } catch {}
      await bounded(violated);
      void next;
    }
  } else await bounded(violated);
  phase = "quarantine";
  // Leave the real SDK's timers running briefly after the irreversible latch.
  // They cannot open a second native query request or another query tunnel.
  if (report.violations) await pause(350);
  report.firstQueryProxyConnects = firstQueryProxyConnects ?? 0;
  report.guardFailed = guard?.snapshot().failed ?? false;
  report.guardAdmitted = guard?.snapshot().admitted ?? report.queryPosts;
  if (scenario !== "cancel-before") {
    assert(report.intendedSQLOnly && report.scalarBindsExact);
    assert.equal(report.queryPosts, scenario === "mutation-no-guard" ? 2 : 1);
  } else assert.equal(report.queryPosts, 0);
  assert.equal(report.destinationRequests, 0);
  assert.equal(report.destinationConnections, 0);
  assert.equal(report.networkDenied, 0);
  assert.equal(report.dnsDenied, 0);
  phase = "destroy";
  await bounded(new Promise((resolve) => connection.destroy(() => resolve())), 1500).catch(
    () => {},
  );
  await finish();
} catch {
  await finish(true);
}
