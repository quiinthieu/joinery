import test from "node:test";
import assert from "node:assert/strict";
import { Readable } from "node:stream";
import fs from "node:fs";
import net from "node:net";
import tls from "node:tls";
import dns from "node:dns";
import dgram from "node:dgram";
import http from "node:http";
import https from "node:https";
import childProcess from "node:child_process";
import { syncBuiltinESMExports } from "node:module";
import { compileManifest, WORKFLOW, SOURCE_PATHS } from "./snowflake-qualification-manifest.mjs";
import {
  createQualificationProtocol,
  PROTOCOL_LIMITS,
} from "./snowflake-qualification-protocol.mjs";
import { Decoder, encode } from "../snowflake-helper/src/framing.mjs";
import { Helper, LIMITS } from "../snowflake-helper/src/helper.mjs";

const generation = "12345678-1234-4234-8234-123456789abc";
const clone = (value) => JSON.parse(JSON.stringify(value));
const input = () => ({
  workflow: WORKFLOW,
  route: "helper-direct",
  runId: "A1".repeat(16),
  window: { notBefore: "2026-11-10T10:00:00.000Z", expiresAt: "2026-11-10T10:10:00.000Z" },
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
});
const hello = () => ({
  v: 1,
  type: "hello",
  protocol: "joinery-snowflake",
  sdkVersion: "3.4.1",
  limits: { ...LIMITS },
});
const sdkId = (id) => `00000000-0000-4000-8000-${BigInt(id).toString(16).padStart(12, "0")}`;
const ids = (request) => ({
  sdkRequestId: sdkId(request.id),
  queryId: sdkId(BigInt(request.id) + 10000n),
});
const frame = (request, type, rest = {}) => ({ v: 1, generation, id: request.id, type, ...rest });
const rejects = (action, code) =>
  assert.throws(action, (error) => {
    assert.equal(error.message, error.safeCode);
    assert.match(error.safeCode, /^[a-zA-Z]+$/);
    return error.safeCode === code;
  });

