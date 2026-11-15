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
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  NODE_ARCHIVE_NAME,
  NPM_VERSION,
  NODE_VERSION,
  prepareSnowflakeRuntime,
} from "./prepare-snowflake-runtime.mjs";

const archiveRootName = `node-v${NODE_VERSION}-darwin-arm64`;

test("repository provenance inventories exact locked notice gaps and retains the unresolved proxy", () => {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const vendorRoot = path.join(repoRoot, "snowflake-helper", "vendor", "notices");
  const lockBytes = readFileSync(path.join(repoRoot, "snowflake-helper", "package-lock.json"));
  const locked = JSON.parse(lockBytes).packages;
  const provenance = JSON.parse(readFileSync(path.join(vendorRoot, "provenance.json"), "utf8"));
  assert.equal(provenance.packageLockSha256, sha256(lockBytes));
  assert.equal(Object.keys(locked).filter((key) => key.startsWith("node_modules/")).length, 183);
  assert.equal(provenance.packages.length, 11);
  assert.equal(provenance.packages.filter((item) => item.kind === "complete-readme").length, 4);
  const supplements = provenance.packages.filter((item) => item.kind === "supplement");
  assert.deepEqual(supplements.map((item) => `${item.name}@${item.version}`).sort(), [
    "@aws-sdk/credential-provider-http@3.972.84",
    "@aws-sdk/credential-provider-login@3.972.89",
    "@aws-sdk/nested-clients@3.998.1",
    "@nodable/entities@3.1.0",
    "asn1.js-rfc2560@5.0.1",
    "asn1.js-rfc5280@3.0.0",
    "proxy-agent-negotiate@1.1.0",
  ]);
  assert.deepEqual(
    provenance.packages.filter((item) => !item.complete).map((item) => item.name),
    ["proxy-agent-negotiate"],
  );
  const files = new Set();
  for (const record of provenance.packages) {
    assert.equal(locked[record.lockPath].version, record.version);
    assert.equal(locked[record.lockPath].integrity, record.integrity);
    assert.equal(locked[record.lockPath].resolved, record.resolved);
    assert.match(record.source.commit, /^[a-f\d]{40}$/);
    if (record.kind === "supplement") {
      const bytes = readFileSync(path.join(vendorRoot, record.notice.file));
      assert.equal(sha256(bytes), record.notice.sha256);
      assert.ok(
        record.source.artifacts.some(
          (item) => item.url === record.notice.sourceUrl && item.sha256 === record.notice.sha256,
        ),
      );
      files.add(record.notice.file);
      if (record.declaredLicense === "MIT") {
        assert.match(bytes.toString(), /Permission is hereby granted/);
        assert.match(bytes.toString(), /COPYRIGHT HOLDERS/);
      } else {
        assert.match(bytes.toString(), /Apache License/);
        assert.match(bytes.toString(), /END OF TERMS AND CONDITIONS/);
      }
    }
  }
  assert.equal(files.size, 4);
});

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
  for (const filename of [
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
  ])
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
  const vendorRoot = path.join(helperRoot, "vendor", "notices");
  mkdirSync(vendorRoot, { recursive: true });
  writeFileSync(
    path.join(vendorRoot, "provenance.json"),
    JSON.stringify({
      format: "joinery-snowflake-notices-v1",
      packageLockSha256: sha256(readFileSync(path.join(helperRoot, "package-lock.json"))),
      packages: [],
    }),
  );
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
  afterInstall = () => {},
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
      afterInstall(options.cwd);
      return { status: 0, stdout: "", stderr: "" };
    }
    throw new Error(`Unexpected command: ${command} ${args.join(" ")}`);
  };
  return { calls, runCommand };
}

