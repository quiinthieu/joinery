import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  copyPreparedSnowflakeRuntime,
  parseNativeSmokeArgs,
  removePreparedSnowflakeRuntime,
  validateSnowflakeResourceTree,
} from "./native-smoke-resources.mjs";
import {
  NODE_ARCHIVE_NAME,
  NODE_ARCHIVE_SHA256,
  NODE_VERSION,
  NPM_VERSION,
} from "./prepare-snowflake-runtime.mjs";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const sourceNames = [
  "disposable-setup-ledger.mjs",
  "sql-contract.json",
  "sql-contract.mjs",
  "framing.mjs",
  "helper.mjs",
  "main.mjs",
  "mutation-batch.mjs",
  "mutation-read.mjs",
  "query-submit-guard.mjs",
  "transport-source-pins.mjs",
];
const nodeBytes = Buffer.from("synthetic pinned node binary");

function put(root, relative, bytes, mode = 0o644) {
  const target = path.join(root, ...relative.split("/"));
  mkdirSync(path.dirname(target), { recursive: true, mode: 0o755 });
  writeFileSync(target, bytes, { mode });
  chmodSync(target, mode);
  return bytes;
}

function createFixture() {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), "native-smoke-runtime-")));
  const stage = path.join(root, ".cache/snowflake-runtime/macos-arm64");
  const contents = path.join(root, "bundle/Contents");
  mkdirSync(path.join(stage, "bin"), { recursive: true, mode: 0o755 });
  mkdirSync(path.join(stage, "helper/node_modules/snowflake-sdk"), {
    recursive: true,
    mode: 0o755,
  });
  mkdirSync(path.join(stage, "notices/node"), { recursive: true, mode: 0o755 });
  mkdirSync(contents, { recursive: true, mode: 0o755 });

  put(stage, "bin/node", nodeBytes, 0o755);
  const nodeLicense = put(stage, "notices/node/LICENSE", "Node release license\n");
  const helperSources = Object.fromEntries(
    sourceNames.map((name) => [name, put(stage, `helper/${name}`, `// ${name}\n`)]),
  );
  const repoSources = Object.fromEntries(
    sourceNames.map((name) => [name, put(root, `snowflake-helper/src/${name}`, `// ${name}\n`)]),
  );
  const packageLock = put(root, "snowflake-helper/package-lock.json", '{"lockfileVersion":3}\n');
  put(stage, "helper/package.json", '{"name":"joinery-snowflake-helper"}\n');
  put(
    stage,
    "helper/node_modules/snowflake-sdk/package.json",
    '{"name":"snowflake-sdk","version":"3.4.1"}\n',
  );
  const notice = put(stage, "notices/dependencies/example/LICENSE", "Example dependency license\n");
  const supplement = put(stage, "notices/supplements/shared-LICENSE", "Shared supplement\n");
  const provenance = {
    format: "joinery-snowflake-notices-v1",
    packageLockSha256: hash(packageLock),
    packages: [
      {
        lockPath: "node_modules/example",
        name: "example",
        version: "1.0.0",
        kind: "supplement",
        notice: {
          file: "shared-LICENSE",
          sha256: hash(supplement),
          sourceUrl: "https://example.invalid/license",
        },
      },
      { lockPath: "node_modules/other", name: "other", version: "1.0.0", kind: "none" },
    ],
  };
  const provenanceBytes = put(
    root,
    "snowflake-helper/vendor/notices/provenance.json",
    `${JSON.stringify(provenance)}\n`,
  );
  put(stage, "notices/supplements/provenance.json", provenanceBytes);

  const links = [
    [
      "helper/node_modules/.bin/fxparser",
      "../fast-xml-parser/src/cli/cli.js",
      "helper/node_modules/fast-xml-parser/src/cli/cli.js",
    ],
    [
      "helper/node_modules/.bin/is-docker",
      "../is-docker/cli.js",
      "helper/node_modules/is-docker/cli.js",
    ],
    [
      "helper/node_modules/.bin/is-inside-container",
      "../is-inside-container/cli.js",
      "helper/node_modules/is-inside-container/cli.js",
    ],
    [
      "helper/node_modules/.bin/semver",
      "../semver/bin/semver.js",
      "helper/node_modules/semver/bin/semver.js",
    ],
    [
      "helper/node_modules/is-inside-container/node_modules/.bin/is-docker",
      "../is-docker/cli.js",
      "helper/node_modules/is-inside-container/node_modules/is-docker/cli.js",
    ],
  ];
  for (const [, , target] of links) put(stage, target, "#!/usr/bin/env node\n", 0o755);
  for (const [relative, target] of links) {
    const link = path.join(stage, ...relative.split("/"));
    mkdirSync(path.dirname(link), { recursive: true, mode: 0o755 });
    symlinkSync(target, link);
  }

  const nodeSha256 = hash(nodeBytes);
  const manifest = {
    format: "joinery-snowflake-runtime-v1",
    target: "darwin-arm64",
    runtime: {
      version: NODE_VERSION,
      platform: "darwin",
      arch: "arm64",
      sha256: nodeSha256,
      archive: NODE_ARCHIVE_NAME,
      archiveSha256: NODE_ARCHIVE_SHA256,
      npmVersion: NPM_VERSION,
    },
    helper: {
      packageName: "joinery-snowflake-helper",
      packageVersion: "0.0.0",
      sdkVersion: "3.4.1",
      packageLockSha256: hash(packageLock),
      sourceSha256: Object.fromEntries(
        Object.entries(repoSources).map(([name, bytes]) => [name, hash(bytes)]),
      ),
    },
    notices: {
      nodeLicenseSha256: hash(nodeLicense),
      provenance: { path: "supplements/provenance.json", sha256: hash(provenanceBytes) },
      packages: [
        {
          lockPath: "node_modules/example",
          name: "example",
          version: "1.0.0",
          files: [{ path: "dependencies/example/LICENSE", sha256: hash(notice) }],
          supplements: [
            {
              path: "supplements/shared-LICENSE",
              sha256: hash(supplement),
              sourceUrl: "https://example.invalid/license",
            },
          ],
        },
        {
          lockPath: "node_modules/other",
          name: "other",
          version: "1.0.0",
          files: [],
          supplements: [],
        },
      ],
    },
  };
  put(stage, "manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
  return { root, stage, contents, nodeSha256, nodeLicenseSha256: hash(nodeLicense), links };
}

