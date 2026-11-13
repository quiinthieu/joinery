import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { NODE_BINARY_SHA256, NODE_VERSION } from "./prepare-snowflake-runtime.mjs";
import { selectRuntime, verifySnowflakeRuntime } from "./verify-snowflake-runtime.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const pinnedNode = path.join(repository, ".cache/toolchains/node-v24.22.2-darwin-arm64/bin/node");
const script = fileURLToPath(new URL("./verify-snowflake-runtime.mjs", import.meta.url));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

function fixture(t) {
  const parent = path.join(repository, ".cache/verification/snowflake-startup");
  mkdirSync(parent, { recursive: true });
  const repoRoot = mkdtempSync(path.join(parent, "fixture-"));
  t.after(() => rmSync(repoRoot, { recursive: true, force: true }));
  const bundle = path.join(repoRoot, "Joinery with spaces.app");
  const stage = selectRuntime([], repoRoot).runtime;
  const bundled = selectRuntime(["--bundle", bundle], repoRoot).runtime;
  const sources = {};
  mkdirSync(path.join(repoRoot, "snowflake-helper/src"), { recursive: true });
  for (const name of [
    "main.mjs",
    "helper.mjs",
    "mutation-batch.mjs",
    "mutation-read.mjs",
    "framing.mjs",
    "query-submit-guard.mjs",
    "transport-source-pins.mjs",
    "disposable-setup-ledger.mjs",
    "sql-contract.json",
    "sql-contract.mjs",
  ]) {
    sources[name] = `// synthetic fixture ${name}\n`;
    writeFileSync(path.join(repoRoot, "snowflake-helper/src", name), sources[name]);
  }
  const manifest = {
    format: "joinery-snowflake-runtime-v1",
    target: "darwin-arm64",
    runtime: {
      version: NODE_VERSION,
      platform: "darwin",
      arch: "arm64",
      sha256: NODE_BINARY_SHA256,
    },
    helper: {
      sdkVersion: "3.4.1",
      sourceSha256: Object.fromEntries(
        Object.entries(sources).map(([name, bytes]) => [name, digest(bytes)]),
      ),
    },
  };
  for (const runtime of [stage, bundled]) {
    mkdirSync(path.join(runtime, "bin"), { recursive: true });
    mkdirSync(path.join(runtime, "helper/node_modules/snowflake-sdk"), { recursive: true });
    // Read the real pinned binary for hash validation; never execute this fixture tree.
    symlinkSync(pinnedNode, path.join(runtime, "bin/node"));
    for (const [name, bytes] of Object.entries(sources))
      writeFileSync(path.join(runtime, "helper", name), bytes);
    writeFileSync(
      path.join(runtime, "helper/node_modules/snowflake-sdk/package.json"),
      JSON.stringify({ name: "snowflake-sdk", version: "3.4.1" }),
    );
    writeFileSync(path.join(runtime, "manifest.json"), `${JSON.stringify(manifest)}\n`);
  }
  return { repoRoot, bundle, stage, bundled, manifest };
}

function helloFrame(overrides = {}) {
  const body = Buffer.from(
    JSON.stringify({
      v: 1,
      type: "hello",
      protocol: "joinery-snowflake",
      sdkVersion: "3.4.1",
      ...overrides,
    }),
  );
  const prefix = Buffer.alloc(4);
  prefix.writeUInt32BE(body.length);
  return Buffer.concat([prefix, body]);
}

function verify(
  f,
  args,
  runChild = () => {
    throw new Error("unexpected child launch");
  },
  // This fixture tests startup orchestration with synthetic helper/dependencies.
  // Actual reviewed bytes and tampering have separate transport-source tests.
  verifyTransport = () => {},
) {
  return verifySnowflakeRuntime({
    args,
    repoRoot: f.repoRoot,
    platform: "darwin",
    arch: "arm64",
    runChild,
    verifyTransport,
  });
}

test("default selection stays fixed to the repository stage", () => {
  assert.deepEqual(selectRuntime([], "/fixture/repository"), {
    kind: "stage",
    runtime: "/fixture/repository/.cache/snowflake-runtime/macos-arm64",
  });
});

