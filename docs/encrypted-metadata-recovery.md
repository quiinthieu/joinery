# Encrypted local metadata recovery

## Status and scope

Local source passes full validation, real database regressions, the browser
recovery workflow and hidden macOS encrypted recovery/restart acceptance on
`feat/datagrip-replacement`. The normal SDK-bearing ARM64 package from clean
`f9c1ce2` passes resource, strict ad-hoc signature and guarded helper-startup
checks. It is local and uninstalled at
`src-tauri/target/release/bundle/macos/Joinery.app`.
This document describes that source and local package. Installed-app behavior,
native picker interaction, OS keychain, external accounts and owner acceptance
remain unqualified.
See the [verification ledger](replacement-verification.md) for actual results and
the [retention design](snowflake-console-retention-design.md) for the full target.
The DataGrip replacement Goal and all remaining required workflows stay open.

Backups include local profiles, groups, settings, saved scripts, permitted query
history, SSH trust and complete durable console receipts. Database credentials,
the operating-system-stored write-passphrase verifier and result rows are not
portable backup secrets. Live metadata and temporary recovery staging remain
plaintext, protected by local filesystem permissions. FileVault and filesystem
snapshots are separate concerns; deleting a file does not promise secure erasure.

## Acceptance walkthrough

Use a disposable application data directory with synthetic profiles and scripts.
Do not replace the owner's current application metadata as an automated test.

1. Disconnect all connections, close all workspace tabs and wait for connection
   setup, execution, exports and durable receipt recording to settle. Native
   admission also checks detached activity and takes the raw exclusive journal
   lease. It does not authenticate or send database SQL.
2. Open Preferences → Backup and recovery. Enter and confirm a separate backup
   passphrase: 12–1024 Unicode scalar values, at most 4096 UTF-8 bytes. Keep it in
   a password manager; Joinery does not save it. It is distinct from the current
   write passphrase. Create a backup and wait for the actual operation status.
3. Export the encrypted backup through the native save dialog to a new file.
   Destination replacement is refused. Keep a separate copy outside this disk;
   an application-local undo copy alone cannot recover a lost disk.
4. Add a clearly named synthetic connection group. Import the exported file
   through the native open dialog. Import copies ciphertext to a fresh opaque
   local ID; authentication and schema validation occur during restore.
5. First try restoring with a deliberately different backup passphrase. Verify
   the synthetic group, original receipts and backup inventory are unchanged.
   An unauthenticated source must not create an undo backup or replace metadata.
6. Select the imported backup, enter its passphrase twice and type `RESTORE`.
   If configured, enter the separate current write passphrase. Wait for the
   actual outcome. The synthetic group should disappear. A mandatory encrypted
   undo backup must appear, encrypted with the supplied backup passphrase.
7. Restore that undo backup and verify the group returns. Existing original
   console evidence must be preserved as an exact union, with conflicting
   identities rejected rather than overwritten. Restore refreshes the UI and
   discards stale queued workspace saves; reconnect only when ready.
8. Restart the disposable application and verify profiles, scripts, metadata and
   receipts. No query, authentication or account write should happen on restart.
   Delete only the test-owned copies by selecting each and typing `DELETE`.

Operations retain a process-local status keyed by a browser-generated UUID. A
lost response is resolved by looking up that same UUID, never by resubmitting.
The browser persists only that UUID, not a key or request body. If status is lost
across restart, the original outcome is unknown. Explicit reconciliation fences
future admission for that ID and reloads metadata before workspace saving resumes.
It does not establish that an earlier operation was cancelled or never ran.
If the original UUID itself is unavailable or malformed, reconciliation closes
all further recovery admission in the native process before reloading metadata
and resuming workspace saving. Fully restart Joinery before starting another
backup, import, export or restore. Inventory and deletion remain available; the
UI reads the native admission state again after remounting.

After a restart with retained or unreadable recovery state, Joinery opens a
mandatory recovery screen before enabling editors, navigation or workspace
shortcuts. It waits for the initial metadata read before reconciliation. Resolve
the retained operation there; new drafts cannot be entered and then discarded
by the subsequent metadata reload. If the original marker is absent after an
earlier storage failure, use the same unknown-ID global reconciliation path.
Workspace close/quit refuses a paused save rather than reporting it saved.

Cancellation is cooperative during preparation and transfer. Once atomic live
replacement or final file publication begins, wait for the recorded result.
Cancellation before replacement may still leave a valid encrypted undo copy;
inspect the retained backup identity and inventory. Use the recovery form to
cancel, then wait for its actual terminal status. Close/quit stays open while
workspace saving is paused. Native shutdown also waits for owned work and keeps
the app open if it cannot settle within its bounded wait.

## Format, limits and compatibility

