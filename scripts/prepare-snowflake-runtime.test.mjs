import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  NODE_ARCHIVE_NAME,
  NPM_VERSION,
  NODE_VERSION,
  prepareSnowflakeRuntime,
} from "./prepare-snowflake-runtime.mjs";

const archiveRootName = `node-v${NODE_VERSION}-darwin-arm64`;

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function makeFixture({ nodeMode = 0o755 } = {}) {
  const repoRoot = mkdtempSync(path.join(os.tmpdir(), "joinery-snowflake-runtime-test-"));
  const toolsRoot = path.join(repoRoot, ".cache", "toolchains");
  const helperRoot = path.join(repoRoot, "snowflake-helper");
  const archiveTree = path.join(repoRoot, "archive-tree", archiveRootName);
  const nodeSource = path.join(archiveTree, "bin", "node");
  const archive = path.join(toolsRoot, NODE_ARCHIVE_NAME);
  const npmCli = path.join(archiveTree, "lib/node_modules/npm/bin/npm-cli.js");
  mkdirSync(path.dirname(nodeSource), { recursive: true });
  mkdirSync(toolsRoot, { recursive: true });
  mkdirSync(path.join(helperRoot, "src"), { recursive: true });
  mkdirSync(path.dirname(npmCli), { recursive: true });
  writeFileSync(
    path.join(archiveTree, "lib/node_modules/npm/package.json"),
    JSON.stringify({ name: "npm", version: NPM_VERSION }),
  );
  writeFileSync(nodeSource, "synthetic node executable\n", { mode: nodeMode });
  chmodSync(nodeSource, nodeMode);
  writeFileSync(path.join(archiveTree, "LICENSE"), "synthetic Node license\n");
  for (const filename of ["main.mjs", "helper.mjs", "framing.mjs"])
    writeFileSync(path.join(helperRoot, "src", filename), `// ${filename}\n`);
  writeFileSync(
    path.join(helperRoot, "package.json"),
    JSON.stringify({
      name: "test-helper",
      version: "0.0.0",
      dependencies: { "snowflake-sdk": "3.4.1" },
    }),
  );
  writeFileSync(
    path.join(helperRoot, "package-lock.json"),
    JSON.stringify({
      name: "test-helper",
      version: "0.0.0",
      lockfileVersion: 3,
      packages: {
        "": { name: "test-helper", version: "0.0.0", dependencies: { "snowflake-sdk": "3.4.1" } },
        "node_modules/snowflake-sdk": {
          version: "3.4.1",
          license: "Apache-2.0",
          resolved: "https://registry.npmjs.org/snowflake-sdk/-/snowflake-sdk-3.4.1.tgz",
          integrity:
            "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        },
        "node_modules/readme-only": {
          version: "1.0.0",
          license: "MIT",
          resolved: "https://registry.npmjs.org/readme-only/-/readme-only-1.0.0.tgz",
          integrity:
            "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        },
        "node_modules/proxy-agent-negotiate": {
          version: "1.1.0",
          license: "MIT",
          resolved:
            "https://registry.npmjs.org/proxy-agent-negotiate/-/proxy-agent-negotiate-1.1.0.tgz",
          integrity:
            "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        },
      },
    }),
  );
  writeFileSync(npmCli, "synthetic npm cli\n");
  execFileSync(
    "/usr/bin/tar",
    [
      "-czf",
      archive,
      `${archiveRootName}/bin/node`,
      `${archiveRootName}/LICENSE`,
      `${archiveRootName}/lib/node_modules/npm`,
    ],
    { cwd: path.join(repoRoot, "archive-tree"), stdio: "pipe" },
  );
  const archiveHash = sha256(readFileSync(archive));
  writeFileSync(
    path.join(toolsRoot, "node24-shasums.txt"),
    `${archiveHash}  ${NODE_ARCHIVE_NAME}\n`,
  );
  return { repoRoot, archive, archiveHash, nodeHash: sha256(readFileSync(nodeSource)) };
}

function commandRunner({
  failInstall = false,
  createPackageTree = true,
  symlinkMode = "none",
} = {}) {
  const calls = [];
  const runCommand = (command, args, options = {}) => {
    calls.push({ command, args, options });
    if (args[0]?.endsWith("npm-cli.js") && args[1] === "--version")
      return { status: 0, stdout: `${NPM_VERSION}\n`, stderr: "" };
    if (args[0] === "--version") return { status: 0, stdout: "v24.22.2\n", stderr: "" };
    if (args.includes("process.platform")) return { status: 0, stdout: "darwin\n", stderr: "" };
    if (args.includes("process.arch")) return { status: 0, stdout: "arm64\n", stderr: "" };
    if (command === "/usr/bin/codesign" && args[0] === "--verify")
      return { status: 0, stdout: "", stderr: "valid on disk\n" };
    if (command === "/usr/bin/codesign" && args[0] === "--display")
      return {
        status: 0,
        stdout: "",
        stderr:
          "Authority=Developer ID Application: Node.js Foundation\nTeamIdentifier=HX7739G8FX\n",
      };
    if (args.includes("ci")) {
      if (failInstall) return { status: 1, stdout: "", stderr: "offline package unavailable" };
      if (createPackageTree) {
        const moduleRoot = path.join(options.cwd, "node_modules");
        mkdirSync(path.join(moduleRoot, "snowflake-sdk"), { recursive: true });
        mkdirSync(path.join(moduleRoot, "readme-only"), { recursive: true });
        mkdirSync(path.join(moduleRoot, "proxy-agent-negotiate"), { recursive: true });
        if (symlinkMode !== "none") {
          mkdirSync(path.join(moduleRoot, ".bin"), { recursive: true });
          if (symlinkMode === "relative") {
            writeFileSync(path.join(moduleRoot, "snowflake-sdk", "cli.js"), "cli\n");
            symlinkSync("../snowflake-sdk/cli.js", path.join(moduleRoot, ".bin", "snowflake-cli"));
          } else if (symlinkMode === "escape") {
            writeFileSync(path.join(options.cwd, "outside-secret"), "outside\n");
            symlinkSync("../../outside-secret", path.join(moduleRoot, ".bin", "escape"));
          }
        }
        writeFileSync(
          path.join(moduleRoot, "snowflake-sdk", "package.json"),
          JSON.stringify({ name: "snowflake-sdk", version: "3.4.1", license: "Apache-2.0" }),
        );
        writeFileSync(path.join(moduleRoot, "snowflake-sdk", "LICENSE.txt"), "sdk license\n");
        writeFileSync(
          path.join(moduleRoot, "readme-only", "package.json"),
          JSON.stringify({ name: "readme-only", version: "1.0.0", license: "MIT" }),
        );
        writeFileSync(
          path.join(moduleRoot, "readme-only", "README.md"),
          "original readme notice\n",
        );
        writeFileSync(
          path.join(moduleRoot, "proxy-agent-negotiate", "package.json"),
          JSON.stringify({ name: "proxy-agent-negotiate", version: "1.1.0", license: "MIT" }),
        );
      }
      return { status: 0, stdout: "", stderr: "" };
    }
    throw new Error(`Unexpected command: ${command} ${args.join(" ")}`);
  };
  return { calls, runCommand };
}

function runFixture(fixture, options = {}) {
  const { calls, runCommand } = commandRunner(options);
  const result = prepareSnowflakeRuntime({
    repoRoot: fixture.repoRoot,
    platform: "darwin",
    arch: "arm64",
    runCommand,
    archiveSha256: fixture.archiveHash,
    nodeBinarySha256: fixture.nodeHash,
    offline: true,
  });
  return { calls, result };
}

test("stages only runtime/helper/dependencies with exact hashes and honest notice gaps", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));

  const { calls, result } = runFixture(fixture);
  assert.equal(
    result.target,
    path.join(realpathSync(fixture.repoRoot), ".cache", "snowflake-runtime", "macos-arm64"),
  );
  const manifest = JSON.parse(readFileSync(path.join(result.target, "manifest.json"), "utf8"));
  assert.equal(manifest.runtime.sha256, fixture.nodeHash);
  assert.equal(manifest.runtime.version, NODE_VERSION);
  assert.equal(manifest.runtime.npmVersion, NPM_VERSION);
  assert.equal(existsSync(path.join(result.target, "helper", "main.mjs")), true);
  assert.deepEqual(manifest.notices.packagesWithNoNoticeFile, ["proxy-agent-negotiate@1.1.0"]);
  assert.deepEqual(manifest.notices.packagesWithReadmeOnly, ["readme-only@1.0.0"]);
  assert.equal(
    readFileSync(
      path.join(result.target, "notices", "dependencies", "readme-only", "README.md"),
      "utf8",
    ),
    "original readme notice\n",
  );
  assert.equal(
    readFileSync(path.join(result.target, "notices", "node", "LICENSE"), "utf8"),
    "synthetic Node license\n",
  );
  assert.equal(
    calls.some(({ args }) => args.includes("ci")),
    true,
  );
  const installCall = calls.find(({ args }) => args.includes("ci"));
  assert.ok(installCall.args.includes("--offline"));
  assert.ok(installCall.args.includes("--omit=dev"));
  assert.ok(installCall.args.includes("--ignore-scripts"));
  assert.equal(Object.hasOwn(installCall.options.env, "NPM_TOKEN"), false);
  assert.equal(Object.hasOwn(installCall.options.env, "HOME"), true);
});

