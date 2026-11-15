// Reviewed transport sources, independent of the replaceable runtime manifest.
// Re-review request scheduling, retries, implicit SQL and redirects before
// changing these pins. This authenticates this bounded set, not every dependency.
import { createHash } from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const pins = [
  [
    "snowflake-sdk/package.json",
    "5515dbf4eeeb2b29f5acf8876000a9acde510e357f3ee8b3c4fa3c973bc1ea89",
  ],
  [
    "snowflake-sdk/dist/index.js",
    "0e2cd5e394091919bc737d39957e6aa8bb6a2856d38939b278d001223760d0cc",
  ],
  [
    "snowflake-sdk/dist/lib/connection/connection.js",
    "90a7e5c02f42f7191175677b55ec65e3553ee978121d01179b759804b5f40a06",
  ],
  [
    "snowflake-sdk/dist/lib/connection/connection_config.js",
    "48ad15fa7e55b248cd9fd9b1a000cba26bb0d37e7fc7501a478b960cb75e7af3",
  ],
  [
    "snowflake-sdk/dist/lib/connection/statement.js",
    "d3472ce19a64e6f7d09b55aaa0af21639e8a9cde69e1922e15e49e4ac4d4ae3f",
  ],
  [
    "snowflake-sdk/dist/lib/connection/result/result.js",
    "c93849d709f305ff33250c3f0197b6f96b03e174b51c031ea2641810edfde9c5",
  ],
  [
    "snowflake-sdk/dist/lib/util.js",
    "fb6ae19efd9cb1697abf947ef56e6e5019c3049330f2c88479315c58a3b955a0",
  ],
  [
    "snowflake-sdk/dist/lib/connection/bind_uploader.js",
    "683bc1bfb99766755eb64e1be4d97a4d5e39e7cfe40b8663dbbf15f277d5030a",
  ],
  [
    "snowflake-sdk/dist/lib/services/sf.js",
    "19bf0de8466178102f85148dc0cc43c16a83b3867dfecb26fc3d8e6d34d992df",
  ],
  [
    "snowflake-sdk/dist/lib/http/base.js",
    "7520a9feff97cfb89fca852138f8cddd05cf8571eccbe675db49cd725c449618",
  ],
  [
    "snowflake-sdk/dist/lib/http/axios_instance.js",
    "1e4924dbee1d1f50c53571921df944ff12a4aeac19c4e63a7d51768f43c084f4",
  ],
  ["axios/package.json", "b4ffd2888e63ccfcabc0e1f0deb316ac156138872f801145934d4a85897d0b80"],
  ["axios/dist/node/axios.cjs", "f21860a63687611d370ae3ffb70332887139a676c4f7cbec0eec32aa8ac2573d"],
  [
    "follow-redirects/package.json",
    "6d8de10cff3afbe74c7933a5424a22699ebb9528a792ae0212f8bdf83ed2866e",
  ],
  ["follow-redirects/index.js", "9f694132c5c09a94d6054b83f75460e45ebcc836a1e013c6e639b8961d352163"],
];

export const TRANSPORT_SOURCE_PINS = Object.freeze(pins.map((pin) => Object.freeze(pin)));

function fail() {
  // Never propagate a dependency path, parser input, URL or raw filesystem error.
  throw new Error("Snowflake transport source verification failed");
}

export function verifyTransportSources(entryUrl = import.meta.url) {
  try {
    const entry = fileURLToPath(entryUrl);
    const directory = path.dirname(entry);
    // Source checkout/preparation uses src/, packaged helpers are flattened.
    const helper = path.basename(directory) === "src" ? path.dirname(directory) : directory;
    const modules = path.join(helper, "node_modules");
    const localRequire = createRequire(entryUrl);
    const sdkPackage = localRequire.resolve("snowflake-sdk/package.json");
    // Resolve from the exact importing source, not the package root. A deeper
    // node_modules directory can shadow a package even with identical bytes.
    const sdkRequire = createRequire(
      path.join(path.dirname(sdkPackage), "dist/lib/http/axios_instance.js"),
    );
    const axiosPackage = sdkRequire.resolve("axios/package.json");
    const axiosRequire = createRequire(sdkRequire.resolve("axios"));
    const expectedResolution = [
      [sdkPackage, "snowflake-sdk/package.json"],
      [localRequire.resolve("snowflake-sdk"), "snowflake-sdk/dist/index.js"],
      [axiosPackage, "axios/package.json"],
      [sdkRequire.resolve("axios"), "axios/dist/node/axios.cjs"],
      [axiosRequire.resolve("follow-redirects/package.json"), "follow-redirects/package.json"],
      [axiosRequire.resolve("follow-redirects"), "follow-redirects/index.js"],
    ];
    for (const [resolved, expected] of expectedResolution) {
      if (resolved !== path.join(modules, expected)) fail();
    }
    for (const [relative, expected] of TRANSPORT_SOURCE_PINS) {
      const filename = path.join(modules, relative);
      for (let parent = path.dirname(filename); ; parent = path.dirname(parent)) {
        const state = lstatSync(parent);
        if (!state.isDirectory() || state.isSymbolicLink()) fail();
        if (parent === modules) break;
        if (parent === path.dirname(parent)) fail();
      }
      const state = lstatSync(filename);
      if (!state.isFile() || state.isSymbolicLink() || state.size > 4 * 1024 * 1024) fail();
      const bytes = readFileSync(filename);
      if (
        bytes.length !== state.size ||
        createHash("sha256").update(bytes).digest("hex") !== expected
      )
        fail();
    }
    return Object.freeze({ format: "joinery-snowflake-transport-pins-v1", files: pins.length });
  } catch {
    fail();
  }
}
