// Local evidence only. Creating or reopening a journal never grants execution.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { canonicalJson, emitExclusive } from "./snowflake-qualification-manifest.mjs";

const fail = (safeCode) => Object.assign(new Error(safeCode), { safeCode });
export function setupRunKey(profile, runId) {
  return createHash("sha256")
    .update(
      canonicalJson({
        account: profile.account.toLowerCase(),
        host: profile.host.toLowerCase(),
        runId,
      }),
    )
    .digest("hex");
}

export function syncSetupDirectory(directory) {
  const before = fs.lstatSync(directory);
  if (
    !before.isDirectory() ||
    before.isSymbolicLink() ||
    (before.mode & 0o7777) !== 0o700 ||
    (process.getuid && before.uid !== process.getuid())
  )
    throw fail("unsafeOutputDirectory");
  const fd = fs.openSync(directory, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const opened = fs.fstatSync(fd);
    if (opened.dev !== before.dev || opened.ino !== before.ino) throw fail("outputIdentityChanged");
    fs.fsyncSync(fd);
    const after = fs.lstatSync(directory);
    if (after.dev !== opened.dev || after.ino !== opened.ino) throw fail("outputIdentityChanged");
  } finally {
    fs.closeSync(fd);
  }
}

// O_EXCL header and directory fsync precede any helper/authentication activity.
// Entries are hash chained and fsynced before the child engine admits more SQL.
// There is intentionally no reopen/resume API.
export function createSetupJournal({ root, filename, executionPlanSha256, runKey }) {
  const header = {
    format: "joinery-snowflake-setup-journal-v1",
    executionPlanSha256,
    runKey,
  };
  emitExclusive(root, filename, header);
  syncSetupDirectory(path.dirname(filename));
  const before = fs.lstatSync(filename);
  const fd = fs.openSync(
    filename,
    fs.constants.O_WRONLY | fs.constants.O_APPEND | fs.constants.O_NOFOLLOW,
  );
  const opened = fs.fstatSync(fd);
  if (opened.dev !== before.dev || opened.ino !== before.ino || (opened.mode & 0o7777) !== 0o600) {
    fs.closeSync(fd);
    throw fail("journalIdentityChanged");
  }
  let closed = false,
    failed = false,
    sequence = 0,
    bytes = Buffer.byteLength(canonicalJson(header)) + 1;
  let previous = createHash("sha256").update(canonicalJson(header)).digest("hex");
  return Object.freeze({
    append(event) {
      if (closed || failed) throw fail("journalUnavailable");
      try {
        const payload = { sequence: ++sequence, previous, event };
        const sha256 = createHash("sha256").update(canonicalJson(payload)).digest("hex");
        const line = `${canonicalJson({ ...payload, sha256 })}\n`;
        bytes += Buffer.byteLength(line);
        if (sequence > 512 || bytes > 2097152) throw fail("journalLimit");
        const current = fs.lstatSync(filename);
        if (current.dev !== opened.dev || current.ino !== opened.ino)
          throw fail("journalIdentityChanged");
        fs.writeFileSync(fd, line);
        fs.fsyncSync(fd);
        previous = sha256;
      } catch {
        failed = true;
        throw fail("journalPersistenceFailed");
      }
    },
    close() {
      if (!closed) {
        // Do not retry an uncertain close: the descriptor might already be
        // closed and subsequently reused. Preserve the failure in the receipt.
        closed = true;
        try {
          fs.closeSync(fd);
        } catch {
          failed = true;
          throw fail("journalCloseFailed");
        }
      }
    },
    snapshot: () => Object.freeze({ sequence, bytes, failed, closed }),
  });
}