New backups use a versioned closed manifest inside a passphrase-encrypted `age`
stream. The manifest declares schema, plaintext byte length and SHA-256. The
reader enforces bounded headers, a single supported scrypt recipient, bounded
work factor, exact payload length, authenticated final chunk and physical EOF.
Unknown formats, extra bytes, truncated files and schema violations are rejected.
Encryption and hashing use fixed buffers; SQLite copies step through bounded
pages. The real production scrypt work factor is 18, serialized by the recovery
activity gate. Ordinary unit fixtures use a compile-time-only cheaper factor;
they cannot establish production cost or memory usage.
One admitted key-derivation call must finish before it can observe cancellation;
the bounded work factor is not a hard wall-clock deadline on every machine.

There are at most 20 local backups, 64 GiB aggregate backup storage and 16 GiB
plaintext per snapshot. Restore requires additional space for decrypted staging,
the evidence union, live replacement and the mandatory encrypted undo. Row,
statement and known JSON-field limits apply independently of total file size.
SQLite may use its private operating-system temporary directory for sorting.
Backup inventory hashes up to 20 files; large inventories may take time.

Legacy schema 1–4 plaintext SQLite backups remain readable through the new
restore path, with a newly chosen passphrase for the encrypted undo. New backup
creation and export produce encrypted files. Older Joinery versions cannot read
the new format. Restore preserves the current write-passphrase authority rather
than adopting an old flag from the backup. Unsafe files, foreign leases,
unfamiliar staging files and conflicting evidence must be rejected or preserved
for inspection, never silently cleaned or adopted.

This does not remove the existing 10,000-positive / 10,000-refusal receipt gates.
Journal byte reservations, complete capacity admission and memory-plateau evidence
remain separate requirements. No unlimited retention or full DataGrip parity is
claimed by this increment.

## Reproducible local checks

Use the repository's pinned Node 24 / Rust toolchains, one Cargo build at a time,
with at least 20 GiB free before building. Run `pnpm validate` first. Start only
disposable local fixtures as described in README; use a unique Compose project
when another project already owns data or ports.

```sh
PLAYWRIGHT_BROWSERS_PATH="$PWD/.cache/playwright-browsers" pnpm exec playwright test tests/e2e/recovery.spec.ts --workers=1
pnpm desktop:snowflake:smoke:build
pnpm test:native:encrypted-recovery:runner
pnpm test:native:encrypted-recovery
```

The browser test uses actual Rust recovery with production encryption strength,
synthetic local metadata and sandbox-selected files. It drops one admitted start
response, verifies exact original-ID lookup without another start, checks exported
ciphertext bytes/hash and imported source preservation, rejects a wrong key, then
restores and undoes through the form. A page reload alone is not native restart.

The separate hidden macOS WebView build uses a fresh private root and in-memory
secrets. It probes all eight recovery commands, six receipt commands and CSV
preview disposal through the real main-window ACL. Preferences create, wrong-key,
restore and undo use production encryption. A real process close/reopen checks
retained group and ciphertext identities, fresh process admission and absent
sessions/results. Its SQL-free evidence uses selected native command counters,
empty resources and source review; it is not universal network instrumentation.
The setup watchdog is 900 seconds and reopen watchdog 120; these are bounded
fixture deadlines, not performance targets. The runner kills only its own child
on timeout and rejects success reports without a clean process exit.

These commands are acceptance steps, not a claim every command has already
passed. Check the ledger for the actual source-pinned runs. Native picker dialog
interaction, the normal installed app, OS keychain and owner data remain separate.

## Dependency review

The source pins `age = 0.11.5` without optional plugins. Its registry archive
SHA-256 is `047a482d1843edf1ce76ada63183698144030fe1191bd5ddba6e41e164e0bc43`,
with upstream VCS revision `7f29934dc58e058e7a588ac3a9a7e946198a73a7`.
The added-dependency OSV check and exact metadata are retained under
`.cache/verification/reviewed-console/encrypted-recovery-age11-*`.
The original selected license texts, copyright notices and embedded data notices
are in `src-tauri/resources/recovery-dependency-notices.txt`. This inventory covers
the 48 added package versions, not every existing application dependency.
The file-specific Git attributes preserve original license bytes and spacing.

The candidate 0.12.1 introduced `crypto-bigint 0.5.5`, affected by
[RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html).
The selected dependency tree avoids adding that version. Two informational
unmaintained advisories remain in the localization proc-macro build path:
[basic-toml](https://rustsec.org/advisories/RUSTSEC-2026-0374.html) and
[proc-macro-error2](https://rustsec.org/advisories/RUSTSEC-2026-0173.html).
Neither has a patched version listed; this is not an advisory-free dependency
claim. Upstream pre-1.0 stability warnings and these maintenance limitations
remain relevant. Source review and tests do not constitute an independent
cryptographic security certification.
