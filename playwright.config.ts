import { defineConfig } from "@playwright/test";
import path from "node:path";

const token = "joinery-disposable-browser-tests-only";
export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 45000,
  expect: { timeout: 10000 },
  reporter: "list",
  use: { baseURL: "http://127.0.0.1:1421", headless: true, viewport: { width: 1440, height: 1000 }, trace: "retain-on-failure", screenshot: "only-on-failure" },
  webServer: [
    {
      command: "cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --bin test-harness",
      url: "http://127.0.0.1:1422/health",
      timeout: 180000,
      reuseExistingServer: false,
      env: { JOINERY_TEST_BRIDGE: "1", JOINERY_TEST_TOKEN: token, JOINERY_TEST_DATA_ROOT: path.resolve(`.cache/e2e-${Date.now()}`) },
    },
    {
      command: "pnpm exec vite --mode integration --host 127.0.0.1 --port 1421",
      url: "http://127.0.0.1:1421",
      timeout: 30000,
      reuseExistingServer: false,
      env: { VITE_TEST_BRIDGE_URL: "http://127.0.0.1:1422", VITE_TEST_BRIDGE_TOKEN: token },
    },
  ],
});
