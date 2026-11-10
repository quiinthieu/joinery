import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
if (process.platform === "win32") {
  throw new Error("Snowflake helper process qualification currently requires Unix");
}
for (const args of [
  ["--lib", "snowflake_"],
  ["--features", "test-harness", "--lib", "execution::snowflake_connection_tests"],
  ["--features", "test-harness", "--test", "snowflake_process"],
  ["--features", "test-harness", "--test", "snowflake_external"],
]) {
  const child = spawn(
    "cargo",
    ["test", "--manifest-path", "src-tauri/Cargo.toml", "--no-default-features", ...args],
    {
      cwd: root,
      env: { ...process.env, JOINERY_TEST_NODE: process.execPath },
      stdio: "inherit",
    },
  );
  const code = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve(signal ? 1 : (code ?? 1)));
  });
  if (code !== 0) process.exit(code);
}
