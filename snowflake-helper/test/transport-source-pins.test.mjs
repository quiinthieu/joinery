import assert from "node:assert/strict";
import test from "node:test";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { TRANSPORT_SOURCE_PINS, verifyTransportSources } from "../src/transport-source-pins.mjs";

const helper = fileURLToPath(new URL("../", import.meta.url));
const message = "Snowflake transport source verification failed";

function fixture(layout = "flat") {
  const root = realpathSync(mkdtempSync(path.join(tmpdir(), "joinery-transport-pins-")));
  const entry = path.join(root, layout === "source" ? "src/main.mjs" : "main.mjs");
  mkdirSync(path.dirname(entry), { recursive: true });
  writeFileSync(entry, "// Synthetic entry: dependencies are never imported.\n");
  for (const [relative] of TRANSPORT_SOURCE_PINS) {
    const target = path.join(root, "node_modules", relative);
    mkdirSync(path.dirname(target), { recursive: true });
    copyFileSync(path.join(helper, "node_modules", relative), target);
  }
  return {
    root,
    entry: pathToFileURL(entry),
    file: (relative) => path.join(root, "node_modules", relative),
    close: () => rmSync(root, { recursive: true, force: true }),
  };
}

function rejects(entry) {
  assert.throws(
    () => verifyTransportSources(entry),
    (error) => {
      assert.equal(error.message, message);
      assert.equal(error.cause, undefined);
      return true;
    },
  );
}

for (const layout of ["source", "flat"]) {
  test(`transport source pins verify locked bytes in ${layout} helper layout`, () => {
    const f = fixture(layout);
    try {
      assert.deepEqual(verifyTransportSources(f.entry), {
        format: "joinery-snowflake-transport-pins-v1",
        files: 13,
      });
      assert(Object.isFrozen(verifyTransportSources(f.entry)));
    } finally {
      f.close();
    }
  });
}

test("each reviewed transport source rejects changed bytes without a manifest override", () => {
  const f = fixture();
  try {
    for (const [relative] of TRANSPORT_SOURCE_PINS) {
      const target = f.file(relative);
      const original = readFileSync(target);
      writeFileSync(target, Buffer.concat([original, Buffer.from("\n")]));
      rejects(f.entry);
      writeFileSync(target, original);
      verifyTransportSources(f.entry);
    }
    assert(Object.isFrozen(TRANSPORT_SOURCE_PINS));
    assert(TRANSPORT_SOURCE_PINS.every(Object.isFrozen));
  } finally {
    f.close();
  }
});

test("missing, oversized and linked critical sources fail with a fixed diagnostic", () => {
  const f = fixture();
  try {
    const target = f.file("snowflake-sdk/dist/lib/http/base.js");
    const original = readFileSync(target);
    rmSync(target);
    rejects(f.entry);
    writeFileSync(target, Buffer.alloc(4 * 1024 * 1024 + 1));
    rejects(f.entry);
    rmSync(target);
    const alternate = path.join(f.root, "private-looking-synthetic-name");
    writeFileSync(alternate, original);
    symlinkSync(alternate, target);
    rejects(f.entry);
  } finally {
    f.close();
  }
});

for (const [parent, dependency] of [
  ["snowflake-sdk", "axios"],
  ["axios", "follow-redirects"],
]) {
  test(`shadowed ${dependency} resolution fails even with identical reviewed bytes`, () => {
    const f = fixture();
    try {
      for (const [relative] of TRANSPORT_SOURCE_PINS) {
        if (!relative.startsWith(`${dependency}/`)) continue;
        const target = f.file(`${parent}/node_modules/${relative}`);
        mkdirSync(path.dirname(target), { recursive: true });
        copyFileSync(f.file(relative), target);
      }
      rejects(f.entry);
    } finally {
      f.close();
    }
  });
}

test("package version and entry changes fail before any dependency import", () => {
  const f = fixture();
  try {
    const filename = f.file("snowflake-sdk/package.json");
    const original = readFileSync(filename);
    const changed = JSON.parse(original);
    changed.version = "0.0.0";
    writeFileSync(filename, JSON.stringify(changed));
    rejects(f.entry);
    changed.version = "3.4.1";
    changed.main = "./replacement.js";
    writeFileSync(filename, JSON.stringify(changed));
    writeFileSync(f.file("snowflake-sdk/replacement.js"), "throw new Error('must not import');");
    rejects(f.entry);
  } finally {
    f.close();
  }
});

for (const [parent, dependency] of [
  ["snowflake-sdk/dist/lib/http", "axios"],
  ["axios/dist/node", "follow-redirects"],
]) {
  test(`shadowed ${dependency} in its actual importer directory is rejected`, () => {
    const f = fixture();
    try {
      for (const [relative] of TRANSPORT_SOURCE_PINS) {
        if (!relative.startsWith(`${dependency}/`)) continue;
        const target = f.file(`${parent}/node_modules/${relative}`);
        mkdirSync(path.dirname(target), { recursive: true });
        copyFileSync(f.file(relative), target);
      }
      rejects(f.entry);
    } finally {
      f.close();
    }
  });
}
