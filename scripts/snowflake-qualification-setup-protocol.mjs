// Closed, pure protocol state. This module cannot authenticate or grant approval.
import { randomUUID } from "node:crypto";
import { canonicalJson } from "./snowflake-qualification-manifest.mjs";
import { verifySetupManifest } from "./snowflake-qualification-setup-manifest.mjs";
import {
  parseQueryResult,
  parseStartupContext,
  createQueryCollector,
} from "./snowflake-qualification-results.mjs";

const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
const UUID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const HELLO_LIMITS = {
  frameBytes: 1048576,
  batchRows: 128,
  batchBytes: 262144,
  queued: 8,
  credits: 32,
};
const ERRORS = new Set([
  "invalidInput",
  "busy",
  "unsupported",
  "connectionClosed",
  "cancelled",
  "database",
  "outcomeUnknown",
  "resultLimitReached",
]);
function exact(value, required, optional = []) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    required.some((key) => !Object.hasOwn(value, key)) ||
    Object.keys(value).some((key) => !required.includes(key) && !optional.includes(key))
  )
    throw fail("setupProtocolShape");
}
function ids(value) {
  const result = {};
  for (const key of ["sdkRequestId", "queryId"])
    if (Object.hasOwn(value, key)) {
      if (typeof value[key] !== "string" || !UUID.test(value[key]))
        throw fail("setupProtocolIdentity");
      result[key] = value[key];
    }
  if (result.queryId && !result.sdkRequestId) throw fail("setupProtocolIdentity");
  return result;
}
const same = (a, b) => canonicalJson(a) === canonicalJson(b);
const copy = (value) => JSON.parse(canonicalJson(value));
const scalar = (value) =>
  value === null ||
  (typeof value === "string" &&
    value.isWellFormed() &&
    !value.includes("\0") &&
    Buffer.byteLength(value) <= 65536);

