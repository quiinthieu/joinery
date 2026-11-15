# Local metadata WAL admission

The existing concurrent-startup regression exposed an intermittent local SQLite
startup failure during Snowflake session-observer qualification on 10 November 2026. The persistence implementation was unchanged when the failure appeared.
This repair is independent of Snowflake authentication or account SQL.

## Cause and boundary

Schema initialization already uses an IMMEDIATE transaction, validates existing
metadata, and rejects unsupported versions without resetting the database. WAL
promotion followed that transaction. Another opener can acquire a reserved writer
lock between schema initialization and WAL promotion; SQLite can then return
`SQLITE_BUSY` immediately from `PRAGMA journal_mode=WAL`, despite a configured
busy timeout. See SQLite's [busy handler](https://www.sqlite.org/c3ref/busy_handler.html),
[WAL behavior](https://www.sqlite.org/wal.html), and
[journal-mode pragma](https://www.sqlite.org/pragma.html#pragma_journal_mode).

The startup helper retries only a raw SQLite `DatabaseBusy` error from that
idempotent WAL pragma. It uses a five-second monotonic admission budget, at most
ten milliseconds between attempts, and sets each native busy timeout to the
remaining budget. It does not start a retry after the budget expires. Success
requires SQLite to return `wal`; another reported mode is an error. It restores
the normal five-second timeout before returning. Schema/version checks retain
their earlier ordering, and connection-local foreign-key and synchronous
settings follow successful WAL admission.

A permanent error or exhausted budget still fails startup with a sanitized
application-database error. The helper does not recreate the database, replay
migrations or retry user SQL. This does not promise that every filesystem or
cross-process startup failure can be recovered.

## Reproduction and verification

Evidence is kept under `.cache/verification/connection-attempts/`:

- `sdk-session-validate-final.log` preserves the initial combined-run failure in
  `concurrent_app_starts_create_one_complete_metadata_store`.
- `sdk-session-persistence-repeat.log` reproduces that unchanged failure on
  attempt 35. A temporary diagnostic build reproduces it on attempt 70 and
  records `DatabaseBusy`, extended code 5, at the durability-pragma step in
  `sdk-session-persistence-diagnostic.log`. The original source is restored
  byte for byte afterward. That diagnostic executable is not a shipped artifact.
- The concurrent-start test now joins every worker before asserting, preventing
  a first failure from dropping its temporary directory under unfinished peers.
  Secondary I/O errors following fixture teardown are not treated as the initial
  product failure.
- `persistence-wal-before-fix.log` disables the BUSY retry for a sensitivity
  check: the held-owner regression fails immediately with `DatabaseBusy`.
  Repaired source is restored byte for byte. `persistence-wal-focused-final.log`
  then passes all ten persistence tests, and `persistence-wal-repeat-final.log`
  passes 100 consecutive four-connection concurrent-start runs. The first new
  test compile's unsupported unsigned timeout read is corrected to SQLite's
  signed integer type; its failed log is retained separately.
- `persistence-wal-browser-restart-final.log` passes the actual Rust-backend
  abrupt-restart browser workflow on disposable PostgreSQL: opted-in drafts and
  preferences survive; sessions/results do not reconnect or replay SQL.

The added real SQLite regressions hold a reserved writer until WAL admission is
waiting, release it and verify WAL plus intact contents; exhaust a short budget
while retaining the owner and verify unchanged file bytes, mode and contents;
and reject non-BUSY errors and non-WAL results without committing a caller's
transaction. The reserved-owner success test uses a scheduler-assisted early
result wait; the held-lock expiry test independently checks the retry duration.
These tests use separate real connections and threads, not separate app
processes. Independent source review clears the bounded repair.

Run with the repository's documented toolchain:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib persistence::tests
pnpm validate
```

Combined application, packaging and native qualification is recorded in the
[verification ledger](replacement-verification.md). No private application state,
credentials, external database or global configuration is required by these
checks.

`sdk-session-validate-after-wal.log` passes the complete `pnpm validate` command:
627 frontend tests, 357 standard Rust tests, contracts/types, production web
build, repository formatting, desktop compilation and strict all-target,
all-feature Clippy. This is combined working-tree evidence with the session
observer increment; package identities remain separately recorded in the ledger.
