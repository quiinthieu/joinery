// Supplied-child exercise only: no CLI, spawn, filesystem, runtime selection or
// approval authority. Offline-v1 remains offline-only. A future executable
// boundary must bind these sources/limits, consume approval and own state roots.
import { verifyManifest } from "./snowflake-qualification-manifest.mjs";
import { createQualificationProtocol } from "./snowflake-qualification-protocol.mjs";
import { createQualificationDecoder } from "./snowflake-qualification-framing.mjs";
import { encode } from "../snowflake-helper/src/framing.mjs";

const TIMINGS = Object.freeze({
  helloMs: 2000,
  authenticationMs: 120000,
  sqlMs: 30000,
  optionsOperationMs: 65000,
  stateOperationMs: 65000,
  admissionWindowMs: 600000,
  overallMs: 360000,
  closeMs: 2000,
  eofMs: 2000,
  termMs: 2000,
  killMs: 3000,
});
export const CHILD_LIMITS = Object.freeze({ stderrBytes: 65536, overallMs: TIMINGS.overallMs });
const byteLength = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(Uint8Array.prototype),
  "byteLength",
).get;
const safeError = (code) => Object.assign(new Error(code), { safeCode: code });
const safeCode = (error, fallback) =>
  typeof error?.safeCode === "string" && /^[a-zA-Z]{1,80}$/.test(error.safeCode)
    ? error.safeCode
    : fallback;
