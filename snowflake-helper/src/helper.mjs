import { deriveSetupLedger } from "./disposable-setup-ledger.mjs";
import { SQL_CONTRACT } from "./sql-contract.mjs";
import {
  executeMutationStep,
  mutationInterruptedSummary,
  mutationOwnerAdmits,
  runMutationRequest,
  validateMutationRequest,
} from "./mutation-batch.mjs";
import {
  validateMutationReadRequest,
  mutationReadOwnerAdmits,
  runMutationRead,
} from "./mutation-read.mjs";

export function connectionOptions(profile, clientConfigFile) {
  const { reviewedConsoleTimeoutSeconds, ...identity } = profile;
  if (
    reviewedConsoleTimeoutSeconds !== undefined &&
    (!Number.isInteger(reviewedConsoleTimeoutSeconds) ||
      reviewedConsoleTimeoutSeconds < 1 ||
      reviewedConsoleTimeoutSeconds > 86400)
  )
    throw new RangeError("Invalid reviewed console timeout");
  return {
    ...identity,
    // Connection-wide HTTP budget, not a server statement timeout. Keep the
    // existing SDK 90s floor for authentication/bootstrap/control and allow the
    // native unit deadline to expire first. Never accept arbitrary SDK options.
    ...(reviewedConsoleTimeoutSeconds === undefined
      ? {}
      : {
          timeout: Math.max(90000, reviewedConsoleTimeoutSeconds * 1000 + 5000),
        }),
    authenticator: "EXTERNALBROWSER",
    rowMode: "array",
    fetchAsString: ["Number", "Date", "Boolean", "Buffer", "JSON"],
    representNullAsStringNull: false,
    streamResult: true,
    clientStoreTemporaryCredential: false,
    clientRequestMFAToken: false,
    clientSessionKeepAlive: false,
    serverSessionKeepAlive: false,
    browserActionTimeout: 120000,
    retryTimeout: 0,
    // Only strict scalar binds enter execute. A positive threshold prevents the
    // SDK's count-zero path from selecting implicit staged-binding setup SQL.
    arrayBindingThreshold: 1_000_000,
    clientConfigFile: clientConfigFile,
  };
}
export const LIMITS = Object.freeze({
  frameBytes: 1048576,
  batchRows: 128,
  batchBytes: 262144,
  queued: 8,
  credits: 32,
});
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SDK_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const WIRE_ID = /^[1-9][0-9]{0,19}$/;
const IDENT = /^[A-Z_][A-Z0-9_]{0,254}$/;
const own = (p, keys) =>
  p && typeof p === "object" && !Array.isArray(p) && Object.keys(p).every((k) => keys.includes(k));
const exact = (p, keys) => own(p, keys) && Object.keys(p).length === keys.length;
const text = (s, max = 256) =>
  typeof s === "string" && s.length > 0 && Buffer.byteLength(s) <= max && !s.includes("\0");