test("explicit app selection uses Contents/Resources and preserves spaces", () => {
  assert.deepEqual(selectRuntime(["--bundle", "./Joinery with spaces.app/"]), {
    kind: "bundle",
    runtime: path.resolve("Joinery with spaces.app/Contents/Resources/snowflake-runtime"),
  });
});

for (const args of [
  ["--unknown"],
  ["--bundle"],
  ["--bundle="],
  ["--bundle", ""],
  ["--bundle", " "],
  ["--bundle", "--bundle"],
  ["--bundle", "Joinery"],
  ["--bundle", "Joinery.app/Contents"],
  ["--bundle", "bad\0.app"],
  ["Joinery.app"],
  ["--bundle", "Joinery.app", "extra"],
  ["--bundle", "Joinery.app", "--bundle", "Other.app"],
]) {
  test(`rejects invalid or duplicate arguments ${JSON.stringify(args)}`, () => {
    assert.throws(() => selectRuntime(args), /Usage:/);
  });
}

for (const kind of ["stage", "bundle"]) {
  test(`${kind} startup selects only that resource tree and reports exact manifest evidence`, (t) => {
    const f = fixture(t);
    const runtime = kind === "stage" ? f.stage : f.bundled;
    // An unusable alternate route cannot affect the explicit selection.
    rmSync(path.join(kind === "stage" ? f.bundled : f.stage, "manifest.json"));
    let privateRoot;
    let calls = 0;
    const result = verify(
      f,
      kind === "stage" ? [] : ["--bundle", f.bundle],
      (executable, args, options) => {
        calls += 1;
        assert.equal(executable, path.join(runtime, "bin/node"));
        assert.equal(args[0], "--require");
        assert.equal(args[2], path.join(runtime, "helper/main.mjs"));
        assert.equal(options.cwd, runtime);
        privateRoot = options.env.JOINERY_SNOWFLAKE_STATE_DIR;
        assert.deepEqual(Object.keys(options.env), ["JOINERY_SNOWFLAKE_STATE_DIR"]);
        assert.equal(options.input.length, 0);
        assert.equal(options.timeout, 15_000);
        assert.equal(options.killSignal, "SIGKILL");
        const guard = readFileSync(args[1], "utf8");
        assert.match(guard, /Socket\.prototype\.connect = deny/);
        assert.match(guard, /Socket\.prototype\.send = deny/);
        assert.match(guard, /dns\.promises\.Resolver\.prototype/);
        return { status: 0, signal: null, stdout: helloFrame() };
      },
    );
    assert.equal(calls, 1);
    assert.equal(existsSync(privateRoot), false);
    assert.equal(result.resourceKind, kind);
    assert.equal(
      result.check,
      `${kind === "stage" ? "staged" : "bundle"}-helper-startup-with-node-network-guard`,
    );
    assert.equal(result.manifestSha256, digest(readFileSync(path.join(runtime, "manifest.json"))));
    assert.equal(result.nodeSha256, NODE_BINARY_SHA256);
    assert.equal(result.authenticationRequests, 0);
    assert.equal(result.sqlRequests, 0);
  });
}

test("missing explicit bundle resources never fall back to a valid stage", (t) => {
  const f = fixture(t);
  rmSync(f.bundled, { recursive: true });
  assert.throws(() => verify(f, ["--bundle", f.bundle]), /ENOENT/);
  assert.equal(existsSync(path.join(f.stage, "manifest.json")), true);
});

test("manifest cannot replace the immutable Node binary pin", (t) => {
  const f = fixture(t);
  f.manifest.runtime.sha256 = "0".repeat(64);
  writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
  assert.throws(() => verify(f, []), /pinned Node release/);
});

test("changed Node bytes are rejected even when manifest retains its pin", (t) => {
  const f = fixture(t);
  rmSync(path.join(f.stage, "bin/node"));
  writeFileSync(path.join(f.stage, "bin/node"), "tampered node");
  assert.throws(() => verify(f, []), /pinned binary/);
});

