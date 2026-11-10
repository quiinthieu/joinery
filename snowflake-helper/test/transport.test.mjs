import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("./transport-probe.mjs", import.meta.url));
const scenarios = [
  "missing-config",
  "lost-success-deduped",
  "lost-success-no-dedupe",
  "documented-options",
  "unsupported-options",
  "persistent-loss",
];

async function bounded(promise, milliseconds) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Transport child did not close")), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

for (const scenario of scenarios) {
  test(`actual SDK HTTP transport: ${scenario}`, { timeout: 30000 }, async (t) => {
    const root = await mkdtemp(join(tmpdir(), "joinery-sdk-transport-"));
    // Parent owns deletion. Never discover or sweep other tests' directories.
    let child,
      closed,
      observedClose = false,
      deadline,
      exceeded = false,
      overflow = false;
    let stdout = "",
      stderr = "",
      spawnError;
    try {
      await chmod(root, 0o700);
      child = spawn(process.execPath, [entry, scenario], {
        cwd: root,
        env: { JOINERY_TRANSPORT_ROOT: root },
        stdio: ["ignore", "pipe", "pipe"],
      });
      closed = new Promise((resolve) => {
        child.once("error", (error) => {
          spawnError = error;
        });
        child.once("close", (code, signal) => {
          observedClose = true;
          resolve({ code, signal });
        });
      });
      child.stdout.on("data", (chunk) => {
        if (stdout.length + chunk.length > 1_048_576) {
          overflow = true;
          child.kill("SIGKILL");
        } else stdout += chunk.toString();
      });
      child.stderr.on("data", (chunk) => {
        if (stderr.length + chunk.length > 65536) {
          overflow = true;
          child.kill("SIGKILL");
        } else stderr += chunk.toString();
      });
      deadline = setTimeout(() => {
        exceeded = true;
        child.kill("SIGKILL");
      }, 20000);
      const status = await bounded(closed, 25000);
      clearTimeout(deadline);
      assert.equal(spawnError, undefined);
      assert.equal(exceeded, false, "transport child exceeded its outer deadline");
      assert.equal(overflow, false, "transport child exceeded output bounds");
      assert.equal(stderr, "");
      const result = JSON.parse(stdout);
      // Evidence is confined to this test root and removed after actual close.
      await writeFile(join(root, "evidence.json"), JSON.stringify({ result, status }), {
        mode: 0o600,
      });
      assert.equal(status.code, scenario === "missing-config" ? 1 : 0);
      assert.equal(status.signal, null);
      assert.equal(result.sdkVersion, "3.4.1");
      assert.equal(result.actualNetworkDenied, 0);
      assert.equal(result.serverError, undefined);
      assert.equal(result.unexpectedEndpoint, undefined);
      assert.equal(result.listenerClosed, true);
      assert.equal(result.sequenceIdPresent, false);
      if (scenario === "missing-config") {
        assert.equal(result.harnessError.code, "405505");
        assert.equal(result.httpEndpoints.length, 0);
        assert.equal(result.statementRequestCount, 0);
      } else {
        assert.equal(result.connectedToOwnedFakeServer, true);
        assert.equal(result.configMode, 0o600);
        assert(!result.ownedArtifacts.some((path) => path.endsWith(".log")));
        assert(result.statementRequestCount >= 2);
        assert.equal(result.sameRequestId, true);
        assert.equal(result.sameBody, true);
        assert.match(
          result.sdkGeneratedRequestId,
          /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
        );
        assert(
          result.requests.every(
            (request) =>
              request.method === "POST" && request.sqlText === "SELECT 'SYNTHETIC_LOCAL_ONLY'",
          ),
        );
        assert.equal(new Set(result.requests.map((request) => request.bodySHA256)).size, 1);
        assert.equal(result.requests[0].retryCount, null);
        assert.equal(result.requests[1].retryCount, "1");
        assert.equal(result.requests[0].requestId, result.sdkGeneratedRequestId);
        if (scenario === "persistent-loss") {
          // This is a bounded no-terminal observation, not retry exhaustion or
          // proof that a remote submitted statement was cancelled/rolled back.
          assert.equal(result.finishedBecause, "bounded-no-terminal");
          assert.equal(result.statementCallbacks, 0);
          assert.equal(result.simulatedServerSuccesses, 1);
        } else {
          assert.equal(result.finishedBecause, "success-callback");
          assert.equal(result.statementCallbacks, 1);
          assert.equal(result.callback.ok, true);
          assert.equal(result.streamRows, 1);
          assert.equal(result.statementRequestCount, 2);
          assert.equal(
            result.simulatedServerSuccesses,
            scenario === "lost-success-no-dedupe" ? 2 : 1,
          );
        }
      }
      t.diagnostic(
        JSON.stringify({
          scenario,
          requests: result.statementRequestCount,
          simulatedSuccesses: result.simulatedServerSuccesses,
          terminal: result.finishedBecause,
        }),
      );
    } finally {
      clearTimeout(deadline);
      if (child && !observedClose) {
        child.kill("SIGKILL");
        await bounded(closed, 4000);
      }
      // If reaping fails, leave the root in place rather than delete live state.
      if (!child || observedClose) await rm(root, { recursive: true, force: true });
    }
  });
}
