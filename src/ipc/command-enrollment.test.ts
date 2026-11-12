import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const read = (file: string) => readFileSync(path.resolve(file), "utf8");

function section(source: string, start: string, end: string) {
  const beginning = source.indexOf(start);
  if (beginning < 0) throw new Error(`Missing command-list start: ${start}`);
  const ending = source.indexOf(end, beginning + start.length);
  if (ending < 0) throw new Error(`Missing command-list end: ${end}`);
  return source.slice(beginning + start.length, ending);
}

function unique(names: string[]) {
  expect(names.length).toBeGreaterThan(0);
  expect(new Set(names).size, "command enrollment must not contain duplicates").toBe(names.length);
  return names.sort();
}

function handlerCommands() {
  const source = section(read("src-tauri/src/lib.rs"), "tauri::generate_handler![", "])");
  return unique(
    [...source.matchAll(/^\s*(?:commands::)?([a-z_]+),?\s*$/gm)].map((match) => match[1]),
  );
}

function permissions(file: string) {
  const capability = JSON.parse(read(file)) as { permissions: string[] };
  return unique(
    capability.permissions
      .filter((permission) => permission.startsWith("allow-"))
      .map((permission) => permission.slice("allow-".length).replaceAll("-", "_")),
  );
}

describe("native IPC enrollment", () => {
  it("generates an app ACL for every registered command, with no retired commands", () => {
    const source = section(read("src-tauri/build.rs"), "AppManifest::new().commands(&[", "])");
    const enrolled = unique([...source.matchAll(/"([a-z_]+)"/g)].map((match) => match[1]));
    expect(enrolled).toEqual(handlerCommands());
  });

  it("grants every production command to main while isolating test-only commands", () => {
    const handlers = handlerCommands();
    const smoke = handlers.filter((command) => command.startsWith("native_smoke_"));
    expect(permissions("src-tauri/capabilities/main.json")).toEqual(
      handlers.filter((command) => !smoke.includes(command)),
    );
    expect(permissions("src-tauri/capabilities/native-smoke.json")).toEqual(smoke);
    const configuration = JSON.parse(read("src-tauri/tauri.conf.json"));
    expect(configuration.app.security.capabilities).toEqual(["main"]);
  });
});
