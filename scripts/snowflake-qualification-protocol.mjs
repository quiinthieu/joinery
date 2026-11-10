// Pure, offline-only protocol admission. This module performs no I/O and grants
// no execution authority. A future runner must separately bind source hashes,
// approval, actual framing byte counts, time, writes, signals and child cleanup.
import { verifyManifest } from "./snowflake-qualification-manifest.mjs";
import {
  createResultBudget,
  createQueryCollector,
  parseStartupContext,
  requireContinuity,
  requireSchemasInContext,
  makeObservationSample,
  compareSamples,
} from "./snowflake-qualification-results.mjs";

export const PROTOCOL_LIMITS = Object.freeze({
  frameBytes: 1048576,
  inboundBytes: 8 * 1024 * 1024,
  inboundFrames: 4096,
  batchRows: 128,
  extraCredits: 1000,
  outstandingCredits: 1,
});
const HELLO_LIMITS = Object.freeze({
  frameBytes: 1048576,
  batchRows: 128,
  batchBytes: 262144,
  queued: 8,
  credits: 32,
});
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SDK_ID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const ERROR_MESSAGES = Object.freeze({
  invalidInput: "Invalid helper request",
  busy: "Console queue is full",
  unsupported: "Only caller-classified read-only queries are supported",
  connectionClosed: "Snowflake helper connection closed",
  cancelled: "Snowflake operation cancelled",
  database: "Snowflake operation failed",
  outcomeUnknown: "Remote statement outcome is unknown",
  resultLimitReached: "Unsupported or oversized result value",
});
const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
function exact(value, required, optional = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw fail("protocolShape");
  const keys = Object.keys(value);
  if (
    required.some((key) => !Object.hasOwn(value, key)) ||
    keys.some((key) => !required.includes(key) && !optional.includes(key))
  )
    throw fail("protocolShape");
}
function freeze(value) {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}

// Feed decoded JSON only. Validate data descriptors before reading them; never
// invoke getters/toJSON while measuring a hostile frame. Actual raw whitespace,
// UTF-8 validity, duplicate JSON members and physical counts belong to framing.
function copyFrame(input) {
  let nodes = 0;
  let measured = 0;
  function charge(size) {
    measured += size;
    if (measured > PROTOCOL_LIMITS.frameBytes) throw fail("protocolFrameLimit");
  }
  function string(value) {
    if (value.length > PROTOCOL_LIMITS.frameBytes || !value.isWellFormed())
      throw fail("protocolShape");
    charge(2);
    for (const character of value) {
      const point = character.codePointAt(0);
      charge(
        point < 32
          ? [8, 9, 10, 12, 13].includes(point)
            ? 2
            : 6
          : point === 34 || point === 92
            ? 2
            : Buffer.byteLength(character),
      );
    }
    return value;
  }
  const seen = new WeakSet();
  function copy(value, depth) {
    if (++nodes > 10000 || depth > 16) throw fail("protocolShape");
    if (value === null || typeof value === "boolean") {
      charge(value === null || value === true ? 4 : 5);
      return value;
    }
    if (typeof value === "number") {
      if (!Number.isSafeInteger(value) || Object.is(value, -0)) throw fail("protocolShape");
      charge(String(value).length);
      return value;
    }
    if (typeof value === "string") {
      return string(value);
    }
    if (!value || typeof value !== "object" || seen.has(value)) throw fail("protocolShape");
    seen.add(value);
    const isArray = Array.isArray(value);
    if (
      !(isArray
        ? Object.getPrototypeOf(value) === Array.prototype
        : [Object.prototype, null].includes(Object.getPrototypeOf(value)))
    )
      throw fail("protocolShape");
    const keys = Reflect.ownKeys(value);
    if (keys.length > 4097) throw fail("protocolShape");
    if (isArray && (value.length > 4096 || keys.length !== value.length + 1))
      throw fail("protocolShape");
    const result = isArray ? [] : Object.create(null);
    charge(2);
    let members = 0;
    for (const key of keys) {
      if (isArray && key === "length") continue;
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (
        typeof key !== "string" ||
        !descriptor.enumerable ||
        !Object.hasOwn(descriptor, "value") ||
        (isArray && !/^(0|[1-9][0-9]*)$/.test(key))
      )
        throw fail("protocolShape");
      if (members++) charge(1);
      if (!isArray) {
        string(key);
        charge(1);
      }
      Object.defineProperty(result, key, {
        value: copy(descriptor.value, depth + 1),
        enumerable: true,
        writable: true,
        configurable: true,
      });
    }
    return result;
  }
  const value = copy(input, 0);
  if (!value || typeof value !== "object" || Array.isArray(value)) throw fail("protocolShape");
  const bytes = Buffer.byteLength(JSON.stringify(value));
  if (!bytes || bytes > PROTOCOL_LIMITS.frameBytes) throw fail("protocolFrameLimit");
  return { value, bytes };
}
function identities(value) {
  const result = {};
  for (const key of ["sdkRequestId", "queryId"]) {
    if (!Object.hasOwn(value, key)) continue;
    if (typeof value[key] !== "string" || !SDK_ID.test(value[key])) throw fail("protocolIdentity");
    result[key] = value[key];
  }
  if (result.queryId && !result.sdkRequestId) throw fail("protocolIdentity");
  return result;
}
const sameIds = (left, right) =>
  left.sdkRequestId === right.sdkRequestId && left.queryId === right.queryId;

