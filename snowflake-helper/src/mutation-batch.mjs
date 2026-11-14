import { randomUUID } from "node:crypto";

// Dedicated generated-only transaction lane. Native admission owns the closed
// SQL grammar; this module supplies no general SQL classification or edit right.
export const MUTATION_LIMITS = Object.freeze({
  units: 500,
  steps: 510,
  ids: 510,
  manualBatches: 65536,
});
export const MUTATION_CONTEXT_SQL =
  "SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CAST(CURRENT_SESSION() AS VARCHAR), CAST(CURRENT_TRANSACTION() AS VARCHAR)";
export const MUTATION_SETTINGS_SQL =
  "SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION";
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const KEYS = ["user", "role", "database", "schema", "warehouse", "session"];
const exact = (p, keys) =>
  p &&
  typeof p === "object" &&
  !Array.isArray(p) &&
  Object.keys(p).length === keys.length &&
  Object.keys(p).every((k) => keys.includes(k));
const text = (s, n) =>
  typeof s === "string" &&
  s.length > 0 &&
  Buffer.byteLength(s) <= n &&
  s.isWellFormed() &&
  !s.includes("\0");
const identifier = (s) => text(s, 1020) && [...s].length <= 255;
const expectedValid = (p) =>
  exact(p, KEYS) &&
  identifier(p.user) &&
  text(p.session, 1024) &&
  ["role", "database", "schema", "warehouse"].every((k) => p[k] === null || identifier(p[k]));
export const sameMutationContext = (a, b) => KEYS.every((k) => a[k] === b[k]);
const scalar = (x) =>
  x === null ||
  typeof x === "boolean" ||
  (typeof x === "string" && Buffer.byteLength(x) <= 262144 && x.isWellFormed());
const transactionId = (s) =>
  text(s, 20) &&
  /^(0|-?[1-9][0-9]{0,18})$/.test(s) &&
  BigInt(s) >= -9223372036854775808n &&
  BigInt(s) <= 9223372036854775807n;

export function validateMutationRequest(r, helper) {
  const p = r.payload;
  // The compiler's 8MiB logical ceiling is not a transport promise. Reject the
  // complete encoded frame, including JSON escaping, before queueing or BEGIN.
  let bytes;
  try {
    bytes = Buffer.byteLength(JSON.stringify(r));
  } catch {
    return false;
  }
  if (
    bytes > 1048576 ||
    !expectedValid(p.expected) ||
    !helper.identityPin ||
    p.expected.user !== helper.identityPin.user ||
    p.expected.session !== helper.identityPin.session
  )
    return false;
  if (r.op === "mutationFinish")
    return (
      exact(p, ["owner", "action", "expected"]) &&
      UUID.test(p.owner) &&
      ["commit", "rollback"].includes(p.action)
    );
  if (
    !exact(p, ["mode", "owner", "rollbackPrior", "expected", "units"]) ||
    !["automatic", "manual"].includes(p.mode) ||
    !(p.owner === null || (typeof p.owner === "string" && UUID.test(p.owner))) ||
    typeof p.rollbackPrior !== "boolean" ||
    !Array.isArray(p.units) ||
    p.units.length < 1 ||
    p.units.length > MUTATION_LIMITS.units ||
    typeof helper.isFileTransfer !== "function"
  )
    return false;
  if (
    (p.owner === null && p.rollbackPrior) ||
    (p.owner !== null && (p.mode !== "manual" || !p.rollbackPrior))
  )
    return false;
  try {
    return p.units.every(
      (u) =>
        exact(u, ["kind", "sql", "binds"]) &&
        ["insert", "update", "delete"].includes(u.kind) &&
        text(u.sql, 262144) &&
        Array.isArray(u.binds) &&
        u.binds.length <= 2000 &&
        u.binds.every(scalar) &&
        !helper.isFileTransfer(u.sql),
    );
  } catch {
    return false;
  }
}

