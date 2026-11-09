# Replacement verification ledger

Run started 9 November 2026 on macOS arm64, source `bcb606f`, task branch
`feat/datagrip-replacement` from fetched `origin/main`. Local work is unmerged,
unreleased and not installed. No external database qualification is claimed.
See [acceptance](replacement-acceptance.md) for required gates and migration.

## Environment and commands

Initial host: Node 26.7.0, pnpm 11.23.0, shell Rust 1.91.1 (the repository
pins 1.99.1). The pinned Rust toolchain was subsequently installed without
changing the default toolchain. README requires Node 24. A checksum-
verified official Node 24.22.2 tarball was extracted into ignored
`.cache/toolchains/node-v24.22.2-darwin-arm64`; no global settings changed.
Use its `bin` at the front of PATH when reproducing commands below. Toolchain
downloads and full transient logs are local and are not repository artifacts.

Fixtures started from the repository Compose file after
`python3 scripts/mysql-tls-fixture.py`:

```sh
docker compose --profile mysql --profile mysql-folded --profile redis up -d --wait
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test postgres --test ssh --test mysql --test redis -- --ignored --test-threads=1
pnpm validate
pnpm test:e2e
pnpm desktop:bundle:mac
```

All six fixture containers were healthy: PostgreSQL 18, MySQL 8.4 and MariaDB
11.4 (normal and folded identifier modes), Redis 8.2. These are synthetic local
fixtures, not the owner's database state. No pre-existing running containers
were stopped or changed. Fixture volume data was retained.

## Outcomes

| Check | Outcome and limits |
|---|---|
| Initial `pnpm validate`, host Node 26 | Failed in frontend tests: 14 suites failed loading `studio.ts` because `localStorage` was undefined; 417 tests passed. Later stages did not run |
| `NODE_OPTIONS=--no-experimental-webstorage pnpm test`, Node 26 | 513 tests / 50 files passed; diagnostic workaround only, not a change to global settings or supported-toolchain qualification |
| Real fixture suite, initial combined source | 86 passed: MySQL/MariaDB 39, PostgreSQL 39, Redis 6, SSH 2. Other ignored suites, native credentials and external Snowflake are not covered by this command |
| Full `pnpm validate`, local Node 24 / shell Rust 1.91 | Frontend 513 passed, frontend build and 176 standard Rust tests passed, fmt/check passed; strict Clippy failed on three pre-existing nonminimal boolean predicates in `ddl.rs`/`safety.rs`. Separately repaired in `5f8e718`; 155 library tests and strict Clippy then passed. Superseded by the pinned-toolchain qualification below |
| First full browser suite | 108 passed / 9 failed (14.2 minutes), contaminated by concurrent source generation and changed assertions against the old harness. Traces show Vite page reloads. Not clean qualification; repeat after builds. Harness talks to real local databases; Snowflake browser tests are synthetic |
| Normal macOS app bundle/signature check, shell Rust 1.91 | Passed: optimized binary and 22.92 MiB `src-tauri/target/release/bundle/macos/Joinery.app`, verified ad-hoc signature. Includes temporary-target fix; predates stale-snapshot follow-up. No installation or Developer ID/notarization claim |
| Actual process restart and restored UI | Passed targeted real backend SIGKILL/reopen with the same isolated metadata root. Drafts/preferences restored; sessions/results/transaction UI absent and no connect/SQL replay. Packaged native restart remains open |

Initial transient logs: `/tmp/joinery-datagrip-validate.log`,
`/tmp/joinery-datagrip-node26-test.log`, `/tmp/joinery-datagrip-fixtures.log`,
`/tmp/joinery-datagrip-validate-node24.log`, `/tmp/joinery-datagrip-e2e.log`,
`/tmp/joinery-datagrip-bundle.log`. Summary outcomes here are the durable evidence;
do not assume `/tmp` logs survive a reboot.

## Findings driving implementation

Temporary-table shadowing was reproduced on PostgreSQL and MySQL: results from
an editor-local temporary table could be marked editable against a permanent
table with the same name. The fix resolves the relation on the serialized editor
connection, rejects temporary/unverified targets, and preserves the actual
qualified permanent target for the separate grid writer. PostgreSQL resolves
the full effective search path, including a quoted table in a later schema.

