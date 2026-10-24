import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { execFileSync } from "node:child_process";

const extension = process.platform === "win32" ? ".exe" : "";
const source = path.resolve(`src-tauri/target/release/joinery${extension}`);
const directory = path.resolve(".cache/native-smoke");
await mkdir(directory, { recursive: true });
const binary = path.join(directory, `joinery-smoke${extension}`);
await copyFile(source, binary);
const sha256 = createHash("sha256")
  .update(await readFile(binary))
  .digest("hex");
const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const build = {
  binary,
  sha256,
  sourceRevision: git("rev-parse", "HEAD"),
  sourceTree: git("rev-parse", "HEAD^{tree}"),
  sourceDirty: git("status", "--porcelain").length > 0,
  platform: process.platform,
  architecture: process.arch,
  builtAt: new Date().toISOString(),
};
await writeFile(path.join(directory, "build.json"), JSON.stringify(build, null, 2));
process.stdout.write(`Hidden native test executable: ${binary}\n`);
