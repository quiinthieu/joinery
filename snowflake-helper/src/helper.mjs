export function connectionOptions(profile, clientConfigFile) {
  return {
    ...profile,
    authenticator: "EXTERNALBROWSER",
    rowMode: "array",
    fetchAsString: ["Number", "Date", "Boolean", "Buffer", "JSON"],
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
const CONTEXT_SQL =
  "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_SESSION()";
const TRANSACTIONS = Object.freeze({
  begin: "BEGIN TRANSACTION",
  commit: "COMMIT",
  rollback: "ROLLBACK",
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
    unsupported: "Only caller-classified read-only queries are supported",
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
    this.closeTimeoutMs = options.closeTimeoutMs ?? 1000;
    this.generation = null;
    this.lastId = 0n;
    this.done = new WeakSet();
    this.queue = [];
    this.active = null;
    this.state = "new";
    this.connection = null;
    this.retired = false;
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
    this.done.add(request);
    const a = this.active?.request === request ? this.active : null;
    this.send(request, "terminal", {
      ...(a ? this.idsFor(a) : {}),
      ...(code ? { ok: false, error: error(code) } : { ok: true, result }),
    });
  }
  submissionViolation() {
    // A violation is a protocol failure even if a native throw is swallowed by
    // a dependency. Preserve any established terminal; never infer delivery.
    if (this.active)
      this.finish(
        this.active.request,
        (this.active.useAcknowledged || this.active.boundaryAcknowledged
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
    if (r.op === "connect") return this.connect(r);
    if (r.op === "close") return this.close(r);
    if (r.op === "cancel") return this.cancel(r);
    if (r.op === "credit") return this.credit(r);
    if (!["query", "context", "switchSchema", "transaction"].includes(r.op))
      return this.finish(r, "unsupported");
    if (this.state !== "connected") return this.finish(r, "connectionClosed");
    if (!this.validateWork(r)) return;
    if (this.queue.length >= LIMITS.queued) return this.finish(r, "busy");
    this.send(r, "accepted");
    this.queue.push(r);
    this.pump();
  }
  connect(r) {
    if (this.state !== "new") return this.finish(r, "invalidInput");
    const p = r.payload;
    if (
      !own(p, ["account", "host", "username", "role", "warehouse", "database", "schema"]) ||
      !text(p.account) ||
      !/^[a-zA-Z0-9_.-]+$/.test(p.account) ||
      !text(p.host, 253) ||
      !/^[a-zA-Z0-9.-]+\.snowflakecomputing\.com$/.test(p.host) ||
      !text(p.username) ||
      !["role", "warehouse", "database", "schema"].every((k) => IDENT.test(p[k]))
    )
      return this.finish(r, "invalidInput");
    this.expectedProfile = { ...p };
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
  validateWork(r) {
    const p = r.payload;
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
        p.database !== this.expectedProfile.database ||
        (this.identityPin &&
          (p.expected.user !== this.identityPin.user ||
            p.expected.session !== this.identityPin.session))
      ) {
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
  async pump() {
    if (this.active || this.retired) return;
    const r = this.queue.shift();
    if (!r) return;
    const a = (this.active = {
      request: r,
      credits: r.payload.credits ?? 0,
      discard: false,
      statement: null,
      stream: null,
      dispatching: false,
      statements: new Map(),
    });
    try {
      a.submitOwner = this.submitGuard?.beginOperation();
      if (r.op === "query") {
        const result = await this.execute(a, r.payload.sql, r.payload.binds, true);
        this.finish(r, null, { ...result, ...this.idsFor(a) });
      } else if (r.op === "transaction") {
        // Recheck at dispatch too: a queued operation never adopts another pin.
        if (!this.validateWork(r)) return;
        this.send(r, "phase", { phase: "dispatching" });
        await this.execute(a, TRANSACTIONS[r.payload.action], [], false, false, r.payload.action);
        if (this.retired || this.done.has(r)) return;
        this.send(r, "phase", { phase: "acknowledged", ...this.idsFor(a) });
        this.finish(r, null, { action: r.payload.action, ...this.idsFor(a) });
      } else {
        this.send(r, "phase", { phase: "dispatching" });
        if (r.op === "switchSchema")
          await this.execute(
            a,
            `USE SCHEMA ${quoteIdentifier(r.payload.database)}.${quoteIdentifier(r.payload.schema)}`,
            [],
            false,
            true,
          );
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
        a.useAcknowledged || a.boundaryAcknowledged
          ? "connectionClosed"
          : ["resultLimitReached", "outcomeUnknown", "connectionClosed", "database"].includes(
                e?.code,
              )
            ? e.code
            : "database",
      );
      if (e?.retire || r.op === "context" || r.op === "switchSchema" || a.boundaryAcknowledged)
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
  execute(a, sql, binds, publish, use = false, boundary = null) {
    // This is per execute: a preceding USE acknowledgement cannot settle the
    // following context readback's outcome before that statement acknowledges.
    a.acknowledged = false;
    a.knownFailure = undefined;
    const ticket = this.submitGuard?.beginStatement(a.submitOwner);
    const task = new Promise((resolve, reject) => {
      if (this.retired) return reject(error("connectionClosed"));
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
      };
      try {
        a.statement = this.connection.execute({
          sqlText: sql,
          binds,
          rowMode: "array",
          fetchAsString: ["Number", "Date", "Boolean", "Buffer", "JSON"],
          streamResult: true,
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
      columns.length > 1000 ||
      columns.some((c) => !text(c.name, 4096) || !text(c.type, 128))
    )
      throw error("resultLimitReached");
    if (publish) this.send(a.request, "columns", { columns });
    const stream = (a.stream = statement.streamRows());
    let rowsSeen = 0,
      rowsRetained = 0,
      bytes = 0,
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
        const size = Buffer.byteLength(JSON.stringify(row)) + 1;
        if (!publish) {
          if (values.length >= 2 || size > 65536) throw error("resultLimitReached");
          values.push(row);
          continue;
        }
        const p = a.request.payload;
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
        batch.push(row);
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
    if (!a || a.request.op !== "query" || a.request.id !== p.target || a.credits + p.batches > 32)
      return this.finish(r, "invalidInput");
    this.send(r, "accepted");
    a.credits += p.batches;
    a.discard ||= p.discard;
    this.wake?.();
    this.finish(r, null, {});
  }
  removeQueued(control, target) {
    const index = this.queue.findIndex(
      (r) => ["query", "switchSchema", "transaction"].includes(r.op) && r.id === target,
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
    if (!a || a.request.id !== p.target || !a.statement) return this.finish(r, "invalidInput");
    this.send(r, "accepted");
    if (a.request.op === "switchSchema" || a.request.op === "transaction") {
      // A cancel ACK cannot verify either the USE outcome or final context.
      try {
        a.statement.cancel(() => {});
      } catch {}
      this.finish(r, null, { requested: true });
      this.finish(
        a.request,
        a.useAcknowledged || a.boundaryAcknowledged
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
    if (this.active) this.finish(this.active.request, "connectionClosed");
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