// Exercise the real framing decoder, including split headers/bodies. No pipe,
// process, filesystem, socket, authentication or Snowflake SDK is used.
function feed(protocol, value, padding = 0) {
  const encoded = encode(value);
  const body = Buffer.concat([encoded.subarray(4), Buffer.alloc(padding, 32)]);
  const header = Buffer.alloc(4);
  header.writeUInt32BE(body.length);
  const bytes = Buffer.concat([header, body]);
  const decoder = new Decoder();
  const chunks = [bytes.subarray(0, 1), bytes.subarray(1, 5), bytes.subarray(5)];
  for (const chunk of chunks)
    for (const decoded of decoder.push(chunk))
      protocol.receive(decoded, { bodyBytes: body.length });
  decoder.end();
}
function setup() {
  const currentInput = input(),
    manifest = compileManifest(currentInput);
  const protocol = createQualificationProtocol({ manifest, currentInput, generation });
  feed(protocol, hello());
  return { protocol, manifest, currentInput };
}
const names = [
  "PRINCIPAL",
  "ROLE_NAME",
  "DATABASE_NAME",
  "SCHEMA_NAME",
  "WAREHOUSE_NAME",
  "SESSION_ID",
  "SEARCH_PATH",
  "SECONDARY_ROLES",
];
function answer(template, overrides = {}) {
  let columns, rows;
  switch (template) {
    case "context":
      columns = names.map((name) => ({ name, type: "TEXT" }));
      rows = [
        [
          "SERVER_NAME",
          "ROLE",
          "DB",
          "PUBLIC",
          "WH",
          "opaque-session-A7/x",
          '["DB.PUBLIC"]',
          '{"roles":"","value":"ALL"}',
        ],
      ];
      break;
    case "schemas":
      columns = ["CATALOG_NAME", "SCHEMA_NAME"].map((name) => ({ name, type: "TEXT" }));
      rows = [
        ["DB", "OTHER"],
        ["DB", "PUBLIC"],
      ];
      break;
    case "version":
      columns = [{ name: "CURRENT_VERSION()", type: "TEXT" }];
      rows = [["synthetic-version"]];
      break;
    case "transaction":
      columns = [{ name: "TRANSACTION_ID", type: "TEXT" }];
      rows = [[null]];
      break;
    case "scalar":
      columns = ["EXACT_INTEGER", "EXACT_BOOLEAN", "EXACT_TEXT", "EXACT_NULL"].map(
        (name, index) => ({ name, type: ["FIXED", "BOOLEAN", "TEXT", "TEXT"][index] }),
      );
      rows = [["9007199254740993", "TRUE", "joinery-qualification", null]];
      break;
    default:
      columns = ["key", "value", "default", "level"].map((name) => ({ name, type: "TEXT" }));
      rows = [
        [
          {
            quotedIdentifierPolicy: "QUOTED_IDENTIFIERS_IGNORE_CASE",
            autocommit: "AUTOCOMMIT",
            abortOnError: "TRANSACTION_ABORT_ON_ERROR",
          }[template],
          template === "autocommit" ? "true" : "false",
          "uninterpreted",
          "SESSION",
        ],
      ];
  }
  return { ...{ columns, rows }, ...overrides };
}
function begin(protocol, request, { enrich = false } = {}) {
  feed(protocol, frame(request, "accepted"));
  if (["query", "context"].includes(request.op)) {
    feed(protocol, frame(request, "phase", { phase: "dispatching" }));
    feed(
      protocol,
      frame(
        request,
        "statement",
        enrich ? { sdkRequestId: ids(request).sdkRequestId } : ids(request),
      ),
    );
    if (enrich) feed(protocol, frame(request, "statement", ids(request)));
    feed(protocol, frame(request, "phase", { phase: "acknowledged", ...ids(request) }));
  }
}
function complete(protocol, request, manifest, { data, enrich = false } = {}) {
  begin(protocol, request, { enrich });
  let result;
  if (request.op === "connect") result = { connected: true };
  else if (request.op === "close") result = { closed: true, logoutAcknowledged: true };
  else if (request.op === "context")
    result = {
      user: "SERVER_NAME",
      role: "ROLE",
      database: "DB",
      schema: "PUBLIC",
      warehouse: "WH",
      session: "opaque-session-A7/x",
      ...ids(request),
      ...data,
    };
  else {
    const template = manifest.payload.sqlLedger.find(
      (entry) => entry.sql === request.payload.sql,
    ).templateId;
    data ??= answer(template);
    feed(protocol, frame(request, "columns", { columns: data.columns }));
    if (data.rows.length) feed(protocol, frame(request, "rows", { rows: data.rows }));
    result = {
      rowsSeen: data.rows.length,
      rowsRetained: data.rows.length,
      truncated: false,
      ...ids(request),
    };
  }
  feed(
    protocol,
    frame(request, "terminal", {
      ok: true,
      ...(["query", "context"].includes(request.op) ? ids(request) : {}),
      result,
    }),
  );
}
function until(protocol, manifest, id) {
  while (protocol.snapshot().nextCommandId !== id) {
    const request = protocol.reserveNext();
    assert(request);
    complete(protocol, request, manifest);
  }
  return protocol.reserveNext();
}
function schemaStart() {
  const state = setup();
  const request = until(state.protocol, state.manifest, "options.schemas");
  begin(state.protocol, request);
  feed(state.protocol, frame(request, "columns", { columns: answer("schemas").columns }));
  return { ...state, request };
}
const creditSuccess = (protocol, credit) => {
  feed(protocol, frame(credit, "accepted"));
  feed(protocol, frame(credit, "terminal", { ok: true, result: {} }));
};
const queryEnd = (protocol, request, count) =>
  feed(
    protocol,
    frame(request, "terminal", {
      ok: true,
      ...ids(request),
      result: { rowsSeen: count, rowsRetained: count, truncated: false, ...ids(request) },
    }),
  );

test("offline manifest binding and hello gate precede any immutable decimal-ID admission", () => {
  const currentInput = input(),
    manifest = compileManifest(currentInput);
  const protocol = createQualificationProtocol({ manifest, currentInput, generation });
  assert.equal(protocol.reserveNext(), null);
  feed(protocol, hello());
  const connect = protocol.reserveNext();
  assert.equal(connect.id, "1");
  assert.equal(connect.op, "connect");
  assert(Object.isFrozen(connect.payload));
  assert.equal(protocol.reserveNext(), null);
  complete(protocol, connect, manifest);
  const identity = protocol.reserveNext();
  assert.equal(identity.id, "2");
  assert.deepEqual(identity.payload, {});
  const modified = clone(manifest);
  modified.payload.commands[2].payload.sql = "DELETE FROM PRIVATE";
  rejects(
    () => createQualificationProtocol({ manifest: modified, currentInput, generation }),
    "manifestTampered",
  );
  rejects(
    () =>
      createQualificationProtocol({
        manifest,
        currentInput: { ...currentInput, route: "application" },
        generation,
      }),
    "unsupportedRoute",
  );
  rejects(
    () => createQualificationProtocol({ manifest, currentInput, generation: "not-a-uuid" }),
    "protocolGeneration",
  );
});