The final focused command passed three tests on PostgreSQL and MySQL/MariaDB:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test postgres --test mysql query_result_ -- --ignored --test-threads=1
```

These tests first failed on the original source. They verify unqualified and
explicitly qualified temporary shadows stay read-only and cannot prepare edits;
permanent tables remain editable; PostgreSQL search-path resolution leaves the
session AutoCommit/Idle. Independent read-only review found no material issue
in the six-file patch. Existing permanent-table drop/recreate identity races
remain a separate limitation; this fix does not persist physical object IDs.
Combined-source qualification and subsequent repairs are recorded below.

The subsequent snapshot repair reproduced both an over-budget/over-row-limit
insert and a real PostgreSQL result inventing a NULL/default row after a
successful generated-key insert. `ResultStore::apply_committed_changes` now
preserves rows/status/bytes, revokes provenance/editability, and emits a read-only
reason until the query is explicitly rerun. Fabricated inserted-row payloads
were removed. Table-browser refresh keeps its separate existing path.

Focused checks passed: five `results::tests` unit tests and two
`query_result_commit_preserves_snapshot` integration tests (the MySQL test loops
over MySQL and MariaDB). They cover PostgreSQL defaults/generated IDs,
MySQL/MariaDB defaults and a BEFORE UPDATE uppercase trigger, stale-edit
rejection, unchanged snapshots/budgets, and actual values after rerun.
Independent read-only review found no material issue within this contract.
Other query snapshots for the same table are not automatically invalidated;
their writes remain conflict-checked. Cross-view/CSV-import invalidation remains
a usability enhancement; this change does not claim all views refresh.

Pinned-toolchain targeted browser checks passed two workflows: abrupt isolated
backend SIGKILL/reopen preserving drafts/preferences without sessions, results,
transactions or SQL replay; and committed query edits showing an explicit stale
snapshot, then fresh actual values and restored editability after rerun. Eight
other studio tests passed in the preceding targeted run. A test-coordinate
failure after the toolbar changed was repaired by recalculating the grid box.
This is real Rust/PostgreSQL/Arrow behavior, not synthetic RPC responses. Native
window shutdown flush, packaged restart and OS credential restoration remain
separate gates.

The qualification pipeline uses explicit local Node 24.22.2 and Rust 1.99.1 compiler/
documentation binaries, in this order: `pnpm validate`, full disposable fixture
command above, `pnpm desktop:bundle:mac`, `pnpm desktop:smoke:build`,
`pnpm test:native`, then `pnpm test:e2e`. No source generators run alongside the
browser suite. Final logs use `/tmp/joinery-replacement-final-*.log`.

## First pinned-toolchain qualification

Application source is committed locally through `a2713bd`; restart tooling is
`c882fc2`, target-identity fix is `5a1fed7`, Clippy repair is `5f8e718`, initial
acceptance/baseline is `b07b738`. No push, merge, publication or installation.

| Check | Current outcome |
|---|---|
| `pnpm validate` | Passed: 513 frontend tests, 178 standard Rust tests, frontend build, formatting, native check and strict Clippy |
| Full disposable fixture command | Passed: 91 tests (41 MySQL/MariaDB, 42 PostgreSQL, 6 Redis, 2 SSH), including cancellation, transactions, TLS/SSH and the new regressions within those test scopes |
| Normal macOS bundle | Passed: 27.33 MiB application bundle and strict ad-hoc signature verification; not installed, Developer ID signing/notarization remain open |
| Hidden native build/runtime | Passed: three isolated native WebView processes, ten cleanup cycles each, all exited successfully. Synthetic native integration, not real native database/credential/restart qualification |
| Full quiescent browser suite | 116 passed / 2 failed (6.2 minutes). PostgreSQL FK completion and initial result inspector selection failures reproduced in a separate three-repeat run (five failed / one passed). Repairs and requalification required |

The Rust 1.99.1 release builds warned that bundled `rust-objcopy` could not load
`libLLVM.dylib` while stripping debug information. Compilation, bundle signature
verification and native runtime still succeeded. No global toolchain changes or
warning suppression were applied.

### Follow-up findings

The FK browser test could observe the previous query's completed footer and
advance before its new execution/context refresh finished. Its assertions now
wait for the new run's actual Arrow page and idle editor/context before starting
the next completion. This is a test synchronization repair; it does not establish
a product completion defect.

The inspector failure exposed a product race: selecting a cell while its Arrow
page was loading notified the inspector with no value; later page arrival only
repainted the canvas. A delayed real-result browser regression now holds the actual Rust/Arrow
response, selects a cold cell, and verifies its inspector updates on arrival
without another click. A focused two-repeat run passed after the repair. No synthetic result data is used
for that browser check.

A focused transaction review found manual toolbar COMMIT transport errors
bypassed the write-outcome uncertainty mapping, and terminal toolbar errors
could leave a session/transaction displayed as live. A second failure path
could obscure an acknowledged COMMIT if the subsequent context refresh failed.
The repair distinguishes an unknown acknowledgement from a known commit,
retires unusable sessions, and preserves ordinary recoverable errors. Six real
`transaction_transport` fixture tests passed: PostgreSQL/MySQL/MariaDB lost
COMMIT acknowledgement; PostgreSQL acknowledged COMMIT followed by failed
context refresh; injected PostgreSQL FATAL/class-08 response after actual
COMMIT; and deferred constraint rejection with restored context and obsolete
plan invalidation. Protocol-aware loopback proxies forward real server traffic
and withhold acknowledgements; independent connections verify persisted data
and the proxy counts one COMMIT. The FATAL frame is explicit fault injection,
not a claim that a natural server FATAL was observed.

PostgreSQL/MySQL transport and PostgreSQL refresh/FATAL/deferred-plan defects
were reproduced before fixing. MariaDB passed after the repair but has no
observed failing baseline because the original combined test stopped earlier.
The shared transaction-control unit regression covers 24 engine/action/error
cases; it checks the Studio contract, not real database behavior. Frontend
regressions cover terminal retirement, recoverable-error context refresh,
identity revision and late responses without retrying COMMIT.

The real toolbar browser test then exposed another closed-session leak before
transaction submission: initial lookup returned early and left the map entry
and pending plans. The new preclosed-driver regression failed before fixing
both COMMIT and ROLLBACK lookup paths. The focused unit and six real fault tests
passed again; independent review found no material issue. The quiescent toolbar browser regression then passed twice (16.8 seconds),
including independent rollback observations, exact resource cleanup and usable
post-constraint context. A preceding two-repeat attempt was contaminated by
this run's concurrent documentation edits triggering Vite reloads (one pass,
one timeout); it is excluded from qualification. All repository edits, including
documentation, must pause during browser execution.

## Repaired-source qualification

Application repairs are local commits `66573be` (inspector), `19727e7`
(transaction outcomes), and `f9abce0` (FK browser synchronization). All substantial
increments received independent review with no unresolved material finding.

| Check | Outcome |
|---|---|
| `pnpm validate`, explicit Node 24.22.2 / Rust 1.99.1 | Passed: 542 frontend tests / 51 files, 179 standard Rust tests, production build, formatting, check and strict Clippy |
| Expanded disposable fixture suite | Passed: 97 tests (41 MySQL/MariaDB, 42 PostgreSQL, 6 Redis, 2 SSH, 6 transaction transport faults) |
| Focused browser repeats | FK completion, delayed inspector and committed-edit workflows each passed twice; final toolbar regression passed twice after the closed-session lookup fix |
| Normal macOS arm64 bundle | Passed: 27.33 MiB reported by Tauri, strict ad-hoc signature verification; bundle not installed/notarized |
| Hidden native build/runtime | Passed: three isolated WebView processes, ten cleanup cycles each, zero exit status; synthetic native integration, not native credential or packaged restart qualification |
| Full quiescent browser suite | Passed: 120/120 workflows (5.7 minutes); real Rust/Arrow and local database fixtures, with Snowflake responses explicitly synthetic |

The expanded fixture command is now also in README:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test postgres --test ssh --test mysql --test redis \
  --test transaction_transport -- --ignored --test-threads=1
```

Runtime qualification built from clean local revision `e481e62`, application
repair through `19727e7`. No repository edits or source generators ran alongside
the final browser suite. The normal app remains at
`src-tauri/target/release/bundle/macos/Joinery.app`; its packaged executable
SHA-256 is `12bc3c6c89d5206e571c3216d6e06662ab576c75c6cbc47e20bedf675158168f`.
Bundle identity is `com.quiinthieu.joinery`, version `0.1.0`; these labels alone
do not identify a published preview. The native-test-only follow-up in the next
section kept this normal bundle unchanged; the later reviewed-writer qualification
below records a newly built normal bundle.
The separate hidden native build does not replace the normal `.app` bundle.
Final documentation review confirmed the current queue, licensing distinction
and qualification claims; both wording findings were resolved.

Native elapsed times were 3719/2961/2984 ms; main-process peak RSS was
110.73/108.12/108.67 MiB. These are smoke observations, not whole-process-tree
memory or representative workload performance qualification. The existing
`rust-objcopy`/`libLLVM.dylib` stripping warning recurred; builds, signature and
native runtime passed. Vite also reports existing large output chunks.

Transient repaired-source logs use `/tmp/joinery-replacement-repaired-*.log`.
The final quiescent toolbar log is
`/tmp/joinery-replacement-toolbar-quiescent.log`.

Graph-assisted review used Tier 2 discovery/traces/snippets and coverage checks.
The early index generation was stale, so source was read directly; later coverage
checks reported current metadata and no recorded gaps for changed code/test
paths. Generated IPC types are intentionally excluded. Fixture SQL parse gaps
are not evidence of complete SQL semantic coverage. Native credentials,
signing/notarization, external accounts/engines/platforms and hands-on acceptance
remain outside the passing local checks.

The first full browser run was contaminated by concurrent generation/builds:
traces show document reloads resetting workspaces during explorer and MariaDB
tests. Do not count those timeouts as proved engine defects or a passing suite.
The first pinned run above was quiescent and exposed two reproducible issues;
the passing post-repair run also kept source generation, builds and repository
edits separate from browser execution.

Open requirements and hands-on acceptance remain pending owner input. Signing,
notarization, Windows/Linux packaged runtime, external authentication and live
Snowflake qualification remain open. No passing local test closes those gates.