function copyRuntime(fixture) {
  return copyPreparedSnowflakeRuntime({
    repoRoot: fixture.root,
    bundleContents: fixture.contents,
    nodeBinarySha256: fixture.nodeSha256,
    nodeLicenseSha256: fixture.nodeLicenseSha256,
  });
}

test("copies the prepared tree with modes and five contained relative links", (t) => {
  const fixture = createFixture();
  t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
  const result = copyRuntime(fixture);
  const copied = result.target;
  assert.equal(result.symlinkCount, 5);
  assert.equal(readFileSync(path.join(copied, "bin/node")).toString(), nodeBytes.toString());
  assert.equal(lstatSync(path.join(copied, "bin/node")).mode & 0o777, 0o755);
  assert.equal(lstatSync(path.join(copied, "helper/main.mjs")).mode & 0o777, 0o644);
  for (const name of sourceNames) {
    assert.deepEqual(
      readFileSync(path.join(copied, "helper", name)),
      readFileSync(path.join(fixture.root, "snowflake-helper/src", name)),
    );
  }
  for (const [relative, target] of fixture.links) {
    const link = path.join(copied, ...relative.split("/"));
    assert.equal(readlinkSync(link), target);
    assert.equal(lstatSync(link).isSymbolicLink(), true);
  }
  assert.equal(
    result.manifestSha256,
    hash(readFileSync(path.join(fixture.stage, "manifest.json"))),
  );
});

test("rejects escaping links and preserves an existing generated copy on validation failure", (t) => {
  const fixture = createFixture();
  t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
  const first = copyRuntime(fixture);
  const existingManifest = readFileSync(path.join(first.target, "manifest.json"));
  put(fixture.stage, "helper/outside.js", "outside\n");
  const link = path.join(fixture.stage, "helper/node_modules/.bin/fxparser");
  rmSync(link);
  symlinkSync("../../outside.js", link);
  assert.throws(() => copyRuntime(fixture), /escapes dependencies|regular dependency file/);
  assert.deepEqual(readFileSync(path.join(first.target, "manifest.json")), existingManifest);
});

test("rejects tampered Node and missing stages without replacing the destination", (t) => {
  const fixture = createFixture();
  t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
  const result = copyRuntime(fixture);
  const sentinel = readFileSync(path.join(result.target, "manifest.json"));
  put(fixture.stage, "bin/node", "tampered", 0o755);
  assert.throws(() => copyRuntime(fixture), /binary hash changed/);
  assert.deepEqual(readFileSync(path.join(result.target, "manifest.json")), sentinel);
  rmSync(fixture.stage, { recursive: true, force: true });
  assert.throws(() => copyRuntime(fixture), /stage is missing/);
});

