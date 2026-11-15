import { fileURLToPath } from "node:url";
import { selectRuntime } from "./verify-snowflake-runtime.mjs";
import { validateSnowflakeResourceTree } from "./native-smoke-resources.mjs";

// Read-only packaging gate. Does not spawn the helper, authenticate or run SQL.
const repoRoot = fileURLToPath(new URL("../", import.meta.url));
if (process.platform !== "darwin" || process.arch !== "arm64")
  throw new Error("Snowflake resource packaging requires macOS ARM64");
const selected = selectRuntime(process.argv.slice(2), repoRoot);
const result = validateSnowflakeResourceTree({ repoRoot, runtimeRoot: selected.runtime });
process.stdout.write(
  `${JSON.stringify({
    passed: true,
    check: "snowflake-package-resources",
    resourceKind: selected.kind,
    manifestSha256: result.manifestSha256,
    entryCount: result.entryCount,
    symlinkCount: result.symlinkCount,
  })}\n`,
);