## Native close and reopen qualification

Local commit `a1ab58b` adds a recovery mode to the separately built hidden native
test executable. It exercises App's actual window-close confirmation, including
**Keep working** with a live PostgreSQL manual transaction, then **Continue**
with a newly changed draft/preferences still pending in the normal workspace
writer. It does not call the writer's flush directly or use the smoke command's
forced exit for successful setup. A second fresh process opens the same owned
workspace with the same in-memory secret-store implementation.

Reproduce with the local Node 24.22.2 / Rust 1.99.1 environment above and the
repository's healthy PostgreSQL 18 fixture on loopback port 55432:

```sh
pnpm validate
pnpm test:native:recovery:runner
pnpm desktop:smoke:build
pnpm test:native:recovery --runs 2
pnpm test:native --runs 1 --cycles 10
```

| Check | Outcome |
|---|---|
| Full validation | Passed: 542 frontend tests / 51 files, 180 standard Rust tests, production build, formatting, compilation and strict Clippy |
| Recovery runner acceptance regressions | Six passed, including missing checks, nonzero backend invocations, process/watchdog failures and the strict pending-save deadline |
| Native options | Both configuration unit tests passed; optional recovery mode preserves ordinary smoke defaults and validates fixture ports |
| Actual close/reopen | Two pairs / four fresh native processes passed. Cancellation retained the live session, transaction and pending row. Confirmed close preserved exact Unicode draft text and preferences |
| Pending-save evidence | Runner observed setup process exit 48.18 ms and 39.20 ms after the draft mutation, both strictly before the writer's 400 ms debounce. Baseline persistence was read back before mutation; the latest bytes were read back after reopening |
| Restart safety | Zero `connect`, `prepare_execution` and `execute_plan` invocations before the first explicit observer connection in both reopened processes. Sessions, results, active runs and transaction UI were absent. Independent fresh PostgreSQL connections found zero rows, proving the uncommitted write rolled back; both owned tables were dropped |
| Ordinary smoke compatibility | One native process / ten cleanup cycles passed, zero exit status, 3128 ms native elapsed time |

The hidden test executable SHA-256 is
`6361c7d1c4ae0ed46a2197607cae102daab9ce3dc3f77ee04e886602ed685d9f`.
Build provenance records base revision `6eda0d5` with `sourceDirty=true`; the
tested source changes were then committed as `a1ab58b`. No source edits occurred
during native runtime. Raw local evidence is
`.cache/native-smoke/recovery-runs.json`, with stage reports and captured logs in
its owned `recovery-*` directories. Transient command logs use
`/tmp/joinery-native-recovery-*.log`. The existing `rust-objcopy`/`libLLVM.dylib`
warning recurred; build and all native processes succeeded.

Independent review found a false-positive risk in inferring no replay from empty
sessions and a zero row count alone: replay followed by rollback/disconnect could
leave the same state. Native-only process-lifetime command counters now close
that gap, and the runner requires their raw counts to be exactly zero. Review
confirmed the repair and found no remaining material issue in the bounded scope.

This is actual hidden native WebView close/reopen with real synthetic PostgreSQL,
not normal installed-bundle or OS credential qualification. For this native-only
increment the normal bundle was not rebuilt. The prior 97-fixture and 120-browser results
remain the application qualification baseline; those full suites were not rerun
for this native-test-only increment. Other engines, external authentication,
visible native accessibility/performance and owner acceptance remain open.

## Reviewed grid and CSV writer safety

Further review exercised the separate PostgreSQL grid/CSV writer, rather than
assuming the console COMMIT tests covered it. Three new real fault tests first
failed because `SessionClosed` was absent and the session remained registered.
Their unknown-outcome, persisted-row and single-COMMIT assertions already passed;
the initial hypothesis that a terminal response would be reported as a definite
database rejection was not reproduced. The shared terminal-error classifier is
now used consistently, and a raw protocol test verifies the injected response
really is a PostgreSQL database error with SQLSTATE `08006`.

The retirement defect was synchronous: aborting PostgreSQL connection tasks did
not immediately change `Client::is_closed()`. An explicit atomic closed flag now
makes retirement visible before execution emits its final events. A second real
regression queued two reviewed writes behind an observer-held row lock. On the
original path, the second write opened a fresh auxiliary connection after the
first writer's uncertain COMMIT retired the session: the observer found two rows
with a sum of 21 instead of one row with value 1. The changes lane now checks the
session and driver closure after acquiring serialization, before submission.
The passing regression proves one COMMIT, no new auxiliary connection, a cancelled
second job, consumed approval and the expected independent observer values.

CSV component regressions also reproduced a false claim that no rows committed
after an unknown COMMIT, and stale primary/pooled editor state after an explicit
closure event. Imports now say the batch may have committed and require table
verification before retry. Only explicit `SessionClosed` retires the editor;
writer-only uncertainty can leave a healthy MySQL editor intact. Unknown outcomes
halt without advancing to another batch or replaying the approved write. Earlier
completed batch counts remain separate. The three new component cases cover
first-batch primary, pooled and writer-only outcomes; later-batch count handling
was source-reviewed, without a new later-batch component regression.

Independent peer reviews found no material findings in the backend, frontend,
fixture relay or browser coverage. Tier 2 graph coverage was checked for every
changed code/test path. The new CSV test has a recorded parse gap at its generic
`vi.mock` call; that range and the complete file were read directly.

Focused checks pass: 12 real transaction-transport tests, three CSV component
cases and four real browser workflows (grid/CSV crossed with acknowledgement
loss/terminal response). Browser workflows review and submit actual changes,
assert unknown outcome and exact session retirement, disable retry/run, then
connect directly to observe the committed value and one COMMIT without replay.
The first four browser attempts failed only because their final observer helper
checked the status bar while the old disconnected console remained active.
Opening the observer's own console before checking its connection fixed the
test setup; all four then passed in 19.8 seconds. This was not a product repair.

Full validation passes 545 frontend tests / 52 files and 180 standard Rust tests,
including strict Clippy. The expanded real fixture command passes 103 tests:
41 MySQL/MariaDB, 42 PostgreSQL, 6 Redis, 2 SSH and 12 transaction-transport tests.
The backend repair is local commit `71d95b6`; CSV reporting, component coverage
and the browser harness/workflows are `cd5bee0`. No push, merge, publication or
installation occurred.

| Combined-source check | Outcome and scope |
|---|---|
| `pnpm validate` | Passed: 545 frontend tests / 52 files, 180 standard Rust tests, production build, formatting, compilation and strict Clippy |
| Full disposable fixture command above | Passed: 103 tests (41 MySQL/MariaDB, 42 PostgreSQL, 6 Redis, 2 SSH, 12 transaction transport) |
| `pnpm desktop:bundle:mac` | Passed: normal macOS arm64 bundle, 27.33 MiB reported by Tauri, strict ad-hoc signature verification; not installed or notarized |
| `pnpm test:native:recovery:runner` | Six acceptance regressions passed |
| Hidden native close/reopen | Two pairs / four fresh processes passed against PostgreSQL. Pending close timings were 43.29 ms and 38.16 ms, before the 400 ms debounce; latest drafts/preferences survived, the write rolled back and all three pre-observer invocation counters were zero |
| `pnpm test:native --runs 1 --cycles 10` | Passed: ten cleanup cycles, zero exit status, 2938 ms native elapsed time and 108.72 MiB main-process peak RSS; smoke observations, not representative performance or whole-process memory qualification |
| Workspace/write-fault ordering regression | Six workflows passed in 29.8 seconds, exercising persistence/layout tests immediately before all four fault cases |
| Final `pnpm test:e2e` | Passed: 124/124 workflows in 6.2 minutes, real Rust/Arrow/local database fixtures; Snowflake responses remain synthetic |

