import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  closeSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash, randomUUID } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const NODE_VERSION = "24.22.2";
export const NODE_ARCHIVE_NAME = `node-v${NODE_VERSION}-darwin-arm64.tar.gz`;
export const NODE_ARCHIVE_SHA256 =
  "aa9549349e47dffaa42dcf95d6e06ade8c4bdafc980979883b88767670df385a";
export const NODE_BINARY_SHA256 =
  "b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a";
export const NPM_VERSION = "11.19.1";
const NODE_SIGNING_TEAM = "HX7739G8FX";
const NODE_SIGNING_AUTHORITY = "Node.js Foundation";
const HELPER_FILES = ["main.mjs", "helper.mjs", "framing.mjs"];
const NOTICE_NAME = /^(?:license|licence|copying|notice)(?:\..*)?$/i;
const README_NAME = /^readme(?:\..*)?$/i;

const scriptPath = fileURLToPath(import.meta.url);
const scriptRoot = path.resolve(path.dirname(scriptPath), "..");

function digestFile(filePath, algorithm = "sha256") {
  return createHash(algorithm).update(readFileSync(filePath)).digest("hex");
}

function requireStatus(result, label) {
  if (result.error) throw new Error(`${label} could not start`);
  if (result.status !== 0)
    throw new Error(`${label} failed with exit status ${String(result.status)}`);
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

function runProcess(command, args, options = {}) {
  return spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
    stdio: "pipe",
    timeout: options.timeout ?? 120_000,
    maxBuffer: 8 * 1024 * 1024,
  });
}

function assertNoSymlinkDirectory(directory, label) {
  const stat = lstatSync(directory);
  if (!stat.isDirectory() || stat.isSymbolicLink())
    throw new Error(`${label} must be a real directory`);
}

function assertContained(root, candidate, label) {
  const relative = path.relative(root, candidate);
  if (
    !relative ||
    relative === ".." ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  )
    throw new Error(`${label} must be inside the repository cache root`);
}

function assertTreeSymlinksContained(directory, containmentRoot = directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isSymbolicLink()) {
      const target = realpathSync(entryPath);
      assertContained(containmentRoot, target, "Node dependency symlink target");
    } else if (entry.isDirectory()) {
      assertTreeSymlinksContained(entryPath, containmentRoot);
    }
  }
}

function ensureCacheRoot(repoRoot) {
  const realRepoRoot = realpathSync(repoRoot);
  const cacheDirectory = path.join(realRepoRoot, ".cache");
  if (existsSync(cacheDirectory)) assertNoSymlinkDirectory(cacheDirectory, ".cache");
  else mkdirSync(cacheDirectory, { mode: 0o700 });

  const runtimeRoot = path.join(cacheDirectory, "snowflake-runtime");
  if (existsSync(runtimeRoot)) assertNoSymlinkDirectory(runtimeRoot, "Snowflake runtime cache");
  else mkdirSync(runtimeRoot, { mode: 0o700 });
  const realRuntimeRoot = realpathSync(runtimeRoot);
  if (realRuntimeRoot !== runtimeRoot)
    throw new Error("Snowflake runtime cache resolved outside its expected path");
  return { realRepoRoot, runtimeRoot: realRuntimeRoot };
}

function ensurePackageCache(repoRoot, runtimeRoot, requestedCache) {
  const cacheRoot = path.join(repoRoot, ".cache");
  const requested = requestedCache
    ? path.resolve(repoRoot, requestedCache)
    : path.join(runtimeRoot, "npm-cache");
  assertContained(cacheRoot, requested, "npm cache directory");
  const relative = path.relative(cacheRoot, requested).split(path.sep).filter(Boolean);
  let current = cacheRoot;
  for (const part of relative) {
    current = path.join(current, part);
    if (existsSync(current)) assertNoSymlinkDirectory(current, "npm cache path component");
    else mkdirSync(current, { mode: 0o700 });
  }
  if (realpathSync(requested) !== requested)
    throw new Error("npm cache directory resolved outside its repository cache path");
  return requested;
}

