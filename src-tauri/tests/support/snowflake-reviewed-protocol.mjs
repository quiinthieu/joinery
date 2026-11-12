// Synthetic child protocol only. No SDK import, network or account activity.
const requestId = "00000000-0000-4000-8000-000000000011";
const queryId = "00000000-0000-4000-8000-000000000012";
const foreignId = "00000000-0000-4000-8000-000000000013";
const context = { database: 'Quoted "DB', schema: "Mixed.Schema", role: null, warehouse: "WH" };
let input = Buffer.alloc(0);
let scenario = "healthy";
let active;
let emittedRows = 0;
let lastId = 0;
const send = (value) => {
  const body = Buffer.from(JSON.stringify(value));
  const prefix = Buffer.alloc(4);
  prefix.writeUInt32BE(body.length);
  process.stdout.write(Buffer.concat([prefix, body]));
};
const event = (request, type, extra = {}) =>
  send({ v: 1, generation: request.generation, id: request.id, type, ...extra });
const done = (request, result, identity = {}) =>
  event(request, "terminal", { ok: true, result, ...identity });
const identity = () => ({ sdkRequestId: requestId, queryId });
const rows = () => [
  ["9007199254740993", null, "first"],
  ["2", 'quoted " cell', "second"],
];
const result = (truncated = false) => ({ rowsSeen: 2, rowsRetained: 2, truncated, ...identity() });
const eof = () => process.stdout.end(() => process.exit(0));
send({
  v: 1,
  type: "hello",
  protocol: "joinery-snowflake",
  sdkVersion: "3.4.1",
  limits: { frameBytes: 1048576, batchRows: 128, batchBytes: 262144, queued: 8, credits: 32 },
});

function receive(request) {
  if (Number(request.id) <= lastId) throw new Error("nonmonotonic wire identity");
  lastId = Number(request.id);
  event(request, "accepted");
  if (request.op === "connect") {
    scenario = request.payload.scenario;
    done(request, { connected: true });
  } else if (request.op === "reviewedSql") {
    if (request.payload.sql !== "SHOW TABLES" || request.payload.credits < 1)
      throw new Error("fixture request");
    if (scenario === "before_ack_eof") return eof();
    event(request, "phase", { phase: "dispatching" });
    if (scenario !== "unregistered_ack") event(request, "statement", { sdkRequestId: requestId });
    if (scenario === "early_phase")
      return event(request, "phase", { phase: "acknowledged", sdkRequestId: requestId });
    const acknowledgement = {
      sdkRequestId: scenario === "foreign_ack" ? foreignId : requestId,
      context: scenario === "malformed_context" ? { ...context, role: 17 } : context,
    };
    event(request, "statementAcknowledged", acknowledgement);
    if (["foreign_ack", "malformed_context", "unregistered_ack"].includes(scenario)) return;
    if (scenario === "duplicate_ack")
      return event(request, "statementAcknowledged", acknowledgement);
    if (scenario === "after_ack_eof") return eof();
    if (scenario === "after_ack_protocol") return event(request, "notAProtocolEvent");
    event(request, "statement", identity());
    event(request, "phase", { phase: "acknowledged", ...identity() });
    event(request, "columns", {
      columns: [
        { name: "same", type: "TEXT" },
        { name: "same", type: "TEXT" },
        { name: "third", type: "TEXT" },
      ],
    });
    if (["held", "cancel_ack_only"].includes(scenario)) {
      active = request;
      emittedRows = 0;
      return;
    }
    event(request, "rows", { rows: rows() });
    if (scenario === "after_ack_drain_error")
      return event(request, "terminal", { ok: false, error: { code: "database" }, ...identity() });
    done(request, result(scenario === "truncated"), identity());
  } else if (request.op === "credit") {
    if (!active || request.payload.target !== active.id) throw new Error("foreign control");
    const original = active;
    if (request.payload.discard) {
      active = undefined;
      done(request, { discarded: true });
      done(
        original,
        { rowsSeen: emittedRows, rowsRetained: emittedRows, truncated: false, ...identity() },
        identity(),
      );
    } else {
      for (let index = 0; index < request.payload.batches; index++) {
        event(original, "rows", { rows: rows() });
        emittedRows += 2;
      }
      done(request, { credited: true });
    }
  } else if (request.op === "cancel") {
    if (!active || request.payload.target !== active.id) throw new Error("foreign cancellation");
    done(request, { cancelRequested: true });
    if (scenario !== "cancel_ack_only") {
      const original = active;
      active = undefined;
      event(original, "terminal", { ok: false, error: { code: "cancelled" }, ...identity() });
    }
  } else if (request.op === "close") {
    done(request, { closed: true, logoutAcknowledged: true });
    eof();
  } else {
    throw new Error("unexpected operation");
  }
}
process.stdin.on("data", (chunk) => {
  input = Buffer.concat([input, chunk]);
  while (input.length >= 4 && input.length >= input.readUInt32BE(0) + 4) {
    const size = input.readUInt32BE(0);
    const request = JSON.parse(input.subarray(4, size + 4).toString("utf8"));
    input = input.subarray(size + 4);
    receive(request);
  }
});