test("preserves relative npm bin links and resolves them inside the promoted target", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));

  const { result } = runFixture(fixture, { symlinkMode: "relative" });
  const nodeModules = path.join(result.target, "helper", "node_modules");
  const link = path.join(nodeModules, ".bin", "snowflake-cli");
  assert.equal(readFileSync(link, "utf8"), "cli\n");
  assert.equal(readlinkSync(link), "../snowflake-sdk/cli.js");
  const resolved = realpathSync(link);
  const relative = path.relative(nodeModules, resolved);
  assert.ok(relative && !relative.startsWith(`..${path.sep}`) && relative !== "..");
});

test("rejects escaping npm links before promotion and preserves the prior runtime", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "keep\n");
  const { runCommand } = commandRunner({ symlinkMode: "escape" });

  assert.throws(
    () =>
      prepareSnowflakeRuntime({
        repoRoot: fixture.repoRoot,
        platform: "darwin",
        arch: "arm64",
        runCommand,
        archiveSha256: fixture.archiveHash,
        nodeBinarySha256: fixture.nodeHash,
        offline: true,
      }),
    /Node dependency symlink target must be inside the repository cache root/,
  );
  assert.equal(readFileSync(path.join(target, "prior-success"), "utf8"), "keep\n");
});

test("reports a retained old backup after promotion cleanup fails", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "old\n");
  const { runCommand } = commandRunner();
  const result = prepareSnowflakeRuntime({
    repoRoot: fixture.repoRoot,
    platform: "darwin",
    arch: "arm64",
    runCommand,
    archiveSha256: fixture.archiveHash,
    nodeBinarySha256: fixture.nodeHash,
    offline: true,
    removeBackup: (backup, options) => {
      if (path.basename(backup).startsWith(".macos-arm64-backup-"))
        throw new Error("injected backup cleanup failure");
      rmSync(backup, options);
    },
  });

  assert.equal(existsSync(path.join(result.target, "helper", "main.mjs")), true);
  assert.deepEqual(result.cleanupWarnings, [
    "Previous runtime backup could not be removed and remains in the cache.",
  ]);
  assert.equal(Object.hasOwn(result.manifest, "cleanupWarnings"), false);
  assert.equal(
    readFileSync(path.join(target, "manifest.json"), "utf8"),
    `${JSON.stringify(result.manifest, null, 2)}\n`,
  );
  const backups = readdirSync(path.dirname(target)).filter((name) =>
    name.startsWith(".macos-arm64-backup-"),
  );
  assert.equal(backups.length, 1);
  assert.equal(
    readFileSync(path.join(path.dirname(target), backups[0], "prior-success"), "utf8"),
    "old\n",
  );
});