test("rejects tampered supplements, provenance, helper source, and Node LICENSE", (t) => {
  const cases = [
    {
      name: "supplement bytes",
      mutate: (fixture) => put(fixture.stage, "notices/supplements/shared-LICENSE", "changed\n"),
      error: /Dependency supplement hash did not match/,
    },
    {
      name: "provenance bytes",
      mutate: (fixture) => put(fixture.stage, "notices/supplements/provenance.json", "{}\n"),
      error: /Snowflake notice provenance hash did not match/,
    },
    {
      name: "repository helper source",
      mutate: (fixture) => put(fixture.root, "snowflake-helper/src/helper.mjs", "// changed\n"),
      error: /does not match the repository source/,
    },
    {
      name: "repository batch source",
      mutate: (fixture) =>
        put(fixture.root, "snowflake-helper/src/mutation-batch.mjs", "// changed\n"),
      error: /does not match the repository source/,
    },
    {
      name: "packaged batch source",
      mutate: (fixture) => put(fixture.stage, "helper/mutation-batch.mjs", "// changed\n"),
      error: /hash did not match/,
    },
    {
      name: "missing batch source",
      mutate: (fixture) => rmSync(path.join(fixture.stage, "helper/mutation-batch.mjs")),
      error: /missing/,
    },
    {
      name: "repository owner metadata source",
      mutate: (fixture) =>
        put(fixture.root, "snowflake-helper/src/mutation-read.mjs", "// changed\n"),
      error: /does not match the repository source/,
    },
    {
      name: "packaged owner metadata source",
      mutate: (fixture) => put(fixture.stage, "helper/mutation-read.mjs", "// changed\n"),
      error: /hash did not match/,
    },
    {
      name: "missing owner metadata source",
      mutate: (fixture) => rmSync(path.join(fixture.stage, "helper/mutation-read.mjs")),
      error: /missing/,
    },
    {
      name: "Node LICENSE bytes",
      mutate: (fixture) => put(fixture.stage, "notices/node/LICENSE", "changed\n"),
      error: /Node release LICENSE hash did not match/,
    },
  ];

  for (const scenario of cases) {
    const fixture = createFixture();
    t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
    const copied = copyRuntime(fixture);
    const originalManifest = readFileSync(path.join(copied.target, "manifest.json"));
    scenario.mutate(fixture);
    assert.throws(() => copyRuntime(fixture), scenario.error, scenario.name);
    assert.deepEqual(readFileSync(path.join(copied.target, "manifest.json")), originalManifest);
  }
});

test("does not accept rewritten stage helper and manifest or omitted provenance supplements", (t) => {
  for (const omission of [false, true]) {
    const fixture = createFixture();
    t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
    const copied = copyRuntime(fixture);
    const originalManifest = readFileSync(path.join(copied.target, "manifest.json"));
    const manifestPath = path.join(fixture.stage, "manifest.json");
    const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    if (omission) {
      manifest.notices.packages[0].supplements = [];
      put(fixture.stage, "manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
      assert.throws(() => copyRuntime(fixture), /supplemental notice mapping changed/);
    } else {
      const changed = put(fixture.stage, "helper/helper.mjs", "// rewritten stage source\n");
      manifest.helper.sourceSha256["helper.mjs"] = hash(changed);
      put(fixture.stage, "manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
      assert.throws(() => copyRuntime(fixture), /does not match the repository source/);
    }
    assert.deepEqual(readFileSync(path.join(copied.target, "manifest.json")), originalManifest);
  }
});

test("validates root mode, sticky bit, Node owner execute, and read-only tree API", (t) => {
  const fixture = createFixture();
  t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
  const validation = validateSnowflakeResourceTree({
    repoRoot: fixture.root,
    runtimeRoot: fixture.stage,
    nodeBinarySha256: fixture.nodeSha256,
    nodeLicenseSha256: fixture.nodeLicenseSha256,
  });
  assert.equal(validation.symlinkCount, 5);

  const rootMode = lstatSync(fixture.stage).mode & 0o7777;
  chmodSync(fixture.stage, rootMode | 0o1000);
  assert.throws(
    () =>
      validateSnowflakeResourceTree({
        repoRoot: fixture.root,
        runtimeRoot: fixture.stage,
        nodeBinarySha256: fixture.nodeSha256,
        nodeLicenseSha256: fixture.nodeLicenseSha256,
      }),
    /unsafe write or set-id permissions/,
  );
  chmodSync(fixture.stage, rootMode);

  chmodSync(path.join(fixture.stage, "bin/node"), 0o644);
  assert.throws(
    () =>
      validateSnowflakeResourceTree({
        repoRoot: fixture.root,
        runtimeRoot: fixture.stage,
        nodeBinarySha256: fixture.nodeSha256,
        nodeLicenseSha256: fixture.nodeLicenseSha256,
      }),
    /not executable/,
  );
});

test("removes only the generated runtime and validates CLI flags", (t) => {
  const fixture = createFixture();
  t.after(() => rmSync(fixture.root, { recursive: true, force: true }));
  const unrelated = path.join(fixture.contents, "Resources/other.txt");
  mkdirSync(path.dirname(unrelated), { recursive: true });
  writeFileSync(unrelated, "keep");
  copyRuntime(fixture);
  assert.equal(removePreparedSnowflakeRuntime({ bundleContents: fixture.contents }), true);
  assert.equal(readFileSync(unrelated, "utf8"), "keep");
  assert.deepEqual(parseNativeSmokeArgs(["--snowflake-resources"]), { snowflakeResources: true });
  assert.throws(() => parseNativeSmokeArgs(["--other"]), /Unknown or duplicate/);
  assert.throws(
    () => parseNativeSmokeArgs(["--snowflake-resources", "--snowflake-resources"]),
    /Unknown or duplicate/,
  );
});
