import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";

if (process.platform !== "darwin") {
  throw new Error("macOS bundle verification requires macOS");
}
const target = path.resolve(process.env.CARGO_TARGET_DIR ?? "src-tauri/target");
const bundle = path.join(target, "release/bundle/macos/Joinery.app");
const verify = () =>
  spawnSync("codesign", ["--verify", "--deep", "--strict", bundle], { encoding: "utf8" });
const signature = spawnSync("codesign", ["--display", "--verbose=2", bundle], { encoding: "utf8" });
if (signature.error) throw signature.error;
if (signature.status !== 0) throw new Error("The macOS bundle has no inspectable signature");
const adHoc = /^Signature=adhoc$/m.test(signature.stderr);
const original = verify();
if (original.error) throw original.error;
if (original.status !== 0) {
  if (!adHoc)
    throw new Error(
      "The signed macOS bundle failed verification; repair signing configuration. Its identity has been preserved.",
    );
  // The Rust linker signature does not seal the assembled app's resources.
  execFileSync("codesign", ["--force", "--sign", "-", bundle], { stdio: "inherit" });
}
execFileSync("codesign", ["--verify", "--deep", "--strict", "--verbose=2", bundle], {
  stdio: "inherit",
});
process.stdout.write(
  adHoc
    ? "Local macOS bundle verified with ad-hoc signing; Developer ID and notarization remain pending.\n"
    : "macOS signing verified; notarization status was not checked.\n",
);