function assertChecksumFile(checksumFile, expectedSha256) {
  const entries = readFileSync(checksumFile, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim().match(/^([a-f\d]{64})\s+\*?(.+)$/i))
    .filter(Boolean);
  const entry = entries.find(([, , filename]) => filename === NODE_ARCHIVE_NAME);
  if (!entry || entry[1].toLowerCase() !== expectedSha256)
    throw new Error(
      "Pinned Node archive checksum is missing or does not match the expected release",
    );
}

function verifyRuntime(nodePath, { runCommand, hashNode = NODE_BINARY_SHA256 }) {
  if (digestFile(nodePath) !== hashNode)
    throw new Error("Extracted Node binary hash did not match the pinned binary");
  const mode = statSync(nodePath).mode;
  if ((mode & 0o111) === 0) throw new Error("Extracted Node binary is not executable");
  const cleanEnv = { PATH: "/usr/bin:/bin:/usr/sbin:/sbin", LANG: "en_US.UTF-8" };
  const version = requireStatus(
    runCommand(nodePath, ["--version"], { env: cleanEnv }),
    "Node version check",
  ).trim();
  const platform = requireStatus(
    runCommand(nodePath, ["-p", "process.platform"], { env: cleanEnv }),
    "Node platform check",
  ).trim();
  const arch = requireStatus(
    runCommand(nodePath, ["-p", "process.arch"], { env: cleanEnv }),
    "Node architecture check",
  ).trim();
  if (version !== `v${NODE_VERSION}` || platform !== "darwin" || arch !== "arm64")
    throw new Error(`Pinned Node check failed (${version}, ${platform}, ${arch})`);

  const signature = requireStatus(
    runCommand("/usr/bin/codesign", ["--verify", "--strict", "--verbose=2", nodePath], {
      env: cleanEnv,
    }),
    "Node code-signature verification",
  );
  const details = requireStatus(
    runCommand("/usr/bin/codesign", ["--display", "--verbose=4", nodePath], {
      env: cleanEnv,
    }),
    "Node code-signature inspection",
  );
  const combined = `${signature}\n${details}`;
  if (
    !combined.includes(`TeamIdentifier=${NODE_SIGNING_TEAM}`) ||
    !combined.includes(NODE_SIGNING_AUTHORITY)
  )
    throw new Error("Node signature did not match the pinned Node.js Foundation signing identity");
  return {
    version: NODE_VERSION,
    platform,
    arch,
    sha256: digestFile(nodePath),
    teamIdentifier: NODE_SIGNING_TEAM,
    authority: NODE_SIGNING_AUTHORITY,
  };
}

function copyHelperInputs(helperSource, packageRoot) {
  for (const filename of ["package.json", "package-lock.json"]) {
    const source = path.join(helperSource, filename);
    if (!existsSync(source) || lstatSync(source).isSymbolicLink() || !lstatSync(source).isFile())
      throw new Error(`Snowflake helper source is missing or unsafe: ${filename}`);
    copyFileSync(source, path.join(packageRoot, filename));
  }
  mkdirSync(path.join(packageRoot, "src"), { recursive: true, mode: 0o700 });
  for (const filename of HELPER_FILES) {
    const source = path.join(helperSource, "src", filename);
    if (!existsSync(source) || lstatSync(source).isSymbolicLink() || !lstatSync(source).isFile())
      throw new Error(`Snowflake helper source is missing or unsafe: src/${filename}`);
    copyFileSync(source, path.join(packageRoot, "src", filename));
  }
}

function copyNotice(source, destination) {
  const item = lstatSync(source);
  if (!item.isFile() || item.isSymbolicLink()) return null;
  mkdirSync(path.dirname(destination), { recursive: true, mode: 0o700 });
  copyFileSync(source, destination);
  return { sha256: digestFile(destination) };
}