const identifier = (s) => text(s, 1020) && s.isWellFormed() && [...s].length <= 255;
const nullableIdentifier = (s) => s === null || identifier(s);
const quoteIdentifier = (s) => `"${s.replaceAll('"', '""')}"`;
const CONTEXT_SQL = SQL_CONTRACT.bootstrapContext;
const TRANSACTIONS = Object.freeze({
  begin: SQL_CONTRACT.begin,
  commit: SQL_CONTRACT.commit,
  rollback: SQL_CONTRACT.rollback,
});
const DML_KINDS = new Set(["insert", "update", "delete"]);
const DDL_KINDS = new Set([
  "createTable",
  "addColumn",
  "alterColumnNullability",
  "renameColumn",
  "dropColumn",
  "dropTable",
  "createView",
  "dropView",
]);
const EXECUTE_FETCH_AS_STRING = Object.freeze([
  "Number",
  "Date",
  "Boolean",
  "Buffer",
  "JSON",
  "Object",
  "Array",
  "Map",
]);
export const CANONICAL_QUERY_PARAMETERS = Object.freeze({
  DATE_OUTPUT_FORMAT: "YYYY-MM-DD",
  TIME_OUTPUT_FORMAT: "HH24:MI:SS.FF9",
  TIMESTAMP_NTZ_OUTPUT_FORMAT: "YYYY-MM-DD HH24:MI:SS.FF9",
  TIMESTAMP_LTZ_OUTPUT_FORMAT: "YYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM",
  TIMESTAMP_TZ_OUTPUT_FORMAT: "YYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM",
  BINARY_OUTPUT_FORMAT: "HEX",
  STRICT_JSON_OUTPUT: true,
});
const integer = (n, min, max) => Number.isInteger(n) && n >= min && n <= max;
const sdkFailure = (failure, acknowledged = false) => {
  const state = failure?.sqlState;
  if (typeof state === "string" && /^[A-Z0-9]{5}$/.test(state) && !state.startsWith("08"))
    return { code: "database" };
  return {
    code: acknowledged ? "connectionClosed" : "outcomeUnknown",
    retire: true,
  };
};
const error = (code) => ({
  code,
  message: {
    invalidInput: "Invalid helper request",
    busy: "Console queue is full",
    unsupported: "Unsupported helper operation",
    connectionClosed: "Snowflake helper connection closed",
    cancelled: "Snowflake operation cancelled",
    database: "Snowflake operation failed",
    outcomeUnknown: "Remote statement outcome is unknown",
    resultLimitReached: "Unsupported or oversized result value",
  }[code],
});
export class Helper {
  constructor(sdk, emit, retire, options = {}) {
    this.sdk = sdk;
    this.emit = emit;
    this.retire = retire;
    this.clientConfigFile = options.clientConfigFile;
    // Direct synthetic tests may omit this seam; production main always injects
    // the mandatory process-local guard after verifying the pinned sources.
    this.submitGuard = options.submitGuard;
    // Production injects the exact source-verified SDK predicates after isolation.
    // Without them the new database-only operation remains unavailable.
    this.isFileTransfer = options.isFileTransfer;
    this.closeTimeoutMs = options.closeTimeoutMs ?? 1000;
    this.generation = null;
    this.lastId = 0n;
    this.done = new WeakSet();
    this.queue = [];
    this.active = null;
    this.state = "new";
    this.connection = null;
    this.retired = false;
    this.setupBootstrap = 0;
    this.setupRun = null;
    this.mutationOwner = null;
    this.mutationReadOwners = new WeakMap();
    emit({
      v: 1,
      type: "hello",
      protocol: "joinery-snowflake",
      sdkVersion: "3.4.1",
      limits: LIMITS,
    });
  }
  send(request, type, rest = {}) {
    if (!this.retired)
      return this.emit({
        v: 1,
        generation: request.generation,
        id: request.id,
        type,
        ...rest,
      });
  }
  finish(request, code, result = {}) {
    if (this.done.has(request)) return;
    const mutationActive =
      this.active?.request === request && ["mutationBatch", "mutationFinish"].includes(request.op);
    if (mutationActive && code) {
      result = mutationInterruptedSummary(
        this.active,
        code === "cancelled" ? "cancelled" : "transport",
      );
      this.active.batch && (this.active.batch.terminal = result);
      code = null;
    }
    this.done.add(request);
    if (
      !this.setupRun &&
      [
        "context",
        "query",
        "switchSchema",
        "switchContext",
        "transaction",
        "dml",
        "ddl",
        "reviewedSql",
        "mutationBatch",
        "mutationFinish",
        "mutationRead",
      ].includes(request.op)
    ) {
      const p = request.payload;
      if (!code && request.op === "context" && this.setupBootstrap === 0) this.setupBootstrap = 1;
      else if (
        !code &&
        request.op === "query" &&
        this.setupBootstrap === 1 &&
        exact(p, ["sql", "binds", "classification", "credits", "maxRows", "maxBytes"]) &&
        p.sql === SQL_CONTRACT.version &&
        p.binds.length === 0 &&
        p.classification === "read" &&
        p.credits === 1 &&
        p.maxRows === 1 &&
        p.maxBytes === 65536 &&
        result.rowsSeen === 1 &&
        result.rowsRetained === 1 &&
        result.truncated === false
      )
        this.setupBootstrap = 2;
      else this.setupBootstrap = -1;
    }
    const a = this.active?.request === request ? this.active : null;
    if (a) this.publishStatementAcknowledged(a);
    const publication = this.send(request, "terminal", {
      ...(a ? this.idsFor(a) : {}),
      ...(code ? { ok: false, error: error(code) } : { ok: true, result }),
    });
    if (mutationActive) Promise.resolve(publication).catch(() => this.stop());
    // Once setup owns the session, even malformed control requests seal it.
    if (code && this.setupRun && !this.retired) this.stop();
  }
  submissionViolation() {
    // A violation is a protocol failure even if a native throw is swallowed by
    // a dependency. Preserve any established terminal; never infer delivery.
    if (this.active)
      this.finish(
        this.active.request,
        (this.active.useAcknowledged ||
        this.active.boundaryAcknowledged ||
        this.active.dmlAcknowledged ||
        this.active.ddlAcknowledged ||
        this.active.statementAcknowledged ||
        this.active.setupAcknowledged
          ? "connectionClosed"
          : undefined) ??
          this.active.knownFailure ??
          (this.active.acknowledged ? "connectionClosed" : "outcomeUnknown"),
      );
    if (this.state === "connecting" && this.connectRequest)
      this.finish(this.connectRequest, "outcomeUnknown");
    for (const queued of this.queue.splice(0)) this.finish(queued, "connectionClosed");
    this.stop();
  }
  fatal() {
    if (!this.retired) {
      this.retired = true;
      this.wake?.();
      this.retire();
    }
  }
  receive(r) {
    if (this.retired) return;
    if (
      !own(r, ["v", "generation", "id", "op", "payload"]) ||
      r.v !== 1 ||
      !UUID.test(r.generation) ||
      typeof r.id !== "string" ||
      !WIRE_ID.test(r.id) ||
      BigInt(r.id) > 18446744073709551615n ||
      !text(r.op, 32) ||
      !r.payload ||
      typeof r.payload !== "object" ||
      Array.isArray(r.payload)
    )
      return this.fatal();
    // Duplicate IDs cannot safely be answered with another terminal of that ID.
    if (BigInt(r.id) <= this.lastId || (this.generation && r.generation !== this.generation))
      return this.fatal();
    this.lastId = BigInt(r.id);
    if (!this.generation) {
      if (r.op !== "connect") return this.fatal();
      this.generation = r.generation;
    }
    if (this.setupRun && !["setup", "close", "cancel"].includes(r.op)) return this.rejectSetup(r);
    if (r.op === "connect") return this.connect(r);
    if (r.op === "close") return this.close(r);
    if (r.op === "cancel") return this.cancel(r);
    if (r.op === "credit") return this.credit(r);
    if (
      ![
        "query",
        "context",
        "switchSchema",
        "switchContext",
        "transaction",
        "dml",
        "ddl",
        "reviewedSql",
        "setup",
        "mutationBatch",
        "mutationFinish",
        "mutationRead",
      ].includes(r.op)
    )
      return this.finish(r, "unsupported");
    if (this.state !== "connected") {
      this.finish(r, "connectionClosed");
      if (r.op === "setup") this.stop();
      return;
    }
    // The offline owner must wait for each durable terminal before advancing.
    // No queued or pipelined request can reserve a future setup statement.
    if (r.op === "setup" && (this.active || this.queue.length)) return this.rejectSetup(r);
    if (!this.validateWork(r)) return;
    if (r.op === "mutationRead") {
      if (!mutationReadOwnerAdmits(this, r)) return this.finish(r, "invalidInput");
      this.mutationReadOwners.set(r, this.mutationOwner);
    }
    if (!mutationOwnerAdmits(this, r)) return this.finish(r, "invalidInput");
    if (r.op === "setup" && !this.setupRun)
      this.setupRun = {
        runId: r.payload.runId,
        next: 3,
        ledger: deriveSetupLedger(r.payload.runId),
      };
    if (this.queue.length >= LIMITS.queued) return this.finish(r, "busy");
    this.send(r, "accepted");
    this.queue.push(r);
    this.pump();
  }
  connect(r) {
    if (this.state !== "new") return this.finish(r, "invalidInput");
    const p = r.payload;
    if (
      !own(p, [
        "account",
        "host",
        "username",
        "role",
        "warehouse",
        "database",
        "schema",
        "reviewedConsoleTimeoutSeconds",
      ]) ||
      (Object.hasOwn(p, "reviewedConsoleTimeoutSeconds") &&
        !integer(p.reviewedConsoleTimeoutSeconds, 1, 86400)) ||
      !text(p.account) ||
      !/^[a-zA-Z0-9_.-]+$/.test(p.account) ||
      !text(p.host, 253) ||
      !/^[a-zA-Z0-9.-]+\.snowflakecomputing\.com$/.test(p.host) ||
      !text(p.username) ||
      !["role", "warehouse", "database", "schema"].every((k) => IDENT.test(p[k]))
    )
      return this.finish(r, "invalidInput");
    this.expectedProfile = { ...p };
    this.currentDatabase = p.database;
    this.currentSchema = p.schema;
    this.state = "connecting";
    this.connectRequest = r;
    this.send(r, "accepted");
    try {
      this.submitGuard?.setAuthority({ protocol: "https:", hostname: p.host, port: 443 });
      this.connection = this.sdk.createConnection(connectionOptions(p, this.clientConfigFile));
      this.connection
        .connectAsync((err) => {
          if (this.retired || this.state !== "connecting") return;
          if (err) {
            this.state = "failed";
            this.finish(r, "database");
          } else {
            this.state = "connected";
            this.finish(r, null, { connected: true });
          }
        })
        .catch(() => {
          if (this.state === "connecting" && !this.retired) {
            this.state = "failed";
            this.finish(r, "database");
          }
        });
    } catch {
      this.state = "failed";
      this.finish(r, "database");
    }
  }
  activeDatabase() {
    return this.currentDatabase !== undefined
      ? this.currentDatabase
      : this.expectedProfile?.database;
  }
  validateWork(r) {
    const p = r.payload;
    if (r.op === "mutationRead") {
      if (!validateMutationReadRequest(r, this)) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "mutationBatch" || r.op === "mutationFinish") {
      if (!validateMutationRequest(r, this)) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "reviewedSql") {
      if (
        !exact(p, ["sql", "binds", "expected", "credits", "maxRows", "maxBytes"]) ||
        !text(p.sql, 262144) ||
        !p.sql.isWellFormed() ||
        !Array.isArray(p.binds) ||
        p.binds.length > 1000 ||
        !p.binds.every(
          (x) =>
            x === null ||
            typeof x === "boolean" ||
            (typeof x === "string" && Buffer.byteLength(x) <= 262144 && x.isWellFormed()),
        ) ||
        !exact(p.expected, ["user", "session"]) ||
        !identifier(p.expected.user) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session ||
        !integer(p.credits, 1, 32) ||
        !integer(p.maxRows, 0, 1000000) ||
        !integer(p.maxBytes, 0, 1099511627776) ||
        typeof this.isFileTransfer !== "function"
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      try {
        if (this.isFileTransfer(p.sql)) {
          this.finish(r, "unsupported");
          return false;
        }
      } catch {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "setup") {
      let ledger;
      try {
        ledger = this.setupRun?.ledger ?? deriveSetupLedger(p.runId);
      } catch {
        return this.rejectSetup(r);
      }
      if (
        !exact(p, ["runId", "step", "expected"]) ||
        !integer(p.step, 3, 28) ||
        this.setupBootstrap !== 2 ||
        !exact(p.expected, ["user", "session"]) ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session ||
        (this.setupRun && p.runId !== this.setupRun.runId) ||
        p.step !== (this.setupRun?.next ?? 3) ||
        ledger[p.step - 1]?.index !== p.step
      )
        return this.rejectSetup(r);
      return true;
    }
    if (r.op === "dml") {
      // Trusted Rust admission owns the SQL grammar and canonical bindings.
      // This transport check is deliberately not a SQL classification claim.
      if (
        !exact(p, ["kind", "sql", "binds", "expected"]) ||
        typeof p.kind !== "string" ||
        !DML_KINDS.has(p.kind) ||
        !text(p.sql, 262144) ||
        !p.sql.isWellFormed() ||
        !Array.isArray(p.binds) ||
        p.binds.length > 1000 ||
        !p.binds.every(
          (x) =>
            x === null ||
            typeof x === "boolean" ||
            (typeof x === "string" && Buffer.byteLength(x) <= 262144 && x.isWellFormed()),
        ) ||
        !exact(p.expected, ["user", "session"]) ||
        !identifier(p.expected.user) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "ddl") {
      // Trusted Rust admission owns the generated DDL grammar and canonical target.
      // This transport check requires a closed parameter-free payload and pinned session.
      if (
        !exact(p, ["kind", "sql", "expected"]) ||
        typeof p.kind !== "string" ||
        !DDL_KINDS.has(p.kind) ||
        !text(p.sql, 262144) ||
        !p.sql.isWellFormed() ||
        !exact(p.expected, ["user", "session"]) ||
        !identifier(p.expected.user) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "transaction") {
      if (
        !exact(p, ["action", "expected"]) ||
        typeof p.action !== "string" ||
        !Object.hasOwn(TRANSACTIONS, p.action) ||
        !exact(p.expected, ["user", "session"]) ||
        !identifier(p.expected.user) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "context") {
      if (!exact(p, [])) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "switchSchema") {
      if (
        !exact(p, ["database", "schema", "expected"]) ||
        !identifier(p.database) ||
        !identifier(p.schema) ||
        !exact(p.expected, ["user", "role", "warehouse", "session"]) ||
        !identifier(p.expected.user) ||
        !nullableIdentifier(p.expected.role) ||
        !nullableIdentifier(p.expected.warehouse) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        p.database !== this.activeDatabase() ||
        (this.identityPin &&
          (p.expected.user !== this.identityPin.user ||
            p.expected.session !== this.identityPin.session))
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (r.op === "switchContext") {
      if (
        !p ||
        typeof p !== "object" ||
        Array.isArray(p) ||
        typeof p.kind !== "string" ||
        !["role", "warehouse", "database", "schema"].includes(p.kind) ||
        !exact(p.expected, ["user", "role", "database", "schema", "warehouse", "session"]) ||
        !identifier(p.expected.user) ||
        !nullableIdentifier(p.expected.role) ||
        !nullableIdentifier(p.expected.database) ||
        !nullableIdentifier(p.expected.schema) ||
        !nullableIdentifier(p.expected.warehouse) ||
        !text(p.expected.session, 1024) ||
        !p.expected.session.isWellFormed() ||
        !this.identityPin ||
        p.expected.user !== this.identityPin.user ||
        p.expected.session !== this.identityPin.session
      ) {
        this.finish(r, "invalidInput");
        return false;
      }
      const validTarget =
        (p.kind === "role" && exact(p, ["kind", "role", "expected"]) && identifier(p.role)) ||
        (p.kind === "warehouse" &&
          exact(p, ["kind", "warehouse", "expected"]) &&
          identifier(p.warehouse)) ||
        (p.kind === "database" &&
          exact(p, ["kind", "database", "schema", "expected"]) &&
          identifier(p.database) &&
          nullableIdentifier(p.schema)) ||
        (p.kind === "schema" &&
          exact(p, ["kind", "database", "schema", "expected"]) &&
          identifier(p.database) &&
          identifier(p.schema) &&
          p.database === this.activeDatabase());
      if (!validTarget) {
        this.finish(r, "invalidInput");
        return false;
      }
      return true;
    }
    if (p.classification !== "read") {
      this.finish(r, "unsupported");
      return false;
    }
    if (
      !own(p, ["sql", "binds", "classification", "credits", "maxRows", "maxBytes"]) ||
      !text(p.sql, 262144) ||
      !Array.isArray(p.binds) ||
      p.binds.length > 1000 ||
      !p.binds.every(
        (x) =>
          x === null ||
          typeof x === "boolean" ||
          (typeof x === "string" && Buffer.byteLength(x) <= 262144),
      ) ||
      !integer(p.credits, 0, 32) ||
      !integer(p.maxRows, 0, 1000000) ||
      !integer(p.maxBytes, 0, 1099511627776)
    ) {
      this.finish(r, "invalidInput");
      return false;
    }
    return true;
  }
  rejectSetup(r) {
    this.finish(r, "invalidInput");
    this.stop();
    return false;
  }
  async pump() {
    if (this.active || this.retired) return;
    const r = this.queue.shift();
    if (!r) return;
    if (
      !mutationOwnerAdmits(this, r) ||
      (r.op === "mutationRead" && !mutationReadOwnerAdmits(this, r, this.mutationReadOwners.get(r)))
    ) {
      this.finish(r, "invalidInput");
      this.pump();
      return;
    }
    const a = (this.active = {
      request: r,
      credits: r.payload.credits ?? 0,
      discard: false,
      statement: null,
      stream: null,
      dispatching: false,
      statements: new Map(),
      mutationReadOwner: this.mutationReadOwners.get(r),
    });
    try {
      a.submitOwner = this.submitGuard?.beginOperation();
      if (r.op === "mutationBatch" || r.op === "mutationFinish") {
        if (!this.validateWork(r)) return;
        await runMutationRequest(this, a);
      } else if (r.op === "mutationRead") {
        await runMutationRead(this, a);
      } else if (r.op === "query" || r.op === "reviewedSql") {
        if (r.op === "reviewedSql" && !this.validateWork(r)) return;
        const result = await this.execute(a, r.payload.sql, r.payload.binds, true);
        this.finish(r, null, { ...result, ...this.idsFor(a) });
      } else if (r.op === "setup") {
        if (!this.validateWork(r)) return;
        const entry = this.setupRun.ledger[r.payload.step - 1];
        a.setupEntry = entry;
        // Consume before phase publication, SDK execution, or any callback.
        this.setupRun.next++;
        this.send(r, "phase", { phase: "dispatching" });
        const result = await this.execute(
          a,
          entry.sql,
          [],
          false,
          false,
          null,
          null,
          entry.mutation ? { runId: r.payload.runId, step: entry.index, key: entry.key } : null,
        );
        if (this.retired || this.done.has(r)) return;
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, {
          runId: r.payload.runId,
          step: entry.index,
          key: entry.key,
          phase: entry.phase,
          mutation: entry.mutation,
          ...result,
          ...this.idsFor(a),
        });
      } else if (r.op === "dml") {
        if (!this.validateWork(r)) return;
        this.send(r, "phase", { phase: "dispatching" });
        const result = await this.execute(
          a,
          r.payload.sql,
          r.payload.binds,
          false,
          false,
          null,
          r.payload.kind,
        );
        if (this.retired || this.done.has(r)) return;
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, { kind: r.payload.kind, ...result, ...this.idsFor(a) });
      } else if (r.op === "ddl") {
        if (!this.validateWork(r)) return;
        this.send(r, "phase", { phase: "dispatching" });
        const result = await this.execute(
          a,
          r.payload.sql,
          [],
          false,
          false,
          null,
          null,
          null,
          r.payload.kind,
        );
        if (this.retired || this.done.has(r)) return;
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, { kind: r.payload.kind, ...result, ...this.idsFor(a) });
      } else if (r.op === "transaction") {
        // Recheck at dispatch too: a queued operation never adopts another pin.
        if (!this.validateWork(r)) return;
        this.send(r, "phase", { phase: "dispatching" });
        await this.execute(a, TRANSACTIONS[r.payload.action], [], false, false, r.payload.action);
        if (this.retired || this.done.has(r)) return;
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, { action: r.payload.action, ...this.idsFor(a) });
      } else {
        if (r.op === "switchContext" && !this.validateWork(r)) return;
        this.send(r, "phase", { phase: "dispatching" });
        if (r.op === "switchSchema")
          await this.execute(
            a,
            `USE SCHEMA ${quoteIdentifier(r.payload.database)}.${quoteIdentifier(r.payload.schema)}`,
            [],
            false,
            true,
          );
        else if (r.op === "switchContext") {
          const p = r.payload;
          const useSql =
            p.kind === "role"
              ? `USE ROLE ${quoteIdentifier(p.role)}`
              : p.kind === "warehouse"
                ? `USE WAREHOUSE ${quoteIdentifier(p.warehouse)}`
                : p.kind === "database"
                  ? p.schema !== null
                    ? `USE SCHEMA ${quoteIdentifier(p.database)}.${quoteIdentifier(p.schema)}`
                    : `USE DATABASE ${quoteIdentifier(p.database)}`
                  : `USE SCHEMA ${quoteIdentifier(p.database)}.${quoteIdentifier(p.schema)}`;
          await this.execute(a, useSql, [], false, true);
        }
        if (this.retired || this.done.has(r)) return;
        const result = await this.execute(a, CONTEXT_SQL, [], false);
        const row = result.values?.[0];
        if (
          result.rowsSeen !== 1 ||
          result.values?.length !== 1 ||
          result.columns?.length !== 6 ||
          result.columns.some((c) => c.type.toLowerCase() !== "text") ||
          !row ||
          row.length !== 6 ||
          !identifier(row[0]) ||
          !nullableIdentifier(row[1]) ||
          !nullableIdentifier(row[2]) ||
          !nullableIdentifier(row[3]) ||
          !nullableIdentifier(row[4]) ||
          !text(row[5], 1024) ||
          !row[5].isWellFormed()
        )
          throw error("database");
        if (
          this.identityPin &&
          (row[0] !== this.identityPin.user || row[5] !== this.identityPin.session)
        )
          throw error("database");
        if (
          r.op === "switchSchema" &&
          (row[0] !== r.payload.expected.user ||
            row[1] !== r.payload.expected.role ||
            row[2] !== r.payload.database ||
            row[3] !== r.payload.schema ||
            row[4] !== r.payload.expected.warehouse ||
            row[5] !== r.payload.expected.session)
        )
          throw error("database");
        if (r.op === "switchContext") {
          const p = r.payload;
          if (row[0] !== p.expected.user || row[5] !== p.expected.session) throw error("database");
          if (p.kind === "role" && row[1] !== p.role) throw error("database");
          if (
            p.kind === "warehouse" &&
            (row[1] !== p.expected.role ||
              row[2] !== p.expected.database ||
              row[3] !== p.expected.schema ||
              row[4] !== p.warehouse)
          )
            throw error("database");
          if (
            p.kind === "database" &&
            (row[1] !== p.expected.role ||
              row[2] !== p.database ||
              (p.schema !== null && row[3] !== p.schema) ||
              row[4] !== p.expected.warehouse)
          )
            throw error("database");
          if (
            p.kind === "schema" &&
            (row[1] !== p.expected.role ||
              row[2] !== p.database ||
              row[3] !== p.schema ||
              row[4] !== p.expected.warehouse)
          )
            throw error("database");
          this.currentDatabase = row[2];
        }
        this.identityPin ??= { user: row[0], session: row[5] };
        this.currentSchema = row[3];
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, {
          ...Object.fromEntries(
            ["user", "role", "database", "schema", "warehouse", "session"].map((k, i) => [
              k,
              row[i],
            ]),
          ),
          ...this.idsFor(a),
        });
      }
    } catch (e) {
      this.finish(
        r,
        a.useAcknowledged ||
          a.boundaryAcknowledged ||
          a.dmlAcknowledged ||
          a.ddlAcknowledged ||
          a.statementAcknowledged ||
          a.setupAcknowledged
          ? "connectionClosed"
          : ["resultLimitReached", "outcomeUnknown", "connectionClosed", "database"].includes(
                e?.code,
              )
            ? e.code
            : "database",
      );
      if (
        e?.retire ||
        r.op === "setup" ||
        r.op === "context" ||
        r.op === "switchSchema" ||
        (r.op === "switchContext" && (a.useAcknowledged || !a.knownFailure)) ||
        a.boundaryAcknowledged ||
        a.dmlAcknowledged ||
        a.ddlAcknowledged ||
        (r.op === "reviewedSql" && (a.statementAcknowledged || (a.dispatching && !a.knownFailure)))
      )
        this.stop();
    } finally {
      if (a.submitOwner) {
        try {
          this.submitGuard.endOperation(a.submitOwner);
        } catch {
          this.submissionViolation();
        }
      }
      if (this.active === a) this.active = null;
      this.wake?.();
      this.pump();
    }
  }
  idsFor(a) {
    if (!a?.lastIdentity) return {};
    return { ...a.lastIdentity };
  }
  publishStatementAcknowledged(a) {
    if (!a.statementAcknowledged || a.statementAcknowledgedPublished) return;
    a.statementAcknowledgedPublished = true;
    // Copy before publication: getters may reenter close/retirement, and an
    // already emitted observation must never acquire fields retrospectively.
    this.send(a.request, "statementAcknowledged", {
      ...a.statementAcknowledged,
      context: { ...a.statementAcknowledged.context },
    });
  }
  captureReviewedContext(a, statement) {
    let state;
    try {
      state = statement.getSessionState?.();
    } catch {
      return;
    }
    for (const [key, getter] of [
      ["database", "getCurrentDatabase"],
      ["schema", "getCurrentSchema"],
      ["role", "getCurrentRole"],
      ["warehouse", "getCurrentWarehouse"],
    ]) {
      if (a.statementAcknowledgedPublished || this.done.has(a.request)) return;
      try {
        const value = state?.[getter]?.();
        if (!a.statementAcknowledgedPublished && !this.done.has(a.request) && identifier(value))
          a.statementAcknowledged.context[key] = value;
      } catch {
        /* Missing/invalid passive observations remain individually unknown. */
      }
    }
  }
  record(a, statement) {
    if (!statement) return;
    let sdkRequestId, queryId;
    try {
      sdkRequestId = statement.getRequestId?.();
      queryId = statement.getQueryId?.();
    } catch {
      return;
    }
    if (typeof sdkRequestId !== "string" || !SDK_UUID.test(sdkRequestId)) return;
    if (
      queryId !== undefined &&
      queryId !== null &&
      queryId !== "" &&
      (typeof queryId !== "string" || !SDK_UUID.test(queryId))
    )
      return;
    queryId = queryId || undefined;
    const prior = a.statements.get(sdkRequestId);
    if (prior?.queryId && queryId && prior.queryId !== queryId) {
      this.fatal();
      return;
    }
    if (a.statements.size >= 2 && !prior) {
      this.fatal();
      return;
    }
    const identity = {
      sdkRequestId,
      ...(queryId ? { queryId } : prior?.queryId ? { queryId: prior.queryId } : {}),
    };
    a.lastIdentity = identity;
    if (prior && prior.queryId === identity.queryId) return;
    a.statements.set(sdkRequestId, identity);
    this.send(a.request, "statement", identity);
  }
  execute(
    a,
    sql,
    binds,
    publish,
    use = false,
    boundary = null,
    dml = null,
    setup = null,
    ddl = null,
  ) {
    if (a.batch) return executeMutationStep(this, a, sql, binds);
    // This is per execute: a preceding USE acknowledgement cannot settle the
    // following context readback's outcome before that statement acknowledges.
    a.acknowledged = false;
    a.knownFailure = undefined;
    // A phase/event callback may close the request while SDK logout remains
    // pending. Retirement alone is too late to prevent that settled dispatch.
    if (this.retired || this.done.has(a.request) || this.state !== "connected")
      return Promise.reject(error("connectionClosed"));
    if (a.request.op === "reviewedSql") {
      try {
        if (typeof this.isFileTransfer !== "function") return Promise.reject(error("invalidInput"));
        if (this.isFileTransfer(sql)) return Promise.reject(error("unsupported"));
      } catch {
        return Promise.reject(error("invalidInput"));
      }
    }
    const ticket = this.submitGuard?.beginStatement(a.submitOwner);
    const task = new Promise((resolve, reject) => {
      if (this.retired || this.done.has(a.request) || this.state !== "connected")
        return reject(error("connectionClosed"));
      a.dispatching = true;
      if (publish) this.send(a.request, "phase", { phase: "dispatching" });
      let returned = false,
        pending,
        completed = false;
      const complete = (err, statement) => {
        // Actual SDK dispatch is deferred. Synthetic callbacks can be
        // synchronous; never consume/complete them before exact-ID registration.
        if (!returned) {
          pending = [err, statement];
          return;
        }
        if (completed) return;
        completed = true;
        if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
        if (err) {
          const failure = sdkFailure(err);
          // Preserve only the existing definitive database classification, before
          // record/publication can cause a reentrant submission violation.
          if (failure.code === "database") a.knownFailure = failure.code;
          this.record(a, statement ?? a.statement);
          return reject(failure);
        }
        a.acknowledged = true;
        try {
          if (a.request.op === "reviewedSql") {
            // Only the exact identity already recorded from execute's return is
            // authoritative. Establish success before any passive SDK getter.
            if (statement !== a.statement || !a.lastIdentity || a.statements.size !== 1) {
              return reject({ code: "outcomeUnknown", retire: true });
            }
            a.statementAcknowledged = {
              ...this.idsFor(a),
              context: { database: null, schema: null, role: null, warehouse: null },
            };
            this.captureReviewedContext(a, statement);
            this.publishStatementAcknowledged(a);
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          if (setup) {
            a.setupAcknowledged = setup;
            this.send(a.request, "setupAcknowledged", { ...setup, ...this.idsFor(a) });
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          if (dml) {
            a.dmlAcknowledged = dml;
            this.send(a.request, "dmlAcknowledged", { kind: dml, ...this.idsFor(a) });
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          if (ddl) {
            a.ddlAcknowledged = ddl;
            this.send(a.request, "ddlAcknowledged", { kind: ddl, ...this.idsFor(a) });
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          if (boundary) {
            // Command success is distinct from verified transaction disposition.
            // Registered identity and ACK survive reentrant record/send failures.
            a.boundaryAcknowledged = boundary;
            this.send(a.request, "boundaryAcknowledged", { action: boundary, ...this.idsFor(a) });
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          // Operation evidence survives the following readback's per-execute reset.
          // Establish it before record/send can reenter submissionViolation().
          if (use) {
            a.useAcknowledged = true;
            // This ID was registered and recorded from execute's return value.
            // Publish ACK before record's query-ID upgrade can reenter the guard.
            this.send(a.request, "useAcknowledged", this.idsFor(a));
            if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          }
          a.statement = statement;
          this.record(a, statement);
          if (this.retired || this.done.has(a.request)) return reject(error("connectionClosed"));
          if (publish)
            this.send(a.request, "phase", {
              phase: "acknowledged",
              ...this.idsFor(a),
            });
          this.consume(a, statement, publish).then(resolve, (e) =>
            reject(e?.code === "resultLimitReached" ? e : sdkFailure(e, true)),
          );
        } catch (failure) {
          reject(failure);
        }
      };
      try {
        if (this.retired || this.done.has(a.request) || this.state !== "connected")
          return reject(error("connectionClosed"));
        if (
          a.request.op === "mutationRead" &&
          !mutationReadOwnerAdmits(this, a.request, a.mutationReadOwner)
        )
          return reject(error("invalidInput"));
        a.statement = this.connection.execute({
          sqlText: sql,
          binds,
          rowMode: "array",
          fetchAsString: EXECUTE_FETCH_AS_STRING,
          streamResult: true,
          ...(a.request.op === "reviewedSql"
            ? { parameters: { MULTI_STATEMENT_COUNT: 1 } }
            : a.request.op === "mutationRead"
              ? { parameters: { MULTI_STATEMENT_COUNT: 1, ...CANONICAL_QUERY_PARAMETERS } }
              : a.request.op === "query"
                ? { parameters: CANONICAL_QUERY_PARAMETERS }
                : {}),
          complete,
        });
        // Do not infer an ID from a native URL or completion callback. Register
        // only the exact ID returned by this execute, before deferred transport.
        if (ticket) this.submitGuard.registerRequestId(ticket, a.statement?.getRequestId?.());
        this.record(a, a.statement);
        returned = true;
        if (pending) complete(...pending);
      } catch {
        returned = true;
        reject(sdkFailure(null));
      }
    });
    // The allowance stays owned throughout acknowledgement and result drain,
    // including cancel acknowledgement. The operation closes after its terminal.
    return task.finally(() => {
      if (ticket) this.submitGuard.endStatement(ticket);
    });
  }
  async consume(a, statement, publish) {
    const columns = statement.getColumns().map((c) => ({ name: c.getName(), type: c.getType() }));
    if (
      Buffer.byteLength(JSON.stringify(columns)) > LIMITS.frameBytes - 512 ||
      columns.length >
        (a.batch
          ? 16
          : (a.setupEntry?.maximumColumns ??
            (a.request.op === "dml" || a.request.op === "ddl" ? 2 : 1000))) ||
      columns.some((c) => !text(c.name, 4096) || !text(c.type, 128))
    )
      throw error("resultLimitReached");
    const privateBudget = a.setupEntry
      ? a.setupEntry.maximumBytes - Buffer.byteLength(JSON.stringify(columns)) - 2
      : a.batch || a.request.op === "dml" || a.request.op === "ddl"
        ? 65536 - Buffer.byteLength(JSON.stringify(columns))
        : 131072;
    if (a.setupEntry && privateBudget < 0) throw error("resultLimitReached");
    const readBytes = a.mutationRead ? Buffer.byteLength(JSON.stringify(columns)) + 2 : 0;
    if (a.mutationRead && readBytes > a.mutationRead.maxBytes) throw error("resultLimitReached");
    if (publish) this.send(a.request, "columns", { columns });
    const stream = (a.stream = statement.streamRows());
    const booleanColumns = columns.some((c) => c.type.toLowerCase() === "boolean")
      ? columns.map((c) => c.type.toLowerCase() === "boolean")
      : null;
    let rowsSeen = 0,
      rowsRetained = 0,
      bytes = readBytes,
      truncated = false;
    const values = [];
    let batch = [],
      batchBytes = 2,
      timer = null,
      flushing = null;
    const flush = () => {
      if (flushing) return flushing;
      clearTimeout(timer);
      timer = null;
      const pending = batch,
        size = batchBytes;
      batch = [];
      batchBytes = 2;
      if (!pending.length) return Promise.resolve();
      const task = (async () => {
        while (a.credits === 0 && !a.discard && !this.retired && !this.done.has(a.request))
          await new Promise((resolve) => {
            this.wake = resolve;
          });
        if (a.discard || this.retired || this.done.has(a.request)) {
          truncated = true;
          return;
        }
        a.credits--;
        rowsRetained += pending.length;
        bytes += size;
        await this.send(a.request, "rows", { rows: pending });
      })();
      const wrapped = task.finally(() => {
        if (flushing === wrapped) flushing = null;
      });
      flushing = wrapped;
      return wrapped;
    };
    try {
      // One bounded batch and at most one next row are held by the helper.
      // The SDK may separately buffer/decompress server chunks internally.
      for await (const row of stream) {
        if (flushing) await flushing;
        if (this.retired || this.done.has(a.request))
          return { rowsSeen, rowsRetained, truncated: true };
        rowsSeen++;
        if (
          !Array.isArray(row) ||
          row.length !== columns.length ||
          row.some((v) => v !== null && typeof v !== "string")
        )
          throw error("resultLimitReached");
        const normalizedRow = booleanColumns
          ? row.map((v, i) =>
              booleanColumns[i] ? (v === "TRUE" ? "true" : v === "FALSE" ? "false" : v) : v,
            )
          : row;
        const size = Buffer.byteLength(JSON.stringify(normalizedRow)) + 1;
        if (!publish) {
          if (
            values.length >=
              (a.batch
                ? 1
                : (a.setupEntry?.maximumRows ??
                  (a.request.op === "dml" || a.request.op === "ddl" ? 1 : 2))) ||
            size > 65536 ||
            ((a.batch || a.request.op === "dml" || a.request.op === "ddl" || a.setupEntry) &&
              bytes + size > privateBudget)
          )
            throw error("resultLimitReached");
          bytes += size;
          values.push(normalizedRow);
          continue;
        }
        const p = a.mutationRead ?? a.request.payload;
        if (
          a.discard ||
          truncated ||
          size > LIMITS.batchBytes - 128 ||
          rowsRetained + batch.length >= p.maxRows ||
          bytes + batchBytes + size > p.maxBytes
        ) {
          await flush();
          truncated = true;
          continue;
        }
        if (batch.length >= LIMITS.batchRows || batchBytes + size > LIMITS.batchBytes - 128)
          await flush();
        batch.push(normalizedRow);
        batchBytes += size;
        if (batch.length >= LIMITS.batchRows) await flush();
        else if (!timer)
          timer = setTimeout(() => {
            flush().catch(() => this.fatal());
          }, 25);
      }
      if (flushing) await flushing;
      await flush();
      return {
        rowsSeen,
        rowsRetained,
        truncated,
        ...(!publish ? { values, columns } : {}),
      };
    } finally {
      clearTimeout(timer);
    }
  }

  credit(r) {
    const p = r.payload,
      a = this.active;
    if (
      !own(p, ["target", "batches", "discard"]) ||
      !text(p.target, 128) ||
      !integer(p.batches, 0, 32) ||
      typeof p.discard !== "boolean"
    )
      return this.finish(r, "invalidInput");
    if (p.discard && this.removeQueued(r, p.target)) return;
    if (
      !a ||
      !["query", "reviewedSql", "mutationRead"].includes(a.request.op) ||
      a.request.id !== p.target ||
      a.credits + p.batches > 32
    )
      return this.finish(r, "invalidInput");
    this.send(r, "accepted");
    a.credits += p.batches;
    a.discard ||= p.discard;
    this.wake?.();
    this.finish(r, null, {});
  }
  removeQueued(control, target) {
    const index = this.queue.findIndex(
      (r) =>
        [
          "query",
          "reviewedSql",
          "switchSchema",
          "transaction",
          "dml",
          "ddl",
          "mutationBatch",
          "mutationFinish",
          "mutationRead",
        ].includes(r.op) && r.id === target,
    );
    if (index < 0) return false;
    const [queued] = this.queue.splice(index, 1);
    this.send(control, "accepted");
    this.finish(queued, "cancelled");
    this.finish(control, null, { cancelledLocally: true });
    return true;
  }
  cancel(r) {
    const p = r.payload;
    if (!own(p, ["target"]) || !text(p.target, 128)) return this.finish(r, "invalidInput");
    if (this.state === "connecting" && p.target === this.connectRequest.id) {
      this.send(r, "accepted");
      this.finish(this.connectRequest, "cancelled");
      this.finish(r, null, { requested: true });
      return this.stop();
    }
    if (this.removeQueued(r, p.target)) return;
    const a = this.active;
    if (
      a &&
      a.request.id === p.target &&
      ["mutationBatch", "mutationFinish", "mutationRead"].includes(a.request.op)
    ) {
      this.send(r, "accepted");
      if (a.batch) a.batch.terminal = mutationInterruptedSummary(a, "cancelled");
      try {
        a.statement?.cancel(() => {});
      } catch {}
      this.finish(r, null, { requested: true });
      this.finish(a.request, "cancelled");
      return this.stop();
    }
    if (!a || a.request.id !== p.target || !a.statement) return this.finish(r, "invalidInput");
    this.send(r, "accepted");
    if (
      a.request.op === "switchSchema" ||
      a.request.op === "transaction" ||
      a.request.op === "dml" ||
      a.request.op === "ddl" ||
      a.request.op === "reviewedSql" ||
      a.request.op === "setup" ||
      a.request.op === "mutationBatch" ||
      a.request.op === "mutationFinish"
    ) {
      // A cancel ACK cannot verify either the USE outcome or final context.
      try {
        a.statement.cancel(() => {});
      } catch {}
      this.finish(r, null, { requested: true });
      this.finish(
        a.request,
        a.useAcknowledged ||
          a.boundaryAcknowledged ||
          a.dmlAcknowledged ||
          a.ddlAcknowledged ||
          a.statementAcknowledged ||
          a.setupAcknowledged
          ? "connectionClosed"
          : (a.knownFailure ?? "outcomeUnknown"),
      );
      return this.stop();
    }
    a.discard = true;
    this.wake?.();
    try {
      a.statement.cancel((err) => this.finish(r, err ? "database" : null, { requested: true }));
    } catch {
      this.finish(r, "database");
    }
    // Cancel acknowledgement is NOT a query terminal. Keep listening/draining.
  }
  close(r) {
    if (!own(r.payload, [])) return this.finish(r, "invalidInput");
    this.send(r, "accepted");
    const prior = this.state;
    this.state = "closing";
    if (this.connectRequest) this.finish(this.connectRequest, "connectionClosed");
    if (this.active) {
      const a = this.active;
      this.finish(
        a.request,
        (a.request.op === "setup" && !a.setupAcknowledged) ||
          (a.request.op === "ddl" && a.dispatching && !a.ddlAcknowledged) ||
          (a.request.op === "reviewedSql" && a.dispatching && !a.statementAcknowledged)
          ? (a.knownFailure ?? "outcomeUnknown")
          : "connectionClosed",
      );
    }
    for (const queued of this.queue.splice(0)) this.finish(queued, "connectionClosed");
    this.wake?.();
    if (prior === "connecting" || !this.connection) {
      this.finish(r, null, { closed: true, logoutAcknowledged: false });
      return this.stop();
    }
    // An explicit close gives actual logout one bounded chance to complete.
    // Timeout/error means only local retirement is known, never server logout.
    let complete = false;
    const done = (err) => {
      if (complete) return;
      complete = true;
      clearTimeout(timer);
      this.finish(r, err ? "connectionClosed" : null, {
        closed: true,
        logoutAcknowledged: !err,
      });
      this.fatal();
    };
    const timer = setTimeout(() => done(true), this.closeTimeoutMs);
    try {
      this.connection.destroy(done);
    } catch {
      done(true);
    }
  }
  stop() {
    // Process retirement is the actual pending-connect cancellation boundary.
    try {
      this.connection?.destroy(() => {});
    } catch {}
    this.fatal();
  }
}