test("SQL reservation exposes explicit unknown evidence before any response or EOF", () => {
  const { protocol, manifest } = setup();
  const connect = protocol.reserveNext();
  assert.deepEqual(protocol.outcomes(), []);
  complete(protocol, connect, manifest);
  const context = protocol.reserveNext();
  assert.equal(context.op, "context");
  assert.deepEqual(protocol.outcomes(), [
    {
      commandId: "startup.identity",
      op: "context",
      requestStatus: "reserved",
      remoteOutcome: "unknown",
      validationStatus: "pending",
    },
  ]);
  // An executor losing its pipe here has explicit unknown evidence even though
  // no statement, phase, result or terminal was received. No EOF is invented.
  assert.equal(protocol.snapshot().ledgerComplete, false);
});

test("remote uncertainty, definite query rejection and context validation errors have separate evidence", () => {
  const messages = {
    outcomeUnknown: "Remote statement outcome is unknown",
    connectionClosed: "Snowflake helper connection closed",
    database: "Snowflake operation failed",
  };
  for (const op of ["query", "context"])
    for (const acknowledged of [false, true])
      for (const code of Object.keys(messages)) {
        const { protocol, manifest } = setup();
        const request = until(
          protocol,
          manifest,
          op === "query" ? "startup.version" : "startup.identity",
        );
        if (acknowledged) begin(protocol, request);
        else {
          feed(protocol, frame(request, "accepted"));
          feed(protocol, frame(request, "phase", { phase: "dispatching" }));
          feed(protocol, frame(request, "statement", ids(request)));
        }
        feed(
          protocol,
          frame(request, "terminal", {
            ok: false,
            ...ids(request),
            error: { code, message: messages[code] },
          }),
        );
        assert.deepEqual(protocol.outcomes().at(-1), {
          commandId: op === "query" ? "startup.version" : "startup.identity",
          op,
          requestStatus: "terminal",
          remoteOutcome: acknowledged
            ? "succeeded"
            : op === "query" && code === "database"
              ? "rejected"
              : "unknown",
          validationStatus: "failed",
          terminalErrorCode: code,
        });
        rejects(() => protocol.reserveNext(), "protocolStopped");
      }
});