export function mutationOwnerAdmits(helper, r) {
  const owner = helper.mutationOwner;
  if (!owner)
    return (
      r.op !== "mutationFinish" &&
      r.op !== "mutationRead" &&
      (!(r.op === "mutationBatch") || r.payload.owner === null)
    );
  return (
    ["mutationBatch", "mutationFinish", "mutationRead"].includes(r.op) &&
    r.payload.owner === owner.token &&
    sameMutationContext(r.payload.expected, owner.expected) &&
    (r.op !== "mutationBatch" ||
      (r.payload.mode === "manual" &&
        r.payload.rollbackPrior &&
        owner.batches < MUTATION_LIMITS.manualBatches))
  );
}

// The compact native receipt preserves canonical UUIDs. Fail closed on other
// spellings rather than silently rewriting original SDK evidence.
const SDK_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const unavailable = () => ({ code: "outcomeUnknown" });
const active = (h, a) => !h.retired && !h.done.has(a.request) && h.state === "connected";
// Batch identities have an independent ceiling. The ordinary record() path's
// two-ID ceiling is unchanged, and one exact SDK statement owns each step.
async function batchIdentity(h, a, statement, acknowledged = false) {
  const b = a.batch,
    s = b.current;
  const sdkRequestId = statement?.getRequestId?.(),
    queryId = acknowledged ? statement?.getQueryId?.() || undefined : undefined;
  if (!SDK_UUID.test(sdkRequestId) || (queryId !== undefined && !SDK_UUID.test(queryId)))
    throw unavailable();
  const prior = b.identities.get(sdkRequestId);
  if (
    (prior && prior.step !== s.step) ||
    (prior?.queryId && queryId && prior.queryId !== queryId) ||
    (s.identity && s.identity.sdkRequestId !== sdkRequestId) ||
    (!prior && b.identities.size >= MUTATION_LIMITS.ids)
  )
    throw unavailable();
  const identity = {
    step: s.step,
    sdkRequestId,
    ...(queryId || prior?.queryId ? { queryId: queryId || prior.queryId } : {}),
  };
  s.identity = identity;
  if (!prior || prior.queryId !== identity.queryId) {
    b.identities.set(sdkRequestId, identity);
    await h.send(a.request, "batchStatement", { ...identity });
  }
  if (!active(h, a)) throw unavailable();
}

export async function executeMutationStep(h, a, sql, binds) {
  if (!active(h, a)) throw unavailable();
  if (
    ["mutationBatch"].includes(a.request.op) &&
    a.batch.current.role === "unit" &&
    (typeof h.isFileTransfer !== "function" || h.isFileTransfer(sql))
  )
    throw unavailable();
  const ticket = h.submitGuard?.beginStatement(a.submitOwner, a.batchCapacity);
  a.knownFailure = undefined;
  a.acknowledged = false;
  try {
    return await new Promise((resolve, reject) => {
      let returned = false,
        pending,
        completed = false,
        registration,
        registrationError,
        acknowledgement,
        acknowledgementError,
        submitted = false;
      const processComplete = () => {
        const [err, statement] = pending;
        (async () => {
          await acknowledgement;
          if (acknowledgementError) throw acknowledgementError;
          await registration;
          if (registrationError) throw registrationError;
          if (!active(h, a) || statement !== a.statement) throw unavailable();
          if (err) {
            const known =
              typeof err.sqlState === "string" &&
              /^[A-Z0-9]{5}$/.test(err.sqlState) &&
              !err.sqlState.startsWith("08");
            if (known) a.knownFailure = "database";
            throw { code: known ? "database" : "outcomeUnknown" };
          }
          await batchIdentity(h, a, statement, true);
          const result = await h.consume(a, statement, false);
          if (!active(h, a)) throw unavailable();
          return result;
        })().then(resolve, reject);
      };
      const complete = (err, statement) => {
        if (completed) return;
        completed = true;
        pending = [err, statement];
        if (!err && active(h, a)) {
          // Establish the callback's original ACK before every passive getter,
          // even for a synchronous callback whose returned identity is missing.
          a.acknowledged = true;
          a.batch.current.phase = "acknowledged";
          try {
            acknowledgement = Promise.resolve(
              h.send(a.request, "batchStep", {
                step: a.batch.current.step,
                role: a.batch.current.role,
                unit: a.batch.current.unit,
                phase: "acknowledged",
              }),
            ).catch((e) => {
              acknowledgementError = e || unavailable();
            });
          } catch (e) {
            acknowledgementError = e || unavailable();
          }
        }
        if (returned) processComplete();
      };
      try {
        if (!active(h, a)) throw unavailable();
        a.dispatching = true;
        a.statement = h.connection.execute({
          sqlText: sql,
          binds,
          rowMode: "array",
          fetchAsString: ["Number", "Date", "Boolean", "Buffer", "JSON", "Object", "Array", "Map"],
          streamResult: true,
          parameters: { MULTI_STATEMENT_COUNT: 1 },
          complete,
        });
        submitted = true;
        if (acknowledgementError || !active(h, a)) throw unavailable();
        if (ticket) h.submitGuard.registerRequestId(ticket, a.statement?.getRequestId?.());
        registration = batchIdentity(h, a, a.statement).catch((e) => {
          registrationError = e || unavailable();
        });
        returned = true;
        if (pending) processComplete();
      } catch (e) {
        returned = true;
        registrationError = e?.code ? e : unavailable();
        if (pending) processComplete();
        else if (!submitted) reject(registrationError);
        // A submitted statement may still ACK. Keep its lane/ticket until the
        // callback or native deadline retires it; never discard that future ACK.
      }
    });
  } finally {
    if (ticket) h.submitGuard.endStatement(ticket);
  }
}