function collectDependencyNotices(packageRoot, noticesRoot, packageLock) {
  const nodeModulesRoot = path.join(packageRoot, "node_modules");
  const packages = [];
  for (const [lockPath, lockInfo] of Object.entries(packageLock.packages ?? {})) {
    if (!lockPath.startsWith("node_modules/")) continue;
    const relativePackagePath = lockPath.slice("node_modules/".length);
    const installedPath = path.resolve(nodeModulesRoot, relativePackagePath);
    assertContained(nodeModulesRoot, installedPath, "Locked package path");
    if (!existsSync(installedPath)) continue;
    const packageStat = lstatSync(installedPath);
    if (packageStat.isSymbolicLink() || !packageStat.isDirectory())
      throw new Error(`Installed dependency is not a real package directory: ${lockPath}`);
    const packageJsonPath = path.join(installedPath, "package.json");
    if (!existsSync(packageJsonPath))
      throw new Error(`Installed dependency has no package.json: ${lockPath}`);
    const manifest = JSON.parse(readFileSync(packageJsonPath, "utf8"));
    const rootFiles = readdirSync(installedPath, { withFileTypes: true })
      .filter((entry) => entry.isFile() && !entry.isSymbolicLink())
      .map((entry) => entry.name);
    const licenseFiles = rootFiles.filter((name) => NOTICE_NAME.test(name));
    const readmeFiles = licenseFiles.length
      ? []
      : rootFiles.filter((name) => README_NAME.test(name));
    const candidates = licenseFiles.length ? licenseFiles : readmeFiles;
    const destinationPart = encodeURIComponent(relativePackagePath);
    const files = candidates.map((filename) => {
      const copied = copyNotice(
        path.join(installedPath, filename),
        path.join(noticesRoot, "dependencies", destinationPart, filename),
      );
      if (!copied) throw new Error(`Could not preserve dependency notice: ${lockPath}/${filename}`);
      return {
        path: path.posix.join("dependencies", destinationPart, filename),
        sha256: copied.sha256,
      };
    });
    const declaredLicense = manifest.license ?? lockInfo.license ?? null;
    let evidence = licenseFiles.length
      ? "package-license-files"
      : readmeFiles.length
        ? "readme-only"
        : "no-root-notice-found";
    const notes = [];
    if (manifest.name === "proxy-agent-negotiate" && manifest.version === "1.1.0")
      notes.push(
        "MIT is declared, but the bounded source audit found no notice text in the exact tarball or checked upstream tag.",
      );
    if (manifest.name === "asn1.js-rfc2560" && manifest.version === "5.0.1")
      notes.push(
        "The project MIT notice exists upstream, but the exact npm source-to-tag mapping remains unresolved.",
      );
    packages.push({
      name: manifest.name,
      version: manifest.version,
      lockPath,
      declaredLicense,
      evidence,
      files,
      notes,
    });
  }
  packages.sort((left, right) => left.lockPath.localeCompare(right.lockPath));
  return {
    packages,
    packagesWithNoNoticeFile: packages
      .filter((item) => item.evidence === "no-root-notice-found")
      .map((item) => `${item.name}@${item.version}`),
    packagesWithReadmeOnly: packages
      .filter((item) => item.evidence === "readme-only")
      .map((item) => `${item.name}@${item.version}`),
    knownEvidenceGaps: packages
      .filter((item) => item.notes.length)
      .map((item) => ({ package: `${item.name}@${item.version}`, notes: item.notes })),
  };
}

function cleanupScratch(directory, runtimeRoot) {
  if (!directory || !existsSync(directory)) return;
  const basename = path.basename(directory);
  if (!basename.startsWith(".macos-arm64-stage-"))
    throw new Error("Refusing to remove an unowned staging directory");
  const realDirectory = realpathSync(directory);
  assertContained(runtimeRoot, realDirectory, "Staging directory");
  if (lstatSync(directory).isSymbolicLink())
    throw new Error("Refusing to remove a staging symlink");
  rmSync(directory, { recursive: true, force: false });
}

function promoteStage(stage, target, runtimeRoot, removeBackup = rmSync) {
  const backup = path.join(runtimeRoot, `.macos-arm64-backup-${randomUUID()}`);
  let movedOld = false;
  if (existsSync(target)) {
    assertNoSymlinkDirectory(target, "Existing Snowflake runtime");
    assertContained(runtimeRoot, realpathSync(target), "Existing Snowflake runtime");
    renameSync(target, backup);
    movedOld = true;
  }
  try {
    renameSync(stage, target);
  } catch (error) {
    if (movedOld) renameSync(backup, target);
    throw error;
  }
  const cleanupWarnings = [];
  if (movedOld) {
    assertNoSymlinkDirectory(backup, "Previous Snowflake runtime backup");
    assertContained(runtimeRoot, realpathSync(backup), "Previous Snowflake runtime backup");
    try {
      removeBackup(backup, { recursive: true, force: false });
    } catch {
      cleanupWarnings.push(
        "Previous runtime backup could not be removed and remains in the cache.",
      );
    }
  }
  return cleanupWarnings;
}

