import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("./sdk-large-result-probe.mjs", import.meta.url));

for (const scenario of ["million", "cancel"]) {
  test(`actual SDK gzip large result: ${scenario}`, { timeout: 90_000 }, async (t) => {
    const root = await mkdtemp(join(tmpdir(), "joinery-sdk-large-result-"));
    await chmod(root, 0o700);
    let child;
    let closed;
    let observedClose = false;
    let deadline;
    let overflow = false;
    let timedOut = false;
    let stdout = "";
    let stderr = "";
    let spawnError;
    try {
      child = spawn(
        process.execPath,
        ["--expose-gc", "--max-old-space-size=256", entry, scenario],
        {
          cwd: root,
          // Explicit isolated environment: no inherited credentials or proxy settings.
          env: {
            HOME: root,
            SNOWFLAKE_HOME: join(root, "snowflake-home"),
            SNOWFLAKE_DISABLE_PLATFORM_DETECTION: "true",
            SF_OCSP_RESPONSE_CACHE_DIR: join(root, "ocsp"),
            SNOWFLAKE_CRL_ON_DISK_CACHE_DIR: join(root, "crl"),
            JOINERY_SDK_LARGE_RESULT_ROOT: root,
          },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      closed = new Promise((resolve) => {
        child.once("error", (error) => {
          spawnError = error;
        });
        child.once("close", (code, signal) => {
          observedClose = true;
          resolve({ code, signal });
        });
      });
      for (const [name, stream] of [
        ["stdout", child.stdout],
        ["stderr", child.stderr],
      ]) {
        stream.on("data", (chunk) => {
          if ((name === "stdout" ? stdout.length : stderr.length) + chunk.length > 32_768) {
            overflow = true;
            child.kill("SIGKILL");
          } else if (name === "stdout") stdout += chunk.toString();
          else stderr += chunk.toString();
        });
      }
      deadline = setTimeout(() => {
        timedOut = true;
        child.kill("SIGKILL");
      }, 75_000);
      const status = await closed;
      clearTimeout(deadline);
      const report = stdout ? JSON.parse(stdout) : undefined;
      // The bounded report contains counts/measurements only, never result rows.
      if (report) t.diagnostic(JSON.stringify(report));
      assert.equal(spawnError, undefined);
      assert.equal(overflow, false);
      assert.equal(timedOut, false);
      assert.equal(stderr, "");
      assert(report, "owned SDK child emitted no report");
      assert.deepEqual(status, { code: 0, signal: null });
      assert.equal(report.failure, undefined);
      assert.equal(report.sdkVersion, "3.4.1");
      assert.equal(report.actualNetworkDenied, 0);
      assert.deepEqual(report.serverErrors, []);
      assert.equal(report.listenerClosed, true);
      assert.equal(report.consumedRows, scenario === "million" ? 1_000_000 : 128);
      assert.equal(report.chunkRequests, scenario === "million" ? 100 : 3);
      assert.equal(report.pausedChunkRequests, 3);
      assert.equal(report.resultFullyConsumed, scenario === "million");
      assert.equal(report.streamReadableEnded, scenario === "million");
      if (scenario === "cancel") {
        assert.equal(report.localStreamDestroyed, true);
        assert.equal(report.cancelAcknowledged, true);
        assert.equal(report.abortRequests, 1);
        assert.equal(report.chunksAfterCancel, 3);
      }
      assert(report.compressedBytesServed > 0);
      assert(report.uncompressedBytesServed > report.compressedBytesServed);
      assert(report.memorySamples > 0);
      assert(report.resourceUsageMaxRssKiB > 0);
      assert(!(await readdir(root, { recursive: true })).some((path) => path.endsWith(".log")));
    } finally {
      clearTimeout(deadline);
      if (child && !observedClose) {
        child.kill("SIGKILL");
        await closed;
      }
      if (!child || observedClose) await rm(root, { recursive: true, force: true });
    }
  });
}
