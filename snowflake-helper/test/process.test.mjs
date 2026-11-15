import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { once } from "node:events";
import { Decoder, encode } from "../src/framing.mjs";
const entry = resolve("src/main.mjs");
async function bounded(promise, ms = 4000) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Child deadline exceeded")), ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
function observeExit(child) {
  const exited = once(child, "exit");
  // A spawn error must not become an unhandled rejection before cleanup runs.
  exited.catch(() => {});
  return exited;
}
async function killAndReap(child, exited) {
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  await bounded(exited);
}
test("helper requires a parent-owned private state directory before SDK import", async () => {
  const child = spawn(process.execPath, [entry], {
    env: {},
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "",
    stderr = "";
  child.stdout.on("data", (chunk) => (stdout += chunk.toString()));
  child.stderr.on("data", (chunk) => (stderr += chunk.toString()));
  const exited = observeExit(child);
  try {
    child.stdin.end();
    const [code, signal] = await bounded(exited);
    assert.equal(code, 2);
    assert.equal(signal, null);
    assert.equal(stdout, "");
    assert.equal(stderr, "");
  } finally {
    await killAndReap(child, exited);
  }
});
test("helper rejects relative, shared, symlink and non-directory state roots", async () => {
  const root = await mkdtemp(join(tmpdir(), "joinery-sdk-root-validation-"));
  let childToReap, exitToReap;
  try {
    const shared = await mkdtemp(join(root, "shared-"));
    await chmod(shared, 0o755);
    const unusable = await mkdtemp(join(root, "unusable-"));
    await chmod(unusable, 0o600);
    const link = join(root, "link");
    await symlink(root, link);
    const file = join(root, "file");
    await writeFile(file, "SYNTHETIC", { mode: 0o600 });
    for (const stateRoot of ["relative", shared, unusable, link, file]) {
      const child = spawn(process.execPath, [entry], {
        env: { JOINERY_SNOWFLAKE_STATE_DIR: stateRoot },
        stdio: ["pipe", "pipe", "pipe"],
      });
      let stdout = "",
        stderr = "";
      child.stdout.on("data", (chunk) => (stdout += chunk.toString()));
      child.stderr.on("data", (chunk) => (stderr += chunk.toString()));
      const exited = observeExit(child);
      childToReap = child;
      exitToReap = exited;
      try {
        child.stdin.end();
        const [code, signal] = await bounded(exited);
        assert.equal(code, 2);
        assert.equal(signal, null);
        assert.equal(stdout, "");
        assert.equal(stderr, "");
      } finally {
        await killAndReap(child, exited);
      }
    }
  } finally {
    if (childToReap) await killAndReap(childToReap, exitToReap);
    await rm(root, { recursive: true, force: true });
  }
});
test(
  "actual SDK load emits only framed hello, no log files; malformed request exits",
  { timeout: 10000 },
  async () => {
    const root = await mkdtemp(join(tmpdir(), "joinery-sdk-process-test-"));
    const child = spawn(process.execPath, [entry], {
      cwd: root,
      env: { JOINERY_SNOWFLAKE_STATE_DIR: root },
      stdio: ["pipe", "pipe", "pipe"],
    });
    const exited = observeExit(child);
    try {
      const frames = [],
        decoder = new Decoder();
      let stderr = "";
      child.stderr.on("data", (b) => (stderr += b.toString()));
      const hello = new Promise((resolve, reject) => {
        child.stdout.on("data", (b) => {
          try {
            frames.push(...decoder.push(b));
            if (frames[0]?.type === "hello") resolve();
          } catch (e) {
            reject(e);
          }
        });
        child.on("error", reject);
        child.once("exit", () => reject(new Error("Child exited before hello")));
      });
      await bounded(hello);
      assert.equal(frames[0].sdkVersion, "3.4.1");
      child.stdin.write(
        encode({
          v: 1,
          generation: "b512e129-c689-48bd-b5c2-848438b1231a",
          id: "1",
          op: "connect",
          payload: { password: "SYNTHETIC-SQL-BIND-TOKEN-SECRET" },
        }),
      );
      // Argument rejection must not open browser or connect. Explicit close retires.
      child.stdin.write(
        encode({
          v: 1,
          generation: "b512e129-c689-48bd-b5c2-848438b1231a",
          id: "2",
          op: "close",
          payload: {},
        }),
      );
      await bounded(exited);
      decoder.end();
      assert.equal(stderr, "");
      assert.equal(JSON.stringify(frames).includes("SYNTHETIC-SQL-BIND-TOKEN-SECRET"), false);
      assert.equal(
        frames.find((e) => e.id === "1" && e.type === "terminal").error.code,
        "invalidInput",
      );
      assert.deepEqual((await readdir(root, { recursive: true })).sort(), [
        "client-config.json",
        "ocsp",
      ]);
      const config = await stat(join(root, "client-config.json"));
      assert.equal(config.mode & 0o777, 0o600);
      assert.equal(config.uid, process.getuid());
      assert.equal(config.gid, process.getgid());
    } finally {
      await killAndReap(child, exited);
      await rm(root, { recursive: true, force: true });
    }
  },
);

test(
  "actual SDK accepts exact helper configuration and destroys before authentication without I/O",
  { timeout: 10000 },
  async () => {
    const root = await mkdtemp(join(tmpdir(), "joinery-sdk-construction-test-"));
    const child = spawn(process.execPath, [resolve("test/configuration-child.mjs")], {
      cwd: root,
      env: {
        HOME: root,
        SNOWFLAKE_HOME: join(root, "disabled-snowflake-home"),
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    const exited = observeExit(child);
    let stdout = "",
      stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk.toString()));
    child.stderr.on("data", (chunk) => (stderr += chunk.toString()));
    try {
      const [code, signal] = await bounded(exited);
      assert.equal(code, 0, stderr);
      assert.equal(signal, null);
      assert.equal(stderr, "");
      assert.deepEqual(JSON.parse(stdout), {
        constructed: true,
        destroyCallback: true,
        networkAttempts: 0,
        browserAttempts: 0,
        privateReads: 0,
      });
      // SDK import creates an empty OCSP directory; no files or owner-home access.
      assert.deepEqual(await readdir(root, { recursive: true }), ["ocsp"]);
      assert.deepEqual(await readdir(join(root, "ocsp")), []);
    } finally {
      await killAndReap(child, exited);
      await rm(root, { recursive: true, force: true });
    }
  },
);