test("query database rejection requires dispatch while queued closure remains uncertain", () => {
  for (const code of ["database", "connectionClosed", "cancelled"]) {
    const { protocol, manifest } = setup();
    const request = until(protocol, manifest, "startup.version");
    feed(protocol, frame(request, "accepted"));
    const receiveTerminal = () =>
      feed(
        protocol,
        frame(request, "terminal", {
          ok: false,
          error: {
            code,
            message: {
              database: "Snowflake operation failed",
              connectionClosed: "Snowflake helper connection closed",
              cancelled: "Snowflake operation cancelled",
            }[code],
          },
        }),
      );
    if (code === "database") rejects(receiveTerminal, "protocolOrder");
    else receiveTerminal();
    assert.deepEqual(protocol.outcomes().at(-1), {
      commandId: "startup.version",
      op: "query",
      requestStatus: "terminal",
      remoteOutcome: "unknown",
      validationStatus: "failed",
      terminalErrorCode: code,
    });
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("lost transcript before ACK remains unknown while success ACK survives absent terminal and semantic failure", () => {
  for (const acknowledged of [false, true]) {
    const { protocol, manifest } = setup();
    const request = until(protocol, manifest, "startup.version");
    if (acknowledged) begin(protocol, request);
    else {
      feed(protocol, frame(request, "accepted"));
      feed(protocol, frame(request, "phase", { phase: "dispatching" }));
      feed(protocol, frame(request, "statement", ids(request)));
    }
    assert.deepEqual(protocol.outcomes().at(-1), {
      commandId: "startup.version",
      op: "query",
      requestStatus: acknowledged ? "acknowledged" : "dispatching",
      remoteOutcome: acknowledged ? "succeeded" : "unknown",
      validationStatus: "pending",
    });
  }
  const { protocol, manifest } = setup();
  const request = until(protocol, manifest, "scalar");
  const data = answer("scalar");
  data.rows[0][0] = "9007199254740992";
  rejects(() => complete(protocol, request, manifest, { data }), "scalarValue");
  assert.deepEqual(protocol.outcomes().at(-1), {
    commandId: "scalar",
    op: "query",
    requestStatus: "terminal",
    remoteOutcome: "succeeded",
    validationStatus: "failed",
  });
  assert.equal(protocol.snapshot().nextCommandId, "scalar");
  assert.equal(protocol.snapshot().resultBudget.failed, false);
  // A second/corrected terminal cannot erase failed validation or retry SQL.
  rejects(() => queryEnd(protocol, request, 1), "protocolDuplicate");
  rejects(() => protocol.reserveNext(), "protocolStopped");
});

test("full24 ledger enforces raw SDK types, ID enrichment, opaque identity and immutable offline observations", () => {
  const { protocol, manifest } = setup();
  let count = 0;
  for (let request; (request = protocol.reserveNext());) {
    complete(protocol, request, manifest, { enrich: request.op === "query" });
    count++;
  }
  assert.equal(count, 26);
  assert.equal(protocol.snapshot().ledgerComplete, true);
  assert.equal(protocol.snapshot().executionStatus, "offlineOnly");
  assert.equal(protocol.snapshot().authorizationGranted, false);
  assert.equal(protocol.snapshot().runnerImplemented, false);
  assert.equal(protocol.snapshot().resultBudget.rows, 25);
  assert.equal(protocol.observations()["startup.identity"].principal, "SERVER_NAME");
  assert.notEqual(protocol.observations()["startup.identity"].principal, input().profile.username);
  assert.equal(protocol.observations()["state.after.transaction"], null);
  assert(Object.isFrozen(protocol.observations()["options.before.context"].searchPath));
  assert.equal(protocol.outcomes().length, 24); // Only context/query SQL; no controls/connect.
  assert(
    protocol
      .outcomes()
      .every(
        (entry) =>
          entry.requestStatus === "terminal" &&
          entry.remoteOutcome === "succeeded" &&
          entry.validationStatus === "validated",
      ),
  );
});

test("startup context mismatch poisons before version, including NULL setup or malformed principal/session", () => {
  for (const data of [
    { role: null },
    { database: "OTHER" },
    { schema: null },
    { warehouse: "OTHER" },
    { user: "" },
    { session: null },
  ]) {
    const { protocol, manifest } = setup();
    complete(protocol, protocol.reserveNext(), manifest);
    const request = protocol.reserveNext();
    assert.throws(() => complete(protocol, request, manifest, { data }));
    assert(protocol.snapshot().failure);
    rejects(() => protocol.reserveNext(), "protocolStopped");
    assert.equal(protocol.snapshot().completedCommands, 1);
  }
});

test("query order rejects ACK-less/column-less completion, duplicate/foreign events and readonly USE acknowledgement", () => {
  const mutations = [
    (p, r) => feed(p, frame(r, "phase", { phase: "dispatching" })),
    (p, r) => {
      feed(p, frame(r, "accepted"));
      feed(p, frame(r, "accepted"));
    },
    (p, r) => {
      feed(p, frame(r, "accepted"));
      feed(p, frame(r, "statement", ids(r)));
    },
    (p, r) => {
      begin(p, r);
      feed(p, frame(r, "useAcknowledged", ids(r)));
    },
    (p, r) => {
      begin(p, r);
      queryEnd(p, r, 0);
    },
    (p, r) => feed(p, { ...frame(r, "accepted"), id: "999" }),
    (p, r) =>
      feed(p, { ...frame(r, "accepted"), generation: "12345678-1234-4234-8234-abcdefabcdef" }),
    (p, r) => {
      begin(p, r);
      feed(p, frame(r, "columns", { columns: answer("version").columns }));
      feed(p, frame(r, "columns", { columns: answer("version").columns }));
    },
  ];
  for (const mutate of mutations) {
    const { protocol, manifest } = setup();
    const request = until(protocol, manifest, "startup.version");
    assert.throws(() => mutate(protocol, request));
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("one readonly statement identity may gain queryID but cannot change, repeat or disappear", () => {
  for (const mutation of ["repeat", "sdk", "query", "ackMissing", "terminalMissing"]) {
    const { protocol, manifest } = setup();
    const request = until(protocol, manifest, "startup.version");
    feed(protocol, frame(request, "accepted"));
    feed(protocol, frame(request, "phase", { phase: "dispatching" }));
    feed(protocol, frame(request, "statement", ids(request)));
    if (mutation === "terminalMissing") {
      feed(protocol, frame(request, "phase", { phase: "acknowledged", ...ids(request) }));
      feed(protocol, frame(request, "columns", { columns: answer("version").columns }));
      feed(protocol, frame(request, "rows", { rows: answer("version").rows }));
      rejects(
        () =>
          feed(
            protocol,
            frame(request, "terminal", {
              ok: true,
              ...ids(request),
              result: { rowsSeen: 1, rowsRetained: 1, truncated: false },
            }),
          ),
        "protocolIdentity",
      );
    } else if (mutation === "ackMissing")
      rejects(
        () =>
          feed(
            protocol,
            frame(request, "phase", {
              phase: "acknowledged",
              sdkRequestId: ids(request).sdkRequestId,
            }),
          ),
        "protocolIdentity",
      );
    else
      rejects(
        () =>
          feed(
            protocol,
            frame(
              request,
              "statement",
              mutation === "repeat"
                ? ids(request)
                : mutation === "sdk"
                  ? { ...ids(request), sdkRequestId: sdkId(999) }
                  : { ...ids(request), queryId: sdkId(999) },
            ),
          ),
        "protocolIdentity",
      );
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("rows need reserved allowance and bounded nonempty batches, columns and exact complete counts", () => {
  const cases = [
    { kind: "empty", code: "resultShape" },
    { kind: "large", code: "resultShape" },
    { kind: "uncredited", code: "protocolOrder" },
    { kind: "truncated", code: "resultCounts" },
    { kind: "seen", code: "resultCounts" },
    { kind: "retained", code: "resultCounts" },
  ];
  for (const { kind, code } of cases) {
    const { protocol, request } = schemaStart();
    if (kind === "empty") rejects(() => feed(protocol, frame(request, "rows", { rows: [] })), code);
    else if (kind === "large")
      rejects(
        () =>
          feed(
            protocol,
            frame(request, "rows", {
              rows: Array.from({ length: 129 }, (_, i) => ["DB", `S${i}`]),
            }),
          ),
        code,
      );
    else {
      feed(protocol, frame(request, "rows", { rows: [["DB", "PUBLIC"]] }));
      if (kind === "uncredited")
        rejects(() => feed(protocol, frame(request, "rows", { rows: [["DB", "OTHER"]] })), code);
      else
        rejects(
          () =>
            feed(
              protocol,
              frame(request, "terminal", {
                ok: true,
                ...ids(request),
                result: {
                  rowsSeen: kind === "seen" ? 2 : 1,
                  rowsRetained: kind === "retained" ? 0 : 1,
                  truncated: kind === "truncated",
                  ...ids(request),
                },
              }),
            ),
          code,
        );
    }
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("reserved credit permits rows before its ACK, then settling control reassesses unsatisfied demand", () => {
  const { protocol, request } = schemaStart();
  feed(protocol, frame(request, "rows", { rows: [["DB", "A"]] }));
  assert.equal(protocol.snapshot().creditNeeded, true);
  const first = protocol.reserveCredit();
  assert.equal(first.payload.target, request.id);
  assert(Object.isFrozen(first));
  feed(protocol, frame(request, "rows", { rows: [["DB", "B"]] }));
  assert.equal(protocol.snapshot().creditNeeded, false);
  creditSuccess(protocol, first);
  assert.equal(protocol.snapshot().creditNeeded, true);
  const second = protocol.reserveCredit();
  creditSuccess(protocol, second);
  feed(protocol, frame(request, "rows", { rows: [["DB", "C"]] }));
  queryEnd(protocol, request, 3);
  assert.equal(protocol.reserveNext().payload.sql.includes(" AS PRINCIPAL"), true);
  assert.equal(protocol.snapshot().extraCredits, 2);
});

test("late unaccepted invalidInput credit settles only after original valid terminal and cannot settle SQL itself", () => {
  const { protocol, request } = schemaStart();
  feed(protocol, frame(request, "rows", { rows: [["DB", "PUBLIC"]] }));
  const credit = protocol.reserveCredit();
  queryEnd(protocol, request, 1);
  assert.equal(protocol.reserveNext(), null);
  assert.equal(protocol.snapshot().activeCommandId, "options.schemas");
  feed(
    protocol,
    frame(credit, "terminal", {
      ok: false,
      error: { code: "invalidInput", message: "Invalid helper request" },
    }),
  );
  assert.equal(protocol.snapshot().failure, null);
  const next = protocol.reserveNext();
  assert.equal(next.id, String(BigInt(credit.id) + 1n));
  assert.notEqual(next.id, credit.id);
  rejects(
    () => feed(protocol, frame(credit, "terminal", { ok: true, result: {} })),
    "protocolDuplicate",
  );
});

test("early or accepted credit rejection and two outstanding credits permanently stop SQL", () => {
  for (const kind of ["early", "accepted", "second"]) {
    const { protocol, request } = schemaStart();
    feed(protocol, frame(request, "rows", { rows: [["DB", "PUBLIC"]] }));
    const credit = protocol.reserveCredit();
    if (kind === "second") rejects(() => protocol.reserveCredit(), "protocolCreditAdmission");
    else {
      if (kind === "accepted") {
        feed(protocol, frame(credit, "accepted"));
        queryEnd(protocol, request, 1);
      }
      rejects(
        () =>
          feed(
            protocol,
            frame(credit, "terminal", {
              ok: false,
              error: { code: "invalidInput", message: "Invalid helper request" },
            }),
          ),
        kind === "early" ? "protocolOrder" : "protocolControlRejected",
      );
    }
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("cancel reserves only the owner's active identified statement and ACK never replaces its terminal", () => {
  const { protocol, manifest } = setup();
  const request = until(protocol, manifest, "startup.version");
  begin(protocol, request);
  const cancel = protocol.reserveCancel("ownerCancellation");
  assert.deepEqual(cancel.payload, { target: request.id });
  feed(protocol, frame(cancel, "accepted"));
  feed(protocol, frame(cancel, "terminal", { ok: true, result: { requested: true } }));
  assert.equal(protocol.snapshot().activeCommandId, "startup.version");
  feed(protocol, frame(request, "columns", { columns: answer("version").columns }));
  feed(protocol, frame(request, "rows", { rows: answer("version").rows }));
  queryEnd(protocol, request, 1);
  assert.deepEqual(protocol.outcomes().at(-1), {
    commandId: "startup.version",
    op: "query",
    requestStatus: "terminal",
    remoteOutcome: "succeeded",
    validationStatus: "validated",
  });
  rejects(() => protocol.reserveNext(), "protocolStopped");
  assert.equal(protocol.snapshot().ownerCancelled, true);
  assert.equal(protocol.snapshot().ledgerComplete, false);
  const close = protocol.reserveClose();
  complete(protocol, close, manifest);
  assert.equal(protocol.snapshot().ledgerComplete, false);
  for (const reason of ["timeout", "ownerCancellation"]) {
    const state = setup();
    state.protocol.reserveNext();
    rejects(() => state.protocol.reserveCancel(reason), "protocolCancelAdmission");
  }
});

test("typed parser failure permanently poisons even while shared budget remains healthy", () => {
  for (const kind of ["scalar", "parameter", "schema", "continuity", "sample"]) {
    const { protocol, manifest } = setup();
    const id = {
      scalar: "scalar",
      parameter: "options.before.autocommit",
      schema: "options.schemas",
      continuity: "options.before.context",
      sample: "options.after.context",
    }[kind];
    const request = until(protocol, manifest, id);
    const template = manifest.payload.sqlLedger.find(
      (entry) => entry.sql === request.payload.sql,
    ).templateId;
    const data = answer(template);
    if (kind === "scalar") data.rows[0][0] = "9007199254740992";
    if (kind === "parameter") data.rows[0][0] = "SOME_OTHER_PARAMETER";
    if (kind === "schema") data.rows[0][0] = "OTHER_DB";
    if (kind === "continuity") data.rows[0][5] = "changed-session";
    if (kind === "sample") data.rows[0][1] = null;
    if (kind === "sample") {
      complete(protocol, request, manifest, { data });
      assert.throws(() => {
        let next;
        while ((next = protocol.reserveNext())) complete(protocol, next, manifest);
      });
      assert.equal(protocol.snapshot().failure, "observationChanged");
    } else assert.throws(() => complete(protocol, request, manifest, { data }));
    assert.equal(protocol.snapshot().resultBudget.failed, false);
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
});

test("control IDs cannot introduce statements and raw error messages never escape safe failures", () => {
  const { protocol } = setup();
  const request = protocol.reserveNext();
  feed(protocol, frame(request, "accepted"));
  rejects(() => feed(protocol, frame(request, "statement", ids(request))), "protocolOrder");
  const second = setup();
  const connect = second.protocol.reserveNext();
  feed(second.protocol, frame(connect, "accepted"));
  const secret = "https://private.invalid/token?secret=PASSWORD";
  const error = assert.throws; // No error object or message is ever copied into observations.
  error(
    () =>
      feed(
        second.protocol,
        frame(connect, "terminal", { ok: false, error: { code: "database", message: secret } }),
      ),
    (failure) => failure.safeCode === "protocolError" && !failure.message.includes(secret),
  );
  assert(!JSON.stringify(second.protocol.snapshot()).includes(secret));
});

test("strict descriptors/unknown fields/underreported bytes/frame sizes poison without invoking accessors", () => {
  for (const kind of ["getter", "unknown", "bytes", "oversize"]) {
    const state = setup();
    const request = state.protocol.reserveNext();
    const event = frame(request, "accepted");
    let calls = 0;
    if (kind === "getter")
      Object.defineProperty(event, "extra", {
        enumerable: true,
        get() {
          calls++;
          throw Error("SECRET");
        },
      });
    if (kind === "unknown") event.extra = "secret";
    assert.throws(() =>
      state.protocol.receive(event, {
        bodyBytes:
          kind === "bytes" ? 1 : kind === "oversize" ? PROTOCOL_LIMITS.frameBytes + 1 : 1024,
      }),
    );
    assert.equal(calls, 0);
    rejects(() => state.protocol.reserveNext(), "protocolStopped");
  }
});

test("wire bounds count actual padded framing bytes and all inbound envelopes", () => {
  const { protocol, manifest } = setup();
  const request = protocol.reserveNext();
  const event = frame(request, "accepted");
  feed(protocol, event, PROTOCOL_LIMITS.frameBytes - encode(event).length + 4);
  // Repeated malformed frames cannot escape cumulative caps after poisoning.
  for (let index = 0; index < 6; index++)
    assert.throws(() =>
      feed(protocol, event, PROTOCOL_LIMITS.frameBytes - encode(event).length + 4),
    );
  rejects(
    () => feed(protocol, event, PROTOCOL_LIMITS.frameBytes - encode(event).length + 4),
    "protocolWireLimit",
  );
  const second = setup();
  const connect = second.protocol.reserveNext();
  feed(second.protocol, frame(connect, "accepted"));
  for (
    let index = second.protocol.snapshot().inboundFrames;
    index < PROTOCOL_LIMITS.inboundFrames;
    index++
  )
    assert.throws(() => feed(second.protocol, frame(connect, "accepted")));
  rejects(() => feed(second.protocol, frame(connect, "accepted")), "protocolWireLimit");
  assert.equal(manifest.payload.caps.maximumFrameBytes, PROTOCOL_LIMITS.frameBytes);
});

test("primitive control results, duplicate terminals and context row/column frames fail closed", () => {
  for (const result of [0, true, "", []]) {
    const { protocol, request } = schemaStart();
    feed(protocol, frame(request, "rows", { rows: [["DB", "PUBLIC"]] }));
    const credit = protocol.reserveCredit();
    feed(protocol, frame(credit, "accepted"));
    rejects(() => feed(protocol, frame(credit, "terminal", { ok: true, result })), "protocolShape");
    rejects(() => protocol.reserveNext(), "protocolStopped");
  }
  for (const type of ["columns", "rows"]) {
    const { protocol, manifest } = setup();
    complete(protocol, protocol.reserveNext(), manifest);
    const context = protocol.reserveNext();
    begin(protocol, context);
    rejects(
      () =>
        feed(protocol, frame(context, type, type === "rows" ? { rows: [[]] } : { columns: [] })),
      "protocolOrder",
    );
  }
  const { protocol, manifest } = setup();
  const connect = protocol.reserveNext();
  complete(protocol, connect, manifest);
  rejects(
    () => feed(protocol, frame(connect, "terminal", { ok: true, result: { connected: true } })),
    "protocolDuplicate",
  );
});

test("schema sentinel and reused statement ownership stop before later SQL", () => {
  const { protocol, request } = schemaStart();
  for (let start = 0; start < 1000; start += 128) {
    if (start) creditSuccess(protocol, protocol.reserveCredit());
    feed(
      protocol,
      frame(request, "rows", {
        rows: Array.from({ length: Math.min(128, 1000 - start) }, (_, index) => [
          "DB",
          `S${start + index}`,
        ]),
      }),
    );
  }
  creditSuccess(protocol, protocol.reserveCredit());
  rejects(
    () => feed(protocol, frame(request, "rows", { rows: [["DB", "SENTINEL"]] })),
    "schemaSentinel",
  );
  rejects(() => protocol.reserveNext(), "protocolStopped");
  assert.equal(protocol.snapshot().resultBudget.failed, false);
  const other = setup();
  complete(other.protocol, other.protocol.reserveNext(), other.manifest);
  const initial = other.protocol.reserveNext();
  complete(other.protocol, initial, other.manifest);
  const version = other.protocol.reserveNext();
  feed(other.protocol, frame(version, "accepted"));
  feed(other.protocol, frame(version, "phase", { phase: "dispatching" }));
  rejects(
    () => feed(other.protocol, frame(version, "statement", ids(initial))),
    "protocolIdentity",
  );
});

test("nullable observed role and opaque open transaction remain observations without profile fallback", () => {
  const { protocol, manifest } = setup();
  for (let request; (request = protocol.reserveNext());) {
    const entry = manifest.payload.sqlLedger.find(
      (candidate) => candidate.sql === request.payload.sql,
    );
    let data;
    if (entry?.templateId === "context") {
      data = answer("context");
      data.rows[0][1] = null;
    }
    if (entry?.templateId === "transaction")
      data = answer("transaction", { rows: [["opaque-TX-A7/x"]] });
    complete(protocol, request, manifest, { data });
  }
  assert.equal(protocol.snapshot().ledgerComplete, true);
  assert.equal(protocol.observations()["options.after.context"].role, null);
  assert.equal(protocol.observations()["state.after.transaction"], "opaque-TX-A7/x");
});

test("pure protocol admission and complete validation touch no filesystem/network/process APIs", () => {
  const restores = [];
  let calls = 0;
  const deny = (object, key) => {
    const previous = object[key];
    object[key] = () => {
      calls++;
      throw Error("unexpectedSideEffect");
    };
    restores.push(() => {
      object[key] = previous;
    });
  };
  try {
    for (const name of [
      "openSync",
      "readFileSync",
      "writeFileSync",
      "mkdirSync",
      "mkdtempSync",
      "unlinkSync",
      "realpathSync",
      "lstatSync",
    ])
      deny(fs, name);
    for (const [object, names] of [
      [net, ["connect", "createConnection"]],
      [tls, ["connect"]],
      [dns, ["lookup", "resolve", "resolve4", "resolve6"]],
      [dns.promises, ["lookup", "resolve", "resolve4", "resolve6"]],
      [dgram, ["createSocket"]],
      [http, ["request", "get"]],
      [https, ["request", "get"]],
      [childProcess, ["spawn", "spawnSync", "exec", "execSync", "fork"]],
    ])
      for (const name of names) deny(object, name);
    deny(globalThis, "fetch");
    syncBuiltinESMExports();
    const { protocol, manifest } = setup();
    for (let request; (request = protocol.reserveNext());) complete(protocol, request, manifest);
    assert.equal(protocol.snapshot().ledgerComplete, true);
    assert.equal(calls, 0);
  } finally {
    for (const restore of restores.reverse()) restore();
    syncBuiltinESMExports();
  }
});

test("actual helper plus split framing completes1000-schema workflow with one connection and one shared1023-row budget", async () => {
  const currentInput = input(),
    manifest = compileManifest(currentInput);
  const protocol = createQualificationProtocol({ manifest, currentInput, generation });
  let connections = 0,
    sequence = 0,
    retired = false;
  const sql = [],
    binds = [];
  const sdk = {
    createConnection() {
      connections++;
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
          const entry = manifest.payload.sqlLedger.find(
            (candidate) => candidate.sql === options.sqlText,
          );
          assert(entry);
          const data =
            entry.templateId === "startupIdentity"
              ? {
                  columns: Array.from({ length: 6 }, (_, index) => ({
                    name: String(index),
                    type: "TEXT",
                  })),
                  rows: [["SERVER_NAME", "ROLE", "DB", "PUBLIC", "WH", "opaque-session-A7/x"]],
                }
              : answer(
                  entry.templateId,
                  entry.templateId === "schemas"
                    ? {
                        rows: Array.from({ length: 1000 }, (_, index) => [
                          "DB",
                          `S${String(index).padStart(4, "0")}`,
                        ]),
                      }
                    : {},
                );
          const id = ++sequence;
          const statement = {
            getRequestId: () => sdkId(id),
            getQueryId: () => sdkId(id + 10000),
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
          queueMicrotask(() => options.complete(null, statement));
          return statement;
        },
      };
    },
  };
  const helper = new Helper(
    sdk,
    (value) => feed(protocol, value),
    () => {
      retired = true;
    },
    { clientConfigFile: "unused-synthetic" },
  );
  const send = (request) => {
    const bytes = encode(request),
      decoder = new Decoder();
    for (const part of [bytes.subarray(0, 3), bytes.subarray(3)])
      for (const decoded of decoder.push(part)) helper.receive(decoded);
    decoder.end();
  };
  for (let turns = 0; !protocol.snapshot().ledgerComplete && turns < 10000; turns++) {
    assert.equal(protocol.snapshot().failure, null);
    if (protocol.snapshot().creditNeeded) send(protocol.reserveCredit());
    else {
      const request = protocol.reserveNext();
      if (request) send(request);
    }
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.equal(protocol.snapshot().ledgerComplete, true);
  assert.equal(retired, true);
  assert.equal(connections, 1);
  assert.equal(sql.length, 24);
  assert.deepEqual(
    sql,
    manifest.payload.sqlLedger.map((entry) => entry.sql),
  );
  assert.deepEqual(binds[13], ["9007199254740993", true, "joinery-qualification", null]);
  assert.equal(protocol.snapshot().resultBudget.rows, 1023);
  assert.equal(protocol.snapshot().extraCredits, 7);
  assert.equal(protocol.snapshot().inboundFrames < 4096, true);
  assert.equal(protocol.observations()["options.schemas"].length, 1000);
});
