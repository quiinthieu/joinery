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
do not identify a published preview. The follow-up below changes native test
instrumentation only; the normal bundle's executable digest remains unchanged.
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
not normal installed-bundle or OS credential qualification. The normal app bundle
retains the digest recorded above. The prior 97-fixture and 120-browser results
remain the application qualification baseline; those full suites were not rerun
for this native-test-only increment. Other engines, external authentication,
visible native accessibility/performance and owner acceptance remain open.
