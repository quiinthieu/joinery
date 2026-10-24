# Joinery

A desktop database studio built with Tauri v2, Rust, React, CodeMirror, and a Canvas data grid.

The approved [development blueprint](https://chatgpt.com/space/page_c4b9e62285c481918eec36b706cca3a7) is also recorded in [docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md).

## Development

Install Node 24, pnpm 11, the pinned Rust toolchain, and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm desktop
```

Run the frontend separately with `pnpm dev` for a UI preview and the synthetic million-row grid. Database access runs inside the desktop Rust process. TypeScript IPC contracts and platform icons are generated automatically before development and builds.

```sh
pnpm check
pnpm test:rust
pnpm desktop:build
```

## Local database and integration tests

```sh
docker compose up -d --wait
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres -- --ignored
pnpm exec playwright install chromium --only-shell
pnpm test:e2e
```

The disposable PostgreSQL fixture listens on `127.0.0.1:55432`; database/user `joinery`, password `joinery-local-only`. Select **Local**, disable TLS for this fixture, and use these credentials. `docker compose down` stops the fixture while retaining its volume; `docker compose down -v` also deletes its test data.

Browser tests use an explicitly enabled, authenticated loopback harness compiled with the `test-harness` feature. The desktop release does not include that HTTP server. Browser tests exercise the actual Rust service and PostgreSQL, with Arrow responses decoded by the same frontend worker used on desktop.

## Current implementation

- PostgreSQL profiles, verified TLS/custom CAs, system credential storage, environment groups, and connection health.
- CodeMirror SQL tabs, current-statement/selection execution, complete-batch execution, schema-aware completion, worker-based formatting, cancellation, process inspection, and metadata history.
- Canvas grid, exact integer/decimal text, explicit NULLs, keyset table browsing, PK/FK structure, sparse edits, typed values, generated SQL/bindings review, and atomic conflict-checked commits across pages.
- Rust-enforced Safe Mode, production password gates, one-use expiring approvals, hash-only audit records, and uncertain commit outcomes.
- Dark/light/high-contrast themes, keyboard commands, inspector, and bounded in-memory result storage.

Grid browsing and commits use login credentials independently of SQL-editor transactions and session settings. A grid commit never commits an editor transaction. Tables without a supported primary key have a bounded first page and cannot be edited.

Results are never written to disk: Rust retains at most 64 MiB in total and 100,000 rows per query result, the display cache retains at most 32 MiB, and pending edits have an 8 MiB budget. A table page stops at 500 rows or 2 MiB. SQL history text is opt-in per connection; passwords stay in memory or the operating-system credential store.

Cancellation discards the affected SQL session after the native PostgreSQL cancel request to prevent a late request from cancelling a subsequent query. Reconnect before running another query. A lost connection during commit requires checking database state before retrying.

SSH transport, additional engines, release signing, and native performance measurements are tracked in the [development blueprint](docs/DEVELOPMENT_PLAN.md). RAM/startup/FPS figures in that blueprint are targets, not measured claims.
