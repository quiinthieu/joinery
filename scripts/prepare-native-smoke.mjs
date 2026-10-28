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
if (process.platform === "darwin") {
  // LaunchServices gives this hidden fixture its own process responsibility.
  // A directly spawned binary inherits the terminal/app's responsibility and
  // cannot safely distinguish its WebKit helpers from unrelated WebViews.
  const bundle = path.join(directory, "Joinery Native Smoke.app");
  const contents = path.join(bundle, "Contents");
  await mkdir(path.join(contents, "MacOS"), { recursive: true });
  await copyFile(binary, path.join(contents, "MacOS", "joinery-smoke"));
  await writeFile(
    path.join(contents, "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>joinery-smoke</string>
<key>CFBundleIdentifier</key><string>com.quiinthieu.joinery.native-smoke</string>
<key>CFBundleName</key><string>Joinery Native Smoke</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSUIElement</key><true/>
</dict></plist>
`,
  );
  execFileSync("codesign", ["--force", "--sign", "-", bundle], { stdio: "pipe" });
  build.bundle = bundle;
  build.bundleBinarySha256 = createHash("sha256")
    .update(await readFile(path.join(contents, "MacOS", "joinery-smoke")))
    .digest("hex");
}
await writeFile(path.join(directory, "build.json"), JSON.stringify(build, null, 2));
process.stdout.write(`Hidden native test executable: ${binary}\n`);
