// Owns an already supplied helper child. No spawn, credentials, approval or SQL text input.
import { createSetupProtocol } from "./snowflake-qualification-setup-protocol.mjs";
import { verifySetupManifest } from "./snowflake-qualification-setup-manifest.mjs";
import { createQualificationDecoder } from "./snowflake-qualification-framing.mjs";
import { encode } from "../snowflake-helper/src/framing.mjs";

const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
const safeCode = (error, fallback) =>
  /^[A-Za-z]{1,80}$/.test(error?.safeCode ?? "") ? error.safeCode : fallback;

export async function exerciseSetupChild(
  child,
  { manifest, currentInput, generation, persist, signal, timingOverrides = {} },
) {
  const verified = verifySetupManifest(manifest, currentInput);
  if (typeof persist !== "function") throw fail("setupChildConfiguration");
  const protocol = createSetupProtocol({ manifest: verified, currentInput, generation });
  const caps = verified.payload.caps;
  const timings = Object.fromEntries(
    [
      "helloMs",
      "authenticationMs",
      "sqlMs",
      "workflowMs",
      "closeMs",
      "eofMs",
      "termMs",
      "killMs",
    ].map((key) => [key, caps[key]]),
  );
  for (const [key, value] of Object.entries(timingOverrides)) {
    if (
      !Object.hasOwn(timings, key) ||
      !Number.isSafeInteger(value) ||
      value < 1 ||
      value > timings[key]
    )
      throw fail("setupChildConfiguration");
    timings[key] = value;
  }
  for (const object of [child, child?.stdin, child?.stdout, child?.stderr])
    if (typeof object?.on !== "function" || typeof object?.removeListener !== "function")
      throw fail("setupChildConfiguration");
  if (typeof child.stdin.write !== "function" || typeof child.kill !== "function")
    throw fail("setupChildConfiguration");
  const notBefore = Date.parse(verified.payload.window.notBefore),
    expiresAt = Date.parse(verified.payload.window.expiresAt);
  let failure = null,
    finished = false,
    scheduled = false,
    writing = false,
    pendingPersistence = 0,
    persistenceFailed = false;
  let stageTimer,
    writeTimer,
    workflowTimer,
    windowTimer,
    cleanupTimer,
    cleanupPhase = null,
    activeId = null,
    cancelFrame = null;
  let stderrBytes = 0,
    exitCode = null,
    exitSignal = null,
    writeAttempts = 0,
    persistence = Promise.resolve();
  let resolve;
  const done = new Promise((complete) => {
    resolve = complete;
  });
  const cleanup = {
    closeRequested: false,
    logoutAcknowledged: false,
    cancelRequested: false,
    stdinEndRequested: false,
    termSent: false,
    killSent: false,
    childReaped: false,
    closeObserved: false,
  };
  const listeners = [];
  const listen = (object, event, callback) => {
    object.on(event, callback);
    listeners.push([object, event, callback]);
  };
  const timer = (callback, ms) =>
    setTimeout(() => {
      if (!finished) callback();
    }, ms);
  function stop(code) {
    failure ??= code;
    protocol.stop(code);
    clearTimeout(stageTimer);
    clearTimeout(workflowTimer);
    clearTimeout(windowTimer);
    if (!cleanupPhase) {
      cleanupPhase = "close";
      cleanupTimer = timer(eof, timings.closeMs);
    }
    schedule();
  }
  function flushEvents() {
    for (const event of protocol.takeEvents()) {
      pendingPersistence++;
      persistence = persistence
        .then(async () => {
          if (!persistenceFailed) {
            let deadline;
            try {
              await Promise.race([
                Promise.resolve().then(() => persist(event)),
                new Promise((_, reject) => {
                  deadline = setTimeout(() => reject(fail("setupJournalTimeout")), timings.sqlMs);
                }),
              ]);
            } finally {
              clearTimeout(deadline);
            }
          }
        })
        .catch(() => {
          persistenceFailed = true;
          stop("setupJournalPersistenceFailed");
        })
        .finally(() => {
          pendingPersistence--;
          schedule();
        });
    }
  }
  function schedule() {
    if (scheduled || finished) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      if (!finished) void pump();
    });
  }
  function safeDestroy(stream) {
    try {
      stream.destroy();
    } catch {
      failure ??= "setupPipeCloseFailed";
    }
  }
  function closeExitedPipes() {
    clearTimeout(cleanupTimer);
    cleanupPhase = "stdio";
    for (const stream of [child.stdin, child.stdout, child.stderr]) safeDestroy(stream);
    if (!finished) cleanupTimer = timer(() => finish("setupStdioNotClosed"), timings.killMs);
  }
  function sendSignal(name) {
    if (cleanup.childReaped) return;
    try {
      const sent = child.kill(name) === true;
      if (name === "SIGTERM") cleanup.termSent = sent;
      else cleanup.killSent = sent;
    } catch {
      failure ??= "setupSignalFailed";
    }
  }
  function terminate() {
    if (cleanup.childReaped) return closeExitedPipes();
    cleanupPhase = "term";
    sendSignal("SIGTERM");
    cleanupTimer = timer(() => {
      if (cleanup.childReaped) return closeExitedPipes();
      cleanupPhase = "kill";
      sendSignal("SIGKILL");
      cleanupTimer = timer(() => finish("setupChildNotReaped"), timings.killMs);
    }, timings.termMs);
  }
  function eof() {
    if (finished || cleanup.stdinEndRequested) return;
    clearTimeout(cleanupTimer);
    cleanupPhase = "eof";
    cleanup.stdinEndRequested = true;
    try {
      child.stdin.end();
    } catch {
      failure ??= "setupEofFailed";
    }
    cleanupTimer = timer(
      () => (cleanup.childReaped ? closeExitedPipes() : terminate()),
      timings.eofMs,
    );
  }
  function write(frame) {
    if (finished || cleanup.stdinEndRequested || cleanup.childReaped || writing) return;
    writing = true;
    let callback = false,
      drained = false,
      returned = false,
      needsDrain = false;
    const complete = () => {
      if (!returned || !callback || (needsDrain && !drained)) return;
      child.stdin.removeListener("drain", onDrain);
      writing = false;
      clearTimeout(writeTimer);
      schedule();
    };
    const onDrain = () => {
      drained = true;
      complete();
    };
    child.stdin.on("drain", onDrain);
    const ms = cleanupPhase
      ? timings.closeMs
      : frame.op === "connect"
        ? timings.authenticationMs
        : timings.sqlMs;
    writeTimer = timer(() => {
      child.stdin.removeListener("drain", onDrain);
      writing = false;
      stop("setupWriteTimeout");
    }, ms);
    try {
      writeAttempts++;
      needsDrain =
        child.stdin.write(encode(frame), (error) => {
          if (finished) return;
          if (error) {
            child.stdin.removeListener("drain", onDrain);
            writing = false;
            clearTimeout(writeTimer);
            stop("setupWriteFailed");
          } else {
            callback = true;
            complete();
          }
        }) === false;
      returned = true;
      if (!needsDrain) child.stdin.removeListener("drain", onDrain);
      complete();
    } catch {
      child.stdin.removeListener("drain", onDrain);
      writing = false;
      clearTimeout(writeTimer);
      stop("setupWriteFailed");
    }
  }
  async function pump() {
    if (finished || writing || pendingPersistence) return;
    if (cleanupPhase) {
      if (cleanup.closeObserved) return void finish();
      if (cleanup.childReaped) return closeExitedPipes();
      if (protocol.snapshot().logoutAcknowledged) {
        cleanup.logoutAcknowledged = true;
        return eof();
      }
      if (cleanupPhase === "close" && cancelFrame && !cleanup.cancelRequested) {
        const frame = cancelFrame;
        cancelFrame = null;
        cleanup.cancelRequested = true;
        flushEvents();
        await persistence;
        if (!finished && cleanupPhase === "close") write(frame);
        return;
      }
      if (cleanupPhase !== "close" || cleanup.closeRequested) return;
      const frame = protocol.reserveClose();
      if (!frame) return;
      cleanup.closeRequested = true;
      flushEvents();
      await persistence;
      if (!finished && cleanupPhase === "close") write(frame);
      return;
    }
    if (Date.now() < notBefore || Date.now() >= expiresAt) return stop("setupWindowUnavailable");
    if (protocol.snapshot().failure) return stop(protocol.snapshot().failure);
    if (protocol.snapshot().ledgerComplete) return stopComplete();
    let frame;
    try {
      frame = protocol.reserveNext();
    } catch (error) {
      return stop(safeCode(error, "setupProtocolFailed"));
    }
    if (!frame) return;
    flushEvents();
    await persistence;
    if (finished || cleanupPhase || persistenceFailed) return;
    // No SQL or authentication is admitted after a slow journal crosses the window.
    if (Date.now() < notBefore || Date.now() >= expiresAt) return stop("setupWindowUnavailable");
    activeId = frame.id;
    clearTimeout(stageTimer);
    stageTimer = timer(
      () => stop("setupStageTimeout"),
      frame.op === "connect" ? timings.authenticationMs : timings.sqlMs,
    );
    write(frame);
  }
  function stopComplete() {
    clearTimeout(stageTimer);
    clearTimeout(workflowTimer);
    clearTimeout(windowTimer);
    cleanupPhase = "close";
    cleanupTimer = timer(eof, timings.closeMs);
    schedule();
  }
  async function finish(fallback) {
    if (finished) return;
    finished = true;
    failure ??= fallback ?? null;
    for (const handle of [stageTimer, writeTimer, workflowTimer, windowTimer, cleanupTimer])
      clearTimeout(handle);
    await persistence;
    if (!cleanup.childReaped) failure ??= "setupChildNotReaped";
    if (!cleanup.closeObserved) failure ??= "setupStdioNotClosed";
    cleanup.logoutAcknowledged = protocol.snapshot().logoutAcknowledged;
    if (!cleanup.logoutAcknowledged) failure ??= "setupLogoutUnconfirmed";
    if (!protocol.snapshot().ledgerComplete) failure ??= "setupWorkflowIncomplete";
    if (exitCode !== 0 || exitSignal !== null) failure ??= "setupChildExitUnclean";
    for (const [object, event, callback] of listeners)
      if (event !== "error") object.removeListener(event, callback);
    signal?.removeEventListener("abort", abort);
    resolve({
      passed: failure === null,
      failureCode: failure,
      outcomes: protocol.outcomes(),
      protocol: protocol.snapshot(),
      cleanup: { ...cleanup, exitCode, exitSignal },
      counts: { writeAttempts, stderrBytes, ...decoder.snapshot() },
      journalFailed: persistenceFailed,
    });
  }
  const decoder = createQualificationDecoder({
    onFrame({ frame }) {
      try {
        protocol.receive(frame);
      } catch (error) {
        stop(safeCode(error, "setupProtocolFailed"));
      }
      flushEvents();
      if (frame.type === "terminal" && frame.id === activeId) {
        activeId = null;
        clearTimeout(stageTimer);
      }
      if (protocol.snapshot().failure) stop(protocol.snapshot().failure);
      schedule();
    },
  });
  function abort() {
    if (finished) return;
    // One explicit owner interruption may request cancellation of an identified
    // statement. Timeouts never invent remote cancellation or cleanup SQL.
    cancelFrame = !cleanupPhase ? protocol.reserveCancel() : null;
    stop("setupOwnerCancelled");
    flushEvents();
  }
  listen(child, "error", () => stop("setupChildProcessError"));
  listen(child.stdin, "error", () => stop("setupWriteFailed"));
  listen(child.stdout, "error", () => stop("setupReadFailed"));
  listen(child.stderr, "error", () => stop("setupStderrFailed"));
  listen(child.stdout, "data", (chunk) => {
    if (finished) return;
    try {
      decoder.push(chunk);
    } catch (error) {
      stop(safeCode(error, "setupDecodeFailed"));
      safeDestroy(child.stdout);
    }
  });
  listen(child.stdout, "end", () => {
    if (finished) return;
    try {
      decoder.end();
    } catch (error) {
      stop(safeCode(error, "setupDecodeFailed"));
    }
    if (!cleanupPhase) stop("setupStdoutEndedEarly");
  });
  listen(child.stderr, "data", (chunk) => {
    if (finished) return;
    if (!(chunk instanceof Uint8Array)) return stop("setupStderrFailed");
    stderrBytes += chunk.byteLength;
    if (stderrBytes > caps.maximumStderrBytes) {
      stop("setupStderrLimit");
      safeDestroy(child.stderr);
    }
  });
  listen(child, "exit", (code, terminationSignal) => {
    if (finished) return;
    cleanup.childReaped = true;
    exitCode = Number.isSafeInteger(code) ? code : null;
    exitSignal =
      terminationSignal == null
        ? null
        : ["SIGTERM", "SIGKILL"].includes(terminationSignal)
          ? terminationSignal
          : "other";
    if (!cleanupPhase) stop("setupChildExitedEarly");
    else schedule();
  });
  listen(child, "close", () => {
    if (!finished) {
      cleanup.closeObserved = true;
      void finish();
    }
  });
  signal?.addEventListener("abort", abort, { once: true });
  stageTimer = timer(() => stop("setupHelloTimeout"), timings.helloMs);
  workflowTimer = timer(() => stop("setupWorkflowTimeout"), timings.workflowMs);
  windowTimer = timer(() => stop("setupWindowUnavailable"), Math.max(1, expiresAt - Date.now()));
  if (signal?.aborted) abort();
  else if (Date.now() < notBefore || Date.now() >= expiresAt) stop("setupWindowUnavailable");
  return done;
}