function context(result) {
  const row = result.values?.[0];
  if (
    result.rowsSeen !== 1 ||
    result.values?.length !== 1 ||
    result.columns?.length !== 7 ||
    result.columns.some((c) => c.type.toLowerCase() !== "text") ||
    !row ||
    row.length !== 7 ||
    !expectedValid(Object.fromEntries(KEYS.map((k, i) => [k, row[i]]))) ||
    !(row[6] === null || transactionId(row[6]))
  )
    throw { code: "outcomeUnknown" };
  return { expected: Object.fromEntries(KEYS.map((k, i) => [k, row[i]])), transaction: row[6] };
}
function settings(result) {
  const names = result.columns?.map((c) => c.name.toLowerCase());
  const key = names?.indexOf("key"),
    value = names?.indexOf("value");
  const row = result.values?.[0];
  return (
    result.rowsSeen === 1 &&
    result.values?.length === 1 &&
    key >= 0 &&
    value >= 0 &&
    row?.[key] === "QUOTED_IDENTIFIERS_IGNORE_CASE" &&
    row[value]?.toLowerCase() === "false"
  );
}
function counts(result, kind) {
  const expectedNames =
    kind === "update"
      ? ["number of rows updated", "number of multi-joined rows updated"]
      : [`number of rows ${kind === "insert" ? "inserted" : "deleted"}`];
  const row = result.values?.[0];
  const length = result.columns?.length;
  if (
    result.rowsSeen !== 1 ||
    result.values?.length !== 1 ||
    !(length === expectedNames.length || (kind === "update" && length === 1)) ||
    !row ||
    row.length !== length ||
    result.columns.some(
      (c, i) => c.name.toLowerCase() !== expectedNames[i] || c.type.toLowerCase() !== "fixed",
    ) ||
    row.some((x) => typeof x !== "string" || !/^(0|[1-9][0-9]{0,37})$/.test(x))
  )
    throw { code: "outcomeUnknown" };
  // A generated UPDATE has no join. Omitted multi-join metadata stays absent;
  // never manufacture a zero observation from an absent status column.
  return {
    affectedRows: row[0],
    multiJoinedRows: kind === "update" && length === 2 ? row[1] : null,
  };
}
function disposition(result, id, session, wanted) {
  const names = result.columns?.map((c) => c.name.toLowerCase());
  if (
    !names ||
    new Set(names).size !== names.length ||
    result.rowsSeen !== 1 ||
    result.values?.length !== 1
  )
    return false;
  const row = result.values[0],
    idIndex = names.indexOf("id"),
    stateIndex = names.indexOf("state"),
    sessionIndex = names.indexOf("session");
  return (
    idIndex >= 0 &&
    stateIndex >= 0 &&
    sessionIndex >= 0 &&
    [idIndex, sessionIndex].every((i) =>
      ["text", "fixed"].includes(result.columns[i].type.toLowerCase()),
    ) &&
    result.columns[stateIndex].type.toLowerCase() === "text" &&
    row[idIndex] === id &&
    row[sessionIndex] === session &&
    row[stateIndex]?.toLowerCase() === wanted
  );
}