function runFixture(fixture, options = {}) {
  const { calls, runCommand } = commandRunner(options);
  const result = prepareSnowflakeRuntime({
    // Synthetic notice/package fixtures do not contain real SDK code. Actual
    // reviewed bytes and tampering are covered by transport-source-pins tests.
    verifyTransport: options.verifyTransport ?? (() => {}),
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

function fixtureNoticeEvidence(fixture, { complete = true, kind = "supplement" } = {}) {
  const vendorRoot = path.join(fixture.repoRoot, "snowflake-helper", "vendor", "notices");
  const packageName = kind === "supplement" ? "proxy-agent-negotiate" : "readme-only";
  const lockPath = `node_modules/${packageName}`;
  const version = kind === "supplement" ? "1.1.0" : "1.0.0";
  const packageBytes = JSON.stringify({ name: packageName, version, license: "MIT" });
  const lock = JSON.parse(
    readFileSync(path.join(fixture.repoRoot, "snowflake-helper", "package-lock.json"), "utf8"),
  );
  const sourceUrl = `https://raw.githubusercontent.com/example/notices/${"1".repeat(40)}/LICENSE`;
  const noticeBytes = "Original upstream fixture notice\nCopyright fixture authors\n";
  writeFileSync(path.join(vendorRoot, "LICENSE"), noticeBytes);
  const record = {
    name: packageName,
    version,
    lockPath,
    integrity: lock.packages[lockPath].integrity,
    resolved: lock.packages[lockPath].resolved,
    kind,
    complete,
    installedFiles: {
      "package.json": sha256(packageBytes),
      ...(kind === "complete-readme" ? { "README.md": sha256("original readme notice\n") } : {}),
    },
    source: {
      repository: "https://github.com/example/notices",
      commit: "1".repeat(40),
      mapping: "Synthetic test fixture only; no upstream applicability claim.",
      artifacts: [{ url: sourceUrl, sha256: sha256(noticeBytes) }],
    },
    ...(kind === "supplement"
      ? { notice: { file: "LICENSE", sha256: sha256(noticeBytes), sourceUrl } }
      : {}),
    notes: complete ? [] : ["Sibling notice applicability is unresolved."],
  };
  const manifestPath = path.join(vendorRoot, "provenance.json");
  const manifest = {
    format: "joinery-snowflake-notices-v1",
    packageLockSha256: sha256(
      readFileSync(path.join(fixture.repoRoot, "snowflake-helper", "package-lock.json")),
    ),
    packages: [record],
  };
  const save = () => writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  save();
  return { vendorRoot, manifestPath, manifest, record, noticeBytes, save };
}

test("copies supplemental original bytes and provenance separately from package evidence", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const evidence = fixtureNoticeEvidence(fixture);
  const { result } = runFixture(fixture);
  const pkg = result.manifest.notices.packages.find(
    (item) => item.name === "proxy-agent-negotiate",
  );
  assert.equal(pkg.evidence, "no-root-notice-found");
  assert.deepEqual(pkg.files, []);
  assert.equal(pkg.coverage, "original-upstream-supplement");
  assert.equal(pkg.complete, true);
  assert.equal(
    readFileSync(path.join(result.target, "notices", "supplements", "LICENSE"), "utf8"),
    evidence.noticeBytes,
  );
  const copied = readFileSync(
    path.join(result.target, "notices", result.manifest.notices.provenance.path),
  );
  assert.deepEqual(copied, readFileSync(evidence.manifestPath));
  assert.equal(sha256(copied), result.manifest.notices.provenance.sha256);
  assert.equal(pkg.supplements[0].sha256, sha256(evidence.noticeBytes));
});

test("preserves unresolved sibling applicability even after copying the complete sibling notice", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  fixtureNoticeEvidence(fixture, { complete: false });
  const { result } = runFixture(fixture);
  const pkg = result.manifest.notices.packages.find(
    (item) => item.name === "proxy-agent-negotiate",
  );
  assert.equal(pkg.complete, false);
  assert.equal(pkg.coverage, "incomplete-sibling-notice-evidence");
  assert.ok(
    result.manifest.notices.packagesWithIncompleteNotice.includes("proxy-agent-negotiate@1.1.0"),
  );
  assert.deepEqual(result.manifest.notices.knownEvidenceGaps[0], {
    package: "proxy-agent-negotiate@1.1.0",
    notes: ["Sibling notice applicability is unresolved."],
  });
});

test("only a hash-verified complete README changes incomplete README classification", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  fixtureNoticeEvidence(fixture, { kind: "complete-readme" });
  const { result } = runFixture(fixture);
  const pkg = result.manifest.notices.packages.find((item) => item.name === "readme-only");
  assert.equal(pkg.evidence, "readme-only");
  assert.equal(pkg.coverage, "verified-complete-readme");
  assert.equal(pkg.complete, true);
  assert.deepEqual(result.manifest.notices.packagesWithIncompleteNotice, [
    "proxy-agent-negotiate@1.1.0",
  ]);
});