export function prepareSnowflakeRuntime({
  repoRoot = scriptRoot,
  platform = process.platform,
  arch = process.arch,
  runCommand = runProcess,
  archiveSha256 = NODE_ARCHIVE_SHA256,
  nodeBinarySha256 = NODE_BINARY_SHA256,
  chmod = chmodSync,
  offline = false,
  cacheDir,
  removeBackup = rmSync,
} = {}) {
  if (platform !== "darwin" || arch !== "arm64")
    throw new Error(
      `Snowflake stateful runtime packaging supports only darwin-arm64; got ${platform}-${arch}`,
    );

  const realRepoRoot = realpathSync(repoRoot);
  const { runtimeRoot } = ensureCacheRoot(realRepoRoot);
  const npmCache = ensurePackageCache(realRepoRoot, runtimeRoot, cacheDir);
  const target = path.join(runtimeRoot, "macos-arm64");
  assertContained(runtimeRoot, target, "Snowflake runtime target");
  const lockPath = path.join(runtimeRoot, ".macos-arm64.prepare.lock");
  let lockFd;
  try {
    lockFd = openSync(lockPath, "wx", 0o600);
  } catch (error) {
    if (error.code === "EEXIST")
      throw new Error("Snowflake runtime preparation is already running or left a lock file");
    throw error;
  }

  let stage;
  try {
    const toolsRoot = path.join(realRepoRoot, ".cache", "toolchains");
    const archive = path.join(toolsRoot, NODE_ARCHIVE_NAME);
    const checksums = path.join(toolsRoot, "node24-shasums.txt");
    const helperSource = path.join(realRepoRoot, "snowflake-helper");
    if (!existsSync(archive) || !existsSync(checksums))
      throw new Error(`Pinned Node archive/checksum cache is missing under ${toolsRoot}`);
    assertNoSymlinkDirectory(toolsRoot, "Pinned toolchain directory");
    if (lstatSync(archive).isSymbolicLink() || !lstatSync(archive).isFile())
      throw new Error("Pinned Node archive must be a regular file");
    if (lstatSync(checksums).isSymbolicLink() || !lstatSync(checksums).isFile())
      throw new Error("Pinned Node checksum list must be a regular file");
    assertChecksumFile(checksums, archiveSha256);
    if (digestFile(archive) !== archiveSha256)
      throw new Error("Cached Node archive hash did not match the pinned release");
    if (
      !existsSync(helperSource) ||
      !lstatSync(helperSource).isDirectory() ||
      lstatSync(helperSource).isSymbolicLink()
    )
      throw new Error("Repository snowflake-helper source directory is missing or unsafe");

    stage = mkdtempSync(path.join(runtimeRoot, ".macos-arm64-stage-"));
    const buildRoot = path.join(stage, ".build");
    const extractRoot = path.join(buildRoot, "node-extract");
    const packageRoot = path.join(buildRoot, "helper-package");
    const npmStateRoot = path.join(buildRoot, "npm-state");
    mkdirSync(extractRoot, { recursive: true, mode: 0o700 });
    mkdirSync(packageRoot, { recursive: true, mode: 0o700 });
    mkdirSync(npmStateRoot, { recursive: true, mode: 0o700 });

    execFileSync(
      "/usr/bin/tar",
      [
        "-xzf",
        archive,
        "-C",
        extractRoot,
        "--no-same-owner",
        "--no-same-permissions",
        "--strip-components=1",
        `node-v${NODE_VERSION}-darwin-arm64/bin/node`,
        `node-v${NODE_VERSION}-darwin-arm64/LICENSE`,
        `node-v${NODE_VERSION}-darwin-arm64/lib/node_modules/npm`,
      ],
      { stdio: "pipe" },
    );
    const extractedNode = path.join(extractRoot, "bin", "node");
    const runtimeNode = path.join(stage, "bin", "node");
    mkdirSync(path.dirname(runtimeNode), { recursive: true, mode: 0o700 });
    copyFileSync(extractedNode, runtimeNode);
    chmod(runtimeNode, 0o755);
    const runtimeInfo = verifyRuntime(runtimeNode, { runCommand, hashNode: nodeBinarySha256 });
    const nodeLicense = path.join(extractRoot, "LICENSE");
    if (!existsSync(nodeLicense))
      throw new Error("Pinned Node archive did not contain its LICENSE file");
    mkdirSync(path.join(stage, "notices", "node"), { recursive: true, mode: 0o700 });
    copyFileSync(nodeLicense, path.join(stage, "notices", "node", "LICENSE"));

    const npmRoot = path.join(extractRoot, "lib", "node_modules", "npm");
    const npmCli = path.join(npmRoot, "bin", "npm-cli.js");
    const npmManifestPath = path.join(npmRoot, "package.json");
    if (!existsSync(npmCli) || !existsSync(npmManifestPath))
      throw new Error("Pinned Node archive did not contain its bundled npm CLI");
    const npmManifest = JSON.parse(readFileSync(npmManifestPath, "utf8"));
    if (npmManifest.name !== "npm" || npmManifest.version !== NPM_VERSION)
      throw new Error(
        `Expected archive-bundled npm ${NPM_VERSION}; found ${String(npmManifest.version)}`,
      );

    copyHelperInputs(helperSource, packageRoot);
    const helperPackage = JSON.parse(readFileSync(path.join(packageRoot, "package.json"), "utf8"));
    const packageLock = JSON.parse(
      readFileSync(path.join(packageRoot, "package-lock.json"), "utf8"),
    );
    if (helperPackage.dependencies?.["snowflake-sdk"] !== "3.4.1")
      throw new Error("Snowflake helper must pin snowflake-sdk exactly to 3.4.1");
    if (
      packageLock.lockfileVersion !== 3 ||
      packageLock.packages?.[""]?.dependencies?.["snowflake-sdk"] !== "3.4.1"
    )
      throw new Error("Snowflake helper lockfile does not match the pinned SDK package");
    for (const [lockPath, packageInfo] of Object.entries(packageLock.packages ?? {})) {
      if (!lockPath.startsWith("node_modules/")) continue;
      if (packageInfo.link)
        throw new Error(`Local or linked npm dependency is not allowed: ${lockPath}`);
      if (!packageInfo.resolved || !packageInfo.integrity)
        throw new Error(`Locked npm package is missing registry URL or integrity: ${lockPath}`);
      const resolved = new URL(packageInfo.resolved);
      if (resolved.protocol !== "https:" || resolved.hostname !== "registry.npmjs.org")
        throw new Error(`Locked npm package is outside the public npm registry: ${lockPath}`);
    }

    const home = path.join(npmStateRoot, "home");
    const temp = path.join(npmStateRoot, "tmp");
    const userConfig = path.join(npmStateRoot, "user.npmrc");
    const globalConfig = path.join(npmStateRoot, "global.npmrc");
    for (const directory of [home, temp]) mkdirSync(directory, { recursive: true, mode: 0o700 });
    writeFileSync(userConfig, "", { mode: 0o600 });
    writeFileSync(globalConfig, "", { mode: 0o600 });
    const npmEnv = {
      CI: "true",
      HOME: home,
      LANG: "en_US.UTF-8",
      PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
      TMPDIR: temp,
      npm_config_cache: npmCache,
      npm_config_globalconfig: globalConfig,
      npm_config_userconfig: userConfig,
      npm_config_update_notifier: "false",
    };
    const npmVersion = requireStatus(
      runCommand(runtimeNode, [npmCli, "--version"], { cwd: packageRoot, env: npmEnv }),
      "Pinned npm CLI version check",
    ).trim();
    if (npmVersion !== NPM_VERSION)
      throw new Error(`Expected cached npm CLI ${NPM_VERSION}; found ${npmVersion}`);
    requireStatus(
      runCommand(
        runtimeNode,
        [
          npmCli,
          "ci",
          "--omit=dev",
          "--ignore-scripts",
          "--no-audit",
          "--no-fund",
          `--cache=${npmCache}`,
          `--userconfig=${userConfig}`,
          `--globalconfig=${globalConfig}`,
          "--registry=https://registry.npmjs.org/",
          ...(offline ? ["--offline"] : []),
        ],
        { cwd: packageRoot, env: npmEnv, timeout: 300_000 },
      ),
      "Production dependency installation",
    );

    const helperTarget = path.join(stage, "helper");
    mkdirSync(helperTarget, { recursive: true, mode: 0o700 });
    for (const filename of HELPER_FILES)
      copyFileSync(path.join(packageRoot, "src", filename), path.join(helperTarget, filename));
    assertTreeSymlinksContained(path.join(packageRoot, "node_modules"));
    cpSync(path.join(packageRoot, "node_modules"), path.join(helperTarget, "node_modules"), {
      recursive: true,
      dereference: false,
      verbatimSymlinks: true,
      preserveTimestamps: true,
    });
    assertTreeSymlinksContained(path.join(helperTarget, "node_modules"));

    const noticesRoot = path.join(stage, "notices");
    const noticeInventory = collectDependencyNotices(packageRoot, noticesRoot, packageLock);
    const manifest = {
      format: "joinery-snowflake-runtime-v1",
      target: "darwin-arm64",
      runtime: {
        ...runtimeInfo,
        archive: NODE_ARCHIVE_NAME,
        archiveSha256,
        npmVersion,
        npmCliSha256: digestFile(npmCli),
      },
      helper: {
        packageName: helperPackage.name,
        packageVersion: helperPackage.version,
        sdkVersion: helperPackage.dependencies["snowflake-sdk"],
        packageLockSha256: digestFile(path.join(packageRoot, "package-lock.json")),
        sourceSha256: Object.fromEntries(
          HELPER_FILES.map((filename) => [filename, digestFile(path.join(helperTarget, filename))]),
        ),
      },
      notices: {
        nodeLicenseSha256: digestFile(path.join(noticesRoot, "node", "LICENSE")),
        ...noticeInventory,
        disclaimer:
          "Root license/notice files and README fallbacks found in the installed lockfile packages are copied verbatim. This is not the final legal audit; unresolved source and notice evidence must remain visible.",
      },
    };
    writeFileSync(path.join(stage, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, {
      mode: 0o644,
    });

    rmSync(buildRoot, { recursive: true, force: false });
    const stagedNode = statSync(runtimeNode);
    if ((stagedNode.mode & 0o111) === 0) throw new Error("Staged Node lost executable permissions");
    if (digestFile(runtimeNode) !== nodeBinarySha256)
      throw new Error("Staged Node hash changed during preparation");
    const cleanupWarnings = promoteStage(stage, target, runtimeRoot, removeBackup);
    stage = undefined;
    return { target, manifest, cleanupWarnings };
  } catch (error) {
    if (stage) cleanupScratch(stage, runtimeRoot);
    throw error;
  } finally {
    closeSync(lockFd);
    rmSync(lockPath, { force: true });
  }
}

function parseCliArgs(args) {
  const result = { offline: false, cacheDir: undefined };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--offline") result.offline = true;
    else if (argument === "--cache-dir") {
      const value = args[index + 1];
      if (!value || value.startsWith("--"))
        throw new Error("--cache-dir requires a path under this repository's .cache directory");
      result.cacheDir = value;
      index += 1;
    } else {
      throw new Error(`Unknown option: ${argument}`);
    }
  }
  return result;
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    const result = prepareSnowflakeRuntime(parseCliArgs(process.argv.slice(2)));
    process.stdout.write(`Prepared Snowflake runtime at ${result.target}\n`);
    for (const warning of result.cleanupWarnings)
      process.stderr.write(`Snowflake runtime warning: ${warning}\n`);
  } catch (error) {
    process.stderr.write(`Snowflake runtime preparation failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