The first full browser attempt passed the previous 120 workflows, then timed out
before the first new case connected: the preceding workspace test had persisted
a hidden sidebar. That run was deliberately stopped: 120 passed, one setup
failure, one interrupted and two not run (exit 130). Each new case now resets
only the disposable harness workspace through its existing typed save command
before page load, preserving profiles. Formatting and TypeScript checks passed
after that test-only change. The ordering regression and final full suite then
passed without repository edits or source generators during browser execution.
The application and native artifact source did not change for this test repair.

The normal packaged executable SHA-256 is
`8b4cecb99466f6d6cb46f0106f0289022a715dc9d0c5a795362a63b818113f31`.
It remains at `src-tauri/target/release/bundle/macos/Joinery.app`, identity
`com.quiinthieu.joinery`, version `0.1.0`. Signature and digest were rechecked
after building the separate hidden executable. The hidden raw executable hash
is `dfd26489c4a9f3a3c0272bc9a766be71208f1cd239f9c79842c50dcf6c79f55b`;
its signed hidden bundle executable hash is
`0cd01d516fe13238d1c043986b1c568841fe64bcfe36fd9f3ec5226cc692ee2b`.
Build provenance records base `4ce9f3c` with `sourceDirty=true`; application
changes were subsequently committed in the two commits above. Existing Vite
chunk-size and `rust-objcopy`/`libLLVM.dylib` stripping warnings recurred; builds,
signature verification and all native processes passed.

Transient logs use `/tmp/joinery-writer-*.log`; the final browser log is
`/tmp/joinery-writer-browser-fixed-full.log`, and the ordering log is
`/tmp/joinery-writer-browser-order.log`. Native reports remain under
`.cache/native-smoke`, including recovery roots `recovery-yv1ju3y9` and
`recovery-4bxzgca1`. These are local evidence, not distributable release assets.
Required external engine/authentication pairs, normal installed-bundle/credential
behavior, signing/notarization, accessibility and owner acceptance remain open.

### Permanent object identity: historical failures and partial PostgreSQL repair

Read-only review confirms retained edit provenance stores schema/name, shape and
keys without a physical relation identity. PostgreSQL compares locked original
rows; MySQL/MariaDB also lock and compare table shape. Source inspection predicts
an identically dropped/recreated table, reseeded with the same row before those
locks, can receive an old approved edit. This was initially a source-inferred
limitation. The initial repository probe reproduced four PostgreSQL failures.
The durable diagnostic on `ff59ca6` then reproduced 27 across
PostgreSQL/MySQL/MariaDB. Those results remain the historical baseline; a
PostgreSQL-only repair and its pending qualification are recorded below. The
passing suites above predate and do not qualify the current identity work.

The original fixture sequence created a UUID table and row, obtained a table
browse result and separately an eligible SELECT result, then prepared/approved an
edit. An independent fixture session dropped/recreated identical DDL and reseeded
the row. The retained approval executed, and a fresh session inspected the replacement.
That sequence established the original four-case PostgreSQL failure. The later
opt-in diagnostic expands it to nine cases per engine; see below.

The initial source audit showed why identity must come from the original
read/preview and be verified before DML: capturing it only at prepare time misses
older retained results, and separate metadata lookup cannot prove which table the
query actually read. The partial PostgreSQL implementation below carries actual
query origin and validates the bound write portal. Engine-specific privileges,
partition/inheritance behavior, MySQL/MariaDB identity and CSV batch lifecycle
still need qualification; no timestamp/hash fallback or broader safety claim has
been introduced.

An attempted separate agent runtime investigation was rejected by automatic
review as a possible cybersecurity risk before any runtime action. It performed
only read-only log/source inspection; no files, processes, connections or tables
were created. No runtime evidence is counted from that rejected attempt.

The lead subsequently used an ordinary development-only repository fixture probe
on revision `95694e8`, through actual `Studio` calls and an independent observer,
against the already authorized synthetic PostgreSQL 18 fixture on port 55432.
The desired-safety assertion failed (exit 101): browse and SELECT updates after
identical replacement before preparation both committed to the new table;
SELECT update and browse insert after authorization did too. The observer found
note `approved` in the three update cases and two rows in the insert case.
Every approval was consumed and all four UUID tables were dropped before the
final assertion. No production or private application state was accessed.

The temporary failing probe was removed from normal fixture source; it is not
counted as passing coverage. Local source and observations are retained under
`.cache/table-identity-probe`. The durable [identity investigation](table-identity-safety.md)
records the case table, independent reproduction steps, code touch points,
engine privilege limits and complete-repair prerequisites. Independent design
reviews recommend backend-owned original-read identity and warn that a
prepare-time OID check cannot close the gap. The owner was asked whether pending
edits/imports must survive concurrent migrations. That remains an open
applicability question, not an automatic optional classification.

No application behavior, dependency, global configuration or privilege was
changed for that historical four-case probe. Namespace/partition changes remain
unexecuted.
DataGrip's reviewed official
documentation does not specify this exact concurrent replacement behavior; no
comparative DataGrip runtime claim is made.

### Durable three-engine identity diagnostic

On unchanged application revision `ff59ca6`, the new opt-in
[`table_identity_probe.rs`](../src-tauri/examples/table_identity_probe.rs) runs nine
actual `Studio` cases on each loopback fixture: PostgreSQL 18.6 (55432), MySQL
8.4.13 (53306) and MariaDB 11.4.13 (53307). These versions are recorded from the
connected drivers' server-version information. Run with the pinned Rust 1.99.1
toolchain, `RUSTC` and `RUSTDOC` used above:

```sh
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example table_identity_probe
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example table_identity_probe -- -D warnings
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example table_identity_probe -- \
  --run-disposable-fixtures --engine all --require-safe
```

Compilation, formatting and strict Clippy pass. Missing opt-in and invalid engine
arguments reject before opening a connection. Two strict actual runs both exited
**2**, reporting `safetyPassed: false`, 27 `Completed` mutations of replacement
tables and zero operational errors. All 27 approvals were consumed; all fresh
read/approved-insert controls worked; all owned table removals were independently
verified. No new passing fixture test is counted. The normal 103-fixture baseline,
browser/native evidence and bundle remain unchanged.

Coverage includes browse/query updates before prepare; query update and browse
insert/delete after approval; empty-result insert; CSV after preview, after
approval and between batch commits. The last case confirms 500 first-batch rows
before replacement, then observes old-preview row 502 committed to the replacement.
See the [case table and operational limits](table-identity-safety.md). Only fixed
loopback fixture profiles, UUID tables, temporary app roots and `MemorySecrets`
are used. The machine report is local ignored evidence at
`.cache/table-identity-probe/three-engine-report.json`; stderr is
`/tmp/joinery-table-identity-probe.log`. Report-only mode may exit 0 despite unsafe
observations; `--require-safe` and the JSON must be used for qualification.