test("rejects a tampered archive before invoking npm or replacing the prior stage", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "keep\n");
  const bytes = readFileSync(fixture.archive);
  writeFileSync(fixture.archive, Buffer.concat([bytes, Buffer.from("tampered")]));
  const { calls, runCommand } = commandRunner();
  assert.throws(
    () =>
      prepareSnowflakeRuntime({
        repoRoot: fixture.repoRoot,
        platform: "darwin",
        arch: "arm64",
        runCommand,
        archiveSha256: fixture.archiveHash,
        nodeBinarySha256: fixture.nodeHash,
      }),
    /archive hash did not match/,
  );
  assert.equal(calls.length, 0);
  assert.equal(readFileSync(path.join(target, "prior-success"), "utf8"), "keep\n");
});

test("rejects unsupported targets without creating a staging tree", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  assert.throws(
    () => prepareSnowflakeRuntime({ repoRoot: fixture.repoRoot, platform: "linux", arch: "x64" }),
    /supports only darwin-arm64/,
  );
  assert.equal(
    readFileSync(path.join(fixture.repoRoot, ".cache", "toolchains", "node24-shasums.txt"), "utf8")
      .length > 0,
    true,
  );
  assert.equal(
    existsSync(path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64")),
    false,
  );
});

test("failed dependency staging leaves the previous successful runtime intact", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "keep\n");
  const { runCommand } = commandRunner({ failInstall: true });
  assert.throws(
    () =>
      prepareSnowflakeRuntime({
        repoRoot: fixture.repoRoot,
        platform: "darwin",
        arch: "arm64",
        runCommand,
        archiveSha256: fixture.archiveHash,
        nodeBinarySha256: fixture.nodeHash,
      }),
    /Production dependency installation failed/,
  );
  assert.equal(readFileSync(path.join(target, "prior-success"), "utf8"), "keep\n");
});