test("target, release and platform manifest changes fail before launching", (t) => {
  const f = fixture(t);
  for (const [object, key, value] of [
    [f.manifest, "format", "unknown"],
    [f.manifest, "target", "linux-x64"],
    [f.manifest.runtime, "version", "24.22.3"],
    [f.manifest.runtime, "platform", "linux"],
    [f.manifest.runtime, "arch", "x64"],
  ]) {
    const original = object[key];
    object[key] = value;
    writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
    assert.throws(() => verify(f, []), /pinned Node release/);
    object[key] = original;
  }
});

test("source drift is rejected even when resource and manifest agree", (t) => {
  const f = fixture(t);
  writeFileSync(path.join(f.stage, "helper/main.mjs"), "changed helper");
  f.manifest.helper.sourceSha256["main.mjs"] = digest("changed helper");
  writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
  assert.throws(() => verify(f, []), /does not match this checkout/);
});

for (const name of [
  "sql-contract.json",
  "sql-contract.mjs",
  "mutation-batch.mjs",
  "mutation-read.mjs",
]) {
  test(`helper resource ${name} cannot drift even with a matching manifest`, (t) => {
    const f = fixture(t);
    writeFileSync(path.join(f.stage, "helper", name), "changed contract");
    f.manifest.helper.sourceSha256[name] = digest("changed contract");
    writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
    assert.throws(() => verify(f, []), /does not match this checkout/);
  });
  test(`helper resource ${name} must be present in the closed source hashes`, (t) => {
    const f = fixture(t);
    delete f.manifest.helper.sourceSha256[name];
    writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
    assert.throws(() => verify(f, []), /exactly the reviewed source hashes/);
  });
}

test("extra helper hash entries and SDK version drift are rejected", (t) => {
  const f = fixture(t);
  f.manifest.helper.sourceSha256["extra.mjs"] = "0".repeat(64);
  writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
  assert.throws(() => verify(f, []), /exactly the reviewed source hashes/);
  delete f.manifest.helper.sourceSha256["extra.mjs"];
  writeFileSync(path.join(f.stage, "manifest.json"), JSON.stringify(f.manifest));
  writeFileSync(
    path.join(f.stage, "helper/node_modules/snowflake-sdk/package.json"),
    JSON.stringify({ name: "snowflake-sdk", version: "3.4.2" }),
  );
  assert.throws(() => verify(f, []), /installed SDK package version/);
});

test("network attempt, failed child, extra frames and wrong hello fail and retire the private root", (t) => {
  const f = fixture(t);
  for (const scenario of ["network", "failed", "extra", "wrong"]) {
    let privateRoot;
    assert.throws(
      () =>
        verify(f, [], (_executable, _args, options) => {
          privateRoot = options.env.JOINERY_SNOWFLAKE_STATE_DIR;
          if (scenario === "network")
            writeFileSync(path.join(privateRoot, "network-attempted"), "blocked");
          return {
            status: scenario === "failed" ? 1 : 0,
            stdout:
              scenario === "extra"
                ? Buffer.concat([helloFrame(), helloFrame()])
                : helloFrame(scenario === "wrong" ? { sdkVersion: "wrong" } : {}),
          };
        }),
      /network access|cleanly|exactly one|version did not match/,
    );
    assert.equal(existsSync(privateRoot), false);
  }
});

test("transport verification failure prevents any startup child", (t) => {
  const f = fixture(t);
  let verified = false;
  assert.throws(
    () =>
      verify(
        f,
        [],
        () => {
          assert.fail("unverified transport must not spawn");
        },
        (entry) => {
          verified = true;
          assert.equal(fileURLToPath(entry), path.join(f.stage, "helper/main.mjs"));
          throw new Error("fixed transport rejection");
        },
      ),
    /fixed transport rejection/,
  );
  assert.equal(verified, true);
});

test("CLI rejects unknown arguments without attempting runtime startup", () => {
  assert.throws(
    () => execFileSync(pinnedNode, [script, "--unknown"], { encoding: "utf8", stdio: "pipe" }),
    (error) => {
      assert.equal(error.status, 1);
      assert.equal(error.stdout, "");
      assert.match(error.stderr, /Usage:/);
      return true;
    },
  );
});