A separate PostgreSQL client foundation is now committed as `ea3ea76`: vendored
`tokio-postgres 0.7.18` with MIT/Apache-2.0 licensing and provenance for three
source changes, preserving simple-query origins and describing bound portals
without execution. Six synthetic protocol tests and one ignored nine-case actual
client-protocol set pass. This is not parity evidence or application qualification.

### PostgreSQL identity repair: scoped evidence and browser status

Backend repair `4b7e4c3` carries actual relation OID, attribute number and type
from the executed SELECT, table browse or CSV preview through retained results,
plans, approvals and all import batches. The writer describes and executes the
same bound DML portal under its guard. The first repaired application report at
`.cache/table-identity-probe/postgres-repaired-report.json` predates the final
review fixes. The final strict `--engine postgres --require-safe` report at
`.cache/table-identity-probe/postgres-csv-final-report.json` passes all nine
PostgreSQL 18.6 scenarios: each rejected with `Conflict`, preserved the
replacement, consumed approval, passed fresh controls and cleanup, with zero
operational errors. A separate run of the formal example regression, exercising
the same nine cases, also passed. These are scoped fixture results, not general
engine qualification.

After the initial report, an auxiliary disconnect race was reproduced: a queued
write committed value `1` and returned `Completed` after its originating session
closed. A registration check under the retirement mutex now rejects that work; the
targeted rerun observed value `0` and `ConnectionClosed`. Raw browse
commit/drain behavior and permission-denied read-only fallback are implemented.
Four actual permission/RLS/raw tests passed, including column-only read-only
fallback and late-error cutoff. Sixteen new frontend state-behavior cases first
exposed 11 failures while five passed; after the fix, the 80-case state test file
passed. The latest `pnpm validate` passed 561 frontend tests / 52 files and 186
standard Rust tests with strict Clippy. One clean seven-suite fixture command
passed 112 tests: 41 MySQL/MariaDB, 43 PostgreSQL, four permission/RLS/raw, one
client-protocol suite covering nine scenarios, six Redis, two SSH and 15
transaction transport. The formal nine-case application diagnostic and its
separate example regression are additional runs, not part of 112. Earlier
transport failures were test-accounting/setup defects: proxy counts included
identity reads and browse setup consumed its approval with an unrelated prepare.
Those harness issues were corrected.

The nine-case focused browser run passed 9/9, including the CSV mapping repair.
After the test-coordinate fix and UUID-based fixture isolation, the full browser
suite passed 129/129 in 8.9 minutes; the focused table-data/raw-browse run passed
11/11 in 51.7 seconds. Both actual raw-browse unknown-outcome UI cases passed.
The earlier 126/127 full attempt and 10/11 focused attempt are retained as
historical test-coordinate and fixture-isolation failures; they are not current
product failures. Fresh native recovery and normal-bundle build/signature checks
also pass, with the exact source and artifact identities recorded below. Current evidence logs are
`/tmp/joinery-identity-csv-validate.log`,
`/tmp/joinery-identity-csv-fixtures.log`,
`/tmp/joinery-identity-csv-example-regression.log` and
`/tmp/joinery-identity-csv-browser-focused.log`,
`/tmp/joinery-identity-browser-full-final.log` and
`/tmp/joinery-identity-isolated-browser-focused.log`.

The application keeps column-only readers read-only; the permission fixture
verifies this behavior. Scope is PostgreSQL 18.6 only. OID identity alone does not prove every
trigger, rule, partition effect or OID-reuse case; grid writes remain separate
from console transactions. MySQL and MariaDB's 18 historical failures on
`ff59ca6` remain open and unsafe. The earlier 545 frontend / 180 Rust / 103
fixtures / 124 browser and normal-bundle SHA results belong to the prior writer
repair, not this identity increment. Normal packaged restart/credential behavior,
external engines/authentication and owner acceptance remain unqualified.

### Clean-source native and bundle evidence for the PostgreSQL identity repair

Both builds used clean commit `5762ee5aaf41cf3eecf3b86e1d250177db353daa`,
tree `82174fe9e3620ad78edc2f504fa8fb6c4b08f3ba`. Runtime repair is `4b7e4c3`.
Other commits in this qualification record isolate the browser fixture, extend
CI and record research/evidence. The working tree was clean before and after builds.

| Check | Result and boundary |
|---|---|
| `pnpm desktop:smoke:build` | Passed, release compilation 4m05s; copied hidden fixture manifest records `sourceDirty: false` |
| Hidden executable SHA-256 | `b35c755251d61bdbfc98d49dd7a8dae5e15b6617cc84ef3aeef0871e937c0879` |
| Hidden ad-hoc bundle executable SHA-256 | `e18637be77b78dce76b01f91293ae10a530f48e346b4a99b4969634f6cc6ceda` |
| `pnpm test:native:recovery:runner` | Six runner acceptance tests passed |
| `pnpm test:native:recovery --runs 2` | Two pairs / four fresh hidden native processes passed against actual loopback PostgreSQL; pending-close exit times 40.74ms and 29.12ms, below the 400ms debounce |
| Recovery behavior | Cancel preserved the session and manual transaction. Confirmed close persisted exact latest Unicode draft/preferences and rolled back the real fixture write. Both reopened processes made zero connect/prepare/execute calls before the explicit observer, restored no sessions/results/transactions, and removed their owned tables |
| `pnpm test:native --runs 1 --cycles 10` | Passed, ten synthetic cleanup cycles; native elapsed 2882ms and main-process peak RSS 107.83MiB. This is smoke evidence, not representative startup/performance qualification |
| `pnpm desktop:bundle:mac` | Passed, release compilation 3m01s; normal macOS arm64 `Joinery.app` 27.42MiB, version `0.1.0`, identifier `com.quiinthieu.joinery`; strict ad-hoc signature verification passed |
| Normal bundle executable SHA-256 | `79e59c646962ecd8793ced3c42bf6a9b62d1fa156d86279671ea6f168226c107` |

Native machine evidence is `.cache/native-smoke/build.json`,
`.cache/native-smoke/recovery-runs.json` and `.cache/native-smoke/runs.json`.
Recovery roots are `recovery-pdzcul_x` and `recovery-6ju1668l` under that directory.
Logs are `/tmp/joinery-identity-native-build.log`,
`/tmp/joinery-identity-native-recovery.log`,
`/tmp/joinery-identity-native-smoke.log` and
`/tmp/joinery-identity-normal-bundle.log`. Existing nonfatal Vite chunk-size and
Rust `rust-objcopy`/`libLLVM` strip warnings remain recorded in build logs.

The normal bundle was built and verified, not launched, installed, notarized,
published or released. Hidden fixtures use temporary roots and in-memory secrets;
they do not qualify normal OS credential persistence, installed restart behavior,
other platforms, accessibility or the owner's actual workflows. MySQL/MariaDB's
18 application identity failures remain open. No production or private
application state was accessed.


## MySQL/MariaDB retained guards and exact CSV preview ownership

Current increment on `feat/datagrip-replacement`, parent `d8e1ece`; uncommitted at
this record's first write. The previous 18 unsafe MySQL/MariaDB observations on
`ff59ca6` are historical baseline evidence. Final integrated qualification and
clean-source artifacts are being recorded here before the increment is committed.