export async function runMutationRequest(helper, a) {
  const r = a.request,
    p = r.payload;
  const previous = helper.mutationOwner;
  let finishCapacity = previous?.finishCapacity;
  const batch = (a.batch = {
    step: 0,
    current: null,
    identities: new Map(),
    priorBatches: previous?.batches ?? 0,
    owner: previous?.token ?? (p.mode === "manual" ? randomUUID() : null),
    transactionId: previous?.transactionId ?? null,
    owns: Boolean(previous),
    terminal: null,
  });
  const summary = (outcome, failure = null) => ({
    outcome,
    owner: batch.owner,
    transactionId: batch.transactionId,
    failure,
    priorBatches: batch.priorBatches,
  });
  const alive = () => {
    if (helper.retired || helper.done.has(r) || helper.state !== "connected")
      throw { code: "outcomeUnknown" };
  };
  const publish = async (type, value) => {
    await helper.send(r, type, value);
    alive();
  };
  const step = async (role, sql, binds = [], unit = null) => {
    alive();
    if (++batch.step > MUTATION_LIMITS.steps) throw { code: "outcomeUnknown" };
    batch.current = { step: batch.step, role, unit, phase: "dispatching" };
    await publish("batchStep", { step: batch.step, role, unit, phase: "dispatching" });
    const result = await helper.execute(a, sql, binds, false);
    alive();
    if (role === "unit") {
      let count;
      try {
        count = counts(result, p.units[unit].kind);
      } catch {
        batch.current.countUnavailable = true;
      }
      await publish(count ? "batchCount" : "batchCountUnavailable", {
        step: batch.step,
        unit,
        ...(count ?? {}),
      });
    }
    batch.current.phase = "drained";
    await publish("batchStep", { step: batch.step, role, unit, phase: "drained" });
    return result;
  };
  const observe = async (role) => context(await step(role, MUTATION_CONTEXT_SQL));
  const check = (value, transaction) =>
    sameMutationContext(value.expected, p.expected) && value.transaction === transaction;
  const finish = (result) => {
    batch.terminal = result;
    try {
      helper.finish(r, null, result);
    } catch {
      helper.stop();
    }
  };
  const unknown = (failure) => {
    batch.terminal = summary("unknown", failure);
    helper.mutationOwner = null;
    try {
      helper.finish(r, null, batch.terminal);
    } finally {
      helper.stop();
    }
  };
  const proveDisposition = async (wanted) => {
    if (!transactionId(batch.transactionId)) throw unavailable();
    const result = await step("disposition", `DESCRIBE TRANSACTION ${batch.transactionId}`);
    if (!disposition(result, batch.transactionId, p.expected.session, wanted)) throw unavailable();
  };
  const rollback = async (failure) => {
    await step("rollback", "ROLLBACK");
    const idle = await observe("idle");
    if (!check(idle, null)) throw { code: "outcomeUnknown" };
    await proveDisposition("rolled back");
    helper.mutationOwner = null;
    finish(summary("rolledBack", failure));
  };
  try {
    if (!mutationOwnerAdmits(helper, r)) return finish(summary("rejected", "admission"));
    if (typeof helper.submitGuard?.reserveCapacity !== "function")
      return finish(summary("rejected", "admission"));
    if (r.op === "mutationFinish") {
      if (!finishCapacity) return unknown("admission");
      a.batchCapacity = finishCapacity;
    } else {
      a.batchCapacity = helper.submitGuard.reserveCapacity(p.units.length + 10);
      if (!a.batchCapacity) return finish(summary("rejected", "admission"));
      if (p.mode === "manual" && !previous) {
        finishCapacity = helper.submitGuard.reserveCapacity(5);
        if (!finishCapacity) return finish(summary("rejected", "admission"));
      }
    }
    if (r.op === "mutationFinish" && p.action === "rollback") {
      // No new SQL is allowed while this private token owns the same physical
      // session. Explicit rollback remains possible when a fresh read fails.
      await rollback(null);
      return;
    }
    const before = await observe("context");
    if (
      !sameMutationContext(before.expected, p.expected) ||
      before.transaction !== (previous?.transactionId ?? null)
    ) {
      if (previous) return unknown("context");
      return finish(summary("rejected", "admission"));
    }
    if (!settings(await step("settings", MUTATION_SETTINGS_SQL))) {
      if (previous) return unknown("context");
      return finish(summary("rejected", "admission"));
    }
    if (r.op === "mutationFinish") {
      await step("commit", "COMMIT");
      if (!check(await observe("idle"), null)) return unknown("context");
      await proveDisposition("committed");
      helper.mutationOwner = null;
      return finish(summary("committed"));
    }
    if (!previous) {
      await step("begin", "BEGIN TRANSACTION");
      batch.owns = true;
      const opened = await observe("transaction");
      if (!sameMutationContext(opened.expected, p.expected) || opened.transaction === null)
        return unknown("context");
      batch.transactionId = opened.transaction;
    }
    // AUTOCOMMIT is irrelevant inside this explicit, observed transaction.
    // TRANSACTION_ABORT_ON_ERROR may abort it; any error prevents pending, and
    // final transaction verification is mandatory even after successful DML.
    for (const [unit, value] of p.units.entries()) {
      const result = await step("unit", value.sql, value.binds, unit);
      if (batch.current.countUnavailable) {
        await rollback("summaryUnavailable");
        return;
      }
      const count = counts(result, value.kind);
      if (
        count.affectedRows !== "1" ||
        (count.multiJoinedRows !== null && count.multiJoinedRows !== "0")
      ) {
        await rollback("cardinality");
        return;
      }
    }
    if (!check(await observe("verify"), batch.transactionId)) return unknown("context");
    if (p.mode === "manual") {
      helper.mutationOwner = {
        token: batch.owner,
        transactionId: batch.transactionId,
        expected: { ...p.expected },
        batches: batch.priorBatches + 1,
        finishCapacity,
      };
      return finish(summary("pending"));
    }
    await step("commit", "COMMIT");
    if (!check(await observe("idle"), null)) return unknown("context");
    await proveDisposition("committed");
    finish(summary("committed"));
  } catch (e) {
    if (helper.done.has(r) || helper.retired) return;
    if (
      e?.code === "database" &&
      batch.owns &&
      batch.current?.role === "unit" &&
      a.knownFailure === "database"
    ) {
      try {
        await rollback("database");
        return;
      } catch {
        /* Boundary/idle uncertainty must not become rollback proof. */
      }
    }
    unknown(e?.code === "database" ? "database" : "transport");
  } finally {
    try {
      if (a.batchCapacity && r.op === "mutationBatch")
        helper.submitGuard.releaseCapacity(a.batchCapacity);
      if (
        finishCapacity &&
        (helper.retired || helper.mutationOwner?.finishCapacity !== finishCapacity)
      )
        helper.submitGuard.releaseCapacity(finishCapacity);
    } catch {
      helper.stop();
    }
  }
}

export function mutationInterruptedSummary(a, failure = "transport") {
  const b = a.batch;
  return (
    b?.terminal ?? {
      outcome: "unknown",
      owner: b?.owner ?? null,
      transactionId: b?.transactionId ?? null,
      failure,
      priorBatches: b?.priorBatches ?? 0,
    }
  );
}
