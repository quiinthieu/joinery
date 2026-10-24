import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";

const extension = process.platform === "win32" ? ".exe" : "";
const source = path.resolve(`src-tauri/target/release/joinery${extension}`);
const directory = path.resolve(".cache/native-smoke");
await mkdir(directory, { recursive: true });
const binary = path.join(directory, `joinery-smoke${extension}`);
await copyFile(source, binary);
const sha256 = createHash("sha256")
  .update(await readFile(binary))
  .digest("hex");
await writeFile(path.join(directory, "build.json"), JSON.stringify({ binary, sha256 }, null, 2));
process.stdout.write(`Hidden native test executable: ${binary}\n`);