The application now retains an ordinary-role READ COMMITTED read-only transaction
on a dedicated connection before eligible browse/query/CSV source reads. Its
metadata lock protects the original table incarnation. The writer first acquires
its own target lock, freshly verifies that original connection with non-reconnecting
`COM_PING`, confirms `IN_TRANS`, generation/session/canonical-table identity and
absolute deadline, then checks schema/full-row conflicts and performs DML in the
same writer transaction. Every CSV batch repeats original-target verification.
Names corroborate the source; names alone never authorize a write. A writer that
crossed its own-lock/fresh-verification point can finish after later lease expiry.

Production bounds are 60-second nonrenewing lifetime, 32 guards per session
(reject full, no eviction), 10-second acquisition, three-second verification,
two-second close, five-second writer/non-read editor metadata-lock wait and the
existing 65-second transport ceiling. Last owner, active expiry, cancellation,
disconnect and shutdown revoke the actual guard socket. Approved application DDL
or unclassified SQL revokes retained MySQL/MariaDB guards across the backend before
submission. External DDL can wait for a lease; applicable migration blocking and
daily usage still require owner acceptance. The diagnostic injects a hidden
Rust-only four-second policy, never a profile/IPC/persisted production setting.

### Reproduced defects and repairs

- Three frontend regressions initially failed while 80 existing state cases
  passed: a completed read became a false schema error when its edit lease ended.
  Pages now retain successful rows and only narrow write capabilities when every
  structural field is unchanged. Negative metadata/capability cases still reject.
- An actual stored-function raw browse produced rows beyond the retention bound
  and a later server `SIGNAL`. Stopping consumption early had falsely reported
  completion. Browse now drains its bounded server query through the terminal
  response, including discarded rows. Known server failures remain `Database`;
  uncertain transport loss after submission becomes `OutcomeUnknown`, closes the
  backend and consumes approval without replay. An unchanged-byte loopback relay
  qualifies that actual loss path with independent marker observation.
- `/tmp/joinery-csv-required-token-baseline.log` records an actual PostgreSQL
  failure before hardening: preview A targeted an original table; after
  replacement, preview B targeted the new table. Omitting the token selected B,
  completed, and independently observed `(1, reviewed), (2, replacement)` instead
  of only `(2, replacement)`. UUID cleanup ran before propagating the failure.
  Core preparation now requires `&str`, named native IPC requires `String`, and
  the typed client requires `string`. Exact `(session, source, preview)` lookup
  replaces latest-preview selection. Stale A, released A while B is the only
  candidate, empty tokens and changed options/mappings reject; fresh B commits
  once. The focused actual regression passes after repair in 1.08 seconds.
- Each CSV preview owns a distinct opaque generation. Late preview responses,
  replacement, session changes, unmount and completion release only their own
  unused plans/targets. Target release and asynchronous plan publication share
  the target lock; release cannot resurrect an already closed preview. Submitted
  DML remains owned by its executing plan and keeps its truthful outcome.

### Preliminary checks before the last token/TLS follow-up

`pnpm validate` passed 580 frontend / 52 files, 193 standard Rust tests, production
build, contracts, formatting, checks and strict all-target/all-feature Clippy.
One clean eight-suite fixture run passed 123 tests: 42 MySQL/MariaDB, 10 guard
lifecycle, 43 PostgreSQL, four permission/RLS/raw, one client-protocol suite
(nine scenarios), six Redis, two SSH and 15 transaction-transport tests.
The strict three-engine application diagnostic passed all 27 original replacement
scenarios with `safetyPassed: true`, no operational errors, unchanged replacements,
consumed approvals, successful fresh controls and verified UUID cleanup. Its JSON
SHA-256 is `30dba3f7848fc1d4fdbb998cc6941308291bca60d31bdcfda3314bb5a97c86b7`.
MySQL/MariaDB external replacement waited 3824–4021ms under the four-second policy.
The formal ignored example regression passed two tests covering those 27 cases.

Focused browser tests passed 11/11 in 49.0 seconds; the full suite passed 137/137
in 7.4 minutes. The two added raw-function workflows independently observe a real
write, preserve completed read-only rows after guard revocation, exercise a
disabled exact-value cell viewer, reject approval replay and verify cleanup.
An initial 9/11 focused attempt used an incorrect assertion that a read-only
cell viewer would not exist; actual disabled-viewer evidence corrected the test.
Cleanup runs sequentially so one prepare cannot consume another cleanup plan.
These counts predate the final mandatory-token and TLS test additions.

Preliminary logs: `/tmp/joinery-mysql-guard-validate-clean.log`,
`/tmp/joinery-mysql-guard-fixtures-final.log`,
`/tmp/joinery-mysql-guard-identity-final-report.json`,
`/tmp/joinery-mysql-guard-example-final.log`,
`/tmp/joinery-mysql-guard-browser-focused-final.log` and
`/tmp/joinery-mysql-guard-browser-full.log`.

### Required-token and verified TLS-over-SSH follow-up

Focused frontend ownership/contract tests pass 15/15, including a preview without
ownership refusing to prepare. Application and explicit E2E TypeScript checks
pass. Real named-RPC missing/null/empty-token assertions are added to all three
CSV replacement browser workflows; valid preview B remains available for its
successful control. Hidden native smoke now bypasses typed callers to exercise
Tauri's actual missing/null argument decoder as well. Final runtime results follow.

The new `mysql_guard` verified TLS-over-SSH test passes on MySQL and MariaDB
(2.97 seconds). A transparent relay forwards original bytes and parses only each
owned channel's 32-byte MySQL SSLRequest/CLIENT_SSL and real TLS ClientHello SNI
`database.fixture.test`. All accepted channels must satisfy those checks; browse
adds at least three channels and an approved writer adds another. Actual opposing
DDL returns 1205 while the guard is held, and an independent ordinary-role
observer reads `tls-approved` after the writer completes. Replay is rejected,
all relay sockets close and owned table absence is checked. This positive test
proves successful configured VerifyFull TLS with original SNI, not independent
wrong-CA/hostname rejection. Existing TLS negative fixture tests retain their
separate scope. No TLS termination, fabricated server responses, privileged grants
or global server settings are used. Focused strict Clippy passes.
Logs: `/tmp/joinery-mysql-guard-tls-ssh-first.log` and
`/tmp/joinery-mysql-guard-tls-ssh-clippy.log`.

Independent bounded reviews of the driver lifecycle, writer order, raw browse,
CSV ownership/publication, frontend capability narrowing, real IPC boundary tests
and TLS relay found no remaining material issue. Review graph coverage metadata
reported no recorded gaps at generation `2026-11-09T12:48:17Z`; exact changed
source was also read. That signal is best effort, not a completeness proof.

The final combined checks and clean-source native/bundle artifacts remain to be
recorded below. This increment does not qualify normal installed credentials,
external authentication, other platforms, representative performance/accessibility
or the owner's required daily workflows. No push, install, publication or private
application-state access occurred. The Goal remains active and unaccepted.


### Owner target clarified during guard qualification