export function createSetupProtocol({ manifest, currentInput, generation = randomUUID() }) {
  const verified = verifySetupManifest(manifest, currentInput);
  if (!UUID.test(generation)) throw fail("setupProtocolGeneration");
  const { sqlLedger: ledger, profile, runId, target } = verified.payload;
  const requests = new Map(),
    sdkIds = new Set(),
    queryIds = new Set(),
    evidence = [],
    events = [];
  let hello = false,
    cursor = 0,
    active = null,
    counter = 0,
    failure = null,
    initial = null;
  let close = null,
    cancel = null,
    logoutAcknowledged = false,
    secondary = null,
    settings = null,
    sample = {};
  function event(kind, data) {
    events.push(copy({ kind, ...data }));
  }
  function stop(code) {
    failure ??= code;
  }
  function reserve(op, payload, step = null) {
    const frame = { v: 1, generation, id: String(++counter), op, payload };
    const request = {
      id: frame.id,
      op,
      step,
      accepted: false,
      dispatching: false,
      acknowledged: false,
      createAcknowledged: false,
      terminal: false,
      ids: {},
    };
    if (step) {
      request.evidence = {
        step: step.index,
        key: step.key,
        mutation: step.mutation,
        remoteOutcome: "unknown",
        validationStatus: "pending",
        requestStatus: "reserved",
      };
      evidence.push(request.evidence);
      event("intent", { step: step.index, key: step.key, mutation: step.mutation });
    } else event("controlIntent", { op });
    requests.set(request.id, request);
    active = request;
    return frame;
  }
  function reserveNext() {
    if (failure || !hello || active || close) return null;
    if (cursor === 0) return reserve("connect", profile);
    if (cursor > ledger.length) return null;
    const step = ledger[cursor - 1];
    if (step.index === 1) return reserve("context", {}, step);
    if (step.index === 2) {
      const frame = reserve(
        "query",
        {
          sql: step.sql,
          binds: [],
          classification: "read",
          credits: 1,
          maxRows: 1,
          maxBytes: 65536,
        },
        step,
      );
      active.collector = createQueryCollector("version");
      return frame;
    }
    if (!initial) throw fail("setupProtocolIdentity");
    return reserve(
      "setup",
      {
        runId,
        step: step.index,
        expected: { user: initial.principal, session: initial.sessionId },
      },
      step,
    );
  }
  function requireIds(request, value) {
    if (!same(ids(value), request.ids)) throw fail("setupProtocolIdentity");
  }
  function observe(step, result) {
    if (step.index === 1) {
      initial = parseStartupContext(result, {
        role: profile.role,
        database: profile.database,
        schema: profile.schema,
        warehouse: profile.warehouse,
      });
      return;
    }
    if (step.index === 2) return;
    exact(
      result,
      [
        "runId",
        "step",
        "key",
        "phase",
        "mutation",
        "rowsSeen",
        "rowsRetained",
        "truncated",
        "columns",
        "values",
        "sdkRequestId",
      ],
      ["queryId"],
    );
    if (
      result.runId !== runId ||
      result.step !== step.index ||
      result.key !== step.key ||
      result.phase !== step.phase ||
      result.mutation !== step.mutation ||
      result.rowsSeen !== 1 ||
      result.rowsRetained !== 0 ||
      result.truncated !== false ||
      !Array.isArray(result.columns) ||
      result.columns.length < 1 ||
      result.columns.length > step.maximumColumns ||
      !Array.isArray(result.values) ||
      result.values.length !== 1 ||
      !Array.isArray(result.values[0]) ||
      result.values[0].length !== result.columns.length ||
      !result.values[0].every(scalar) ||
      Buffer.byteLength(canonicalJson({ columns: result.columns, values: result.values })) > 65536
    )
      throw fail("setupResultShape");
    for (const column of result.columns) {
      exact(column, ["name", "type"]);
      if (
        typeof column.name !== "string" ||
        !column.name ||
        Buffer.byteLength(column.name) > 4096 ||
        typeof column.type !== "string" ||
        !column.type ||
        Buffer.byteLength(column.type) > 128
      )
        throw fail("setupResultShape");
    }
    if (step.mutation) {
      if (
        result.columns.length !== 1 ||
        result.columns[0].type.toLowerCase() !== "text" ||
        typeof result.values[0][0] !== "string"
      )
        throw fail("setupCreateResult");
      return;
    }
    const template = step.key.split(".").at(-1);
    const answer = {
      columns: result.columns,
      rows: result.values,
      result: { rowsSeen: 1, rowsRetained: 1, truncated: false, ...ids(result) },
    };
    if (
      ["context", "quotedIdentifierPolicy", "autocommit", "abortOnError", "transaction"].includes(
        template,
      )
    ) {
      const parsed = parseQueryResult(template, answer);
      if (template === "context") {
        const database = step.phase === "initial" ? profile.database : target.database;
        const schema =
          step.phase === "initial"
            ? profile.schema
            : step.phase === "database"
              ? "PUBLIC"
              : target.schema;
        if (
          parsed.principal !== initial.principal ||
          parsed.sessionId !== initial.sessionId ||
          parsed.role !== profile.role ||
          parsed.warehouse !== profile.warehouse ||
          parsed.database !== database ||
          parsed.schema !== schema
        )
          throw fail("setupContextMismatch");
        // CURRENT_SCHEMAS resolves against current context; CREATE changes it.
        // All object SQL is fully qualified. Do not claim path atomicity or ABA protection.
        secondary ??= parsed.secondaryRoles;
        if (!same(secondary, parsed.secondaryRoles)) throw fail("setupSecondaryRolesChanged");
        sample = { context: true };
      } else {
        if (!sample.context) throw fail("setupObserverOrder");
        if (
          (template === "autocommit" && parsed !== true) ||
          (template === "transaction" && parsed !== null)
        )
          throw fail("setupSessionNotIdle");
        sample[template] = parsed;
        if (template === "transaction") {
          if (
            sample.quotedIdentifierPolicy !== false ||
            sample.autocommit !== true ||
            parsed !== null ||
            typeof sample.abortOnError !== "boolean"
          )
            throw fail("setupSessionNotIdle");
          const current = {
            quotedIdentifierPolicy: sample.quotedIdentifierPolicy,
            autocommit: sample.autocommit,
            abortOnError: sample.abortOnError,
          };
          settings ??= current;
          if (!same(settings, current)) throw fail("setupSettingsChanged");
        }
      }
      return;
    }
    const row = result.values[0],
      names = result.columns.map((c) => c.name),
      types = result.columns.map((c) => c.type.toLowerCase());
    if (step.key === "verify.database") {
      if (
        !same(names, ["DATABASE_NAME", "DATABASE_OWNER", "IS_TRANSIENT", "COMMENT", "CREATED"]) ||
        !same(types, ["text", "text", "text", "text", "timestamp_ltz"]) ||
        !same(row.slice(0, 4), [target.database, profile.role, "YES", target.label]) ||
        typeof row[4] !== "string" ||
        !row[4]
      )
        throw fail("setupDatabaseMismatch");
    } else if (step.key === "verify.schema") {
      if (
        !same(names, ["CATALOG_NAME", "SCHEMA_NAME", "SCHEMA_OWNER", "IS_TRANSIENT", "COMMENT"]) ||
        !types.every((type) => type === "text") ||
        !same(row, [target.database, target.schema, profile.role, "YES", target.label])
      )
        throw fail("setupSchemaMismatch");
    } else if (step.key === "verify.empty") {
      if (!same(names, ["ROW_COUNT"]) || !same(types, ["fixed"]) || !same(row, ["0"]))
        throw fail("setupTableNotEmpty");
    } else throw fail("setupUnknownStep");
  }
  function receive(raw) {
    const frame = copy(raw);
    if (frame.type === "hello") {
      exact(frame, ["v", "type", "protocol", "sdkVersion", "limits"]);
      if (
        hello ||
        frame.v !== 1 ||
        frame.protocol !== "joinery-snowflake" ||
        frame.sdkVersion !== "3.4.1" ||
        !same(frame.limits, HELLO_LIMITS)
      )
        throw fail("setupProtocolHello");
      hello = true;
      return;
    }
    if (!hello || frame.v !== 1 || frame.generation !== generation)
      throw fail("setupProtocolEnvelope");
    const request = requests.get(frame.id);
    if (!request || request.terminal) throw fail("setupProtocolForeignId");
    const base = ["v", "generation", "id", "type"];
    if (frame.type === "accepted") {
      exact(frame, base);
      if (request.accepted) throw fail("setupProtocolOrder");
      request.accepted = true;
      return;
    }
    if (frame.type === "terminal") {
      exact(
        frame,
        [...base, "ok", frame.ok ? "result" : "error"],
        request.step ? ["sdkRequestId", "queryId"] : [],
      );
      if (typeof frame.ok !== "boolean") throw fail("setupProtocolShape");
      if (!frame.ok) {
        exact(frame.error, ["code", "message"]);
        if (
          !ERRORS.has(frame.error.code) ||
          typeof frame.error.message !== "string" ||
          frame.error.message.length > 256
        )
          throw fail("setupProtocolError");
        if (request.step) {
          requireIds(request, frame);
          if (
            !request.acknowledged &&
            !request.createAcknowledged &&
            request.dispatching &&
            ["query", "setup"].includes(request.op) &&
            frame.error.code === "database"
          )
            request.evidence.remoteOutcome = "rejected";
          Object.assign(request.evidence, {
            terminalErrorCode: frame.error.code,
            requestStatus: "terminal",
            validationStatus: "failed",
          });
          event("terminal", request.evidence);
        }
        request.terminal = true;
        if (active === request) active = null;
        stop("setupHelperOperationFailed");
        return;
      }
      if (!request.accepted) throw fail("setupProtocolOrder");
      if (request.op === "connect") {
        exact(frame.result, ["connected"]);
        if (frame.result.connected !== true) throw fail("setupConnectFailed");
        cursor = 1;
      } else if (request.op === "close") {
        exact(frame.result, ["closed", "logoutAcknowledged"]);
        if (frame.result.closed !== true || frame.result.logoutAcknowledged !== true)
          throw fail("setupLogoutUnconfirmed");
        logoutAcknowledged = true;
      } else if (request.op === "cancel") {
        exact(frame.result, ["requested"]);
        if (frame.result.requested !== true) throw fail("setupCancelUnconfirmed");
      } else {
        if (
          !request.acknowledged ||
          !request.ids.sdkRequestId ||
          (request.step.mutation && !request.createAcknowledged)
        )
          throw fail("setupProtocolOrder");
        requireIds(request, frame);
        requireIds(request, frame.result);
        try {
          if (request.op === "query") request.collector.finish(frame.result);
          observe(request.step, frame.result);
        } catch (error) {
          Object.assign(request.evidence, {
            requestStatus: "terminal",
            validationStatus: "failed",
            validationErrorCode: /^[A-Za-z]{1,80}$/.test(error.safeCode ?? "")
              ? error.safeCode
              : "setupResultInvalid",
          });
          event("validationFailed", request.evidence);
          request.terminal = true;
          if (active === request) active = null;
          throw error;
        }
        Object.assign(request.evidence, {
          requestStatus: "terminal",
          validationStatus: "validated",
        });
        event("terminal", { ...request.evidence, result: frame.result });
        cursor++;
      }
      request.terminal = true;
      if (active === request) active = null;
      return;
    }
    if (!request.accepted || !request.step) throw fail("setupProtocolOrder");
    if (frame.type === "phase") {
      exact(frame, [...base, "phase"], ["sdkRequestId", "queryId"]);
      if (frame.phase === "dispatching") {
        if (request.dispatching || Object.keys(ids(frame)).length) throw fail("setupProtocolOrder");
        request.dispatching = true;
        request.evidence.requestStatus = "dispatching";
      } else if (frame.phase === "acknowledged") {
        if (!request.dispatching || request.acknowledged || !request.ids.sdkRequestId)
          throw fail("setupProtocolOrder");
        requireIds(request, frame);
        request.acknowledged = true;
        request.evidence.remoteOutcome = "succeeded";
      } else throw fail("setupProtocolOrder");
    } else if (frame.type === "statement") {
      exact(frame, [...base, "sdkRequestId"], ["queryId"]);
      if (!request.dispatching || request.acknowledged) throw fail("setupProtocolOrder");
      const next = ids(frame);
      if (!request.ids.sdkRequestId) {
        if (sdkIds.has(next.sdkRequestId.toLowerCase())) throw fail("setupProtocolIdentity");
        sdkIds.add(next.sdkRequestId.toLowerCase());
      } else if (
        request.ids.sdkRequestId !== next.sdkRequestId ||
        request.ids.queryId ||
        !next.queryId
      )
        throw fail("setupProtocolIdentity");
      if (next.queryId) {
        if (queryIds.has(next.queryId.toLowerCase())) throw fail("setupProtocolIdentity");
        queryIds.add(next.queryId.toLowerCase());
      }
      request.ids = next;
      Object.assign(request.evidence, next);
      event("statement", { step: request.step.index, ...next });
    } else if (frame.type === "setupAcknowledged") {
      exact(frame, [...base, "runId", "step", "key", "sdkRequestId"], ["queryId"]);
      if (
        !request.step.mutation ||
        request.createAcknowledged ||
        !request.dispatching ||
        request.acknowledged ||
        frame.runId !== runId ||
        frame.step !== request.step.index ||
        frame.key !== request.step.key
      )
        throw fail("setupProtocolOrder");
      requireIds(request, frame);
      if (!request.ids.sdkRequestId) throw fail("setupProtocolIdentity");
      request.createAcknowledged = true;
      request.evidence.remoteOutcome = "succeeded";
      event("createAcknowledged", {
        step: request.step.index,
        key: request.step.key,
        ...request.ids,
      });
    } else if (frame.type === "columns" && request.op === "query") {
      exact(frame, [...base, "columns"]);
      if (!request.acknowledged || request.columns) throw fail("setupProtocolOrder");
      request.collector.addColumns(frame.columns);
      request.columns = true;
    } else if (frame.type === "rows" && request.op === "query") {
      exact(frame, [...base, "rows"]);
      if (!request.acknowledged || !request.columns || request.rows)
        throw fail("setupProtocolOrder");
      request.collector.addRows(frame.rows);
      request.rows = true;
    } else throw fail("setupProtocolFrameType");
  }
  return Object.freeze({
    reserveNext,
    reserveClose() {
      if (!hello || close) return null;
      close = reserve("close", {});
      return close;
    },
    reserveCancel() {
      if (cancel || !active?.ids.sdkRequestId || active.terminal) return null;
      const target = active;
      cancel = reserve("cancel", { target: target.id });
      active = target;
      return cancel;
    },
    receive(frame) {
      try {
        receive(frame);
      } catch (error) {
        stop(error.safeCode ?? "setupProtocolFailed");
        throw error;
      }
    },
    stop,
    takeEvents() {
      return events.splice(0);
    },
    outcomes: () => copy(evidence),
    snapshot: () => ({
      hello,
      failure,
      nextStep: cursor,
      ledgerComplete: cursor === ledger.length + 1,
      activeId: active?.id ?? null,
      activeSdkRequestId: active?.ids.sdkRequestId ?? null,
      logoutAcknowledged,
    }),
  });
}