/**
 * Offline v1 ledger validation, not approval or a runner. bodyBytes passed to
 * receive must come from the trusted decoder's actual JSON body length; the
 * module checks underreporting against compact JSON, but cannot measure a pipe.
 * Limits count INBOUND bytes (including each four-byte header) and frames.
 * Outbound is independently bounded by the immutable ledger +1000 credits
 * +one owner cancel +one close, with every body <=1MiB. No timers or I/O occur.
 */
export function createQualificationProtocol({ manifest, currentInput, generation }) {
  const verified = verifyManifest(manifest, currentInput);
  if (typeof generation !== "string" || !UUID.test(generation)) throw fail("protocolGeneration");
  const { commands, sqlLedger } = verified.payload;
  const profile = commands[0].payload;
  const budget = createResultBudget();
  const requests = new Map();
  const values = new Map();
  const sdkIds = new Set();
  const queryIds = new Set();
  let hello = false,
    cursor = 0,
    lastId = 0n,
    active = null,
    failure = null,
    cancelReserved = false,
    closedEarly = false,
    closeRequest = null,
    extraCredits = 0,
    inboundBytes = 0,
    inboundFrames = 0,
    outboundBytes = 0,
    outboundFrames = 0,
    initial;

  function guarded(action) {
    try {
      return action();
    } catch (error) {
      const code =
        typeof error?.safeCode === "string" && /^[a-zA-Z]{1,80}$/.test(error.safeCode)
          ? error.safeCode
          : "protocolInvalid";
      failure ??= code;
      if (active?.evidence?.validationStatus === "pending")
        active.evidence.validationStatus = "failed";
      throw fail(code);
    }
  }
  function admitted() {
    if (failure || cancelReserved || closedEarly) throw fail("protocolStopped");
  }
  function pendingControls(target) {
    return [...requests.values()].filter(
      (request) => request.control && !request.terminal && (!target || request.target === target),
    );
  }
  function release() {
    if (active?.terminal && !pendingControls(active).length) active = null;
  }
  function reserve(op, payload, rest) {
    if (++lastId > 18446744073709551615n) throw fail("protocolIdLimit");
    const frame = freeze({ v: 1, generation, id: String(lastId), op, payload });
    const bytes = Buffer.byteLength(JSON.stringify(frame));
    if (bytes > PROTOCOL_LIMITS.frameBytes || outboundFrames >= commands.length + 1001)
      throw fail("protocolOutboundLimit");
    const request = {
      id: frame.id,
      op,
      accepted: false,
      terminal: false,
      terminalReceived: false,
      ...rest,
      // A reservation is not proof that a writer submitted anything. Preserve
      // explicit uncertainty if the runner loses its pipe before any response.
      evidence: rest.ledger
        ? {
            commandId: rest.command.id,
            op,
            requestStatus: "reserved",
            remoteOutcome: "unknown",
            validationStatus: "pending",
          }
        : null,
    };
    // Ownership is installed before the frame is exposed to a future writer.
    requests.set(frame.id, request);
    outboundBytes += bytes + 4;
    outboundFrames++;
    return { frame, request };
  }
  function reserveClose() {
    if (!hello || !requests.size || closeRequest) throw fail("protocolCloseAdmission");
    if (cursor !== commands.length - 1 || active || pendingControls().length) closedEarly = true;
    const reserved = reserve("close", {}, { control: true, kind: "close" });
    closeRequest = reserved.request;
    return reserved.frame;
  }
  function reserveNext() {
    admitted();
    if (!hello || active || pendingControls().length || cursor === commands.length) return null;
    const command = commands[cursor];
    if (command.op === "close") return reserveClose();
    const ledger = command.sqlOrdinals.length ? sqlLedger[command.sqlOrdinals[0] - 1] : null;
    const reserved = reserve(command.op, command.payload, {
      command,
      ledger,
      dispatched: false,
      acknowledged: false,
      ids: {},
      allowance: command.op === "query" ? 1 : 0,
      batches: 0,
      collector:
        command.op === "query" ? createQueryCollector(ledger.templateId, { budget }) : null,
    });
    active = reserved.request;
    return reserved.frame;
  }
  function creditNeeded() {
    return Boolean(
      !failure &&
      !cancelReserved &&
      !closedEarly &&
      active?.ledger?.templateId === "schemas" &&
      !active.terminal &&
      active.batches &&
      active.allowance === 0 &&
      !pendingControls(active).length,
    );
  }
  function requireIds(request, frame) {
    const observed = identities(frame);
    if (!sameIds(request.ids, observed)) throw fail("protocolIdentity");
  }
  function statement(request, frame) {
    if (!request.dispatched || request.acknowledged || request.terminal)
      throw fail("protocolOrder");
    const next = identities(frame);
    if (!next.sdkRequestId) throw fail("protocolIdentity");
    if (!request.ids.sdkRequestId) {
      const key = next.sdkRequestId.toLowerCase();
      if (sdkIds.has(key)) throw fail("protocolIdentity");
      sdkIds.add(key);
    } else if (
      next.sdkRequestId !== request.ids.sdkRequestId ||
      request.ids.queryId ||
      !next.queryId
    )
      throw fail("protocolIdentity");
    if (next.queryId) {
      const key = next.queryId.toLowerCase();
      if (queryIds.has(key)) throw fail("protocolIdentity");
      queryIds.add(key);
    }
    request.ids = next;
  }
  function observe(request, result) {
    const id = request.command.id;
    if (request.op === "context") {
      initial = parseStartupContext(
        result,
        {
          role: profile.role,
          database: profile.database,
          schema: profile.schema,
          warehouse: profile.warehouse,
        },
        { budget },
      );
      values.set(id, initial);
      return;
    }
    const value = request.collector.finish(result);
    if (request.ledger.templateId === "context") requireContinuity(value, initial);
    if (request.ledger.templateId === "schemas")
      requireSchemasInContext(value, values.get("options.before.context"));
    values.set(id, value);
    if (id === "options.after.transaction" || id === "state.after.transaction") {
      const group = id.split(".")[0];
      const sample = (prefix) =>
        makeObservationSample({
          context: values.get(`${prefix}.context`),
          quotedIdentifierPolicy: values.get(`${prefix}.quotedIdentifierPolicy`),
          autocommit: values.get(`${prefix}.autocommit`),
          abortOnError: values.get(`${prefix}.abortOnError`),
          transactionId: values.get(`${prefix}.transaction`),
        });
      if (compareSamples(sample(`${group}.before`), sample(`${group}.after`)) !== "stable")
        throw fail("observationChanged");
    }
  }
  function terminal(request, frame) {
    if (request.terminalReceived) throw fail("protocolDuplicate");
    if (typeof frame.ok !== "boolean") throw fail("protocolShape");
    exact(
      frame,
      ["v", "generation", "id", "type", "ok", frame.ok ? "result" : "error"],
      request.control || request.op === "connect" ? [] : ["sdkRequestId", "queryId"],
    );
    if (!request.accepted) {
      // Credits are separately owned. A query may validly finish while its
      // reserved credit is still in the pipe; only that unaccepted rejection is
      // benign. No accepted rejection or early rejection can use this escape.
      if (!(
        request.kind === "credit" &&
        request.target.terminal &&
        !frame.ok &&
        frame.error?.code === "invalidInput"
      ))
        throw fail("protocolOrder");
    }
    if (!frame.ok) {
      exact(frame.error, ["code", "message"]);
      if (
        !Object.hasOwn(ERROR_MESSAGES, frame.error.code) ||
        frame.error.message !== ERROR_MESSAGES[frame.error.code]
      )
        throw fail("protocolError");
      if (
        request.kind === "credit" &&
        !request.accepted &&
        request.target.terminal &&
        frame.error.code === "invalidInput"
      ) {
        request.terminalReceived = true;
        request.terminal = true;
        release();
        return;
      }
      if (!request.control && request.op !== "connect") requireIds(request, frame);
      request.terminalReceived = true;
      if (request.evidence) {
        request.evidence.requestStatus = "terminal";
        request.evidence.terminalErrorCode = frame.error.code;
        request.evidence.validationStatus = "failed";
        // Query database errors before ACK use the helper's existing non-08
        // SQLSTATE rejection classifier. Context's public ACK occurs only after
        // its result validation, so context database errors cannot prove that
        // its SELECT failed. An existing success ACK always takes precedence.
        if (!request.acknowledged && request.op === "query" && frame.error.code === "database") {
          // The helper announces dispatch before execute can classify a query
          // rejection. A missing phase is not evidence of a failed SQL submit.
          if (!request.dispatched) throw fail("protocolOrder");
          request.evidence.remoteOutcome = "rejected";
        }
      }
      request.terminal = true;
      if (request.control) throw fail("protocolControlRejected");
      failure ??= "helperOperationFailed";
      release();
      return;
    }
    if (request.control || request.op === "connect") {
      if (request.op === "connect") {
        exact(frame.result, ["connected"]);
        if (frame.result.connected !== true) throw fail("protocolResult");
        cursor++;
      } else if (request.kind === "close") {
        exact(frame.result, ["closed", "logoutAcknowledged"]);
        if (frame.result.closed !== true || frame.result.logoutAcknowledged !== true)
          throw fail("logoutUnconfirmed");
        if (!closedEarly && cursor === commands.length - 1) cursor++;
      } else if (request.kind === "cancel") {
        exact(frame.result, ["requested"]);
        if (frame.result.requested !== true) throw fail("protocolResult");
      } else exact(frame.result, []);
    } else {
      if (
        !request.acknowledged ||
        !request.ids.sdkRequestId ||
        (request.op === "query" && !request.columns)
      )
        throw fail("protocolOrder");
      requireIds(request, frame);
      request.terminalReceived = true;
      request.evidence.requestStatus = "terminal";
      requireIds(request, frame.result);
      // Typed parsing, continuity and full sample comparison must complete
      // before either the ledger cursor or original-terminal proof advances.
      observe(request, frame.result);
      if (request.evidence.validationStatus === "pending")
        request.evidence.validationStatus = "validated";
      cursor++;
    }
    request.terminalReceived = true;
    request.terminal = true;
    release();
  }
  function receive(frame, { bodyBytes } = {}) {
    if (!Number.isSafeInteger(bodyBytes) || bodyBytes < 1 || bodyBytes > PROTOCOL_LIMITS.frameBytes)
      throw fail("protocolFrameLimit");
    if (
      ++inboundFrames > PROTOCOL_LIMITS.inboundFrames ||
      bodyBytes + 4 > PROTOCOL_LIMITS.inboundBytes - inboundBytes
    )
      throw fail("protocolWireLimit");
    inboundBytes += bodyBytes + 4;
    const { value, bytes } = copyFrame(frame);
    if (bodyBytes < bytes) throw fail("protocolFrameBytes");
    frame = value;
    if (frame.type === "hello") {
      exact(frame, ["v", "type", "protocol", "sdkVersion", "limits"]);
      exact(frame.limits, Object.keys(HELLO_LIMITS));
      if (
        hello ||
        frame.v !== 1 ||
        frame.protocol !== "joinery-snowflake" ||
        frame.sdkVersion !== "3.4.1" ||
        Object.keys(HELLO_LIMITS).some((key) => frame.limits[key] !== HELLO_LIMITS[key])
      )
        throw fail("protocolHello");
      hello = true;
      return;
    }
    if (!hello || frame.v !== 1 || frame.generation !== generation || typeof frame.id !== "string")
      throw fail("protocolEnvelope");
    const request = requests.get(frame.id);
    if (!request) throw fail("protocolForeignId");
    if (request.terminalReceived) throw fail("protocolDuplicate");
    const base = ["v", "generation", "id", "type"];
    if (frame.type === "terminal") return terminal(request, frame);
    if (frame.type === "accepted") {
      exact(frame, base);
      if (request.accepted) throw fail("protocolDuplicate");
      request.accepted = true;
      if (request.evidence) request.evidence.requestStatus = "accepted";
      return;
    }
    if (!request.accepted || request.control || request.op === "connect")
      throw fail("protocolOrder");
    if (frame.type === "phase") {
      exact(frame, [...base, "phase"], ["sdkRequestId", "queryId"]);
      if (frame.phase === "dispatching") {
        if (
          request.dispatched ||
          Object.hasOwn(frame, "sdkRequestId") ||
          Object.hasOwn(frame, "queryId")
        )
          throw fail("protocolOrder");
        request.dispatched = true;
        request.evidence.requestStatus = "dispatching";
      } else if (frame.phase === "acknowledged") {
        if (!request.dispatched || request.acknowledged || !request.ids.sdkRequestId)
          throw fail("protocolOrder");
        requireIds(request, frame);
        request.acknowledged = true;
        request.evidence.requestStatus = "acknowledged";
        request.evidence.remoteOutcome = "succeeded";
      } else throw fail("protocolOrder");
    } else if (frame.type === "statement") {
      exact(frame, [...base, "sdkRequestId"], ["queryId"]);
      statement(request, frame);
    } else if (frame.type === "columns") {
      exact(frame, [...base, "columns"]);
      if (request.op !== "query" || !request.acknowledged || request.columns)
        throw fail("protocolOrder");
      request.collector.addColumns(frame.columns);
      request.columns = true;
    } else if (frame.type === "rows") {
      exact(frame, [...base, "rows"]);
      if (
        request.op !== "query" ||
        !request.columns ||
        !request.acknowledged ||
        request.allowance < 1
      )
        throw fail("protocolOrder");
      request.allowance--;
      request.collector.addRows(frame.rows);
      request.batches++;
    } else throw fail("protocolFrameType");
  }
  return Object.freeze({
    reserveNext: () => guarded(reserveNext),
    reserveClose: () => guarded(reserveClose),
    reserveCredit: () =>
      guarded(() => {
        admitted();
        if (!creditNeeded() || extraCredits >= PROTOCOL_LIMITS.extraCredits)
          throw fail("protocolCreditAdmission");
        extraCredits++;
        active.allowance++;
        return reserve(
          "credit",
          { target: active.id, batches: 1, discard: false },
          { control: true, kind: "credit", target: active },
        ).frame;
      }),
    reserveCancel: (reason) =>
      guarded(() => {
        admitted();
        if (
          reason !== "ownerCancellation" ||
          !active?.ids?.sdkRequestId ||
          active.terminal ||
          cancelReserved ||
          closeRequest
        )
          throw fail("protocolCancelAdmission");
        cancelReserved = true;
        return reserve(
          "cancel",
          { target: active.id },
          { control: true, kind: "cancel", target: active },
        ).frame;
      }),
    receive: (frame, counts) => guarded(() => receive(frame, counts)),
    snapshot: () =>
      freeze({
        executionStatus: "offlineOnly",
        authorizationGranted: false,
        runnerImplemented: false,
        hello,
        failure,
        ownerCancelled: cancelReserved,
        closedEarly,
        ledgerComplete:
          !failure &&
          !cancelReserved &&
          !closedEarly &&
          cursor === commands.length &&
          !active &&
          !pendingControls().length,
        completedCommands: cursor,
        activeCommandId: active?.command?.id ?? null,
        nextCommandId: commands[cursor]?.id ?? null,
        creditNeeded: creditNeeded(),
        pendingControls: pendingControls().length,
        extraCredits,
        inboundBytes,
        inboundFrames,
        outboundBytes,
        outboundFrames,
        resultBudget: budget.snapshot(),
      }),
    // Private typed observations: never suitable for console/evidence logging.
    observations: () => freeze(Object.fromEntries(values)),
    // SQL-only evidence, installed before a frame escapes. Request progress,
    // remote execution certainty and qualification validation are independent.
    // Controls/connect never acquire a SQL outcome. No identities/values leak.
    outcomes: () =>
      freeze(
        [...requests.values()]
          .filter((request) => request.evidence)
          .map((request) => ({ ...request.evidence })),
      ),
  });
}