The owner confirmed Snowflake SSO on macOS, specifically
`authenticator=externalbrowser`, and all current DataGrip capabilities applicable
to that workflow. Reads, writes, schema changes and explicit transactions are
required. The earlier read-only browser-OAuth pilot is not an accepted narrowed
target. Official driver research and a source/architecture audit now determine
the stateful implementation route. Current local-database checks continue to
protect the existing application, but do not demonstrate Snowflake replacement.
No live account, private DataGrip state, billable query or Snowflake write has
been accessed or authorized by this clarification alone.


### Combined required-token/TLS source: validation and real fixtures

The final integrated `pnpm validate` passes **581 frontend tests / 52 files**,
**193 standard Rust tests**, contracts, production build, formatting/checks and
strict all-target/all-feature Clippy. The unchanged nonfatal Vite chunk-size
warning remains. Explicit E2E TypeScript checking passes as well.

One clean eight-suite actual fixture command passes **124 tests**: 42
MySQL/MariaDB, 11 retained-guard lifecycle/TLS, 43 PostgreSQL, four
permission/RLS/raw, one client-protocol suite (nine scenarios), six Redis, two SSH
and 15 transaction transport. The 11 guard tests loop across both MySQL and
MariaDB; that is not counted as 22 tests. The strengthened actual PostgreSQL CSV
ownership regression is included in 43, not counted again. A separate formal
example run passes two tests covering all 27 strict identity scenarios in 83.55
seconds. These results include the required-token API and the TLS-over-SSH test.

Commands use the pinned local Node/Rust environment above:

```sh
pnpm validate
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test postgres --test ssh --test mysql --test mysql_guard --test redis \
  --test transaction_transport --test postgres_protocol \
  --test postgres_identity_permissions -- --ignored --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example table_identity_probe -- --ignored --test-threads=1
pnpm exec tsc --ignoreConfig --noEmit --strict --skipLibCheck \
  --module ESNext --moduleResolution Bundler --target ES2022 --lib ES2022,DOM \
  --esModuleInterop tests/e2e/table-identity.spec.ts
```

Logs: `/tmp/joinery-mysql-guard-token-validate.log`,
`/tmp/joinery-mysql-guard-token-fixtures.log`,
`/tmp/joinery-mysql-guard-token-example.log` and
`/tmp/joinery-csv-required-token-e2e-types.log`.
Full final browser and clean-source native/bundle outcomes follow when measured.


### Frozen final browser qualification and local code commit

The first final browser attempt passed 135/137 and failed completion/context and
index/constraint workflows. Both traces showed unexpected page reloads while
documentation was being edited. The run is recorded as failed, not qualification;
its log/traces are archived in `/tmp/joinery-browser-contaminated-20261109/`. No
assertions or timeouts were weakened. With **all repository edits frozen**, the
full rerun passes **137/137 in 7.3 minutes**, including those two workflows, actual
named-RPC missing/null/empty CSV preview ownership, stale-target rejection and
known/unknown transaction outcomes. Log:
`/tmp/joinery-mysql-guard-token-browser-clean.log`. Command: pinned-environment
`pnpm test:e2e`. The reload cause is inferred from trace timing; the clean rerun
does not establish a deterministic watcher root cause.

Code is locally committed as `997680e` (`fix(mysql): retain original edit target
leases`). Final source evidence is 581 frontend / 193 standard Rust tests, 124
actual fixture tests, 27 strict identity scenarios and 137 browser workflows.
Clean-source native/bundle qualification follows separately; earlier artifacts
do not qualify this revision. Nothing was pushed, installed, merged or published.

### Latest-release and workload clarification

The owner selected the latest stable DataGrip release and results up to
**1,000,000 rows**. JetBrains' official release API was checked directly on
9 November and reports DataGrip **2026.3**, build `263.7307.216`, released
5 November 2026. The source-linked Snowflake official benchmark records current
features, including Air, cross-database queries and column controls, without
assuming their provider entitlements or Snowflake applicability. A research draft
misidentified 2026.3 as prerelease; direct release-API verification corrected it
before repository integration. Current result retention (100,000 rows and a
global 64 MiB RAM budget) cannot satisfy the required workload. Million-row
streaming, complete bounded paging/export and failure/cleanup behavior are
required implementation and actual-engine acceptance gates.

The official Node SDK 3.4.1 is the selected conditional transport candidate for
externalbrowser and persistent sessions. Scratch helper/supervisor development
is separate from the application, with no enabled Snowflake write capability.
No real Snowflake account/SSO/warehouse was accessed. SDK retries, remote outcomes,
macOS browser lifecycle, packaging and live correctness remain unqualified.

### Clean-source native and bundle qualification: `6218e60`

All repository edits stayed frozen during these checks. Source revision is
`6218e603fcd3915360fadbc3846fd8b5820d56b7`, tree
`7798b0f48cbe3815d00971953306a9fbe447a57e`, dirty flag **false**.
Pinned Node 24.22.2 / Rust 1.99.1 commands:

```sh
pnpm desktop:smoke:build
pnpm test:native:recovery:runner
pnpm test:native:recovery --runs 2
pnpm test:native --runs 1 --cycles 10
pnpm desktop:bundle:mac
```

The recovery runner passes six tests. Both fresh-process recovery pairs pass:
pending edits/preferences survive, the actual disposable PostgreSQL transaction
rolls back on close, and startup restores no session/result/transaction or SQL
replay. The ten-cycle hidden native smoke passes all 17 checks, including the
new missing/null CSV preview-token IPC rejection. Its measured native completion
is 4,289 ms; main-process peak RSS is 99.14 MiB, not total application memory or
visible cold-start qualification. Local evidence is `.cache/native-smoke/runs.json`
(SHA-256 `c24301f52c4a5939a148dc2dbea27d286427065e733cd6fc82e71dc3fc0d313d`)
and `recovery-runs.json`
(`458043dcef8cf0ff76a5a1d27fa3705829c8dd7ecd8c20da106df217408ffb12`).
Hidden executable SHA-256:
`2de206dcdf516e6d3b0e815af6f4a56ed6883e1050b4f6a717dcf3342518185f`.

The normal 27.56 MiB macOS bundle builds and passes strict deep ad-hoc signature
verification. Executable SHA-256:
`b99b0b2874ff12d0cdf3a3a5001fb6b4123d08bf7c46f69e03062a87d23ecfd7`.
Logs are `/tmp/joinery-guard-native-{build,runner,recovery,smoke}.log` and
`/tmp/joinery-guard-normal-bundle.log`. The existing Vite chunk-size and missing
`libLLVM.dylib` stripping warnings remain nonfatal. No normal bundle was launched
or installed; normal packaged restart, OS credentials, Developer ID signing,
notarization and Snowflake acceptance remain open. Nothing was published.

### Million-row storage increment: baseline and first pass

The owner requires results up to one million rows. Before implementation, the new
`large_results` actual PostgreSQL fixture failed with **99,840 retained rows**
instead of 1,000,000 (`/tmp/joinery-million-row-baseline.log`, 2.28 s). Initial test
construction errors (an incorrect event variant and a read-only classification
rejection) were corrected before that meaningful baseline; no product policy was
weakened. The test uses only synthetic `generate_series` data on the disposable
PostgreSQL fixture and performs no account or persistent table writes.

