# Joinery

A desktop database studio built with Tauri v2, Rust, React, CodeMirror, and a Canvas data grid.

The private [Joinery project hub](https://joinery-project-hub.otohime-nokimi.chatgpt.site)
is the canonical plan and shipping-status destination. Update the Site after
implementation, verification, merge, and release milestones. The detailed
engineering blueprint is also recorded in [docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md).

The [latest macOS preview](https://github.com/quiinthieu/joinery/releases)
includes the PostgreSQL studio and a read-only Snowflake SQL/catalog pilot.
Preview.4 adds the MySQL 8.4/MariaDB 11.4 studio. Current source also includes a
standalone Redis workspace; see the Site for
the exact published version and verification state.
Shipped features, remaining work, and connection-test results are tracked on the
Site. Keep connection secrets in the ignored local `.env`; `.env.example`
provides blank inventory fields. Transfer these manually into the app; the
frontend does not load or automatically import them.

In New connection, choose Snowflake and enter the account hostname, username,
database, uppercase role, warehouse, and schema. Joinery opens the system browser
using local-application OAuth; no password or token is entered in Joinery. The
account must enable `SNOWFLAKE$LOCAL_APPLICATION`. Use a database-enforced
read-only role and a bounded warehouse/query budget.

Snowflake supports read-only SQL, catalog discovery, lazy column completion,
structure inspection, and unordered table previews capped at 500 rows or 2 MiB.
Use SQL for larger or ordered results. Catalog discovery covers the configured
database and rejects inventories beyond 10,000 objects. Editing stays disabled.
The live canary covered browser SSO, context verification and `SELECT 1`;
catalog/table tests use synthetic fixtures and live table qualification is open.

## Development

Install Node 24, pnpm 11, the pinned Rust toolchain, and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm desktop
```

Run the frontend separately with `pnpm dev` for a UI preview and the synthetic million-row grid. Database access runs inside the desktop Rust process. TypeScript IPC contracts and platform icons are generated automatically before development and builds.

### Validation and local packaging

GitHub Actions validates pull requests and pushes to `main` using standard
Linux, Windows, and macOS runners. Jobs only run while the repository is public,
where standard runner minutes are free. The workflow has no artifact uploads,
caches, custom images, larger runners, or paid external services. It uses
disposable integration fixtures and no production database secrets. See
[GitHub's runner policy](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

Run the same baseline validation locally before committing or shipping:

```sh
pnpm validate
pnpm desktop:build
pnpm desktop:bundle:mac
```

`pnpm validate` checks frontend formatting, regenerates Rust/TypeScript contracts,
runs TypeScript and frontend tests, builds frontend assets, runs Rust unit tests,
and checks Rust formatting, compilation, and strict Clippy. It stops on the first
failure. The desktop commands compile/package on the current host; the macOS
bundle command requires macOS. Windows/Linux qualification requires running on
those platforms. Release packaging remains local; CI builds frontend assets and
checks native compilation without uploading installers. Existing GitHub release
downloads remain available.

Database/SSH/browser integration and hidden native checks are separate local
steps below. Rust integration tests marked ignored do not run in `pnpm validate`.

## Local database and integration tests

```sh
python3 scripts/mysql-tls-fixture.py
docker compose --profile mysql --profile redis up -d --wait
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres --test ssh --test mysql --test redis -- --ignored --test-threads=1
pnpm exec playwright install chromium --only-shell
pnpm test:e2e
```

The disposable PostgreSQL fixture listens on `127.0.0.1:55432`; database/user `joinery`, password `joinery-local-only`. Select **Local**, disable TLS for this fixture, and use these credentials. `docker compose down` stops the fixture while retaining its volume; `docker compose down -v` also deletes its test data.

The optional `mysql` Compose profile adds MySQL 8.4 on `127.0.0.1:53306` and
MariaDB 11.4 on `127.0.0.1:53307`, with the same disposable database/user/password.
The TLS preparation script creates an ignored two-day fixture CA/server key;
select **MySQL / MariaDB**, choose Local, and either disable TLS or select
`.cache/mysql-fixture-tls/ca.pem` as the custom CA. Never reuse these fixture
credentials or keys outside local tests. Stop just these services with
`docker compose --profile mysql stop mysql mariadb`.

Browser tests use an explicitly enabled, authenticated loopback harness compiled with the `test-harness` feature. The desktop release does not include that HTTP server. PostgreSQL, MySQL, MariaDB, and Redis browser tests exercise the actual Rust service and disposable databases. The Snowflake browser workflow intercepts that API boundary with synthetic responses and Arrow data; separate Rust tests exercise the Snowflake adapter against a loopback HTTP fixture. Neither Snowflake test path signs in or queries a production account.

The optional `redis` profile starts Redis 8.2 on `127.0.0.1:56379`, database `0`,
ACL username `default`, password `joinery-redis-local-only`. Select **Redis**,
Local, and disable TLS for this disposable fixture. Its data stays in memory.
Stop only this fixture with `docker compose --profile redis stop redis`.

## Redis workspace

Open a query tab on a Redis connection for the native command console and
expand **Key browser** for SCAN, type/TTL inspection and bounded string/hash/
list/set/sorted-set previews. MATCH patterns and opaque next-page cursors stay
scoped to the current session. Keys can expire, change or repeat during a live
scan; the browser does not represent a snapshot. Binary values have explicit
escaped-byte labels. String previews stop at 64 KiB; other previews stop at
500 entries / 2 MiB. Use bounded console ranges for larger values.

Commands use newline boundaries outside quotes; semicolons are argument bytes.
Double/single quotes and `\\xHH` escapes preserve byte arguments. Command completion
and current-line/selection execution use CodeMirror. Typed edit forms generate
commands for review; string edits use `SET ... KEEPTTL`. Every Redis write needs
one-use approval, with the normal production passphrase gate. Batches execute
commands sequentially without automatic rollback. Blocking, script, transaction,
administrative/session commands, KEYS and unbounded aggregate commands are
unavailable. Cluster, Sentinel, streams and Pub/Sub are not supported.

RESP2 replies are bounded before decoding: 2 MiB, 10,000 elements and 16 levels.
The adapter uses redis-rs serialization/parsing over owned async TCP/TLS streams
so cancellation can close the socket without leaving a background task. A write
whose acknowledgement is lost is outcome unknown; reconnect and check state
before retrying. Read-only ACL grants remain the authorization boundary. TLS
uses the original hostname, including through SSH; custom ACL usernames are
authenticated explicitly and never fall back to the default user.

## Current implementation

- PostgreSQL and MySQL/MariaDB profiles, verified TLS/custom CAs, system credential storage, hierarchical environment groups, and connection health.
- SSH bastions with password, encrypted OpenSSH key, or agent authentication; explicit host fingerprint trust; shared tunnels and loopback-only forwarding. Database TLS verifies the original database hostname through the tunnel.
- CodeMirror SQL tabs, current-statement/selection execution, complete-batch execution, schema-aware completion, worker-based formatting, cancellation, process inspection, and metadata history.
- Canvas grid, exact integer/decimal text, explicit NULLs, keyset table browsing, PK/FK structure, sparse edits, typed values, generated SQL/bindings review, and atomic conflict-checked commits across pages.
- Rust-enforced Safe Mode, production password gates, one-use expiring approvals, a hash-only audit viewer, and uncertain commit outcomes.
- Dark/light/high-contrast themes, keyboard commands, inspector, and bounded in-memory result storage.

Grid browsing and commits use login credentials independently of SQL-editor transactions and session settings. A grid commit never commits an editor transaction. Tables without a supported primary key have a bounded first page and cannot be edited.

Results are never written to disk: Rust retains at most 64 MiB in total and 100,000 rows per query result, the display cache retains at most 32 MiB / 512 pages, and pending edits have an 8 MiB budget. A table page stops at 500 rows or 2 MiB. IPC frames stop at 4 MiB and the frontend adapts page sizes for wide results. SQL history text is opt-in per connection; passwords stay in memory or the operating-system credential store.

Cancellation discards the affected SQL session after a native PostgreSQL cancel request or MySQL/MariaDB owned-query cancellation to prevent a late request from cancelling a subsequent query. Reconnect before running another query. A lost connection during commit requires checking database state before retrying.

MySQL/MariaDB editing requires an InnoDB base table and a supported primary key.
Views, nontransactional tables, generated/binary/floating-point columns, and
arbitrary SQL results remain read-only. The backend fixes SQL mode, character
set, and timezone before editor statements; attempts to change parser settings
or use executable comments are rejected. DDL can implicitly commit an editor
transaction. MySQL process inspection is available; external process cancellation
is disabled because process-list identity cannot safely authorize a later kill.
Use the SQL toolbar to cancel Joinery’s own run. A cancellation request may race
a successful write: the observed server outcome takes precedence.

SSH host trust is local to Joinery. A changed key is blocked; the app does not silently replace a trusted key. Agent forwarding is disabled. A custom agent socket path is available when a GUI launch does not inherit `SSH_AUTH_SOCK`.

Safe Mode provides operational safeguards. Database roles remain the access-control boundary; use a read-only role for production browsing.

## Hidden native validation

```sh
pnpm desktop:smoke:build
pnpm test:native
```

This separate feature build opens a hidden system WebView, uses disposable application data, and checks native binary IPC, Arrow decoding, workers, editor/grid mounting, and close protection. It also checks ten tab/result cleanup cycles with Arrow requests in flight and records shell/loading/release checkpoints with main-process memory samples. Its executable is copied to `.cache/native-smoke/` before a normal build can overwrite it; the runner verifies the binary digest. It does not use the operating-system credential store or any database credentials. Build provenance records the source revision, tree, and whether the working tree was dirty. The shell checkpoint is hidden DOM readiness; WebView helper memory, visible paint, and display FPS remain unqualified.

To explicitly qualify the native credential store with one disposable random entry:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test credential_store native_store_roundtrip_and_cleanup -- --ignored --test-threads=1
```

This writes, reads, deletes, and checks absence of that synthetic entry. It never reads saved connection credentials. It is ignored in ordinary validation/CI; it needs native credential-store access on the local host. Locked/denied-store behavior is separately covered through injected failures; real OS locked-store qualification is still pending. Detailed scope and measurements are in [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

Broader Redis, MySQL/MariaDB and Snowflake production qualification,
distribution signing, and full performance qualification remain on the
[development roadmap](docs/DEVELOPMENT_PLAN.md). The preview is available for
macOS Apple Silicon; RAM/startup/FPS targets remain unqualified.