test("refuses cache roots that resolve through a symlink", (t) => {
  const fixture = makeFixture();
  const outside = mkdtempSync(path.join(os.tmpdir(), "joinery-snowflake-outside-"));
  t.after(() => {
    rmSync(fixture.repoRoot, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  });
  rmSync(path.join(fixture.repoRoot, ".cache"), { recursive: true, force: true });
  symlinkSync(outside, path.join(fixture.repoRoot, ".cache"), "dir");
  assert.throws(
    () =>
      prepareSnowflakeRuntime({
        repoRoot: fixture.repoRoot,
        platform: "darwin",
        arch: "arm64",
        archiveSha256: fixture.archiveHash,
        nodeBinarySha256: fixture.nodeHash,
      }),
    /real directory/,
  );
  assert.deepEqual(readdirSync(outside), []);
});

test("rejects a Node binary that remains non-executable", (t) => {
  const fixture = makeFixture({ nodeMode: 0o644 });
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "keep\n");
  const { runCommand } = commandRunner();
  assert.throws(
    () =>
      prepareSnowflakeRuntime({
        repoRoot: fixture.repoRoot,
        platform: "darwin",
        arch: "arm64",
        runCommand,
        archiveSha256: fixture.archiveHash,
        nodeBinarySha256: fixture.nodeHash,
        chmod: () => {},
      }),
    /not executable/,
  );
  assert.equal(readFileSync(path.join(target, "prior-success"), "utf8"), "keep\n");
});