An initial opt-in store passes the actual test in **23.08 s**:
`/tmp/joinery-million-row-first.log`. It retains one million five-column rows with
exact bigint/decimal text, Unicode, NULL/empty values and 96-byte payloads; checks
random/boundary/last-row Arrow pages; streams all one million CSV records and
compares their full hash/count; and releases the temporary bytes. This pass
predates the final combined metadata accounting, so final qualification follows
separately. Commands use pinned repository toolchains:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test large_results -- --ignored --nocapture
```

Independent review found that restoring a metadata backup did not reapply the
stored privacy preference to the active result store. The new restore regression
fails before the fix (`/tmp/joinery-storage-restore-baseline.log`). Restore now
disables temporary storage immediately after replacement, then enables it only
when the restored setting explicitly confirms `true`. The reviewer also required
combined retained metadata/schema/index accounting and conservative disk charging
when failed write rollback cannot inspect the physical tail. Final review/tests are recorded below. No real Snowflake SSO, queries or mutations were performed.

### Million-row final-source checks

Independent review cleared combined retained metadata/schema/index accounting,
map-capacity refunds, private anonymous handles, lazy batch reads, prefix
preservation after append failure, and conservative charging when rollback and
file metadata both fail. Fourteen focused result-store tests pass. An additional
UI regression reproduced recovery controls remaining enabled during a pending
storage-preference save (`/tmp/joinery-storage-preference-lock-baseline.log`);
recovery/passphrase controls now block that overlap and all five Preferences
tests pass (`/tmp/joinery-storage-preference-lock-final.log`).

`pnpm validate` passes **582 frontend / 205 standard Rust tests**, contracts,
types, production frontend build, formatting, native check and strict all-feature
Clippy (`/tmp/joinery-million-row-validate-combined.log`). The first run reached
the same passing tests but failed a collapsible-if lint; the equivalent let-chain
fix passes. This final combined validation includes the revised fixture assertions
and native preference/restart checks.

The final accounting implementation passes all three `large_results` tests in
**21.75 s** (`/tmp/joinery-million-row-qualified.log`): actual PostgreSQL one
million rows with exact values/boundary/random/last pages and full streamed CSV
hash/count; opt-in persistence without result replay; and restoration of both
enabled and disabled metadata preferences. No Snowflake account was used.

The first combined fixture attempt found an obsolete zero-byte assumption in
MySQL export setup: empty statement descriptors now have accounted metadata.
A PostgreSQL run reproduced the same assumption in export and comparison
(`41/43` passed). Tests now compare the exact before/after retained bytes and
descriptor counts, plus temporary bytes, proving those workflows add no result
storage while allowing setup metadata. Logs:
`/tmp/joinery-million-row-fixtures.log` and
`/tmp/joinery-million-row-postgres-accounting-baseline.log`.
The final combined actual fixture run passes **124/124**
(`42 MySQL/MariaDB + 11 guards + 43 PostgreSQL + 4 permission + 1 protocol
fixture + 6 Redis + 2 SSH + 15 transaction transport`):
`/tmp/joinery-million-row-fixtures-final.log`. These remain local fixture
results; no actual Snowflake behavior is inferred. Browser and native outcomes follow below.

The owner requested reuse of the current Snowflake connection. Only allowlisted
non-secret default-profile configuration was read and retained in an ignored,
owner-only local cache. No credential/token state, DataGrip private state,
Snowflake connection, billable query or external write was accessed.

### Million-row browser qualification

All **138/138 browser workflows pass in 7.7 minutes** with repository edits and
generators frozen (`/tmp/joinery-million-row-browser-all.log`). The dedicated
million-row flow also passes independently in **19.7 s**
(`/tmp/joinery-million-row-browser-focused-qualified.log`). It saves and reloads
the explicit privacy preference, runs the actual disposable PostgreSQL query,
reaches row 1,000,000 and 999,999 by keyboard, checks bounded frontend caching,
disables future spill while retaining the current snapshot, exports the full
snapshot and closes it to release temporary bytes. No console errors occur.

The CSV has **103,888,906 bytes**, one header and **1,000,000 data records**.
It uses LF separators without a final LF, preserving existing export behavior.
The complete independently computed expected/file SHA-256 is
`958d2140f0d9ac59335d33e2c17b9ed363c4c08ee8cf5e4db5775ae59f89d34e`.
Early test construction assumed synchronous checkbox acknowledgement and a final
LF; both assertions were corrected without changing application behavior. Those
logs are `/tmp/joinery-million-row-browser-focused.log` and
`/tmp/joinery-million-row-browser-focused-final.log`; artifacts are retained in
the correspondingly named checkbox/newline baseline directories under `/tmp`.
Final dedicated TypeScript and Prettier checks pass. Native preference IPC and
fresh-process restart outcomes follow below.

### Million-row increment: clean native and bundle checks

The implemented storage increment is `f3f257e`. Native and normal bundle checks
use clean source `c860a786c6bd7051da00b6da2fe6101017c99140`, tree
`076f44144c40719402a6a67b96a52af3887c1bdb`, dirty flag **false**. All edits
and generators remained frozen while the checked artifacts were built and run.
Pinned Node 24.22.2 / Rust 1.99.1 commands:

```sh
pnpm desktop:smoke:build
pnpm test:native:recovery:runner
pnpm test:native:recovery --runs 2
pnpm test:native --runs 1 --cycles 10
pnpm desktop:bundle:mac
```

The runner passes six tests. Both fresh-process recovery pairs pass, including
the new `temporaryResultStorageRestored` check. Startup restores no result rows,
live session or transaction and invokes no SQL; actual disposable PostgreSQL
confirms rollback of the previous pending transaction. Ordinary hidden native
smoke passes all **17 checks**, including temporary-storage command permission,
enable/disable persistence and restoration of the prior setting, over ten
cleanup cycles. Its measured native completion is 3,082 ms; main-process peak RSS
is 107.83 MiB. These are synthetic hidden-fixture observations, not total memory,
visible/cold startup, native million-row performance or Snowflake qualification.

Local evidence under `.cache/native-smoke/`:

| Artifact | SHA-256 |
|---|---|
| `joinery-smoke` | `8c785b4f3aba2138ae00786abd86affccf7221fc76c4fc58da0182cd3e685e0f` |
| `runs.json` | `09737c9da7eabe7b0738b0c4b0c00027441b30f8135be9178df1f8e6601a58a1` |
| `recovery-runs.json` | `a2c418d957be0aac4a7015d7d13ee74e6e0078045c835f49b0263d9cc7eee09d` |

The normal **29.03 MiB** app bundle passes strict deep ad-hoc signature
verification. Its executable SHA-256 is
`f38a8a6eb3e4a79f15a05f33064c94f18f816456bc872546f35f5bdbd4eeeeb8`.
Logs: `/tmp/joinery-million-row-native-{build,runner,recovery,smoke}.log` and
`/tmp/joinery-million-row-normal-bundle.log`. The pre-existing Vite chunk-size
and missing `libLLVM.dylib` stripping warnings remain nonfatal. The normal bundle
was not launched or installed; normal packaged restart, credential persistence,
Developer ID signing, notarization and actual Snowflake qualification remain open.
Nothing was pushed, published or deployed.

The integration workflow now selects the dedicated `large_results` fixture in
its actual-database command. The exact additional selection passes locally:
`cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test
large_results -- --ignored --test-threads=1` (**1/1**, 18.53 s;
`/tmp/joinery-million-row-ci-selection.log`). Workflow YAML parses. This is
local verification of the configured check, not a remote CI run.
