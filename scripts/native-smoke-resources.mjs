import { createHash, randomUUID } from "node:crypto";
import {
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  NODE_ARCHIVE_NAME,
  NODE_ARCHIVE_SHA256,
  NODE_BINARY_SHA256,
  NODE_VERSION,
  NPM_VERSION,
} from "./prepare-snowflake-runtime.mjs";

const scriptRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const HELPER_FILES = [
  "framing.mjs",
  "helper.mjs",
  "main.mjs",
  "query-submit-guard.mjs",
  "transport-source-pins.mjs",
  "disposable-setup-ledger.mjs",
];
const HELPER_PACKAGE = "joinery-snowflake-helper";
const HELPER_VERSION = "0.0.0";
const SDK_VERSION = "3.4.1";
const MANIFEST_FORMAT = "joinery-snowflake-runtime-v1";
const NODE_LICENSE_SHA256 = "5888dbb9a1d2b18f2c3e6c5f6af1b39de658372b402a0577b002777f14c62ace";
const MAX_TREE_ENTRIES = 20_000;
const MAX_TREE_DEPTH = 32;
const FORBIDDEN_MODE_BITS = 0o7022;

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function contained(root, candidate) {
  const relative = path.relative(root, candidate);
  return (
    relative !== "" &&
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

function assertRealDirectory(directory, label) {
  let stat;
  try {
    stat = lstatSync(directory);
  } catch (error) {
    if (error.code === "ENOENT") throw new Error(`${label} is missing`);
    throw error;
  }
  assert(stat.isDirectory() && !stat.isSymbolicLink(), `${label} must be a real directory`);
  assert(realpathSync(directory) === path.resolve(directory), `${label} resolved unexpectedly`);
  return stat;
}

function assertSafeMode(stat, itemPath, isDirectory = false) {
  assert(
    (stat.mode & FORBIDDEN_MODE_BITS) === 0,
    `Snowflake runtime entry has unsafe write or set-id permissions: ${itemPath}`,
  );
  assert(
    (stat.mode & 0o400) !== 0 && (!isDirectory || (stat.mode & 0o100) !== 0),
    `Snowflake runtime entry lacks required owner permissions: ${itemPath}`,
  );
}

function assertHash(value, label) {
  assert(typeof value === "string" && /^[a-f0-9]{64}$/i.test(value), `${label} is invalid`);
}

function readRegularFile(filePath, label) {
  let stat;
  try {
    stat = lstatSync(filePath);
  } catch (error) {
    if (error.code === "ENOENT") throw new Error(`${label} is missing`);
    throw error;
  }
  assert(stat.isFile() && !stat.isSymbolicLink(), `${label} must be a regular file`);
  assertSafeMode(stat, filePath);
  return readFileSync(filePath);
}

function assertTree(root) {
  const rootStat = lstatSync(root);
  assert(
    rootStat.isDirectory() && !rootStat.isSymbolicLink(),
    "Snowflake runtime root must be a real directory",
  );
  assertSafeMode(rootStat, ".", true);
  const nodeModules = path.join(root, "helper", "node_modules");
  const realNodeModules = realpathSync(nodeModules);
  let entryCount = 0;
  const symlinks = [];

  function walk(directory, relativeDirectory = "", depth = 0) {
    assert(depth <= MAX_TREE_DEPTH, "Snowflake runtime tree exceeds the maximum depth");
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      entryCount += 1;
      assert(entryCount <= MAX_TREE_ENTRIES, "Snowflake runtime tree exceeds the entry limit");
      const relative = relativeDirectory ? `${relativeDirectory}/${entry.name}` : entry.name;
      const fullPath = path.join(directory, entry.name);
      const stat = lstatSync(fullPath);

      if (stat.isSymbolicLink()) {
        assert(
          relative.startsWith("helper/node_modules/"),
          `Snowflake runtime symlink is outside helper dependencies: ${relative}`,
        );
        const target = readlinkSync(fullPath);
        assert(!path.isAbsolute(target), `Snowflake runtime symlink must be relative: ${relative}`);
        const resolved = realpathSync(fullPath);
        assert(
          contained(realNodeModules, resolved),
          `Snowflake runtime symlink escapes dependencies: ${relative}`,
        );
        const targetStat = lstatSync(resolved);
        assert(
          targetStat.isFile() && !targetStat.isSymbolicLink(),
          `Snowflake symlink target must be a regular dependency file: ${relative}`,
        );
        symlinks.push(relative);
        continue;
      }

      if (stat.isDirectory()) {
        assertSafeMode(stat, relative, true);
        walk(fullPath, relative, depth + 1);
      } else {
        assert(stat.isFile(), `Unsupported Snowflake runtime entry type: ${relative}`);
        assertSafeMode(stat, relative);
      }
    }
  }

  walk(root);
  return { entryCount, symlinks };
}

function assertPosixRelativeFile(relative, label) {
  assert(
    typeof relative === "string" &&
      relative.length > 0 &&
      !relative.startsWith("/") &&
      !relative.includes("\\") &&
      relative.split("/").every((part) => part !== "" && part !== "." && part !== ".."),
    `${label} path is invalid`,
  );
}

function assertPathHash(root, relative, expected, label) {
  assertPosixRelativeFile(relative, label);
  assertHash(expected, `${label} hash`);
  const fullPath = path.resolve(root, ...relative.split("/"));
  assert(contained(root, fullPath), `${label} path escapes its root`);
  const bytes = readRegularFile(fullPath, label);
  assert(sha256(bytes) === expected, `${label} hash did not match the runtime manifest`);
}

function validateManifestAndBytes(
  root,
  nodeBinarySha256,
  nodeLicenseSha256,
  expectedPackageLockSha256,
  expectedSourceSha256,
  expectedProvenance,
) {
  const manifestBytes = readRegularFile(
    path.join(root, "manifest.json"),
    "Snowflake runtime manifest",
  );
  let manifest;
  try {
    manifest = JSON.parse(manifestBytes.toString("utf8"));
  } catch {
    throw new Error("Snowflake runtime manifest is not valid JSON");
  }
  assert(manifest?.format === MANIFEST_FORMAT, "Snowflake runtime manifest format is unsupported");
  assert(manifest?.target === "darwin-arm64", "Snowflake runtime target is not darwin-arm64");

  const runtime = manifest.runtime;
  assert(runtime?.version === NODE_VERSION, "Snowflake runtime Node version is not pinned");
  assert(
    runtime?.platform === "darwin" && runtime?.arch === "arm64",
    "Snowflake Node target is incorrect",
  );
  assert(
    runtime?.sha256 === nodeBinarySha256,
    "Snowflake runtime manifest has the wrong pinned Node hash",
  );
  assert(runtime?.archive === NODE_ARCHIVE_NAME, "Snowflake runtime archive name is not pinned");
  assert(
    runtime?.archiveSha256 === NODE_ARCHIVE_SHA256,
    "Snowflake runtime archive hash is not pinned",
  );
  assert(runtime?.npmVersion === NPM_VERSION, "Snowflake runtime npm version is not pinned");

  const nodeBytes = readRegularFile(path.join(root, "bin", "node"), "Pinned Snowflake Node binary");
  assert(sha256(nodeBytes) === nodeBinarySha256, "Pinned Snowflake Node binary hash changed");
  assert(
    (statSync(path.join(root, "bin", "node")).mode & 0o100) !== 0,
    "Pinned Node binary is not executable",
  );

  const helper = manifest.helper;
  assert(helper?.packageName === HELPER_PACKAGE, "Snowflake helper package name changed");
  assert(helper?.packageVersion === HELPER_VERSION, "Snowflake helper package version changed");
  assert(helper?.sdkVersion === SDK_VERSION, "Snowflake SDK version is not pinned");
  assertHash(helper?.packageLockSha256, "Snowflake helper package lock hash");
  assert(
    helper.packageLockSha256 === expectedPackageLockSha256,
    "Snowflake helper manifest does not match the repository package lock",
  );
  assert(
    JSON.stringify(Object.keys(helper?.sourceSha256 ?? {}).sort()) ===
      JSON.stringify([...HELPER_FILES].sort()),
    "Snowflake helper manifest must contain exactly the pinned source files",
  );
  for (const filename of HELPER_FILES) {
    assertPathHash(
      path.join(root, "helper"),
      filename,
      helper.sourceSha256[filename],
      `Snowflake helper source ${filename}`,
    );
    assert(
      helper.sourceSha256[filename] === expectedSourceSha256[filename],
      `Snowflake helper source ${filename} does not match the repository source`,
    );
  }

  const sdkManifestBytes = readRegularFile(
    path.join(root, "helper", "node_modules", "snowflake-sdk", "package.json"),
    "Installed Snowflake SDK package manifest",
  );
  let sdkManifest;
  try {
    sdkManifest = JSON.parse(sdkManifestBytes.toString("utf8"));
  } catch {
    throw new Error("Installed Snowflake SDK package manifest is invalid JSON");
  }
  assert(
    sdkManifest.name === "snowflake-sdk" && sdkManifest.version === SDK_VERSION,
    "Installed Snowflake SDK package version changed",
  );

  const notices = manifest.notices;
  assert(notices && typeof notices === "object", "Snowflake runtime notices are missing");
  assertPathHash(
    path.join(root, "notices"),
    "node/LICENSE",
    notices.nodeLicenseSha256,
    "Node release LICENSE",
  );
  assert(
    notices.nodeLicenseSha256 === nodeLicenseSha256,
    "Node release LICENSE does not match the independently pinned notice",
  );
  assert(Array.isArray(notices.packages), "Snowflake dependency notice inventory is missing");
  assert(
    notices.packages.length <= 10_000,
    "Snowflake dependency notice inventory exceeds its limit",
  );
  const copiedNoticePaths = new Map([["node/LICENSE", notices.nodeLicenseSha256]]);
  let noticeFileCount = 0;
  function validateListedNotice(file, prefix, label, allowSameHash) {
    assert(
      file && typeof file.path === "string" && file.path.startsWith(prefix),
      `${label} path is invalid`,
    );
    assert(file.path.length <= 512, `${label} path is too long`);
    noticeFileCount += 1;
    assert(noticeFileCount <= 20_000, "Snowflake notice inventory exceeds its limit");
    assertHash(file.sha256, `${label} hash`);
    const previousHash = copiedNoticePaths.get(file.path);
    if (previousHash !== undefined) {
      assert(allowSameHash && previousHash === file.sha256, `${label} path is duplicated`);
      return;
    }
    assertPathHash(path.join(root, "notices"), file.path, file.sha256, label);
    copiedNoticePaths.set(file.path, file.sha256);
  }
  for (const packageNotice of notices.packages) {
    assert(
      packageNotice && Array.isArray(packageNotice.files),
      "Snowflake dependency notice record is malformed",
    );
    for (const file of packageNotice.files) {
      validateListedNotice(file, "dependencies/", "Dependency notice", false);
    }
    if (packageNotice.supplements !== undefined) {
      assert(
        Array.isArray(packageNotice.supplements) && packageNotice.supplements.length <= 64,
        "Snowflake dependency supplement inventory is malformed",
      );
      for (const file of packageNotice.supplements) {
        validateListedNotice(file, "supplements/", "Dependency supplement", true);
      }
    }
  }
  assert(
    notices.provenance && typeof notices.provenance === "object",
    "Snowflake notice provenance is missing",
  );
  assert(
    notices.provenance.path === "supplements/provenance.json" &&
      notices.provenance.sha256 === expectedProvenance.sha256,
    "Snowflake notice provenance does not match repository evidence",
  );
  validateListedNotice(notices.provenance, "supplements/", "Snowflake notice provenance", false);
  let provenance;
  try {
    provenance = JSON.parse(expectedProvenance.bytes.toString("utf8"));
  } catch {
    throw new Error("Repository notice provenance is invalid JSON");
  }
  assert(
    provenance?.format === "joinery-snowflake-notices-v1" &&
      provenance.packageLockSha256 === expectedPackageLockSha256 &&
      Array.isArray(provenance.packages),
    "Repository notice provenance does not match the package lock",
  );
  const stagedPackages = new Map(notices.packages.map((item) => [item.lockPath, item]));
  assert(provenance.packages.length <= 10_000, "Repository notice provenance exceeds its limit");
  const sourcePackages = new Map(provenance.packages.map((item) => [item.lockPath, item]));
  for (const sourceRecord of provenance.packages) {
    const stagedRecord = stagedPackages.get(sourceRecord.lockPath);
    assert(
      stagedRecord &&
        stagedRecord.name === sourceRecord.name &&
        stagedRecord.version === sourceRecord.version,
      `Snowflake notice provenance package is missing: ${sourceRecord.lockPath}`,
    );
    const expected =
      sourceRecord.kind === "supplement" && sourceRecord.notice
        ? [
            {
              path: `supplements/${sourceRecord.notice.file}`,
              sha256: sourceRecord.notice.sha256,
            },
          ]
        : [];
    const actual = stagedRecord.supplements ?? [];
    assert(
      actual.length === expected.length &&
        actual.every(
          (item, index) =>
            item?.path === expected[index].path && item?.sha256 === expected[index].sha256,
        ),
      `Snowflake supplemental notice mapping changed: ${sourceRecord.lockPath}`,
    );
  }
  for (const stagedRecord of notices.packages) {
    const sourceRecord = sourcePackages.get(stagedRecord.lockPath);
    const expected =
      sourceRecord?.kind === "supplement" && sourceRecord.notice
        ? [{ path: `supplements/${sourceRecord.notice.file}`, sha256: sourceRecord.notice.sha256 }]
        : [];
    const actual = stagedRecord.supplements ?? [];
    assert(
      actual.length === expected.length &&
        actual.every(
          (item, index) =>
            item?.path === expected[index].path && item?.sha256 === expected[index].sha256,
        ),
      `Snowflake supplemental notice mapping changed: ${stagedRecord.lockPath}`,
    );
  }

  const rootEntries = readdirSync(root).sort();
  assert(
    JSON.stringify(rootEntries) === JSON.stringify(["bin", "helper", "manifest.json", "notices"]),
    "Snowflake runtime root contents are unexpected",
  );
  assert(
    JSON.stringify(readdirSync(path.join(root, "bin")).sort()) === JSON.stringify(["node"]),
    "Snowflake runtime bin contents are unexpected",
  );
  assert(
    HELPER_FILES.every((filename) => existsSync(path.join(root, "helper", filename))) &&
      existsSync(path.join(root, "helper", "node_modules")),
    "Snowflake helper runtime files are incomplete",
  );

  const tree = assertTree(root);
  return {
    manifest,
    manifestBytes,
    manifestSha256: sha256(manifestBytes),
    entryCount: tree.entryCount,
    symlinks: tree.symlinks,
  };
}

function runtimePaths(repoRoot, bundleContents, runtimeRoot) {
  const realRepoRoot = realpathSync(repoRoot);
  const cache = path.join(realRepoRoot, ".cache");
  assertRealDirectory(cache, "Repository .cache directory");
  const runtimeCache = path.join(cache, "snowflake-runtime");
  assertRealDirectory(runtimeCache, "Snowflake runtime cache directory");
  const stage = runtimeRoot ? path.resolve(runtimeRoot) : path.join(runtimeCache, "macos-arm64");
  assertRealDirectory(stage, "Prepared Snowflake runtime stage");
  const packageLock = readRegularFile(
    path.join(realRepoRoot, "snowflake-helper", "package-lock.json"),
    "Snowflake helper repository package lock",
  );
  const sourceSha256 = Object.fromEntries(
    HELPER_FILES.map((filename) => [
      filename,
      sha256(
        readRegularFile(
          path.join(realRepoRoot, "snowflake-helper", "src", filename),
          `Snowflake helper repository source ${filename}`,
        ),
      ),
    ]),
  );
  const provenanceBytes = readRegularFile(
    path.join(realRepoRoot, "snowflake-helper", "vendor", "notices", "provenance.json"),
    "Snowflake notice provenance source",
  );
  const contents = path.resolve(bundleContents);
  assertRealDirectory(contents, "Native smoke bundle Contents directory");
  const resources = path.join(contents, "Resources");
  if (existsSync(resources))
    assertRealDirectory(resources, "Native smoke bundle Resources directory");
  const destination = path.join(resources, "snowflake-runtime");
  return {
    stage,
    contents,
    resources,
    destination,
    packageLockSha256: sha256(packageLock),
    sourceSha256,
    provenance: { bytes: provenanceBytes, sha256: sha256(provenanceBytes) },
  };
}

function assertKnownGeneratedArtifact(destination) {
  let stat;
  try {
    stat = lstatSync(destination);
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
  assert(
    stat.isDirectory() && !stat.isSymbolicLink(),
    "Existing Snowflake resource path is not a real directory",
  );
  const manifestPath = path.join(destination, "manifest.json");
  const bytes = readRegularFile(manifestPath, "Existing Snowflake resource manifest");
  let manifest;
  try {
    manifest = JSON.parse(bytes.toString("utf8"));
  } catch {
    throw new Error("Refusing to replace an unknown Snowflake resource directory");
  }
  assert(
    manifest?.format === MANIFEST_FORMAT,
    "Refusing to replace an unknown Snowflake resource directory",
  );
  return true;
}

export function parseNativeSmokeArgs(args) {
  const options = { snowflakeResources: false };
  for (const argument of args) {
    if (argument === "--snowflake-resources" && !options.snowflakeResources) {
      options.snowflakeResources = true;
    } else {
      throw new Error(`Unknown or duplicate native smoke option: ${argument}`);
    }
  }
  return options;
}

export function validateSnowflakeResourceTree({
  repoRoot = scriptRoot,
  runtimeRoot = path.join(repoRoot, ".cache/snowflake-runtime/macos-arm64"),
  nodeBinarySha256 = NODE_BINARY_SHA256,
  nodeLicenseSha256 = NODE_LICENSE_SHA256,
} = {}) {
  const runtime = path.resolve(runtimeRoot);
  const paths = runtimePaths(repoRoot, path.dirname(runtime), runtime);
  const validated = validateManifestAndBytes(
    runtime,
    nodeBinarySha256,
    nodeLicenseSha256,
    paths.packageLockSha256,
    paths.sourceSha256,
    paths.provenance,
  );
  return {
    manifestSha256: validated.manifestSha256,
    entryCount: validated.entryCount,
    symlinkCount: validated.symlinks.length,
  };
}

export function copyPreparedSnowflakeRuntime({
  repoRoot = scriptRoot,
  bundleContents,
  nodeBinarySha256 = NODE_BINARY_SHA256,
  nodeLicenseSha256 = NODE_LICENSE_SHA256,
} = {}) {
  const initialPaths = runtimePaths(repoRoot, bundleContents);
  const validated = validateManifestAndBytes(
    initialPaths.stage,
    nodeBinarySha256,
    nodeLicenseSha256,
    initialPaths.packageLockSha256,
    initialPaths.sourceSha256,
    initialPaths.provenance,
  );
  assertKnownGeneratedArtifact(initialPaths.destination);

  if (!existsSync(initialPaths.resources)) mkdirSync(initialPaths.resources, { mode: 0o755 });
  assertRealDirectory(initialPaths.resources, "Native smoke bundle Resources directory");
  const tempParent = mkdtempSync(path.join(initialPaths.resources, ".snowflake-runtime-stage-"));
  const stagedCopy = path.join(tempParent, "snowflake-runtime");
  let backup;
  let movedOld = false;
  try {
    cpSync(initialPaths.stage, stagedCopy, {
      recursive: true,
      dereference: false,
      verbatimSymlinks: true,
      preserveTimestamps: true,
    });
    const copied = validateManifestAndBytes(
      stagedCopy,
      nodeBinarySha256,
      nodeLicenseSha256,
      initialPaths.packageLockSha256,
      initialPaths.sourceSha256,
      initialPaths.provenance,
    );
    assert(
      copied.manifestSha256 === validated.manifestSha256,
      "Copied Snowflake runtime manifest changed during staging",
    );

    if (existsSync(initialPaths.destination)) {
      assertKnownGeneratedArtifact(initialPaths.destination);
      backup = path.join(initialPaths.resources, `.snowflake-runtime-backup-${randomUUID()}`);
      renameSync(initialPaths.destination, backup);
      movedOld = true;
    }
    try {
      renameSync(stagedCopy, initialPaths.destination);
    } catch (error) {
      if (movedOld) renameSync(backup, initialPaths.destination);
      throw error;
    }
    let cleanupWarning;
    if (movedOld) {
      try {
        rmSync(backup, { recursive: true, force: false });
      } catch {
        cleanupWarning = "Previous Snowflake runtime resource backup could not be removed.";
      }
    }
    return {
      target: initialPaths.destination,
      manifestSha256: validated.manifestSha256,
      entryCount: validated.entryCount,
      symlinkCount: validated.symlinks.length,
      cleanupWarning,
    };
  } finally {
    rmSync(tempParent, { recursive: true, force: true });
  }
}

export function removePreparedSnowflakeRuntime({ bundleContents } = {}) {
  const contents = path.resolve(bundleContents);
  assertRealDirectory(contents, "Native smoke bundle Contents directory");
  const resources = path.join(contents, "Resources");
  if (!existsSync(resources)) return false;
  assertRealDirectory(resources, "Native smoke bundle Resources directory");
  const destination = path.join(resources, "snowflake-runtime");
  if (!assertKnownGeneratedArtifact(destination)) return false;
  rmSync(destination, { recursive: true, force: false });
  return true;
}