function freeze(value) {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
function timingOptions(overrides, caps) {
  if (
    !overrides ||
    typeof overrides !== "object" ||
    Array.isArray(overrides) ||
    Reflect.ownKeys(overrides).some((key) => {
      const descriptor = Object.getOwnPropertyDescriptor(overrides, key);
      return (
        !Object.hasOwn(TIMINGS, key) ||
        !descriptor?.enumerable ||
        !Object.hasOwn(descriptor, "value") ||
        !Number.isSafeInteger(descriptor.value) ||
        descriptor.value < 1 ||
        descriptor.value > TIMINGS[key]
      );
    })
  )
    throw safeError("childConfiguration");
  for (const [key, value] of Object.entries(TIMINGS))
    // Offline-v1 did not bind an overall execution cap. This engine enforces
    // its own fixed ceiling; the future executable envelope must bind it too.
    if (key !== "overallMs" && caps[key] !== value) throw safeError("childConfiguration");
  return { ...TIMINGS, ...overrides };
}

/**
 * Takes lifecycle ownership of a VALID supplied child until exit+close or the
 * bounded cleanup deadline. Invalid configuration rejects BEFORE ownership;
 * the caller retains that child. No private result values/IDs/errors are
 * returned. Call before allowing another consumer to read child.stdout.
 *
 * timingOverrides is a reduction-only synthetic-test seam, never approval.
 * Node's exit event is the reaping evidence; close additionally proves stdio
 * closure. A close/cancel ACK is recorded only after protocol validation. No
 * rollback, remote cancellation, auth-resource or filesystem cleanup is claimed.
 */
export async function exerciseQualificationChild(child, options) {
  let verified, protocol, timings, manifest, currentInput, generation, signal, timingOverrides;
  try {
    const required = ["manifest", "currentInput", "generation"];
    const optional = ["signal", "timingOverrides"];
    if (
      !options ||
      typeof options !== "object" ||
      Array.isArray(options) ||
      required.some((key) => !Object.hasOwn(options, key)) ||
      Reflect.ownKeys(options).some((key) => {
        const descriptor = Object.getOwnPropertyDescriptor(options, key);
        return (
          (!required.includes(key) && !optional.includes(key)) ||
          !descriptor?.enumerable ||
          !Object.hasOwn(descriptor, "value")
        );
      })
    )
      throw safeError("childConfiguration");
    ({ manifest, currentInput, generation, signal, timingOverrides = {} } = options);
    verified = verifyManifest(manifest, currentInput);
    timings = timingOptions(timingOverrides, verified.payload.caps);
    protocol = createQualificationProtocol({ manifest: verified, currentInput, generation });
    for (const object of [child, child?.stdin, child?.stdout, child?.stderr])
      if (typeof object?.on !== "function" || typeof object?.removeListener !== "function")
        throw safeError("childConfiguration");
    for (const method of ["write", "end", "destroy"])
      if (typeof child.stdin[method] !== "function") throw safeError("childConfiguration");
    if (
      typeof child.kill !== "function" ||
      typeof child.stdout.destroy !== "function" ||
      typeof child.stderr.destroy !== "function" ||
      (signal !== undefined &&
        (typeof signal.addEventListener !== "function" ||
          typeof signal.removeEventListener !== "function" ||
          typeof signal.aborted !== "boolean"))
    )
      throw safeError("childConfiguration");
  } catch {
    throw safeError("childConfiguration");
  }

  let failure = null,
    finished = false,
    scheduled = false,
    writing = false,
    activeId = null,
    identifiedId = null,
    group = null,
    stageTimer = null,
    writeTimer = null,
    groupTimer = null,
    windowTimer = null,
    overallTimer = null,
    cleanupTimer = null,
    cleanupStarted = false,
    cleanupPhase = "none",
    cancelWanted = false,
    closeReserved = false,
    stdoutEnded = false,
    stderrEnded = false,
    stderrBytes = 0,
    stdoutBytes = 0,
    writeAttempts = 0,
    writeCallbacks = 0,
    drains = 0,
    pendingDrain = null,
    exitCode = null,
    exitSignal = null;
  let stageExpiry = 0,
    writeExpiry = 0,
    groupExpiry = 0,
    windowExpiry = 0,
    overallExpiry = 0,
    stageCode = "childHelloTimeout";
  const cleanup = {
    cancelReserved: false,
    cancelWriteAttempted: false,
    cancelWriteCompleted: false,
    cancelAcknowledged: false,
    closeReserved: false,
    closeWriteAttempted: false,
    closeWriteCompleted: false,
    closeAcknowledged: false,
    logoutAcknowledged: false,
    stdinEndRequested: false,
    stdinFinished: false,
    termSent: false,
    killSent: false,
    exitObserved: false,
    closeObserved: false,
  };
  let cancelId = null,
    closeId = null,
    resolve;
  const done = new Promise((complete) => {
    resolve = complete;
  });
  const decoder = createQualificationDecoder({
    onFrame({ frame, bodyBytes }) {
      expired(); // Still record a late known ACK; it cannot restore admission.
      try {
        protocol.receive(frame, { bodyBytes });
      } catch (error) {
        stop(safeCode(error, "childProtocolFailed"));
        throw safeError("childProtocolFailed");
      }
      if (frame.type === "statement") identifiedId = frame.id;
      if (frame.type === "terminal") {
        if (frame.id === activeId) {
          clearTimeout(stageTimer);
          stageExpiry = 0;
          activeId = null;
          identifiedId = null;
        }
        if (frame.id === cancelId && frame.ok) cleanup.cancelAcknowledged = true;
        if (frame.id === closeId && frame.ok) {
          cleanup.closeAcknowledged = true;
          cleanup.logoutAcknowledged = true;
        }
      }
      const snapshot = protocol.snapshot();
      if (snapshot.failure) stop(snapshot.failure);
      if (group && !snapshot.nextCommandId?.startsWith(`${group}.`)) {
        clearTimeout(groupTimer);
        groupExpiry = 0;
        group = null;
      }
      schedule();
    },
  });
  const listeners = [];
  function listen(object, event, callback) {
    object.on(event, callback);
    listeners.push([object, event, callback]);
  }
  function timer(action, milliseconds) {
    return setTimeout(() => {
      if (!finished) action();
    }, milliseconds);
  }
  function stop(code) {
    failure ??= code;
    clearTimeout(stageTimer);
    clearTimeout(groupTimer);
    clearTimeout(windowTimer);
    clearTimeout(overallTimer);
    if (!cleanupStarted) beginCleanup();
    schedule();
  }
  function expired() {
    if (cleanupStarted) return false;
    const now = performance.now();
    const candidates = [
      [stageExpiry, stageCode],
      [writeExpiry, "childWriteTimeout"],
      [groupExpiry, "childGroupTimeout"],
      [windowExpiry, "childWindowExpired"],
      [overallExpiry, "childOverallTimeout"],
    ]
      .filter(([deadline]) => deadline > 0 && now >= deadline)
      .sort((left, right) => left[0] - right[0]);
    if (!candidates.length) return false;
    stop(candidates[0][1]);
    return true;
  }
  function schedule() {
    if (scheduled || finished) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      if (!finished) pump();
    });
  }
  function write(frame) {
    if (writing || finished || cleanup.stdinEndRequested || cleanup.exitObserved) return false;
    writing = true;
    let callbackDone = false,
      drainDone = false,
      returned = false,
      needsDrain = false;
    const removeDrain = () => {
      child.stdin.removeListener("drain", onDrain);
      if (pendingDrain === onDrain) pendingDrain = null;
    };
    const complete = () => {
      if (!returned || !callbackDone || (needsDrain && !drainDone)) return;
      removeDrain();
      writing = false;
      clearTimeout(writeTimer);
      writeExpiry = 0;
      if (frame.id === closeId) cleanup.closeWriteCompleted = true;
      if (frame.id === cancelId) cleanup.cancelWriteCompleted = true;
      schedule();
    };
    const onDrain = () => {
      drainDone = true;
      drains++;
      complete();
    };
    // Drain/error listeners precede write, including synthetic synchronous events.
    pendingDrain = onDrain;
    child.stdin.on("drain", onDrain);
    const writeMs = cleanupStarted
      ? timings.closeMs
      : frame.op === "connect"
        ? timings.authenticationMs
        : timings.sqlMs;
    writeExpiry = performance.now() + writeMs;
    writeTimer = timer(() => {
      if (!expired()) stop("childWriteTimeout");
    }, writeMs);
    try {
      const body = encode(frame);
      writeAttempts++;
      if (frame.id === closeId) cleanup.closeWriteAttempted = true;
      if (frame.id === cancelId) cleanup.cancelWriteAttempted = true;
      needsDrain =
        child.stdin.write(body, (error) => {
          if (finished) return;
          if (error) {
            removeDrain();
            writing = false;
            clearTimeout(writeTimer);
            writeExpiry = 0;
            stop("childWriteFailed");
            return;
          }
          callbackDone = true;
          writeCallbacks++;
          complete();
        }) === false;
      returned = true;
      if (!needsDrain) removeDrain();
      complete();
      return true;
    } catch {
      removeDrain();
      writing = false;
      clearTimeout(writeTimer);
      writeExpiry = 0;
      stop("childWriteFailed");
      return false;
    }
  }
  function beginCleanup() {
    if (cleanupStarted || finished) return;
    cleanupStarted = true;
    cleanupPhase = "close";
    clearTimeout(stageTimer);
    clearTimeout(groupTimer);
    clearTimeout(windowTimer);
    clearTimeout(overallTimer);
    cleanupTimer = timer(eof, timings.closeMs);
  }
  function eof() {
    if (finished || cleanupPhase !== "close") return;
    clearTimeout(cleanupTimer);
    cleanupPhase = "eof";
    cleanup.stdinEndRequested = true;
    try {
      child.stdin.end();
    } catch {
      failure ??= "childEofFailed";
    }
    cleanupTimer = timer(() => {
      if (cleanup.exitObserved) {
        // The process has exited but inherited pipes may still be open. Close
        // only our handles; never signal a dead/reused PID or unknown descendant.
        closeExitedPipes();
      } else terminate();
    }, timings.eofMs);
  }
  function destroy(stream) {
    try {
      stream.destroy();
    } catch {
      failure ??= "childPipeCloseFailed";
    }
  }
  function closeExitedPipes() {
    clearTimeout(cleanupTimer);
    cleanupPhase = "stdio";
    destroy(child.stdout);
    destroy(child.stderr);
    destroy(child.stdin);
    if (!finished) cleanupTimer = timer(() => finish("childStdioNotClosed"), timings.killMs);
  }
  function sendSignal(name) {
    if (cleanup.exitObserved) return;
    try {
      const sent = child.kill(name);
      if (name === "SIGTERM") cleanup.termSent = sent === true;
      else cleanup.killSent = sent === true;
    } catch {
      failure ??= "childSignalFailed";
    }
  }
  function terminate() {
    cleanupPhase = "term";
    sendSignal("SIGTERM");
    cleanupTimer = timer(() => {
      cleanupPhase = "kill";
      sendSignal("SIGKILL");
      cleanupTimer = timer(() => finish("childNotReaped"), timings.killMs);
    }, timings.termMs);
  }
  function pump() {
    if (finished) return;
    if (cleanupStarted) {
      if (cleanup.closeObserved) return finish();
      if (cleanup.exitObserved && ["term", "kill"].includes(cleanupPhase))
        return closeExitedPipes();
      if (cleanup.closeAcknowledged || cleanup.exitObserved) return eof();
      if (cleanupPhase !== "close" || writing || cleanup.stdinEndRequested) return;
      if (cancelWanted) {
        cancelWanted = false;
        if (!protocol.snapshot().failure && !decoder.snapshot().failure && identifiedId) {
          try {
            const frame = protocol.reserveCancel("ownerCancellation");
            cleanup.cancelReserved = true;
            cancelId = frame.id;
            write(frame);
            return;
          } catch {
            /* Admission remains stopped; cancellation is not invented. */
          }
        }
      }
      if (!closeReserved && protocol.snapshot().hello) {
        try {
          const frame = protocol.reserveClose();
          closeReserved = true;
          closeId = frame.id;
          cleanup.closeReserved = true;
          write(frame);
        } catch {
          eof();
        }
      }
      return;
    }
    if (writing || failure || decoder.snapshot().failure || protocol.snapshot().failure) return;
    if (expired()) return;
    const wallNow = Date.now();
    if (wallNow < notBefore) return stop("childWindowUnavailable");
    if (wallNow >= expiresAt) return stop("childWindowExpired");
    let frame;
    try {
      frame = protocol.snapshot().creditNeeded ? protocol.reserveCredit() : protocol.reserveNext();
    } catch (error) {
      return stop(safeCode(error, "childProtocolFailed"));
    }
    if (!frame) return;
    if (frame.op === "close") {
      closeReserved = true;
      closeId = frame.id;
      cleanup.closeReserved = true;
      beginCleanup();
    } else if (frame.op !== "credit") {
      activeId = frame.id;
      clearTimeout(stageTimer);
      stageCode = frame.op === "connect" ? "childAuthenticationTimeout" : "childSqlTimeout";
      const stageMs = frame.op === "connect" ? timings.authenticationMs : timings.sqlMs;
      stageExpiry = performance.now() + stageMs;
      stageTimer = timer(() => {
        if (!expired()) stop(stageCode);
      }, stageMs);
      const next = protocol.snapshot().activeCommandId;
      const nextGroup = next?.startsWith("options.")
        ? "options"
        : next?.startsWith("state.")
          ? "state"
          : null;
      if (nextGroup && group !== nextGroup) {
        group = nextGroup;
        clearTimeout(groupTimer);
        const groupMs =
          nextGroup === "options" ? timings.optionsOperationMs : timings.stateOperationMs;
        groupExpiry = performance.now() + groupMs;
        groupTimer = timer(() => {
          if (!expired()) stop("childGroupTimeout");
        }, groupMs);
      }
    }
    write(frame);
  }
  function finish(fallback) {
    if (finished) return;
    if (fallback) failure ??= fallback;
    if (!cleanup.exitObserved) failure ??= "childNotReaped";
    if (!cleanup.closeObserved) failure ??= "childStdioNotClosed";
    if (!cleanup.logoutAcknowledged) failure ??= "childLogoutUnconfirmed";
    if (!protocol.snapshot().ledgerComplete) failure ??= "childWorkflowIncomplete";
    if (exitCode !== 0 || exitSignal !== null) failure ??= "childExitUnclean";
    finished = true;
    for (const handle of [
      stageTimer,
      writeTimer,
      groupTimer,
      windowTimer,
      overallTimer,
      cleanupTimer,
    ])
      clearTimeout(handle);
    if (pendingDrain) child.stdin.removeListener("drain", pendingDrain);
    for (const [object, event, callback] of listeners) {
      // Keep harmless error observers until GC; late pipe errors must not become
      // uncaught exceptions after the bounded result has already been returned.
      if (event !== "error") object.removeListener(event, callback);
    }
    signal?.removeEventListener("abort", abort);
    resolve(
      freeze({
        boundary: "suppliedChildExerciseOnly",
        authorizationGranted: false,
        fullRunnerImplemented: false,
        manifestSha256: verified.manifestSha256,
        passed: failure === null,
        failureCode: failure,
        protocol: protocol.snapshot(),
        decoder: decoder.snapshot(),
        outcomes: protocol.outcomes(),
        counts: { writeAttempts, writeCallbacks, drains, stdoutBytes, stderrBytes },
        cleanup: {
          ...cleanup,
          stdoutEnded,
          stderrEnded,
          childReaped: cleanup.exitObserved,
          exitCode,
          exitSignal,
        },
      }),
    );
  }
  function abort() {
    if (finished) return;
    cancelWanted = Boolean(identifiedId && !cleanupStarted);
    stop("childOwnerCancelled");
  }
  listen(child, "error", () => {
    if (!finished) stop("childProcessError");
  });
  listen(child.stdin, "error", () => {
    if (!finished) stop("childWriteFailed");
  });
  listen(child.stdin, "finish", () => {
    cleanup.stdinFinished = true;
  });
  listen(child.stdout, "error", () => {
    if (!finished) stop("childReadFailed");
  });
  listen(child.stderr, "error", () => {
    if (!finished) stop("childStderrFailed");
  });
  listen(child.stdout, "data", (chunk) => {
    if (finished) return;
    try {
      decoder.push(chunk);
    } catch (error) {
      stop(safeCode(error, "childDecodeFailed"));
      destroy(child.stdout);
    }
    stdoutBytes = decoder.snapshot().inboundBytes;
  });
  listen(child.stdout, "end", () => {
    stdoutEnded = true;
    if (finished) return;
    try {
      decoder.end();
    } catch (error) {
      stop(safeCode(error, "childDecodeFailed"));
    }
    if (!cleanupStarted) stop("childStdoutEndedEarly");
  });
  listen(child.stderr, "data", (chunk) => {
    if (finished) return;
    try {
      if (!(chunk instanceof Uint8Array)) return stop("childStderrFailed");
      stderrBytes += byteLength.call(chunk);
    } catch {
      return stop("childStderrFailed");
    }
    if (stderrBytes > CHILD_LIMITS.stderrBytes) {
      stop("childStderrLimit");
      destroy(child.stderr);
    }
  });
  listen(child.stderr, "end", () => {
    stderrEnded = true;
  });
  listen(child, "exit", (code, terminationSignal) => {
    if (finished) return;
    cleanup.exitObserved = true;
    exitCode = Number.isSafeInteger(code) ? code : null;
    exitSignal =
      terminationSignal == null
        ? null
        : ["SIGTERM", "SIGKILL"].includes(terminationSignal)
          ? terminationSignal
          : "other";
    if (!cleanupStarted) stop("childExitedEarly");
    else schedule();
  });
  listen(child, "close", () => {
    if (finished) return;
    cleanup.closeObserved = true;
    finish();
  });
  signal?.addEventListener("abort", abort, { once: true });
  const now = Date.now(),
    notBefore = Date.parse(verified.payload.window.notBefore),
    expiresAt = Date.parse(verified.payload.window.expiresAt);
  if (signal?.aborted) abort();
  else if (now < notBefore || now >= expiresAt) stop("childWindowUnavailable");
  else {
    stageExpiry = performance.now() + timings.helloMs;
    stageTimer = timer(() => {
      if (!expired()) stop("childHelloTimeout");
    }, timings.helloMs);
    const remainingWindow = Math.min(expiresAt - now, timings.admissionWindowMs);
    windowExpiry = performance.now() + remainingWindow;
    windowTimer = timer(() => {
      if (!expired()) stop("childWindowExpired");
    }, remainingWindow);
    overallExpiry = performance.now() + timings.overallMs;
    overallTimer = timer(() => {
      if (!expired()) stop("childOverallTimeout");
    }, timings.overallMs);
    schedule();
  }
  return done;
}