for (const failure of [
  "missing",
  "tampered",
  "version",
  "integrity",
  "lock",
  "installed",
  "source",
  "escape",
  "symlink",
  "readme",
  "missing-dependency",
]) {
  test(`rejects ${failure} supplemental evidence and preserves the prior successful stage`, (t) => {
    const fixture = makeFixture();
    t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
    const evidence = fixtureNoticeEvidence(
      fixture,
      failure === "readme" ? { kind: "complete-readme" } : {},
    );
    const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
    mkdirSync(target, { recursive: true });
    writeFileSync(path.join(target, "prior-success"), "keep\n");
    const notice = path.join(evidence.vendorRoot, "LICENSE");
    if (failure === "missing") rmSync(notice);
    if (failure === "tampered") writeFileSync(notice, "changed\n");
    if (failure === "version") evidence.record.version = "1.1.1";
    if (failure === "integrity") evidence.record.integrity = "sha512-different";
    if (failure === "lock") evidence.manifest.packageLockSha256 = "0".repeat(64);
    if (failure === "source") evidence.record.source.commit = "mutable-tag";
    if (failure === "escape") evidence.record.notice.file = "../outside-notice";
    if (failure === "symlink") {
      rmSync(notice);
      writeFileSync(path.join(evidence.vendorRoot, "original"), evidence.noticeBytes);
      symlinkSync("original", notice);
    }
    evidence.save();
    assert.throws(
      () =>
        runFixture(fixture, {
          afterInstall: (packageRoot) => {
            if (failure === "installed")
              writeFileSync(
                path.join(packageRoot, evidence.record.lockPath, "package.json"),
                JSON.stringify({ name: "wrong-package", version: "1.1.0" }),
              );
            if (failure === "readme")
              writeFileSync(
                path.join(packageRoot, evidence.record.lockPath, "README.md"),
                "MIT label without complete notice\n",
              );
            if (failure === "missing-dependency")
              rmSync(path.join(packageRoot, "node_modules", "snowflake-sdk"), { recursive: true });
          },
        }),
      /ENOENT|notice|Notice|dependency is missing/,
    );
    assert.equal(readFileSync(path.join(target, "prior-success"), "utf8"), "keep\n");
    assert.deepEqual(readdirSync(path.dirname(target)), ["macos-arm64", "npm-cache"]);
  });
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
  for (const name of [
    "sql-contract.json",
    "sql-contract.mjs",
    "mutation-batch.mjs",
    "mutation-read.mjs",
  ]) {
    assert.equal(
      readFileSync(path.join(result.target, "helper", name), "utf8"),
      readFileSync(path.join(fixture.repoRoot, "snowflake-helper", "src", name), "utf8"),
    );
    assert.equal(typeof manifest.helper.sourceSha256[name], "string");
  }
  assert.deepEqual(manifest.notices.packagesWithNoNoticeFile, ["proxy-agent-negotiate@1.1.0"]);
  assert.deepEqual(manifest.notices.packagesWithReadmeOnly, ["readme-only@1.0.0"]);
  assert.deepEqual(manifest.notices.packagesWithIncompleteNotice, [
    "proxy-agent-negotiate@1.1.0",
    "readme-only@1.0.0",
  ]);
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
        verifyTransport: () => {},
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

test("transport source rejection prevents promotion and preserves the prior runtime", (t) => {
  const fixture = makeFixture();
  t.after(() => rmSync(fixture.repoRoot, { recursive: true, force: true }));
  const target = path.join(fixture.repoRoot, ".cache", "snowflake-runtime", "macos-arm64");
  mkdirSync(target, { recursive: true });
  writeFileSync(path.join(target, "prior-success"), "keep\n");
  let verified = false;
  assert.throws(
    () =>
      runFixture(fixture, {
        verifyTransport: (entry) => {
          verified = true;
          assert.equal(path.basename(fileURLToPath(entry)), "main.mjs");
          assert.equal(existsSync(fileURLToPath(entry)), true);
          throw new Error("fixed transport rejection");
        },
      }),
    /fixed transport rejection/,
  );
  assert.equal(verified, true);
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
    verifyTransport: () => {},
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
        verifyTransport: () => {},
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
        verifyTransport: () => {},
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
        verifyTransport: () => {},
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
        verifyTransport: () => {},
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
