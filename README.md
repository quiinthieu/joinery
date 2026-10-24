# Joinery

A desktop database studio built with Tauri v2, Rust, React, CodeMirror, and a Canvas data grid.

The approved [development blueprint](https://chatgpt.com/space/page_c4b9e62285c481918eec36b706cca3a7) is also recorded in [docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md).

For local trials against your own PostgreSQL and future Snowflake connections,
follow the [production connection test plan](docs/PRODUCTION_CONNECTION_TEST_PLAN.md).
Keep connection secrets in the ignored local `.env`; `.env.example` provides
blank inventory fields. These fields are transferred manually into the app,
not loaded by the frontend or automatically imported into profiles.

## Development

Install Node 24, pnpm 11, the pinned Rust toolchain, and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm desktop
```

Run the frontend separately with `pnpm dev` for a UI preview and the synthetic million-row grid. Database access runs inside the desktop Rust process. TypeScript IPC contracts and platform icons are generated automatically before development and builds.

```sh
pnpm check
pnpm format:check
pnpm test:rust
pnpm desktop:build
pnpm desktop:bundle:mac
```

## Local database and integration tests

```sh
docker compose up -d --wait
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres --test ssh -- --ignored
pnpm exec playwright install chromium --only-shell
pnpm test:e2e
```

The disposable PostgreSQL fixture listens on `127.0.0.1:55432`; database/user `joinery`, password `joinery-local-only`. Select **Local**, disable TLS for this fixture, and use these credentials. `docker compose down` stops the fixture while retaining its volume; `docker compose down -v` also deletes its test data.

Browser tests use an explicitly enabled, authenticated loopback harness compiled with the `test-harness` feature. The desktop release does not include that HTTP server. Browser tests exercise the actual Rust service and PostgreSQL, with Arrow responses decoded by the same frontend worker used on desktop.

## Current implementation

- PostgreSQL profiles, verified TLS/custom CAs, system credential storage, hierarchical environment groups, and connection health.
- SSH bastions with password, encrypted OpenSSH key, or agent authentication; explicit host fingerprint trust; shared tunnels and loopback-only forwarding. Database TLS verifies the original database hostname through the tunnel.
- CodeMirror SQL tabs, current-statement/selection execution, complete-batch execution, schema-aware completion, worker-based formatting, cancellation, process inspection, and metadata history.
- Canvas grid, exact integer/decimal text, explicit NULLs, keyset table browsing, PK/FK structure, sparse edits, typed values, generated SQL/bindings review, and atomic conflict-checked commits across pages.
- Rust-enforced Safe Mode, production password gates, one-use expiring approvals, a hash-only audit viewer, and uncertain commit outcomes.
- Dark/light/high-contrast themes, keyboard commands, inspector, and bounded in-memory result storage.

Grid browsing and commits use login credentials independently of SQL-editor transactions and session settings. A grid commit never commits an editor transaction. Tables without a supported primary key have a bounded first page and cannot be edited.

Results are never written to disk: Rust retains at most 64 MiB in total and 100,000 rows per query result, the display cache retains at most 32 MiB / 512 pages, and pending edits have an 8 MiB budget. A table page stops at 500 rows or 2 MiB. IPC frames stop at 4 MiB and the frontend adapts page sizes for wide results. SQL history text is opt-in per connection; passwords stay in memory or the operating-system credential store.

Cancellation discards the affected SQL session after the native PostgreSQL cancel request to prevent a late request from cancelling a subsequent query. Reconnect before running another query. A lost connection during commit requires checking database state before retrying.

SSH host trust is local to Joinery. A changed key is blocked; the app does not silently replace a trusted key. Agent forwarding is disabled. A custom agent socket path is available when a GUI launch does not inherit `SSH_AUTH_SOCK`.

Safe Mode provides operational safeguards. Database roles remain the access-control boundary; use a read-only role for production browsing.

## Hidden native validation

```sh
pnpm desktop:smoke:build
pnpm test:native
```

This separate feature build opens a hidden system WebView, uses disposable application data, and checks native binary IPC, Arrow decoding, workers, editor/grid mounting, and close protection. Its executable is copied to `.cache/native-smoke/` before a normal build can overwrite it; the runner verifies the binary digest. It does not use the operating-system credential store or any database credentials. Detailed scope and measurements are in [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

Additional engines, distribution signing, and full performance qualification remain on the [development roadmap](docs/DEVELOPMENT_PLAN.md). The current implementation is the PostgreSQL milestone; RAM/startup/FPS targets are not release guarantees.
