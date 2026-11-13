# Replacement verification ledger

Run started 9 November 2026 on macOS arm64, source `bcb606f`, task branch
`feat/datagrip-replacement` from fetched `origin/main`. Local work is unmerged,
unreleased and not installed. Actual-account evidence is limited to the explicit
canary scopes recorded below; complete external workflow qualification is open.
See [acceptance](replacement-acceptance.md) for required gates and migration.

## Environment and commands

### Process-owned history recovery (13 November 2026)

Base `b575c4c` on `feat/datagrip-replacement`. Three bounded `gpt-6.1-sol`
high-effort agents own storage/tests, the real child-process fixture, and
independent read-only review. The lead owns runtime integration, coordination,
serial verification and documentation. Existing model selections were reused;
no fallback or global configuration change. Build checks use the 20 GiB
free-space gate and preserve unique logs plus exact source hashes under
`.cache/verification/reviewed-console/`. Synthetic local data only; no account
activity. The private hub remains unchanged under the no-publication boundary.

| Evidence label | Actual result |
| --- | --- |
| `history-owner-process-red` | Exit 101 before the intended assertion: fixture released result descriptors too early. This is not defect reproduction |
| `history-owner-process-fixture-corrected-red` | Actual PostgreSQL confirms the child is sleeping in the reviewed script; after killing/reaping only that child, restart incorrectly leaves history Admitted instead of Interrupted. Exit 101, 6.054 seconds, unchanged source. Scoped synthetic table/sequence and backend cleanup |
| `history-owner-first-unit` | Nine pass / one fails at imported-branch fixture with unrelated privacy lineage. One audit-reader version-routing helper changed during compilation, so this is not frozen-source qualification. The privacy conflict is preserved; fixture corrected to fork a compatible pre-admission SQLite snapshot |
| `history-owner-unit-corrected` | All ten real SQLite ownership cases pass, exit zero, 25.272 seconds including compile, unchanged sources. Covers held/dead/missing/invalid proof, hardlinks/symlinks/permissions/stale handles, original completion, privacy suppression, exclusive gate, restore provenance, invalid ownership metadata and cached v5 writer fencing |
| `history-owner-process-green` | Actual PostgreSQL child-process crash passes, exit zero, 20.595 seconds including compile, unchanged sources. Live-owner observer leaves Admitted; killed/reaped owner recovers Interrupted with exact original SQL, no status/duration, prior completion preserved. Sequence/table detect prefix or suffix replay. No automatic remote execution |
| `history-owner-validate` | Full validation stops at standard Rust: 1,038 frontend and 622 library cases pass / five fail / four intentional library ignores. All five failures occur in one test fault helper still constructing dynamic `{table}_v5` trigger targets; no fault was installed. Exit 101, 133.57 seconds, unchanged source. Helper corrected to schema-6 names; assertions and production behavior unchanged |
| `history-owner-validate-fixture-corrected` | All 1,038 frontend / 687 standard Rust cases pass, 144 default fixture ignores. Full validation then fails formatting on one assertion in the new process fixture; desktop compile/strict Clippy not reached. Exit 1, 191.862 seconds, unchanged sources. Fixture formatted with the repository's Rust 2024 settings; no semantic change |
| `history-owner-check-rust-formatted` | Formatting, desktop compilation and strict all-target/all-feature Clippy pass, exit zero, 40.508 seconds, unchanged source |
| `history-owner-validate-reviewed` | Complete pnpm validate passes: 1,038 frontend / 687 standard Rust cases, 144 default fixture ignores, formatting, contracts, TypeScript, production UI build, desktop compile and strict Clippy. Exit zero, 222.248 seconds, unchanged source |
| `history-owner-postgres` | All ten actual isolated PostgreSQL script cases pass, 3.321 seconds, unchanged source; cancellation, connection loss, script/transaction handling and active-query privacy/failed completion retry remain verified |
| `history-owner-process-reviewed` | Final-source actual PostgreSQL child-process crash case passes again, 0.852 seconds with compiled executable reuse, unchanged source. This includes the fixture-only metadata target and Rust 2024 formatting corrections |
| `history-owner-browser` | All six actual headless Chromium cases pass, 72.333 seconds including bridge startup, unchanged source. Real SQLite paging/search/palette and actual PostgreSQL active-query clear, late completion, renderer restart and reviewed parameter writes pass; screenshots are not substituted for interaction |
| `history-owner-snowflake-producers` | All 73 synthetic SDK execution cases pass, 262.794 seconds including compilation, unchanged source. Includes actual SQLite release-failure regression retaining the hold after completion/audit without redispatch; remote behavior is synthetic, not account qualification |
| `history-owner-qualification-runners` | All 27 offline runner cases pass, two intentional optimized/full-runtime ignores, 446.313 seconds including Cargo startup, unchanged source. Covers mutation child exits before/after ACK, dropped waiters, storage failures, context drift, deadline cleanup and bounded capture; no account activity |
| `history-owner-independent-review` | Read-only substantial increment review finds no material issue: all three admissions, task lifetimes, completion/audit/release holds, dead-owner recovery, both restore paths, current-only authority, physical v5 fencing and legacy validation routing. Latest audit-reader source checked directly after graph metadata changed; no recorded coverage gaps in reviewed paths, best-effort scope |
| `history-owner-final-coverage` | All 28 changed Rust paths report metadata-match / no recorded issue at graph generation 2026-11-13T05:24:22Z, metadata 05:30:57Z. Recording and hashes complete. This is best-effort recorded coverage, not proof of exhaustive correctness |
| `history-owner-doc-links` / `history-owner-doc-review` | All 783 repository-relative Markdown path targets exist; fragments not validated. Read-only documentation review found two stale historical/schema-scope descriptions, corrected before commit; no acceptance gate was removed |
| `history-owner-fixture-stopped` | Only the lead-owned isolated PostgreSQL fixture was stopped after database/browser checks. The unrelated logistics database remained running; no user process was stopped |
| `history-owner-prior-app-preservation` | Prior clean `952758b` normal app preserved using APFS copy-on-write before later packaging. All 15,120 inventory entries match the original artifact and strict signature passes. Regular-file inventory 242,581,015 bytes. Net free-space decrease was 212,934,656 bytes while full validation was also active, so it is not attributed solely to cloning. Nothing deleted or installed |
| `history-owner-normal-package` | Normal SDK-bearing ARM64 app packages from clean `f39dce374eb3844765ad0460405565c0c96a15de`, exit zero, 263.887 seconds. All 562 source files unchanged. Resources and guarded SDK 3.4.1 startup pass with zero authentication/SQL; existing nonfatal LLVM stripping and ts-rs attribute warnings remain |
| `history-owner-app-artifact` | Version 0.1.0, 34,699,712-byte ARM64 executable; strict ad-hoc signature and original license bytes verified. All 562 hashes equal all six final check receipts, including both native suites. Preserved schema-5 app inventory unchanged. No install, account qualification or owner UI acceptance |
| `history-owner-final-doc-links` / `history-owner-final-doc-review` | All 783 relative Markdown path targets exist; fragments not validated. Final independent read-only review finds no material issue and separately confirms the package pins and all six matching source scopes. Next durable-outcome design explicitly includes phase/dedup authority, all-live-owner migration exclusion, volatile rollback proof and hard memory/churn accounting; implementation and process acceptance remain required |

Full validation, database/browser/native checks and independent review pass.
All six final check receipts capture the same 562 source files without changes
during their run. The normal local app at
`src-tauri/target/release/bundle/macos/Joinery.app` is clean `f39dce3`, schema 6.
Executable SHA-256:
`3162d333473fea17f7670816f4e2599957e208dfe2b18beaf2b7d7ecdd7e1b46`.
The prior `952758b` schema-5 app remains at
`.cache/verification/reviewed-console/preserved-release-952758b/Joinery.app` with
all 15,120 inventory entries unchanged. Keep older apps separate from schema-6
evaluation metadata. Artifact capture records 206,131,306,496 free bytes
(about 192 GiB), 9,344,732 KiB debug and 3,190,416 KiB release artifacts. No build
remains running and no additional deletion occurred. Dead-owner proof assumes
Joinery's private directory/stable inodes, not resistance to a same-user actor
replacing valid proof files. Lifetime proof/ticket/privacy budgets and durable
failed generic/typed completion payloads remain required; a recorded admission
does not preserve an uncommitted original outcome across process death.

Reproduce the crash acceptance only against the disposable loopback PostgreSQL
fixture described in README:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test history_process \
  postgres_history_recovers_only_after_real_process_death_without_replay \
  -- --ignored --exact --nocapture --test-threads=1
```

The helper-only ignored case must never be invoked separately. Standard test
runs intentionally leave both process cases ignored.

### Atomic history and privacy integration (13 November 2026)

Base is `e3ebaaf` on `feat/datagrip-replacement`. Three bounded
`gpt-6.1-sol` high-effort agents own storage, console runtime/independent review,
and encrypted recovery coverage. The lead owns generic/typed execution,
integration, real-database/browser checks and final verification. Shared sources
are frozen during each serialized check; each receipt captures exact source
hashes and disk usage under the 20 GiB free-space preflight. These checks use the
coordinated worktree source; normal packaging uses clean feature commit
`952758b039fba06ef9aaa9b284edb6acabe71c43`.
No account activity is authorized or performed by these local checks.

| Evidence label | Actual result |
| --- | --- |
| `history-atomic-prepare-reproduced` | Exit zero but **zero tests** because an unqualified name was combined with `--exact`; this is not reproduction evidence |
| `history-atomic-prepare-red` | Correct fully qualified case fails at `Prepare must durably preserve history before Started`, exit 101, 2.67 seconds, unchanged sources. Actual Studio/SQLite/private Node helper with synthetic SDK; callback prevents dispatch, no Snowflake account |
| `history-atomic-first-compile` | Same exact case now passes: one passed / 1,118 filtered, exit zero, 24.499 seconds including compile, unchanged captured sources. This proves the admission regression only, not the full workflow |
| `history-atomic-first-image-persistence` | Pinned first-compile executable: 109 pass / 29 fail / four ignored, 32.607 seconds, unchanged binary SHA-256. All eight then-compiled encrypted history privacy cases pass. Failures expose stale physical-schema fixtures, unticketed raw history, changed publisher rules and restore integration errors; they are not dismissed as pre-existing. Later source edits are outside this compiled image's scope |
| `history-atomic-standard-first` | Current standard library run: 576 pass / 32 fail / four ignored, exit 101, 63.986 seconds, unchanged sources. All nine encrypted history cases pass, including actual v4 migration → encrypted backup → live clear → two product restores. Generic panic/callback/shutdown tests and typed privacy races pass. Three old typed tests assumed history stays empty until ACK; updated assertions must instead require one admitted row with no fabricated completion. Remaining failures overlap the storage/legacy fixtures above |
| `history-atomic-standard-corrected` | Compile stops before tests: five missing/ambiguous imports in newly added storage test helpers, exit 101, 9.887 seconds, unchanged sources. Fix required; no passing runtime claim from this attempt |
| `history-atomic-standard-fixtures` | Interrupted during compilation without a result receipt or observed tests. The original PTY handle is missing and no matching build/test process remains; preserved log and interruption receipt record that evidence. No pass inferred; resumed under a new label |
| `history-atomic-standard-resumed` | All 615 standard library tests pass / four intentional ignores, 91.329 seconds including compile, unchanged sources. Includes corrected fixtures, physical v4 writer fencing, actual pruning and all nine encrypted history cases. This run precedes the recovery guard addition below |
| `history-atomic-recovery-guard-red` | New real-SQLite regression fails because recovery Create admits while a known generic completion remains pending after an injected history UPDATE failure. One fails / 619 filtered, exit 101, 29.916 seconds, unchanged sources. Review established missing guard enrollment; it did not establish restore data loss |
| `history-atomic-recovery-guard-green` | The same real-SQLite case passes after enrolling pending generic history completions in exclusive recovery admission. One passes / 619 filtered, 22.681 seconds including compile, unchanged sources. After the injected fault is removed, the original known completion is recorded and the same previously refused recovery identity succeeds |
| `history-atomic-postgres` | All ten actual isolated PostgreSQL script cases pass, 20.816 seconds including compile, unchanged sources. Includes exact original SQL across restart; clear, opt-out/re-enable and delete/recreate during a verified active query; failed completion writes followed by clear/retry/restart; cancellation, interrupted connection, skipped COMMIT and explicit rollback. Synthetic local database only |
| `history-atomic-validate` | Full validation stops at a legacy integration fixture: 1,038 frontend tests and 616 standard library tests pass; history integration is seven pass / one fail because raw unticketed rows were inserted into current schema 5. Startup correctly rejects that invalid authority. Exit 101, 175.428 seconds, unchanged sources. The fixture must create an actual legacy schema and exercise migration; desktop/strict checks were not reached |
| `history-atomic-legacy-integration` | Compile stops before tests because the revised legacy fixture passes `u64` directly to rusqlite. Exit 101, 0.725 seconds, unchanged sources. Corrected to checked `i64` conversion; no runtime pass inferred |
| `history-atomic-legacy-integration-corrected` | All eight history integration cases pass, 3.389 seconds, unchanged sources. Legacy case physically creates version-one tables/profile/rows, migrates through actual Persistence open to schema 5, validates backup and reopens again. Independent read-only review found no material issue; current-schema corruption tests remain separate and validators were not relaxed |
| `history-atomic-validate-final` | Full tests, formatting, TypeScript, UI build and desktop compile pass, but strict all-target/all-feature Clippy rejects five issues: large Prepare enum payload, complex ticket tuple and three conditional-style lints. Exit 101, 220.626 seconds, unchanged sources. Corrections box only the seed, name the stored tuple and preserve short-circuit/error order in equivalent conditions; no warning suppression |
| `history-atomic-clippy-corrected` | Strict all-target/all-feature Clippy passes, 26.097 seconds, unchanged sources. Independent read-only review confirms equivalent evaluation order, errors, privacy checks and exact SQL; boxing changes allocation only |
| `history-atomic-browser-final` | Three actual browser cases pass, including the new active PostgreSQL clear/completion/reload flow and reviewed parameter privacy. Three fixture-backed cases fail during seeding because they reuse cleared immutable execution IDs. Exit 1, 43.823 seconds, unchanged sources; traces/screenshots are preserved. The test-only seed now allocates a fresh namespace and returns exact IDs, preserving production authority instead of resetting privacy state |
| `history-atomic-browser-corrected` | All six actual headless Chromium cases pass, 29.468 seconds, unchanged sources. Five history cases include exact admitted SQL before completion, clear during an actual PostgreSQL sleep, late completion and renderer reload; real SQLite paging, keyboard palette and delayed-detail invalidation. The sixth verifies actual PostgreSQL reviewed writes/NULL/injection-shaped bindings without persisting values. Fixture seeds use fresh IDs and preserve clear authority |
| `history-atomic-snowflake-producers` | Native synthetic-SDK suite: 66 pass / seven fail, exit 101, 264.397 seconds including compile, unchanged sources. Authentication-abandonment test times out waiting for synthetic auth entry. Two typed dropped-waiter tests stop on newly admitted history before ACK evidence; two recording-fault tests target old physical tables; two archive/lease fixtures require investigation against new publisher obligations. These failures require reproduction/correction; no package qualified from this run. All five new console lifecycle/privacy cases pass |
| `history-atomic-auth-reproduction` | Exact unchanged authentication-abandonment case passes, one pass / 1,127 filtered, 3.619 seconds including Cargo startup, unchanged sources. The earlier auth-entry timeout is not reproduced; no root cause or production fix is claimed. The complete corrected native suite must still pass |
| `history-atomic-native-waiter-fixtures` | Broad name filter runs 12 cases: eight pass / three fail / one intentional optimized fixture ignore, 78.147 seconds including compile, unchanged sources. Both corrected typed readiness cases pass. Two fault tests reach their original exact-count assertions but remaining raw audit readers still use the old table; corrected to audit_v5. The broad filter also exposes a qualification-runner post-ACK failure outside the original 73-case filter, requiring separate investigation. No account activity |
| `history-atomic-native-retention-fixtures` | All ten native retention cases pass, 47.690 seconds including compile, unchanged sources. The two corrected archive/lease cases require publisher sealing, reject sealed late observations, preserve legitimate pending observations through actual owner recovery and invalidate cached unowned detail. New terminal-restart, write-failure and panic lifecycle cases also pass; no account activity |
| `history-atomic-native-recording-fixtures` | Both native recording-fault cases pass, 7.669 seconds, unchanged sources. They inject actual local SQLite failures, drop the caller and retain the original helper DML/boundary acknowledgement through recording retries without replay. Synthetic SDK only |
| `history-atomic-runner-reproduction` | Exact offline qualification-runner case reproduces a production integration regression: phase-five ownership read rejects the closed event stream, exit 101, 35.289 seconds including compile, unchanged sources. Independent review identifies RunMetrics emitted before SessionState after moving history completion before secondary observation. The closed evidence validator correctly requires SessionState, RunMetrics, Finished. No account activity; validators and exact SQL-ledger assertions must remain intact |
| `history-atomic-event-order-red` / `history-atomic-event-order-red-exact` | Both commands exit zero but match zero cases because the nested test-module name was wrong. These are not passing or reproduction evidence. Cargo's test list establishes the actual qualified path before the next run |
| `history-atomic-event-order-red-qualified` | The augmented existing held-observation case fails on the exact event tail, one failed / 619 filtered, exit 101, 0.252 seconds, unchanged sources. History is already Recorded/Completed during the held secondary observation, but the public tail is RunMetrics, SessionState, Finished. The narrow correction moves only RunMetrics after the state block; completion persistence remains before the await |
| `history-atomic-event-order-green` | All five generic history-owner cases pass, 23.613 seconds including compile, unchanged sources. The augmented held-observation case preserves durable completion before the wait and restores SessionState → RunMetrics → Finished. Panic/callback/shutdown cases preserve original known completion. Independent read-only review finds no material concern in the narrow correction |
| `history-atomic-qualification-runners-reviewed` | All 27 offline runner cases pass / two intentional optimized full-runtime ignores, 466.942 seconds including compile, unchanged sources. The originally failing dropped-waiter case and full successful read event streams pass without relaxing evidence validation. Includes all-seven-mutation child exits, definitive rejections, observation/callback failures, dropped waiters, journal faults, bounded event capture, drift and cleanup. Synthetic SDK/isolated roots only; this is not account qualification |
| `history-atomic-prior-app-preservation` | The normal clean-adabd44 app is preserved with APFS copy-on-write before replacement. All 15,120 inventory entries match, strict signatures verify, and regular-file inventory is 242,395,383 bytes. Net free-space decrease during preservation is about 6.4 MiB; no prior app or source was deleted |
| `history-atomic-snowflake-producers-reviewed` | All 73 feature-specific native execution cases pass, 219.359 seconds including Cargo startup, unchanged sources. Includes corrected admission/ACK readiness, physical-schema recording faults, publisher sealing, privacy races, actual helper framing, cancellation, interrupted connections, unknown outcomes, retained sessions and restart lookup without replay. The earlier authentication-entry timeout is unreproduced in the exact rerun and this complete suite; no production authentication fix is claimed. Synthetic SDK only |
| `history-atomic-validate-reviewed` | Full pnpm validate passes: 1,038 frontend cases / 72 files, 676 standard Rust cases / 142 default fixture ignores, formatting, regenerated contracts, TypeScript, production frontend build, desktop compilation and strict all-target/all-feature Clippy. Exit zero, 191.211 seconds, all 559 captured source/test/tooling files unchanged. This includes all eight history integration and nine encrypted history privacy cases |
| `history-atomic-postgres-reviewed` | All ten actual isolated PostgreSQL script cases pass again on the final source, 2.738 seconds with compiled binaries reused, unchanged sources. Covers exact admission/reopen, active-query privacy changes, failed local completion recording, cancellation, connection loss, skipped COMMIT and rollback. No external database |
| `history-atomic-browser-reviewed` | All six actual headless Chromium cases pass on final source, 49.636 seconds including Rust bridge startup, all 559 captured sources unchanged. Five history cases use real SQLite and include an actual PostgreSQL active query, clear, late completion and renderer reload. The sixth verifies actual reviewed PostgreSQL writes/NULL/injection-shaped bindings without persisting values. Only the lead's local PostgreSQL container was stopped afterward; the unrelated user container remains untouched |
| `history-atomic-display-text-equivalence` | Final diff review restores the pre-feature user-facing history error instead of exposing the physical table suffix. Exact reverse-byte SHA verification proves this is the only difference among all 559 native-suite source hashes; error construction/code, validation and control flow are unchanged. Independent review agrees. Native suite results remain pinned to their original wording; full validation, PostgreSQL and browser checks rerun on the plain wording before packaging |
| `history-atomic-validate-display-reviewed` | Full pnpm validate passes again on the plain-wording source: 1,038 frontend / 676 standard Rust cases, 142 default ignores, formatting, contracts, TypeScript, UI build, desktop compile and strict Clippy. Exit zero, 194.945 seconds, all 559 captured sources unchanged |
| `history-atomic-postgres-display-reviewed` | All ten actual PostgreSQL cases pass again on the plain-wording source, 2.766 seconds, unchanged sources. The lead restarted only its own fixture for these final checks |
| `history-atomic-browser-display-reviewed` | All six actual Chromium cases pass again on the plain-wording source, 49.708 seconds, all 559 captured sources unchanged. Actual PostgreSQL active-query clear/reload and reviewed parameter writes remain verified; the lead's fixture is stopped after this run |
| `history-atomic-normal-package` | Normal SDK-bearing ARM64 app packages from clean `952758b039fba06ef9aaa9b284edb6acabe71c43` in 270.445 seconds, exit zero. All 559 captured sources remain unchanged and match final full validation, PostgreSQL and browser checks. Bundle resources, strict ad-hoc signature and guarded SDK 3.4.1 startup pass with zero authentication/SQL. Existing nonfatal LLVM stripping warning remains |
| `history-atomic-app-artifact` | Version 0.1.0 ARM64 executable: 34,663,152 bytes, SHA-256 `3926796c0a4a3175f9c3ac79deede76b2303842277073bf2ca330965cd9af445`. Original dependency-notice bytes match source/Git/bundle. All 15,120 preserved adabd44 inventory entries remain unchanged. Artifact receipt explicitly verifies the sole diagnostic-wording difference from native producer/runner sources by exact reverse-byte SHA; all other 558 hashes match. App is local/uninstalled; no account or owner UI acceptance |
| `history-atomic-doc-links` | All 778 repository-relative Markdown path targets exist; none missing. Fragment targets were not validated |

Review found that legacy v4 rows initially lacked ticket authority and could
evade a later clear during restore. Schema-5 migration now registers SQL-free
authority; the actual encrypted legacy/clear/restore regression passes in the
615-case standard run. Mutation-time validation of immutable row identity and
SQL digest also has passing regression coverage. The final integration reviewer
found no material defects in generic/typed execution and accepted the recovery
guard regression. The subsequently found public event-order regression is
corrected and independently reviewed. Final browser, feature-specific execution,
offline runner and full validation checks pass; the clean-source normal package
also passes within the explicit source scopes above.
The lead's graph transport was initially closed, with source review as fallback;
it is available again at generation `2026-11-13T03:56:26Z`. The obsolete private
history-import alias has one test-only caller, confirmed by search, inbound
trace, exact snippet, source search and coverage with no recorded gap; the alias
is removed instead of suppressing its unused warning.
The storage reviewer used generation `2026-11-12T18:30:58Z` with matching metadata
and no recorded gaps for six paths, plus exact source reads. This is not proof of
exhaustive graph coverage.
Final coverage records generation `2026-11-13T04:18:00Z`, metadata recorded at
`04:28:39Z`: all 34 changed source/test paths have matching metadata and no
recorded indexing issue. Exact current source remains authoritative; this
best-effort signal does not establish exhaustive graph coverage. The complete
response is retained as `history-atomic-final-coverage.json`.

Do not infer restart recovery for generic pending owners, durable reservations,
bounded lifetime ticket/era/generation growth or full-capacity readiness. Those
remain required work. Count gates remain and `retentionEnabled=false`. The
normal runnable app is now the local, uninstalled clean-`952758b` schema-5
package. The preceding `adabd44` schema-4 app is preserved under
`.cache/verification/reviewed-console/preserved-release-adabd44/Joinery.app`;
keep its metadata separate from schema-5 evaluation roots. The package is not
installed and normal owner UI/SSO/restart acceptance remains open. The existing
nonfatal Rust `rust-objcopy`/missing LLVM stripping warning persists; resource,
strict signature and guarded helper startup checks pass. No global toolchain
configuration was changed.

Storage after the artifact check: 206,189,969,408 free bytes (about 192 GiB),
9,372,112 KiB debug output (8.9 GiB) and 3,189,928 KiB release output (3.1 GiB).
The earlier approved debug-only cleanup reclaimed 145,766,936,576 bytes
(135.8 GiB); its receipt is
`.cache/verification/build-storage/approved-debug-cleanup.json`. This increment
performed no further cache deletion. Reduced debug settings, serialized checks
and the 20 GiB free-space preflight remain in effect; they are not automatic
cache reclamation. No build or test remains running.

Final independent documentation review matched seven updated guides against the
package/artifact receipts and found one stale current-bundle label in the usage
guide. It now labels `fb091db` and its native evidence as historical and points
to the current `952758b` package. No product source changed after packaging.
The canonical private project hub was not published or changed; repository
status and evidence are updated locally within the no-publication instruction.

### History completion evidence (13 November 2026)

Base is `718530e` on `feat/datagrip-replacement`. The lead coordinates three
bounded `gpt-6.1-sol` high-effort agents for the history model/SQLite contracts,
UI/browser coverage and integration tests/independent review. The
[workflow guide](query-history.md#completion-and-recovery-evidence) describes
the representation and its limits. Nullable history-only status/duration and
closed string provenance preserve original known evidence without guessing an
unknown outcome or duration. Shared validation covers writes, all history readers
and backup validation. Missing legacy provenance defaults to Recorded; explicit
null, unknown and unit-object encodings are rejected. Physical SQLite schema
remains 4. Normal terminal producers preserve their existing outcomes; the
console panic fallback no longer invents duration from wall-clock distance.

Production admission, restart and restore do not yet create new pending/recovered
history. Fixture coverage of these states is not proof of runtime crash recovery.
The atomic history/privacy, publisher settlement and reservation workflow remains
required. Keep both count gates and `retentionEnabled=false`.

Evidence labels refer to `.cache/verification/reviewed-console/`. Receipts record
exact commands, source hashes, elapsed time and before/after disk usage.

| Label | Actual result |
| --- | --- |
| `history-lifecycle-null-reproduced` | Expected exit 101 before implementation: the original non-nullable status/duration contract rejects admitted unknown evidence. 15.551 seconds, unchanged captured sources |
| `history-lifecycle-object-reproduced` | Expected exit 101 after independent review: derived enum deserialization accepts a unit-variant object instead of the closed string representation. 13.367 seconds, unchanged captured sources. Replaced by a string-only decoder |
| `history-lifecycle-integration-first` | Seven pass / one fails, 13.641 seconds, unchanged sources. The test expected `recorded` to match only provenance; existing generic search also matches “counts were not recorded.” Corrected the test oracle without changing search semantics |
| `history-lifecycle-focused-corrected` | All nine name-matching cases pass: four inline lifecycle and five matching integration cases; other integration names are filtered. 37.744 seconds, unchanged sources. All eight integration cases run in full validation below |
| `history-lifecycle-focused-ui` | 53 pass / one fails, 5.031 seconds. New test locator did not account for the existing accessible button name; corrected the locator |
| `history-lifecycle-ui-corrected` | 53 pass / one fails, 3.768 seconds. The replacement test used an unavailable jest-dom matcher; corrected it to the project's Vitest DOM-presence assertion. No production change |
| `history-lifecycle-ui-final` | All 54 focused cases in four files pass, 3.765 seconds, unchanged captured sources. Includes known/unknown status and duration, legacy rows, accessible history activation, metadata search and palette presentation |
| `history-lifecycle-full-validation` | `pnpm validate` passes: 1,038 frontend cases / 72 files, 653 standard Rust cases / 139 default fixture ignores, formatting, regenerated contracts, TypeScript, production UI build, desktop compilation and strict all-target/all-feature Clippy. 133.577 seconds, all 554 captured source files unchanged |
| `history-lifecycle-browser` | All five actual headless Chromium cases pass, 30.307 seconds, all 554 captured sources unchanged. Four history cases use real SQLite with 1,011 entries, including five older fixed lifecycle fixtures; cover exact reopening, reload, keyboard palette, clear during delayed detail, and unknown completion/duration without reconnect or execution. The fifth uses isolated PostgreSQL for reviewed writes/NULL/injection-shaped bindings and template-only history. Only the lead's PostgreSQL fixture was started/stopped; the unrelated running container was preserved |
| `history-lifecycle-snowflake-producers` | All 68 feature-specific native cases pass with `--features test-harness --lib execution::snowflake_ -- --test-threads=1`, 190.423 seconds, all 554 captured sources unchanged. Covers local synthetic SDK/helper and native execution paths, original ACK/history after DML/DDL, lost ACKs, shutdown, retries and publication panic with unavailable duration. No account connection or billable activity |
| `history-lifecycle-prior-app-preservation` | The clean-`99a3ca3` app is preserved with APFS copy-on-write; all 15,120 inventory entries match and both strict signatures verify. Regular-file inventory is 242,393,335 bytes. The first helper assumed the executable's case and stopped before copying; it now reads the executable name from the bundle plist |
| `history-lifecycle-normal-package` | Normal SDK-bearing ARM64 app packages from clean feature commit `adabd44` in 205.053 seconds. Resource verification, strict ad-hoc signature and guarded actual SDK 3.4.1 helper startup pass with zero authentication/SQL. All 554 source hashes equal full-validation/browser/feature-specific producer hashes and remain unchanged during packaging |
| `history-lifecycle-app-artifact` | Version 0.1.0 executable is 34,477,520 bytes, SHA-256 `5caf64e894f6767bfc69952e1e31f52a2f5a4c6365fa4d208369e76e4ce5f8dc`. Original dependency-notice bytes equal source, Git and bundle. All 15,120 entries of the preserved `99a3ca3` app remain unchanged. Complete inventory and source/artifact proofs are retained; the new app is local and uninstalled |
| `history-lifecycle-doc-links-final` | All 773 repository-relative Markdown path targets exist; no missing paths. Fragment targets were not validated |

The full Rust run includes all eight new integration cases, 15 inline history
cases and one backup-invariant case. Snowflake producer regressions, including
publication panic, pass in the separate `test-harness` feature run above.
The integration cases preserve 33 valid lifecycle combinations through actual
SQLite close/reopen, three-row paging, detail, compatibility/full-SQL search and
an exact copied test database. This copied-database check does not claim product
metadata restore or owner crash recovery. Invalid typed writes, inconsistent raw
rows and all four unit-object provenance encodings are rejected. Original known
status with unavailable duration remains valid; measured zero stays zero.

Independent review found the enum object-encoding issue above. Final review has
no material findings within the frozen model/persistence/producer and UI scopes.
The final decoder hash is
`ca4f94d442be0da71d99ccdca44d15c079757cc8717fa33b0b12e3c764e21d49`.
The lead's graph transport was closed; reviewers used Tier 2 generations
`2026-11-12T17:57:04Z` and `17:59:22Z`, matching metadata and coverage checks
plus direct source reads. No call-path conclusion relies on unreliable graph
edges. Subsequent UI changes corrected test-only locators/matchers.

The approved debug-cache deletion was already completed and reclaimed 135.8 GiB;
its receipt is `.cache/verification/build-storage/approved-debug-cleanup.json`.
Current checks run serially under the 20 GiB free-space preflight. After full
validation, browser/producer checks and the one normal package, about 189.6 GiB
remains free; debug output is about 8.9 GiB, release output 3.0 GiB. No additional
cache deletion is authorized or performed. The preceding clean-`99a3ca3` normal
app was preserved before this package build. Normal packaged owner UI/SSO/restart, actual account effects,
installation/update and Developer ID/notarization remain open. No account activity,
installation, push or publication occurred; the private project hub is unchanged
because publication is not authorized. The runnable local app is
`src-tauri/target/release/bundle/macos/Joinery.app`; its predecessor is at
`.cache/verification/reviewed-console/preserved-release-99a3ca3/Joinery.app`.
The existing nonfatal `rust-objcopy`/LLVM loading warning recurs during release
builds. No global toolchain/configuration change was made; packaging exits zero
and all resource/signature/startup checks pass.

### Paged query history (13 November 2026)

Base is `2b4ee70` on `feat/datagrip-replacement`. The lead coordinates three
bounded `gpt-6.1-sol` high-effort agents for SQLite/API implementation,
history/palette UI and independent read-only review. The
[workflow guide](query-history.md) records limits and reproducible acceptance.
History and palette readers now request summary pages, search the retained store
before limiting matches, and fetch exact current SQL only on explicit activation.
Shared clear invalidation fences both surfaces and pending detail opens. Stored
empty SQL is distinct from metadata-only history. No schema migration or account
execution is included.

Evidence labels below refer to `.cache/verification/reviewed-console/`. Receipts
capture exact commands, complete source hashes and before/after disk usage.

| Label | Actual result |
| --- | --- |
| `history-paging-older-match-reproduced` | Expected exit 101 before the production fix: older match omitted after the newest 1,000 rows; zero matches instead of one. 17.837 seconds, unchanged captured sources |
| `history-paging-focused-rust` | Ten pass / one fails: SQLite empty-BLOB substring returns NULL, losing the empty saved preview. 17.029 seconds, unchanged captured sources |
| `history-paging-focused-ui` | 37 pass / one fails: a test-fixture edit incorrectly renamed saved-query/session SQL fields. Production contracts retained; fixture corrected. 3.699 seconds, unchanged captured sources |
| `history-paging-focused-corrected` | All 11 real SQLite cases pass, 16.407 seconds, unchanged captured sources. Includes full-retention/compatibility-cap search, ties, Unicode and embedded-NUL previews, empty/unavailable/exact SQL, reopen/clear, metadata terms, malformed cursors and corrupt identities |
| `history-paging-ui-corrected` | All 38 focused cases in three files pass, 3.545 seconds, unchanged captured sources. Includes page replacement, deep server matches, exact detail, retry/error and search/refresh/unmount/shared-clear races |
| `history-paging-full-first` | `pnpm validate` passes: 1,022 frontend cases / 71 files, 640 standard Rust cases / 139 default fixture ignores, formatting, regenerated contracts, TypeScript, production frontend, desktop compilation and strict all-target/all-feature Clippy. 140.019 seconds, unchanged captured sources |
| `history-paging-browser` | All four actual headless Chromium cases pass, 27.191 seconds including Rust bridge startup, unchanged captured sources. Three new real-SQLite history cases cover 1,006 tied-time entries, old/deep Unicode search, bounded summary payload, exact full SQL opening without execution, renderer reload, keyboard palette, empty/metadata-only SQL and clear during an actually delayed detail response. The fourth uses isolated PostgreSQL for reviewed writes/NULL/injection-shaped bindings and verifies only templates enter history. The lead started/stopped only its PostgreSQL fixture; the unrelated running container was preserved |
| `history-paging-native-build` | Hidden native release build passes, 200.490 seconds, unchanged captured sources. The existing nonfatal `rust-objcopy`/LLVM loading warning also occurs in the preceding normal-package log; no toolchain/global configuration was changed |
| `history-paging-native-webview` | Actual hidden main WebView passes all 21 checks including new history-page and missing-detail IPC permissions, plus ten cleanup cycles; exit zero, 3.568-second runner / 3,109-ms native elapsed, unchanged captured sources. Exact build and run metadata are preserved as `history-paging-native-build-metadata.json` / `history-paging-native-webview-runs.json` |
| `history-paging-final-metadata` | All 12 real SQLite cases pass after the review correction; 17.706 seconds, unchanged captured sources. New metadata-only positive/negative fixtures check both summary pages and compatibility search, validated UPDATE multi-joined counts and unavailable DML/DDL summary status |
| `history-paging-full-final` | Corrected-source `pnpm validate` passes: 1,022 frontend cases / 71 files, 641 standard Rust cases / 139 default fixture ignores, formatting, contracts, TypeScript, production UI, desktop compile and strict Clippy. 136.747 seconds, all 551 captured product/test/tooling files unchanged during the check |
| `history-paging-browser-final` | All four browser cases pass again on the corrected metadata-search source, 27.102 seconds, all 551 captured source files unchanged. Exact full-SQL reopening, delayed-response clear protection and actual PostgreSQL template-only history remain verified; the lead's PostgreSQL fixture is stopped afterward |
| `history-paging-prior-app-preservation` | Previous clean-`f9c1ce2` app preserved with copy-on-write before replacement. All 15,120 inventory entries match; strict signature verifies. The complete preserved inventory remains unchanged after packaging |
| `history-paging-normal-package` | Normal SDK-bearing ARM64 app packages from clean feature commit `99a3ca3` in 210.164 seconds. Resource verification, strict ad-hoc signing and actual guarded helper/SDK 3.4.1 startup pass with zero authentication/SQL requests. All 551 source hashes equal final full-validation/browser hashes and remain unchanged during packaging |
| `history-paging-app-artifact` | Current version 0.1.0 executable is 34,475,472 bytes, SHA-256 `dfcc94bc98508459f3a37c6e6f7ccd3315e2001bc689c1d3d12c2b30f1a57506`. Original bundled dependency-notice bytes equal source and Git. Full inventory, preserved predecessor and source/hash proofs are recorded. About 188.2 GiB free, debug output 8.9 GiB / release output 3.0 GiB |

The first browser run and hidden native run precede the review correction for two
displayed Snowflake summary search fields. Enrollment, UI and detail/paging logic remain
unchanged; final source checks separately qualify the corrected search terms.
Independent review identified omitted multi-joined-row counts and unavailable
summary status. The correction indexes validated counts only, preserving the
displayed distinction between unavailable and verified summaries.
Final independent review has no material findings. The reviewer reconstructed
the preceding reviewed `history.rs` hash exactly by reversing only the narrow
search blocks, one new regression and comment-spacing changes. Final SHA-256 is
`5de03aa428148ace3ccca7f3938a7e44ccd05e677b0be22a4177c90149dd9e52`.
The lead's graph transport was closed. Review used Tier 2 generation
`2026-11-12T17:28:35Z`, then `17:39:52Z` for the final history/display paths,
with matching metadata and no recorded coverage gaps plus direct source reads.
Unreliable graph edges were not used as call-path evidence.

The runnable local app is
`src-tauri/target/release/bundle/macos/Joinery.app`; the preceding app is preserved
at `.cache/verification/reviewed-console/preserved-release-f9c1ce2/Joinery.app`.
Normal packaged owner UI/SSO/restart acceptance, installation/update, Developer
ID/notarization and actual account qualification remain open. No account activity,
installation, push or publication occurred. The private project hub remains
unchanged because publication is not authorized.

Search streams one SQL row at a time under the shared SQLite mutex; the sorter
holds keys. This is not a bound on arbitrary legacy row bytes or proof of
full-capacity latency/memory. The compatibility API retains its 1,000 matching
full-entry cap, while current UI readers use summaries. Atomic pending/completed
history, durable reservations, publisher settlement, configurable retention,
actual Snowflake and owner acceptance remain required. `retentionEnabled=false`.

### Checked receipt accounting (13 November 2026, local source verified)

Base is `257bb7e` on `feat/datagrip-replacement`. The lead integrated an
independent checked fold over two integer columns per stored journal row.
A `gpt-6.1-sol` high-effort worker owned only the accounting module/tests; a
separate high-effort agent reviewed the frozen four-file change. No material
finding remains within that scope. The lead's graph transport was closed;
the reviewer's generation `2026-11-12T17:03:31Z` reported matching metadata and
no recorded issues for five relied paths. Material conclusions use direct source.

The existing charging formulas and exact schema-4 trigger definitions are
unchanged. The independent check derives a conservative current-journal ceiling
of 3,335,896 logical bytes for a run with all 100 maximal bounded units, and 456
bytes for a refusal using the reader's 64-byte code bound. These bounds exclude
future durable history/audit/settlement storage and do not reserve filesystem
space. Eight arithmetic/SQLite tests cover UTF-8/escaping, nullable fields,
all-100-unit bounds, atomic overflow rejection and malformed aggregates. Two
additional real SQLite cases cover growth and shrinkage, late terminal delivery
observations, archive, reopen, v3 migration, exact originals and disjoint restore.

Evidence labels refer to `.cache/verification/reviewed-console/`; JSON receipts
record complete commands, source hashes and before/after disk usage:

| Label | Actual result |
| --- | --- |
| `receipt-accounting-focused-first` | All 24 matching tests pass, including the eight new arithmetic/SQLite and two lifecycle cases; 20.097-second wrapper / 0.17-second tests. Captured source unchanged |
| `receipt-accounting-full-validation` | `pnpm validate` passes: 997 frontend cases / 70 files, 629 standard Rust cases / 139 default fixture ignores, formatting, contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 149.277 seconds; all 547 source pins unchanged |
| `receipt-accounting-count-boundary-profile` | Existing 10,000-positive small-prepared-record capacity/restore-union test passes in the source-pinned debug test binary: 1.27 seconds, 28,442,624-byte maximum RSS, zero swaps; 1.280-second wrapper |
| `receipt-accounting-refusal-boundary-profile` | Existing 10,000-refusal dedup/independent-positive-capacity/atomic-restore-union test passes: 0.82 seconds, 24,231,936-byte maximum RSS, zero swaps; 0.824-second wrapper |

The profiles measure complete test processes, including fixture construction;
they do not isolate the new fold or compare it with the previous SQLite `SUM`.
They do not establish all-100-unit/max-payload storage, an 8 GiB admission policy,
full-capacity recovery or a memory plateau. The new row fold adds per-row FFI
work; full-capacity performance remains an acceptance requirement. Unchanged
ts-rs attribute/future proc-macro compatibility warnings remain visible. External
database fixture cases were not requalified in this accounting-only increment.

No new normal package was built. The runnable local encrypted-recovery app from
`f9c1ce2` retains executable SHA-256
`23f100e40a553d9e18c1f62c530d64aa63ca76f4082a0cd8b75188c21238f2b5`.
No changed UI flow, account activity, installation or publication occurred. Both
10,000-count gates, unavailable reservation/budget DTO fields and
`retentionEnabled=false` remain. Continue durable lifecycle/reservation integration,
full-capacity verification, all applicable DataGrip capabilities and owner acceptance.

### Encrypted metadata recovery (12 November 2026, local workflow verified)

Base is `433fbac` on `feat/datagrip-replacement`. The new
[workflow and acceptance steps](encrypted-metadata-recovery.md) pass local browser
and hidden native acceptance. Feature commit `f9c1ce2` also passes normal
SDK-bearing ARM64 packaging from a clean checkout. The earlier `72ee3e4` app is
preserved separately with an unchanged complete inventory. No account activity,
installation or
publication occurred. Three bounded workers used `gpt-6.1-sol` at high effort for
crypto/publishing and review, SQLite streaming recovery, and UI/workspace plus
browser acceptance source. The UI worker additionally owns the isolated hidden
macOS encrypted recovery runner and actual main-WebView probes. The lead owns
integration, serialized builds and final checks. Cargo
builds are serialized and gated on at least 20 GiB free space. The fixture
Compose project is `joinery-recovery-1794497502`; its six initially healthy
containers are independent of the pre-existing logistics database. Fixture
startup/health and ports are retained in `encrypted-recovery-fixtures*.json`.

Evidence labels below refer to `.cache/verification/reviewed-console/`:

| Label | Actual result |
| --- | --- |
| `encrypted-recovery-ui-first` | 26 cases in three files pass; 9.098 seconds including wrapper, unchanged captured frontend hashes |
| `encrypted-recovery-ui-second` | 27 cases in three files pass after unknown-outcome tombstone coverage; 4.341 seconds including wrapper, unchanged captured frontend hashes; precedes the later missing-ID global-fence change |
| `encrypted-recovery-first-compile` | Exit 101 before tests: four unsupported SQLite `u64` conversions; 49.224 seconds, unchanged captured product hashes |
| `encrypted-recovery-second-compile` | Compiles; 51 pass / 19 fail / 3 ignored / 998 filtered; 66.393 seconds total, unchanged captured product hashes. Most failures expose a 21-byte buffer for the 22-byte age prefix; separate staging fixture failure remains under correction |
| `encrypted-recovery-stream-corrected` | 20 pass / one fail / one internal child ignored; 72.959 seconds total. Header defects corrected. Crash test asserted staging existence after inventory had already cleaned it |
| `encrypted-recovery-stream-final` | 21 pass / one internal child ignored / 1051 filtered; 59.835 seconds total, unchanged captured native hashes. Real SQLite restore/undo, cancellation, concurrent-writer fencing, post-link sync failure, killed replacement and >64 MiB staging pass |
| `encrypted-recovery-ui-fenced` | 28 pass / eight fail; new authority read made old immediate-click tests premature and two preference mocks incomplete. 8.608 seconds, unchanged captured frontend hashes |
| `encrypted-recovery-ui-authority-corrected` | 36 cases in three files pass; 3.539 seconds total, unchanged captured frontend hashes. Tests wait for actual native-authority readiness; production checks remain enforced |
| `encrypted-recovery-integrated-native` | 72 pass / three ignored / 998 filtered; 79.569 seconds total, unchanged captured product/test hashes. Includes retained jobs, two delayed starts rejected after unknown-ID fencing and restart reconciliation |
| `encrypted-recovery-types` | TypeScript fails on three fixtures using null progress totals; generated contract incorrectly declared only number. Native nullable value is correct; fix the generation annotation before full validation |
| `encrypted-recovery-production-kdf` | The already-built, source-pinned native test binary passes the real work-factor-18 encrypt/decrypt round trip: one pass, 1072 filtered, 35.03-second test / 35.146-second wrapper. `/usr/bin/time -l` measures 278,216,704-byte (265.3 MiB) maximum RSS, zero swaps. Small fixture in the debug test binary; not full restore or large-capacity performance |
| `encrypted-recovery-full-first` | Full validation reaches 976 frontend passes / 67 files and 618 standard Rust passes (139 default fixture ignores), then fails desktop compilation because a cheap async `State` command returns bare `bool`. 180.127 seconds, unchanged captured product/test hashes. Getter changed to synchronous; final validation still required |
| `encrypted-recovery-acl-regression-reproduced` | Both new cross-file enrollment tests fail in 1.115 seconds, unchanged captured product/test hashes: 14 normal handlers absent from app manifest, 15 absent from main capability, four retired backup entries still enrolled |
| `encrypted-recovery-acl-regression-corrected` | Both enrollment tests pass in 1.187 seconds, unchanged captured product/test hashes after aligning handler/manifest/main permissions, removing retired entries and preserving separate test-only permissions. Actual native WebView verification remains required |
| `encrypted-recovery-database-fixtures` | All 124 ignored database/SSH/transport cases pass across eight binaries; 83.876 seconds, unchanged captured native hashes. Real isolated PostgreSQL 18, MySQL 8.4, MariaDB 11.4 (including folded-name fixtures) and Redis 8.2; covers SQL execution, TLS/SSH, metadata, writes/DDL, cancellations, interrupted transports and transaction outcomes |
| `encrypted-recovery-native-clippy` | Strict all-target/all-feature Clippy fails on four needless borrowed references and one platform-specific redundant filesystem-size cast; 37.770 seconds, unchanged captured native hashes. References corrected; filesystem available-byte multiplication now widens both fields to `u128` and checks conversion back to `u64`, preserving overflow rejection |
| `encrypted-recovery-native-clippy-corrected` | Strict Clippy reaches test targets and rejects the new jobs test module preceding production items; 30.876 seconds, unchanged captured native hashes. Test module moved to the end without changing its cases; no lint suppression |
| `encrypted-recovery-native-clippy-final` | Strict all-target/all-feature Clippy passes in 33.809 seconds, unchanged captured native hashes; informational proc-macro future-compatibility warning remains documented |
| `encrypted-recovery-runner-tests` | All 11 new hidden-native runner acceptance tests pass, 0.561-second wrapper, unchanged captured product/test hashes. They validate report rejection, process/deadline checks, ciphertext/hash/source evidence; actual native execution is separate |
| `encrypted-recovery-full-corrected` | Final `pnpm validate` passes: 978 frontend cases / 68 files, 619 standard Rust cases and 139 default fixture ignores; formatting, regenerated contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 181.469 seconds, unchanged captured product/test hashes |
| `encrypted-recovery-browser-first` | Actual headless Chromium → Rust Studio/SQLite/encryption workflow passes in 118.210 seconds including server compilation/startup (about 1.5 minutes for the case), unchanged captured product/test hashes. Real create/export/import, exact ciphertext hash/size, source preservation, wrong-key no mutation, restore/undo and page reload; one admitted response deliberately lost, exactly six distinct original starts and same-ID lookup without replay |
| `encrypted-recovery-browser-regressions` | Remaining 162 existing browser cases: 152 pass / 10 fail, 543.642 seconds, unchanged captured product/test hashes. Failures include obsolete transaction-warning wording, ambiguous global status queries, changed Snowflake form expectations and a retained-console profile-save failure under investigation. Original 30 trace/context/image files are preserved separately with hashes; no blanket browser pass is claimed |
| `encrypted-recovery-startup-hold-reproduced` | New startup regression fails before the fix because the required recovery gate is absent; 2.892 seconds, unchanged captured product/test hashes |
| `encrypted-recovery-startup-hold-frozen` | 58 cases across seven App/workspace/close/recovery/Preferences files pass in 4.007 seconds, unchanged captured frontend hashes. Startup gating, initial-bootstrap ordering, failed reconciliation, malformed/unreadable storage and strict durable close are covered; actual browser/native execution remains separate |
| `encrypted-recovery-retained-isolated` | Retained-console workflow passes in isolation: 19.1-second case / 26.373-second wrapper, unchanged captured product/test/tooling hashes. No production change, retry or timeout extension. The earlier full-suite helper startup failure remains unexplained; an isolated pass does not prove it fixed |
| `encrypted-recovery-startup-pause-reproduced` | Transient storage failure leaves the writer paused after the marker becomes readable/absent; new regression fails before the predicate fix. 2.934 seconds, unchanged captured frontend hashes |
| `encrypted-recovery-startup-pause-escape-final` | 60 cases / seven files pass in 3.887 seconds, unchanged captured frontend hashes. An already-paused startup still gates editing; an absent original marker uses the existing unknown-ID global fence and fresh reload, with no fabricated UUID |
| `encrypted-recovery-final-validation` | Exit 1 after 32.614 seconds, unchanged captured product/test/tooling hashes: TypeScript rejects the new App regression's overly broad `vi.fn` mock type as `initialize: () => Promise<void>`. Stops before full test execution; requires a precise test-only mock annotation and rerun |
| `encrypted-recovery-startup-mock-typecheck`, `encrypted-recovery-startup-mock-focused` | Precise test-only async mock annotation: TypeScript passes in 5.723 seconds; seven App recovery cases pass in 2.201 seconds, unchanged captured frontend hashes. Production hashes remain exactly as independently reviewed |
| `encrypted-recovery-final-corrected-validation` | Corrected-source `pnpm validate` passes: 989 frontend cases / 69 files, 619 standard Rust cases / 139 default fixture ignores; formatting, contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 169.352 seconds, unchanged captured product/test/tooling hashes |
| `encrypted-recovery-browser-final` | Six affected files: 29 pass / one new startup case fails; 274.913 seconds, unchanged captured product/test/tooling hashes. Production-strength crypto workflow passes again (1.7 minutes); all nine corrected stale cases and the retained-console workflow (18.3 seconds) pass. The new test holds ordinal bootstrap 2, which is actually a second initial request under development StrictMode; the gate correctly waits rather than offering reconciliation. Four failure artifacts are retained with hashes. Initial-load duplication and test hold selection require correction before final runtime qualification |
| `encrypted-recovery-startup-strictmode-reproduced` | Actual StrictMode component regression fails: expected one initial metadata load, observed two; 2.015 seconds, unchanged captured frontend hashes |
| `encrypted-recovery-startup-strictmode-focused`, `encrypted-recovery-startup-strictmode-typecheck` | Shared initial promise across effect replay: 61 cases / seven files pass in 5.938 seconds; TypeScript passes in 8.825 seconds, unchanged captured frontend hashes. Bounded independent review verifies the exact App/test/E2E hashes in receipt `915259b22f5a6a3759996f30dd076ed45d6c4f0e41df1942714a74f38d8dc2bf` |
| `encrypted-recovery-startup-browser-corrected` | Actual startup gate → same-ID native reconciliation → held fresh bootstrap → editing enabled passes, 1.5-second case / 7.875-second wrapper, unchanged captured product/test/tooling hashes. No connect, execution, export or recovery start. The hold is tied to observed reconciliation, not request ordinal |
| `encrypted-recovery-strictmode-full-validation` | Final shared-initialization source passes `pnpm validate`: 990 frontend cases / 69 files, 619 standard Rust cases / 139 default fixture ignores, formatting, contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 190.114 seconds, unchanged captured product/test/tooling hashes |
| `encrypted-recovery-native-smoke-build` | SDK-bearing hidden macOS build passes in 348.998 seconds, unchanged captured product/test/tooling hashes. Actual resource verification and network-guarded helper startup pass with SDK 3.4.1 and zero authentication/SQL requests. Rust reports a non-fatal debug-strip warning because this pinned toolchain's `rust-objcopy` cannot load `libLLVM.dylib`; no global toolchain changes or warning suppression. Native workflow execution remains separate |
| `encrypted-recovery-native-actual` | Fails in 6.461 seconds, unchanged captured product/test/tooling hashes: all 22 native setup checks pass, but fresh-process reopen rejects its serialized inventory comparison. Full result preserved as `encrypted-recovery-native-first-runs.json`. Independent ciphertext-only recheck passes exact IDs, sizes and SHA-256 for both retained files; this does not establish the full metadata/restart contract. Comparison/serialization is under investigation; no native pass claimed |
| `encrypted-recovery-native-inventory-reproduced` | One regression fails / six pass in 1.567 seconds, unchanged captured product/test/tooling hashes. The exact old stringify comparator rejects equal full inventories after `serde_json::Value` sorts object keys. Per-field/ordered-inventory mutations remain rejected. This reproduces an acceptance-oracle defect; native rerun remains required after correction |
| `encrypted-recovery-native-inventory-corrected` | All seven regressions pass in 1.239 seconds, unchanged captured product/test/tooling hashes. Comparator preserves format, ID, creation time, size, digest, inventory length and array order while ignoring property insertion order. Fresh returned inventory is forwarded to independent runner ciphertext checks; native rebuild/rerun remains separate |
| `encrypted-recovery-final-oracle-validation` | Final oracle-corrected source passes `pnpm validate`: 997 frontend cases / 70 files, 619 standard Rust cases / 139 default fixture ignores, formatting, contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 173.030 seconds, unchanged captured product/test/tooling hashes |
| `encrypted-recovery-native-rebuilt` | Corrected hidden SDK-bearing macOS build passes in 264.524 seconds, unchanged captured product/test/tooling hashes; actual SDK 3.4.1 resource and guarded helper-startup checks pass with zero authentication/SQL. Existing non-fatal toolchain warnings remain |
| `encrypted-recovery-native-corrected` | Actual encrypted native acceptance passes in 4.990 seconds, unchanged captured product/test/tooling hashes. All 22 setup and five fresh-process reopen checks pass; both child exits are clean. Real main ACL probes, Preferences create/wrong-key/restore/undo, exact two retained ciphertext identities and production single-scrypt N18 headers pass. Selected native connection/SQL counts remain zero. Full receipt is preserved as `encrypted-recovery-native-corrected-runs.json` |
| `encrypted-recovery-native-postgres-restart` | Both real PostgreSQL/native close/reopen pairs pass in 2.974 seconds, unchanged captured product/test/tooling hashes. Actual pending-draft close exits in 56.34 and 54.80 ms, below the unchanged 400 ms debounce window. Close-cancel retains session/transaction; restart restores latest draft/preferences but no sessions/results/transactions or SQL replay; observer verifies rollback and drops only each generated test table. Four clean process exits. Full receipt is preserved as `encrypted-recovery-native-postgres-restart-runs.json` |
| `encrypted-recovery-normal-package` | Normal SDK-bearing ARM64 package from clean `f9c1ce2` passes in 204.870 seconds, unchanged captured product/test/tooling hashes. Strict ad-hoc signature/resource checks and guarded SDK 3.4.1 helper startup pass with zero authentication/SQL requests. Build reports 226.86 MiB. Existing toolchain warnings remain non-fatal; Developer ID/notarization and installed/owner UI acceptance remain open |
| `encrypted-recovery-final-package-attestation` | Actual package is version 0.1.0, ARM64; executable 34,355,344 bytes, SHA-256 `23f100e40a553d9e18c1f62c530d64aa63ca76f4082a0cd8b75188c21238f2b5`. Source/index/bundled original license bytes match exactly. All 546 validated source pins still match; all 13,055 files in preserved `72ee3e4` match its original inventory. Local and uninstalled |

Review found and corrected early export cancellation cutoff, cancellation
discarding a published artifact identity, and failed directory-sync paths losing
known publication identity. Unknown status after restart must remain unknown,
not a fabricated cancellation or operation kind. A malformed browser marker also
needs a native fence for all later starts rather than a new unrelated UUID. These
corrections pass the focused checks above; full validation and final bounded independent
rereview now pass as recorded here; runtime workflow gates remain. The generated nullable-total annotation is corrected
and contracts regenerated before full validation. Independent review then found
that the HTTP harness bypasses Tauri ACL enrollment: all eight recovery commands,
six receipt inventory/release commands and CSV-preview release were unavailable
through the actual main WebView. Exact pinned Tauri source confirms denial before
handler dispatch. Manifest and main capability are now aligned, with retired
backup entries removed. New source-level enrollment regressions fail before the
fix and pass after it. The subsequent actual main-WebView probes above also
pass; source enrollment and HTTP harness checks alone did not establish that.

The added dependency check has 48 package versions: 46 without returned OSV
findings and two informational unmaintained compile-time macro dependencies.
The guide records exact source pins and advisories. This is neither a whole-app
security audit nor an advisory-free claim. Exact added-dependency original licenses, copyright notices and embedded
data notices are collected in `src-tauri/resources/recovery-dependency-notices.txt`
(130,911 bytes; SHA-256
`00504cf7c37bff567ef2c4711975538d8c5efd9adab672740d589b522cc54fbb`).
The Tauri resource map includes this file; actual assembled bundle bytes match
source and Git exactly in the final attestation. Source provenance for the 48
packages is retained in
`recovery-dependency-notices-sources.json` under the evidence directory. Full-capacity/resource and normal native
workflow acceptance remain open; lifetime receipt gates stay unchanged.

Initial staging exposed automatic CRLF conversion and upstream trailing-space
warnings in the original legal texts. A file-specific `.gitattributes` entry
preserves this notice file's bytes and spacing; source whitespace checks remain
unchanged. Restaging verifies exact working/index equality at the 130,911-byte
SHA-256 above, and the staged diff check passes. No global Git settings changed.

Final bounded review rechecked all 121 typed commands/handlers/manifest entries,
118 main app grants and three isolated smoke grants. It found no remaining
material issue after corrections in admission fencing, UI remount state, nullable
progress, publication callbacks and SQLite writer witnesses. The added native
runner source was reviewed separately: production work-factor-18 path, exact
error probes, distinct original IDs, ciphertext hashes, real child-process exit
and restart, unique roots and own-child-only timeout termination. The reviewer
ran no checks. Graph coverage reported matching/no recorded issues where current;
changed build/capability files were checked directly. Root graph transport remains
unavailable. These bounded results do not prove graph or whole-app completeness.
Native "no SQL" evidence uses selected command counters and empty sessions/results,
not universal network tracing. Native picker dialogs, installed app, keychain,
actual accounts and visible user interaction remain separate qualification.

The native inventory-oracle correction was separately reviewed against exact
source hashes `798f2d4d42baad250b85a7bdb5509b296425cd0b8ba3deb7aa39eb5d93c066bc`
and `4036af59b197c379f6be8db823faf434d5e4c4e6a16ae2c5edb4cc9519d5af6f`.
It preserves every current backup field and ordered inventory, forwards the
fresh reopened inventory to independent physical-file/header/hash checks and
changes no deadlines, retry behavior or selected no-SQL counters. No material
finding remains in this bounded delta; actual execution is recorded separately.
Final independent receipt review confirms distinct native setup/reopen PIDs,
22 + 5 true named checks, four original operation IDs, two exact physical
ciphertext identities and clean process exits. All 546 captured source hashes
still match across validation, rebuilt binary and native acceptance. The reviewer
executed no tests. After both PostgreSQL/native close/reopen pairs passed, only
the six `joinery-recovery-1794497502` fixture containers were stopped, preserving
their volumes. The pre-existing logistics database remains running.

A subsequent integration review found a material startup/close gap not covered
by the preceding passes: a remembered recovery operation pauses workspace saves,
but the ordinary close path can treat a paused no-op flush as durable success.
Recovery is mounted only after Preferences opens; a new draft entered before
reconciliation could also be discarded by metadata reload. Both this path and a
transient-storage/absent-marker variant were reproduced and corrected. Startup
editing now remains blocked until native fencing and fresh metadata reload;
strict close refuses a paused save. The final 60 focused cases pass, and an
independent reviewer checked the exact three production hashes in receipt
`encrypted-recovery-startup-pause-implementation.json` (SHA-256
`b933f1e3b30e7ed236a8d19a3d496a73c913caccf0f849b346e513317a661420`).
No further material issue was found in those bounded paths. Fresh graph coverage
reported matching metadata/no recorded issue for eight reviewed evidence paths,
with direct source confirmation; this does not establish graph completeness.
Browser/native and final validation of this corrected source remain separate.

History confirms nine full-browser failures predate base `433fbac`: transaction
warning text changed in `b204fed`, statement summaries added another status in
`278754b`, and opted-in SDK write controls changed the form in `c3bada6`. Tests now
select the intended commit notification, retain independent database readback,
check both transaction warnings and verify default-off SDK consents plus forced
read-only OAuth. The retained-console test retains its full workflow and now
asserts the actual connection response early, attaching only its closed error
code and bounded synthetic-child diagnostics on failure. The original helper
startup failure remains unexplained despite the isolated pass.

The five existing browser-spec corrections are separately committed as
`874821e` (`test(e2e): update current workflow assertions`). Bounded independent
review found no material weakening or diagnostic disclosure: real database
readbacks and resource checks remain, and failure attachments are restricted to
the owned synthetic child's bounded output plus a closed connection outcome.
This is provenance for this fixture, not general-purpose log redaction or a
diagnosis of the original helper failure.

Storage follow-up at 15:57 UTC: debug artifacts **9,493,088 KiB** (about 9.1 GiB),
release **2,501,696 KiB**, free **204,499,623,936 bytes** (about 190.5 GiB).
`build-storage/encrypted-recovery-followup-space.json` retains that measurement.
The earlier approved cleanup reclaimed 135.8 GiB; its delayed approval has not
been reused and no additional artifacts were deleted. Incremental caching stays
disabled. Before rebuilding the normal app, the prior `72ee3e4` bundle was
copy-on-write preserved with matching complete file inventories and strict
signature verification (`encrypted-recovery-preserved-release.json`).

Final package/storage attestation at 16:52 UTC records **203,807,354,880 bytes
free (about 189.8 GiB)**, debug **9,234,044 KiB (about 8.8 GiB)** and release
**3,192,248 KiB (about 3.0 GiB)**. The app is
`src-tauri/target/release/bundle/macos/Joinery.app`. No further cache cleanup,
installation, account activity or publication occurred. The private project hub
is unchanged because publication is not authorized. The complete DataGrip
replacement Goal, full-capacity recovery, normal picker/installed/keychain
behavior and external/owner acceptance remain open. The original broader-suite
helper-startup failure is still unexplained; subsequent scoped passes do not
erase that observation.

### Paged receipt inventory and hot-state compaction (12 November 2026)

Base is `cfdcb23` on the existing `feat/datagrip-replacement` branch. The
[retention design](snowflake-console-retention-design.md) distinguishes this
implemented slice from the full encrypted/capacity target. No account activity,
private application-state access, installation or publication occurred. Existing
10,000-positive / 10,000-refusal gates remain; `retentionEnabled=false`. Metadata
backup is still plaintext and limited to 64 MiB / 20 copies. No full-capacity,
RSS plateau or encrypted recovery result is claimed.

Three bounded workers used `gpt-6.1-sol` with high effort for persistence/migration,
native ownership, and recovery/scheduler plus independent review. The lead owned
contracts, command integration, UI/store, end-to-end checks and documentation.
Workers shared the checkout, preserved others' edits and ran no concurrent builds.
Root graph transport was unavailable; direct source reads were used. Worker
coverage reports for relied paths were metadata-matching with no recorded issues,
which is not proof of graph completeness.

Implemented source behavior:

- Schema 4 atomically migrates exact legacy schemas under a raw exclusive OS
  lease, physically excludes old cached writers, and validates every original
  decision. SQL-free summaries use decimal strings for 64-bit counters/revisions,
  epoch-bound keyset pages, immutable identity and separate browsing revisions.
- Archive uses exact CAS and a raw temporary owner. It cannot recover an unrelated
  abandoned run. Restore preserves the exact evidence union with a fresh ordering
  epoch. Logical charged bytes are derived/validated; unavailable reservations,
  byte budget and free journal budget are explicit nulls.
- Native owners separate active/publisher/dirty state and result ownership from
  bounded settled detail caching. Browser details and settled refusals share a
  16-entry / 16 MiB LRU independently of storage success. Unhanded identities/proof
  remain preserved. Background reconciliation uses four shared slots/backoff;
  manual history uses 25-row pages and one expanded detail.
- Fresh exact native proof precedes browser-ID removal. Malformed/quota-failed
  browser persistence is preserved and holds new starts. Explicit settled buffer
  release is serialized with lookup, clears all tabs on confirmed success, and
  filters queued/late row events at delivery. Original outcomes are unchanged.

Meaningful failed checks and review corrections:

| Evidence label under `.cache/verification/reviewed-console/` | Observed failure / correction |
| --- | --- |
| `retention-frontend-first` | 80 pass / 4 fail and seven unhandled malformed-accounting errors; native DTO validation now rejects unavailable/malformed values without fabricated zero counts; old UI expectations were updated |
| `retention-ui-regressions-first` | 90 pass / 1 fail: a later page error overwrote storage failure; storage errors now remain independently visible |
| `retention-ui-regressions-second` | 90 pass / 1 fail: proof-only entries survived after successful handoff; evict only exact-handed-off settled entries, preserving start-pending callback identity |
| `retention-tscheck-second`, `retention-tscheck-release`, `retention-final-validation` | Test fixture type errors (oversized error-code field, invalid `succeeded` run status and Testing Library's unsupported `exact` option); corrected fixtures use valid types / `completed` and exact string accessible names; no compiler check was bypassed |
| `retention-result-race-reproduce` | Deferred running snapshot erased a newer Result owner; active/pending snapshots now merge newer same-run results; settled native snapshots remain authoritative |
| `retention-release-lane-reproduce`, `retention-release-queued-reproduce` | Release overlapped exact lookup; queued Result/progress resurrected released rows. Reserve the lookup lane across release/local cleanup and recheck current row authority at callback and frame flush; Finished/contradictions remain observed |
| `retention-browser-first` | Real browser detail disappeared after durable-refusal handoff. Refusals now share the detail LRU and refresh stamps recency; keyboard Home returns to the selected detail above inventory |
| `retention-browser-second` | Exact filter-label lookup timed out. Explicit accessible names now match the three visible filter labels; browser rerun passes |

Independent review also found raw archive ownership accidentally recovering other
runs, keyset validation skipping empty/NULL/non-text IDs, and unowned cached views
remaining trusted after a new lease. Separate regressions cover raw temporary
archive claims, malformed identity preflight across v2/v3/v4, and cache invalidation
on every owner acquisition/release. All material findings were corrected. The
reviewer read source/logs and did not execute the lead's tests or browser checks.
The final accessibility-only delta was separately reviewed. Removing an
unsupported test query option then restored TypeScript checking; the exact string
accessible-name assertion remains, and production source is unchanged.

Recorded checks (labels refer to adjacent JSON/log files):

| Check | Result and scope |
| --- | --- |
| `retention-native-first-validation` | 98 pass, one internal child fixture ignored by default; 926 filtered. Real SQLite/OS lease, native owner, helper framing with synthetic SDK; 222.55 seconds including compilation/wrapper, unchanged captured native hashes |
| `retention-full-first-validation` | `pnpm validate` passes on the preceding UI snapshot: 943 frontend / 572 standard Rust tests, 137 default fixture ignores; formatting, contracts, TypeScript, production UI build, native compile and strict all-target/all-feature Clippy. Native hashes unchanged; later UI fixes are qualified separately below |
| `retention-cache-release-final` | 196 cases in four files pass, 1.95 seconds; unchanged captured frontend source. Includes null-after-release, mixed refusal/positive LRU, malformed browser storage and 10,000-ID scheduler cases |
| `retention-all-frontend-final`, `retention-tscheck-final2` | 948 cases / 66 files pass in 27.80 seconds; TypeScript passes in 5.52 seconds. These precede the final accessible-name-only delta |
| `retention-browser-third` | Actual headless browser → Rust Studio/helper/SQLite/Arrow passes, 25.08 seconds including server startup (18.9-second case), unchanged all changed product/test hashes; synthetic SDK, no account semantics claim |
| `retention-final-corrected-validation` | Final `pnpm validate` passes: 948 frontend cases / 66 files, 572 standard Rust cases and 137 default fixture ignores; formatting, regenerated contracts, TypeScript, production UI build, desktop compile and strict all-target/all-feature Clippy. 163.95 seconds; all captured native source hashes unchanged |

The browser receipt JSON SHA-256 is
`8a7a274bcb3c3a4a97e15459696f0ca69a3fa85b6f8346550399095962f7d351`;
it pins every changed product/test file before and after the passing case. The
final compiler-only correction removes the unsupported `exact` test option, with
no production delta. Final full-validation log SHA-256 is
`aee3fbbd3c86060789dcdc4b0b1be6dc556530edd9b023b08776c8ab1aa34a85`.

The browser case creates 26 positive decisions plus one durable refusal, beyond
both detail cache and page bounds. It verifies paging, lost start/refusal/archive
responses, exact original preservation during release, released Arrow retrieval
failure, keyboard scrolling/focus, archive/unarchive filters, browser-ID drain,
and complete UI/native restart without authentication or SQL replay. Native
inventory remains discoverable with no browser correlations. Unit regressions
separately check absent profiles and corrupt browser persistence. This is not
normal packaged macOS or actual Snowflake acceptance.

Review-pinned production hashes:

| Path | SHA-256 |
| --- | --- |
| `src/features/studio/SnowflakeConsole.ts` | `78f7a4230bcf58283e964ddd67d3857803c49b7d0e7102d2bb8df938df5b7e3b` |
| `src/state/studio.ts` | `c4058d5c3827b7d4b49ee6cbc5c2e4d31df7466c4aeac47e4b79d3ebca6e3ea8` |
| `src/features/studio/SnowflakeConsoleReceipts.tsx` | `1ae61fc3c9705d7eeae35e7c2111789eb807f475175610aae12f7872d621a507` |
| `src-tauri/src/persistence/snowflake_console/retention.rs` | `e0fae7b4e1de575cd48bb297a68c3c2a722ef6187c379cb053d6d8b8a0bcfa03` |
| `src-tauri/src/execution/snowflake_console.rs` | `cf09e7c6ec314fa548f5e8bd387ad7caabdaef86d4606c3f296f7930d7a9b277` |
| `src-tauri/src/persistence/recovery.rs` | `8d039e869b6403602a71a96f0d6ede333273046e3841b20e547e33cbf767ada5` |

Reproduce using the pinned Node 24.22.2 / Rust 1.99.1 environment, one Cargo
build at a time and the README's 20 GiB free-space preflight:

```sh
pnpm validate
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib console -- --test-threads=1
PLAYWRIGHT_BROWSERS_PATH="$PWD/.cache/playwright-browsers" pnpm exec playwright test tests/e2e/snowflake-retained-console.spec.ts --workers=1
```

The approved one-time debug cleanup receipt remains
`.cache/verification/build-storage/approved-debug-cleanup.json`: actual net free
space increased 145,766,936,576 bytes (135.8 GiB). The delayed approval response was
not reused to delete rebuilt artifacts. Debug symbols remain line tables with
incremental builds disabled. Existing release bundle `72ee3e4` and the preserved
`9b20aa2` predecessor were not rebuilt or removed for this increment; it is not
present in the runnable release app. Final validation storage: target
**8,092,400 KiB** (about 7.72 GiB), free **211,839,623,168 bytes** (about
197.3 GiB). Debug artifacts measured **5,589,912 KiB**, release **2,502,408 KiB**.
The preserved release binary still hashes to
`e29d03ee6c653361d7a9e7ef1ddf295c5a84991c52fd5c57cafac3748baf0732`.
Private hub publication remains unauthorized.

### Configured console query limits (12 November 2026, locally verified)

Base is clean `b86ac44` on `feat/datagrip-replacement`. The
[contract](snowflake-console-query-limits.md) separates the selected execution/
result-delivery budget from acknowledgement recording and safe settlement.
Local synthetic verification and normal packaging pass within the source scopes
below. Historical passes do not establish new source or account
behavior. Corrected fixture failures and their reruns are retained explicitly.

Before changing the fixed runtime deadline, minimal optional-profile scaffolding
and a held-drain regression were added. With a one-second setting the original
runtime remained waiting beyond the fixture's five-second observation bound.
The actual helper/framing synthetic-SDK case failed with
`configured one-second deadline did not settle within five seconds: Elapsed(())`.
It ran one test, failed one, and changed no captured native source during the
check: **66.32 seconds** including **58.66 seconds** compile and **5.16 seconds**
test time. This demonstrates the old fixed runtime ignored the newly introduced
setting, not that earlier versions already offered configurable deadlines.
Preserved evidence: `.cache/verification/reviewed-console/timeout-baseline.json`
and adjacent log. No account authentication or SQL occurred.

The frozen frontend settings/review increment passes **121 cases in five files**
in **8.57 seconds**, including raw input bounds, legacy default, preserved config,
native returned-plan validation and use of its reviewed value rather than a
possibly changed profile. All eight captured frontend source hashes stayed
unchanged; native implementation proceeded independently. See
`timeout-frontend.json` and adjacent log under the same evidence directory.
This Vitest run precedes native contract regeneration and is not TypeScript,
browser, native runtime or packaged-app qualification.

The initial complete native console build fails before executing tests with
`E0425`: the new Studio fingerprint fixture called a private sibling `compile`
function. The test-only correction calls the public script compiler with the
same SQL, parsed-statements mode and parameters. Production code is unchanged.
The failed **36.54-second** attempt preserves all **226 native source hashes**;
see `timeout-integrated-validation.json` and its focused log. A corrected run is
in progress under a separate label.

The corrected focused native run executes **79 passing cases, one failing case
and one internal child fixture ignored by default**, in **197.63 seconds** of
test time. The new profile-edit case unwraps an absent journal record while
waiting for its asynchronous admission. Source inspection confirms start returns
an allocated run ID after scheduling the owner, before its preparation must be
committed to SQLite. The test-only correction treats an absent row as not ready
within its unchanged 15-second wait, still requiring the actual durable ACK
before editing the profile. Independent review confirms the correction preserves
the suffix and accounting assertions. Production is unchanged; a new frozen
focused run is in progress. The failed run does not establish a passing
profile-edit workflow.
See `timeout-corrected-validation.json` and its focused log. The other new
deadline, quiet-result, cancellation and policy-binding cases pass in this run.

The synchronized frozen-source native run passes **80 cases**, with one internal
child fixture ignored by default, in **287.04 seconds** including compile
(**200.13 seconds** tests). All **226 native hashes** remain unchanged. It
exercises a deliberately quiet result beyond the old 65-second limit, short
expiry before/after ACK, a slow ACK recorder crossing expiry, cancellation with
a 24-hour selected limit, trusted action binding, profile edits and SQL-free
unknown-outcome recovery. No account activity occurs. See
`timeout-synchronized-validation.json` and its focused log. After this run,
target occupies **7,933,016 KiB**, with **212,002,541,568 bytes** free.

The required full `pnpm validate` passes in **271.47 seconds** on unchanged
captured native source: **921 frontend tests in 64 files**, **561 standard Rust
tests** and **137 default fixture ignores**. Formatting, actual contract
regeneration, TypeScript, production UI build, desktop compilation and strict
all-target/all-feature Clippy pass. The generated DTO exposes optional
`reviewedConsoleTimeoutSeconds` and required native `timeoutSeconds`. Both new
strict-profile/persist/reload/backup/malformed-storage tests execute and pass.
See `timeout-final-validation.json` and its full-validation log. Target occupies
**8,046,368 KiB**, with **211,681,787,904 bytes** free afterward. This does not
qualify default-ignored fixtures or actual Snowflake behavior.

The first changed browser run fails in **61.84 seconds** including server build
(**13.2 seconds** test time). The real form submits the exact synthetic profile
with limit 120, but the test harness's `save_profile` guard rejects that profile:
the new bounded normalization was applied to authentication, not the separate
save guard. No console SQL is reached. The native response and request are
preserved in `timeout-browser.json`, adjacent log and
`timeout-browser-failure/trace.zip`. The correction is restricted to the isolated
harness: share the exact fixed-profile predicate, allow only its bounded timeout
change while the synthetic runtime is active, and reject changed identity/auth/
policy. Production code is unchanged; affected checks and browser rerun are
pending. The full validation above precedes this fixture-only correction.

The exact-profile fixture correction passes **three binary regression tests** in
**6.11 seconds**, with unchanged captured source; see `timeout-harness-tests.json`
and log. Identity, authentication and other policy fields cannot vary; only the
valid query limit can, and both save/auth exceptions require the active synthetic
runtime. Independent bounded review finds no material issue. The affected strict
checks and browser rerun use separate receipts. The browser command was queued
before strict checks had fully terminated; its log records Cargo waiting for
the exclusive build-directory lock, so compilation remains serialized. No
additional Cargo command will start until both reach terminal results.

The corrected harness's `pnpm check:rust` passes in **92.85 seconds**, including
formatting, desktop compilation and strict all-target/all-feature Clippy. Captured
source is unchanged; see `timeout-harness-strict.json` and log. Production and
frontend source still match their preceding full-validation pins; only the
test-harness predicate/regressions changed after that full run.

The next browser run saves limit 120 and connects successfully but fails its
visibility assertion for the hover-only sidebar Disconnect control. It takes
**82.16 seconds** including the queued build (**16.2 seconds** test time).
The trace preserves actual successful native save/connect responses; source
`.connection-actions` uses `display: none` until row hover/focus. The test now
hovers the same profile row before asserting that control, preserving the normal
UI route, native checks and subsequent status/results/restart assertions.
There is no force click, widened timeout or product change. See
`timeout-browser-corrected.json`, log and
`timeout-browser-corrected-failure/trace.zip`.

The hover-corrected run reaches the intentional backend restart, then fails
because its still-live page's ten-second `resource_usage` poll is forwarded while
the owned backend is down. The proxy's uncaught `ECONNREFUSED` fails the test;
this is not a Snowflake connection or product SQL failure. The run takes
**20.36 seconds** (**13.3 seconds** test time); preserve
`timeout-browser-hover.json`, log and `timeout-browser-hover-failure/trace.zip`.
The lifecycle correction unloads the UI to `about:blank`, waits within the
existing bounded assertion deadline for pending bridge forwarding to finish,
then stops/restarts the backend and loads the UI afresh. It retains same-origin
storage and all profile/review/receipt/no-auth/no-SQL assertions, without catching
or masking proxy errors. Independent review confirms this qualifies complete
application restart, not live-frontend resilience while its backend is absent.

The lifecycle run exposes a separate existing browser polling race before
reaching restart: the start-request counter increments before native admission,
and `snowflake_console_run` may still return null. Dereferencing that result
throws instead of waiting. The failed run takes **20.81 seconds** (**9.6 seconds**
test time); retain `timeout-browser-lifecycle.json`, log and
`timeout-browser-lifecycle-failure/trace.zip`. Nullable lookup now remains
not-ready in the existing terminal/recording waits; the normal receipt accessor
still requires a non-null view. No RPC errors are swallowed, absence cannot
satisfy either wait, and mandatory post-restart receipts remain mandatory.

The corrected actual browser workflow passes **1/1** in **32.70 seconds**
(**24.0 seconds** test time), with all captured product source hashes unchanged;
see `timeout-browser-admission.json` and log. It exercises the real form's legacy
65-second default, saving 120, native returned review value, retained-session
adjacency/transaction-shaped queries, Arrow results, lost positive/negative
responses, ordinary hover/click/keyboard receipt interaction at 1024 × 720,
complete UI/backend restart and persisted 120. Original receipts recover with
zero restarted SDK authentication or SQL. This uses a synthetic SDK, not an
actual Snowflake engine or native macOS webview. No test exception is masked and
no force click or timeout widening is used.

The initial complete helper run executes **389 cases: 386 pass and three fail**
in **293.60 seconds**. All three new transport-policy cases successfully deliver
the guarded SDK query but record no timeout calls because the test observes
native `ClientRequest.setTimeout`; pinned Axios actually invokes the
`follow-redirects` request facade. Pinned source confirms the latter delegates
to its socket and timer without calling that native method. A transparent,
test-only wrapper now observes the actual facade, preserving the request,
receiver, arguments and return values. Production, SDK pins and the strict
`[125000]` assertion are unchanged. The corrected **three HTTP/TLS/proxy cases
pass in 7.62 seconds**, with unchanged captured helper hashes and zero account
activity. Their delayed synthetic reply takes longer than the probe's ordinary
one-second request budget. See `timeout-helper.json`, `timeout-sdk-policy.json`
and adjacent logs. The corrected full affected SDK file then passes **194/194**
in **285.35 seconds**, with all four captured helper hashes unchanged; see
`timeout-sdk-corrected.json` and log. The other **195 helper cases** passed in
the earlier full run, before the two test-observer files changed. This is
source-scoped coverage across two runs, not a claim of one new 389-case pass.

Runtime preparation first fails in **14.29 seconds** because the default offline
cache lacks the locked `@smithy/types` 4.20.0 tarball (`ENOTCACHED`). Repeating
with the existing repository cache `.cache/snowflake-helper-dev/npm-cache`
succeeds offline in **23.69 seconds**, without changing any of the four captured
helper source hashes. Resource verification passes in **0.55 seconds**, followed
by guarded staged-helper startup in **1.78 seconds** with **zero authentication
and SQL requests**. The SDK remains **3.4.1**, and the new staged manifest SHA256
is `2f7ddcf0c2ad93b0179be8105456573b6e9a3467ba5dfb295acf7061dfc9a91a`.
See `timeout-runtime-staging*.json`, `timeout-runtime-resources.json`,
`timeout-runtime-startup.json` and adjacent logs. This is staged resource/startup
evidence, not a new normal app bundle or UI acceptance.

The reviewed product/test increment is committed as `b3ce956`; the separate
retention proposal and roadmap link are `72ee3e4`. The final bounded independent
review reports no remaining material findings. Its ignored report is
`console-timeout-final-review.json`, SHA-256
`9c8019e472eaddf4e07440da60a23ed60cba7af58775827a696332521cb7ebbc`.
It independently matches all 30 final-browser product hashes and the 226 native
pins, attributing the later harness-only change to its separate three binary,
strict and browser checks. It ran no additional builds or tests. Receipt retention
remains a design, not implemented capacity or recovery qualification.

Before replacing the default bundle, the complete `9b20aa2` app was preserved at
`.cache/verification/reviewed-console/preserved-release-9b20aa2/Joinery.app`
using macOS `cp -cRp` APFS cloning. All **13,055 regular files**, **241,019,097
bytes** and zero symlinks match the source before/after copying; strict deep
signature verification passes. Observed free space changes by less than 3 MiB
while data blocks are shared; that whole-volume change is not an exact clone
allocation measurement. See `timeout-prior-app-preservation.json`.

The single serialized `pnpm desktop:snowflake:bundle:mac` run passes in
**315.99 seconds** from clean commit
`72ee3e4248a302388c9bc91d0384f8ab188ada2d`, tree
`0ae509c15aa1ad95103c15a43c19dd497486b21c`. The checkout stays clean throughout.
It rebuilds the production UI/desktop app, verifies resources and the strict
deep ad-hoc signature, then checks the packaged helper behind a network guard:
SDK **3.4.1**, **zero authentication / SQL requests**. Packaged resource manifest
SHA-256 is `2f7ddcf0c2ad93b0179be8105456573b6e9a3467ba5dfb295acf7061dfc9a91a`,
with **15,109 resource entries and zero symlinks**. Existing `ts-rs` ignored-serde
attribute warnings recur. The installed Rust toolchain's optional `rust-objcopy`
strip step again warns about missing `libLLVM.dylib`/SIGABRT; the build proceeds
and all final bundle checks pass. No global toolchain changes are made.

The normal local app at `src-tauri/target/release/bundle/macos/Joinery.app` is
**ARM64, version/build 0.1.0**. Its executable is **33,235,136 bytes**, SHA-256
`e29d03ee6c653361d7a9e7ef1ddf295c5a84991c52fd5c57cafac3748baf0732`.
The complete app has **13,055 regular files**, **241,021,796 bytes** and zero
symlinks. A separate post-build whole-file comparison confirms the preserved
`9b20aa2` app remains unchanged; both signatures still verify. See
`timeout-app-package.json`, its adjacent log and `timeout-app-artifact.json`.
The app is neither installed nor launched for native UI acceptance. These checks
do not establish external-browser SSO, account semantics or daily replacement.

Independent package/docs follow-up finds no material issue and independently
checks the current executable hash and unchanged non-document source/dependency
pins. Its separate report `console-timeout-package-review.json` has SHA-256
`d95fa5e35967fccb0f2c5464bdfdbdf9baa37330025e2dd29c43f5c0e8c1c594`;
the historical final source review above remains unchanged. Storage and complete
preserved-app comparisons are attributed to the lead's checks. The reviewer ran
no builds, tests, native UI or account operations.

The pre-build 20 GiB free-space gate observes **211,172,790,272 bytes** free;
post-build observation is **210,786,861,056 bytes (196.3 GiB)**. These are actual
whole-volume observations, not an allocation attribution. Scoped `du` reports
**8,080,336 KiB (7.7 GiB)** for the whole Rust target, including **5,577,920 KiB**
debug and **2,502,408 KiB** release. The earlier specifically approved cleanup
receipt remains `../build-storage/approved-debug-cleanup.json`: it already
reclaimed **145,766,936,576 bytes (135.8 GiB)**. The delayed approval does not
authorize deleting rebuilt caches again. Source, release apps, test evidence,
dependencies and application data remain preserved. Development builds retain
the committed smaller-symbol/no-incremental policy; no automatic cache deletion
is introduced. Actual Snowflake/native/owner acceptance and full applicable
DataGrip parity remain open; the Goal stays active.

### Durable console admission refusals (12 November 2026)

Base is `4ac8eb2` on `feat/datagrip-replacement`. This increment adds SQL-free
request lookup for both admitted runs and immutable, committed pre-admission
refusals. A lost refusal response can be recovered without replay. SQLite schema
3 separates physical journal tables, excludes already-open version-2 SQL writers
and preserves current decisions when restoring older backups. An uncertain
refusal write retains its original in-memory payload and holds its identity;
absence, arbitrary errors and process loss before commit still prove nothing.
The frontend retains opaque client UUIDs and holds contradictory decisions.
The [design](snowflake-console-negative-receipts-design.md) and
[walkthrough](snowflake-sdk-console-workflow.md) describe the limits.

Native implementation is committed as `ed99fa4`, frontend recovery/layout and
browser coverage as `4956562`, and the separate navigation fixture correction as
`ebb0d53`. The checks below ran before these local commits; the lead compared
their captured source hashes with the reviewed and staged source without drift.

Local verification for this increment is complete within the synthetic scope
below. Normal packaging passes; external/native UI acceptance remains pending. Focused
frontend checks pass **205 cases** across
seven files, IPC checks pass **9 cases** across three files, and TypeScript
compiles against the actual newly generated contract. Local records are under
`.cache/verification/reviewed-console/`: `negative-frontend-implementation.json`,
`negative-frontend-focused.log`, `negative-frontend-typecheck.log` and
`negative-ipc-focused.log`. The contract generator exits zero; its receipt
`negative-contracts.json` records a concurrent change only to the new persistence
test file, with no production-source drift. It is contract-generation evidence,
not a frozen-source native test result.

The initial native test compile fails before running tests: test code accessed a
private coordinator field, and two fixture parameters passed `u64` directly to
SQLite. A test-only cloned accessor and checked SQLite integer conversion fix
those compile errors. The next frozen-source run passes **69 cases**, fails two,
and ignores one internal child fixture invoked by its parent. Its failures are
test synchronization/execution mistakes: the synthetic helper's stream marker
can precede the parent's SQLite ACK commit, and a prepared SQLite SELECT must
step its lazy cursor to exercise a missing-table error. The cancellation test now
waits within its existing 15-second barrier budget for the actual durable ACK;
the stale-writer SELECT test steps the cursor. Production deadlines and migration
behavior are unchanged by these corrections. Failed receipts/logs remain
`negative-final-validation.json` and `negative-corrected-validation.json`.
Independent review confirms the ordering and both corrections. The corrected
focused run passes **71 cases**, with one internal child fixture invoked by its
parent, in **185.66 seconds** including compile (**117.84 seconds** tests).
Captured Rust source hashes remain unchanged; see
`negative-synchronized-validation.json` and adjacent log.

The first full validation passes **892 frontend tests** across 64 files, the
production UI build, **559 standard Rust tests** with **137 default fixture
ignores**, formatting, contracts, TypeScript and the desktop compiler check.
Strict Clippy then rejects `.err().expect()` in the corrected lazy-cursor fixture;
the narrow correction uses `expect_err` without changing production behavior.
See `negative-integrated-validation.json` and adjacent full-validation log.
The next full run stops on an unchanged SQL-navigation usage-limit test: 891
frontend cases pass and that case returns no symbol. It passed in the prior full
run and again in isolation. Its approximately 11 KiB cold CodeMirror fixture
depends on timed parsing while asserting a 500-usage cap and Unicode positions.
Source inspection proves this timing sensitivity, but the actual failed branch
and scheduler cause were not observed. A test-only bounded preparse now asserts
the parse precondition before the unchanged semantic assertions. Production
75/10 ms budgets remain unchanged. The corrected narrow case passes; source and
failed-run evidence are retained in `navigation-usage-fixture-investigation.json`,
the adjacent two narrow logs and `negative-checked-validation.json`.

The separate pre-layout `pnpm check:rust` passes formatting, desktop compilation and
strict all-target/all-feature Clippy in **80.41 seconds** on unchanged captured
native source; see `negative-strict.json` and log. The pre-layout full `pnpm validate`
passes in **198.66 seconds**: **892 frontend tests** across 64 files, **559
standard Rust tests** with **137 default fixture ignores**, formatting, contracts,
TypeScript, production UI build, desktop compilation and strict Clippy. Captured
Rust source hashes remain unchanged; see `negative-final-checked-validation.json`
and adjacent log. Target occupies 5,239,580 KiB (5.0 GiB), with 216,950,030,336
bytes (202.1 GiB) free after that run. Browser verification at that point was
pending; the corrected current-source result follows below.

The first actual browser run fails after eleven acknowledged runs: expanding the
unstyled app-scope receipts section creates an implicit shell-grid row, leaving
the editor's Run button behind the footer/tab strip. Its ordinary click times
out; no force click or bypass is used. `negative-browser.json` and log retain the
unchanged-source failure; the trace/context are archived in
`negative-browser-failure/`. A bounded nonmodal card removes receipts from grid
sizing, keeps its header and pending notices visible, and puts history inside a
keyboard-focusable scroll region. It overlays a bounded lower results area while
open; settled history can be collapsed. The App layout source is unchanged.
Focused UI checks pass **61 cases** across four files and TypeScript compiles;
see `negative-receipt-layout-implementation.json` and adjacent logs.

The corrected actual browser case passes **1/1** in **26.56 seconds** including
server startup (**18.9 seconds** test time), on unchanged captured source; see
`negative-browser-layout.json` and log. It exercises browser → real Rust Studio,
SQLite journal, helper/framing and Arrow results with an isolated synthetic SDK.
After its same-session SQL/transaction-shaped sequence and lost positive
response, it resizes to **1024 × 720**, keyboard-scrolls eleven receipts and
submits another review with ordinary clicks. The fault route removes approval
from the actual native request, verifies its committed refusal and discards only
the response. SQL-free reconciliation recovers the decision with one start
request and no extra helper SQL. Clicking Reconcile verifies the real response.
A graceful backend restart restores positive and negative receipts; another UI
reconciliation leaves both SDK events and SQL empty. Native responses are never
fabricated. This proves local correlation/layout behavior, not actual Snowflake
effects, account cancellation, transaction semantics or performance.

The final post-layout `pnpm validate` passes in **206.21 seconds** total (**205.89 seconds** for the validation stage):
**893 frontend tests** across **64 files**, **559 standard Rust tests** and **137 default fixture
ignores**. Formatting, contracts, TypeScript, production UI build, desktop
compilation and strict all-target/all-feature Clippy pass. Captured source hashes
remain unchanged; see `negative-layout-checked-validation.json` and
`negative-layout-checked-retained-full-validate.log`. Target occupies
**5,545,644 KiB**, with **216,742,141,952 bytes** free after the run.

The bounded independent review records no remaining material source finding in
`negative-final-review.json`; it is a read-only source review, not another test
run or account qualification. Its final documentation/source pins are refreshed
separately so the ledger does not create a hash self-reference.

Storage verification uses serialized Rust runs with a 20 GiB free-space gate.
Both default target output trees became absent before the first Cargo invocation
of this increment, outside these commands. No further cleanup was performed.
The older preserved `278754b` app remains available under
`.cache/verification/reviewed-console/preserved-release-278754b/`; it predates both
console application increments. Historical packaging results below describe
their recorded source, not a currently retained default-target artifact.

One normal SDK-bearing ARM64 package is now built from clean committed source
`9b20aa232d47ae78880c977e0d9bc318e712105e` (tree
`03d5cab4c0ad2aff02ca5f8acc93b37ebf4966a6`). The command
`pnpm desktop:snowflake:bundle:mac` passes in **718.09 seconds**, with no captured
source or checkout drift. Strict ad-hoc signature/resources pass, the bundled
Snowflake SDK is 3.4.1, and the network-guarded helper startup reports **zero
authentication and SQL requests**. This does not launch or qualify the normal
native UI. The existing `rust-objcopy` missing-`libLLVM.dylib` warning recurs for
the library and executable; the stripping steps abort, but compilation, bundling
and all package verification commands finish successfully. No global toolchain
configuration is changed. See `negative-app-package.json` and adjacent log.

The uninstalled version-0.1.0 app is at
`src-tauri/target/release/bundle/macos/Joinery.app`. Executable SHA256 is
`87cdeec3a9ae9932cbae6172a9863e6ee19073e92be0bce18c3be1a2eed3ae50`.
It contains 13,055 regular files totaling **241,019,097 bytes**; the builder
reported **225.66 MiB** before final package checks. After packaging, debug occupies
**5,511,768 KiB**, release **2,511,820 KiB**, and actual free space is
**214,530,072,576 bytes** (about 199.8 GiB). All preserved previous-app files and
symlinks match their earlier manifest. See `negative-app-artifact.json`.

Reproduce local checks with the pinned README toolchain and prepared runtime:

```sh
pnpm validate
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib console -- --test-threads=1
PLAYWRIGHT_BROWSERS_PATH="$PWD/.cache/playwright-browsers" pnpm exec playwright test tests/e2e/snowflake-retained-console.spec.ts --workers=1
pnpm desktop:snowflake:bundle:mac
```

These runs used Rust 1.99.1, Node 24.22.2 and pnpm 11.23.0. The local runners set
the toolchain PATH and pointed `JOINERY_TEST_NODE` at that Node executable. The
ignored receipts capture commands and source hashes. Source review is separate
from package/startup evidence.
The private project hub is unchanged under the owner's no-publication constraint;
local repository documentation carries this milestone.

No account/profile/approval-marker access, sign-in, billable operation, app
installation, push or publication occurs in this increment. Real Snowflake
effects, million-row interaction, packaged native workflows and owner acceptance
remain open, along with the other required capabilities in the matrix.

### Retained console application integration (12 November 2026)

This and subsequent sections retain historical results and gaps at their named
source. The current admission-refusal increment above supersedes the lost-negative
recovery gap below; it does not extend historical package or account qualification.

Base is `ec7e473` on `feat/datagrip-replacement`; native integration is committed
as `e20c416` and UI/browser coverage as `c3bada6`. The retained Studio owner,
native review/start/SQL-free lookup commands, opt-in profile and editor workflow
are locally implemented. The [walkthrough](snowflake-sdk-console-workflow.md)
describes explicit review, same-session runs and receipt/result recovery. No
Snowflake account/profile/approval-marker access or billable activity occurs.
Root graph access remains unavailable (Transport closed); workers record bounded
Tier 2 graph coverage and exact source fallback in their local manifests.

Independent review found partial passphrase rotation could mutate the secret and
then fail its SQLite settings write without revoking a script's next statement.
The actual synthetic-helper regression failed first (two ACKs instead of one;
exit 101, `console-app-passphrase-reproduction.json` and log). Revocation now
precedes the first secret mutation. The corrected focused suite passes **54
cases**, with one internal child fixture ignored by default and invoked by its
parent. This includes 17 retained Studio owner cases, adapter/journal regressions,
real OS lease/crash recovery and actual SQLite history/audit retry/restart.
All captured Rust source hashes remained unchanged during the run; see
`console-app-corrected-validation.json` and adjacent log under
`.cache/verification/reviewed-console/`. The initial compile before reproduction
failed with a temporary admission fixture borrow; zero tests ran in that compile.

The frontend owner records **217/217** focused cases across eight files and root
IPC checks pass **6/6**. Receipt-owned native rows survive edit/rebind/close/next
run and late delivery; reopening uses existing result IDs without SQL/sign-in.
An exact native pre-admission negative settles only its matching unadmitted
request. Lookup/lease uncertainty still throws; no arbitrary error or null lookup
proves non-dispatch. A **lost negative response has no durable tombstone yet** and
can remain held; durable negative recovery is required before daily acceptance.

A final closed-coordinator lifecycle fix preserves known-run lookup/dedup after
shutdown while preventing lease reacquisition through fresh starts or abandoned
receipt recovery. The affected owner suite passes **18/18** on unchanged final
source (127.28 seconds including compile; 73.42 seconds tests), including a real
two-coordinator/SQLite lease regression with no additional helper SQL. See
`console-app-owner-final.json` and log. Independent bounded final source review
has no additional material finding across 38 captured paths; report
`console-app-review.json` SHA256
`256fa8e0a52ee6c760d43146db00da9bdba2816ff876168e7e9ba1ac5c6c09b8`.
This includes the subsequent narrow TypeScript guard/test and Clippy corrections.
Root compares the report's captured hashes again and finds no drift. The reviewer
ran no tests.

The first full validation stopped after contract generation at TypeScript
TS2352: a generic connection response was cast directly to SessionInfo. The
correction checks the response's string ID and true console flag before sealing
those exact fields. The second run passes frontend/build and **546 standard Rust
tests** (137 default ignores), then fails strict Clippy on three collapsible
conditionals, one needless test borrow and the public fixture's missing Default.
Those narrow fixes preserve evaluation order and journal lock lifetime; the
reviewer reconstructed the exact earlier source hashes to confirm the delta.
Both failed receipts are retained as `console-app-full-validation.json` and
`console-app-full-corrected-validation.json` with their logs.

The final full `pnpm validate` passes in **252.13 seconds**: **872 frontend tests**
across 64 files, **546 standard Rust tests** with **137 default fixture ignores**,
formatting, contracts, TypeScript, production UI build, native compiler checks and
strict all-target/all-feature Clippy. All captured Rust hashes remained unchanged.
See `console-app-final-check-validation.json` and adjacent log. After validation,
target is 11,536,956 KiB (11.0 GiB), with 147,602,878,464 bytes (137.5 GiB) free.
After the Clippy-only delta, the complete feature-gated owner suite again passes
**18/18**, with unchanged source hashes in **109.18 seconds** including compile.
See `console-app-owner-checked.json` and log.

The first browser attempt failed before application exercise because Playwright
looked outside the existing repository browser cache. Setting only the command's
`PLAYWRIGHT_BROWSERS_PATH` uses the already cached Chromium headless shell 1248,
without download or global configuration changes. The next attempt exposed a
test-flow mistake: selecting the profile opens ConnectDialog, while a status-bar
profile name alone does not prove a connection. The corrected test explicitly
submits Connect, waits for the dialog to close and observes Disconnect before
opening a query. Action waits are bounded to ten seconds.

The final actual headless browser case passes **1/1** in **21.66 seconds** including
server startup, on unchanged captured source. It exercises UI → Rust Studio →
real helper/framing/journal/Arrow storage with an isolated synthetic SDK: SET/read,
SHOW/RESULT_SCAN, temporary-table and transaction-shaped cross-run sequences,
lost admission-response recovery with one SQL submission, edit/close and SQL-free
retained-result reopen, then graceful backend restart and receipt restore with
zero login or replay. Assertions inspect actual helper requests and Arrow bytes;
no RPC response is mocked. See `console-app-browser-connected.json` and log.
Failed attempts remain in `console-app-browser{,-cached}.json` and adjacent logs;
the connection-step trace is archived locally. Reproduce with the pinned tools:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib execution::snowflake_console_tests:: -- --test-threads=1
PLAYWRIGHT_BROWSERS_PATH="$PWD/.cache/playwright-browsers" pnpm exec playwright test tests/e2e/snowflake-retained-console.spec.ts
```

The browser-cache override is specific to this checkout; a fresh environment
must install the Playwright headless shell as described in README.

Normal SDK-bearing macOS packaging passes in **520.08 seconds**, from clean
committed source `12fef9a`, with unchanged captured source. Command:
`pnpm desktop:snowflake:bundle:mac`. The ARM64 0.1.0 app at
`src-tauri/target/release/bundle/macos/Joinery.app` contains **240,961,177 regular
file bytes (229.80 MiB)**. Its binary SHA256 is
`ab95811a6fbb21ef15031ff9a2eec5e5132e234c4491bfbbcbfc68ef3d61c996`.
Strict ad-hoc signature/resource checks and the genuine bundled SDK helper's
network-guarded startup pass with **zero authentication and SQL requests**.
This checks helper startup, not ordinary app UI startup. Developer ID and
notarization remain open. The compiler reported that its `rust-objcopy` could
not load `libLLVM.dylib` while stripping library and executable debug information;
the build still exited zero and all bundle checks passed. No global toolchain
configuration was changed, and complete stripping is not claimed.

The preceding `278754b` app is preserved under
`.cache/verification/reviewed-console/preserved-release-278754b/Joinery.app`.
All 15,118 captured entries were verified on copying, and retained file bytes/
symlink targets were checked again after packaging. See
`prior-release-preservation.json`, `console-app-package.json`/log and
`console-app-artifact.json`. Final free space is **147,337,420,800 bytes (137.2
GiB)**; debug output is 5,392,944 KiB (5.14 GiB), and release output is 6,321,976
KiB (6.03 GiB). The smaller development profile is not an automatic cache cap.
Browser fixture servers have stopped; unrelated workspace listeners remain.

These results do not establish real Snowflake transaction semantics,
SSO, million-row interaction/export, normal packaged operation or owner
acceptance. The new app is local, uninstalled and unpublished. Journal retention,
durable negative admission recovery, configurable query deadlines and the
remaining DataGrip scope stay required; the persistent Goal is incomplete.

### Retained console adapter and journal (12 November 2026)

Base is `f490d23` on `feat/datagrip-replacement`. The
[contract](snowflake-sdk-console-journal.md) distinguishes private adapter/journal
evidence from the still-missing Studio owner and usable UI. Independent bounded
source review covers adapter ownership/revocation, ACK ordering, journal originals
and accounting, migration, OS lease/crash recovery, restore and test fixtures.
No material finding remains in that scope.

The initial focused compile failed with new fixture references to the nonexistent
`SafeMode::ConfirmWrites` and unsupported rusqlite `u64` conversions. No tests ran.
Those were corrected to the real enum and checked SQLite `i64` boundaries. The
second run passed 34 cases but failed an existing Studio helper-start test; an
exact isolated rerun reproduced it. The prior helper change imports SDK
`dist/lib/util.js` before hello, while the tiny synthetic SDK omitted that module.
The fixture now supplies synthetic predicates. Genuine SDK source pins and
loopback qualification remain separate and unchanged.

Review also caught three integration hazards before admission wiring: ACKs must
be recordable before publication, metadata restore must not roll back live ACKs/
client identities, and a recorded intent followed by proven local non-dispatch
must not become a database rejection or unknown. Mandatory async recording,
compatible live-journal merge and a closed non-dispatch original resolve them.
Original ACKs, terminal-time observations, later delivery failures and crash
uncertainty stay separate.

On the final frozen 223 Rust/manifest source paths, this command passes **35 cases**
with one internal child fixture ignored (the parent explicitly invokes it):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib console -- --test-threads=1
```

This includes 13 adapter, 19 journal, actual old/v1 metadata-restore, closed
qualification-guard and existing Studio relation-lane cases. The real subprocess
case proves a live owner blocks recovery, forced process termination releases
the OS lease and later local recovery seals unresolved intent without SQL/replay.
It does not prove any remote outcome or power-loss durability. Full `pnpm validate`
passes in 346.4 seconds on unchanged Rust sources: 823 frontend tests, 545 standard
Rust tests (137 default fixture/helper ignores), formatting, contracts, TypeScript,
production UI build, native compiler checks and strict all-target/all-feature
Clippy. Receipts/logs are ignored local evidence
under `.cache/verification/reviewed-console/retained-{second,third}-validation.json`
and adjacent logs; first compile evidence is `retained-validation.json`.

Additional feature-gated checks on the same unchanged source pass **117 adapter
cases**, **9 qualification-profile cases** and **18 qualification-envelope cases**
(one internal envelope child ignored). The envelope mutation case rejects console
authority even after untrusted fields are rehashed. Adding fixed false consent to
the declaration changes its digest; future account qualification needs a fresh
reviewed envelope and approval, without widening old authority.

The first broader Studio scope passed 35/36 but failed
`sdk_studio_stable_observation_preserves_plans_but_full_runtime_drift_revokes_them`
at helper connection, before observation logic. Its exact isolated rerun passes
1/1; the single full rerun passes **36/36** in 222.55 seconds. All runs retain
unchanged source hashes. The startup failure remains unexplained: the current
adapter maps process creation, hello timeout, EOF and invalid-frame failures to
one message. Independent source/log review establishes no material runtime
defect; host startup latency is plausible but unproven. No deadline was loosened.
Closed test-only startup diagnostics remain a follow-up, and these reruns do not
qualify actual SSO reliability. Receipts are
`retained-regressions-validation.json` and
`retained-final-boundaries-validation.json`, plus
`studio-observation-isolated.log`. The final boundary receipt has all three
stages exiting zero on the same 223 source hashes.

After full validation, `target` is 11,406,688 KiB (10.9 GiB) with 147,823,386,624
bytes (137.7 GiB) available. After the additional regressions, target size is
unchanged and 148,024,225,792 bytes (137.9 GiB) is free. Smaller development
settings preserve space; no automatic cache cleanup or size cap is claimed.
No owner account/profile/approval-marker access,
billable activity, packaging, installation, publication or owner acceptance ran.
Current runnable release app remains the prior verified `278754b` bundle and does
not contain these foundations. Full native console/SSO/restart, Snowflake data
effects, million-row interaction/export and the complete DataGrip scope remain
open. No new UI flow exists to qualify end to end yet.

### Build storage recovery (12 November 2026)

The owner reported low iMac storage during the next retained-console increment.
Scoped `du` measured 142 GiB in `src-tauri/target`, including 136 GiB under
`debug` (54 GiB incremental cache, 78 GiB dependencies/object/test artifacts).
No Cargo/rustc/packaging process or open target file remained. After explicit
approval, only `src-tauri/target/debug` was deleted. The completed cleanup receipt
under `.cache/verification/build-storage/approved-debug-cleanup.json` records
145,766,936,576 bytes (135.8 GiB) net free-space increase, with 140.6 GiB free.
All 578 captured tracked/source hashes and the retained release binary SHA256
`37cec732c57030b7e6dfed54bf0d6df9442bd5e7dee688c8bba0692da29ab085`
match after cleanup. Source, release app, dependencies and verification evidence
were preserved. APFS available-space measurements, rather than summed artifact
sizes, establish the recovery.

Repository development/test profiles now disable incremental caching and use
line-table debug information. Cargo metadata parses offline successfully and
the release profile is unchanged. These settings reduce future artifact growth
but do not impose a size cap or perform automatic cleanup. Recompilation can be
slower and full debugger variable/type information is absent. README requires
space checks before/after substantial builds and no new builds below 20 GiB
free. No rebuild was started for the cleanup/configuration check. Existing test
results below remain evidence for their recorded frozen sources. At cleanup the
retained adapter/journal increment was unverified; subsequent verification is
recorded in the section above.

### Reviewed Snowflake console foundations (12 November 2026)

Base is clean `591055d` on `feat/datagrip-replacement`. The
[foundation contract](snowflake-sdk-console-transport.md) implements a pure
mixed-script compiler and private `reviewedSql` transport without an application
command, profile opt-in, retained console owner or UI. The
[complete design](snowflake-sdk-console-scripts-design.md) remains required.
No account SQL, authentication, owner-profile/marker-store access, installation
or publication occurs. Account effects and daily replacement acceptance stay
open.

Ignored receipts live under `.cache/verification/reviewed-console/`. Commands use
pinned Node 24.22.2, pnpm 11.23.0 and Rust/Cargo 1.99.1 with explicit RUSTC/RUSTDOC;
child tests receive the pinned absolute JOINERY_TEST_NODE. The initial source
freeze binds 17 changed source/test files, manifest SHA256
`45e4820633e109f3760b61d78fc45fea998b5a2056f38799ab88cd6ba8031755`.
Root graph calls return Transport closed. Delegated Tier 2 coverage uses matching
project/generations, source reads for the process cfg gaps and excluded SDK
sources, and direct final-delta reads. Coverage is best effort, not proof of
complete indexing.

The compiler review corrected named-input test keys before final verification.
The first compiler run exits 101 with 11 passes/two fixture failures and changed
source; it is not a frozen qualification. Corrected focused compilation passes
13/13, exit 0 in 1.212 seconds, unchanged captured compiler source
(compiler-focused-corrected.json). It covers mixed ordering, global anonymous
binds, shared named values, scalar types, original spans, procedural selections,
whole-plan rejection, canonical forgery, exact client file-prefix veto and
per-unit/aggregate expansion bounds. Unsupported selected syntax remains
explicitly server-validated; no universal dry-run or application permission is
claimed.

```sh
pnpm test:snowflake:helper
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib drivers::snowflake_process:: -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --test snowflake_process -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib safety::snowflake_script::tests:: -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib reviewed_console_transport_cannot_substitute_for_any_qualification_entry -- --test-threads=1
```

Canonical helper regression passes 385/385, no skips, exit 0 in 241.855 seconds;
all seven owned hashes are unchanged (helper-implementation.json and
helper-full-canonical.log). New coverage includes 12 synthetic helper cases and
12 actual pinned-SDK cases across local HTTP/TLS/proxy services: fixed
MULTI_STATEMENT_COUNT=1, unchanged wire SQL/file veto, passive final-response
context including individual unknowns, and original ACK before drain failure.
The initial invocation incorrectly used repository cwd for package-relative
subprocess paths: 381 pass/four MODULE_NOT_FOUND-related failures. It is retained
as helper-initial-invalid-cwd, superseded by the clean canonical run without a
source change. SDK loopback is actual driver behavior against synthetic local
services, not Snowflake account qualification.

Final process tests pass 70/70 in 34.650 seconds including compilation and actual
child-process integration passes 43/43 in 58.727 seconds (23.51 seconds test time).
Receipts process-unit-final.json and process-child-final.json record unchanged
owned source. They cover strict shape/order/identity, passive context bounds,
query-ID upgrades, arbitrary bounded rows, credits/discard, cancellation,
EOF/protocol/drain failure, dropped consumers and stale controls.

Review identified a newly introduced ordering race: failed original terminal
publication preceded retirement, briefly allowing replacement admission. The
reviewed-only fix retires under State before waking that terminal recipient,
retains the original ACK/report, and publishes other retirements outside the
lock. Explicit closing already seals admission and allows priority logout to
continue. A deterministic observer regression verifies both intervals. Reversing
only the new retirement ordering reproduces the exact replacement-admission
panic, exit 101 (process-retirement-race-baseline-corrected.json). The first
baseline failed to compile a test receiver and establishes no reproduction.
Fixed source SHA256 is
`aecb9384221358bedb505d034e6e0b9f3b128562ffe67ad44f49ebceeef890a8`.

The qualification wire regression passes 1/1 in 0.280 seconds, rejecting the new
operation for all 393 closed ledger entries. No qualification route or account
authority is expanded. Final process/compiler/gate receipts have unchanged
captured source. Independent bounded compiler/transport reviews have no remaining
material finding; review source hashes and limitations are in compiler-review.json
and transport-review.json. Reviewers did not run these checks themselves.

Full `pnpm validate` passes, exit 0 in 698.215 seconds with all 17 captured
source/test hashes unchanged (validate.json/log): 823 frontend and 521 standard
Rust tests, with 136 fixture/helper ignores, formatting, contract generation,
TypeScript, production UI build, compiler checks and strict all-target/all-feature
Clippy. Separately executed helper/child tests above are not inferred from these
default ignores. Existing nonfatal ts-rs attribute and UI bundle-size/timing
warnings remain. New contract/design documentation passes Prettier; the relative
file-target check finds no missing target (fragments are not validated).

Runtime tooling passes 54/54 in 3.557 seconds. Offline preparation first fails
because the default repository cache lacks @smithy/types 4.20.0. The documented
existing cache resolves it without a source change:

```sh
pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline
node scripts/verify-snowflake-package.mjs
node scripts/verify-snowflake-runtime.mjs
```

Preparation passes in 19.456 seconds; staged resources pass with 15,109 entries,
five contained symlinks and manifest SHA256
`8b9005d801c8242aeac4572e5e2507cb4367b3078c41cc243942f565818fa622`.
Guarded staged SDK 3.4.1 startup passes in 1.879 seconds with zero authentication
and zero SQL requests. This is a staged helper, not a rebuilt/installed macOS
app. The existing normal package still matches the recorded `278754b` binary
hash; it does not contain this new foundation. No changed UI flow exists yet;
browser/native console acceptance requires the remaining application integration.

### Script early-stop accounting (12 November 2026)

Base is clean e8f0765 on feat/datagrip-replacement. Ignored evidence lives under
`.cache/verification/script-accounting/`; source-freeze-initial.json binds the
25 changed source/test paths used by the database/browser checks and initial
review. Final commands use pinned Node 24.22.2, pnpm 11.23.0 and
Rust/Cargo 1.99.1 with the repository-local Node/browser cache. Root graph calls
still return Transport closed. Delegated Tier 2 coverage uses generation
08:21:56Z, matching metadata/no recorded issues plus direct reads; coverage remains
best effort. Independent bounded review reports no material finding and matches
all 25 hashes (review.json, freeze-manifest SHA256
50be0e767344e39ee246bb95fd87fd062157c3be4fcbccf911c30f4a9fab97fa).

The corrected real PostgreSQL baseline fails 2/2 as intended, exit 101 in
12.570 seconds, unchanged source (real-postgres-reproduction-corrected.json).
Both classified reads truncate and leave a session-local sequence unadvanced,
but the terminal event incorrectly says Completed and has no summary. The first
baseline byte fixture used repeat(), which the classifier treats as Unknown; it
therefore did not exercise the classified-read stop. That fixture was corrected
to a large text literal with an explicit Read assertion. The earlier run and its
changed-source flag remain recorded; neither is promoted to a clean baseline.

The [fixed contract](script-execution-accounting.md) adds SQL-only optional
executionSummary to Finished/history and Stopped only when a limited read leaves
a suffix. Attempts count local driver invocations; successful returns remain
separate from commit/full retention. Non-SQL and typed Snowflake mutation outcomes
retain their prior contracts. No replay, reconnect, automatic commit/rollback,
account admission or qualification allowlist expansion is added.

Focused frontend passes 160/160, followed by the final search helper 11/11 and
TypeScript after correcting one saved-query formatter caller. Frontend formatting
and diff checks pass. Focused executor tests pass 13/13 in 0.080 seconds
(27.340 seconds compilation); owned hashes are unchanged (studio-focused.json).
Two earlier compile-only worker jobs were stopped with exit 130 and ran no tests.
Initial lead contract generation ran before the execution fields were integrated
and failed with two missing-field errors; corrected generation exits 0 in
27.806 seconds while implementation continues. Generation is not a frozen check.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --test script_execution -- --ignored --nocapture --test-threads=1
pnpm exec playwright test tests/e2e/script-accounting.spec.ts
```

Real PostgreSQL cases pass 7/7, exit 0 in 7.410 seconds including compilation,
unchanged captured backend source (real-postgres-corrected.json). They cover
row/byte stopping with a sequence proving the suffix untouched, a successful
control, metadata-only history across a fresh Studio, database error prefixes,
actual cancellation, administrative termination, and a skipped COMMIT that leaves
pending writes until explicit rollback. The preceding run passed six and failed
one incorrect test expectation: pg_terminate_backend supplies PostgreSQL 57P01,
which maps to explicit Database/Failed, not an unacknowledged socket loss. The
corrected test asserts 57P01 before checking Failed. This is explicit server-
termination evidence; unknown transport outcomes have separate synthetic coverage.
Production behavior was not changed to fit the fixture.

The isolated Docker Compose project joinery-script-accounting-20261112 starts
only its own PostgreSQL 18 fixture on previously free 55432 with a separate test
volume. Existing containers and user services are preserved. Tests use synthetic
reads, session-local objects and uniquely named fixture tables only.

The headless browser case passes 1/1, exit 0 in 50.519 seconds including harness
startup (18.3 seconds test), unchanged frozen source (browser.json). It uses the
actual core and isolated PostgreSQL, 1,000,001 generated rows, explicit SQL-text
storage off, a fresh observer proving a skipped UPDATE untouched, run/history/
keyboard history-search summaries, visible wrapping and next-run replacement.
Screenshots are not the success criterion. It supplies no Snowflake account,
normal macOS app, assistive-technology or million-row Snowflake qualification.

The first full `pnpm validate` exits 101 in 544.762 seconds on unchanged
captured source: 823 frontend and 500 standard Rust tests pass (136 fixture/helper
ignores), as do formatting, TypeScript and the UI build. Strict all-feature
Clippy then finds two new test-helper lints (let-and-return and a double-ended
iterator lookup). These are implementation-introduced check failures, not
pre-existing failures. The helper is corrected without production changes; the
final validation result is recorded after rerun. The final source-freeze.json
binds all 25 paths (manifest SHA256
acb44aa4b75e27df9d2b162256a8f8315a1c2286701139f190ac81aeebc22c80).
Only the synthetic executor helper differs from the initial freeze; production,
real-database fixtures and browser source are unchanged. A separate read-only
review confirms both corrections preserve test semantics and all final hashes
match (review-final-delta.json).

The final `pnpm validate` exits 0 in 939.155 seconds with unchanged captured
source (validate-final.json / log): 823 frontend and 500 standard Rust tests,
136 fixture/helper ignores, formatting, TypeScript, production UI build,
compiler checks and strict all-target/all-feature Clippy. The 13 executor cases
run again within this pass. Existing nonfatal ts-rs attribute and Vite chunk/
timing warnings remain recorded. The seven ignored real database cases were run
explicitly as above; the remaining skipped fixtures are not promoted to passes.
Hidden native build exits 0 in 1,397.139 seconds (20m19s optimized compilation),
with unchanged frozen source (native-build.json / log). The isolated native runner
exits 0 in 6.074 seconds, passes all 20 categories and ten cleanup cycles,
4,928 ms native elapsed and 99.30 MiB main-process peak RSS (native-smoke.json /
log). native-final-build.json / native-final-runs.json preserve source/binary
and run evidence before normal packaging replaces the release binary. The smoke
binary SHA256 is
76203d78d684c9394ce305a20e39aa82abbc7c4c33303b101c147319e51fb807.
Resources are absent in this hidden fixture. This qualifies startup/IPC/cleanup,
not normal packaged UI/SSO, fully cold visible startup or representative memory/
performance. The known rust-objcopy/libLLVM stripping warning remains nonfatal;
no machine/global setting or build flag is changed to suppress it.

The task-owned Compose project and its volume are removed, exit 0 in 2.810 seconds
(postgres-stop.json / log). The pre-existing unrelated database container remains
running (fixture-cleanup.json). No user service or browser profile is changed.
Relative file targets in the six changed documentation files pass (414 targets;
fragments are not validated).

The implementation/regression increment is committed as
`278754b688a05da045a5cf21e5e5f88dcd345aea`, tree
`6a6e10107644d8bb876918c7d944b5df19dafc09`. The normal SDK-bearing ARM64 package
passes `pnpm desktop:snowflake:bundle:mac`, exit 0 in 784.396 seconds, on that clean
source with no changes during the check (normal-bundle.json / log). Artifact
capture runs before any following documentation edits and verifies all 25 frozen
source hashes (normal-bundle-build-source.json, normal-bundle-artifacts.json).
Strict ad-hoc signature/resources and network-guarded SDK 3.4.1 startup pass:
15,109 resource entries, zero symlinks, zero authentication and zero SQL requests.
Version/build 0.1.0 totals 240,495,545 final regular-file bytes (229.35 MiB), at
`src-tauri/target/release/bundle/macos/Joinery.app`. Binary SHA256 is
37cec732c57030b7e6dfed54bf0d6df9442bd5e7dee688c8bba0692da29ab085;
resource manifest SHA256 is
74e1dfac7b3346d163fad04879dc7f558c73111d2eac6230a904a98e34621f1f.
The known nonfatal rust-objcopy/libLLVM stripping warning remains recorded.

The bundle is local and uninstalled/unpublished. Developer ID/notarization,
normal packaged UI/SSO/restart, owner hands-on acceptance, generalized mixed
scripts, actual Snowflake writes/transactions/DDL and complete applicable
DataGrip parity remain open. Preceding package evidence retains its source
boundary below. No account activity or owner marker/profile-store access occurred.

### Next required reviewed SQL console/script design

Three bounded read-only reviews use gpt-6.1-sol/high for backend architecture,
UI integration and official Snowflake/SDK semantics. Their ignored reports are
next-mixed-script-architecture.json, next-mixed-script-ui.json,
next-mixed-script-official.json and next-mixed-script-response-context.json in
.cache/verification/script-accounting. Source hashes match their captured paths;
Tier 2 coverage generations 08:21:56Z/09:27:33Z and direct source reads cover the
bounded evidence. SDK/dependency/generated paths are excluded and checked directly.
These are design/research results, not application or account test passes.

The resulting [required design](snowflake-sdk-console-scripts-design.md) keeps a
retained console owner across runs, suppresses implicit SQL observers, separates
passive SDK response context from generated-operation authority and preserves
broader owner-authored SQL as required scope. Single-statement request overrides,
scalar-only binds, exact pinned client file-transfer veto, original ACK/outcome
receipts, lost-start recovery and rollback-only restrictions are specified.
Independent review found two material omissions: lost-start-response admission
correlation and explicit protection of recovery holds from the broad route. Both
are resolved; final design review has no remaining material finding
(mixed-script-design-review.json). Reviewed design SHA256 is
d0345f36a0e093044977fde801a24a6ab6136eb4a15952722d6815718e130548.
No application behavior, SQL admission, account plan or execution authority is
changed by this design. Implementation, real Snowflake effects, normal packaged
workflow and owner acceptance remain open. The private canonical hub is not
published under the user's no-publication instruction; repository status remains
the durable local record.

The following documentation-only checkpoint checks all 381 relative file
targets in its five owned files with no missing targets; fragments are not
validated (doc-links-final.json). Git diff/identity checks pass. The new design
passes Prettier. The four existing status/matrix/roadmap/ledger files have
Prettier warnings on both committed baseline and current text, confirmed by
read-only checks (doc-format-check.json); no unrelated bulk formatting is added.
Application source is unchanged since the clean package capture, so application
checks above retain their named source boundary.

### CHECK-constraint inspection (12 November 2026)

The independent retained-target CHECK action is implemented locally; the
[contract](snowflake-sdk-check-constraints-design.md) and
[prepared walkthrough](migration-from-datagrip.md#snowflake-check-constraints-prepared-acceptance-walkthrough)
record its limits. Full validation, changed browser and hidden native checks
pass. The normal SDK-bearing package passes on clean feature commit 313bc11;
normal account/native acceptance remains open. Preceding package/test results
below retain their own source boundaries.
No account requests, owner marker-store access or account approvals are included.

Evidence uses `.cache/verification/check-constraints/`. Base is clean 73e47c8 on
`feat/datagrip-replacement`. source-freeze-final.json binds all 21 changed
source/test/ACL files; documentation is recorded separately in commits. Each
command receipt records HEAD/tree, changed source hashes before/after, exit and
elapsed time. Independent Rust/UI collectors permit implementation in the other
scope; final checks freeze all source. Pinned Node 24.22.2, pnpm 11.23.0 and
Rust/Cargo 1.99.1 are used, with repository-local JOINERY_TEST_NODE and browser cache.

The initial `pnpm contracts` exits 0 in 83.230 seconds while independent source
work continues; its receipt correctly records changed source, so it is generation
rather than a frozen verification claim. Focused commands on unchanged captured
source all exit 0:

```sh
pnpm exec vitest run src/features/studio/RelationBrowser.test.tsx src/ipc/command-map.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib check_constraints_ -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests:: -- --test-threads=3
```

Frontend/IPC passes 140/140 (138 relation-browser and two typed-contract tests),
12.871 seconds. Forty-one new UI cases cover exact outer/row identity, inert
multiline/Unicode text, independent denial preserving other metadata, retry/empty,
displayed and pending-response invalidation, and still-connected old handlers
after transient holds. Rust focused passes 13/13, 48.409 seconds including
compilation: four pure parser/factory cases, seven synthetic SDK adapter cases,
one seven-state Studio admission case and one test-classifier rejection case.
All synthetic SDK session regressions pass 104/104, 12.238 seconds.

Cases verify exact quoted/bound identities, one retained helper connection and no
USE, denied versus empty metadata, malformed/NULL/duplicate/truncated results,
row/aggregate/JSON-escaping bounds, forged/stale/busy/unavailable targets before
SQL, pre/post drift and held-read refresh/disconnect. The new qualification
panic test rejects the new kind in the test ledger classifier; it is not itself
a runtime wire-admission proof. Production qualification ledger, exact wire
matcher, plan, policy and sequence are unchanged.

Independent bounded UI/integration and backend reviews have no material finding
(ui-review.json, backend-review.json). Tier 2 generations 07:49:25Z cover 15 UI/
integration paths and ten backend/qualification paths with matching metadata and
no recorded gaps, supplemented by direct source reads. Root list/coverage calls
still return Transport closed; no index or global configuration is changed.
The backend pure fixture was corrected before freeze to use BASE TABLE with
Iceberg/hybrid traits, preserving current availability semantics. No failed
product test or accepted family claim is hidden by that correction.

Full `pnpm validate` exits 0 in 456.852 seconds with unchanged captured source
(validate.json / log): 803 frontend tests, 486 standard Rust tests, 129 fixture/
helper ignores, formatting, TypeScript, production UI build, compiler checks and
strict all-target/all-feature Clippy. Existing nonfatal ts-rs attribute and Vite
chunk/timing warnings remain in the log. This run adds no real Snowflake fixture
qualification; skipped database/helper cases remain skipped.

The changed headless Playwright workflow exits 0, 1/1 in 45.994 seconds including
harness startup (browser.json / log; test 13.8 seconds), with unchanged source.
It exercises the real UI through synthetic RPC: keyboard CHECK activation, exact
quoted/multiline clause display, denial preserving other views, explicit retry to
empty results, clearing on referenced-table selection, and held-response refresh
invalidation. No account, user browser profile or native UI acceptance is inferred.

Hidden native build and runner both exit 0 with unchanged frozen source
(native-build.json / log, native-smoke.json / log). The build takes 227.631
seconds overall (3m24s optimized compilation). The isolated runner takes 4.264
seconds, passes 20 categories and 10 cleanup cycles, with 3,639 ms native elapsed
and 100.05 MiB main-process peak RSS. The new command reaches the disconnected-
session guard through native IPC; no unknown-command or permission error is
accepted. native-final-build.json / native-final-runs.json preserve binary/source
and run details before normal packaging replaces the release binary. Runtime
resources are absent in this fixture. It qualifies hidden startup, IPC and
cleanup only, not normal account UI/SSO, cold visible startup or representative
performance. The known rust-objcopy/libLLVM SIGABRT stripping warning remains
nonfatal; no machine/toolchain/global setting is changed to suppress it.

The implementation and regression/docs increment is committed as 313bc11
(`feat(snowflake): inspect CHECK constraints`), tree
`ddef55546eac43687cdcf5b2b53a3e39ef21e2c9`. The normal SDK-bearing ARM64
package builds from this clean commit, exit 0 in 239.042 seconds (3m30s optimized
compilation), with unchanged captured source (normal-bundle.json / log).
normal-bundle-build-source.json records the clean pre-build source and no binary
in-use handles. normal-bundle-artifacts.json binds the final app before any
post-package documentation edit, checks all 21 frozen source hashes and records:

- `src-tauri/target/release/bundle/macos/Joinery.app`, version/build 0.1.0,
  240,477,449 final regular-file bytes (229.34 MiB).
- Executable SHA256
  `310662185fd77eb11f38a2d8e251eb21030bf8b85b64934b6a48523e1b5e4d7b`.
- Strict ad-hoc signature/resources pass, SDK 3.4.1, 15,109 runtime entries and
  zero symlinks; guarded bundled helper startup reports zero authentication and
  zero SQL requests. Existing runtime manifest and Node hashes are preserved in
  the artifact receipt.

The normal app is local, uninstalled and unpublished. Developer ID/notarization,
normal app UI/SSO/restart and real account acceptance remain unqualified. The
private project Site is not published/updated under the user's no-publication
instruction. Local Orca worktree status records progress. No owner process,
browser profile, global configuration, account marker store or credentials are
changed.

A separate read-only SQL planning review (next-script-review.json) confirms in
source that parameterized classified reads run serially on one SDK session;
generalized mixed scripts remain restricted. Source suggests truncation can stop
an unexecuted suffix while status/count reporting stays ambiguous. This is a
source-level concern, not a reproduced runtime defect. Next work must reproduce
row/byte-limit cases with a distinctive second read and a nontruncated control,
then verify planned/completed/skipped accounting in UI/history. A later retained
mixed-script coordinator must preserve typed write/boundary authority and
original outcomes; broader DDL/session/scripting remains required. Tier 2
generation 08:05:01Z matched evidence metadata; a recorded
snowflake_process.rs:1314–1318 gap was read directly and noisy graph relationships
were checked against source. No tests, builds, edits or account activity were
performed for that review.

Official source refresh (official-refresh.json) confirms the five VARCHAR fields,
including CONSTRAINT_TABLE, and current-role visibility. The view supplies no
enforcement/validation flag. Actual account output, visibility, propagation,
interruptions, object-family compatibility and normal packaged usability remain
unqualified. Local synthetic checks do not establish real database behavior.
CHECK creation/modification/validation and all other replacement gates remain
required; the Goal stays active and incomplete.

### Referenced-table selection (12 November 2026)

Local source, browser and hidden native checks pass. The
[selection contract](snowflake-sdk-key-mappings-design.md#referenced-table-selection)
defines exact retained-target navigation without an RPC or session switch.
Cross-database navigation, inbound relationships, related-row navigation and
actual account/native acceptance remain required. Earlier package evidence does
not qualify changed source; the new normal package is bound to clean feature
commit 5219db5 below.

Evidence uses `.cache/verification/reference-navigation/`. The backend worker
froze its two test files before checks; source-freeze-backend.json records those
hashes and clean documentation HEAD 97d5f4d. The checkout was dirty with the new
implementation. Rust-only command receipts capture the two changed Rust files
while UI work continues separately. The 135,125-byte production prefix before
the cfg attribute gating session_tests is byte-identical to that HEAD
(production-prefix.json). No production driver command, SQL factory, ACL or
frozen account-ledger admission is changed.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib reference_navigation_ -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests:: -- --test-threads=3
```

Both completed commands exit 0 with unchanged captured Rust source: 3/3 focused
regressions (rust-focused.log), then 97/97 synthetic SDK session regressions
(rust-sdk-regressions.log). They select pinned Rust 1.99.1 and repository-local
Node 24.22.2/JOINERY_TEST_NODE. Cases traverse source FK mapping to the exact
quoted cross-schema opaque target among same-name alternatives, assert fully
qualified reads/empty binds, one helper connection, unchanged context and no
USE/ALTER SESSION. Forged/stale targets reject before SQL; post-inspection drift
revokes the destination. These synthetic results do not prove account behavior.

Independent review of the two-file backend delta has no material finding.
Tier 2 generation 2026-11-12T07:10:53Z reports matching metadata/no recorded issue
for both paths, supplemented with direct reads. Existing fixture defaults and
SQL inventory checks are preserved. Root graph discovery/coverage still returns
Transport closed, so bounded source reads supplement delegated graph evidence.
The first focused UI run exits 1: 96 pass and one pending-session-rerender
case fails (frontend-focused.log). That fixture queued new props inside an outer
React batch, then expected the still-authoritative old handler to reject before
the replacement rendered. The corrected test actually replaces the live session,
commits new props and awaits the new catalog. It checks that the old mapping
action disappears and only the replacement catalog RPC is sent. Production UI
and browser source do not change for this correction. All 97 focused UI tests
then pass, exit 0 (frontend-corrected.log).

source-freeze-initial.json preserves the first five-file snapshot;
source-freeze-final.json records the corrected snapshot. The lead temporarily
removed only `activationEpoch !== generation.current` and ran the single
still-connected transaction-hold regression. It fails as expected, exit 1
(frontend-epoch-mutant.log): the stale click incorrectly clears the filter.
epoch-mutation-result.json records original/mutated/restored hashes and exact
restoration before independent UI review or full validation. This demonstrates
that the transient-hold regression exercises the generation guard. No mutant
source is retained in the implementation.

Independent bounded UI review has no material finding. Its Tier 2 generation
2026-11-12T07:13:02Z covers the three UI/browser files and the contract/walkthrough.
The TSX metadata changed after exact-byte restoration, so the reviewer inspected
the current functions/component directly; the other four paths report matching
metadata and no recorded issue. Full relevant direct reads and snippets/trace
supplement coverage. ui-review.json records the scope; the reviewer ran no builds
or tests. All three reviewed source hashes match the final freeze.

Full validation exits 0 with unchanged captured source (validate.log / json),
513.535 seconds: 762 frontend / 481 standard Rust tests, 129 fixture/helper
ignores, formatting, TypeScript, production UI build, compiler checks and strict
all-target/all-feature Clippy. environment.json records pinned Node 24.22.2,
pnpm 11.23.0 and Rust/Cargo 1.99.1. Nonfatal ts-rs attribute and Vite chunk/timing
warnings remain visible in the log.

Completed UI/application commands:

```sh
pnpm exec vitest run src/features/studio/RelationBrowser.test.tsx
pnpm validate
pnpm exec playwright test tests/e2e/snowflake-sdk-browsing.spec.ts
pnpm desktop:smoke:build
python3 scripts/native-smoke.py --runs 1
```

The changed headless browser workflow passes 1/1, exit 0 with unchanged source
(browser.log / json), 36.612 seconds. It selects the quoted cross-schema target
while the text filter hides it and a same-name decoy remains in another schema.
Keyboard activation makes zero RPCs, clears the filter/details and focuses the
exact retained destination. A later explicit **Inspect relation** sends that
destination's opaque ID. Existing denial/empty/unsupported and held-response
flows also pass. Synthetic IPC exercises the real UI, not account metadata or
normal packaged macOS focus behavior. No user browser profile is used.

Hidden native build and runner both exit 0 with unchanged frozen source
(native-build.log / native-smoke.log). The build takes 312.169 seconds including
frontend/preparation; optimized compilation takes 3m52s. The synthetic hidden
runner passes 20 check categories and 10 cleanup cycles, 3,524 ms native elapsed
and 99.45 MiB main-process peak RSS. This is startup/IPC/cleanup evidence, not
native account navigation, visible cold startup or representative performance.
Runtime resources are absent in this hidden fixture. native-final-build.json and
native-final-runs.json preserve its source/binary identity and run details before
the normal build replaces the release binary. The known nonfatal LLVM
rust-objcopy/libLLVM stripping warning is retained.

All five final frozen source/browser files match clean feature commit
`5219db5bb20cecafca24c080385d4d0b687a3e80`, tree
`e4a3f01304dd431077156bb504d0ffa0ca42609d`. The normal SDK-bearing ARM64 package
then passes `pnpm desktop:snowflake:bundle:mac`, exit 0 with unchanged source
(normal-bundle.log / json), 259.103 seconds including frontend and verification;
optimized compilation takes 3m42s. normal-bundle-build-source.json records the
clean source boundary, and normal-bundle-artifacts.json records final verified
regular-file bytes and executable/resource identity. Strict ad-hoc signature
verification passes. Both bundle resource/startup checks pass: 15,109 runtime
entries, no symlinks, SDK 3.4.1 and zero authentication/SQL requests under the
Node network guard. This guard is not an OS network sandbox or an SSO test.

The runnable local bundle is
`src-tauri/target/release/bundle/macos/Joinery.app`, version/build 0.1.0. Final
verified regular files total 240,421,113 bytes (229.28 MiB); this differs from the
pre-verification size printed by Tauri. Executable SHA-256:
`e7722ef30db96d3c3a6f0746539f3bed7fac93387b486e75d400a72d1c4aa92b`.
The bundle is uninstalled/unpublished. Developer ID/notarization, normal app
UI/SSO/restart, actual account effects and owner replacement acceptance remain
open. No account requests, private marker-store access, installation, push or
publication occurred. The local Orca status card is updated. The private Site hub
was not updated for this milestone because publishing changes is not authorized.

### Ordered Snowflake key columns (12 November 2026)

Local source verification passes. The independent
`inspect_relation_constraint_mappings` command resolves a retained catalog target,
re-reads declarations and runs three fixed per-table SHOW/pipe projections. The
[contract](snowflake-sdk-key-mappings-design.md) records official/owned source
evidence, strict matching and bounds. This is descriptive metadata with no edit
authority. Dynamic-table mappings, unknown-trait inspection, table navigation and
normal account behavior remain unqualified/unsupported as described there.

Logs and command receipts are under `.cache/verification/relation-key-mappings/`.
The contracts receipt captures 21 source files before the first check. The
browser test was initially outside that collector's path prefix; subsequent
receipts include all 22 source/browser files. The first frontend run passed
84 tests and failed one because the lead's IPC test still expected seven calls
after adding the eighth call. Awaiting the new promise and correcting that count
produced an 85/85 focused pass, without a product change.

The first focused Rust run passed seven tests and failed five in the shared
fixture's final SQL inventory assertion, which did not yet admit the new SHOW
reads. The test-only correction admits the three complete closed factory strings
for DB.PUBLIC.ORDERS by equality; it does not accept arbitrary SHOW statements.
Independent review confirmed only that assertion changed, with no production or
frozen qualification-ledger expansion. The command receipt records a source
change during the first Rust run because of the unrelated IPC test correction;
the Rust files did not change during that run. Initial failures remain retained.

The corrected focused Rust run passes all 12 tests: four pure mapping cases,
seven synthetic SDK session cases and one Studio admission case. The complete
synthetic SDK session suite passes 94/94. `source-freeze-final.json` captures all
22 final source/browser hashes after both test-only corrections; those hashes
match current source. Command JSON receipts retain before/after source identity,
terminal exit status, elapsed time and the corresponding log name.

```sh
pnpm contracts
pnpm exec vitest run src/features/studio/RelationBrowser.test.tsx src/features/studio/ObjectInspectorView.test.tsx src/ipc/command-map.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib mappings_ -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests:: -- --test-threads=3
pnpm validate
pnpm exec playwright test tests/e2e/snowflake-sdk-browsing.spec.ts
pnpm desktop:smoke:build
python3 scripts/native-smoke.py --runs 1
pnpm desktop:snowflake:bundle:mac
```

These completed commands select the same pinned Rust 1.99.1 and repository-local
Node 24.22.2/JOINERY_TEST_NODE as the preceding increment. They make no account
requests. Full validation exits 0 with 743 frontend / 481 standard Rust tests,
129 fixture/helper ignores, formatting/TypeScript/production build/compiler
checks and strict all-target/all-feature Clippy (validate.log). The changed
browser workflow passes 1/1 (browser.log): keyboard access, composite column
ordering/pairing and quoted identities, hidden/unsupported states, denial/empty
retry preserving other metadata, and held responses after refresh. It uses
synthetic IPC and a page-local clipboard stub, not an account or OS clipboard.
Both command receipts record unchanged source. Hidden native build and runner
exit 0 (native-build.log / native-smoke.log), with 10 cleanup cycles, 3747 ms
native elapsed and 99.36 MiB main-process peak RSS. Those synthetic measurements
do not qualify representative performance, visible startup or total app memory.
The normal SDK-bearing package also passes on clean feature commit 54f59b6;
its source-specific artifact evidence follows below. Normal packaged UI/SSO and
account acceptance remain open.

The hidden build compiles in 4m05s (326.565 seconds including frontend and
preparation) at dirty documentation HEAD d173215. All 22 frozen source hashes
still match. native-final-build.json / native-final-runs.json preserve the binary
identity and runner evidence before the normal build replaces the release binary.
The WebView mapping request reaches the structured missing-session guard, not
successful Snowflake inspection. Generated ACL inspection verifies the new
permission admits only inspect_relation_constraint_mappings and is granted to
the main window only (native-acl-generated.json). Existing ts-rs attribute and
LLVM rust-objcopy stripping warnings are retained and nonfatal. No generated
ACL or verification artifacts are committed.

The independent source review found no material issue, including the test-only
corrections. Tier 2 generation 2026-11-12T06:39:41Z matched the repository; coverage
checked 25 increment paths plus the qualification ledger. A reported stale IPC
test was directly re-read. Root graph calls still fail with Transport closed;
bounded direct reads were used. No account requests, private marker-store access,
installation, push or publication occurred. The expired table plan is unchanged
and grants no authority for the mapping reads. Actual Snowflake pipe/result
typing, role visibility, data effects, million-row workflows, normal packaged
usage and owner acceptance remain required.

The normal SDK-bearing ARM64 bundle builds from clean feature commit **54f59b6**
with terminal exit **0**: 4m06s of optimized compilation, 290.983 seconds for the
complete command (normal-bundle.log / normal-bundle.json). Version/build **0.1.0**
is local at `src-tauri/target/release/bundle/macos/Joinery.app`. Final regular-file
bytes total **240,421,113** (229.28 MiB); Tauri reports 225.09 MiB before final
ad-hoc signing. The binary SHA-256 is
`34dced95f634409209464e60d489f63c94a6b66e255e1dcd4326e83270abc09d`.
Strict signature verification, **15,109** runtime entries / **zero** symlinks and
network-guarded SDK **3.4.1** helper startup pass with **zero authentication and
zero SQL requests**. The worktree stays clean during the build, and all 22 frozen
source hashes still match. normal-bundle-build-source.json and
normal-bundle-artifacts.json retain source/tree, artifact identity, version and
checks. terminal-results.json records all 11 completed commands, including the
two initial failed test runs and their passing corrections. Nothing is installed,
pushed, published or merged. Developer ID/notarization, normal packaged daily
usage, actual key inspection and every remaining required workflow stay open.

### Declared Snowflake key constraints (12 November 2026)

This increment adds a separate `inspect_relation_constraints` request to the SDK
relation browser. Only the retained catalog revision and opaque target ID cross
IPC. A closed query binds the retained exact table identity and reads the quoted
database's TABLE_CONSTRAINTS/REFERENTIAL_CONSTRAINTS views. Primary, unique and
foreign-key declarations retain their reported flags and reference visibility;
they do not establish edit authority or validated existing data. The
[contract](snowflake-sdk-constraints-design.md) records scope and official sources.
Column mappings, CHECK/NOT NULL inventories, diagrams/navigation and account
qualification remain required.

Source manifests and terminal logs use
`.cache/verification/relation-constraints/`. The first six-file frontend manifest
was captured after the worker had frozen its files and the first 60-test run
finished. Independent review then found a test-only matchOption value of NONE
outside the backend's accepted FULL/SIMPLE/PARTIAL set. The two fixture files were
aligned to FULL; no product code changed. The focused rerun passes all 60 tests,
with `frontend-aligned-source.json` preserving that snapshot.

The initial full source freeze contains 21 changed source/test files. A subsequent
test-only MutexGuard scope correction in execution.rs ends the recording guard
before a missing-session await. It precedes all focused Rust checks; no Clippy
failure is claimed to have occurred. `source-freeze-final.json` captures all 21
final hashes and differs only in that file. Contract generation was already
compiling without cfg(test), so its product source is unaffected by the test fix.
No global settings or user processes are changed.

```sh
pnpm contracts
pnpm exec vitest run src/features/studio/RelationBrowser.test.tsx src/features/studio/ObjectInspectorView.test.tsx src/ipc/command-map.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib constraints -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests:: -- --test-threads=3
pnpm validate
pnpm exec playwright test tests/e2e/snowflake-sdk-browsing.spec.ts
pnpm desktop:smoke:build
python3 scripts/native-smoke.py --runs 1
pnpm desktop:snowflake:bundle:mac
```

Pinned Rust 1.99.1, repository-local Node 24.22.2 and JOINERY_TEST_NODE select the
same toolchains as the preceding increment; PLAYWRIGHT_BROWSERS_PATH selects the
existing ignored repository-local browser cache. No user browser profile or OS
clipboard is used by the synthetic browser flow.

| Check | Current terminal evidence |
| --- | --- |
| Contracts | Exit 0, contracts.log |
| Focused frontend, corrected fixtures | 60/60, exit 0, frontend-aligned.log; initial 60/60 in frontend-focused.log |
| Focused Rust | 14/14, exit 0, rust-focused.log; includes 12 new regressions and two existing matching cases |
| SDK session regressions | 87/87, exit 0, rust-sdk-regressions.log; synthetic SDK transport, no account effects |
| Full validation | Exit 0, validate.log; 718 frontend / 476 standard Rust tests, 129 fixture/helper ignores, formatting/TypeScript/build/compiler checks and strict all-target/all-feature Clippy |
| Changed browser workflow | 1/1, exit 0, browser.log; keyboard key inspection, hidden references, denial/empty retry and held response after refresh, plus existing definition/inspection/preview flow. Synthetic IPC, no account effects |
| Hidden native build and smoke | Build/runner exit 0, native-build.log / native-smoke.log; 10 cleanup cycles, 3678 ms native elapsed, 97.47 MiB main-process peak RSS. Guard/cleanup evidence, not representative performance or successful Snowflake key inspection |
| Normal SDK-bearing package | Exit 0, normal-bundle.log; clean feature commit fabeb62, strict ad-hoc signature/resources and network-guarded SDK startup pass with zero authentication/SQL requests. Local and uninstalled; normal app/SSO acceptance remains open |
| Independent review | No unresolved material finding; final 21 hashes match. Tier 2 generation 2026-11-12T06:03:28Z, 22 relied source/test/docs paths checked plus direct source reads. Root MCP transport remains unavailable, with bounded direct-source fallback |

Review covers the private ReadSpec binds change (all older factories retain SQL,
limits and empty binds), target/context checks and lane serialization, complete
bounded parser and hidden-reference states, UI retry/coexistence/invalidation,
closed typed IPC and main-window-only command/ACL registration. New read kinds
remain outside the frozen account qualification ledger. The single-table
account plan uses copied source 103cb45 and does not test or authorize this new
command. Synthetic SDK transport/browser and hidden native command rejection
cannot establish actual Snowflake result typing, role visibility or normal app
acceptance. No account requests, approval file or owner marker-store access have
occurred for this increment. The separately reviewed account Plan expired at
06:22 UTC on 12 November without approval or execution. Its
[original review](snowflake-table-attempt-9a8d9d3d.md) remains retained; a new
Plan/window and explicit approval are required before account activity.

The hidden build compiles in 4m05s and records dirty source at documentation HEAD
67b2e39. All 21 source hashes still match source-freeze-final.json. Copied
native-final-build.json / native-final-runs.json retain the exact binary identity
and runner result before the normal package build replaces the release binary.
The WebView request reaches the structured missing-session guard; generated ACL
inspection confirms only inspect_relation_constraints is allowed by its new
main-window permission (native-acl-generated.json). No generated ACL files are
committed. Existing ts-rs attribute and LLVM rust-objcopy stripping warnings remain
nonfatal and visible in the logs. Normal packaged SSO and successful account key
inspection remain unqualified.

The normal SDK-bearing ARM64 bundle builds from clean feature commit **fabeb62**
in 4m43s of optimized compilation, with terminal exit **0**. It is local
version/build **0.1.0** at
`src-tauri/target/release/bundle/macos/Joinery.app`. Final verified regular-file
bytes total **240,344,585** (229.21 MiB); Tauri reports 225.01 MiB before final
ad-hoc signing. The binary SHA-256 is
`c37fd9cdf80adcdcd7c9b5ad6e0d18e53bd6178e951b2f7114ce1b83a510839b`.
Strict signature verification, **15,109** runtime entries / **zero** symlinks and
network-guarded SDK **3.4.1** startup pass, with **zero authentication and zero SQL
requests**. All 21 frozen source hashes still match and the worktree is clean
after the build. normal-bundle-build-source.json / normal-bundle-artifacts.json
retain source/tree, artifact identity, version and check results; terminal-results.json
records all ten completed commands. The normal app is not installed or opened
for SSO. Developer ID/notarization, successful account key inspection and every
normal packaged daily workflow remain open. Nothing is pushed or published.

### Generated Snowflake relation definitions (12 November 2026)

This product increment adds `define_relation` to the same-session Snowflake
relation browser. The request contains a retained revision and opaque target ID,
never SQL or a caller object name. The driver resolves and rechecks the catalog,
serializes through the browsing lane, observes context and exact relation metadata
before/after a closed GET_DDL query, and publishes only one complete, nonblank
text value. Both metadata and final DTO serialization are bounded to 1 MiB.
Column inspection failure does not prevent definition retrieval. Supported kinds,
escaping and GET_DDL normalization/privilege limits are documented in the
[browsing contract](snowflake-sdk-browsing-design.md). Concurrent external DDL is
not made atomic by these observations.

The UI exposes View definition and Copy DDL, with no execute action. Tests cover
stale context, transaction/metadata holds (including transient changes), selection,
refresh, failure, target identity and malformed results. Browser copy uses a
page-local clipboard stub; the OS clipboard is untouched. The native smoke addition
checks command registration and the missing-session rejection, not successful
Snowflake GET_DDL execution.

Source was frozen before the initial checks. The initial 19-file manifest is
`.cache/verification/relation-definition/source-freeze-final-ui.json`; it was
written during the focused frontend check, after the frontend worker had frozen
its source. The earlier provisional manifest preceded the final UI edits and is
not used as an identity record for those checks. The UI correction described
below has its own 19-file `source-freeze-context-fix.json`; exactly the component
and its unit test differ. Logs below use the same directory. Rust
1.99.1 and repository-local Node 24.22.2 are selected explicitly, with
JOINERY_TEST_NODE set to that Node; no global configuration changes.

```sh
pnpm contracts
pnpm exec vitest run src/features/studio/RelationBrowser.test.tsx src/features/studio/ObjectInspectorView.test.tsx src/ipc/command-map.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib definition -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests:: -- --test-threads=3
pnpm validate
pnpm exec playwright test tests/e2e/snowflake-sdk-browsing.spec.ts
pnpm check:rust
pnpm desktop:smoke:build
python3 scripts/native-smoke.py --runs 1
pnpm desktop:snowflake:bundle:mac
```

| Check                            | Terminal result and evidence                                                                                                                                                                                                                                                                                 |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Contracts                        | Exit 0, `contracts.log`                                                                                                                                                                                                                                                                                      |
| Focused frontend                 | 37/37 across three files, exit 0, `frontend-focused.log`                                                                                                                                                                                                                                                     |
| Focused Rust                     | 12/12, exit 0, `rust-focused.log`; eleven new definition regressions plus one existing matching case                                                                                                                                                                                                         |
| Existing SDK session regressions | 80/80, exit 0, `rust-adapter.log`; synthetic SDK, not account evidence                                                                                                                                                                                                                                       |
| Initial full validation          | Exit 0, 695 frontend / 471 standard Rust tests, 129 fixture/helper ignores, plus formatting/build/compiler checks and strict Clippy. `validate.log`; precedes the two-file UI correction                                                                                                                     |
| Final full validation            | Exit 0, 697 frontend / 471 standard Rust tests, 129 fixture/helper ignores and all strict checks. `validate-context-fix.log`; corrected snapshot                                                                                                                                                             |
| Changed browser workflow         | Final corrected 1/1 pass, exit 0, `browser-context-fix.log`; synthetic IPC, no account effects                                                                                                                                                                                                               |
| Hidden native build and smoke    | Final build and runner exit 0, 10 cleanup cycles; 4355 ms native elapsed, 98.91 MiB main-process peak RSS. `native-build-acl-fix.log`, `native-smoke-acl-fix.log`, `native-final-build.json`, `native-final-runs.json`. Guard/cleanup evidence, not representative performance or successful account GET_DDL |
| Independent review               | No unresolved material finding; bounded Tier 2 generation `2026-11-12T05:06:24Z`, 24 relied paths checked plus direct source reads. Root MCP transport unavailable; root integration uses bounded direct reads                                                                                               |

The first browser command, `pnpm test:e2e -- tests/e2e/snowflake-sdk-browsing.spec.ts`,
exits 1 and unexpectedly selects all 161 tests. All 161 fail at browser launch:
the expected default-cache headless Chromium executable is absent. No application
assertion is reached. The failure is preserved as `browser.log`. Chromium build
1248 and FFmpeg 1013 were then installed into the ignored repository-local
`.cache/playwright-browsers` using Playwright's installer (`browser-install.log`,
exit 0). No user browser profile is changed. The corrected command above uses
PLAYWRIGHT_BROWSERS_PATH pointing to that directory; `--list` confirms exactly one
test in one file (`browser-selection-pnpm.log`, exit 0). Browser checks remain
synthetic IPC UI evidence and cannot qualify account privileges or data effects.

The corrected single-test browser run (`browser-corrected.log`) exits 1 after
24.0 seconds in the test. It reaches the app, but initial catalog loading changes
to **Refresh required** before the expected relation list is displayed. The
failure is at the initial catalog assertion; definition/copy assertions have not
run. The prior validation and focused tests therefore do not establish a working
browser flow. The trace identifies the catalog starting before the SQL editor
completion callback initializes absent session runtime with unknown defaults.
The invalidation key previously distinguished absent fields from those equivalent
unknown values, falsely revoking an in-flight catalog. Normalizing only those
defaults preserves actual context, revision, owner and hold transitions. Two
new tests invoke the real refreshSessionContexts action with a denied synthetic
observation while the catalog is pending or already displayed. The corrected
focused frontend command passes **39/39** (`frontend-context-fix.log`, exit 0).
The E2E assertion is unchanged. The corrected browser run passes **1/1**
(`browser-context-fix.log`, exit 0, 18.3 seconds overall / 13.0 in the test),
exercising keyboard definition display and copy, unavailable targets, permission
denial, delayed stale definition suppression, column inspection and row preview.
Final `pnpm validate` passes on the corrected frozen snapshot: **697 frontend /
471 standard Rust tests**, **129 fixture/helper ignores**, formatting, production
build, compiler checks and strict all-target/all-feature Clippy, exit 0
(`validate-context-fix.log`). Rust source is identical to the focused adapter
checks; only the two UI files changed after those checks.
A narrow second review at generation `2026-11-12T05:19:07Z` checks four relied paths
with no recorded gaps plus direct reads and finds no material issue in the fix.

The first hidden native build passes (`native-build.log`, exit 0, optimized
compilation 4m02s). The existing LLVM rust-objcopy stripping warning is nonfatal
and remains in the log. The hidden smoke runner then exits **1**
(`native-smoke.log`): `define_relation` does not reach the expected Rust
`connectionClosed` guard. The process itself exits 0, but that does not make the
runner successful. It stops before cleanup cycles. Initial build/run manifests
are copied to `native-initial-build.json` and `native-initial-runs.json`. Native
registration was correct, but `build.rs` omitted the command from AppManifest
and `capabilities/main.json` omitted its main-window permission. The correction
adds exactly those two entries; the smoke assertion is unchanged. The 19 runtime
and test source files remain byte-identical to the corrected validation/browser
snapshot. Final `source-freeze-native-acl-fix.json` adds the two ACL inputs, for
21 files. Full validation above precedes this ACL-only correction; strict Rust
checks and actual native build/smoke are rerun afterward as recorded below. No account
connection is involved.

After the ACL fix, `pnpm check:rust` passes (`rust-check-native-acl-fix.log`,
exit 0): Rust formatting, compiler checks and strict all-target/all-feature
Clippy. Independent narrow review confirms only the existing main-window
permission is added; six relied paths have no recorded gaps plus direct reads,
generation `2026-11-12T05:19:07Z`. The unchanged runtime assertion is retained
for the corrected hidden-app build below.

An additional optional `prettier --check src-tauri/capabilities/main.json`
exits 1 (`acl-format.log`). The committed HEAD version also differs from the
formatter, confirmed without editing either version (`acl-format-baseline.json`).
This file is outside the repository's format:check selection. The two-line ACL
change preserves its existing layout; this pre-existing extra-check failure is
not hidden or repaired by unrelated formatting.

The corrected hidden build exits **0** (`native-build-acl-fix.log`, optimized
compile 3m41s); the unchanged runner exits **0** (`native-smoke-acl-fix.log`).
The actual WebView request reaches the Rust disconnected-session guard and all
**10 cleanup cycles** complete. Native elapsed time is 4355 ms and main-process
peak RSS 98.91 MiB; these are synthetic smoke observations, not representative
startup/performance or whole-process memory acceptance. The build is explicitly
recorded as dirty source at documentation HEAD `0c8433e`; all **21** final source
hashes match the frozen manifest. Copied `native-final-build.json` and
`native-final-runs.json` retain that provenance. Generated ACL verification finds
only `define_relation` under `allow-define-relation` (`native-acl-generated.json`).
No generated permission file is committed. Successful account GET_DDL, packaged
SSO and owner acceptance remain open.

The normal SDK-bearing macOS ARM64 package then passes
`pnpm desktop:snowflake:bundle:mac` from clean feature commit **`e218aef`**
(`normal-bundle.log`, exit **0**, optimized compilation 4m45s). It is a local
version/build **0.1.0** at
`src-tauri/target/release/bundle/macos/Joinery.app`. Final verified regular-file
bytes total **240,288,137** (229.16 MiB); Tauri reports 224.96 MiB
before the final ad-hoc signing step. The packaged binary SHA-256 is
`1bf65d800fb4ad6c2447d07e3379ca1a6903d9540246dc571d8df9b089161c06`.
Strict ad-hoc signature validation, **15,109** runtime entries/**zero** symlinks,
and network-guarded bundled SDK **3.4.1** startup pass with **zero authentication
and zero SQL requests**. All 21 frozen source hashes still match and the worktree
is clean after the build. `normal-bundle-build-source.json` and
`normal-bundle-artifacts.json` retain source/tree, hashes, version and check results.
The normal application is not installed or opened for SSO. Developer ID signing,
notarization, normal UI/account/restart behavior and owner acceptance remain open.

The separate [expired offline preview](snowflake-existing-schema-qualification.md)
uses frozen source `103cb45`, not this new feature. Its emit/verify exit 0 grants
no approval or account evidence. No SSO, account SQL, native owner marker-store
access, install or publication occurs in these checks. Actual Snowflake GET_DDL,
normal packaged workflow, million-row results/export and all remaining applicable
DataGrip requirements stay open; the Goal is incomplete.

### Fixed execute entry and real process signals (12 November 2026)

The dedicated command dispatches `emit`/`verify` synchronously and `execute`
through a two-worker Tokio runtime. Pure parsing rejects invalid arguments before
I/O. Execute installs SIGINT/SIGTERM streams before blocking root/envelope/approval
preparation, retains that task through observed cancellation, drains delivered
signals before starting, and revokes future admissions through the existing
sticky handle. The owner, original calls, shutdown and terminal persistence are
awaited without task abortion or background runtime shutdown. Closed summaries
separate command success, persisted terminal evidence and conservative possible
account activity. Fixture home/runtime injection is entirely `cfg(test)`, absent
from the production binary even with `test-harness` enabled.

The helper shares its parent's foreground process group. A signal only to the
Rust PID can retain the admitted callback; terminal Ctrl-C also reaches Node,
whose handler exits immediately. The group test therefore requires an unknown
original CREATE outcome rather than an invented acknowledgement. Neither form
establishes remote cancellation or rollback. See the
[command guide](snowflake-table-offline-command.md) for inputs, receipts and limits.

Five substantive process acceptance tests explicitly select an internal child
entry in the optimized library test executable. Each child has its own process
group and only temporary synthetic home/runtime/profile/output. Parents send
actual Unix signals to those owned PIDs/groups, inspect durable journal/terminal
files and reap cleanup. No test invokes the owner's native home resolver or
keychain, opens SSO or sends account SQL. The SDK is synthetic; Node and the helper
are real. The internal child protocol is not a sixth substantive acceptance test.
Actual-binary integration tests exercise only safe argument/early envelope failures
for execute; no valid native account route is used by them.

Before building changed source, the preceding source's existing optimized test
binary is run directly with the pinned Node environment and three test threads:

```text
src-tauri/target/release/deps/joinery_core-f79e8f96716fc26d snowflake_table_qualification:: --test-threads=3
```

`entry-baseline-qualification-reproduction.log` exits **0**: **263 passed /
0 failed / 6 default ignores**, **164.58 seconds**. Artifact record
`entry-baseline-reproduction-artifact.json` binds executable SHA-256
`e05593bce064110d4ae9497a079c75e5913ef7376fb58f8783e5b4ae0a9ce1ed`
to the verified 45-file prior snapshot at commit `760e996`. This is an additional
old-artifact repeat, not verification of the new entry. It does not reproduce or
resolve the preceding intermittent three-case synthetic failure; that stability
limitation remains open.

The new 47-file source snapshot is recorded separately in
`.cache/verification/connection-attempts/entry-source-freeze.json`. The commands,
terminal results, review and final identity checks below apply only to their
explicitly recorded snapshots. Logs and manifests remain ignored local evidence;
no account approval, package rebuild, installation or product gate is added.

Current-source commands use Rust **1.99.1** from its physical ARM64 toolchain,
Node **24.22.2** from the repository-local prepared toolchain, and `JOINERY_TEST_NODE`
set to that Node. No global settings change. Release checks run serially at the
command level; the qualification module uses three test threads:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --lib snowflake_table_qualification::entry::tests:: -- --ignored --skip entry_subprocess_child --test-threads=1 --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --test snowflake_table_qualification_cli
pnpm validate
```

`entry-qualification-release.log` exits **0** on the frozen 47-file snapshot:
**263 passed / 0 failed / 12 default ignores**, **156.77 seconds** after an
optimized build of **4 minutes 2 seconds**. The six new default ignores are five
substantive optimized process cases plus their internal child protocol. They are
not counted as executed by this ordinary suite; their explicit command is above.
The existing ts-rs attribute and missing LLVM `rust-objcopy` stripping warnings
remain nonfatal and are not hidden by global configuration changes.

The first explicit process run, `entry-process-acceptance-release.log`, exits
**101**: **3 passed / 2 failed / 0 ignored**, **70.19 seconds**. The full
32-phase lifecycle/fresh-process replay, invalid approval/tamper and both signals
before preparation pass. Foreground-group and PID-held-CREATE cases fail their
exact SQL-list comparison: the expected prefix selects every phase-3 entry,
including five post-CREATE session observers. Actual revocation correctly stops
at CREATE. Typed original outcome, terminal digest and settlement checks before
that assertion pass; later assertions are not counted as executed.

The correction changes only
`snowflake_table_qualification_entry_tests.rs`: find the unique typed
`SqlKind::Ddl(CreateTable)` ledger entry, assert phase 3 and exact equality to the
Plan's CREATE SQL, and use the closed ledger slice through that entry inclusive.
The strict actual-versus-expected SQL equality stays. No production behavior,
deadline, ignored-test selection or full-success ledger changes. The corrected
47-file manifest is `entry-source-freeze-corrected.json`; its sole difference
from the first snapshot is this test file. Production entry source remains the
same as in the passing 263-test run. The affected five-case process command is
rerun on the corrected snapshot.

`entry-process-acceptance-corrected-release.log` exits **0** on the corrected
snapshot: **5 substantive cases passed / 0 failed / 0 ignored**, **82.30 seconds**
after the optimized rebuild (**3 minutes 9 seconds**). Both SIGINT and SIGTERM
before preparation launch no helper/SQLite. Both PID-only signals during held
CREATE retain its successful original acknowledgement separately from later
retirement, stop at the exact CREATE prefix and persist journal/terminal evidence.
Foreground group SIGINT reaches the applied-but-pending CREATE, kills the helper,
and preserves typed unknown/retired outcome with no acknowledgement or SQL suffix,
settled cleanup, empty helper state and two spent markers. The full success case
runs all **32 phases / 393 synthetic SQL requests**, checks every validated phase
and exact ledger order, then rejects the original RUN/digest/approval from a fresh
process with different output/environment before helper or SQLite construction.
Invalid approval and changed original source fail without helper/keychain access.
The internal child protocol is excluded from the five-case count.

`entry-cli-integration-release.log` exits **0**: **9 actual-command tests passed /
0 failed / 0 ignored**, **1.81 seconds** after the release build (**4 minutes
38 seconds**). Offline emit/verify and trap zero-launch checks remain covered;
new cases reject invalid execute grammar and a missing original envelope before
approval/native-store access. Help accurately advertises the separate route
without starting it. These tests do not execute a valid account attempt.

Final `pnpm validate`, `entry-validate.log`, exits **0** on the corrected
snapshot: Prettier, contracts/TypeScript, **679 frontend tests in 57 files**,
frontend production build, **467 standard Rust tests / 129 fixture/helper
ignores**, Rust formatting, normal compiler check and strict all-target/all-feature
Clippy all pass. The ignored fixtures are not represented as executed acceptance.
All **47** corrected source hashes match after the completed commands. All
**362** local links across the seven changed documentation files resolve;
`git diff --check` and `pnpm check:git` pass. No tested code changes follow verification.

`entry-executable-artifact.json` records the actual release command at
`src-tauri/target/release/snowflake-table-qualification`, **23,395,008 bytes**,
SHA-256 `f58aa8e648e1a900aa8e6c135da0b88ad1563b3ee183d14c9c365687360428fc`,
and its corrected source-manifest digest. This is a local tested artifact record,
not build-provenance attestation or a current account approval envelope. Complete
runtime/source/compiler/executable binding and a fresh review window still
precede any account attempt. No native owner store, keychain or account is used
by the new process tests. No normal package is rebuilt or installed. The private
project Site is not published because publication remains unauthorized.

Independent bounded review (`gpt-6.1-sol`, high effort) reports no material
production finding. It identifies the foreground-group acceptance gap; the added
owned-group SIGINT test and command guide receive a second clean review. Review
covers entry/CLI/binary/parent/Cargo, fixture wrappers, actual process tests and
the guide. Fresh graph evidence at generations `2026-11-12T04:20:36Z` and
`2026-11-12T04:24:56Z` matches the reviewed paths with no recorded gaps; the
helper-launch coverage gap is read directly. Root graph calls fail with
`Transport closed`, so root uses targeted source reads and the review evidence,
without treating an empty/unavailable graph as absence. Reviews perform no
builds, writes, native owner-store access or account activity.

A separate read-only debugging review (`gpt-6.1-sol`, high effort) examines the
retained intermittent failure and current diagnostics. It establishes one factory
case stopped in phase 1 after successful construction; the other two report a
missing CREATE barrier (`RecvError` and a 120-second marker timeout), which does
not establish their failed phase or a shared cause. Phase 1 includes guard
revalidation and connection/authentication; its ordinal alone does not identify
the failing operation. The old log lacks the terminal error/original result
needed for attribution. Timeout/resource explanations remain hypotheses, and no
unsafe dispatch or lost acknowledgement is demonstrated by this log. Earlier
shorthand describing all three as failures before CREATE is too broad.

Retain the existing diagnostics. At the next qualification validation opportunity,
use the same frozen binary with the normal three-thread run. If failure recurs,
preserve its terminal record, synthetic journal/SQL prefix and helper diagnostics
before fixture cleanup, then compare a focused serial run of that same artifact.
Further passing repeats alone cannot determine the cause. This investigation
performs no edits, builds, process changes, private owner-state or account access.
Account-entry stability remains unproven.

### Fixed permanent store and retained revocation (12 November 2026, preceding integration)

`ConsumptionStore::open_native` obtains the macOS account home with bounded
`getpwuid_r` for the effective UID. The closed namespace is
`Library/Application Support/Joinery/private-qualification/table-lifecycle-consumption-v1`
under that physical home. No output, environment or configuration override selects
the store. Other platforms fail closed. Descriptor-relative bootstrap permits
safe existing shared user directories, requires private `0700` application
directories, synchronizes parent/child entries and retains native ancestor
descriptors, identities and mode checks for later consumption/revalidation.
Unsafe existing nodes fail without chmod, adoption, deletion or alternate roots.
The existing fixture store opener still requires an existing private leaf.

The new isolated tests use synthetic temporary homes only; **the real OS-account
resolver and owner's Application Support namespace are not invoked**. They check
private creation under shared `0755` parents, unsafe modes, symlink/file collisions,
UID mismatch, held-path replacement and later ancestor mode/identity drift. Four
separate test processes start together against one synthetic fixed namespace;
exactly one may consume. Reopening in another process with different output,
`HOME` and `XDG_DATA_HOME` still rejects the spent markers. The subprocess protocol
entry is ignored in the ordinary suite and explicitly selected by its parents;
it is not an extra default acceptance pass. This tests the bootstrap and storage
logic, not an executed native resolver or a complete account entry.

`PreparedAttempt::revocation` now returns a cloneable capability containing only
the shared sticky failure latch. `revoke` is memory-only, preserves an earlier
failure and cannot restore authority. Dropping that handle does not cancel.
Revocation before starting is checked before runtime/SQLite/helper construction.
If local construction has already started, its current stage may finish before
the next check. Future physical admissions remain guarded. Already admitted
operations retain their original results independently of later cancellation,
validation, shutdown and terminal storage errors. Permanent markers are never
refunded; a stale revocation handle cannot rewrite finished evidence.

Five new envelope/factory regressions check shared first-failure behavior,
zero launch before startup and two retained CREATE paths. One pauses after the
returned CREATE but before recording completion. The other holds the actual
helper's synthetic SDK CREATE callback after the synthetic effect, then revokes
and releases it. Both drop the caller's waiter, retain the original succeeded ACK
in completion/journal/terminal evidence, and require the exact captured SQL prefix
to remain unchanged. No ALTER, DROP, replay or cleanup SQL is newly authorized by
revocation. These use actual pinned Node/helper/Studio paths with a synthetic SDK;
they do not prove Snowflake data effects or remote cancellation.

The two workers and independent reviewer use **gpt-6.1-sol/high**, with bounded
file ownership; no requested selection is unavailable. The lead integrates and
runs builds/checks serially. Review reports no material findings in the six changed
Rust files. Parent graph calls return `Transport closed`; workers/reviewer obtain
coverage with no recorded issue, latest review generation
`2026-11-12T03:32:19Z`, and read final source directly. This bounded evidence is not
repository completeness proof.

The initial 45-file source manifest is
`.cache/verification/connection-attempts/attempt-boundary-source-freeze.json`.
Commands use the pinned README toolchains and `JOINERY_TEST_NODE`:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --test snowflake_table_qualification_cli
pnpm validate
```

`attempt-boundary-qualification-release.log` preserves the initial run: exit
**101**, **262 passed / 1 failed / 6 ignored**, **155.23 seconds** after a
**2 minute 58 second** optimized build. The pending SDK callback case wrongly
requires `completionRecordError` to be null. Its actual original CREATE ACK is
retained; `Policy::record_operation_completion` correctly reports the already
latched cancellation even when storing that original succeeds. The test now
requires the cancellation, retained succeeded/retired ACK in journal and terminal,
and unchanged SQL prefix. Independent review confirms the correction; production
outcome handling is unchanged. Two narrow test-only `dead_code` annotations mark
native entry/OS lookup intentionally unused by isolated tests; they do not invoke
the owner's storage or suppress other checks. The final source manifest,
`attempt-boundary-source-freeze-final.json`, records all 45 files separately.
Only consumption annotations and the corrected factory test differ from the
initial manifest; the complete qualification suite is rerun on this pre-Clippy
snapshot. The later test-only loop correction is recorded below.

`attempt-boundary-qualification-final-release.log` exits **0** on that snapshot:
**263 passed / 0 failed / 6 default ignores**, **161.97 seconds** after
the optimized build (**2 minutes 54 seconds**). All eleven new substantive
regressions pass, including six native-store cases and five envelope/factory
revocation cases. The child protocol runs only through its explicitly selecting
subprocess parents. Other default ignores are not counted as executed acceptance.

`attempt-boundary-process-release.log` exits **0**: **7 actual-command tests
passed / 0 failed / 0 ignored**, **1.41 seconds**. Emission/restoration, copied
executable and tamper rejection, safe arguments and zero trap launches remain
covered on the same pre-Clippy snapshot. The known nonfatal missing-LLVM `rust-objcopy`
stripping and ts-rs attribute warnings remain visible; no global toolchain or
configuration changes are made.

The first full validation, `attempt-boundary-validate.log`, exits **101** at
strict Clippy's `needless_range_loop` in the new native-store collision test.
Frontend checks/build, 679 frontend tests and 467 standard Rust tests already pass;
this is not a complete validation pass. The correction enumerates `PARTS` and
uses the same item directly instead of indexing it; all test cases/assertions and
production behavior remain unchanged. No Clippy lint is suppressed.
`attempt-boundary-clippy-corrected.log` then exits **0** for the complete strict
all-target/all-feature command (**27.01 seconds**). The final checked 45-file
manifest is `attempt-boundary-source-freeze-checked.json`; only that test file
differs from the preceding snapshot. Qualification, actual-command and full
validation checks are repeated against the checked source.

The first repeat on that checked snapshot,
`attempt-boundary-qualification-after-clippy-release.log`, exits **101**:
**260 passed / 3 failed / 6 ignored**, **196.13 seconds**. The existing
journal/terminal fault case stops at phase 1 instead of reaching CREATE at
phase 3; the new pending-completion waiter closes early and pending-callback
waiter times out. The run uses `--test-threads=3`. This is an unresolved repeat
failure at this point, not a final qualification pass. Later process/validation
steps in that serial check sequence do not run. Diagnostic follow-up must expose
the earlier terminal failure before assigning a cause or changing fixture timing;
production admission deadlines and evidence assertions remain unchanged.

The diagnostic-only follow-up keeps a mutable reference to the retained waiter
while awaiting the CREATE barrier. Early owner completion now reports its
terminal record immediately; a failed barrier collects that record for up to
ten seconds before failing. The successful path still revokes and drops the
waiter, releases the original callback/completion, and checks the same original
ACK and exact SQL prefix. The existing journal/terminal fault assertion also
prints the early terminal record. Independent review finds no material issue;
these diagnostics do not themselves fix the intermittent failure.

The new 45-file manifest is `attempt-boundary-source-freeze-diagnostic.json`;
only `snowflake_table_qualification_factory_tests.rs` differs from the checked
snapshot. `attempt-boundary-factory-diagnostic-parallel.log` exits **0**:
**9 passed / 0 failed / 1 ignored**, **7.57 seconds**, using three test threads
after a **2 minute 48 second** optimized build. On the same unchanged source,
`attempt-boundary-diagnostic-full-parallel.log` exits **0**: **263 passed /
0 failed / 6 ignored**, **160.17 seconds**, again with three threads.

The failure is not reproduced by those diagnostic runs; **its cause remains
unconfirmed**. A bounded literal audit across Rust source and relevant synthetic
JavaScript finds no parent environment/process-setting mutation; child
`Command.env` and `Command.current_dir` remain confined to their subprocesses.
This narrows an interaction hypothesis without proving absence of other races or
resource pressure. The earlier failed repeat remains evidence, not a resolved
defect. No production deadline is widened, no regression is newly ignored, and
this increment does not establish reliable account entry or replacement readiness.

Final source-specific checks on the diagnostic snapshot:

- `attempt-boundary-factory-diagnostic-serial.log`: exit **0**, **9 passed /
  0 failed / 1 ignored**, **21.37 seconds**, one test thread. This control also
  does not reproduce the earlier failure.
- `attempt-boundary-process-diagnostic-final.log`: exit **0**, **7 actual-command
  tests passed / 0 failed / 0 ignored**, **1.47 seconds**. Offline emission,
  restoration/tamper checks and zero trap launches remain covered.
- `attempt-boundary-validate-diagnostic-final.log`: complete `pnpm validate`
  exits **0**, including formatting, TypeScript/contracts, frontend build,
  **679 frontend tests / 57 files**, **467 standard Rust tests / 129 fixture
  ignores**, Rustfmt, compiler check and strict all-target/all-feature Clippy.
  These standard counts do not include the separately selected qualification
  tests above. Ignored fixtures are not claimed as executed acceptance.
- All **45** frozen source hashes still match after the completed checks.
  Local links in all seven changed documents resolve (**353 targets**);
  `git diff --check` and `pnpm check:git` pass before staging. The final source
  receives bounded independent review with no unresolved material finding.

No UI behavior changes in this increment, and no new browser/native UI or
package acceptance is claimed. The known nonfatal LLVM stripping and ts-rs
warnings remain visible. Application validation passing does not explain the
earlier qualification failure or qualify actual account effects. Next work must
retain its diagnostics and the open stability limitation while completing the
fixed execute entry and interruption handling.

The command remains offline-only: the fixed native store and revocation capability
are not yet connected to an account execute route. Whole-process SIGINT/SIGTERM,
runtime teardown and SIGKILL/power-loss evidence are not qualified by detached
task tests. No account, native app-state, new package, install or publication
activity occurs. Source-specific preceding package/performance checks are not
claimed as rerun. Fresh exact account/billing approval for the table-only plan
under `SANDBOX.BILLIE_TESTING`, all applicable DataGrip product workflows and owner
hands-on acceptance remain open; the Goal remains active.

### Dedicated offline table command (12 November 2026)

The new `snowflake-table-qualification` binary requires the existing
`test-harness` feature, but starts neither the HTTP harness nor an async runtime.
Its narrow facade dispatches only `emit`, `verify` and help. Unknown commands
(including `execute`), duplicate/unknown/missing options, invalid windows and
relative/non-UTF-8 paths fail with a closed JSON error before artifact/output I/O.
Emission alone generates a fresh Plan. The actual current executable is selected
internally; there is no executable/SQL/source-list/limit/policy override.

Emission captures the fixed envelope, rejects source/output overlap and reserves
one exclusive 0700 `plan-<RUN>` directory. Closed descriptor-relative names create
0600 single-link `execution-envelope.json` and `plan.json` files, with no adoption,
overwrite or cleanup. Success includes exact content/identity readback and file/
directory synchronization after all writes. Envelope and Plan bounds remain
16 MiB / 64 KiB. Partial failures retain output. Verification reads the original
private envelope and explicit digest, recaptures separately supplied roots and
requires full canonical equality without regenerating a RUN. It can inspect an
expired window offline; it does not grant execution authority.

The process regressions invoke the actual binary against disposable synthetic
source, profile and runtime artifacts. Trap Node/compiler files must never run.
They check exact persisted bytes/hashes/build recipe/current-executable path and
hash, cross-process unchanged RUN/digests, read-only verification, source/profile/
envelope tamper rejection, copied-executable rejection with identical bytes and
permissions, path/argument failures and safe stdout/stderr. Eight storage tests
cover private modes/bounds, same-RUN collisions/concurrency, links and changed
identity/content, and retained partial evidence. These are local filesystem/
process tests; the new command sends no authentication, SQL or account requests.

The inventory explorer uses gpt-6-luna/high; the storage worker and independent
reviewer use gpt-6.1-sol/high. Ownership is bounded; the lead integrates and runs
checks serially. No selection is unavailable. Review identifies a non-UTF-8 output
path that could persist before JSON receipt serialization panicked. The parser
now rejects unencodable paths before I/O, with a real-process regression; this
was a source-review finding, not a separately reproduced pre-fix binary failure.
The reviewer confirms the fix and added actual-executable checks; no material
finding remains. Parent graph calls report `Transport closed`, so current direct
source reads are used. The storage worker reports no recorded coverage gap at
generation `2026-11-12T02:54:30Z`; that bounded result is not completeness proof.

The initial checked source identities are recorded in
`.cache/verification/connection-attempts/offline-command-source-freeze.json`
(44 source/build files). Use the pinned README toolchains and `JOINERY_TEST_NODE`.
The focused all-target development check in `offline-command-check-initial.log`
exits **0**, **1 minute 22 seconds**; later test-only additions and the path guard
are verified by the final commands below, rather than attributed to that check.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --test snowflake_table_qualification_cli
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
pnpm validate
```

`offline-command-process-final-release.log` exits **0**: **7 passed / 0 failed /
0 ignored**, **1.59 seconds** after the optimized build (**5 minutes 27 seconds**).
Every actual-binary process case passes, including the reviewed UTF-8 and copied
executable regressions. No trap executable runs. The profile fields remain absent
from emitted/verified stdout; the private envelope intentionally contains them.

`offline-command-qualification-final-release.log` exits **0**: **252 passed /
0 failed / 5 default ignores**, **152.49 seconds**. All eight new emission storage
tests and the preceding qualification/factory/error-route cases pass on that
frozen source. The five default ignores are not counted as executed acceptance.

The initial full application run, preserved as
`offline-command-validate-initial-clippy.log`, exits **101** at strict Clippy's
`chunks_exact_to_as_chunks` lint in the new parser. Frontend formatting/contracts/
types/tests/build and **467 standard Rust tests / 129 fixture ignores** already
pass; that is not a complete validation pass. The sole source correction changes
`chunks_exact(2)` to `as_chunks::<2>().0`. The prior argument-count guard makes
the remainder empty; independent review confirms equivalent pairs. No lint is
suppressed. Final identities are recorded separately in
`offline-command-source-freeze-final.json` (44 files); only the CLI parser differs
from the earlier qualification run. The affected actual-command suite and full
application validation are rerun on this final source. The 252-test qualification
result above remains evidence for its explicitly preceding source, not a claimed
rerun of the whole suite after the parser-only correction.

`offline-command-process-after-clippy-release.log` exits **0** on the final source:
**7 passed / 0 failed / 0 ignored**, **1.02 seconds**. Every real-process case is
rerun, including argument pairing, tamper/identity rejection and zero trap launches.
`offline-command-validate-final.log` exits **0**: **679 frontend tests / 57 files**,
**467 standard Rust tests / 0 failed / 129 fixture ignores** across 22 summaries.
Formatting, contracts/types, production frontend build, Rust formatting/check and
strict all-target/all-feature Clippy pass. No ignored external/native acceptance
is silently counted as run. All **44 final frozen source hashes match** after
completed checks. The known `rust-objcopy` missing-LLVM strip warning, ts-rs serde
attribute warnings and frontend chunk/timing warnings remain visible; no global
toolchain/configuration repair or lint suppression is made. Git diff/identity and
all seven edited documents' local link targets are checked before commit.

The [command guide](snowflake-table-offline-command.md) supplies local emission/
verification steps. The execute route, independently fixed permanent marker root
and cross-process permanent consumption remain open. The preceding complete
packaged-runtime performance checks and ignored full lifecycles apply only to
their recorded source; none is counted as rerun here. No account activity,
normal UI acceptance, package rebuild, installation, push, merge or publication
occurs. The canonical private Site is not published because the owner forbids
publication. Fresh exact account/billing approval under `SANDBOX.BILLIE_TESTING`,
all applicable DataGrip capabilities and owner hands-on acceptance remain required.
The Goal is active and incomplete.

### Fresh consumed factory and persistent terminal owner (12 November 2026)

`AttemptStorage` reserves one exclusive persistent `attempt-<RUN>` directory,
separate new `workspace`/`helper-state`, and exclusive 0600 journal/terminal
files. Directories remain 0700. Strict original physical paths, effective owner,
ancestor safety, held/named identities, single-link files and expected terminal
bytes are checked. Transfer and terminal persistence are one-shot, including
failure. Terminal encoding is limited to 1 MiB, 64 levels and 65,536 nodes;
partial write and file/directory-sync fault tests preserve evidence and refuse
retry. No production Drop/delete/reopen/resume/refund route is added.

The factory rejects output overlap with permanent markers and reviewed
artifacts before reservation. It reserves output before consuming approval,
then revalidates the consumed identity and builds only the bound runtime with
fresh state. Runtime rejection precedes SQLite or helper launch. The generic
Studio opener receives only the newly allocated empty workspace. Profile save,
reload and connection preparation use normal Studio APIs; fixed Staging/Confirm
and privacy settings are rechecked. A deny-all secret provider counts calls and
never initializes/delegates to OS credential storage. Test-only fixture injection
reuses pins, not the supplied Studio/runtime/state object, and requires the same
consumed root before loading fresh state.

One detached owner retains construction, normal runner execution, shutdown and
blocking terminal persistence. The receipt keeps original outcomes, completion
errors, shutdown, settlement, final integrity, journal result and terminal result
separate. Mutations and boundaries remain in terminal evidence even after a
later journal/storage error. Metadata/read detail remains in the journal to
bound terminal size. `executionSuccessful` describes execution only; terminal
durability requires the separate successful persistence receipt/digest.
Persistent workspace/state is not removed after failure or incomplete shutdown.

The storage worker owns the attempt/consumption files, the test worker owns the
factory regressions, and the lead owns integration. An independent bounded
correctness review has no material finding. All three use gpt-6.1-sol/high;
no unavailable model or fallback is involved. The initial optimized focused run
exits **101**: **4 passed / 1 failed / 1 ignored**, **7.55 seconds**. The failure
occurs after original CREATE ACK and error-retention assertions: the test wrongly
requires an SDK destroy log after intentionally revoking journal/transport.
An unchanged-binary backtrace reproduction exits **101**, **7.10 seconds**,
confirming `records -> assert_factory_cleanup`. These runs precede final source.

The corrected failure test requires local reaping/settlement, empty local helper
state, the sole new connection/state path and fixed persisted profile, without
claiming server logout. If a destroy callback exists it must be exactly one
successful callback. The full successful lifecycle still requires that callback.
Independent follow-up review confirms the distinction; no product repair is
needed for this test expectation. The frozen 38-source manifest is
`factory-owner-source-freeze.json` under `.cache/verification/connection-attempts/`.

Use README's pinned toolchains and explicit `JOINERY_TEST_NODE`. The final checks
are serial:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_factory_complete_lifecycle_outlives_dropped_waiter -- --ignored --test-threads=1
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_runtime:: -- --test-threads=3
pnpm validate
```

`factory-owner-qualification-final-release.log` exits **0**: **244 passed /
0 failed / 5 default ignores**, **155.42 seconds**. All 17 new active cases pass:
nine filesystem tests, two store-separation tests and six factory tests. The
CREATE failure case exercises actual Node/helper with a synthetic SDK and keeps
the original ACK after both journal and terminal failure. The five ignores
include the new detached-owner test run separately by the command above.

`factory-owner-detached-final-release.log` exits **0**: **1 passed / 0 failed**,
**55.61 seconds**. The caller is dropped at phase three; the retained owner still
finishes all **32 phases / 393 synthetic SQL requests**, each original completion
matching its validated evidence and the complete physical ledger. Terminal
execution success, zero credential-provider calls, exact new persisted profile,
one connection using fresh state, empty retired helper state and one successful
SDK destroy callback are checked after the detached terminal persistence signal.
This is the new minimal bound-runtime factory fixture, not a rerun of the full
packaged-runtime performance gate or real account behavior.

`factory-owner-runtime-final-release.log` exits **0**: **22 passed / 0 failed /
0 ignored**, **0.11 seconds**. The final source remains frozen; release logs retain
the existing nonfatal missing-`libLLVM.dylib` debug-strip and `ts-rs` warnings.

`factory-owner-validate-final.log` exits **0**: **679 frontend tests / 57 files**,
**467 standard Rust tests / 0 failed / 129 fixture ignores** across 21 summaries.
Formatting, contracts/types, production frontend build, Rust format/check and
strict all-target/all-feature Clippy pass. The existing frontend chunk/timing
warnings remain visible. All **38 frozen source hashes match** after completed
checks. Git diff/identity checks pass before the local commit. No material review
finding remains, including the failure-route cleanup follow-up.

No changed UI workflow or normal account path is claimed. Actual Node/helper
fixtures use the synthetic SDK, isolated SQLite and test-owned filesystem state.
The dedicated binary, independently fixed permanent marker location and
cross-process entry acceptance remain open. Existing complete packaged-runtime
performance checks apply only to their recorded source; the four preceding
performance ignores are not counted as rerun here. No Snowflake authentication,
SQL, package rebuild, installation, push, merge or publication occurs. The
canonical private Site is not published because the owner forbids publication;
repository evidence remains current. Fresh exact account/billing approval,
normal packaged acceptance, all applicable DataGrip capabilities and owner
hands-on acceptance remain required. The Goal is active and incomplete.

### Exact runtime and reserved journal construction (12 November 2026)

The harness-only crate-private runtime constructor selects exactly the supplied
root with supported-host enforcement and compiled pins. It does not append a
packaged suffix, discover an environment/development runtime or fall back to a
different tree. Existing resource validation and the separate/private state
requirement remain enforced. Five synthetic tests cover valid decoys, invalid
roots, overlap, modes, symlinks, compiled-pin rejection and zero launch/state
allocation. Complete consumed-inventory validation remains the factory's duty.

Policy can now take ownership of an already exclusively reserved Journal for
the same Plan and validated frozen profile. It accepts only the untouched
header-only owner, rejects prior events or a failed append, and checks held/named
storage identities, permissions, single-link status and length. It never reopens,
resumes, truncates or repairs the journal. Four tests cover normal first admission,
creation collision, mismatched Plan/profile, used and failed-but-header-only
journals, truncation/replacement/symlink and changed file/directory modes. The
ordinary create route keeps its validation-before-file-creation behavior.

Independent bounded review has no material finding in the journal/policy delta;
the lead reviewed the separately implemented runtime delta. This does not remove
the documented same-user in-place rewrite limitation. Four changed source hashes
are recorded in `factory-input-source-freeze.json` under
`.cache/verification/connection-attempts/`.

Use the pinned toolchains and `JOINERY_TEST_NODE` from README. Verification is
serial, with the exact commands:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_runtime:: -- --test-threads=3
pnpm validate
```

The initial focused `factory-input-journal-release.log` has terminal exit **0**,
**4 passed / 0 failed**, **0.05 seconds**. That run precedes the additional
failed-append assertion and is not final-source evidence for it. The final
source is frozen before the full commands above. No earlier ignored full-runtime
or packaged-artifact pass is counted as verification of this new source.

`factory-input-qualification-final-release.log` has terminal exit **0**:
**227 passed / 0 failed / 4 default ignores**, **148.93 seconds**.
`factory-input-runtime-final-release.log` also exits **0**: **22 passed /
0 failed / 0 ignored**, **0.08 seconds**. All nine new cases pass: four owning
policy/journal tests and five runtime-selection tests. Existing actual-helper/
synthetic-SDK lifecycle and fault cases complete successfully, including the
long callback-loss case. The four full-runtime/performance ignores are not rerun
for this source. Existing nonfatal release `rust-objcopy`/`libLLVM.dylib` and
`ts-rs` warnings remain visible in the logs.

`factory-input-validate-final.log` has terminal exit **0**: **679 frontend tests
in 57 files**, **467 standard Rust tests / 0 failed / 129 fixture ignores** across
21 Rust summaries. Formatting, contracts/types, production frontend build, Rust
format/check and strict all-target/all-feature Clippy pass. All four frozen
source hashes still match after the completed commands. Scoped documentation
formatting and `pnpm check:git` also pass before the local commit. No material
review finding remains; the suggested failed-but-header-only journal case is
included in the final source and passes.

The fixed private factory, attempt/workspace/state allocation, terminal evidence
inside the detached owner, permanent consumption-store placement and dedicated
binary remain incomplete. These constructors add no account or normal UI
workflow. No SSO, account SQL, package rebuild, installation, push, merge or
publication occurs. Fresh table-only execution/billing approval for
`SANDBOX.BILLIE_TESTING`, actual data effects, all applicable DataGrip workflows
and hands-on acceptance remain required.

### Fixed qualification profile and private envelope input (12 November 2026)

The private seven-field connection input now validates the account against the
external-browser driver's lowercase hostname derivation. A new profile function
saves a fresh UUID through the ordinary Staging group and checks the returned
and persisted profile against fixed Confirm, reviewed DML/DDL, external-browser,
verified TLS/443 and privacy settings. It rejects existing profiles and
passphrase/temporary-result-storage state. Synthetic SQLite save/reopen tests
record zero calls to the injected rejecting secret store; Production still
normalizes to PasswordGated and rejects an unapproved profile change.

The fixed policy is serialized into the execution envelope and recomputed during
restoration. It cannot be supplied through the connection input. Fifteen
individually rehashed policy-field mutations preserve canonical ordering and are
rejected; missing policy is also rejected. The dedicated private envelope reader
accepts at most 16 MiB, retaining the existing physical-path, owner, exact 0600,
regular-file, single-link and identity checks. Tests exercise the exact limit,
one-byte overflow, growth, truncation, replacement, unsafe ancestors and links.
The smaller 64 KiB general reader ceiling, 16 KiB profile and 4 KiB approval
bounds remain unchanged. File restoration passes on a valid snapshot larger
than 64 KiB and rejects an insecure file.

Two defects were reproduced before their fixes. Logs are under
`.cache/verification/connection-attempts/`:

- `fixed-profile-account-mismatch-reproduction.log`: terminal exit **101**,
  **0 passed / 1 failed**, **0.01 seconds**. The old envelope accepts a reviewed
  account inconsistent with the driver's hostname-derived account. Strict input
  validation now rejects it before consumption.
- `fixed-profile-host-case-reproduction.log`: terminal exit **101**,
  **0 passed / 1 failed**, **0.04 seconds**. Changing only hostname case creates
  another permanent RUN marker for the same Plan/account/target. The key now
  follows the driver's lowercase host while exact reviewed profile bytes stay
  bound to the execution envelope. The regression requires a different envelope
  hash, the same RUN key, rejected second consumption and exactly two markers.
  This establishes hostname-case consistency, not equivalence of distinct DNS
  aliases or remote Snowflake account identities.

The first combined run, `fixed-profile-envelope-combined-release.log`, was
interrupted without a terminal summary; its process/session was confirmed gone.
It is not counted as a completed pass. The final source is checked with:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
pnpm validate
```

`fixed-profile-envelope-final-release.log` has terminal exit **0**:
**223 passed / 0 failed / 4 default ignores**, **165.93 seconds**. The 19 added
regressions comprise six reader, nine profile and four envelope cases. Existing
actual-helper/synthetic-SDK lifecycle and failure cases also pass. The callback
loss case takes over 60 seconds and completes successfully; it is not a hung or
restarted test. Use the pinned toolchains and `JOINERY_TEST_NODE` from README.
The release build reports the existing `rust-objcopy` missing `libLLVM.dylib`
debug-stripping warning and `ts-rs` serde-attribute warnings; compilation and the
test command nevertheless complete. No global toolchain/configuration repair is
performed by this increment.

`fixed-profile-validate-final.log` also has terminal exit **0**: **679 frontend
tests in 57 files**, **467 standard Rust tests / 0 failed / 129 fixture ignores**
across 21 Rust summaries. Formatting, regenerated contracts/types, frontend
production build, Rust format/check and strict all-target/all-feature Clippy pass.
All seven frozen source hashes still match after both completed commands. The
changed documentation sections pass scoped Prettier checking; historical sections
are preserved. `pnpm check:git` passes before the local commit.

Independent bounded reviews cover the reader/profile/envelope and final
hostname-case correction, with no unresolved material finding. Profile username
length also matches the sequencer's 1,020-byte boundary. Seven changed source
hashes are recorded in `fixed-profile-source-freeze.json`.

This is offline qualification integration. The factory must still allocate an
exclusive fresh workspace and inject a keychain-free secret store; the profile
function alone does not prove construction provenance. The fixed account binary,
permanent-store placement, runtime construction, retained output ownership and
ordinary plan-token integration remain incomplete. Review identifies a concrete
cleanup constraint: failed Studio shutdown may leave owners unsettled, so the
future attempt workspace/state must be retained rather than automatically
deleted. Terminal evidence must be written inside the detached owner, before
its receipt returns, to survive a dropped waiter.

The four ignored runtime/performance cases are not rerun for this source. Earlier
full-runtime and normal package results apply only to their recorded source.
No account activity, SSO, normal application UI, package rebuild, installation,
push, merge or publication is claimed. Fresh table-only approval for
`SANDBOX.BILLIE_TESTING`, actual effects, million-row account behavior, all
applicable DataGrip workflows and hands-on acceptance remain required.

### Normal macOS package after performance repair (12 November 2026)

`pnpm desktop:snowflake:bundle:mac` exits **0** from clean source
`f5d02e4cfccd1597e19c34e02ebce02c7e29961d`, which contains the reviewed source
commits `db18e03` and `a05cfae`. Log:
`.cache/verification/connection-attempts/real-content-normal-bundle-final.log`;
structured artifact evidence: `real-content-normal-bundle-final-evidence.json`
in the same directory. All 13 tested source hashes still match after packaging.

The local normal bundle is
`src-tauri/target/release/bundle/macos/Joinery.app`, version **0.1.0**, **ARM64**,
**224.93 MiB**. Its executable is **32,480,896 bytes**, SHA-256
`f7e5ffbe70f4273651dd1b3a72e340d578e5d1678af428daf9b58bda4c2d73f6`.
Resource verification passes with **15,109 descendant entries / zero symlinks**;
this count excludes the root, unlike the 15,110-entry inventory below. The staged
resource input still has five dependency symlinks; the assembled bundle resolves
them. Manifest SHA-256:
`74e1dfac7b3346d163fad04879dc7f558c73111d2eac6230a904a98e34621f1f`.

The normal bundle's local ad-hoc signature verifies. Its genuine SDK-bearing
helper starts and exits on empty stdin under the Node network guard, emitting
exactly one protocol hello; **zero authentication requests / zero SQL requests**.
This is helper startup, not normal application UI or real-account acceptance.
No installation, existing application-state access or account activity occurs.
Developer ID/notarization, installation/update, normal packaged SSO/restart/UI,
actual Snowflake effects and owner acceptance remain open. The private Site hub
is not published by this milestone: the available update path publishes, which
the owner's no-publish instruction does not authorize. Repository status is the
durable local evidence; nothing is pushed, merged or published.

### Complete-content performance acceptance (12 November 2026, local verification)

This increment adds explicitly selected optimized offline measurements. The
standalone envelope test inventories the unchanged normal packaged runtime,
actual fixed repository scopes, running optimized executable, resolved pinned
compiler and synthetic private profile. It captures, restores exact canonical
bytes, consumes a synthetic offline approval and performs three full
revalidations. The separate Studio fixtures copy every packaged runtime entry,
replace only fixture SDK routing/source verification before capture, and execute
the same captured runtime through the real Node/helper/Studio path. All mutable
controls, logs, state, workspace and journals live outside the inventory.
Neither fixture authenticates or imports the genuine account SDK.

Use the pinned toolchains from README and `JOINERY_TEST_NODE` pointing at the
prepared Node 24 executable. Run these commands serially in this checkout, with
the existing normal macOS ARM64 bundle present; the explicit ignored fixtures
fail rather than substitute a minimal runtime when required artifacts are absent:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib envelope_real_content_optimized_capture_restore_consume_and_revalidate -- --ignored --test-threads=1 --nocapture
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner_consumed_full_runtime_s -- --ignored --test-threads=1 --nocapture
```

Independent review found premature disposal of the test-only admission timer:
Rust async field capture retained the success flag rather than the Drop owner.
The reproduction exits **101**, **7 passed / 1 failed**, **135.24 seconds**, in
`real-content-guard-reproduction.log`; a successful admission recorded zero
successes. Explicit whole-value capture fixes the timer lifetime. Existing tests
now also require zero completed measurements while the scanner is paused and
before an unpolled future is dropped. The final delta review has no outstanding
material finding. The timer repair changes no query behavior or production deadline.

Results under `.cache/verification/connection-attempts/`:

- `real-content-guard-fixed-release.log`: exit **0**, **8 passed**, **1.99 seconds**.
- `real-content-separated-fixture-release.log`: exit **0**, **1 passed**,
  **1.78 seconds**, actual synthetic SDK helper startup with external output paths.
- `real-content-envelope-release.log`: exit **0**, **1 passed**, **12.71 seconds**.
  Full runtime **15,110 entries / 202,538,012 bytes**, inventory SHA-256
  `4f768186a648e14233d18d31941b96c20818176c2c075c8fdecb5f146df4e699`.
  Capture **3,774,548 microseconds**, restore **2,527,007**, consumption
  **2,529,387**; full revalidation **1,272,013 / 1,251,448 / 1,242,941**.
  Canonical envelope **4,182,125 bytes**; running test executable **32,342,208
  bytes**. Runtime contents were not executed in this standalone measurement.

The initial full-runtime S=1 workflow exits **101**, **0 passed / 1 failed**,
**153.88 seconds**, in `real-content-runner-s1-release.log`. It completes four
phases and 67 SQL requests; **113 admissions / 112 successful / 1 failed**,
maximum **2,036 ms**, total **141,790 ms**, consumption-to-settlement **144,895
ms**. Phase five retains its original cancelled read and a separate physical
sequence failure; helper/transport/scanner settle and final guard evidence is
recorded, while persistence correctly reports the retained failure. These are
failed acceptance results, not a successful partial lifecycle.

Investigation identifies the unrelated 15-second read collector cutoff: phase
five requires 11 SQL requests plus result controls, so complete artifact checks
can exhaust that cutoff while individual admissions remain within three seconds.
The reviewed correction now bounds read completion by the consumed attempt
remaining absolute monotonic/window deadline, without renewing 900 seconds per
read. The 15-second terminal cleanup drain remains separate; unconsumed synthetic
fixtures retain their existing 15-second read wait. No sequencing authority is
relaxed to clear the secondary cancellation error.

Corrected-source focused checks:

- `real-content-slow-read-fixed-release.log`: exit **0**, **1 passed**, **27.04
  seconds**. The actual helper receives one explicitly logged synthetic ownership
  read callback delayed **16 seconds**. All five original phase outputs, including
  the completed read, and the exact closed SQL prefix are retained. Only the
  deliberate journal-permissions fault after phase five fails the attempt;
  cleanup settles and no cancellation/retirement is recorded. Artifacts in this
  focused fixture are synthetic except the actual running executable; it is not
  full-runtime or account evidence.
- `real-content-envelope-deadline-fixed-release.log`: exit **0**, **13 passed /
  1 ignored**, **0.64 seconds**. Includes nonrenewal of the original monotonic
  deadline, a shorter wall-window bound, sticky expiry and memory-only behavior.
- `real-content-envelope-deadline-release.log`: exit **0**, **1 passed**, **12.27
  seconds**. Complete unchanged packaged runtime on the corrected source;
  capture **3,468,194 microseconds**, restore **2,478,836**, consumption
  **2,489,122**, and full checks **1,235,120 / 1,264,182 / 1,240,648**.

The corrected S=1 full-runtime workflow passes in
`real-content-runner-s1-deadline-fixed-release.log`: exit **0**, **1 passed**,
**676.38 seconds**. It completes all **32 phases / 393 SQL requests** with
**527 admissions / 527 successful / 0 failed**, maximum **1,983 ms**, total
**662,413 ms**, consumption-to-final-persistence **667,423 ms**. Original call,
cleanup, completion recording, final guard revalidation/recording and persistence
all succeed; scanners and transport settle. The exact copied runtime used by
Studio has **15,112 entries / 202,526,102 bytes**, including the frozen synthetic
SDK routing; inventory SHA-256
`52050a2e9c65f15c696b9bf388d03e21999fa9972597c3524c908e55f5478c1b`.
No account authentication occurs. This is the complete single-schema synthetic
workflow result, not actual Snowflake performance or S=32 evidence.

S=32 fails in `real-content-runner-s32-deadline-fixed-release.log`: exit **101**,
**0 passed / 1 failed**, **422.33 seconds**. Nine phase outputs and **241 SQL
requests** complete. **310 admissions / 309 successful / 1 failed**, maximum
**3,000 ms**, total **408,751 ms**; consumption-to-settlement **412,197 ms**.
The phase-ten original call reports a closed connection after the process
admission deadline interrupts the guard. Completion recording/persistence retain
the guard cancellation; final integrity failure is recorded and scanners,
transport and shutdown settle. This is not a 900-second lifecycle failure and
does not prove whether the complete S=32 workflow fits that budget. All nine
source hashes in `real-content-performance-deadline-source-freeze.json` still
match after both runs.

The diagnostic-only standalone run in `real-content-envelope-profiled-diagnostic.log`
passes (exit **0**); macOS `sample` also exits **0**. The five-second profile
`real-content-envelope-sample-diagnostic.txt` reports 3,576 active-worker samples:
software SHA-256 **1,045**, `openat` **1,081**, reads **835**, `fstatat` **199**
and directory reads **196**. These sampled proportions locate costs, not exact
stage durations or the cause of the individual timeout. The pinned
[sha2 0.10.9 implementation](https://docs.rs/sha2/0.10.9/src/sha2/sha256.rs.html)
gates its ARM64 acceleration behind `asm`; the current feature set uses software.
The targeted candidate enables that feature only for macOS ARM64, retaining its
CPU detection/software fallback and every inventory check. The lockfile adds only
`sha2-asm` 0.6.4. A standard million-'a' digest vector exercises the actual
multi-read inventory hash independently of the selected backend. Focused and
complete performance reruns and final combined validation now pass as recorded
below. The earlier frozen source results do not establish the changed
candidate's performance. The initial
accelerated checks now pass:

- `real-content-accelerated-sha-vector-release.log`: exit **0**, **1 passed**,
  **0.00 seconds** after the optimized build. The actual file inventory matches
  the independent SHA-256 million-'a' reference vector.
- `real-content-accelerated-inventory-release.log`: exit **0**, **15 passed /
  1 ignored**, **0.01 seconds**. The ignored packaged-runtime measurement is
  covered separately by the complete-envelope test below.
- `real-content-envelope-accelerated-release.log`: exit **0**, **1 passed**,
  **8.78 seconds**. Capture **2,693,193 microseconds**, canonical **3,188**,
  restore **1,729,561**, consumption **1,707,628**; full checks
  **851,619 / 851,611 / 865,807**. The normal packaged runtime remains
  **15,110 entries / 202,538,012 bytes**, with the exact unchanged inventory
  SHA-256 `4f768186a648e14233d18d31941b96c20818176c2c075c8fdecb5f146df4e699`.
  This confirms content/digest compatibility, not complete admission latency.

The accelerated S=32 rerun passes in
`real-content-runner-s32-accelerated-release.log`: exit **0**, **1 passed**,
**711.52 seconds**. It completes all **32 phases / 610 SQL requests** with
**751 admissions / 751 successful / 0 failed**, maximum **1,419 ms**, total
**699,838 ms**, consumption-to-final-persistence **703,735 ms**. Original calls,
cleanup, completion recording, final guard revalidation/recording and persistence
all succeed; scanners and transport settle. The exact copied runtime used by
Studio contains **15,112 entries / 202,526,103 bytes**, inventory SHA-256
`d7fc41715299496eca56f30aba3a45589d706d64e0536b545f4e6ad66dcf0f4a`.
The bound optimized test executable is **32,363,152 bytes**, inventory SHA-256
`7e678271da9bf615c811d69b0f6bcab268df382b1e6515af34ef50552ada7c40`.
No account authentication or genuine SDK import occurs. All 13 source hashes
match after this terminal result.

The same-source S=1 rerun passes in
`real-content-runner-s1-accelerated-release.log`: exit **0**, **1 passed**,
**502.77 seconds**. It completes all **32 phases / 393 SQL requests** with
**525 admissions / 525 successful / 0 failed**, maximum **1,206 ms**, total
**491,414 ms**, consumption-to-final-persistence **495,200 ms**. Original calls,
cleanup, completion recording, final guard revalidation/recording and persistence
all succeed; scanners and transport settle. The exact copied runtime used by
Studio contains **15,112 entries / 202,526,102 bytes**, inventory SHA-256
`395f28d8db0c8c525fc90b8dc238571a89b9d8df78436a8f219052ab75395bdb`.
The executable is the same as S=32. All 13 frozen source hashes still match after
both terminal results. Neither run authenticates or imports the genuine account
SDK. The complete optimized module now passes in
`real-content-qualification-combined-release.log`: exit **0**, **204 passed /
4 default ignores**, **147.36 seconds**. The complete-envelope and full-runtime
S=1/S=32 cases were explicitly selected and passed separately above; the remaining
runtime-only ignore is covered by the full-envelope inventory, not claimed as a
separate current-source execution. Final `pnpm validate` passes in
`real-content-validate-final.log`: terminal exit **0**, **679 frontend tests /
57 files**, **467 standard Rust tests / 0 failed / 129 fixture cases ignored**,
plus formatting, contracts/types, production frontend build, Rust format/check and
strict all-target/all-feature Clippy. The 13-path frozen source still matches
after validation. The ignored account/native workflows are not executed by this
command. Source is committed locally as `db18e03` (ARM64 hashing) and `a05cfae`
(consumed read deadlines and full-runtime regressions). No source changes follow
these successful checks. The separate normal package check now also passes as
recorded above.

Independent bounded review finds no blocker in the target-specific feature or
new digest vector. Feature unification affects other `sha2` 0.10.9 consumers on
macOS ARM64, including its CPU-detected SHA-512 path; it is not an inventory-only
switch. Existing consumers therefore remain covered by final application checks.
The process writer's three-second bound includes semaphore acquisition,
blocking-pool scheduling, full scan, policy waiting and journal sync. Queueing
before writer selection is outside that span. If the changed candidate still
fails, collect bounded test-only stage timings before altering traversal.
The candidate's 13-path freeze is
`real-content-performance-accelerated-source-freeze.json`; it also records the
unchanged inventory implementation. Independent Python `hashlib` confirms the
million-byte reference digest, and scoped Rust formatting / `git diff --check`
pass. These checks alone do not establish the performance gate.

The final cumulative read-only review also finds no material integration issue
across all 12 changed source/fixture/dependency paths. It confirms the absolute
consumed read deadline is computed before preparation/execution, preserves the
original monotonic bound and shorter wall window, and leaves terminal drains
separate. Fixture capture and execution use the same runtime root; immutable SDK
routing changes precede capture and mutable controls remain external. Residual
coverage limitation: no dedicated integration case expires an already-active
consumed read against a short remaining deadline. Unit expiry/nonrenewal, the
16-second real-helper read and existing cancellation/drain cases provide
composed evidence, not that direct scenario. The review does not supply runtime
or account acceptance. No reviewer edits, tests or builds occurred.

A separate read-only documentation review checks the new product-readiness
overview against README, acceptance and linked design contracts. No misleading
readiness/account claim is found. Its scope pointer now explicitly retains
AI/agents, diagrams, data search/comparison, Git/Local History and other applicable
baseline capabilities beyond the twelve requested areas. Local matrix links
resolve and scoped formatting preserves the historical remainder. This review
adds no execution evidence.

Read-only architecture review resolves the next factory policy decision: create
a fresh isolated Staging/Confirm profile, with a recomputed fixed policy declaration
bound in the canonical envelope. Use the ordinary Staging group, profile saving,
safety preparation and plan authorization; verify the saved policy before launch.
Read no existing application profile and never downgrade Production/PasswordGated.
The runner's existing passphrase rejection means this first route cannot qualify
production password gating. Exact offline policy/tamper/group/opt-in/TLS/target and
ordinary-token negatives remain implementation work. The reviewed seven-field
connection input stays unchanged. No new approval ceremony is added; fresh exact
plan/billing approval is still the existing external gate after concrete review.

A targeted 12 November official-documentation recheck confirms the existing
grid-editing constraint: Snowflake [object references](https://docs.snowflake.com/en/sql-reference/references#objects-identified-by-name)
identify names and can authorize a replacement or swapped object when the encoded
roles retain privileges. This does not establish an atomic original-table pin.
The existing bounded design finding remains; it is not proof that no supported
mechanism exists and does not justify enabling grid/CSV writes. No account test
or broader baseline refresh is implied.

A single revalidation measurement does not establish the complete admission or
workflow gate. Initial measured source is frozen in
`real-content-performance-source-freeze-final.json`; deadline corrections require
a new freeze and rerun. No account activity, normal package rebuild,
installation or publication is represented by these tests.

### Consumed runner admission and final evidence (12 November 2026)

The retained internal runner now accepts the non-cloneable consumed envelope by
value. Its guard compares exact canonical Plan bytes/digest and frozen seven-field
profile with the owning policy. The same guard wraps physical admissions and
metadata/runtime publication. A full filesystem revalidation precedes helper
launch/authentication and each physical request; synchronous callbacks perform
only memory/time checks. No filesystem scan occurs under process or policy locks.

Authentication is bounded to 120 seconds; each complete guard wait, including
single-slot acquisition, scan and durable admission, is bounded to 30 seconds.
The existing process writer retains its stricter three-second deadline. The
15-minute monotonic execution deadline and wall window remain checked at
publication. Cancellation, timeout or an unpolled dropped check revokes the
attempt. A retained blocking worker owns the sole scan permit until its actual
OS work returns; late completion cannot admit SQL. An overdue worker may be
reported unsettled rather than represented as cancelled.

Review found that a final integrity error retained only in `RunnerReceipt`
could disappear if its caller dropped. The corrected runner records exactly one
`executionEnvelopeFinal` journal event after cleanup, containing the original
integrity result and scanner settlement, before final evidence persistence.
Storage status is separate from the retained policy failure. Original helper/
local reports and typed operation completions, including successful CREATE
acknowledgements, remain unchanged. Interrupted/failed final writes reserve the
original event and cannot replace it with success. This is filesystem consistency
evidence, not a simulated power-loss durability guarantee.

Independent bounded source review is clean, including the final-record repair.
Tier2 graph coverage reports matching metadata/no recorded issue for the changed
paths; direct source reads cover the complete deltas. The process driver's known
partial parse at lines 1314–1318 was read directly. Graph metadata is best-effort
and does not prove exhaustive correctness.

Root-owned optimized Guard/consumed-runner/module tests, explicit packaged-runtime
inventory timing and final `pnpm validate` pass. Release mode
measures real deadlines rather than inferring them from earlier debug timings.
New tests use actual envelope consumption and the running Rust test executable;
source/runtime/compiler/profile inputs are synthetic. Runner tests exercise the
actual Studio/Node/helper route with a synthetic SDK. They cover preauth drift,
post-CREATE drift, dropped waiters and stalled authentication, not account effects.

Initial optimized checks are retained under
`.cache/verification/connection-attempts/`:

- `consumed-runner-guard-release.log`: exit **0**, **8 passed**, 1.86 seconds.
- `consumed-runner-studio-release.log`: exit **101**, **2 passed / 2 failed**,
  20.93 seconds. Preauth drift and actual stalled-authentication cleanup pass;
  post-CREATE drift and dropped-waiter tests exit before their phase-three pause,
  reporting only a closed oneshot. These are failed checks, not successful
  CREATE coverage.
- `consumed-runner-create-diagnostic-release.log`: exit **101**, **0 passed /
  1 failed**, 2.39 seconds, after adding test-only early-exit receipt diagnostics.
  Both startup SQL statements succeed, then a queued result-stream credit is
  rejected after the query terminal has already arrived. The original phase-one
  failure is `Physical Snowflake control does not match its qualification owner`;
  final integrity failure and cleanup are retained durably. The slower awaited
  admission exposes a terminal/control race. The reviewed correction is now
  frozen; final corrected checks are recorded below. No production deadline
  or control payload grammar was relaxed.
- `consumed-runner-final-policy-release.log` and
  `consumed-runner-final-policy-cancel-release.log`: exit **0**, **3 + 1 passed**,
  0.03/0.01 seconds. Final fsync, original ACK/first-error retention, unsettled
  scans, duplicate rejection, poisoned journal and interrupted write pass.
- `consumed-runner-packaged-runtime-release.log`: exit **0**, **1 passed**, 3.28
  seconds, running the existing optimized test binary directly while the next
  diagnostic test source was being prepared. Actual unchanged packaged runtime:
  **15,110 entries / 202,538,012 bytes**, SHA-256
  `4f768186a648e14233d18d31941b96c20818176c2c075c8fdecb5f146df4e699`;
  capture **2,185 ms**, verify **1,081 ms**. This measures runtime inventory only,
  not the complete envelope, real account latency or worst-case deadline margin.
  No Node/helper/SSO is executed by this test.

The correction introduces an explicit inactive-only reservation when the exact
query completes while its queued credit/discard awaits admission. It requires
the same generation, logical identity, query ticket, physical target and phase;
the later control ticket establishes completed-target provenance, not a new
global monotonic-ticket rule. It consumes the candidate once without advancing
SQL, creating an active query or metadata debt. Pending local completion blocks
phase advancement. A helper terminal is rejected while retaining its original
report; only the exact local inactive receipt with no actual wire ID can settle
the reservation. Process already suppresses the stale write after admission.

Three sequence regression functions cover credit/discard completion and nine
identity/grammar/replay negatives. Actual helper adapter tests cover both the
existing after-durable-admission ordering and a new deterministic pause before
inner admission. The new pause models the guard await; it does not itself scan
the envelope. These cases check the exact 14-SQL prefix, skipped candidate,
original query receipt and durable settlement. Independent bounded review finds
no material issue in the frozen correction.

Corrected frozen-source focused checks exit **0**:

- `consumed-runner-inactive-sequence-release.log`: **13 passed**, 0.02 seconds,
  including the three new sequence functions.
- `consumed-runner-inactive-adapter-release.log`: **2 passed**, 0.62 seconds,
  with both deterministic actual-helper orderings.
- `consumed-runner-studio-final-release.log`: **4 passed**, 20.94 seconds,
  including the two CREATE cases that failed before the correction. These use
  actual Studio/helper processes with a synthetic SDK and consumed synthetic
  artifacts; they do not establish account effects.

The corrected complete qualification module exits **0**, **200 passed / 0 failed /
1 ignored**, **160.50 seconds**, normal three-thread execution,
`consumed-runner-qualification-final-release.log`. It includes all 179 preceding
cases and 21 new default cases. The default ignore is the separately executed
packaged-runtime inventory benchmark. Focused counts overlap the module and must
not be added to it.

The first full validation, `consumed-runner-validate.log`, exits **101** at strict
Clippy after passing **679 frontend / 467 standard Rust tests**, with **129
fixture cases ignored** across 21 Rust summaries. Clippy rejects a new test's
literal `Err(...).unwrap_err()` assertion at policy-tests line 407. The correction
names the original `AppError`, clones it into the same error result and compares
the durable JSON against that named value directly. It changes no production
code or assertion coverage. The complete optimized qualification rerun exits
**0**, **200 passed / 0 failed / 1 ignored**, **162.32 seconds**, in
`consumed-runner-qualification-post-clippy-release.log`, on the final source
hashes below. Formatting, contracts,
TypeScript, production frontend build and Cargo check had passed before Clippy;
the first full validation is not reported as successful.

The final `pnpm validate` exits **0**, `consumed-runner-validate-final.log`, on the
same frozen source: **679 frontend / 467 standard Rust tests pass**, with **129
fixture cases ignored** across 21 Rust result summaries. Formatting, contracts,
TypeScript, production frontend build, Cargo fmt/check and strict all-target/
all-feature Clippy pass. Existing ts-rs attribute/frontend chunk warnings remain
nonfatal. Optimized Cargo checks also retain the existing nonfatal rust-objcopy
stripping warning for missing libLLVM; it does not prevent test execution. The
ignored database/native/account cases and earlier browser/package checks retain
their separate revision/scope. No changed UI workflow or external database is
claimed by these qualification-only checks.

The code is committed locally as `f80c8c5`. All 12 committed source blobs match
the frozen hashes below. No source changes follow the final checks.

Documentation checks: the complete migration guide and all newly added sections
pass Prettier; `git diff --check` passes. Whole-file formatting of the five other
documents also fails on their unchanged HEAD versions, so historical formatting
is preserved. Scoped results are in `consumed-runner-docs-scoped-format.log`;
the initial whole-document failure is `consumed-runner-docs-format-initial.log`.

```sh
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification::guard::tests -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib sequence_local_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib owning_adapter_real_helper_stale_credit_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner_consumed_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification -- --test-threads=3
cargo test --release --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_table_qualification::inventory::tests::inventory_packaged_runtime_complete_content_capture_and_verify -- --ignored --exact --nocapture
pnpm validate
```

The fixed account executable/private manifest/approval/output lifecycle is still
absent. Construction of Studio/runtime/profile exclusively from the bound roots
remains to be implemented and verified, as does complete runtime/envelope latency
against the existing process limit. A fresh concrete `SANDBOX.BILLIE_TESTING`
plan and one-attempt billing approval remain necessary before account activity.
No private connection, SSO, billable SQL, normal UI/package change, installation,
push, publication or merge occurs in this increment. Live effects, packaged SSO,
million-row performance, all applicable DataGrip workflows and owner acceptance
remain gates. The Goal remains active/incomplete.

The next performance acceptance must use actual repository fixed scopes, current
optimized executable, pinned compiler and unchanged full packaged runtime for
the offline Envelope benchmark. Separately, S=1/32 routes must inventory the exact
full runtime copies actually used by Studio, with synthetic SDK/verifier changes
finished before capture. Current fixture logs/controls live inside its helper
tree; they must move to separate private mutable output rather than be excluded
from inventory. Runtime copies with synthetic replacements are not genuine SDK
or sealed account-factory evidence. Record every complete scan/admission duration,
including policy fsync, and consumption-to-final-persistence time without changing
deadlines. Even excluding controls and other work, the 900-second limit allows
less than about 2.29 seconds per SQL for S=1 (393 SQL) or 1.48 seconds for S=32
(610 SQL); actual allowed averages are lower. No complete-envelope or real-content
route performance pass is claimed yet.

Frozen reviewed/tested Rust source SHA-256 values (committed as `f80c8c5`):

| Source under `src-tauri/src/drivers/`                   | SHA-256                                                            |
| ------------------------------------------------------- | ------------------------------------------------------------------ |
| `snowflake_table_qualification.rs`                      | `8dad18a728072babfcf1db1ec1069998149c3512385a29e611fdbe5d38c6a5ea` |
| `snowflake_table_qualification_envelope.rs`             | `f7caf792bb76b9053ae01adf812719b8058d0302486ee3dabfe03348a0fdffdd` |
| `snowflake_table_qualification_envelope_tests.rs`       | `b6d302f24b438e6586a9d079d2c911bfcab53be75f9e8f4ee63cc4d937871e56` |
| `snowflake_table_qualification_policy.rs`               | `1ec3973d65ee688d11bf72e81d260b76c2cf7df64087942409d1d576327d263a` |
| `snowflake_table_qualification_policy_tests.rs`         | `0534048d900741ea698776ce72c0858467f3bd45c8101992e07f340eaf5a0650` |
| `snowflake_table_qualification_policy_adapter_tests.rs` | `d4b8472697431b293989065ae2a444bd3e7e16b8dc30e17bd2971e884421bc74` |
| `snowflake_table_qualification_runner.rs`               | `bfc595c68e0c6fc4d8ba497ac9f74d66ee06c5e80dba774b5f0acbcd3d8df30f` |
| `snowflake_table_qualification_runner_tests.rs`         | `8bd326a68347969c7888431c5829693554d9be1f826d1c20f5ddabc0c7f61d78` |
| `snowflake_table_qualification_sequence.rs`             | `67a47915406e1ec1fc539727a58ba3680c185295054e858e3de25ee6b65900c8` |
| `snowflake_table_qualification_sequence_tests.rs`       | `4fe89fb99a6e3ccc28c08cb8e95b969d69d37b45cd4348a0e97fae7c37e3f4b8` |
| `snowflake_table_qualification_guard.rs`                | `1bf8495b45af757d087f392f9ac3b29c38e85fcfeab84fc6879ffc06f2511042` |
| `snowflake_table_qualification_guard_tests.rs`          | `8275b124e12791c267dcd24270d65ca1729bad023fac853e3ede0f0e6e84c464` |

### Offline artifact binding and one-use consumption (12 November 2026)

This harness-only increment adds an offline envelope for the existing closed
32-phase Plan. It does not add an account entry, authentication authority or a
normal application workflow. The envelope binds the fixed Rust/helper/vendor and
build-input scopes, the complete runtime tree, executable and compiler contents,
an exact seven-field private connection profile, the Plan, proposed limits and
an admission window of at most 30 minutes. Restoration recaptures trusted caller
roots and requires identical canonical bytes rather than trusting snapshot paths,
limits or hashes. Byte binding does not attest that the executable was built from
those sources, freeze loaded memory or defeat an adversarial same-user process.

The inventory includes root/empty directories, lossless relative path bytes,
modes, full file lengths and SHA-256 contents. Descriptor-relative no-follow
walks reject links, hard-linked files, unsafe ownership/modes and special nodes;
named/held identities and directory listings are rechecked. Fixed caps are 64
levels, 32,768 entries, 512 MiB aggregate regular-file bytes and 256 MiB per file.
Physical input paths are bounded to 64 KiB/64 components. The profile is bounded
to 16 KiB and exactly mode 0600. Canonical envelope output is bounded to 16 MiB;
that output bound is not a total-process-memory guarantee.

A distinct approval scope/hash plus the actual current executable is required
before consuming local state. Consumption creates the RUN marker first, then
the execution-hash marker with exclusive no-follow creation, file and directory
syncs and readback. Once the RUN marker is created, later partial failure,
failed launch or a dropped capability cannot refund it; a changed execution hash
or window cannot reuse that RUN. The sealed capability retains original file
handles and rechecks identity, modes, link counts and exact marker contents after
the store drops. Post-consumption artifact/window/deadline failures latch and
cannot be cleared by restoring bytes. These are local filesystem guarantees,
not proof of account execution or power-loss durability.

Independent source review found and resolved: the need to retain marker handles
and contents through later checks; protection of all path ancestors and raw
alias rejection; checking the observed named stamp before opening; and canonical
mutation tests that preserve field order instead of failing only on JSON key
ordering. Final bounded source review is clean. The complete runtime scan is
outside process/policy locks here; placement before authentication/every physical
SQL and after execution remains unimplemented. The bound 120-second auth and
30-second admission waits are proposed values, not demonstrated runtime limits.

Preserved initial failures in `.cache/verification/connection-attempts/`:

- `execution-envelope-inventory-focused.log`: exit **101** before tests because
  Plan canonical bytes were assigned to a String; corrected with checked UTF-8.
- `execution-envelope-inventory-repaired.log`: exit **101**, **13 passed /
  1 failed / 1 ignored**, 0.06 seconds. APFS rejects a deliberately invalid UTF-8
  fixture name with EILSEQ. The test accepts only that exact filesystem rejection,
  always checks lossless raw encoding and path aliases, and still performs the
  real invalid-name capture on filesystems that support it. A separate existing
  file supplies the wrong-kind check; no production inventory check is relaxed.

Final focused logs exit **0**:

- `execution-envelope-inventory-final.log`: **14 passed / 1 ignored**, 0.06
  seconds. The separate existing-file wrong-kind assertion was subsequently
  strengthened and is included in the complete frozen-source run.
- `execution-envelope-consumption-focused.log`: **14 passed**, 35.29 seconds;
  includes one envelope-consumption test as well as the 13 consumption functions.
- `execution-envelope-focused.log`: **11 passed**, 35.01 seconds; includes one
  preceding wire test alongside the ten envelope tests. Focused counts overlap.
- `execution-envelope-packaged-runtime-inventory.log`: **1 passed**, 17.54
  seconds. This explicitly runs the normally ignored macOS inventory test against
  the existing normal bundle's runtime, without executing Node, helper or SSO.

The complete frozen-source qualification module exits **0**, **179 passed /
0 failed / 1 ignored**, **358.80 seconds**, normal three-thread execution,
`execution-envelope-qualification-all.log`. It includes all 142 preceding cases
and 37 new default cases. The separately executed packaged-runtime case is the
single default ignore; focused counts overlap this total.

**`pnpm validate` exits 0** on the same frozen source,
`execution-envelope-validate.log`: **679 frontend / 467 standard Rust tests pass**,
with **129 fixture cases ignored** across 21 Rust result summaries. Formatting,
contracts, TypeScript, production frontend build, Cargo check and strict
all-target/all-feature Clippy pass. Existing ts-rs attribute and frontend chunk-
size warnings remain nonfatal. Ignored database fixtures and earlier browser/
native/package evidence retain their separate scope; none is newly claimed by
this validation. The seven committed index blobs match the hashes below; code
is committed locally as `6b95a4d`. No source changes follow these checks.

That actual runtime contains **15,110 entries including root** and **202,538,012
regular-file bytes**. Its complete inventory SHA-256 is
`4f768186a648e14233d18d31941b96c20818176c2c075c8fdecb5f146df4e699`.
Capture took 9,218 ms and verification 8,082 ms in the unoptimized Rust test build.
These are offline debug scan timings, not account latency or release-entry
performance evidence. The future per-SQL guard must be measured against its
retained deadline before account approval.

The new tests cover actual cross-process exclusive-creation races (the Rust test
binary), partial-write/sync failures, unsafe/replaced paths, complete dependency
content drift, canonical snapshot mutations, private profile bounds, exact
approval scope, actual test-executable binding, no marker creation on bad
approval/executable, RUN reuse under a changed envelope, and sticky failure.
Synthetic files supply source/compiler/profile fixtures. They neither access the
owner's private profile nor launch authentication.

The existing normal macOS bundle is unchanged. No account activity, UI change,
new package build, installation, push, merge or publication occurs here. The next
gate is a fixed executable entry that composes these primitives with the retained
runner, private manifest/approval/output handling and actual guards around
authentication/SQL. Only then can a fresh concrete plan for new tables under
`SANDBOX.BILLIE_TESTING` be reviewed for execution and possible billing.

Frozen reviewed source hashes:

| File under `src-tauri/src/drivers/` | SHA-256 |
| --- | --- |
| `snowflake_table_qualification.rs` | `297f4b36dc059294e864d66a0db1de8861a319c2fc10b1734dca8ffd5468dcb2` |
| `snowflake_table_qualification_inventory.rs` | `4f1d99d7e77fdd21de593f497b502344f6850070a53464ba3cb2ae5431290589` |
| `snowflake_table_qualification_inventory_tests.rs` | `d74575a7761b52108dab1b9de0aff27e007e14780ad906c464ca00fad7591896` |
| `snowflake_table_qualification_consumption.rs` | `b08ac4e9b3de8638cf83c76bbec3c1cac2ecabb0b134990adf4ee4f24b6b8620` |
| `snowflake_table_qualification_consumption_tests.rs` | `9ef308003c091287e4bd4d9101e67b1c20631ab5519a444113ddf28c77dad523` |
| `snowflake_table_qualification_envelope.rs` | `45359f923bb9474c1aac8bf0dc4cefef852b86253572d78dd2595208f74c9028` |
| `snowflake_table_qualification_envelope_tests.rs` | `c9dfebea61d721ebc6ce88347afc90dbed65aabef3d871293aa6e236eb3ce9de` |

```sh
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib inventory_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib consumption_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib envelope_ -- --test-threads=3
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_table_qualification::inventory::tests::inventory_packaged_runtime_complete_content_capture_and_verify -- --ignored --exact
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification -- --test-threads=3
pnpm validate
```

### Retained runner and aggregate payload bound (12 November 2026)

The private shared runner now owns the same 32 Studio phases used by all existing
synthetic route/fault fixtures: original completion recording precedes phase
validation, and cleanup retains local ownership after the waiting handle drops.
Its constructor is test-only, compares exact Plan canonical bytes/hash with Policy
before creating the same-owner attachment, and exposes no account entry, SQL/
target selector, private-profile lookup or authentication authority.

Capture overflow/missing terminal drain now returns typed failures instead of
panicking. Review identified an original-error retention gap in the first
extraction: durable failed-read records could contain only the replacement capture
error. The correction adds typed `incompleteRead` completions with the capture
error, original operation error, actual bounded event prefix and cleanup error
separately. Both late-completion edges are covered: a later shutdown suffix and
a successful second drain after the first failed drain cannot upgrade a failed
collection. The actual late `Finished` remains recorded. Collector-backed tests
use the same completion/helper code as execution with a real local policy/journal;
they do not claim an actual SDK timeout or crash/power-loss durability.

Qualification transport now limits aggregate **all-frame body bytes to 32 MiB**,
including handshake/control/metadata/malformed frames, excluding four-byte headers.
Declared lengths reserve nonrefundable budget before allocation/read, while the
report counts actual consumed body bytes, including partial frames. Interrupted
framing cannot resume. The independent 64 MiB wire, 1 MiB frame, 32,768 frame-count
and 64 KiB stderr limits remain. This is a conservative upper bound on result
content rather than a semantic-only counter; normal million-row queries do not
use this qualification attachment.

Initial failed checks are retained under `.cache/verification/connection-attempts/`:

- `retained-runner-payload-focused.log`: exit **101** before tests, missing
  `TransportPolicy` test import after extraction; an unused test import is also
  removed. This is a compilation failure, not a passing transport check.
- `retained-runner-payload-repaired-focused.log`: exit **101**, **26 passed /
  1 failed**, **0.09 seconds**. The new scripted child emits an invalid connect
  `result` event instead of `terminal`, retiring before its cap check. Correcting
  that test-only protocol typo preserves the original cap/ACK assertions and
  deadlines; production transport is unchanged by that repair.
- `retained-runner-and-payload-qualification-all.log`: exit **101**, **141 passed /
  1 failed**, **347.44 seconds**, normal three-thread execution. The new dropped-
  core test counts nonexistent `phaseValidated` rather than actual `validatedPhase`,
  obtaining zero instead of 32. All four new misspelled event assertions are
  corrected; the factory assertion also checks actual admission `wireId` records.
  No runner code, deadline or fixture response changes in this repair.

`retained-runner-payload-final-focused.log` exits **0**, **27/27 passed**,
**0.10 seconds**, covering constructor bounds, exact multi-frame boundary,
header-only overflow before body allocation/read, partial/malformed accounting,
cancelled-frame non-reuse and prior ACK retention after policy failure. The new
real-child case uses the actual Supervisor with scripted wire frames, not the
production helper/SDK: an earned DML ACK survives the subsequent header-only
overflow, original terminal/local observations are retained, and actual child
reaping/task settlement complete. `retained-runner-dropped-core-repaired.log`
exits **0**, **1/1 passed**, **20.94 seconds**: the waiting core handle drops after
CREATE, yet the same owner records all 32 original completions and validated
phases, exact SQL ledger and local helper settlement. Focused counts overlap the
complete module coverage.

The complete corrected-source qualification module exits **0**, **142/142 passed**
in **335.80 seconds**, normal three-thread execution,
`retained-runner-and-payload-qualification-repaired-all.log`. This includes all
135 preceding cases, six new runner regressions and one new durable-policy
regression, both S=1/32 success routes with exact 393/610 physical SQL ledgers,
all seven-mutation fault loops and the corrected assertions. Focused counts
overlap this total. No production source changes between the preceding 141/1 run
and this corrected-assertion run.

**`pnpm validate` exits 0** on the same frozen source,
`retained-runner-and-payload-validate.log`: **679 frontend / 467 standard Rust
tests pass**, with **129 fixture cases ignored** across 21 Rust result summaries.
Formatting, contracts, TypeScript, production frontend build, Cargo check and
strict all-target/all-feature Clippy pass. Existing ts-rs attribute and frontend
chunk-size warnings remain nonfatal. Separate real database/native/browser/package
evidence below keeps its recorded earlier scope; these ignored fixtures are not
claimed as executed by this application check.

Independent bounded source review is clean after the retention corrections.
Root verifies the final event-name/admission assertion repair against the actual
emitted journal shapes. The cap is committed locally as `963a455`; the shared
runner and durable incomplete-read evidence as `68fff80`. All seven index blobs
match the reviewed/tested source hashes before commit; no code changes after
these checks. Frozen SHA-256 values identify the reviewed source:

| File under `src-tauri/src/drivers/` | SHA-256 |
| --- | --- |
| `snowflake_table_qualification.rs` | `749347b79aa90aac4c835390456e7c0af961b0e1674a6ff5359ee62514c0188b` |
| `snowflake_table_qualification_policy.rs` | `053b0ddc96061c6531a94679e7537572e384cab1c506a596ca4cf47726565008` |
| `snowflake_table_qualification_policy_tests.rs` | `15d8d6d358b0cb783416ad8eb7da4ad745e66368162ba89bc996a038742010fb` |
| `snowflake_table_qualification_runner.rs` | `c3b6070f05f15bacbdbda15ee520e7d6721de44ea87e8bc916269e119ca2bc2e` |
| `snowflake_table_qualification_runner_tests.rs` | `dd4b1128ac5e7f5c86d62afecd3e55691a21c0aab1734933f52eacb3ee49f865` |
| `snowflake_qualification_transport.rs` | `47d43007c9356c7fb6c6674e1c6c78a72431db66e88694330ebd29f3a08eb50b` |
| `snowflake_qualification_transport_tests.rs` | `950faf17f6024a51a5bc24dfc1c484b3b366efe80e8e219cab0c859d6dc9d025` |

```sh
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_transport_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner_dropped_core_handle -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
pnpm validate
```

No UI or normal package behavior changes in this harness-only increment. The
preceding normal macOS bundle remains present/executable; its executable SHA-256
is rechecked as `e556602265ca8ddb3004a7c8306d1416d80cd8124c8add8b4c323c5bf6e68265`.
It is not rebuilt, launched or installed here; its earlier package/startup checks
retain their original scope. No new UI/native/real-PostgreSQL workflow is claimed.
No saved private profile, SSO, account SQL or possible billing is exercised here.

Source/binary/full-runtime/private-profile binding, one-use consumption and the
fixed account entry remain incomplete. Bind a fully inventoried zero-link tree;
the earlier normal bundle supplies one while the default stage has five links.
Only after those checks pass, prepare a concrete executable plan for fresh tables
under owner-permitted `SANDBOX.BILLIE_TESTING` and request fresh execution/billing
approval. Live account effects/interruptions, packaged SSO, million-row account
performance, all applicable DataGrip capabilities and owner acceptance remain
required. The Goal remains active/incomplete; nothing is pushed, published,
merged or installed. Earlier evidence below retains its historical scope.

### Closed-plan restoration and context drift (11 November 2026)

Seven focused identity tests pass: three `closed_snapshot_restore_`, two
`sequence_with_` and two `owning_create_with_` cases. Logs are
`closed-plan-restore-focused.log`, `closed-plan-sequence-focused.log` and
`closed-plan-owner-focused.log` under `.cache/verification/connection-attempts/`.
The internal restoration API reads at most 64 KiB, extracts only RUN, recompiles
the private closed factory and requires exact canonical bytes and SHA-256.
It preserves the original table, SQL, S=1/32 expanded ledgers and journal-header
identity. Rehashed typed-copy tampering, malformed/duplicate/noncanonical JSON,
invalid digest and invalid profile cases fail. Invalid profiles create no journal.
Restoring a definition supplies no approval, freshness or execution authority.

Four full-route drift tests pass all eight scenarios in **59.13 seconds**, normal
three-thread execution, `full-route-context-drift-focused.log`: initial census
overflow and reversed original row order; same-count schema-set replacement before
CREATE and during catalog refresh; initial role and warehouse mismatch; and later
secondary-role and search-path drift. The last search-path case adds an entry;
it does not establish full-route order-only permutation coverage. These fixtures
launch the actual helper with a synthetic SDK, alter successful wire results and
exercise production decoding and policy. They verify the exact stopping SQL
prefix, original typed failure/read report, retained earlier CREATE success/ACK
where applicable, no invented unknown outcome, no replay or cleanup DROP, and
actual local child/task settlement and private-root removal.

Independent bounded source reviews find no material issues in either increment.
Plan identity is committed locally as `d96be31`; drift coverage as `376622b`.
All seven frozen source hashes still match after validation, packaging and these
commits; commit metadata does not change the tested source bytes.
Frozen SHA-256 values below identify the reviewed and tested driver files:

| File under `src-tauri/src/drivers/` | SHA-256 |
| --- | --- |
| `snowflake_table_qualification.rs` | `07e19a529241f122c300acf4b098927706a300add49dd066ff7e959879b5c450` |
| `snowflake_table_qualification_tests.rs` | `e0da0582d63adc1bbb88a5addea9295a7714792ff7f2937493a8a48224b4851d` |
| `snowflake_table_qualification_sequence.rs` | `afcc74a8c68660dc43cfa6890d5522cc04da2ad5313a5830c86a0c12b4a23e2d` |
| `snowflake_table_qualification_sequence_tests.rs` | `2a3d50f669bc7d773b796cbcfb14c5a2bb0dce89a28eb431df36c03a7b3612af` |
| `snowflake_table_qualification_policy.rs` | `b9446b0429e8e9aab4914ff7027bdc1001062c79aa381c160ae2c43d088f2158` |
| `snowflake_table_qualification_policy_tests.rs` | `c3c0ead8517e3e8f77380f232af7eb589f18b37b463787e4f3ccbec597c8ddf3` |
| `snowflake_table_qualification_runner_tests.rs` | `0a1e44ea4161471971b297e5f631c5c570b3a625a0c801e49ca70aeac657668d` |

The complete combined-source qualification module exits **0**, **135/135 passed**
in **324.08 seconds**, normal three-thread execution,
`drift-and-closed-plan-qualification-all.log`. This includes all 124 preceding
cases plus the seven identity and four drift tests, including both complete
census-extreme routes and all rejection/interruption/storage-failure loops.
Focused counts overlap this module total. **`pnpm validate` exits 0** on the same
frozen source, `drift-and-closed-plan-validate.log`: **679 frontend / 467 standard
Rust tests pass**, with **129 fixture tests ignored**. Formatting, contracts,
TypeScript, production frontend build, Cargo check and strict all-target/all-
feature Clippy pass. Existing ts-rs attribute and chunk-size warnings remain
nonfatal. Earlier full-validation results below describe the preceding increment.

`pnpm desktop:snowflake:bundle:mac` exits **0** on the same frozen source,
`drift-and-closed-plan-macos-bundle.log`, producing the normal **224.93 MiB**
arm64 `src-tauri/target/release/bundle/macos/Joinery.app`, version/build **0.1.0**.
Strict ad-hoc signature verification and resource verification pass. The bundle
resource inventory reports **15,109 entries / zero symlinks**; guarded real-SDK
3.4.1 helper startup passes with **zero authentication / SQL requests**. The guard
is a Node network guard, not an OS sandbox or packaged account/desktop workflow.
Executable SHA-256 is
`e556602265ca8ddb3004a7c8306d1416d80cd8124c8add8b4c323c5bf6e68265`;
runtime manifest SHA-256 is
`74e1dfac7b3346d163fad04879dc7f558c73111d2eac6230a904a98e34621f1f`;
Node SHA-256 is
`b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a`.
The pre-existing missing-`libLLVM.dylib`/`rust-objcopy` debug-stripping warning
recurs but does not fail the build or verifiers. No global toolchain changes,
installation, Developer ID signing, notarization or account qualification occur.

```sh
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib closed_snapshot_restore_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib sequence_with_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib owning_create_with_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner_drift_ -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
pnpm validate
pnpm desktop:snowflake:bundle:mac
```

The execution-boundary review identifies remaining work before an executable
account plan: extract the same retained owner from its synthetic fixture, bind
and exclusively consume one source/binary/runtime/private-profile envelope, and
enforce the proposed **32 MiB aggregate semantic-result cap**. The existing
64 MiB transport and 64 KiB per-read capture bounds do not establish that aggregate
cap. The default staged runtime contains five dependency symlinks; the newly
verified bundle's runtime has zero. The executable boundary still needs to bind
and fully inventory its chosen zero-link runtime rather than silently accept
the stage's links.
No account connection, saved private-profile access, authentication, account SQL
or billing occurs in this increment. Live effects, account interruption, packaged
SSO, million-row account performance, all applicable DataGrip workflows and owner
acceptance remain open.

### Combined startup and child-exit validation (11 November 2026)

**`pnpm validate` exits 0** on the combined source,
`startup-and-child-exit-validate.log`: **679 frontend / 467 standard Rust tests
pass**, with **129 fixtures ignored**. Formatting, generated contracts,
TypeScript, production frontend build, Cargo check and strict all-target/all-
feature Clippy pass. The normal parallel PostgreSQL protocol target passes in
this full run, including both original startup-cleanup cases. Existing ts-rs
attribute and frontend chunk-size warnings remain nonfatal. This is current
source evidence; the preceding failed full runs remain recorded below.

The PostgreSQL correction is committed locally as `6a5c65e`, and the helper-exit
tests as `3d74855`. Frozen reviewed SHA-256: `postgres.rs`
`4766c12c45655b01b59aefb2387fd27ff7da6911ea2e8c73f96872d72ed8998c`;
`postgres_protocol.rs`
`19bf543fc64b52f05c1a8aefdbb39f30a77809254e8b21829b613e3ed4f6405b`;
`snowflake_table_qualification_runner_tests.rs`
`eb284910a2e7d3631a30988dd7f9ab5fd4cbb52658620e7214b3789553ca96e3`.
The source bytes are unchanged after these checks; commit metadata does not
change the tested source. Earlier runner hashes below describe their recorded
preceding increment.

The isolated PostgreSQL/SSH/TLS and actual-helper interruption checks below are
separate scopes. No desktop UI end-to-end workflow, installed application,
package, real Snowflake SSO, account SQL, the user's private credentials or billing
is exercised in this increment. The Goal remains active/incomplete; packaged
Snowflake daily workflows, million-row account behavior, all applicable DataGrip capabilities and owner
acceptance remain gates.

The complete combined-source Snowflake qualification module subsequently exits
**0**, **124/124 passed**, **281.16 seconds**, normal three-thread execution,
`qualification-with-actual-child-exit-all.log`. This includes all 122 preceding
cases and the two new seven-mutation child-exit loops: both complete census-
extreme success routes, cancellation, definitive rejection, SDK callback loss,
observer failure, dropped waiter, post-ACK storage failure and offline policy/
wire/lifecycle validators. Counts overlap the focused tests below. No application
or test source changed after full validation or while running this module.

```sh
pnpm validate
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
```

### PostgreSQL startup timing and disabled TLS (11 November 2026)

The two startup-cleanup failures below are reproduced with temporary, fixed-stage
timing probes, without changing deadlines or scheduling. The normal parallel
PostgreSQL protocol target exits **101**, **6 passed / 2 failed / 1 ignored**, in
3.79 seconds, `postgres-startup-stage-diagnostic.log`. Native-root enumeration
takes approximately **3.745 / 3.770 seconds** in the two failing calls. Both peer
timelines stop at listener-ready/accept-wait; neither connection is accepted
before the unchanged three-second dispatch deadline expires. This attributes
these reproduced failures to synchronous native trust loading with TLS disabled;
it does not explain the separate earlier Snowflake helper-start failures.

The driver now enumerates native roots only for `VerifyFull`. Explicit CA-file
bounded reading, PEM validation and root insertion remain outside that condition.
TLS configuration, authentication and startup task ownership are unchanged. The
temporary driver prints are removed. The existing socket-cleanup regressions
retain bounded peer-stage diagnostics; a new actual-open regression rejects empty
and malformed explicit CA files under both TLS modes before any connection.

The repaired normal parallel target exits **0**, **9 passed / 1 ignored**, in
4.18 seconds, `postgres-startup-repaired-protocol.log`. This includes both original
startup cancellation/drop tests and the new CA-validation case. The latter does
not prove a valid TLS handshake or count native-loader calls. Independent bounded
review finds no material issue. Tier 2 generation `2026-11-11T16:25:18Z` reports
matching metadata/no recorded gap for both changed files; exact source/diffs are
also reviewed. Heuristic cross-domain graph call edges are discarded.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --test postgres_protocol -- --nocapture
```

Logs are under `.cache/verification/connection-attempts/`. The earlier full
validation failures remain historical failed runs, not erased or reclassified.

The isolated real-database run passes **43 PostgreSQL tests** in **12.38 seconds**
and **2 SSH integration tests** in **4.62 seconds**, exit **0**,
`postgres-startup-real-fixtures.log`. The latter exercise real queries/cancellation
through the bastion and a trusted fixture TLS connection with original-hostname
verification, including wrong-hostname rejection. The other four non-ignored SSH
tests are filtered by this explicit fixture run. This qualifies the changed
PostgreSQL behavior locally, not Snowflake or every TLS deployment.

```sh
JOINERY_POSTGRES_PORT=55439 docker compose -p joinery-pg-startup-20261111 up -d --wait postgres
JOINERY_TEST_PORT=55439 cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --test postgres --test ssh -- --ignored --test-threads=1
JOINERY_POSTGRES_PORT=55439 docker compose -p joinery-pg-startup-20261111 down -v
```

The new project used synthetic fixture data and a previously unused loopback
port. Its container, volume and network are removed after testing. The two
unrelated running PostgreSQL containers are preserved and checked afterward.

### Actual helper child exit in the full lifecycle (11 November 2026)

Two new synthetic-SDK tests pass, **2/2** in **134.94 seconds**,
`qualification-actual-child-exit.log`. Each covers all seven mutation phases
(CREATE, ADD, two INSERTs, UPDATE, DELETE and DROP): **14 scenarios** total.
The generated test-owned SDK wrapper exits its actual Node helper process after
the synthetic effect, either before delivering the mutation callback or during
the first subsequent parent-originated observation callback. The latter causally
establishes that the parent consumed the preceding mutation ACK. Production code
and the standalone SDK fixture are unchanged by this test increment.

The pre-ACK cases retain the original unknown typed mutation outcome, no mutation
ACK, and the actual retirement report. The post-ACK cases retain the original
successful mutation/report/SDK request/query IDs and the later observation
retirement; a connection failure does not rewrite that success. Every case
compares the exact SQL-ledger prefix, a single simulated effect/exit marker,
original operation completion and journal records. No replay, later phase or
cleanup DROP is admitted. Local settlement includes all transport tasks, child
reap and removal of the owned private runtime root. Both stdout EOF and child
wait normalize to the same retirement cause; the test does not require a
particular stdout-task disposition.

```sh
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner_child_exit_ -- --test-threads=2
```

Independent bounded review has no material finding. Tier 2 generation
`2026-11-11T16:25:18Z` reports matching runner/SDK metadata with no recorded gap;
the process partial range 1314–1318 and actual retirement/reaper/reader source
are inspected directly. These tests establish actual local child-process
interruption with simulated Snowflake effects. They do not establish an actual
Snowflake request, remote effect, billing or remote exactly-once execution.
Remaining full-route census/context drift, one-use source/runtime/binary/private
profile binding and the executable account entry remain open.

### Full Studio qualification composition (11 November 2026)

The synthetic-only owner now runs the full 32-phase lifecycle through real Studio
APIs, the adapter and production main/helper. The sole Plan factory generates
its target; no private-profile, account, SQL or target selector is accepted.
The stateful SDK independently spells the nine expected statements and models
three/four-column metadata, pending versus committed rows, two transaction IDs,
rollback, commit, update, delete and final DROP. This is actual local route
composition with a synthetic SDK, not real Snowflake/SDK qualification.

Local commits: `933b2fa` closes the DDL action factory, `b3409ce` repairs finalizing
read outcomes, and `4bc4be2` adds the full synthetic Studio lifecycle and original
operation-completion recording. Current source is unmerged, unpublished and
uninstalled. Both repaired-source full validation failures below use this same
code, before the commit metadata was created.

Initial S=1/32 success coverage passes with exactly **393/610** SQL, **28** runtime
publications and **32** validated phases. Each original typed operation completion
is separately queued/fsynced before validation; the success test compares the
original DTOs and journal ordering phase by phase. Rows are decoded losslessly
from local retained Arrow IPC; retrieval issues no SQL.

Focused checks currently pass **4** DDL action-factory tests (0.02 seconds),
**20** owning-policy/route tests (18.68 seconds) and **10** standalone SDK tests.
Logs are under `.cache/verification/connection-attempts/`:
`qualification-ddl-factory-tests.log`,
`qualification-owning-completion-tests.log`, and
`qualification-stateful-sdk-tests.log`.

Full application validation exits **0**, `qualification-full-runner-validate.log`:
**679 frontend / 466 standard Rust tests pass**, with **129 fixtures ignored**.
Formatting, contracts, TypeScript, production build, Cargo check and strict
all-target/all-feature Clippy pass. Existing ts-rs/chunk-size warnings remain
nonfatal. This pre-repair validation precedes the production finalization fix and
later diagnostic assertion changes; it is not a current-source full pass.

The expanded qualification run exits **101**, **119 passed / 3 failed**, 221.30
seconds, `qualification-full-runner-final.log`. All seven mutations across each
definitive rejection, post-effect SDK callback-error and post-ACK observer-error
case pass the strengthened exact-ledger/original-outcome assertions. Dropping the
caller at the post-CREATE-ACK barrier also passes through all 32 phases. The three
failures occur at phase 1 with generic helper-start errors before any SQL. Their
startup cause is not established.

The unchanged focused serial rerun exits **101**, **2 passed / 1 failed**, 95.39
seconds, `qualification-full-runner-failed-serial.log`: both cancellation variants
and the full S=1/32 success route pass. The post-ACK journal-failure test instead
closes the session at phase 2, before its intended phase-3 storage fault. Adding
diagnostic assertions and rerunning the same three cases in parallel again exits
**101**, **2 passed / 1 failed**, 52.21 seconds,
`qualification-full-runner-startup-diagnostics.log`. The same phase-2 failure has
four completed outbound writes, 22 received frames, no stderr, and an interrupted,
settled failed transport whose first error is `ConnectionClosed` with
`completion_invalidated=true`. Success/cancellation pass. Neither rerun proves
the intended storage-failure behavior or resolves the earlier startup failures.

An isolated diagnostic run, `qualification-full-runner-journal-diagnostics.log`,
exits **101**, **0 passed / 1 failed**, 22.14 seconds, at phase 1. Both bootstrap
and version SQL have original successful terminal reports, followed by a locally
submitted CLOSE and its successful logout acknowledgement before startup returns
`ConnectionClosed`. No transport error preceded that close. Source investigation
identified a race between removing an operation from pending, observing its
terminal response and publishing its original terminal watch: result handling
can request credit in that interval and mistake the finalizing operation for an
inactive target.

The deterministic actual-helper reproducer holds the terminal observer at that
boundary. Both accepted and denied observer variants fail with the same premature
`ConnectionClosed` before repair: **0 passed / 2 failed**, 0.19 seconds,
`qualification-finalizing-credit-red-compiled.log`. Its earlier compile attempt
missed the required `MetadataObserver::check` method and ran no tests; corrected
before this reproduced failure.

The repair waits only after the precise inactive-control error and only when the
same operation's owned completion evidence proves terminal finalization. It waits
for the original public watch after callbacks; it does not expose private cached
success, issue another control/SQL request or change production deadlines. Four
actual-helper regressions cover credit/discard with observer success/denial and
pass **4/4**, 0.17 seconds, `qualification-finalizing-credit-green.log`. They assert
the stream stays pending and the public watch stays empty while observation is
held, then compare the actual published original report and reject observer
failure without extra SQL or control admission. The original full-route journal-
failure test now passes **1/1**, 14.78 seconds,
`qualification-full-runner-journal-repaired.log`, reaching the intended phase-3
storage fault and retaining the original successful DDL result without advancing.
Failed storage is not claimed durable. The complete repaired-source qualification
module passes **122/122**, **213.80 seconds**, normal three-thread execution,
`qualification-full-runner-repaired-all.log`. This includes all three seven-
mutation fault loops, both S=1/32 success routes, both cancellation variants,
caller-drop, journal failure, overflow guards, original completion policy tests
and existing offline validators. Subsequent process/adapter, strict checks and
failed full validation results follow. Counts overlap the focused results above.

The repaired-source full validation exits **101**,
`qualification-full-runner-repaired-validate.log`: **679 frontend** and **418 main
Rust library tests** pass, but PostgreSQL protocol ends **6 passed / 2 failed /
1 ignored**, 3.61 seconds. Both previously recorded startup-cleanup cases time
out at `tests/postgres_protocol.rs:121` waiting for identity dispatch. The failure
has the same location as the earlier unchanged-source failures documented below;
the production Snowflake repair does not change PostgreSQL code. This is a failed
full run, and strict lint is not reached. Separate affected checks follow; no
deadline or test is relaxed.

The independent existing process/adapter regression passes **131/131**, 3.90
seconds, `qualification-finalizing-process-adapter.log`, including the four new
finalization cases. Separate **`pnpm check:rust` exits 0**,
`qualification-finalizing-rust-checks.log`: formatting, Cargo check and strict
all-target/all-feature Clippy pass. Neither result converts the failed full
validation into a pass.

The unchanged PostgreSQL protocol target passes serially **8 passed / 1 ignored**,
1.00 second, `qualification-finalizing-postgres-serial.log`. Source review confirms
unnecessary synchronous native-root enumeration before connecting even with TLS
disabled; that can block the fixtures' current-thread runtimes, but timing
attribution for the failed run is unproven. No PostgreSQL code, fixture deadline
or scheduling policy is changed in this increment. The complete unchanged rerun,
`qualification-full-runner-repaired-validate-rerun.log`, also exits **101** at the
same two PostgreSQL startup-cleanup cases, **6 passed / 2 failed / 1 ignored**,
3.40 seconds. Frontend/main Rust tests pass again, but this is another failed full
run and does not reach strict lint. The separate strict check above passes.
Do not substitute the earlier pre-repair full pass for either failed current-
source run. Root cause diagnosis remains open.

Independent review has no material finding in the three-file repair. Tier 2
generation `2026-11-11T16:07:46Z` reports matching metadata, with process partial
lines 1314–1318 read directly; the missing graph match-guard caller is checked in
source. External and new-test paths have no recorded coverage issue. These are
best-effort index signals supplemented by exact source/diff review. These findings
do not establish the cause of the separate generic helper-start failures.

Frozen reviewed repair SHA-256: `snowflake_process.rs`
`e6037a4da586474b8cb8ef342ed9bce286ae11ad99ff011ab1c7ef6fd34fd1d7`;
`snowflake_external.rs`
`658eef3c5b568efdde11a61ef36014e8c6ba8f7b7902a2a76831fb96c80438ef`;
`snowflake_qualification_finalizing_adapter_tests.rs`
`b2f5456e6d8f83e01960411ad09e1d8ca7e58446933aa1eb44c35ac52f0f5b14`.
The final runner hash is
`ea44d22d01d2faa467078da23015f0c8712845d46e7b8049c9c7572eecaf0aef`;
its SDK and Node-test hashes are respectively
`a08b46ab21612a818c2399d2794eb61c9ed68aa9eeda261bff1915f41a90398a`
and `7f858cc25bad110016a600d2a9c60c34df98f7e01daf5b4d8014a077bc016731`.
The read-only impact review confirms startup/version, metadata, ordinary and
bound reads all use repaired `stream_bounded`; unfinished-operation cleanup
already treats inactive targets as benign while preserving observer ownership.

Reproduce using the pinned Node/Rust environment documented below:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib ddl_factory_ -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib owning_ -- --test-threads=1
node --test src-tauri/tests/support/snowflake_table_qualification_sdk.test.mjs
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_finalizing_query_ -- --test-threads=1
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification_studio_runner -- --test-threads=3
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=3
pnpm validate
```

The first five-test fault run ended **4 passed / 1 failed** in 484.38 seconds,
`qualification-full-runner-faults-first.log`. All seven mutations in each rejection,
post-effect SDK callback-error and post-ACK observer-error case reached the expected
stopping phase. The cancellation assertion incorrectly required a non-success
terminal after the fixture released its stalled stream with clean EOF. Source
review confirmed that production correctly preserves that original successful
terminal; a cancel acknowledgement alone cannot replace it. Keep that EOF case
and a distinct SQLSTATE-57014 stream-error case, retaining both original events
and the actual cancel error. Callback loss is not helper wire EOF.

Review also corrected early read snapshots after cancellation, overflow being
mistaken for complete capture, cross-phase ACK assertions, weak rejection/loss
certainty checks and replacement of original errors with generic harness errors.
Final tests include a dropped waiter at the post-CREATE-ACK barrier and a journal
permission failure before completion recording. The latter passes after repairing
the separately reproduced finalization race described above. No claim is made
that failed storage became durable. Independent bounded source review has no
remaining material finding; complete qualification results are recorded above.

Still open: actual helper wire interruption, remaining drift cases, the one-use
source/runtime/binary/private-profile envelope and account entry. No account SDK,
SSO, private state, external SQL, billing, UI change, package or installation was
involved. Nothing is pushed, merged or published. Full DataGrip replacement,
real writes/DDL/transactions, normal packaged SSO, 1M-row account performance and
owner acceptance remain unqualified.

### Same-call reconciliation bridge (11 November 2026)

Studio's harness-only refresh returns the original final context options and
middle catalog from its existing options/catalog/options reconciliation. The
ordinary API continues returning options only. The implementation shares one
body: no extra SQL, cache lookup, authority refresh or hold-release path is added.
The qualification owner can consume these originals without a second catalog
request. Full 32-phase actual Studio/helper composition is still unimplemented.

Focused `ddl_studio` coverage passes **15/15** in 1.05 seconds,
`qualification-reconcile-studio-tests.log`. Two new recording-driver regressions
verify full original catalog/revision/target/traits, returned context, exactly two
context calls and one catalog call, and hold clearance/no-hold rejection. Existing
incoherence, failed reconciliation and stale queued refresh checks pass in that
run. These are local recording-driver tests, not actual-helper full-route or
account behavior. Reproduce with the pinned environment documented below:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib ddl_studio -- --test-threads=1
pnpm validate
```

Independent bounded review found no material issue in the two bridge files:
lane/revision/hold checks, successful clearing and failure invalidation are
unchanged. No new runtime, UI or account path is exposed.

The first full run, `qualification-reconcile-validate.log`, passed 679 frontend
tests, frontend build and standard library tests, then failed both unchanged
PostgreSQL startup-cleanup tests at `tests/postgres_protocol.rs:121` waiting for
identity dispatch. The protocol target passes unchanged both serially (**8 passed,
one ignored**, 1.05 seconds, `qualification-reconcile-postgres-serial.log`) and
with normal parallel execution (**8 passed, one ignored**, 1.01 seconds,
`qualification-reconcile-postgres-parallel.log`). The earlier analogous failure
remains recorded below. Cause is unconfirmed; strict lint was not reached in the
failed full run. Bounded review confirms unnecessary synchronous native-trust
enumeration for disabled PostgreSQL TLS before connection dispatch, but does not
establish it as the timeout cause. No PostgreSQL code or deadline changed.

The complete unchanged rerun, `qualification-reconcile-validate-rerun.log`, exits
**0**: **679 frontend / 466 standard Rust tests pass**, **129 fixtures ignored**.
Formatting, contracts, production frontend build, Cargo check and strict
all-target/all-feature Clippy pass. The protocol target passes within that run
(eight passed, one ignored, 1.80 seconds). Existing ts-rs/chunk-size warnings remain
nonfatal. Passing the rerun does not resolve the recurrent timeout diagnosis.

Bridge source SHA-256: `snowflake_ddl.rs`
`9f8b27bf81ba1831b54c3d8c569140b97c19971c2cb767064dcc9469e500139d`;
`snowflake_ddl_tests.rs`
`2c01005127f55751bd1cc68702e5df3453a199d932d5b7c0d2a8728cb20b0586`.
Tier 2 coverage generation `2026-11-11T15:08:17Z` reports metadata match and no
recorded issue for both bridge files and the qualification policy; this is a
best-effort signal, supplemented by targeted source review. The policy change is
a stale-comment correction only. The proposal now reflects previously completed
policy/journal work; the roadmap records the closed action/event/Arrow runner
work still required. No private profile or account activity occurred.

### Owning qualification policy (11 November 2026)

Local harness implementation now composes exact physical SQL/control admission,
census/context/target lifecycle checks, runtime publication observations, typed
phase evidence and a private fsync journal. There is no desktop/account runner,
profile lookup, executable approval envelope or resume path. No private profile
was read and no account activity occurred. `SANDBOX.BILLIE_TESTING` remains the
permitted new-table location; the previous CREATE DATABASE approval is consumed.

| Check | Result and scope |
| --- | --- |
| Complete qualification regression | **150/150**, `qualification-owner-all-repaired.log`, 133.70 seconds: includes the owning policy, controls, journal, sequencer, evidence, existing metadata/transport and Studio integration cases. This precedes the final test-only cached-string comparison fix; the affected adapter composition is rerun separately below. Counts overlap |
| Complete offline owner routes | Within the 150: all 32 phases at census sizes 1 and 32, exactly **393/610 SQL**; original report/SDK/query-ID/typed-ACK equality, 28 runtime publications, all 32 durable phase records and independent journal hash-chain verification. Shaped callbacks/DTOs are synthetic; this is not a complete actual-adapter route |
| Actual adapter/helper policy composition | Within the 150: three cases through actual `SnowflakeExternal::open` and adapter methods with production helper/framing and a synthetic SDK. Startup and CREATE preflight use exact **14 SQL**, parsed operation results, census/runtime observations and first two typed phase barriers. Another case blocks preflight before the third SQL if typed startup completion is omitted. The third forces query EOF during durable credit admission and verifies original local completion, candidate consumption and durable accounting without adding SQL. None launches production `main.mjs` or accesses Snowflake |
| Final affected adapter-policy rerun | **3/3**, `qualification-owner-adapter-post-lint.log`, 0.76 seconds, against the accepted source manifest after the comparison-only allocation fix; production sources remain identical to the 150-case run |
| Exact staged transport commit | **58/58**, `qualification-transport-index-process-corrected.log`: all process, cleanup and qualification-transport cases, including seven new local-original regressions. An isolated checkout of the staged tree excludes the uncommitted owning policy; committed as `98aab26` |
| Exact staged prerequisite commits | Runtime observation **3/3**, `qualification-runtime-index-tests.log`; private journal **11/11** plus formatting, `qualification-journal-index-tests.log`. Separate isolated staged trees; committed as `4cafd4c` and `59fc3e5` |
| Exact staged request sequencing | **77/77**, `qualification-sequence-index-final-tests.log`, 0.77 seconds; includes eight control and 27 sequencer cases plus existing definitions/journal. Committed as `87d2318`; the owner-only plan accessor is deferred to the owner commit |
| Exact staged typed evidence | **88/88**, `qualification-evidence-index-tests.log`, 0.89 seconds; adds 11 typed-evidence cases to that prerequisite tree. Committed as `0894688` |
| Final adapter regression | **69/69**, `qualification-owner-adapter-repaired.log`, 11.04 seconds: production adapter/helper with a synthetic SDK. Includes three new runtime-observation tests, callback rejection before publication, mutation ACK retention and unchanged SQL budgets; these three tests launch five synthetic SDK cases |
| Final application validation | **`pnpm validate` exits 0**, `qualification-owner-validate-accepted.log`: **679 frontend / 465 standard Rust tests**, **129 fixture tests ignored**. Formatting, contracts, production frontend build, Cargo check and strict all-target/all-feature Clippy pass. Existing ts-rs attribute/chunk-size warnings remain nonfatal |

The journal tests exercise exclusive private creation, symlinks/replacement,
permission/length/link changes, independent lossless canonical hashes, bounded
records/depth/node counts, partial writes and fsync failures. They establish local
filesystem/fault behavior, not crash/power-loss durability or authenticated audit
evidence. The owner tests block journal I/O, drop admission/finish futures and
repeat final persistence: receipt consumption occurs exactly once, failure stays
latched and the phase cannot advance. Original reports are retained before
policy rejection; final local draining does not repeat SQL.

Independent review found a transport-to-owner failure propagation gap. The new
regression failed before repair (`qualification-owner-shared-failure-red.log`):
the attachment rejected work while the concrete phase owner remained healthy.
`SharedPolicy::failed` now forwards failure to the concrete owner before latching
the shared error, outside shared/counter locks. The passing regression verifies
concrete/shared first-error retention, unchanged phase and repeated cleanup
failure. Final bounded review found no material issue in this fix or the owner's
fsync/cancellation ordering. Another reviewer checked the full-route tests;
exact persisted typed-ACK contents were added alongside their count assertions.

The broader regression then reproduced an actual startup reservation leak: a
query could finish while its result-credit admission was syncing. The process
published an inactive local result without notifying the owner, which retained
the reservation and could reuse the candidate ID. Separate local observation now
captures queued cancellation/close, inactive controls and all six retirement
reasons. Original ACKs remain accessible while callbacks run outside State;
in-flight publications block settlement. Admitted candidates are consumed before
pending-ticket/target revalidation, while actual wire assignment stays nullable.
The deterministic actual-adapter regression gates EOF and admission explicitly.
Exact close accounting includes only durably recorded admitted inactive controls;
it does not relax equality to an inequality. Independent review found no material
issue in the repair. The original pre-repair 91-case pass is retained in
`qualification-owner-integrated-corrected.log`; it did not expose this race.
Final bounded source review also checked operation completion ordering: the
production collector awaits each credit/discard control, whose original callback
precedes terminal-watch publication. Successful startup therefore cannot return
with that control's sequencer reservation still pending. The forced race fixture's
explicit callback signal is redundant synchronization, not an extra SQL or drain
mechanism in production.

A shaped retired-CREATE owner fixture retains the original typed ACK and
`StatementSucceeded` in exactly one durable local record, even though the original
report is unsuccessful and has no final result. Repeated local persistence and
new admission cannot advance phase three or repeat the SQL. This composes the
local callback, policy and real filesystem journal; it is not an actual helper
EOF-after-ACK or account mutation claim.

The final 23 changed code/config/test inputs are frozen in
`qualification-owner-source-accepted.json`, SHA-256
`3e9eb3cbcfae7c42f1822cf2f98fd51467022eccb9cae9d6cc30f519894ce45d`.
The preceding 150-case source is retained in
`qualification-owner-source-repaired.json`, SHA-256
`ec5606b661d5f8dcefc3ae3bf382c431b04c0da2a73becb067fbedede9f338fd`;
only the adapter fixture's cached-string assertions differ. Production inputs
are identical between these manifests.
This is a bounded changed-file freeze, excluding documentation and the full
dependency/runtime tree. Tier 2 graph coverage generation
`2026-11-11T14:59:26Z`, recorded `2026-11-11T15:00:51Z`, reports metadata match for
all 21 changed Rust paths, best effort. The process file has a partial parse at
lines 1299–1303; those lines were read directly, along with targeted source review.
Other changed Rust paths have no recorded issue, which is not proof of complete
coverage. Configuration/lock changes were inspected directly.

Failed development checks remain recorded:

- `qualification-runtime-adapter-first.log`: two passed, one failed because a new
  test expected `session_state` to return an error; existing behavior returns an
  unavailable state after observer denial. Correcting the test passed all three
  in `qualification-runtime-adapter-corrected.log`, without a production change.
- `qualification-owner-validate-first.log`: frontend and standard Rust suites
  passed; strict Clippy rejected two unused methods before new tests were
  registered and a new test's standard mutex guard across an await. Registering
  actual-route tests exercises the methods; synchronous polling in a lexical
  guard scope fixes the lock issue without a lint allowance.
- `qualification-owner-integrated-final.log`: new tests initially failed to
  compile due to a missing `Poll` import and nonexistent `SafeMode::LocalConfirm`.
  Corrected to the current `Confirm` enum. That invocation also unnecessarily
  enabled default desktop features; the successful 91-case rerun uses the normal
  library/no-default-features harness command below.
- `qualification-owner-validate-final.log`: all 679 frontend / 465 standard Rust
  tests passed (129 fixture cases ignored); strict Clippy then rejected an owned
  string allocation in a new report assertion. The corrected comparison keeps
  the persisted wire ID as a string and compares borrowed strings, without a
  lint allowance. The final-source qualification and validation reruns follow.
- `qualification-owner-all-final.log`: **130 passed / one failed**. The actual
  adapter startup/preflight case failed its phase barrier because an admitted
  credit completed locally without notifying the owning sequencer. The final
  150-case run above passes after the local completion repair.
- `qualification-local-original-red.log`: both new local-original tests failed
  before capture was wired (zero observed callbacks, one expected).
- `qualification-stale-credit-red.log`: the deterministic actual-adapter race
  failed waiting for its missing local completion callback before the repair.
- `qualification-local-original-green.log`: the first repair compilation failed
  four E0695 diagnostics for unlabeled `continue` statements inside labeled
  blocks. Explicit outer-loop labels fix compilation without changing control
  flow; the 150-case rerun includes all seven new local-original tests.
- `qualification-transport-index-tests.log`: **28/28** ordinary process tests
  passed, but that filter omitted cleanup and qualification-transport cases.
  Widening it initially passed **56/58** in
  `qualification-transport-index-process-all.log`: two existing child-process
  fixtures could not find their manifest-relative pinned Node binary in the
  exported checkout. Provisioning that same existing binary in the isolated
  fixture's expected cache path, without source changes, passes all **58/58** in
  the corrected log. The full working-tree 150-case run had already passed those
  cases. The export and provisioned binary are local cache artifacts.
- `qualification-owner-validate-repaired.log`: frontend **679/679** and the
  production frontend build passed; the standard Rust run stopped at two
  PostgreSQL startup-cleanup dispatch timeouts in `tests/postgres_protocol.rs:121`,
  before strict lint checks. Those tests, the PostgreSQL driver and startup paths
  are unchanged from `86c5da6`. The exact target passed unchanged on rerun:
  **8 passed / one fixture ignored**, 2.48 seconds,
  `qualification-owner-postgres-protocol-rerun.log`. Cause is unconfirmed, not
  classified as a proven pre-existing failure. Source review identified synchronous
  native-certificate loading even with PostgreSQL TLS disabled as an unmeasured
  hypothesis; the failure occurred before cancellation/EOF assertions. No timeout
  or PostgreSQL source was changed. A full validation rerun is recorded above.
- `qualification-owner-validate-repaired-rerun.log`: **679 frontend / 465 standard
  Rust tests passed**, **129 fixtures ignored**; formatting, frontend build and
  Cargo check passed. Strict Clippy rejected three comparison-only allocations
  in the newly added deterministic race fixture. One cached string and explicit
  JSON `as_str()` comparisons retain string-only ID semantics without a lint
  allowance or production change. The affected adapter cases and final full
  validation are rerun after this correction.

Reproduce with pinned Rust **1.99.1**, Node **24.22.2**, pnpm **11.23.0**, and an
explicit helper executable. Logs are under
`.cache/verification/connection-attempts/` and are not committed.

```sh
export JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node"
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib qualification -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_process:: -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests -- --test-threads=1
pnpm validate
```

The fixed runner still needs actual same-call reconciliation catalog evidence,
complete production-route interruption/rejection coverage and end-to-end durable
outcome qualification using the separate original local-completion callback.
One-use source/runtime/binary/private-profile binding remains open. No account
execution plan or approval follows from the journal/ledger hash. Actual effects,
normal packaged SSO, million-row account performance, full DataGrip functionality
and hands-on acceptance remain required. No UI behavior changed; prior native,
browser, actual local database and packaged-app evidence retains its recorded
source/scope and is not refreshed by these harness tests. Nothing is pushed,
merged, published, installed or externally qualified in this increment.

### Offline wire/lifecycle validators and terminal evidence (11 November 2026)

Local implementation adds strict final-frame matching, census/context/target
lifecycle validation and a harness-only pre-publication helper-terminal callback.
The validators are not an attached phase/admission policy, journal or account
runner. No private profile was read and no account activity occurred. The permitted
fresh-table location remains `SANDBOX.BILLIE_TESTING`; the previous setup approval
is consumed and no new execution approval is requested here.

| Check | Result and scope |
| --- | --- |
| Qualification definition | **31/31**: existing 14 compiler cases plus nine wire and eight lifecycle tests. One wire case matches all 393 entries for a one-schema ledger; this is not complete production-route execution. `qualification-primitives-compiler-final.log` |
| Process/transport | **51/51**, including three terminal cases with concurrent ACK/phase access, metadata-side shared failure, retirement, logical-handle/new-ticket reuse, malformed-terminal rejection and original-report/cache assertions. `qualification-primitives-process-final.log` |
| Final application validation | **679 frontend / 465 standard Rust tests**, **129 fixture tests ignored**. Formatting, generated contracts, frontend build, Cargo check and strict all-target/all-feature Clippy pass. `qualification-primitives-validate-final.log` |
| Adapter regression | **66/66**, production adapter/helper with synthetic SDK, including DML/DDL/transaction acknowledgement and interruption cases. `qualification-primitives-adapter-final.log` |
| Attached Studio route | **1/1**, production main/helper with synthetic SDK: startup, catalog, both attachments, disconnect, transport settlement and private-root cleanup. `qualification-primitives-studio-final.log`; not account SSO or SQL |

The first terminal regression failed before callback wiring: the observer saw no
event (`qualification-policy-terminal-red.log`). It passed after wiring. Review
then found an ACK visibility gap while the callback ran outside State. Both
terminal variants reproduced the gap before repair
(`qualification-policy-ack-gap-red.log`). A ticket-bound in-memory report cache
now preserves ACK/phase evidence until unchanged terminal publication and cache
removal under State. This closes the callback window and the accessors' prior
report-before-lock race. Normal connections retain locked terminal publication.
Counts overlap; they are not a total of distinct tests. Existing nonfatal ts-rs
attribute and frontend chunk-size warnings remain recorded by full validation.

Initial full validation failed strict Clippy on a new test's complex tuple type
(`qualification-primitives-validate.log`). A named test alias resolves it; no check
was bypassed. Earlier focused passes remain recorded in
`qualification-policy-terminal-green.log` and `qualification-primitives-process.log`.

Independent source review found no remaining material finding after the ACK fix.
The reviewed process/test SHA-256 values are
`bd222fa377bbfac85a210b072ffe57b98fa955cfb059a411c30822af868512c3` and
`29a11b37a1beb64974f147d3b9e8962d078ee6605ae0486938cdc6b9e9fe7e37`.
Tier 2 coverage generation `2026-11-11T13:49:15Z` reported metadata match/no
recorded issue, best effort, supplemented by direct source. The nine changed
code/test files are frozen in `qualification-primitives-source-final.json`,
SHA-256 `e6329a90fe7d877f8b1db71fc321b443be67effb828f49ff714ff6d4863f6fc2`.
This is a bounded changed-file freeze, not a complete dependency/runtime freeze.

Reproduce using the pinned toolchains from the shared SQL ledger entry below and an
explicit helper Node executable:

```sh
export JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node"
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_table_qualification:: -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_process:: -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib qualification_studio_attaches_both_domains_before_startup_and_drains_after_close -- --test-threads=1
pnpm validate
```

Terminal notification covers validated helper terminals only. Locally cancelled
and retirement-synthesized reports bypass it, even when they retain an early ACK.
The cache preserves existing caller access; it is not durable receipt capture.
Terminal notification also does not prove row draining or phase completion.
Exact phase/control ordering, callback correlation, complete outcome capture,
durable intents/receipts, one-use source/runtime/profile binding and a fixed
runner remain required. Full 393/610-entry synthetic routes, actual account
effects, normal packaged SSO, million-row performance and owner acceptance remain
open. The prior normal bundle/native/UI/database evidence retains its recorded
source and scope; these focused tests do not refresh it.

### Shared SQL and offline ledger (11 November 2026)

Implementation commit: `a8bcc61`, unmerged and unpublished.

The [SQL contract](snowflake-qualification-sql-ledger.md) shares bootstrap and
typed-boundary definitions between production helper and generated Rust constants.
Production metadata calls and the offline compiler consume immutable read specs.
The ledger expands every phase, preserving **386 + 7S**, exact wire SQL, empty
binds, query retention bounds and route-specific rejection tails. This is an
offline definition, not physical admission enforcement or an account runner.

| Check | Result and scope |
| --- | --- |
| Offline compiler | **14/14**, all 32 phases, every census size 1–32, exact ordering, target/census escaping, bounds, hashes and rejected tails. `qualification-ledger-focused.log` |
| Production read specs | **3/3**, one pure order/bounds case and two framed-helper/synthetic-SDK wire cases checking dispatched SQL/binds/ceilings. `qualification-ledger-read-specs-rerun.log` |
| Existing qualification regression | **48/48**, compiler, transport, metadata and Studio tests, 118.13 seconds. `qualification-ledger-all-focused.log`; not complete 393/610-entry application execution routes |
| Helper suite | **361/361**, synthetic helper/process and actual SDK loopback HTTP/TLS/proxy behavior; no Snowflake account. `qualification-ledger-helper-all.log`, 214.87 seconds |
| Contract/runtime/old manifests | **84/84**, `sql-contract-focused-final.log` |
| Focused helper regression | **88/88**, `sql-contract-helper-regressions.log`; overlaps the complete helper suite |
| Offline qualification/canary definitions | **147/147**, `sql-contract-offline-regressions.log`; these tests do not run an account canary |
| Offline runtime preparation/startup | Pass, new eight-source resource contract; Node 24.22.2 / SDK 3.4.1, guarded startup **zero authentication / zero SQL**. `qualification-ledger-runtime-{prepare,startup}.log` |
| Final application validation | **679 frontend / 465 standard Rust tests**, **129 fixture tests ignored**; formatting, generated contracts, production frontend, Cargo check and strict all-target/all-feature Clippy pass. `qualification-ledger-validate-final.log` |
| Final-source compiler | **14/14**, `qualification-ledger-compiler-final.log` |
| Final-source adapter | **66/66** internal and **32/32** integration; includes the three named-read-spec cases and synthetic 1M-row driver/store/CSV hash regression. `qualification-ledger-adapter-final.log`, `qualification-ledger-adapter-integration-final.log`; synthetic results do not establish account throughput |
| Fresh hidden native fixture | **3/3** successful runs, each with 20 check categories and ten cleanup cycles. SDK resources and guarded helper startup pass, **zero authentication / zero SQL**. `qualification-ledger-native-{build,runs}.{log,json}` |
| Fresh normal macOS SDK bundle | Pass, strict ad-hoc signature, package resources and guarded helper startup; **zero authentication / zero SQL**. `qualification-ledger-normal-bundle.log`; local/uninstalled, Developer ID and notarization open |

The initial read-spec invocation passed its pure case but failed both helper
cases before launch because root omitted the required `JOINERY_TEST_NODE`.
The corrected environment passed all three without a source change; the initial
log remains `qualification-ledger-read-specs.log`. The 48-case run preceded the
equivalent template-array migration to `sample_reads()` and getter-only build
hygiene change; affected final-source checks are recorded with final validation.
Counts overlap and are not a total of distinct cases.

Reproduce with the pinned toolchains and an explicit helper Node path:

```sh
export JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node"
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_table_qualification -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib named_read_specs -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib qualification -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --test snowflake_external -- --test-threads=1
pnpm test:snowflake:helper
pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline
pnpm test:snowflake:startup
pnpm validate
pnpm desktop:snowflake:smoke:build
python3 scripts/native-smoke.py --runs 3
pnpm desktop:snowflake:bundle:mac
```

Ledger suffix review followed actual production rejection paths: normal Studio
readback may run two samples after a rejected SELECT; DDL has one dedicated
post-rejection sample; DML, typed boundaries and metadata stop without subsequent
SQL. Every failed observer read stops its remaining suffix. No phase advancement,
automatic rollback, cleanup or replay is added. Review found no material issue
in the offline expansion and adapter migration; final integrated review covers
the source/resource contract separately. That integrated review found no material
issue in 27 files, including the new contract document. Its sorted
`sha256  relative-path\n` reviewed-set digest is
`1adddcf7ff33d4c1cbb4b3f3d9f1d9219247403b61b7a2004994f127143e4ae0`.
Tier 2 generation `2026-11-11T13:13:12Z` reported no recorded coverage issue and
matching metadata for the 26 code/config/test paths; this best-effort signal is
supplemented by direct source review. Cross-platform cfg paths were inspected,
not independently compiled.

The first full validation also passed. Its normal-library test build emitted
an unused `ReadSpec::kind` warning; the final source narrows that getter to its
actual `test-harness` callers. Only this compilation gate changed between the
two full validations. The final snapshot freezes **443 source/config/test
inputs**, excluding documentation, in
`qualification-ledger-source-final.json` (SHA-256
`4b72fb0e4543c8a938c83d64dd826c6acf5cdbc59ddbda4cc70179c7ec15d29c`).
`qualification-ledger-source-transition.json` records the single-file transition
from the initial snapshot. Existing ts-rs attribute and Vite chunk-size warnings
remain nonfatal.
Release builds also retain the previously recorded `rust-objcopy` stripping
warning from the pinned toolchain's missing `libLLVM.dylib`; global toolchain
configuration was not changed.

The hidden native binary is SHA-256
`74367edc666a94c0ab9f75e958eed796b8d93e58dca0eb9f682fff50bb873891`.
Its synthetic hidden WebView checks do not establish normal-app SSO, visible
startup, display performance or attributable total idle memory.

The final normal bundle is
`src-tauri/target/release/bundle/macos/Joinery.app`, built from `a8bcc61`.
Tauri reports **224.93 MiB**; a sum of all **13,055 regular files**, with zero
symlinks, is **240,258,489 bytes / 229.13 MiB**. The whole-bundle logical sum and
Tauri's displayed size are distinct measurements. Binary SHA-256 is
`6357be58300e60aaf1b09c6003e7b6ad92ffdfcc3ef804b954a9259885275f53`;
the sorted JSON file-manifest SHA-256 is
`9f2d5ec2c9388afa08f4e6c628228f103afa52d4a95f3a86c36a53e031993275`.
The packaged runtime manifest SHA-256 is
`74e1dfac7b3346d163fad04879dc7f558c73111d2eac6230a904a98e34621f1f`.
`qualification-ledger-normal-bundle-{artifact,files}.json` retain the receipt and
per-file hashes. All **443** frozen inputs still match after packaging. The normal
application was not launched, installed or used with an account.

No frontend workflow changed in this increment. The existing browser and actual
PostgreSQL results below retain their original source scope; they were not rerun
or relabeled as current-source checks. The changed adapter behavior is exercised
through the production framed helper with a synthetic SDK and SDK loopback tests.

No private connection was read and no account activity occurred. Target permission
for a fresh table in `SANDBOX.BILLIE_TESTING` persists, but the prior database setup
approval remains consumed. Physical ledger/lifecycle enforcement, durable
intent/outcome journal, reviewed execution envelope, fixed runner and complete
synthetic routes remain required before a new executable account approval.
Actual account effects, million-row performance, complete DataGrip workflows,
installation/signing and owner hands-on acceptance remain open. The private
project hub was not updated under the user's no-publication scope.

### Internal qualification attachments (11 November 2026)

Implementation commits: `e74e65a` (transport), `dd93739` (metadata), and
`7c5d469` (shared failure/publication), unmerged and unpublished.

The [attachment contract](snowflake-qualification-attachments.md) describes the
implemented boundary. This increment has no account connection, SQL, executable
approval, private profile access or desktop entry. Normal connections supply no
attachment. The closed SQL ledger, execution envelope, durable journal and fixed
account runner remain required.

The shared failure latch now reaches both physical transport and metadata
publication. Physical admission occurs outside the process mutex, with policy
and ownership rechecks on each physical write poll after contention or pipe
backpressure. Zero accepted bytes remains `NotSubmitted`; partial delivery stays
`OutcomeUnknown`. Counter-task settlement and actual child/private-root cleanup
are separate receipts. Original acknowledged mutation outcomes are preserved.

The final Studio check occurs after configuration, attempt-registry and session-map
lock waits. It rejects both shared policy failure and an exited driver before
insertion. A rejected publication retains the attempt until its owner closes the
driver. Metadata checks similarly follow cache/authority lock waits, with a final
successful-read check; mutation-specific ACK mappings remain separate.

| Check | Result and evidence scope |
| --- | --- |
| Transport races before fix | **0/2**, both expected failures reproduced: mutex contention after admission and revocation after a pending zero/partial write. `qualification-transport-final-policy-red.log` |
| Final transport focus | **13/13**, actual pipe/frame bounds, synthetic Node child lifecycle and controlled write polls. Includes unstarted disposal, live task settlement, cancellation/target replacement and known DML ACK retention. `qualification-transport-final.log` |
| Existing process regression | **28/28**, `qualification-transport-existing-process.log` |
| Metadata publication before fix | **0/3**, expected failures reproduced for catalog snapshot, transaction grant and successful-read return after rejection. `qualification-metadata-publication-red.log` |
| Final metadata focus | **13/13**, `qualification-metadata-final.log`: ten adapter workflow cases, two direct publication/wrapper seam cases after real-helper setup/sample, and one pure attachment unit. The 12 adapter cases use a handwritten launcher importing production `helper.mjs`/framing with a synthetic SDK; they do not launch production `main.mjs` |
| Existing mutation ACK regression | **4/4**, DML, DDL, TX and schema-switch cases. `qualification-metadata-ack-regressions.log` |
| Studio publication before fix | **0/2**, both expected failures reproduced: late policy rejection and actual owned-helper TERM/reap still returned a published session. The red tests explicitly close any wrongly published session. `qualification-studio-publication-red.log` |
| Final Studio focus | **8/8**, serial run, 119.50 seconds. Production main/helper through Studio with a synthetic SDK; startup denial, metadata rejection, physical stderr overflow and cross-domain revocation, late policy/child exit, unstarted zero counters and settled/reaped/private-root cleanup. `qualification-studio-final.log` |
| Independent final source review | No outstanding material findings. Tier 2 generation `2026-11-11T11:55:12Z`, 13 exact evidence paths best-effort `metadata_match`/`no_recorded_issue`, plus fresh direct source checks. Seven core source hashes remain identical after canonical formatting. Reviewer ran no tests, build or account activity |

Commands use pinned Rust 1.99.1 and Node 24.22.2, with explicit
`JOINERY_TEST_NODE` for actual-helper fixtures:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib qualification_transport -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib qualification_metadata -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib qualification_studio_ -- --nocapture --test-threads=1
```

All logs are under `.cache/verification/connection-attempts/`. An initial metadata
fixture compile found an omitted schema-switch argument and was corrected before
its focused pass. Root's first green attempt failed compilation because its new
registry unit named the existing shutdown method incorrectly
(`qualification-studio-focused.log`). After that correction, a parallel focused
run passed 6/8 but failed one helper startup and the stderr test
(`qualification-studio-focused-rerun.log`). The latter used
`process.stderr.write`, which production main deliberately silences. Its corrected
disposable SDK fixture writes directly to fd 2 and waits for independently drained
physical overflow; it does not require the counter task to run before connection
setup returns. The generic startup failure's cause was not established. All eight
passed in the final serial run without changing production startup deadlines.
Failed runs are not counted as passing qualification.

The first broad validation (`qualification-attachments-validate.log`) passed
**679 frontend / 464 standard Rust tests**, with **129 fixture tests ignored**,
then failed strict Clippy on five test-only findings: three recorder field types
and two lock guards that Clippy treated as live across an await. Named tuple
aliases and lexical guard scopes resolve them without lint allowances. The two
test files are the only differences between the initial snapshot and
`qualification-attachments-source-final.json`; the transition is recorded in
`qualification-attachments-lint-transition.json`. Production source is unchanged.

The first combined 44-case run then passed **43/44** in 178.36 seconds
(`qualification-attachments-focused-final.log`). Its helper-exit coordination
thread timed out waiting for setup. An isolated diagnostic rerun reproduced the
failure: the real connection later reached the publication hook, whose send then
failed because that ten-second receiver had already timed out
(`qualification-studio-exit-diagnostic.log`, 0/1). The corrected tests wait on a
channel owned by the setup lifetime; early connection failure drops the sender.
They retain bounded post-hook resume/cleanup waits and unchanged production
deadlines. Failure assertions now include the connection result and numeric
transport report. This is a third test-file-only source transition, recorded in
`qualification-attachments-final-transition.json`.

The corrected combined run passes **44/44** in 191.13 seconds
(`qualification-attachments-focused-final-rerun.log`). Independent review
confirmed the final channel ownership and unchanged production deadlines.
The next broad validation stopped on the existing PostgreSQL protocol test's
three-second identity-dispatch wait, before cancellation had begun
(`tests/postgres_protocol.rs:121`, suite 7 passed/1 failed/1 ignored). Its exact
unchanged rerun passes 1/1 in 2.34 seconds
(`qualification-attachments-postgres-cancellation-repro.log`). The cause remains
unresolved; this is not a confirmed pre-existing defect or a passing broad run.
The sequential driver overwrote that broad log when restarted; the preserved
result and explicitly labelled terminal excerpt are
`qualification-attachments-broad-first-results.json` and
`qualification-attachments-validate-intermittent-excerpt.json`.
Independent read-only diagnosis found no deterministic publication regression:
the fixture withholds the identity response, so setup cannot reach publication.
It identified unconditionally synchronous native certificate loading before TCP
connection, even with TLS disabled, as an unmeasured timing candidate. The pinned
loader accesses macOS trust settings synchronously. This remains an actionable
follow-up, not a demonstrated cause of the failed run.

Final unchanged-source `pnpm validate` passes (250.52 seconds): **679 frontend /
464 standard Rust tests**, with **129 fixture tests ignored**, formatting,
generated contracts/types, frontend build, desktop check and strict all-target /
all-feature Clippy. Receipt: `qualification-attachments-validate-final.log`.
The prior failed readiness wait remains recorded above rather than being erased
by the successful rerun.

Further current-source regression checks pass: **63/63** adapter-internal cases,
**32/32** adapter integration cases, **4/4** normal Studio startup/cancellation
cases without attachments, and **43/43** against the actual isolated PostgreSQL
18 fixture on `127.0.0.1:56432`. Logs are respectively
`qualification-attachments-{adapter-internal,adapter-external,normal-studio,postgres}.log`;
the sequential commands, exits and elapsed times are retained in
`qualification-attachments-broad-results.json`. The adapter integration includes
synthetic million-row driver/store/CSV coverage, which does not establish account
million-row performance. Counts overlap the focused suite and must not be added
to claim distinct tests. All external-browser helper cases here use synthetic
SDK/local data, not a Snowflake account.

The first browser selection could not launch any case because the expected
Chromium headless shell was absent from Playwright's default cache. Its failed
launch log and isolated trace archive are retained as
`qualification-attachments-browser.log` and
`qualification-attachments-browser-launch-failure.tar.gz`. No UI behavior is
qualified by that run. The browser is installed under the repository-only
`.cache/playwright-attachments`, selected with `PLAYWRIGHT_BROWSERS_PATH`; no
user browser profile or global configuration is modified.

The corrected browser run passes **24/24** in 40.4 seconds
(`qualification-attachments-browser-final.log`), using headless Chromium
156.0.8078.4 / Playwright shell revision 1248. It exercises connection deletion
and persistence, PostgreSQL attempt cancellation, Snowflake authentication
profile persistence, and synthetic-RPC SDK browsing, reviewed DDL/DML,
transactions, schema selection and session-state workflows. The browser is real;
the Snowflake RPC responses are synthetic and do not establish helper/account
effects. The isolated PostgreSQL fixture was removed with its task-owned volume
and network; ports 1421/1422/56432 are free and the unrelated inventory database
remains running (`qualification-attachments-postgres-cleanup.log`).

Fresh SDK-enabled hidden native build and **3/3 smoke runs** pass, each with ten
cleanup cycles. Receipts are `qualification-attachments-native-{build,runs}.log`
and the separately retained `qualification-attachments-native-{build,runs}.json`.
The copied executable SHA-256 is
`1d39bd7ad772d99ec4bbbe6e67fba156cbe45936593a86917170d2d704c62d83`;
the isolated signed smoke-bundle binary is
`56cb0b636b3eaff36e5b982f7c8692757c7499d131e8386d7339c4130b3024de`.
The build records source revision `7c5d469` and tree `92d8ffd033ddbc260fca0abe8bb26855d836290b`;
`sourceDirty=true` reflects the documentation-only worktree. All 437 frozen
source/config/test inputs still match after the build. Runs execute the copied
hidden test binary, not the normal application bundle. They do not qualify
fully cold visible startup, total application memory, display FPS, normal
packaged SSO or representative account workloads.

The fresh normal SDK-enabled macOS arm64 bundle also passes
`pnpm desktop:snowflake:bundle:mac`
(`qualification-attachments-normal-bundle.log`). It is runnable locally at
`src-tauri/target/release/bundle/macos/Joinery.app`, uninstalled and unreleased.
Strict ad-hoc signature verification, SDK package/resource verification and
guarded offline Node/helper startup pass; the startup receipt reports **zero
authentication and SQL requests**. SDK 3.4.1 resource manifest SHA-256 remains
`dd08932407ed6a540379dc5711a464fa51600498196a8637af671e23ea9256a0`,
with 15,107 resource entries and zero symlinks in the normal bundle.
Developer ID, notarization and normal packaged SSO/restart remain unqualified.

Normal signed binary SHA-256:
`1319a2699248acff64de5d16958870677014dd12387b4a8ae69f8bd5774a8b0d`.
The final complete-file manifest SHA-256 is
`ba0cc4b730b7fdad3f663ffa97f52b301e3d9ae26313abceb881612db129d4b4`
(`qualification-attachments-normal-bundle-files.json`): 13,053 regular files,
zero symlinks, 240,227,767 logical bytes / 229.10 MiB. Tauri's earlier build
display reports 224.90 MiB; these are recorded as separate measurements.
`qualification-attachments-normal-bundle-artifact.json` binds those receipts to
source `7c5d469`, its tree, and the unchanged final 437-input manifest. The only
working paths at artifact capture were the seven documentation files.

The recurring nonfatal `ts-rs` attribute, frontend chunk-size and pinned
`rust-objcopy` missing-LLVM stripping warnings remain in the logs. No global
configuration or toolchain was changed to hide them. No account activity,
installation, push, merge, deployment or publication occurred. The private
project hub was left unchanged under the local-only/no-publication scope.

The pre-validation source manifest `qualification-attachments-source.json` freezes
**437** source/config/test inputs, SHA-256
`e376117b2808cc78c439a4be96bbf11e88ab0e456f3b726b6fc2797d300c6cda`.
The lint-only 437-input snapshot has SHA-256
`84f40fadf2f619ad1390c36bcb0dfe70df0f13e77e821f121a4c413b35788566`.
The final `qualification-attachments-qualified-source.json` also freezes 437
inputs, SHA-256
`32070d2e6c78517e2723041d890292e0c9eb37fe73c37f7d070685b88186ae02`.
Documentation is outside that compiled-input manifest. This source snapshot and
focused evidence do not establish complete route qualification, normal packaged
SSO, real Snowflake effects, million-row account performance or DataGrip parity.
The Goal remains active and incomplete.

### Mutation receipt recovery and offline lifecycle definition (11 November 2026)

Implementation commits: `b06f030` (shared receipt recovery and regression coverage)
and `3039975` (harness-only offline lifecycle definition), unmerged and unpublished.

The shared in-process receipt outbox now covers original typed SDK DDL, DML and
transaction results. Transaction execution has an owned task that outlives a
dropped caller. Failed local audit/history stages remain pending independently;
retries write only the missing local stages and never repeat remote SQL. Pending
recording blocks new reviewed mutations, BEGIN/COMMIT and normal shutdown.
Explicit typed rollback retains its existing recovery policy; one unrecorded
rollback per console is allowed, then another requires its earlier receipt to
record. Primary remote errors keep their code/location and message prefix, with
an appended local-recording notice rather than a fabricated unknown outcome.

Transaction boundaries retain their existing audit evidence, without adding
script history. If only a rollback's started audit fails, its terminal ACK can
record immediately and the missing started stage can record later. Insertion
timestamps therefore need not follow action chronology. An adapter rejection
before submission preserves its original error and creates no invented terminal
receipt; an unrecordable start in that no-submit case is not retained. Once the
SDK operation is queued, failures become typed outcomes under the reviewed
adapter contract. These are in-memory pending receipts, not forced-exit/power-loss
durability while SQLite is unwritable.

The separate harness-only offline compiler generates a definition for one fresh
table in `SANDBOX.BILLIE_TESTING`: production-compiled DDL/DML, the closed 32-phase
order, and validators for typed outcomes, state, readbacks and a stable bounded
schema census.
Its budget is `386 + 7S`: 393 at one schema, 610 at 32. Its snapshot hash binds the
offline definition only. It has no helper, executable manifest, physical SQL
admission/counters, raw census/creation-marker interception, private connection
or runtime binding, durable execution journal, approval or account runner.
The official [TABLES view reference](https://docs.snowflake.com/en/sql-reference/info-schema/tables)
was rechecked on 11 November: the ownership projection's owner, type, transience,
temporary status and creation fields are documented. This verifies the reference
contract, not successful account execution.

| Check | Result and evidence scope |
| --- | --- |
| Reproduction before repair | **0/2**, expected failures: DML dropped waiter plus unavailable local recording, and discarded transaction terminal audit. `receipts-red.log` |
| Final receipt unit tests | **7/7**, default-feature compile also checks the owned transaction command receiver. `receipts-focused-final.log` |
| Actual production-helper receipt fixtures | **9/9**, including the same seven unit cases and two actual framed-helper/synthetic-SDK dropped-caller/storage-failure cases, 32.03 seconds. Both assert one remote submission and zero SQL during repeated local recovery/close attempts. `receipts-helper-final.log`; no account effects claimed |
| Complete focused Studio suite | **56/56**, including existing DDL, policy, observer holds and new receipt regressions. `receipts-studio-final.log` |
| Offline lifecycle compiler | **10/10**, exact generated SQL, phase budgets/order, census bounds/drift, snapshot tampering and selected runtime/effect/readback/inspection/catalog validators. Final formatted source: `table-qualification-unit-formatted.log`; earlier tightened-validator pass: `table-qualification-unit-final.log`. Selective validator tests do not qualify an executable workflow |
| Actual isolated PostgreSQL regression | **43/43**, 26.16 seconds, `receipts-postgres-final.log`; same command below, fresh owned Compose project `joinery-receipts-20261111-1040` on 56432. This does not qualify Snowflake |
| Affected browser workflows | **16/16**, 1.3 minutes, `receipts-browser-final.log`; reviewed DDL/DML/boundaries, browsing, schema selection and session observations. New DML/COMMIT local-recording warnings preserve known ACKs and quiet-window/no-replay assertions. Headless real UI with synthetic RPC interceptions; no account effects claimed |
| Combined application validation | Final `pnpm validate`, exit 0: **679 frontend / 463 standard Rust**, **129 ignored** fixture tests; formatting, contracts/types, production frontend build, desktop check and strict all-target/all-feature Clippy pass. `receipts-validate-rerun.log`; separate strict Clippy also passes (`receipts-clippy-final.log`) |
| Hidden native application | SDK build passes, followed by **3/3** hidden native runs, 20 check categories and ten cleanup cycles each. `receipts-native-build.log`, `receipts-native-runs.log`, frozen build/run JSON files with the same prefixes. Synthetic isolated application, not normal-app SSO or account qualification |
| Final normal macOS package | `pnpm desktop:snowflake:bundle:mac`, exit 0, **224.90 MiB**. Strict ad-hoc signature, 15,107 runtime entries, zero symlinks and guarded SDK 3.4.1 startup pass with zero authentication/SQL requests. `receipts-normal-bundle.log`, `receipts-final-artifacts.json`. Version/build 0.1.0; not installed, Developer ID/notarization and normal-app SSO remain open |
| Independent review | Bounded final source reviews found no material findings in receipt recovery or the offline definition. Tier 2 generation `2026-11-11T10:46:27Z`, best-effort `metadata_match`/`no_recorded_issue` coverage on relied paths plus direct final source reads. Reviewer ran no tests/account activity |

Focused commands use pinned Rust 1.99.1 and Node 24.22.2 in PATH, with
`JOINERY_TEST_NODE` set to the pinned Node executable for actual-helper fixtures:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib receipt_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib receipt_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib browse_classification_tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_table_qualification
JOINERY_TEST_PORT=56432 cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres -- --ignored --test-threads=1
pnpm validate
pnpm desktop:snowflake:smoke:build
python3 scripts/native-smoke.py --runs 3
pnpm desktop:snowflake:bundle:mac
```

Logs are under `.cache/verification/connection-attempts/`. Initial compiler compilation
found a fixture's boolean field mismatch; it was corrected before the final pass.
Final review tightened idle authority and unknown-column validation, with another
10-case passing run. The first full validation (`receipts-validate-final.log`)
passed the frontend and standard Rust tests but failed Cargo formatting in the two
new compiler files. The earlier standalone formatting command used edition 2021;
Cargo uses edition 2024. Running the pinned Cargo formatter changed only those two
files, then the compiler tests and full validation passed again. No failed run is
counted as a passing validation.

The final source manifest `receipts-source-qualified-final.json` freezes **430**
source/config/test inputs, SHA-256
`9030148f476674ec875ebd9632c41151f55c574fe1e3d8c08b53a92c521ee666`.
Its predecessor `receipts-source-qualified.json` differs in only the two offline
compiler files' formatting. The production receipt, helper and UI paths exercised
by the earlier focused/database/browser runs did not change during formatting.
Documentation is outside this manifest.

The hidden executable SHA-256 is
`1ff415d65064178e8f28baf56ed5a135bb03b2873ccd275e6e008e76cd4f6716`;
its isolated bundle executable is
`8dee1ae86cb8af944dcf817dbdb173ec0c183826b860238440f9e9f773b13520`.
Build metadata records source `3039975` and `sourceDirty: true` because the status
documentation was being updated; the frozen source/config/test inputs remain
unchanged. The SDK 3.4.1 resource manifest remains
`dd08932407ed6a540379dc5711a464fa51600498196a8637af671e23ea9256a0`.
Native resource checks cover 15,107 entries and five deliberate fixture symlinks;
guarded helper startup records zero authentication and SQL requests. The existing
`rust-objcopy` / missing `libLLVM.dylib` stripping warning remains nonfatal; no
global toolchain settings were changed.

The final normal bundle executable SHA-256 is
`243e09c2c9555690a5b574c4794f87a72b7bfd598b8da2f079877e8a4ecaac11`.
All 430 frozen inputs still match after native runs and final packaging.
`receipts-final-artifacts.json` binds these source and artifact pins to code
revision `3039975`. This does not qualify installation, normal packaged SSO or
restart with live Snowflake sessions, account mutations or complete application
memory/performance with one million account rows. The private project hub was
not updated because this Goal explicitly forbids publishing; repository status
and evidence are maintained locally.

The owned PostgreSQL project, volume and network were removed after verification
(`receipts-postgres-cleanup.log`); owned browser servers on 1421/1422 were stopped.
During low disk space, eight older synthetic browser fixture roots generated by
this Goal were archived with every file hash verified and no open handles before
their originals were removed. The 105 files remain recoverable in private
`.cache/verification/browser-fixture-archives/` archives and inventories; the
summary/restore example is `receipts-fixture-archives.json`. No blanket cache
cleanup, user data or account evidence removal occurred.

No account authentication or SQL occurred for this increment.
The [existing-schema proposal](snowflake-existing-schema-qualification.md) still
requires complete runner/gate regression tests and a fresh exact executable plan
with possible-billing approval. Full DataGrip replacement and owner acceptance
remain open.

### Integrated reviewed table DDL (11 November 2026)

Implementation commits: `2864282` (helper transport), `93d5e26` (native core,
adapter, typed contracts and enrollment), and `00ca4af` (UI, native guards and
browser coverage), all on `feat/datagrip-replacement`, unmerged and unpublished.

The SDK external-browser route now connects generated CREATE TABLE, nullable ADD
COLUMN and DROP TABLE through typed native IPC, separate profile consent, exact
SQL/target review, one-use adapter grants and bounded private helper status. Broad
`ddlEditing` remains false: these three actions have a separate capability. This
is local integration evidence; no new Snowflake account SQL has been dispatched.
The owner permits new tables under `SANDBOX.BILLIE_TESTING`; the prior database
setup approval is consumed. A fresh reviewed executable plan and billing approval
are still required. Normal-app SSO and owner acceptance remain open.

Reproduced and repaired defects include close before DDL acknowledgement being
misclassified, reentrant close dispatching after the request was settled, adapter
lock-order inconsistency, stale context after catalog revalidation, cross-console
form ownership, stale result notices, and lost terminal evidence when a dropped
waiter coincides with failed local persistence. A retained receipt now retries only
SQLite audit/history writes and blocks normal exit/further DDL until recording
succeeds. No SQL is retried. Abrupt loss while SQLite cannot write is not claimed
durable. A denied catalog refresh is recoverable by explicit coherent observation,
catalog reload and final observation; partial or stale reconciliation retains the
hold. Later transaction/DML holds cannot downgrade it to runtime-only refresh.

| Check | Result and evidence scope |
| --- | --- |
| Combined application validation | Final `pnpm validate` after the browser repair, exit 0: **679 frontend / 456 standard Rust**, **129 ignored** fixture tests; formatting, production frontend build, desktop check and strict all-target/all-feature Clippy pass. `ddl-validate-post-browser.log`; earlier passing run retained as `ddl-validate-final.log` |
| Frontend focused | **181/181**, dialog/profile/store tests. `ddl-ui-unit-final.log`; final combined application validation is recorded below |
| Complete pinned helper | **356/356**, `pnpm test:snowflake:helper`, Node 24.22.2 with the checked-in npm lockfile. `ddl-helper-all-pinned.log` |
| Actual pinned SDK DDL | **33/33** across HTTP, trusted TLS and CONNECT proxy; full actual-SDK guard run **179/179**. `ddl-helper-sdk.log`, `ddl-helper-sdk-full.log`. Loopback, preconnected identity seam; no SSO/account effects claimed |
| Adapter focused | **43/43**, then driver regressions **143/143**. `ddl-adapter-focused.log`, `ddl-adapter-shared-regression.log`; grant/policy/generation/expiry/catalog/context/cancel/ACK preservation |
| Studio DDL ownership | **13/13**, `ddl-studio-unit-receipts-final.log`; includes dropped waiter plus history/audit/both failures, blocked shutdown and local retry without another SQL call |
| Production-helper Studio fixtures | **5/5** pass together on final source in 97.84 seconds, `ddl-studio-helper-final-all.log`. Synthetic SDK with actual framed helper; includes lost ACK, final observer denial, dropped caller and after-ACK disconnect/shutdown. Earlier denied-catalog retry failure was repaired and rerun; its initial log remains `ddl-studio-helper.log` |
| Browser workflows | **14/14** in 55.8 seconds, `ddl-browser-rerun.log`: CREATE/nullable ADD/DROP, reviewed DML, typed boundaries, relation browsing, schema selection and session observations. New DDL cases exercise exact target confirmation, pending disconnect, two 31-second quiet windows, denied reconciliation retaining the hold and explicit successful retry. Real UI with intercepted synthetic RPCs; no SSO/account effects claimed |
| Actual disposable PostgreSQL regression | **43/43**, `JOINERY_TEST_PORT=56432 cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres -- --ignored --test-threads=1`. Fresh owned Compose project `joinery-ddl-20261111-0948`; real rollback/commit, schema DDL, cancellation, exact values, paging/export/import and conflict checks. `ddl-postgres-final.log`; this does not qualify Snowflake. Owned container, volume and network removed afterward (`ddl-postgres-cleanup.log`); browser servers also stopped |
| Hidden native application | SDK smoke build passes, then `python3 scripts/native-smoke.py --runs 3` passes **3/3**, 20 check categories and ten cleanup cycles each. The five new DDL commands reach Rust missing-session/missing-plan guards rather than command/ACL failures. `ddl-native-build.log`, `ddl-native-runs.log`, frozen `ddl-native-build.json` and `ddl-native-runs.json`. Synthetic native app; no account or normal packaged UI qualification |
| Final normal macOS package | Final `pnpm desktop:snowflake:bundle:mac`, exit 0, **224.88 MiB**. Strict ad-hoc signature, 15,107 runtime entries, zero symlinks and guarded SDK 3.4.1 startup pass with zero authentication/SQL requests. `ddl-normal-bundle-final.log`, `ddl-final-artifacts.json`. Local version 0.1.0; not installed, Developer ID/notarization and normal-app SSO remain open |
| Independent review | Helper, adapter and Studio reviews; retained-receipt finding fixed and re-reviewed. Final bounded UI/native enrollment review found no material issue. Bounded Tier 2 graph traces plus changed-source reads, best-effort coverage with no recorded issues on the reviewed paths; final generation `2026-11-11T10:16:17Z` |

Logs above are under `.cache/verification/connection-attempts/`. The first aggregate
helper invocation used pnpm 11 inside the npm-managed helper, which replaced its
dependency layout and caused failures (`ddl-helper-all.log`). Restoring the
checked-in `package-lock.json` using offline `npm ci --ignore-scripts` and changing
the root helper test wrapper to `npm --prefix snowflake-helper test` produced the
356-case passing run. No failing run is counted as a pass. Initial combined
validation found formatting and one missing fixture `primary` field; both were
corrected before the final validation run. Strict Clippy also found a complex
new return type; a named type alias resolved it without changing behavior. Native
command enrollment checks found the five new commands absent from the Tauri build
manifest/window ACL; both lists were updated and native guard coverage was added.

Reproduce focused checks with pinned Node/Rust in PATH and `JOINERY_TEST_NODE`
pointing to that Node executable:

```sh
pnpm test:snowflake:helper
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib ddl_studio -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib sdk_studio_ddl -- --nocapture
JOINERY_POSTGRES_PORT=56432 pnpm test:e2e tests/e2e/snowflake-sdk-ddl.spec.ts tests/e2e/snowflake-sdk-dml.spec.ts tests/e2e/snowflake-sdk-transactions.spec.ts tests/e2e/snowflake-sdk-browsing.spec.ts tests/e2e/snowflake-sdk-schema-switch.spec.ts tests/e2e/snowflake-sdk-session.spec.ts
pnpm desktop:snowflake:smoke:build
python3 scripts/native-smoke.py --runs 3
```

The browser command uses an isolated local PostgreSQL fixture already running on
56432 and requires free 1421/1422 test-server ports. The SDK route responses in
these six files are intercepted synthetic RPCs; use the separate actual-driver
fixtures above for framed-helper evidence. Never substitute an account URL or
real connection in these commands. The owned database fixture was removed after
this run, so recreate it with a new owned Compose project for reproduction.

The first browser run passed 10/14 and reproduced an enabled Browse relations
button while catalog reconciliation was held. Rust already rejected that path;
the UI now disables/hides browsing until the explicit refresh succeeds. The same
run found an older session fixture expecting disabled generic transaction buttons
that the current capability UI correctly omits; it now asserts absence and zero
both generic/typed transaction requests. `ddl-browser-final.log` retains that
initial result. Fresh browser and packaging results are recorded separately.

The hidden executable SHA-256 is
`77464d86608fe76d4ad3b357ad3b519b5d5db775e14db8b7bb8be7b0eb190f31`;
its isolated bundle executable is
`ad22ff70dac2c6b0b815afe52092092498edec6101695ad8ec988dbc84d21263`.
Native resources/startup checks pass with SDK 3.4.1, manifest
`dd08932407ed6a540379dc5711a464fa51600498196a8637af671e23ea9256a0`,
15,107 entries and five deliberate native-fixture symlinks. Guarded SDK startup
records zero authentication/SQL requests. The intermediate normal bundle also
passed signature/resources/startup, but preceded the final Workbench repair;
`ddl-normal-bundle.log` is not qualification of the final UI. The toolchain emits
a pre-existing `rust-objcopy`/missing `libLLVM.dylib` stripping warning; it does
not prevent successful builds. Global toolchain configuration was not changed.

The final normal bundle executable SHA-256 is
`7dab01832b4ea8c153c2a3b65aec2a06bf429787a7bd434fc1bf0b8e4688080d`.
All **426** frozen source/config/test inputs remain unchanged after native runs
and final packaging, including the last browser repair. The input manifest is
`ddl-source-qualified.json`, SHA-256
`6ed9640b83871fbc3578141c6c4246d1e6aebd3046829603b97986ff5be622c3`.
Documentation changed during these checks; it is outside that source manifest.
This does not qualify installation, normal packaged SSO, restart with live
Snowflake sessions, or complete application memory with one million account rows.

An independent existing-schema plan review confirmed the successful budget
`386 + 7S` (393 at one schema, 610 at 32), but identified prerequisites for the
next runner on that recorded source: automatic observations can occur inside an
application call after a rejected statement, and DML/transaction terminal
persistence lacked DDL's retained-receipt recovery. DML returned local persistence
failure; transaction controls discarded a failed terminal audit append. Those
recording defects were subsequently reproduced and repaired in the receipt
increment above, with separate coverage. The new
plan must disclose and enforce its failure-observation policy, retain original
acknowledgements without remote replay, and report unfinished local recording
through graceful shutdown. Source review used Tier 2 generation
`2026-11-11T10:26:13Z` and direct reads; it did not execute account SQL.
The revised proposal permits only the active call's already-budgeted read-only
post-observation suffix after rejection, then stops before every later phase.
This preserves the production path and the 610 maximum. It still requires a
schema-census callback and inbound resource counters; it does not claim an
outbound gate alone can freeze hidden schema results or enforce inbound caps.

The [migration walkthrough](migration-from-datagrip.md#reviewed-sdk-table-ddl-prepared-acceptance-walkthrough)
records owner acceptance steps, including explicit catalog reconciliation. Complete
Snowflake DDL, grid/import, broader SQL/session behavior, actual cloud cancellation,
normal packaged restarts and million-row account workflows remain unqualified.
The private project hub is not published because this Goal forbids publishing.

### Approved disposable setup attempt (11 November 2026)

The owner explicitly approved one helper-direct disposable setup attempt for
execution plan `60844de7e1a65152c4adbfd8dd4d708ccc14873fe06445f0eed85fdd9560658a`,
ledger `83458feb13d79f8829fe0b87d02f93cc14f1c179136dd71dc9e26d2a8ffe5a25`,
runtime manifest `16b1aa4ea0ed0e3435570fc098d6a6aee56b831dfa2697ef7b72327370378da4`,
inside the 11 November 2026 03:57:48.859 UTC through 12 November 2026 03:57:48.859 UTC
window, including external-browser SSO, up to 28 fixed SQL statements (three
CREATEs), and possible billing.

The single attempt completed at **11 November 2026 05:41:35.426 UTC** with
**qualification failed** (`failureCode: setupHelperOperationFailed`). Steps 1–7
succeeded and validated on the owner's actual external-browser SSO connection:
`startup.identity`, `startup.version`, `before.context`,
`before.quotedIdentifierPolicy`, `before.autocommit`, `before.abortOnError` and
`before.transaction` (confirming idle `AUTOCOMMIT=true` and
`QUOTED_IDENTIFIERS_IGNORE_CASE=false` before the first DDL statement). Step 8
(`create.database`, `CREATE TRANSIENT DATABASE "JOINERY_ACCEPTANCE_<RUN>" ...`)
received a definitive database rejection before acknowledgement
(`remoteOutcome: rejected`, `terminalErrorCode: database`,
`validationStatus: failed`). No later ledger statement (steps 9–28, including
`CREATE SCHEMA` and `CREATE TABLE`) was admitted or dispatched. As required by
the [disposable setup contract](snowflake-disposable-setup.md), the runner did
not switch roles, grant privileges, retry, or fall back to the existing
configured database/schema.

Upon the step 8 failure, the helper retired immediately (`stop()`), stdio closed
and the child exited with code 0 and was reaped (`childReaped: true`,
`closeObserved: true`; `logoutAcknowledged: false` because helper retirement on
setup failure destroys the connection and exits directly without waiting for a
separate close RPC reply). No `SIGTERM`, `SIGKILL` or cancellation was needed.
The original temporary authentication root identity was confirmed and removed
(`rootIdentityConfirmed: true`, `stateRemoved: true`), all pinned source and
runtime hashes revalidated (`sourceRevalidated: true`), and both the private
hash-chained journal and runner-confirmed receipt (`finalizationStatus:
runnerConfirmed`, `evidencePersisted: true`) persisted cleanly (`52` inbound
frames, `12,410` inbound bytes, `0` stderr bytes).

Safe log: `.cache/verification/connection-attempts/setup-owner-attempt.log` and
preflight `.cache/verification/connection-attempts/setup-owner-approved-preflight.log`.
Both permanent consumption markers (`setup-consumed-plan-*` and
`setup-consumed-run-*`), the private journal and the private result remain under
`.cache/snowflake-qualification`; no profile, object names, session/query IDs,
tokens or raw diagnostics are committed. **Approval and the generated run token
are permanently consumed; no retry was attempted.** The owner subsequently
permitted new tables in `SANDBOX.BILLIE_TESTING`; the
[new existing-schema proposal](snowflake-existing-schema-qualification.md) avoids
database/schema creation. Its executable runner, exact plan and fresh approval
remain to be completed. Actual account writes, DDL and transaction effects remain
unqualified.

### Actual-SDK million-row gzip chunk streaming (11 November 2026)

New helper tests `snowflake-helper/test/sdk-large-result-probe.mjs` and
`snowflake-helper/test/sdk-large-result.test.mjs` pass **2/2 in 9.47 seconds**
with pinned Node 24.22.2 (`--expose-gc --max-old-space-size=256`) and installed
`snowflake-sdk` `3.4.1` against an owned loopback HTTP server using synthetic
OAuth and lazily generated 10,000-row gzip-compressed storage chunks
(`content-encoding: gzip`, comma-separated JSON row fragments). Non-loopback
socket connections are denied (`actualNetworkDenied: 0`).

- **`million` scenario:** Streams all **1,000,000 rows** across **100 gzip
  chunks** (`5,966,624` compressed bytes / `124,006,438` uncompressed bytes) and
  verifies every row's exact 38-digit integer above 2^53, signed 38-digit
  decimal, quoted/newline/Unicode text and `null`/empty strings. Default SDK
  chunk prefetch is preserved (`chunksBeforeStream: 0`; pausing at row 128
  observes `pausedChunkRequests: 3` and `pausedReadableLength: 10` within
  `rowHighWaterMark: 10`). Sampled peak RSS for the SDK and in-process lazy
  chunk server combined is `304,414,720` bytes (~290.31 MiB; `maxRSS` `297,280`
  KiB) and peak `heapUsed` is `96,917,112` bytes (~92.43 MiB), returning to
  `27,877,592` bytes (~26.59 MiB) after cleanup and GC.
- **`cancel` scenario:** Pauses at row 128 (`3` chunk requests), destroys the
  local row stream and calls `statement.cancel()`, observing `1` synthetic abort
  request and zero additional chunk downloads (`chunksAfterCancel: 3`,
  `consumedRows: 128`).
- Independent review (`setup_runtime_review`) found no material issues. Log
  `.cache/verification/connection-attempts/sdk-million-chunks-final.log`.
- Reproduce: `.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node --test
  snowflake-helper/test/sdk-large-result.test.mjs`. This qualifies local SDK
  chunk decompression, backpressure and stream-destroy behavior only; remote
  Snowflake cloud-storage chunk URLs, external-browser SSO, remote cancellation
  disposition, natural SDK process exit without explicit child exit, and end-to-end
  packaged-application memory bounds remain unqualified.

### Disposable setup runner (11 November 2026, offline qualification)

At the owner's request, prepared a separate helper-only setup workflow. It has
exactly 28 fixed SQL statements, including three CREATEs for one fresh transient
database, its SCRATCH schema and empty ROW_CHECKS table. There is no arbitrary
SQL, DML, transaction command, existing-table read, role/grant/warehouse change,
retry or cleanup SQL. At this recorded offline checkpoint no setup had run. The later consumed owner
attempt and failure are recorded above; prior approvals grant no retry authority. See the
[exact ledger, charges and execution gate](snowflake-disposable-setup.md).

| Check | Result and evidence scope |
| --- | --- |
| Full application validation | `pnpm validate`: **671 frontend / 437 standard Rust tests**, **129 ignored** fixtures; formatting, production frontend build, Rust check and strict Clippy pass. `setup-validate-final.log`, exit 0 |
| Complete helper suite | **305/305**, correct helper working directory, 172.81 seconds. Includes existing reads/DML/transactions/process/million-row tests and new fixed setup cases. `.cache/verification/setup-helper-final-all.log` |
| New actual pinned-SDK setup cases | **39/39** across HTTP, verified TLS and CONNECT proxy: one POST/effect per CREATE, private results, lost ACK, rejection, drain failure, guard retirement, cancel, local EOF and explicit close before/after ACK. Bootstrap/prior setup cursor is fixture-seeded in this probe; it does not prove SSO or the complete admission sequence. `.cache/verification/setup-helper-close-sdk-after-fix.log` |
| Complete qualification scripts | **136/136**, including **17** runtime tests using the production Helper with synthetic SDK and a full 28-statement run. Lost ACK at each CREATE, definitive rejection, partial creation, journal barriers/failures, bounded deadline, owner interruption, no replay after restart and receipt failures pass. `setup-qualification-finalization-final.log` |
| Compiler/resources | **75/75** read/setup compiler and runtime-resource cases. Closed SQL/shape/hash binding, deep immutability, private exclusive CLI output, file allowlists, setup windows up to 24 hours and unchanged ten-minute read window pass. `setup-compiler-resources-final.log` |
| Offline runtime | Rebuilt from the repository-owned npm cache with `pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline`; staged startup passes SDK 3.4.1 hello/EOF with **zero authentication / SQL requests**. `setup-runtime-prepare.log`, `setup-staged-startup.log` |
| Actual offline owner plan | Emission and exact-plan verification pass with the retained current-connection allowlist and original proposal token; no helper is launched. `setup-owner-offline-emission.log`, `setup-owner-offline-verified.log` |
| Current normal SDK macOS bundle | **224.33 MiB**, strict ad-hoc signature valid, **15,107 resource entries / zero symlinks**. Packaged SDK 3.4.1 hello/EOF sends **zero authentication / SQL requests**. `setup-bundle.log`, exit 0. Built but not installed; no normal-app sign-in or new hidden-native run is claimed |
| Independent review | Helper and runtime reviews found and resolved the close-before-ACK outcome distinction and receipt durability findings below; final bounded re-review has no unresolved material finding. This is local evidence, not remote exactly-once or account-effect proof |

Logs without a directory prefix above are under
`.cache/verification/connection-attempts/`. One earlier helper aggregate used the
wrong working directory and failed four process tests; the final correct-directory
305-case run supersedes it. The new regression failures were reproduced before
their repairs: `setup-helper-close-before-fix.log` under `.cache/verification/`,
and `setup-receipt-before-fix.log` / `setup-finalization-before-fix.log` under the
connection-attempts directory. Final affected runs pass. No failure is counted as
a pass.

Exclusive plan and account/host/run markers, with directory sync, precede SSO.
The journal syncs each intent and each CREATE ACK before admitting the next SQL.
Known acknowledgements remain separate from private result validation and complete
qualification. Close before ACK reports unknown delivery; after ACK retains known
success. Journal-close failures preserve known outcomes and stop qualification.
Final receipts stay provisional (`requiresRunnerConfirmation`, qualification and
evidence flags false); only the returned runner report confirms final file and
directory sync. This prevents an already-written receipt from claiming success
if that final sync fails. No automatic overwrite or replay occurs.

Execution plan SHA-256:
`60844de7e1a65152c4adbfd8dd4d708ccc14873fe06445f0eed85fdd9560658a`;
ledger SHA-256:
`83458feb13d79f8829fe0b87d02f93cc14f1c179136dd71dc9e26d2a8ffe5a25`;
runtime manifest SHA-256:
`16b1aa4ea0ed0e3435570fc098d6a6aee56b831dfa2697ef7b72327370378da4`.
The proposed single-attempt window is **11 November 2026 03:57:48.859 UTC through
12 November 2026 03:57:48.859 UTC**. These hashes and window are preparation,
not approval. Source drift, consumed markers, expiry or changed profile deny
execution. This helper-only setup does not enable application DDL, qualify DML
or transaction effects, or close the broader DataGrip replacement Goal.

Commits `fa0fd45` (helper/resources) and `4f787a1` (runner/tests/documentation)
are local only. All **347 frozen source/tooling/test inputs** are unchanged
through final packaging (`setup-source-freeze.json`, `setup-source-qualified.json`).
The normal application executable SHA-256 is
`339799ebfe1d10f4d6e6970a5a2e9dcda53003f4fdba1e06583f93cbaed364c5`.
The existing missing-`libLLVM.dylib` debug-strip warning repeats; Cargo and bundle
verification still finish successfully. Developer ID/notarization, installation,
normal-app credentials/restart and owner acceptance remain open. The private
project hub was not published because this Goal explicitly forbids publishing.

### Million-row Snowflake driver/store/export bridge (11 November 2026)

New integration test `synthetic_million_rows_stream_through_driver_store_and_csv_hash`
passes **1/1 in 6.91 seconds** using pinned Node 24.22.2, the real framed helper
child and Rust Snowflake driver with an injected lazy synthetic SDK stream.
All one million rows enter an 8 MiB RAM-budget ResultStore with anonymous disk
spooling explicitly enabled. Exact values above 2^53 survive sparse/random reads
and the final page; a complete CSV export matches an independently generated
SHA-256 and exact row count. Release returns store/disk accounting to zero.
Log `.cache/verification/connection-attempts/snowflake-million-bridge.log`.
Rust formatting and strict all-target/all-feature Clippy pass separately.

Reproduce: `JOINERY_TEST_NODE=/absolute/path/to/pinned/node cargo test
--manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness
--test snowflake_external synthetic_million_rows_stream_through_driver_store_and_csv_hash
-- --exact`. No account activity or existing data is used. This bridges previously
separate helper/driver and result-store/export tests; it is not a Studio/UI run,
a remote Snowflake benchmark, pinned SDK chunk/decompression-memory measurement,
or proof of total-process RSS bounds. Those required performance gates stay open.

### Pure Snowflake DDL syntax foundation (11 November 2026)

The separately enrolled `safety::snowflake_ddl` module generates exactly one
CREATE TABLE, nullable ADD COLUMN or DROP TABLE RESTRICT from a structured
action and explicit database. There is no application DDL execution route,
profile permission or account qualification. Broader DDL remains required.

- Full final `pnpm validate`: **671 frontend / 437 standard Rust tests**, 129
  ignored fixture tests; formatting/contracts, production frontend build, Rust
  check and strict all-target/all-feature Clippy pass. Log
  `.cache/verification/snowflake-ddl-pure/validate-final.log`, exit 0.
- All **11** full-crate inline generator tests pass, including exact identifier
  escaping, closed scalar types, forbidden supplied fields/options, action
  variants, duplicate names, bounds and whole-AST roundtrip. Initial crate run
  `crate-tests.log`; final validation includes the repaired tests.
- Independent review found invalid NUMBER scale 38; lead review found standard
  DROP's implicit CASCADE default. Three isolated assertions reproduced failure
  before changes (`restrict-scale-before-fix.log`); all 11 pass afterward
  (`restrict-scale-after-fix.log`). Generate explicit RESTRICT and enforce scale
  at most 37. Independent recheck found no further material issue in this scope.
- CREATE's explicit NOT NULL is intentionally allowed only on its new empty
  table, with the design corrected to state this. ADD remains nullable. Pure
  syntax does not establish runtime quoted-case policy, complete dependencies,
  atomic identity, idle transactions or permission.

Reproduce with pinned toolchains: `cargo test --manifest-path src-tauri/Cargo.toml
--no-default-features --lib generated_ddl_ -- --test-threads=1`, then
`pnpm validate`. No account, database fixtures or browser actions are needed for
this unused pure generator. The preceding normal DML package/native artifacts
remain pinned to `fb091db`; they do not claim qualification of later sources.
The [DDL design](snowflake-sdk-ddl-design.md) records remaining integration gates.

### SDK reviewed SQL DML (11 November 2026)

Implemented in two local commits: `eae1e81` (strict grammar, typed helper/adapter
transport and exact summary evidence) and `fb091db` (reviewed editor workflow,
history, metadata hold and graceful shutdown). Neither is merged, published or
installed. These commits do not qualify actual Snowflake DML, transaction effects,
DDL, complete results or owner replacement suitability.

The profile's `reviewedDml` opt-in defaults false. ReadOnly and OAuth deny writes.
The editor admits only a strict single INSERT VALUES, plain UPDATE or plain DELETE
against an explicit three-component name in the observed database, with bounded
scalar binds. Every write is reviewed and one-use; canonical SQL/binds, fresh
revision, current persisted policy and passphrase are rechecked before dispatch.
Generic SDK execution still denies writes. No grid-edit/immutable relation identity
claim is made: review warns about concurrent DDL, temporary shadows, defaults,
policies and dependencies. Broader syntax remains a required gap.

Counts are exact decimal strings from a bounded one-row status result, never SDK
numeric approximations or status-row counts. UPDATE's optional multi-joined count
stays separate. Missing/malformed evidence is unavailable, not zero. Known ACKs
survive observer/drain/history/UI-waiter failure; they do not independently prove
commit or row disposition. Unknown delivery retires without replay. Ordinary
observer denial can retain same-session rollback-only recovery.

| Check | Result and evidence scope |
| --- | --- |
| Final `pnpm validate` | **671 frontend tests / 56 files; 426 standard Rust tests**. Formatting, contracts/types, frontend production build, Rust check and strict all-target/all-feature Clippy pass. **129 ignored fixture tests are not passes**. Log `dml-validate-qualified.log` |
| Independent transport commit | Source-only snapshot of the exact staged index builds and passes **37 DML Rust tests**, including strict AST/binds, exact summaries, supervisor and adapter cases; `dml-transport-index-tests.log`. The snapshot excluded the dependent uncommitted UI/Studio workflow |
| Rust DML qualification | **51/51** focused tests before the later shutdown repair; final **15/15** Studio RecordingDriver tests include that repair. Earlier adapter suite **41/41**. Logs `dml-rust-focused-final.log`, `dml-shutdown-studio-suite.log`; these are local behavior, not account effects |
| Helper / actual pinned SDK | **255 cases covered passing across two runs**: 107 actual-SDK loopback cases and 148 remaining helper cases. Includes 27 new DML fault cases across HTTP, verified TLS and CONNECT proxy, precise one-POST/effect assertions and no public status rows. SDK EOF seam uses `Helper.stop()`, not actual child IPC EOF. Logs `dml-sdk-full.log` and `dml-helper-nonsdk-final.log` |
| Studio + actual Node/helper + synthetic SDK | **21/21** complete fixture cases, including five new DML cases: canonical repeated binds, exact counts/history across local SQLite restart, eleven-SQL cadence, denied final observer, lost ACK/no replay, queued metadata and dropped IPC waiter. `dml-studio-integrated-full.log`, 337.64 seconds. This full run precedes the subsequent TaskTracker repair; the 15 final Studio tests and full validation qualify that repair separately |
| Browser | **7/7** headless isolated flows: four DML outcomes (normal, rollback-only, unknown, explicit pending-write disconnect) plus three transaction regressions. Keyboard Enter approves exact SQL/typed parameters; counts above 2^53 stay exact; prior metadata stays quiet across simulated 31-second intervals and resumes only on explicit coherent refresh. `dml-browser-final.log`, 35.7 seconds. RPCs are intercepted: UI evidence only |
| Actual PostgreSQL regressions | **43/43** ignored fixture tests explicitly executed on a fresh `joinery-dml-acceptance` Compose project with synthetic data; `dml-postgres-regression.log`, 28.53 seconds. Includes manual transaction/error recovery, exact values, history-adjacent persistence paths, reviewed writes, DDL, imports, cancellation and shared execution. Its container, private fixture volume and network were removed afterward; the unrelated running container was preserved. This does not prove Snowflake semantics |
| Hidden SDK native app | **3/3 runs**, **20 check categories** and **10 cleanup cycles** each. DML IPC reaches its Rust missing-plan guard; negative enrollment only, not positive account DML. Packaged SDK hello/EOF reports zero authentication/SQL. Logs/receipts `dml-native-build.log`, `dml-native-runs.log`, `dml-native-build.json`, `dml-native-runs.json` |
| Normal SDK macOS bundle | **224.33 MiB**, strict ad-hoc signature valid; **15,106 resource entries, zero symlinks**. Actual packaged SDK 3.4.1 guarded hello/EOF sends **zero authentication/SQL requests**. Logs/receipt `dml-bundle-final.log`, `dml-bundle-final.json`; build exit 0. No normal-app sign-in, installation or release |
| Independent review | Adapter/helper, transport, Studio and frontend reviews resolved material findings. Final read-only review found no outstanding material issue in the bounded scope. Setup SQL/cleanup semantics also reviewed; no account activity |

The normal bundle executable SHA-256 is
`3fd244000f171dfc8c124d21549ff4bfb01f9dccf70ccbc4d8c51475012abd6c`;
the hidden native executable is
`2c2cb0641d3e366d56e2556d624ff981a0671757841c62fdaa6b7d1bf36333cb`.
Both use runtime manifest
`4103035eec7994b25a14b823680c21d41b0da08c784df858fa803f2dab0066a8`.
All **336 frozen application/runtime/tooling/test inputs** are unchanged through
commit `fb091db` and these checks (`dml-source-freeze.json`,
`dml-source-qualified.json`). The new, unenrolled pure DDL module is outside this
DML package qualification. Build warnings include the pinned Rust toolchain's
missing `libLLVM.dylib` for debug stripping (Cargo continued successfully),
ts-rs ignoring a serde attribute during type generation, and a frontend chunk
size warning. Runtime serde validation remains enforced. Developer ID signing,
notarization, normal-app restart/credentials and owner acceptance remain open.

Normal DML dispatch is five baseline observer reads, one DML and five final reads
on the existing session. Preparation sends zero SQL. No automatic BEGIN/COMMIT,
generic ten-read follow-up or metadata refresh is added. Session settings and
actual commit semantics still need approved account testing. At the native lane,
health, completion and browsing recheck a revisioned metadata hold before driver
entry. Only a deliberate refresh with coherent transaction observation and normal
authority clears it; unavailable or rollback-only refresh preserves the hold.

Reproduced failures and fixes:

- The first helper run failed one synthetic DML fixture missing its submission
  guard method and four process fixtures launched from the wrong directory.
  Corrected fixture setup/cwd passed the 148 non-SDK cases. All 107 actual-SDK
  cases passed in that initial full log; do not read its overall failure as a
  single 255-case pass.
- A supervisor regression initially dropped the fixture's channel receivers and
  misreported replacement admission as a full queue. Retaining those fixture
  receivers fixed the test; no product bypass was introduced.
- Initial browser run: all three new DML flows failed because the busy review
  modal hid the pending-write disconnect control; all three existing transaction
  cases passed. The review now closes after authorization and guards late closure
  by plan ID. The corrected six flows passed, then the final seven also exercised
  clicking disconnect and retaining the later unknown outcome.
- Review found dropped IPC waiters could lose outcome history. A Studio-owned
  task now finishes evidence independently. A subsequent focused regression
  reproduced graceful shutdown returning before that owner persisted its ACK.
  TaskTracker admission/close share the configuration gate; graceful shutdown
  waits for owners before success. A ten-second drain timeout leaves the task
  alive and the window open. The strengthened regression passes and checks
  history exists when shutdown returns, exact counts, retirement and rejected
  late admission. Logs `dml-shutdown-before-fix.log` (expected failure),
  `dml-shutdown-after-fix.log` and `dml-shutdown-studio-suite.log`.

Reproduce locally with pinned Node 24.22.2 / Rust 1.99.1 and no account sign-in:

```sh
pnpm validate
pnpm --dir snowflake-helper test
JOINERY_TEST_NODE=/absolute/path/to/pinned/node cargo test \
  --manifest-path src-tauri/Cargo.toml --no-default-features \
  --features test-harness --lib dml -- --test-threads=1
JOINERY_TEST_NODE=/absolute/path/to/pinned/node cargo test \
  --manifest-path src-tauri/Cargo.toml --no-default-features \
  --features test-harness --lib execution::snowflake_connection_tests \
  -- --test-threads=1
pnpm exec playwright test tests/e2e/snowflake-sdk-dml.spec.ts \
  tests/e2e/snowflake-sdk-transactions.spec.ts
```

The owner requested a [disposable setup plan](snowflake-disposable-setup.md).
Six exact proposed setup statements and one separately gated cleanup statement
are rendered in a private 0600 offline proposal. It has no executable runner,
runtime provenance, execution budget or authorization; **zero account statements
were executed** for this increment. Build/review the bounded setup runner before
requesting fresh execution/billing approval. Prior read approvals are consumed.
The [DDL design](snowflake-sdk-ddl-design.md) is proposed and not implemented.

During local testing, an agent mistakenly printed the environment in a tool
response, exposing GitHub/Orca token values there. No raw values were stored in
repository files, test fixtures or committed evidence. The owner was informed;
subsequent diagnostics were restricted to named non-secret variables. Tokens are
not reproduced here and no global configuration or credential was changed.

### SDK explicit transaction application controls (10 November 2026, locally qualified)

The increment adds reviewed SDK BEGIN/COMMIT/ROLLBACK buttons, separate typed
prepare/execute IPC, one-use revision-bound authority and persisted policy
rechecks. SDK profiles default ReadOnly; only deliberate external-browser policy
selection permits BEGIN/COMMIT. ROLLBACK keeps its recovery exemption. OAuth,
legacy profiles and public SQL remain read-only. No account transaction or write
activity is authorized by this increment.

Local checks pass with the repository-pinned toolchains:

| Check | Result and scope |
| --- | --- |
| Full `pnpm validate` | 659 frontend tests in 54 files; 384 Rust tests; formatting, contracts/types, production frontend build, Rust check and strict all-target/all-feature Clippy. The standard Rust run reports 129 ignored fixture tests, not passes |
| SDK adapter | 31 internal and 31 separate integration cases, with actual helper children and synthetic SDKs; stale/cross-session/policy rejection, cancellation, known ACK retention, protocol loss and cleanup |
| Studio/helper | Corrected full suite 16/16. Its binary predates the last Unknown-outcome safeguard; all six affected final-source cases also pass in 99.24 seconds, including normal/recovery budgets and lost ACK |
| Studio policy/races | Seven RecordingDriver tests pass: one-use exact-plan approvals, production passphrase, policy drift, generic-route exclusion, audit-failure rollback exemption, queued invalidation, disconnect and mandatory Unknown retirement. These do not establish database semantics |
| Profile persistence | Actual local SQLite restart and metadata backup/restore across all five SDK policies; ReadOnly preserved and production normalization checked. No OS keychain or account qualification claim |
| Helper | 216 tests pass, including actual pinned-SDK loopback transport/transaction fault cases; no account activity |
| Browser | Three new transaction flows pass on final frozen spec; seven related existing flows passed in the prior run. RPC-intercepted UI behavior, not browser-to-real-Snowflake execution |
| SDK hidden native | Three runs pass 20 check categories and ten cleanup cycles each. Typed IPC reaches disconnected-session/missing-plan guards; this is negative enrollment, not a positive account transaction |

Normal BEGIN/COMMIT uses five baseline reads, one boundary and five final reads;
recovery ROLLBACK uses one boundary and five final reads with no mandatory preread.
Studio publishes returned observations without its generic ten-read follow-up.
Ordinary observer denial retains rollback-only ownership; transport/protocol
uncertainty retires without replay. Command ACK remains distinct from verified
transaction/data effects. Adapter counts are synthetic evidence, not an account
SQL approval or total network-cost bound.

Independent review found and root repaired mandatory auditing that could trap
rollback, late disconnect authority, automatic post-boundary metadata reloads and
delayed query events replacing recovery. Editor/health metadata stays paused
until owner refresh. A stale relation browser cannot remount, including with a
fast batched response. Unsettled sibling queries block a boundary before tab
revision invalidation. Final guard review cleared mandatory Unknown retirement
in Rust and the UI even when a driver omits its retirement flag.

Browser checks cover keyboard focus/Enter, held BEGIN exclusion, denied-observer
recovery and no-approval ROLLBACK; Unknown retirement without replay; and immediate
COMMIT with a 31-second quiet window. A metadata browser revokes prior authority,
so the owner explicitly refreshes before BEGIN. During rollback-only recovery,
the refresh button stays disabled; the refresh hint appears after recovery.
Standalone E2E TypeScript and formatting pass. Final spec SHA-256:
`4436e1f6f66e7383b339656029a209fc8940a81ff3cb1aa719fddea7321d0e28`.

Failed attempts remain evidence. The first Studio fixture run was 14/16 because
its helper wrongly required ROLLBACK approval; corrected fixtures passed 16/16.
Browser attempts were 7/10, 8/10 and 9/10 before fixing synthetic plan ownership,
metadata-authority refresh, no-approval wire expectations and duplicate/priority
status locators. No product behavior was changed to satisfy those fixture
mistakes. Logs are `sdk-transactions-studio-interim.log`,
`sdk-transactions-browser-interim.log`, `sdk-transactions-browser-final.log` and
`sdk-transactions-browser-qualified.log`; filenames do not imply a pass. Failed
traces are retained under `sdk-transactions-browser-final-failures/` and
`sdk-transactions-browser-qualified-failure/`. The final control pass is
`sdk-transactions-browser-controls-final.log`.

The initial and final source freezes each record 324 application/runtime/test/
tooling inputs, based on `b9673ab` plus the owned increment. Only the synthetic
browser fixture changed after full validation; all product inputs stayed fixed.
Receipts are `sdk-transactions-source-freeze.json` and
`sdk-transactions-qualified-source-freeze.json`. The isolated SDK native build
passes packaged resource/startup checks with zero authentication and SQL requests.
Copied native build/run receipts and logs have prefix `sdk-transactions-native-`.
The known nonfatal `rust-objcopy`/`libLLVM.dylib` stripping warning persists.
Normal SDK application packaging passes for commit `929ae06`: **224.14 MiB**,
strict ad-hoc signature verification, **15,106 resource entries and zero symlinks**,
and actual packaged SDK hello/EOF with **zero authentication/SQL requests**.
The executable is 31,669,808 bytes, SHA-256
`f1e2660b3455c6a5cd24477e824c6ed5265595fe26e90a12af5ae58da7d4f40c`.
All 324 frozen inputs remain unchanged through packaging and exactly match the
committed source/test files. Evidence: `sdk-transactions-bundle-final.log` and
`sdk-transactions-bundle-artifact.json`. The runnable local application is
`src-tauri/target/release/bundle/macos/Joinery.app`. It is not installed or launched
against private app state; Developer ID, notarization, account effects and owner
acceptance remain open.

Reproducible commands (set `JOINERY_TEST_NODE` to the pinned Node binary):

```sh
pnpm validate
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib explicit_boundary -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib drivers::snowflake::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib drivers::snowflake_external::session_tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --test snowflake_external -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib execution::snowflake_connection_tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib sdk_studio_ -- --nocapture
pnpm test:snowflake:helper
pnpm exec playwright test tests/e2e/snowflake-sdk-transactions.spec.ts
pnpm desktop:snowflake:smoke:build
python3 scripts/native-smoke.py
pnpm desktop:snowflake:bundle:mac
```

Evidence is under `.cache/verification/connection-attempts/` with prefix
`sdk-transactions-`. The [transaction contract](snowflake-sdk-transactions-design.md)
and [migration walkthrough](migration-from-datagrip.md#explicit-sdk-transaction-controls-local-acceptance-walkthrough)
record behavior and remaining actual-account/owner acceptance. DML, DDL,
AUTOCOMMIT changes, account effects and complete replacement remain open.

### Corrected account read/metadata qualification (10 November 2026)

The owner freshly approved exactly one attempt for execution plan
`4133ffac4585e8167b207e1dc435322d661e0d561963090186c817ed2ccc71b0`,
ledger `92fd58414eba58c8c5e11c3554502fbda63980df6d0458ff3a047329a40fd35b`,
source `29bd71a`, runtime manifest
`375e0639ee6d2285b25fd4e9646a1078b6302faa1630a9bcd002ed653ab23114`,
inside the 12:56:16–13:06:16 UTC admission window, including possible billing.
The actual helper-direct attempt **passed at 12:59:21 UTC**. All **24 planned SQL
statements succeeded and validated**: startup identity/version, coherent before/
after context and session parameters, bounded current-database schema discovery,
synthetic scalar values and final state observations. This includes the corrected
`TO_VARCHAR(CURRENT_SCHEMAS())` query on the owner's actual connection.

The protocol completed its ledger with 26 commands and one additional credit;
all framing and aggregate result budgets passed (43,517 inbound bytes, 196 frames;
12,956 retained result bytes across 66 rows over the complete ledger). These are
aggregate counts, not a schema-count disclosure or representative performance
benchmark. No existing table data, schema switch, DML, DDL or transaction boundary
was requested. SDK network traffic and possible charges are not limited by the
helper SQL count.

Logout/close acknowledgement, stdin completion, closed stdout/stderr, normal
exit code 0, actual child reaping, original authentication-directory identity and
removal, source/runtime revalidation and private receipt persistence all passed.
No cancellation, TERM or KILL was needed. These observations do not prove an
account-wide session inventory or remote transaction disposition. The exclusive
approval marker is consumed; this success grants no further account access.

Safe log: `.cache/verification/connection-attempts/`
`sdk-context-normalization-owner-attempt.log`; offline plan summary:
`sdk-context-normalization-plan.json`. Private input/plan/approval/result remain
under `.cache/snowflake-qualification`; no profile, schema names, session/query
identifiers, tokens or raw diagnostics are committed. The runner command is
documented in [the execution boundary](snowflake-qualification-runner.md).

This qualifies the fixed helper workflow and its actual account result shapes.
Normal packaged application sign-in, browsing/completion, restart, cancellation,
million-row results, writes, schema changes, explicit transactions and owner
replacement acceptance remain required and incomplete. The earlier failed
attempt below remains part of the evidence.

The same `29bd71a` application source also passes a fresh
`pnpm desktop:snowflake:bundle:mac`: normal **224.02 MiB** macOS arm64 bundle,
strict ad-hoc signature/resource validation (15,106 entries, no dependency links),
and actual packaged SDK hello/EOF startup with **zero authentication/SQL requests**.
All **322 frozen application/tooling inputs** remain unchanged through the build.
The 31,548,464-byte executable SHA-256 is
`d778e0dca71f9f7f4a1dc74f42c877278e84bc4e1575f1eca73e20c90a5da73a`.
The runnable bundle is `src-tauri/target/release/bundle/macos/Joinery.app`;
it was not installed or launched against private application state. Developer ID,
notarization and normal account/native restart acceptance remain open. The known
nonfatal `rust-objcopy`/`libLLVM.dylib` stripping warning remains unchanged.
Logs/receipt: `sdk-context-normalization-bundle-final.log` and
`sdk-context-normalization-bundle-artifact.json` in the verification directory.
The earlier three hidden native runs predate this SQL-only correction; this
fresh normal bundle does not claim a new native UI qualification run.

### SDK context text normalization (10 November 2026)

The SDK context query and its exact qualification template now use
`TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH`. Snowflake's
[TO_VARCHAR contract](https://docs.snowflake.com/en/sql-reference/functions/to_char)
accepts any type, returns VARCHAR and serializes ARRAY inputs as JSON, while
[TO_JSON](https://docs.snowflake.com/en/sql-reference/functions/to_json) requires
VARIANT. The CURRENT_SCHEMAS reference shows a JSON list without declaring its SQL
type. This is a documented normalization correction, **not confirmed attribution**
of the generic rejection in the account attempt below. No additional account
authentication or SQL was performed during the local correction. The later
approved account attempt and fresh normal package are recorded above; the
preceding hidden native package predates this correction.

The production adapter and offline compiler retain identical SQL. Strict JSON
path/secondary-role parsing, required descriptors, byte/row caps, all source
checks and the 24/10/11/13 statement counts are unchanged. A new parser regression
preserves quoted components, whitespace and empty string roles while rejecting
double-encoded arrays and wrong JSON types. No fallback or second decode was added.

The exact-ledger regression failed before repair (exit 1), and an actual Node/
production-helper synthetic-SDK adapter regression failed with the old production
query (exit 101). The fixture independently requires the corrected SQL and
explicitly rejects the old expression. Final checks pass: **113 qualification
tooling / 16 metadata / 20 session-switch adapter / 31 adapter integration tests**,
scoped formatting and strict test-harness Clippy. Independent review reports no
material finding, with a separate **51/51** manifest/result/protocol run. These
fixtures establish local routing/validation behavior, not actual SQL compilation.

Full `pnpm validate` passes **649 frontend / 374 standard Rust tests**, 129 opt-in
checks ignored, plus contracts, TypeScript/web build, Rustfmt, desktop check and
strict all-target/all-feature Clippy. Root also reproduced a stale exact context
SQL expectation in the opt-in Studio fixture, then updated that expectation and
the proposed qualification-plan SQL. Production source is unchanged by those
two follow-ups. All **13/13** opt-in Studio connection tests then pass using the
actual helper subprocess and synthetic SDK, including interrupted authentication,
context/session drift, denied post-query observations and schema options. This
does not add actual Snowflake account evidence. Independent follow-up review
cleared both changed expectations without material findings.

Logs under `.cache/verification/connection-attempts/`:
`sdk-context-normalization-{manifest,adapter}-before-fix.log`,
`sdk-context-normalization-{tooling,metadata,adapter,integration,clippy,validate}-final.log`,
`sdk-context-normalization-studio-before-fixture-fix.log`,
`sdk-context-normalization-studio-final.log`.
The alternate legacy OAuth completion query still uses the older wrapper and
has no account qualification; this SDK-specific correction does not certify it.

### Approved account qualification attempt (10 November 2026)

The owner explicitly approved one helper-direct attempt for execution plan
`356b09068ff09f6096ee83e443f7aac0775a21e6bcfec700220e2fe27240da3c`,
ledger `5acab6327c378b055d2c99c7a7645eae1f0be2249f8e7450454fcb216ba8a3d1`,
inside the 12:26:22–12:36:22 UTC window, including external-browser SSO,
up to 24 fixed read/SHOW statements, bounded schema names and possible billing.
The single attempt ended at **12:31:29 UTC** with **qualification failed**.
Actual SSO/startup identity and server-version queries succeeded and validated.
The third statement, `options.before.context`, received a definitive database
rejection before acknowledgement. No later ledger statement was admitted, so
schema discovery, settings observations and the scalar group were not tested.
The helper intentionally retains no raw database error text; the receipt alone
does not establish which expression caused rejection.

The runner stopped at the first failure. Logout and close were acknowledged,
stdin/stdout/stderr closed, the child exited normally and was reaped, and the
original temporary authentication root was removed after identity verification.
No TERM/KILL or cancellation was needed. Source/runtime revalidation and private
receipt persistence passed. These are local cleanup observations, not an
account-wide session inventory or proof of remote transaction disposition.

Safe evidence: `.cache/verification/connection-attempts/`
`sdk-qualification-owner-attempt.log`. The exclusive consumed marker and private
result remain under `.cache/snowflake-qualification`; no profile, schema names,
tokens, query IDs or raw diagnostics are committed. **Approval is consumed; no
retry was attempted.** Any corrected account attempt needs a new exact plan and
fresh owner approval. The required daily account workflow remains unqualified.

### Transaction increment package and native checks (10 November 2026)

At source `c543ef5`, `pnpm desktop:snowflake:bundle:mac` passes a fresh normal
**224.02 MiB** macOS arm64 bundle, strict ad-hoc signature/resource checks and
actual packaged SDK hello/EOF startup with **zero authentication/SQL requests**.
The executable is 31,548,464 bytes, SHA-256
`99e2d0cbc922a997ba258002f50fa1f25504b6f3a8f320efe38098ae4cebdf48`.
It includes the synthetic-function policy repair and typed transaction transport;
application transaction actions remain disabled. Developer ID signing,
notarization, installation and normal visible/account startup are still open.

`pnpm desktop:snowflake:smoke:build` and `python3 scripts/native-smoke.py` pass:
three isolated hidden runs, each with **19 check categories / ten cleanup cycles**.
Native fixture elapsed times are 3,756 / 3,051 / 3,040 ms and sampled main-process
peak RSS is 100.88 / 100.61 / 100.97 MiB. These synthetic fixture measurements do
not qualify visible cold startup, total application memory or Snowflake workloads.
The hidden bundle executable SHA-256 is
`092285b00697eb36e045657053e2c0fff605f87a106144f781f75d840611acd5`.
Packaged runtime manifest:
`375e0639ee6d2285b25fd4e9646a1078b6302faa1630a9bcd002ed653ab23114`.

All **322 frozen application/runtime/test inputs** remain unchanged through both
builds. Logs and safe artifacts under `.cache/verification/connection-attempts/`:
`sdk-transaction-{bundle-final,native-build-final,native-final}.log`,
`sdk-transaction-{bundle-artifact,qualified-source,source-freeze}.json`.
The pre-existing missing-`libLLVM.dylib` debug-strip warning remains nonfatal;
release compilation and subsequent signature/resource/startup checks pass.

### Qualification execution boundary (10 November 2026)

Status: **locally verified and independently reviewed; approved account attempt failed above**.
The [runner contract](snowflake-qualification-runner.md) binds the unchanged
offline-v1 24-statement ledger, five executable-module hashes, runtime evidence,
profile/window and physical caps in a separate execution-plan envelope. No
authentication, SQL or execution consent is granted by producing that envelope.
The private approval record must match the complete plan and explicit scope;
the operator may create it only after fresh owner consent. A deterministic
exclusive consumed marker is synced before launch and is never removed or reused.

Independent review found three P2 issues, all repaired with regressions:
directory sync was missing for the new consumption marker; an actual failed OS
launch was conservatively but inaccurately reported as a spawned/unreaped child;
and emergency cleanup could leave pipes open after an already observed exit.
The final implementation syncs the validated directory before auth allocation,
distinguishes spawn return/launch/no-child/exit evidence, and closes owned pipes
before the emergency early return. Replacement authentication roots are preserved.

The first runner test run passed 11/12; the failure was a test's overly specific
ordering expectation for actual stdout-end versus process-exit events. Both
legitimate first-failure codes are now accepted while reaping, redaction and
cleanup remain mandatory. Subsequent **18/18 runner tests** pass, including an
actual subprocess/production-helper full-24 workflow with a synthetic SDK, drift,
approval reuse, failed directory sync, permission/symlink rejection, actual
ENOENT, evidence collision and emergency cleanup. Independent combined
result/protocol/framing/child/runner checks pass **101/101**. Root's
`pnpm test:snowflake:qualification` passes **113/113**, adding the 12 manifest
tests. Scoped format and `git diff --check` pass.

Logs: `.cache/verification/connection-attempts/`
`sdk-qualification-runner-{first,second,review,final,independent}.log`,
`sdk-qualification-all-{lead,independent}.log`,
`sdk-qualification-{format-final,runner-child-independent-format}.log`.
Final runner SHA-256:
`63e2fd8b085ce7890d1a25fbc043a06f35286d1c94f52fe6eac2b7de89563c64`.
The actual CLI dry-run, emit and verify paths pass with a synthetic profile and
the current prepared runtime, with no helper/authentication/SQL. Safe recorded
dry-run evidence: `sdk-qualification-runner-cli.json`; runtime manifest hash:
`375e0639ee6d2285b25fd4e9646a1078b6302faa1630a9bcd002ed653ab23114`.
The recorded synthetic plan is not an owner-account execution plan or approval.

These tests establish local orchestration and evidence behavior. They cannot
establish actual Snowflake descriptors, account/session settings, schema metadata,
transaction effects, million-row behavior, packaged-app usage or owner acceptance.
Source verification is pathname-based pre/post checking, not an OS sandbox or a
defense against hostile same-user mutation. SQL admissions do not bound every SDK
authentication/result/logout HTTP request or account charges. Prior canary
approval remains consumed and authorizes no further attempt.

### Qualification child lifecycle (10 November 2026)

Status: **locally tested and independently reviewed supplied-child engine**;
it has no CLI, process/runtime selection, filesystem cleanup or approval authority.
The immutable offline-v1 ledger remains offline-only. The
[protocol contract](snowflake-qualification-protocol.md#supplied-child-lifecycle)
now has serialized physical dispatch, callback/drain backpressure, dual failure
latches, owner-only identified cancellation and bounded close/EOF/TERM/KILL.
Receipts retain known outcomes and distinguish attempted cleanup from confirmed
logout, exit and closed stdio. No account or actual SDK authentication was used.

The original **19 child tests** and **80 combined result/protocol/framing/child
tests** passed. Independent review reproduced a clock rollback that admitted
later SQL below the approved lower window edge, and identified the missing
independent overall deadline. Three regression tests now cover both absolute
edges and a fixed monotonic **360-second** ceiling through authentication, SQL
and grouped observation. The final child suite passes **22/22**, and the four
modules together pass **83/83**. Actual subprocess fixtures exercise the
production helper with synthetic SDK responses through exactly 24 SQL, malformed
trailing frames, early exit, inherited stdio and TERM-resistant/KILL cleanup.
In-process production-helper fixtures cover credits, late terminals, synchronous
and late write failures, stalled drain, cancellation and unconfirmed reaping.
They establish local lifecycle behavior, not database effects or account behavior.

Commands: `node --test scripts/snowflake-qualification-child.test.mjs` and Node's
combined result/protocol/framing/child files; syntax and scoped Prettier pass.
Logs under `.cache/verification/connection-attempts/`:
`sdk-qualification-child-{final,combined,format}.log`,
`sdk-qualification-child-deadlines-{before-fix,final,combined,format}.log`.
Final child hash: `115218a087c7ea3f86a656d20fef61f17932cf5612d10e558449d52e8e19eb8f`.
The offline manifest's 12 tests also pass. `pnpm test:snowflake:qualification`
now selects all qualification regression files and is included in the existing
Snowflake CI integration step. Formatting includes the same qualification files;
package/workflow Prettier and `git diff --check` pass. CI itself was not triggered.

### SDK transaction transport (10 November 2026)

Status: **transport foundation locally tested; application and account workflows incomplete**.
No account authentication, SQL or billable work was part of this increment.
The [transaction design](snowflake-sdk-transactions-design.md) distinguishes typed
boundary commands, command acknowledgement, observed state and verified effects.
The helper accepts exactly `begin`, `commit` or `rollback` plus the established
principal/session pin. It submits one fixed statement, retains the submission
guard through drain, and publishes a distinct associated boundary ACK. The Rust
supervisor pairs action and impact, validates registered statement identity, and
preserves known ACKs through cancellation, drop, drain failure and retirement.
Generic ACKs produce `StatementSucceeded`; this layer never infers committed or
rolled-back effects. Existing application transaction/write capabilities stay disabled.

Review found a helper cancellation reentry gap: an already classified definitive
database rejection could become unknown before terminal publication. The new
regression failed before repair (`sdk-schema-transport-transaction-cancel-before-fix.log`)
and now passes. Root independently reviewed the helper/supervisor diffs, outcome
branches, action/impact association and process tests; no material finding remains.

Focused verification uses the repository's pinned Node 24 and Rust 1.99.1:

- Supervisor unit tests: **19/19**; actual Node/production-helper process tests:
  **37/37**, including the existing repeated two-million-row lifetime test.
  New tests cover exact SQL/pins, definitive rejection followed by same-session
  recovery, pre/post-ACK cancellation/drop, drain failure and stale-handle cleanup.
- Helper tests: **56/56**. The complete helper set covers **216 passing tests**
  across the final-source runs, including **80 actual pinned-SDK loopback cases**.
  Eighteen of those exercise HTTP/TLS/proxy BEGIN/COMMIT/ROLLBACK, lost COMMIT
  response, definitive rejection and post-ACK guard violations. These endpoints
  are local synthetic servers, not Snowflake transaction-effect evidence.
- Strict scoped Clippy, Rustfmt, scoped Prettier and `git diff --check`: pass.
  Logs: `.cache/verification/connection-attempts/sdk-schema-transport-transaction-`
  `{supervisor,process,clippy,format,node-all,node-process,sdk}.log`.
  The first complete Node invocation used the repository cwd and had four
  relative-path process-test failures; the correct helper cwd rerun passed **4/4**.
  Both logs are retained; no failing check was bypassed.

Offline runtime preparation initially used the empty default npm cache and failed
dependency installation. Repeating with the documented repository-local cache
passed without network: `pnpm snowflake:prepare --cache-dir
.cache/snowflake-helper-dev/npm-cache --offline`. Logs are
`sdk-transaction-runtime-prepare{,-final}.log`. Full `pnpm validate` passes:
**649 frontend / 373 standard Rust tests**, 129 opt-in checks ignored, plus
contracts, TypeScript, web build, Rustfmt, desktop check and strict all-target/
all-feature Clippy. No UI control changed in this transport-only increment.
Fresh packaging/native checks follow separately; the preceding package does not
contain this transport slice.

The first full validation attempt passed formatting/contracts/types but stopped
at two unchanged frontend tests: a five-second index-dialog timeout and a
time-bounded SQL JOIN inspection. The two files passed **30/30** in isolation
without source or timeout changes, and the full rerun passed **649/649** frontend
tests. Logs: `sdk-transaction-validate-final.log`,
`sdk-transaction-frontend-focused.log`, `sdk-transaction-validate-rerun.log`.
These first-run timing failures are retained; they are not product-fix evidence.

### SDK guarded schema selection (10 November 2026)

Status: **locally qualified; account and owner acceptance open**. No account
authentication or SQL is part of this increment. The previous two-SELECT canary
approval is consumed and does not authorize observation, metadata or switching.

The typed `switch_session_schema` command carries exact string revision,
fingerprint, database and schema fields. The adapter consumes matching authority
before awaiting, verifies the complete idle baseline and fresh schema membership,
then uses exactly thirteen executions under one bounded lane. It retains the
actual helper switch operation through final observation and publication. A known
USE acknowledgement followed by failed verification closes the session with a
fixed message preserving that distinction. Studio returns the driver's verified
options directly, without another eleven-query observation. Public SQL USE,
writes, DDL and transaction controls remain unavailable on the SDK route.

Evidence under `.cache/verification/connection-attempts/`:

- `sdk-switch-before-fix.log`: the new adapter success test reproduces inherited
  Unsupported before implementation.
- `sdk-switch-adapter-final.log`: **19/19** actual Node/production-helper tests
  with a synthetic SDK pass, including eleven new switch cases and a thirteen-
  variant post-ACK failure matrix. Quoted names, nullable role/warehouse, manual
  idle mode, stale/forged/ABA/open-transaction requests, schema disappearance,
  cancellation, abandonment, supersession and child/auth-root cleanup are covered.
- `sdk-switch-existing-integration.log`: **30/30** existing actual-helper adapter
  fixture tests pass; `sdk-switch-clippy.log` passes scoped strict Clippy.
- `sdk-schema-switch-studio-first.log`: **2/2** strict DTO/Studio regressions pass;
  the Studio case checks success, busy, closed, error, wrong-session and missing-
  revision outcomes and proves zero follow-up observer calls.
- `sdk-schema-switch-frontend-first.log`: **643/643** frontend tests pass before
  the final shared legacy-switch gate and visible refresh hint. The focused state
  suite initially passes **114/114**, including sixteen new authority/race cases.
  `sdk-schema-switch-browsing-ui-final.log` passes **14/14** relation UI tests,
  including authority revocation before catalog/inspection/preview and zero SQL
  while switching.

Independent substantial backend review finds no unresolved material issue.
Frontend review identified a shared legacy pending-state gap, now addressed, and
expired-authority discoverability. The selector stays visible disabled and says
“Refresh to change schema”; metadata lookups do not add hidden observer reads.

The first focused browser run reproduces a pre-existing console-test assertion
expecting plural “transactions” after the product warning changed to “open or
unknown transaction.” Both strings are present at preceding `f4bb5df`. PostgreSQL
read/write/transaction/grid assertions reached the final close prompt; its failed
assertion left the dialog open and the test's cleanup then timed out. The test
assertion is corrected and the affected rerun passes.
The first run passes **10/16**, including both new guarded-switch UI cases.
It also reproduces a new template-dialog regression: publishing the template's
context observation globally dismisses the dialog before NULL-schema validation.
The template now uses the same guarded observation but returns state directly,
without publishing a new editor context or adding SQL. This repair and a late
PostgreSQL completion failure are covered by the passing rerun below. The log is
`sdk-schema-switch-browser-first.log`; six failing traces are preserved in
`schema-switch-browser-first-traces/`.

`sdk-schema-switch-validate-first.log` stops at frontend tests: **635 pass / 8
fail**. One assertion expects the former absent legacy-switch flag rather than
its new settled `false` value. Seven failures are timeouts or ensuing assertions
while browser tests ran concurrently. The five affected component files pass all
**26/26** tests in `sdk-schema-switch-frontend-recheck.log` without concurrent
browser work; the passing full final validation is recorded below. The first run does not
qualify the final source or later validation stages.

`sdk-schema-switch-browser-second.log` passes **16/16** focused headless browser
workflows in 2.7 minutes: PostgreSQL/MySQL transactions, grid isolation and last-
console cleanup; schema/database context isolation; SQL files/library; SDK
browsing/completion/state; both new guarded-schema outcomes; and positive/negative
template-context workflows on PostgreSQL, MySQL and MariaDB. SDK cases use
synthetic RPC seams, not account behavior. Independent source review clears the
new template observer's authority revocation, stale/retired ownership rejection,
pending cleanup and no-global-publication/no-follow-up-SQL behavior.

`sdk-schema-switch-state-final.log` passes **120/120** state tests, including
the final shared legacy switch gate and template observation regressions.
`sdk-schema-switch-postgres-final.log` and `sdk-schema-switch-mysql-final.log`
pass the actual isolated PostgreSQL and MySQL/MariaDB manual-transaction,
grid-isolation and schema/database-switch tests. These exercise the retained
legacy route and do not establish Snowflake account behavior.

`sdk-schema-switch-validate-final.log` passes full `pnpm validate`: **649 frontend /
365 standard Rust** tests, formatting, generated contracts/types, production web
build, desktop compilation and strict all-target/all-feature Clippy. The standard
Rust run reports 129 opt-in fixture tests ignored; relevant actual database tests
are run separately above. The existing bundle chunk-size/plugin-timing notices
remain nonfatal.

`sdk-schema-switch-bundle-final.log` passes a fresh normal SDK macOS arm64 app,
strict ad-hoc signature/resource verification and actual packaged SDK hello/EOF
startup with **zero authentication and SQL requests**. The 31,546,624-byte normal
executable has SHA-256
`c4d4160468f8e1a8a8603ce46efb5e6e53511945f24f504696142fcb9d541d79`;
`sdk-schema-switch-bundle-artifact.json` records its path and bundle size. The
runtime manifest remains `f863ea753dbd2908ea7f63f60604f162c888f6a65d3301a593187f32ab35fbbe`.
The known missing-libLLVM debug-strip warning remains nonfatal; Developer ID,
notarization, installation and actual account/normal-app acceptance remain open.
`sdk-schema-switch-native-build-final.log` passes the separate fresh hidden-native
build and resource/startup verification. `sdk-schema-switch-native-final.log`
passes **three runs**, each with **19 native check categories / ten cleanup
cycles**, including the newly enrolled schema-switch command's disconnected-
session IPC guard. The native build SHA-256 is
`f89c70a41a8862834a960ab09239cf66339f65f0f4d4aa639738bbeb10efbe61`.
This hidden synthetic fixture does not qualify normal visible startup, SSO or
Snowflake account switching.

`sdk-schema-switch-qualified-source.json` confirms all **322 frozen application,
runtime and test inputs** remain unchanged through qualification and the normal
bundle executable retains its recorded hash. Documentation and the separate pure
qualification modules are outside that application source freeze. The local app
is runnable at `src-tauri/target/release/bundle/macos/Joinery.app`; it is not
installed or externally accepted.

### Snowflake synthetic read admission (10 November 2026)

Status: **locally verified SQL admission; account performance unqualified**. This changes
public SQL classification, not account authorization. The existing normal macOS
bundle recorded above predates this policy increment.

`sdk-synthetic-functions-before-fix.log` reproduces `PolicyBlocked` for the exact
proposed million-row query and independent scalar/sequence examples. The repair
admits plain unquoted/unqualified Snowflake `MOD`, `REPEAT`, `SEQ1`/`SEQ2`/`SEQ4`/
`SEQ8` and `GENERATOR` with reviewed argument shapes. Nested expression visitation
and whole-batch classification remain intact. It is not a special SQL-text bypass
or a result/compute/billing limit. Official references:
[MOD](https://docs.snowflake.com/en/sql-reference/functions/mod),
[REPEAT](https://docs.snowflake.com/en/sql-reference/functions/repeat),
[sequence functions](https://docs.snowflake.com/en/sql-reference/functions/seq1),
[GENERATOR](https://docs.snowflake.com/en/sql-reference/functions/generator), and
[system-function names](https://docs.snowflake.com/en/developer-guide/udf-stored-procedure-naming-conventions).

`sdk-synthetic-functions-safety-final.log` passes **22/22** safety tests, including
engine isolation, argument/modifier constraints, quoted/qualified calls, unknown
nested functions, duplicate generator arguments and appended writes. Independent
bounded source review finds no material issue. The initial adapter invocation
fails its explicit `JOINERY_TEST_NODE` environment prerequisite before helper
launch; the corrected invocation passes in
`sdk-synthetic-functions-adapter-second.log`. It uses actual Node and the production
helper with a synthetic SDK: twelve rows, one exact SQL dispatch, exact large
integer strings, NULL versus empty strings, and zero dispatch for rejected calls.
This is not a Snowflake server or million-row transport result.

`sdk-synthetic-functions-adapter-final.log` passes all **31/31** existing and new
production-helper/synthetic-SDK adapter fixtures. Full `pnpm validate` passes in
`sdk-synthetic-functions-validate-first.log`: **649 frontend / 368 standard Rust**
tests, formatting, contracts/types, production web build, desktop compilation and
strict all-target/all-feature Clippy. Its 129 opt-in database fixtures are ignored;
this engine-specific admission slice adds no actual-account evidence. The separate
transaction transport remains uncommitted and outside this policy change.

### Offline qualification result/protocol validation (10 November 2026)

Committed separately as `7a7ea12`. The four pure modules/tests and
[usage contract](snowflake-qualification-protocol.md) add typed bounded result
collection, observation comparison, fixed-manifest request reservation and
streaming credit/cancel/close ownership. They perform no I/O and confer no
execution authority; the v1 manifest remains `offlineOnly` / `runner: null`.

`sdk-qualification-pure-combined-final.log` under the evidence directory passes
**39/39** synthetic tests: 16 result tests and 23 protocol tests. An in-process
production helper with a synthetic SDK and split framing exercises the complete
24-SQL ledger, one connection, 1,000 schemas and a shared 1,023-row successful
budget. No subprocess, network, real SDK authentication or account work occurs.

Independent review reproduces an evidence-classification defect: an unknown
terminal was labelled rejected and pending SQL reservations were absent from
`outcomes()`. `sdk-qualification-protocol-outcomes-before-fix.log` records the
failing regression. Evidence now separates request progress, remote certainty
and typed-result validation, installs unknown before a frame escapes, and retains
known success acknowledgement when later validation fails. Context rejection
before its public acknowledgement remains unknown. A definitive query rejection
also requires its dispatch phase; the missing-phase regression fails before the
repair in `sdk-qualification-protocol-dispatch-before-fix.log`. The final combined
run and independent 39-test rerun pass; scoped formatting passes and review clears
the repair. Strict physical framing, approval/freshness/source enforcement and
child cleanup remain separate required runner work.

### Offline qualification framing (10 November 2026)

The pure strict decoder and its tests add physical byte accounting, bounded
incremental copying, fatal UTF-8 and strict JSON validation to the separate
[qualification contract](snowflake-qualification-protocol.md). There is no
executable runner, process launch, authentication or account SQL in this slice.

Independent review reproduces two defects in
`sdk-qualification-framing-review-before-fix.log`: a callback could swallow a
reentrant-call exception and let the outer decoder deliver another frame; a
typed-array constructor/species getter could escape as an unlatched raw error.
Both are repaired with regressions. Copying now bypasses those hooks and latches
detached-input errors; a swallowed reentry still stops subsequent delivery.

`sdk-qualification-framing-review-combined.log` and the independent lead rerun
`sdk-qualification-framing-lead-final.log` pass **61/61** synthetic tests: 16
result, 23 protocol and 22 framing cases. The latter include one-byte splits,
physical limits, duplicate escaped keys, malformed UTF-8, exact decimal-number
validation and a composed decoder/protocol success-ACK followed by malformed or
truncated bytes. Known success survives the later failure. Independent review
and scoped formatting pass after the repairs. This establishes no subprocess,
approval, cleanup or remote behavior; those remain required executable work.

### SDK schema-switch transport (10 November 2026)

Status: **transport locally qualified; application switching incomplete**.
The [transport contract](snowflake-sdk-session-design.md) separates strict
read-only `context {}` from a typed `switchSchema` operation. The latter quotes
database/schema independently, retains USE acknowledgement separately from
final readback and retires uncertain context changes. Its supervisor requires
distinct USE/readback statement identities, final readback association and exact
operation ownership. Public SQL USE/ALTER, the application adapter and schema
selector remain disabled. This foundation does not qualify account switching.

Evidence under `.cache/verification/connection-attempts/`:

- `sdk-schema-transport-before-fix.log` reproduces the preceding helper's strict
  context/new-operation failures. `sdk-schema-transport-association-before-fix.log`
  reproduces a missing final acknowledgement SDK request ID being accepted;
  the supervisor now requires association with the registered readback.
- `sdk-schema-transport-node-final.log`: **169/169** Node tests pass: 47 helper,
  60 submit-guard and 62 actual pinned-SDK loopback cases. Twelve added cases
  exercise switch success, lost USE acknowledgement, readback denial and lost
  readback acknowledgement over HTTP, verified TLS and CONNECT. These owned
  fixtures use synthetic login and a preconnected helper seam; they do not
  establish Snowflake account semantics or external-browser authentication.
- `sdk-schema-transport-association-final.log`: **14/14** supervisor unit tests
  pass. `sdk-schema-transport-process-final.log`: **33/33** process tests pass,
  including cancellation, retained USE evidence, abandonment and child cleanup.
- `sdk-schema-transport-startup-negative.log`: one regression covers **eight**
  startup identity mismatches/NULLs (configured role, warehouse, database and
  schema). Each is rejected before version SQL; only the initial identity read
  runs, and child/auth-root cleanup is confirmed. The full actual Node/core-helper
  adapter fixture suite passes **30/30** in
  `sdk-schema-transport-adapter-final.log`; its SDK is synthetic.
- `sdk-switch-transport-validate-final.log`: full `pnpm validate` passes
  **627 frontend / 363 standard Rust** tests, contracts/types, production web
  build, formatting, desktop compilation and strict all-target/all-feature Clippy.
  The final Rust-only association regression follows the 169-test Node run;
  helper sources are unchanged between those checks.

`sdk-switch-transport-browser-final.log` passes all **three** focused headless
browser workflows: new SDK profile persistence, legacy OAuth preservation and
explicit observed/unavailable transaction state with unsupported controls blocked.
The SDK state case is synthetic, not account evidence.
`sdk-switch-transport-bundle-final.log` passes a fresh **223.95 MiB** normal SDK
macOS arm64 bundle, strict ad-hoc signature/resource checks and packaged SDK
hello/EOF startup with **zero authentication and SQL requests**.
`sdk-switch-transport-bundle-artifact.json` records the 31,469,152-byte executable
SHA-256 `2f9646c1e086a2ec1603b30076cd8c8c03c90f1b294a2c70ef1a5687210b1fcf`.
The existing missing-libLLVM debug-strip warning remains nonfatal. Developer ID,
notarization, installation and normal app/account workflows remain open.
`sdk-switch-transport-source-freeze.json` /
`sdk-switch-transport-qualified-source.json` confirm **321** existing application/
runtime/test inputs unchanged through qualification on `d8f4011` plus this dirty
transport increment. Two new pure offline qualification-result tooling files
were developed separately during the build and are outside this artifact scope.
Hidden-native fixtures are deferred to the dependent typed IPC/UI slice; the public IPC/UI did not change in this transport foundation. No new
hidden-native qualification is claimed here. Offline resource
preparation passes; the current runtime manifest is
`f863ea753dbd2908ea7f63f60604f162c888f6a65d3301a593187f32ab35fbbe`.
Independent substantial source review finds no material unresolved issue.
No account authentication/SQL, installation, publication or hub update occurs.
The private canonical hub cannot be updated without the prohibited publication
step; this repository retains the durable status.

### Offline account qualification manifest (10 November 2026)

The [compiler contract](snowflake-qualification-manifest.md) is implemented and
locally tested. It expands the distinct helper-direct
`session-options-scalar-state-v1` workflow to exactly **24 SQL / 26 commands**,
including startup identity, eleven-read schema options, one typed scalar and
ten-read state observation. It is offline-only; no executable runner or account
authorization is established.

`pnpm test:snowflake:qualification:manifest` passes **12/12** synthetic tests;
syntax, scoped formatting and diff checks pass. Evidence under
`.cache/verification/connection-attempts/` is `sdk-manifest-tests-final.log`
and `sdk-manifest-format-check.log`. Actual CLI checks using synthetic connection
identifiers reject the old prepared helper source, then pass dry-run, exclusive
emission and independent verification after offline resource preparation;
re-emission is refused. The `sdk-manifest-current-stage-*`,
`sdk-manifest-emit-final.log`, `sdk-manifest-verify-final.log` and
`sdk-manifest-overwrite-refused.log` logs retain those outcomes. No authentication
or SQL ran. Independent review clears this bounded tooling after correcting
documentation about supplied versus fresh IDs/windows and private input storage.
The future runner still needs tested dispatch, result validation, credit, cleanup
and fresh approval enforcement. All required Snowflake replacement gates remain.

### SDK session observation (10 November 2026)

Status: **locally qualified; account and owner acceptance open**. Source
follows `5d242d2`, with the separate persistence repair `a27865c`. The [observer contract](snowflake-sdk-session-observation.md)
defines ten fixed reads for state, eleven for coherent bounded schema options,
opaque transaction IDs, nullable roles and explicit Unknown state. SDK schema
switching and transaction controls remain disabled. The earlier two-query
account canary does not authorize or qualify this operation.

Evidence under `.cache/verification/connection-attempts/`:

- `sdk-session-state-reproduced.log`: restoring the preceding store source
  makes four new stale-response/failed-observation regressions fail; the current
  source is restored byte for byte. `sdk-session-ui-final.log` passes **101**
  focused store/shell tests, including stable refresh generations and retired
  session responses. Standard combined qualification is recorded separately.
- `sdk-session-pure-final.log`: **12/12** pure observation-parser tests pass.
  `sdk-session-dto-final.log` passes the strict tagged evidence contract test;
  development caught and repaired incomplete/extra-field acceptance before
  qualification. `sdk-session-studio-drift-final.log` passes stable-versus-changed
  runtime cache invalidation, including nullable-role changes.
- `sdk-session-adapter-first.log`: **8/8** actual Node/core-helper observer tests
  using a synthetic SDK pass. The exact ten/eleven-read sequences, NULL role,
  opaque/open transaction, disagreement, denial, schema omission and superseded
  options authority are checked. This is not actual SDK transport/account evidence.
- `sdk-session-real-drivers-final.log`: one admission regression passes on all
  three real disposable engines: PostgreSQL, MySQL and MariaDB. Requesting the
  observation-only Unknown mode is rejected before driver/session access, keeps
  runtime state unchanged, and remains InvalidInput after closure.
  `sdk-session-postgres-final.log` and `sdk-session-mysql-final.log` also pass the
  actual manual-transaction/grid-isolation/schema-or-database-switch regressions.
- `sdk-session-browser-first.log`: seven existing focused browser workflows
  pass; the new synthetic SDK session test reaches its expected UI states but
  fails on a hidden disconnect-action locator. The test is repaired to hover
  the profile row, matching the actual interaction. The affected workflow then
  passes in `sdk-session-browser-final.log`: explicit open/usability-unknown and
  unavailable states, disabled transaction/schema actions, unavailable current
  schema, bounded refreshes and conservative close confirmation. No Snowflake
  account or SSO is used by this test.

`sdk-session-studio-final.log` passes all **13** actual Node/core-helper Studio
fixtures in 267.18 seconds, including the three added observation cases. One
retained connection executes the exact eleven-read options sequence with a NULL
role and opaque open transaction. Stable observations preserve prepared plans;
warehouse and transaction-only drift revoke them. A completed SELECT retains its
terminal status and exact result after a denied observer, failed options, and
fresh-revision recovery. Existing completion, browsing and cleanup fixtures pass.
These fixtures use a synthetic SDK, not actual transport or account behavior.

`sdk-session-validate-first.log` passes 627 frontend and 354 standard Rust tests,
contracts/types and the production build, then fails strict Clippy on a new
test's `DoubleEndedIterator::last`. The test now uses `next_back`; the final
combined validation was interrupted by the pre-existing concurrent SQLite WAL
startup race described below. After its separate repair,
`sdk-session-validate-after-wal.log` passes the full `pnpm validate`: **627
frontend / 357 standard Rust** tests, contracts/types, production web build,
formatting, desktop compilation and strict all-target/all-feature Clippy.
`sdk-session-bundle-final.log` passes a fresh normal SDK-bearing macOS arm64
bundle build (**223.94 MiB**), strict ad-hoc signature/resource checks and actual
packaged SDK hello/EOF startup with **zero authentication and SQL requests**.
`sdk-session-bundle-artifact.json` records executable SHA-256
`e7d32b39ab46e3aaf47e33a72ecc72ce0e276ef9fa4c1d0adc1ca9dc5839b9c6`
and runtime manifest
`8dfda76a001b44c51f84a3cb298b085c674ca75cc35661c5b643524f1af8ab9c`.
Developer ID signing,
notarization, installation and ordinary account-connected app workflows remain
open. The toolchain's existing missing-libLLVM debug-strip warning is nonfatal.
`sdk-session-lint-freeze-transition.json` records the iterator test-only change.
Independent source review clears the parser,
adapter publication and shared state changes. Review caught a missing successful
runtime-drift invalidation; the final implementation preserves plans only for
identical stable observations and revokes them after changed or unavailable
observations. A separately acknowledged query outcome is never rewritten by an
observer failure. No new Snowflake account authentication or SQL has run.
No installation or publication has run.

The separate startup repair is committed as `a27865c`; its
[reproduction and verification](local-metadata-wal-admission.md) preserve the
failed combined run, raw synthetic SQLite diagnosis, before-fix sensitivity,
ten focused tests, 100 four-connection runs and actual abrupt-backend-restart
browser workflow. No private application data was accessed.

`sdk-session-read-adapter-first.log` passes 28 of 29 existing adapter fixtures,
then encounters the old Unsupported-session-state expectation. That metadata
fixture lacks the new observer shapes; the updated assertion requires explicit
Unknown/Unavailable with every verified context field cleared. Final
`sdk-session-read-adapter-final.log` passes **29/29** actual Node/core-helper
read/completion/browsing fixtures using the synthetic SDK. The related
`SDK session observation` parser/helper cases retain separate evidence above.
The test-only transition is recorded in
`sdk-session-adapter-test-freeze-transition.json`. The earlier full 13-fixture
Studio run precedes the iterator and separate persistence repair;
`sdk-session-studio-completed-final.log` reruns the affected acknowledged-result/
observer-denial workflow on final source: **1/1** passes in 16.45 seconds.
The complete Studio suite is not claimed as rerun after those changes.

`sdk-session-native-build-final.log` passes a fresh SDK-bearing hidden-native
build and packaged helper startup with zero authentication/SQL. The isolated
hidden app then passes all **19** check categories and **10** cleanup cycles in
`sdk-session-native-final.log` / `sdk-session-native-runs.json`: 4,077 ms native
fixture elapsed, 98.55 MiB sampled main-process peak RSS. The launched bundle
executable SHA-256 is
`4167a5235e207ff28e8e949828909af8d6fa9f74af01da7a086a8b4bc068d9cf`;
the separate fixture executable is
`1309d60baf7b683a50a87bb061fded39ffbc705a6c69f78cc17d5586191e7c06`.
All six checkpoints report SDK runtime availability and zero connect/execute/
prepare invocations. These synthetic checks do not establish normal-app SSO,
cold visible startup, total application idle memory or account acceptance.
`sdk-session-final-source-freeze.json` / `sdk-session-qualified-source.json`
verify **319** application/runtime/test inputs unchanged through qualification,
including the regenerated contract. The snapshots describe the dirty observer
increment on `a27865c`, not a clean earlier commit. Final
`sdk-session-rust-final.log` passes formatting, desktop check and strict
all-target/all-feature Clippy after the last adapter-test-only migration.

Documentation review also records automatic cadence: Studio requests up to ten
observer reads after each qualifying serialized SQL action, including some
database-error outcomes, while the run/session remain eligible. Account scope
must include these reads and explicit editor/context refreshes.

### SDK relation browsing (10 November 2026)

Status: **locally qualified**, with account and owner acceptance still open.
Source follows `44a8b0c`. The local SDK route adds a distinct `relationBrowsing`
capability; generic `tableBrowsing` and all edit/write/DDL/transaction capabilities
remain false. This is a bounded current-database relation workflow, not complete
DataGrip object inspection or account qualification. See the
[implementation contract](snowflake-sdk-browsing-design.md).

Catalog, inspection and preview use the retained console session and existing
Studio/driver lanes. Catalog refresh invalidates old targets before admission,
even when the console is busy. Opaque target/inspection revisions reject stale
requests before SQL. Context/session/target observations bracket publication;
they detect observed drift without promising atomic identity under external DDL.
Unknown kinds/traits, permanent targets shadowed by temporary tables and
ambiguous collisions remain visible with actions disabled. No automatic
reconnection or alternative metadata connection is introduced.

Inspection combines bounded COLUMNS, relation-specific DESCRIBE and zero-row
resolved columns. It retains ordinal gaps, full types, aliases, nullability,
identity attributes, comments and independent COLUMNS/descriptor/write-default
observations. Missing write-default fields differ from reported NULL. Keys are
explicitly `notFetched`; metadata never establishes edit eligibility. Preview
uses an explicit quoted projection, retains at most 500 unordered rows/2 MiB,
and pages those retained rows/columns locally without re-execution.

Regression evidence under `.cache/verification/connection-attempts/` includes:

- `sdk-browsing-reproduce-context_list_denial_is_not_published_as_empty_success.log`
  and `sdk-browsing-reproduce-failed_switch_revokes_targets_and_plans_before_dispatch.log`:
  restoring the two old Studio behaviors makes the new assertions fail.
  Current code propagates context-list denial and invalidates targets/completion/
  plans before switch dispatch, including failure. Original source is restored
  after each sensitivity run.
- `sdk-browsing-iceberg-before-fix.log`: the official initial-default `2` versus
  write-default `3` example reproduces the missing observation. The repair keeps
  COLUMNS expression defaults independent of DESC observations on Iceberg;
  ordinary non-Iceberg equality remains checked. Missing, NULL, empty, malformed
  and oversized write-default cases have regressions. See
  [the official semantics](https://docs.snowflake.com/en/user-guide/tables-iceberg-default-values).
- `sdk-browsing-studio-final.log`: **10/10** actual Node/core-helper application
  fixtures pass, using the isolated synthetic SDK. The added workflow runs
  catalog → inspect → preview → refresh, checks exact large integers/NULL/empty
  strings and ordinal gaps, rejects stale targets without SQL, and verifies
  that a busy catalog refresh also revokes old targets. One connection is
  retained and cleanup is awaited. This combined run precedes the reviewed
  Iceberg DTO repair; `sdk-browsing-studio-workflow-final.log` then reruns the
  affected browsing workflow on final source: **1/1** passes in 19.70 seconds.
  These tests do not use actual SDK transport or an account.

- `sdk-browsing-pure-final.log`: **19/19** parser/builder tests pass after
  the Iceberg repair. Two additional DTO wire-contract tests pass in validation.
- `sdk-browsing-validate-final.log`: **619 frontend / 338 standard Rust**
  tests, contracts/types, production web build, repository formatting, desktop
  compilation and strict all-target/all-feature Clippy pass. The first combined
  run passed its tests but failed the repository Rust formatter on test layout;
  `sdk-browsing-validate-first.log` retains that result. The corrected source
  passes the complete command. Standard validation does not execute the ignored
  external-engine suites or the feature-gated actual-helper adapter fixtures.
  The existing Vite chunk-size warning remains nonfatal.

- `sdk-browsing-adapter-final.log`: **29/29** actual Node/core-helper
  read-adapter tests pass on final source using the synthetic SDK. The ten added
  browsing cases cover stale/forged/cross-database targets before SQL, superseded
  inspection attempts, concurrent refresh ownership, denial/malformed results,
  preview bounds, drift and busy-lane refresh invalidation.
- `sdk-browsing-browser-final.log`: all **11** focused browser workflows pass
  in 1.4 minutes. The new Snowflake browse flow verifies opaque request binding,
  disabled shadowed-target actions, exact scalar rendering, local paging without
  another preview query, failed-refresh clearing, Escape and focus restoration.
  Snowflake RPCs are synthetic. The context/template cases use actual disposable
  PostgreSQL, MySQL and MariaDB; the late-navigation case uses PostgreSQL.
  The earlier full 145-workflow result is not a rerun of this increment.

- `sdk-browsing-postgres-context-final.log`: the actual disposable PostgreSQL
  manual-transaction/failed-state/grid-isolation/schema-switch regression passes
  after the shared Studio context-list and pre-dispatch invalidation changes.
  The preceding 124-fixture suite was not rerun for this increment.

- `sdk-browsing-bundle-final.log`: fresh normal SDK-bearing macOS arm64
  bundle, strict ad-hoc signature, 15,106 resource entries/zero links, source pins
  and actual packaged-SDK hello/EOF pass with **zero authentication and SQL**.
  The 223.89 MiB bundle executable is 31,409,792 bytes, SHA256
  `59c06ca283b981c139be32e52d182d6fc8da088291a9610812e1250cdf1c84cd`.
  `sdk-browsing-bundle-artifact.json` records the artifact and source snapshot.
  Compilation passes despite the previously observed local `rust-objcopy`
  missing-`libLLVM.dylib` stripping warning. No global toolchain changed.

- `sdk-browsing-native-build-final.log` and `sdk-browsing-native-final.log`:
  fresh SDK-bearing hidden native build/resource/startup checks and **19 check
  categories / ten cleanup cycles** pass (4,114 ms native elapsed). This includes
  disconnected-session IPC admission for all three new commands, WebView workers,
  exact values, persistence and cleanup. All six checkpoints report the runtime
  available and zero connect/execute/prepare invocations. The actually launched
  isolated bundle executable SHA256 is
  `e6943dd4035bdd16fb17d15977b528c85017e1006182930f7e632ced65cabc9a`;
  the separate fixture executable is
  `7b587336c106bb3e8bc5da8ce4308d02b5d76e3f1d18b9fd4afc0cda87091321`.
  `sdk-browsing-native-runs.json` preserves the report. This is synthetic hidden
  native evidence, not normal account sign-in, installed restart, OS credentials
  or representative performance. The known stripping warning also occurs here.
- `sdk-browsing-qualified-source.json`: all **316** application/runtime/test
  input hashes remain unchanged through final verification. The recorded
  `sdk-browsing-source-freeze-transition.json` captures the reviewed Iceberg fix,
  UI regressions and generated contracts; its formatting-only successor is
  `sdk-browsing-format-freeze-transition.json`. Derived Python bytecode is
  excluded from source inputs. Artifact provenance records `44a8b0c` and a dirty
  tree because qualification precedes the local commit; no clean-tree artifact
  claim is made. Independent implementation review clears the shared/adapter
  workflow and repaired Iceberg observations. Documentation review's two scope
  ambiguities are corrected.

No installation, push, publication, merge or private project-hub update ran.
No additional authentication or account SQL ran. The prior two-SELECT approval was not reused. Actual descriptor
shapes, privileges, supported type families, temporary collision inspection,
external DDL, full session observation/schema switching, broader object metadata,
writes/DDL/transactions and million-row Snowflake behavior remain open gates.

Reproduce with the pinned Node/Rust environment documented below. The fixture
commands use only synthetic SDK data or disposable local databases:

```sh
pnpm validate
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib snowflake_browsing
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --test snowflake_external
JOINERY_TEST_NODE="$PWD/.cache/toolchains/node-v24.22.2-darwin-arm64/bin/node" cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_connection_tests
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres postgres_manual_transactions_failed_state_grid_isolation_and_schema_switch -- --ignored --test-threads=1
pnpm test:e2e tests/e2e/snowflake-sdk-browsing.spec.ts tests/e2e/snowflake-sdk-completion.spec.ts tests/e2e/snowflake-authentication.spec.ts tests/e2e/sql-template-context.spec.ts
pnpm desktop:snowflake:bundle:mac
pnpm desktop:snowflake:smoke:build
pnpm test:native --runs 1 --cycles 10 --attributed-memory
```

### SDK submission admission (10 November 2026; locally qualified)

Source follows `5df743c`. The original process-local guard is installed before
SDK import and admits one canonical native query POST for the exact request ID
returned by each intended execute. Ownership lasts through result drain and the
terminal decision. Redirects, stale owners, duplicate submissions, malformed
targets and exhausted lifetime capacity latch admission and retire the helper.
No profile, environment or IPC override enables another transport or disables
enforcement. Writes, DDL and transaction controls remain disabled.

The helper and runtime independently pin the reviewed source: five helper
modules plus 13 specific SDK/Axios/follow-redirects files, including resolution
from their actual importing files. This is a bounded transport-source contract,
not authentication of every dependency file or an OS network sandbox. See the
[guard contract](snowflake-sdk-submit-guard-design.md).

Local evidence is under `.cache/verification/connection-attempts/`:

- `sdk-submit-guard-helper-final.log`: **98 tests pass**: 60 native guard,
  29 Helper and nine source-pin regressions. Native loopback tests include
  legitimate encoded query values, query endpoint aliases, ambiguous native
  authorities, irreversible admission, exact registration and retained ownership.
- `sdk-submit-guard-resource-final.log`: **57 tests pass**: 50 preparation/startup
  script tests plus seven native resource tests. Rejected source verification
  preserves the previous stage or prevents a startup child, respectively.
- `sdk-submit-guard-runtime-rust-final.log`: all **18** runtime factory tests
  pass, including missing/tampered new modules and rewritten manifests.
- `sdk-submit-guard-studio-final.log`: **nine** application connection/completion
  tests pass with the real core helper/guard and pinned Node. Their private
  synthetic SDK/source verifier does not prove actual SDK transport. Earlier
  `sdk-submit-guard-studio-command-setup.log` records an invocation error: a
  wrong filter and omitted explicit `JOINERY_TEST_NODE`; this is not a source
  regression. The corrected fixture run supplies the explicit runtime pin.
- `sdk-submit-guard-stage.log`: first offline preparation failed because the
  default npm cache was empty. Using the previously populated explicit local
  cache succeeds. `sdk-submit-guard-stage-frozen.log` stages final production
  bytes; `sdk-submit-guard-startup-final.log` passes actual SDK hello/EOF with
  **zero authentication and SQL requests**. Manifest SHA256:
  `8dfda76a001b44c51f84a3cb298b085c674ca75cc35661c5b643524f1af8ab9c`.

Independent review reproduced and repaired native header-accessor, effective
default-port and URL-owned-property target mismatches; outcome classification
after acknowledgement and definitive SQLSTATE failure; and deeper importer
shadowing missed by package-root resolution. A legitimate non-query URL with
encoded percent values also reproduced a false latch; only raw paths are now
decoded for candidate detection. The actual SDK fixture first caught rejection
of its supported `request_guid` spelling; the closed key lexer was corrected.
The relevant `submit-guard-review-*`, `sdk-submit-guard-importer-shadow-before-fix.log`
and `submit-guard-sdk-first.log` preserve the unsuccessful cases. Independent
review clears the final lifetime/outcome mapping and source-resolution checks.

Further final evidence:

- `sdk-submit-guard-all-helper-final.log`: **159/159** pass, including the
  unchanged six-case unguarded actual-SDK retry baseline. The guard matrix in
  `submit-guard-sdk-final.log` passes **50/50**: 49 actual-SDK scenarios and one
  fixture network-boundary self-check. Twelve SDK scenarios drive production
  Helper terminal handling through a preconnected actual SDK connection;
  remaining scenario outcomes are explicitly harness mappings. Local synthetic
  OAuth login and the preconnected seam do not exercise external-browser SSO.
  HTTP/verified-TLS/CONNECT cases count server requests and simulated effects.
  Rejected redirects reach no second destination; untrusted direct/proxy TLS
  fails before login/query. Removing enforcement admits two POSTs/two simulated
  effects versus one with enforcement, without relying on server deduplication.
  Children and owned certificate subprocesses close before private root removal.
  The socket/DNS/UDP test boundary is self-tested and is not an OS sandbox.
- `sdk-submit-guard-process-final.log`: all process-runner stages pass:
  **68** Snowflake unit tests, **nine** actual-runtime application fixtures,
  **31** child-supervisor tests and **19** synthetic-SDK read-adapter tests.
  These categories overlap other focused runs; they are not additional unique
  coverage counts. No synthetic SDK result establishes actual account behavior.
- `sdk-submit-guard-validate-final.log`: **607 frontend / 314 standard Rust**
  tests, contracts/types, production build, formatting, desktop check and strict
  all-target/all-feature Clippy pass. External engine suites remain ignored in
  this standard command. The preceding 124-engine-fixture result belongs to its
  earlier source; it was not rerun for this Snowflake/runtime-only increment.
- `sdk-submit-guard-browser-final.log`: all **eight** focused browser workflows
  pass in 29.4 seconds. Snowflake RPCs are synthetic; the stale-publication case
  uses disposable PostgreSQL metadata. The full 145-workflow result remains a
  preceding-source result, not a rerun of this increment.

- `sdk-submit-guard-bundle-final.log`: fresh normal SDK-bearing macOS arm64
  bundle, strict ad-hoc signature, 15,106 resource entries/zero links, reviewed
  pins and actual SDK hello/EOF pass with zero authentication/SQL. The 223.60 MiB
  bundle's executable is 31,110,464 bytes, SHA256
  `2ad7fe7c162f1c89f025f87b1a682019d31e21fb01091ae16f3c9ccd5a03dbd8`.
  `sdk-submit-guard-bundle-artifact.json` records its manifest and scope.
- `sdk-submit-guard-native-build-final.log` and
  `sdk-submit-guard-native-final.log`: fresh SDK-bearing hidden native build,
  resource/startup verification and **19 check categories / ten cleanup cycles**
  pass. All six checkpoints load the runtime and report zero connect, execute
  and prepare invocations. `sdk-submit-guard-native-runs.json` records the
  isolated bundle executable SHA256
  `769d69477eeed38a5c95f25188ef7ae825a98e9fa82124331b00b4c8a643c991`.
  This is synthetic hidden WebView/IPC/cleanup evidence, not normal account UI,
  representative performance, installed restart or credential persistence.
  Both release builds pass despite the previously recorded `rust-objcopy`
  missing-`libLLVM.dylib` stripping warning; no global toolchain was changed.
- `sdk-submit-guard-qualified-source.json`: all **308** application/runtime/test
  input hashes remain unchanged at the end of qualification. The preceding
  `sdk-submit-guard-source-freeze-transition.json` records only the final SDK
  test self-check and documentation-only helper README refresh. Production
  source bytes stayed frozen. Build provenance records `5df743c` with a dirty
  tree because verification precedes the local commit; this is not a clean-tree
  artifact claim. No installation, push, publication or project-hub update ran.

No result here qualifies account metadata, remote write
effects, cancellation, rollback or the million-row Snowflake workflow. The
existing two-SELECT approval was not reused for this increment.

Reproduction uses the pinned Node/Rust environment documented below:

```sh
pnpm test:snowflake:helper
pnpm test:snowflake:process
pnpm test:snowflake:runtime
pnpm test:native:resources
pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline
pnpm test:snowflake:startup
pnpm validate
pnpm desktop:snowflake:bundle:mac
pnpm desktop:snowflake:smoke:build
pnpm test:native --runs 1 --attributed-memory
```

### Snowflake starter repair (10 November 2026; local checks)

Source follows `459db5a`. Joinery's exact default Snowflake script,
`-- A little space to think.\nSELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();`,
previously failed the ReadOnly classifier before dispatch. Both
`sdk-starter-policy-before-fix.log` and `sdk-starter-child-before-fix.log` under
`.cache/verification/connection-attempts/` reproduce `PolicyBlocked` (exit 101).

The repair admits only unquoted, unqualified, ordinary zero-argument Snowflake
`CURRENT_USER()` and `CURRENT_VERSION()` observers. Function parameters,
arguments, aggregate/window/null/ODBC modifiers and unknown functions remain
blocked; the shared builtin policy and other engines are unchanged.
[CURRENT_USER](https://docs.snowflake.com/en/sql-reference/functions/current_user)
and [CURRENT_VERSION](https://docs.snowflake.com/en/sql-reference/functions/current_version)
document these observers.

- `sdk-starter-safety-final.log`: all 19 safety tests pass, including the exact
  commented starter with no approval/passphrase and malformed-call negatives.
- `sdk-starter-child-final.log`: all 19 actual Node/helper tests with a synthetic
  SDK pass. The starter reaches that helper exactly once and returns three
  expected scalar values; 15 rejected variants submit no additional SQL, leave
  result sinks empty and preserve the connection. Relative-history guards pass.
  This counts helper dispatch, not actual SDK HTTP submissions or account effects.
- `sdk-starter-clippy-final.log`: scoped strict Clippy passes. Owned-file rustfmt
  and diff checks pass. Independent review reports no material finding.
- `sdk-starter-validate.log`: first full validation stopped at formatting on an
  unrelated in-progress guard module. After that module was formatted,
  `sdk-starter-validate-final.log` passes **607 frontend / 314 standard Rust**
  tests, contracts, production build, formatting, desktop check and strict
  all-target/all-feature Clippy. External fixture suites remain explicitly
  ignored in this standard command; their previous results are not a fresh run.
- `sdk-starter-browser-final.log`: all eight focused authentication, plan,
  completion and browsing/error browser workflows pass (52 seconds). Snowflake
  RPC responses are synthetic in these flows; the stale-publication case uses
  disposable PostgreSQL metadata. The existing editor check displays the
  default starter; its policy/dispatch behavior is covered separately above.

No account SQL, SSO, packaging or native run was performed for this narrow repair.
The preceding completion increment's normal bundle and hidden native evidence
identify their earlier source and remain distinct. The guard module and source
pin work in progress do not qualify a submission guard or enable writes.

### SDK completion increment (10 November 2026; locally qualified)

Implementation and its local qualification record are committed as `4c77cc4`.
The subsequent [browsing/session](snowflake-sdk-browsing-design.md) document is a
reviewed design only. The [submission guard](snowflake-sdk-submit-guard-design.md)
has a later implementation and separate qualification; it does not expand this
completion increment's account evidence.

Working source follows `5229bba`. It adds bounded same-session Snowflake relation
and resolved-column completion, context fingerprints, stale-response rejection
and structured global context invalidation. Browsing/inspection, writes, DDL and
transactions remain unsupported. All tests below are local; the actual-account
canary approval covered two fixed SELECTs and does not authorize metadata reads.

Evidence under `.cache/verification/connection-attempts/`:

- `sdk-metadata-pure-tests.log`: 15 parser/builder tests pass.
- `sdk-metadata-child-history.log`: 17 actual Node/helper adapter tests pass
  against a synthetic SDK. Expanded history-index guards pass separately in
  `sdk-metadata-history-indexes.log`; no history query reaches the helper.
- `sdk-metadata-studio-interim.log`: the first parallel fixture run failed six
  of nine tests. `sdk-metadata-studio-serial.log` passes eight and reproduces one
  fixture defect: an unclassified synthetic Error correctly becomes
  `OutcomeUnknown`. A synthetic definitive permission denial now supplies
  SQLSTATE `42501`. A process sample found synchronous full-Node SHA256 work
  during fixture construction; the nine actual-runtime fixtures now serialize
  locally without widening production startup/authentication deadlines.
- `sdk-metadata-browser-focused.log`: two existing Snowflake flows pass; the
  new intercepted SDK flow fails a missing case-policy assertion. The synthetic
  identity/expectation now explicitly uses `caseSensitiveTableNames: true`.
- `sdk-metadata-validate.log`: the intermediate source passes 607 frontend /
  312 standard Rust tests, formatting, contracts/TypeScript, production build,
  desktop compilation and strict all-target/all-feature Clippy. This precedes
  the global invalidation repair and does not qualify its final source.
- `sdk-metadata-global-before-fix.log`: independent review's stale-completion
  finding is reproduced. The existing outer identity-mismatch fixture fails
  because its error lacks the structured `completionInvalidated` signal.
  Backend snapshot removal alone cannot clear frontend relation/column caches.
  The repair preserves original error codes/messages and distinguishes global
  context failure from per-table denial; final combined checks remain pending.
- `sdk-metadata-child-global-final.log`: 18 actual Node/helper adapter tests
  pass against the synthetic SDK, including unusable NULL database context and
  cancellation during metadata. A definitive engine error retains its code while
  a retired session marks completion invalid. No retry or replacement connection
  is created by these cases.
- `sdk-metadata-studio-global-final.log`: nine serialized actual-runtime Studio
  tests pass, including identity changes at both the driver and outer Studio
  boundary, malformed/denied refreshes, per-table permission isolation and exact
  cleanup. This run precedes the final stale/missing revision marker assertion;
  its affected tests need a final rerun.
- The new browser regression initially passes two and fails two of four flows:
  one assertion expected text on an absent popup, and another held the sidebar's
  initial object response rather than a settled editor refresh. Attempt two
  passes three and fails one; attempt three also passes three and fails the
  unsettled sidebar case. Raw logs and retained artifacts use the
  `sdk-metadata-browser-global-attempt*` names. The settled editor fixture then
  passes all four focused flows in `sdk-metadata-browser-global-attempt4.log`.
- `sdk-metadata-cache-publication-mutation.log`: removing only the editor's
  cache-generation publication condition makes the targeted PostgreSQL flow
  fail: the released obsolete object reply appears in F1 documentation as
  `stale-object-list-marker`. The screenshot/context/trace are retained in the
  matching `-artifacts/` directory. The source was restored byte-for-byte
  (SHA256 `e1573c2b5b43bea0b2e43833beb7652115c1d8ada428549b997d90eb60da0abf`).
  Independent review clears the final guards and deterministic test; final
  combined qualification remains pending.
- The two final revision-marker regressions pass in
  `sdk-metadata-{stale,missing}-revision-final.log`. A separate strict E2E
  TypeScript check passes with `--types node`
  (`sdk-metadata-e2e-types-node-final.log`); its first invocation omitted those
  environment types and failed on `process`, without requiring a source change.
- `sdk-metadata-validate-final.log`: 607 frontend and 312 standard Rust tests,
  formatting, contracts/TypeScript, production build and desktop check pass;
  strict Clippy rejects a new pass-through `map_err` callback. It now uses
  `inspect_err`, preserving the original error and invalidation side effect.
  Independent review clears that adjustment. Final validation and Studio checks
  are recorded separately below.
- `sdk-metadata-studio-combined-final.log`: all nine actual-runtime Studio tests
  pass on the final callback/revision-marker source (185.44 seconds).
  `sdk-metadata-validate-combined-final.log` passes **607 frontend / 312 standard
  Rust tests**, formatting, contracts/TypeScript, production build, desktop
  compilation and strict all-target/all-feature Clippy. The 305 source/test/build
  input hashes in `sdk-metadata-qualified-source.json` match this source.
  The serial eight-suite real-engine run then passes **124/124**
  (`sdk-metadata-real-fixtures-final.log`): 42 MySQL/MariaDB, 11 retained guards,
  43 PostgreSQL, four permission, one client-protocol, six Redis, two SSH and 15
  transaction-transport tests.
- `sdk-metadata-browser-all-final.log`: **145/145** browser workflows pass in
  10.5 minutes, including both new completion regressions, actual existing-engine
  flows, million-row result-store coverage and workspace recovery. All 305
  recorded source/test/build-input hashes remain unchanged.
- `sdk-metadata-bundle-final.log`: the fresh normal SDK-bearing macOS bundle
  passes resource validation (15,104 entries, zero links after bundle copy),
  strict ad-hoc signature verification and actual packaged SDK hello/EOF startup
  with zero authentication/SQL requests. The existing local `rust-objcopy` LLVM
  stripping warning remains non-fatal. `sdk-metadata-bundle-artifact-final.json`
  records the 31,077,632-byte executable SHA256
  `6fca5fe3b11350740c6b02d7fec28b5de2f72b193b6da4ca18ff89273113f7bb` and unchanged
  runtime manifest `e99ae96378b685b7a49027c77fe36039d6a32a6f2f470b9353ae36ad27307d76`.
  Nothing was installed or published.
- `sdk-metadata-native-build-final.log` and its matching JSON record the fresh
  hidden SDK-bearing bundle. Resource validation, signature verification and
  guarded actual SDK startup pass. Standalone executable SHA256:
  `85e0071d3a4097503d8ee33337e0bce0452a9b1422a1c745567c1abbdcd0bc7c`;
  isolated bundle executable SHA256:
  `6c99911e08cbc910450acdc6e6e7d1dc393bf3a16d2da0e3d358019dbe5514e3`.
- `sdk-metadata-native-smoke-final.log` and `sdk-metadata-native-runs-final.json`:
  all **19 native check categories / ten cleanup cycles** pass (4,138 ms,
  main-process peak RSS 97.81 MiB in this synthetic fixture). Every checkpoint
  reports SDK runtime availability and zero connect/execute/prepare calls. This
  proves isolated local loading/workflows, not normal packaged sign-in, remote
  session persistence or representative account performance. All 305 recorded
  source/test/build-input hashes still match. Independent review has no remaining
  material finding in this increment.

The [metadata contract](snowflake-sdk-metadata-design.md) records exact bounds,
fixed SQL, session ownership, relative-history limitations and the next complete
browsing/session-context workflow. Fresh artifact identities are recorded above;
older artifact and canary evidence below retains its original source and scope.
No account metadata or complete Snowflake workflow is qualified by these checks.

### Connection-startup increment (10 November 2026; locally qualified)

Source follows local SDK foundation commit `1b7876f`. The frontend now reserves
an attempt before authentication, cancels by that exact identifier, and retires
late session responses instead of adopting them. A failed cleanup retains its
identifier for retry and reports an error. Native close saves workspace state,
blocks new UI interaction, rechecks pending work, cancels setup, and awaits the
backend shutdown barrier before destroying the window. That barrier establishes
local setup ownership; it is not proof of remote rollback or SDK child cleanup.

Reproduced defects and focused evidence are under ignored
`.cache/verification/connection-attempts/`:

- `ui-before-fix.log`: connecting disabled the dialog cancellation action.
- `shutdown-before-fix.log`: a legacy connection could publish after shutdown.
- `cancellation-before-fix.log`: cancellation could acknowledge during credential
  persistence or leave an already published session alive.
- `ui-focused.log`: 24 focused tests pass (18 connection dialogs and six close
  sequencing/interaction tests), including delayed responses, failed cleanup,
  cancellation during SSH preflight, declined host trust and reservation reuse.
- `validate.log`: intermediate validation passes 596 frontend and 239 standard
  Rust tests, formatting, TypeScript, build and strict Clippy. This predates the
  final startup-resource fixes and cannot qualify their final source.

Independent review identified and repaired detached startup tasks when PostgreSQL's
initial identity query, SSH key exchange, or MySQL's driver-internal settings query
stalls. Dedicated cancellation-owned socket/task cleanup regressions now pass.
Such loopback protocol fault tests prove the tested lifecycle interval;
they do not qualify a database engine or Snowflake account.

The SSH socket-owner fix separately passes three stalled-peer EOF regressions
and four existing SSH tests. Review then found that dispatched host-key probes also need registry
ownership: UI response suppression alone does not cancel native IPC. Prepared
preflight now uses the same identifier before probing and connecting; backend
claim/rearm and shutdown admission/drain checks pass nine integration tests.
A declined trust prompt retires its reservation. Final combined application
evidence follows below; intermediate pending states retain their original scope.

The MySQL regression first observed the hidden `SELECT @@wait_timeout`. Sending
result metadata, withholding its row and allowing 50 ms for consumption reproduced
failure to close within three seconds. That leak reproduction is timing-assisted.
The final regression has no sleep on its supported path: first query is the
application's `SELECT VERSION()`, and cancellation produces physical socket
closure. The option caches a client-side default and does not issue a server SET.
Independent review found no material issue in the startup/preflight repairs.

Six fresh isolated real database fixtures are healthy in Compose project
`joinery-datagrip-acceptance`; startup evidence is `fixtures-start.log` in the same
directory. The final eight-suite real fixture run passes **124/124** (42 MySQL/
MariaDB, 11 retained guards including TLS-over-SSH, 43 PostgreSQL, four permission,
one protocol fixture, six Redis, two SSH and 15 transaction-transport tests).
Command: the eight-suite command below under the identity increment, using the
current source and pinned toolchains; log `fixtures-final.log` in this directory.
`validate-final.log` passes **598 frontend and 258 standard Rust tests**, plus
formatting, TypeScript, production build and strict all-target/all-feature Clippy.
This includes the final startup/preflight changes, before the fixture reset repair
below. Browser/native/bundle qualification for this increment remains pending.

The first final browser run reproduced an obsolete harness assumption:
`disconnect_fixtures` called irreversible application shutdown. Subsequent
`prepare_connection` was rejected as cancelled before network setup. The first
browse-outcome test timed out; the lead interrupted the run rather than collecting
repeated downstream failures. Log and browser trace are retained as
`browser-before-harness-fix.log` and `browser-before-harness-fix/` in this directory.
The repaired test-only reset retires attempts/sessions at its configuration-gate
snapshot without reopening shutdown. Review found a second race: waiting for
**global** idleness while still admitting fresh requests could hang on a later
unclaimed reservation. The deterministic regression failed after three seconds
before repair (`fixture-reset-race-before-fix.log`). Reset now drains only the
claimed owners it canceled. All four reset regressions pass, including real
loopback connections, stalled authentication, fresh admission during drain and
irreversible shutdown (`fixture-reset-tests-final.log`). Final validation after
this harness repair passes **598 frontend / 262 standard Rust tests**, formatting,
TypeScript, production build and strict Clippy
(`validate-after-harness-fix.log`). The eight affected connection/browse browser
flows pass (`browser-focused.log`); full browser/native/bundle checks are pending.
The compiled source is committed locally as backend `363b893` and frontend
`b23dd7f`, with prior socket fixes `1b2c867` and `ec72f49`. The ignored
`qualified-source-snapshot.json` records the exact content hashes used for these
checks, including the preserved, unregistered future cleanup prototype; that
prototype is not compiled by this increment.

The subsequent full browser attempt was interrupted after reproducing two
harness/environment issues. At test 76, the trace records a complete Vite page
reload immediately after the lead's documentation edit; compiled-source hashes
were unchanged. Later tests exhausted the 32-connection bound because separate
browser pages shared the backend and left sessions alive. The recorded dialog
error is `At most 32 connections may be open or connecting`. The lead interrupted
the owned run (exit 130); this is not a passing full suite. Logs/traces are retained
as `browser-before-test-isolation.log` and `browser-before-test-isolation/`.
The repair disables integration-mode HMR and adds automatic per-test fixture
retirement. All 34 specs use the fixture; HTTP/RPC reset errors fail the test and
teardown runs in `finally`. The shared harness requires the configured single
worker. Production capacity remains unchanged. Independent review found no
material issue. The 33 affected connection, browse, rename, template and studio
flows pass in `browser-isolation-focused.log`. A fresh full run then passes
**141/141 in 7.8 minutes**, including the previously failing flows
(`browser-isolated-final.log`). All 350 source hashes in
`isolated-source-snapshot.json` remained unchanged during the run.

Final combined validation also passes **598 frontend / 262 standard Rust tests**,
formatting, TypeScript, production build and strict Clippy
(`validate-isolated-final.log`). The test-isolation repair is committed locally
as `504ab8d`; subsequent native and normal bundle qualification follows below.

The first native build succeeded but all three ordinary smoke runs failed on
the new test profile's empty username, before reservation checks. The existing
required-field validation rejected `save_profile`; production behavior was not
changed. Failure evidence and build identity are preserved in
`native-smoke-before-fix.json`, `native-smoke-before-fixture-fix.log` and
`native-build-before-fixture-fix.json`. The fixture now supplies a valid Redis
username (`d2d22cc`), and independent review confirms cancelled reservation claims
reject before socket dispatch. Full validation after that test-only fix again
passes **598 frontend / 262 standard Rust tests** and strict checks
(`validate-native-fixture-final.log`). Two actual PostgreSQL native recovery
pairs also passed on the earlier build, with pending-close intervals of
67.05 and 47.70 ms.

The corrected hidden native binary passes all three ordinary runs, each with
**19 summary check categories and 10 cleanup cycles**, plus **two actual
PostgreSQL close/reopen pairs**. The latter observe pending-close intervals of
69.20 and 57.05 ms, exact latest draft/preferences restoration, no pre-observer
connection/SQL invocation, real rollback and owned table cleanup. All processes
exit zero. Six recovery-runner acceptance tests pass. Logs are
`native-smoke-final.log`, `native-recovery-final.log` and
`native-recovery-runner-final.log`; complete copied reports are
`native-smoke-runs.json` and `native-recovery-runs.json`.

Native build identity (`native-build.json`): source `d2d22cc`, executable SHA-256
`5997f93e471e84a89a3e63faf8c12186d8b737b929e6795bbbc1b09a72cd7d94`,
isolated bundle binary SHA-256
`ad26fdba507759ed9f613eda249b13b2b76a15b1e5d35be9f14ed370d556a0ff`.
`sourceDirty` is true because documentation and preserved, unregistered SDK
drafts exist. The 350-file `native-source-snapshot.json` records compiled-source
content; its only difference from the passing full browser source is the native
fixture username. These are hidden WebView/isolated-workspace tests with an
in-memory secret store, not normal installed-app or OS credential qualification.
Native smoke cancels reservations before driver dispatch; it does not exercise
in-flight authentication or a native publication race. Rust socket/owner tests
and PostgreSQL browser races provide separate complementary evidence. Actual
Snowflake sign-in/cancellation and normal packaged restart remain open.

The normal macOS arm64 bundle also passes `pnpm desktop:bundle:mac` and
strict deep ad-hoc signature verification: **29.17 MiB** reported by Tauri,
identifier `com.quiinthieu.joinery`, executable SHA-256
`c1d7e9c30c42b2f512f3858dcd6e6cb07b9f0ca183858290965f1a93417f3706`.
Evidence is `bundle-final.log` and `bundle-final.json` in the same directory.
It was built from `d2d22cc` with the same compiled-source content as the native
snapshot, before SDK integration. It was not launched, installed, notarized or
published. This completes the local lifecycle increment's combined checks; it
does not qualify the still-unimplemented Snowflake application SDK route.

The first release build's debug-stripping subprocess could not resolve the
installed Rust toolchain's `libLLVM.dylib`; compilation still exited zero.
`rust-objcopy --version` succeeds with command-local
`DYLD_FALLBACK_LIBRARY_PATH` pointing at that toolchain's `lib` directory. The
corrected native build was also attempted with that scoped setting, but the full
build chain still reports a stripping warning. The direct tool check is not a
successful pipeline repair. No global configuration was changed; no binary-size
or stripping qualification is claimed.

That attempt did complete the actual PostgreSQL million-row browser flow in
21.2 seconds. Its CSV contains 103,888,906 bytes and 1,000,000 line breaks,
SHA-256 `958d2140f0d9ac59335d33e2c17b9ed363c4c08ee8cf5e4db5775ae59f89d34e`;
see `million-row-browser-export.json`. The fresh isolated run repeats that flow
in 19.0 seconds, with the same independently verified byte count, line count and
SHA-256 (`million-row-browser-isolated.json`). This local fixture does not qualify
Snowflake results or representative performance.

The exact Snowflake helper production lock also passes a fresh public-registry
npm audit on 10 November: zero advisories in all severities across 183 dependencies
(Node 24.22.2/npm 11.19.1, lock SHA-256 `e23933ecb16badf5e9621cb089796193388493f9230976fa4b8c48efa9f13098`).
The audit used copied public package files, isolated HOME/cache/configuration,
`--package-lock-only --omit=dev --ignore-scripts`, and changed neither source nor
lock. Evidence is `.cache/verification/snowflake-security/20261110T043620Z/`.
This is advisory-database evidence, not proof of application/runtime security.

The added SSH browser case uses an intercepted synthetic host-key probe with a
real backend reservation; it does not qualify actual SSH transport. No external
SSO, query, write or billable operation has been performed.

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

## 10 November 2026: SDK driver foundation

Pinned Node 24.22.2 / Rust 1.99.1 checks on the local feature branch:

```sh
pnpm validate
pnpm test:snowflake:helper
pnpm test:snowflake:process
pnpm test:snowflake:runtime
pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline
pnpm test:snowflake:startup
```

Full validation passes **582 frontend / 213 standard Rust tests**, application
build, formatting, desktop Cargo check and all-target/all-feature strict Clippy.
The first full run caught the old SQL API test expecting an untyped integer
placeholder. The request now uses `CAST(? AS NUMBER(38,0))`; the assertion was
updated and the complete validation rerun passes. SQL API binding payloads and
source-location mapping remain covered by that loopback test.

The helper passes **31/31 tests**: six actual SDK loopback HTTP tests and 25
synthetic protocol/configuration/stream tests. The process runner passes **8
supervisor units, 30 real-child supervisor integrations and 11 adapter
integrations**. The adapter tests use the actual framed helper with a synthetic
SDK; they are not Snowflake account tests. A deterministic barrier reproduces
late credit/discard rejection overriding acknowledged success before the repair.
The repaired adapter preserves original success, database/transport errors and
unknown outcomes while treating cancellation acknowledgement separately.
Independent review of the helper, supervisor, adapter and staged-startup check
has no unresolved material finding.

Runtime preparation passes **9/9 tests** and the actual offline command above.
It extracts npm from the verified Node archive, installs locked dependencies
with scripts disabled and isolated configuration, preserves relative internal
symlinks, and atomically promotes the stage. Post-promotion backup-cleanup
warnings do not rewrite the promoted manifest or falsely report install failure.
The staged startup check imports the actual installed SDK, detects no attempted
Node socket/DNS traffic, emits one valid hello and exits cleanly on EOF without
authentication/SQL requests. This is not an OS network sandbox or SSO evidence.

Local logs now live under `.cache/verification/snowflake-foundation/`:
`validate.log`, `helper.log`, `process.log`, `runtime-tests.log`,
`offline-prepare.log`, `staged-startup-final.log`, and
`adapter-race-before-fix.log`. `stage-identity.json` records the current staged
identity. The 183-package stage manifest SHA-256 is
`1a7b068a85f4d085c8415b8fd6256acb54b803ab0e22b668068541de26bb182d`;
the Node binary hash is
`b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a`.
The original Node signature identifies the Node.js Foundation, team
`HX7739G8FX`. These hashes identify a developer stage, not a signed Joinery
bundle. Earlier `/tmp` paths in this ledger are historical evidence references;
those files did not survive the environment refresh and are not currently
available for inspection. Earlier recorded outcomes are not fresh reruns.

The SDK retransmits after a lost successful response using the same request ID
and body hash. A deduplicating simulator applies one effect; a simulator without
deduplication applies two. Persistent loss again has no terminal within the
bounded observation; the observed POST count varies and is not an exhaustion
guarantee. Real Snowflake deduplication, write/commit outcomes and rollback remain
unqualified, so writes, DDL and transaction controls stay disabled in this route.

Application profile/SSO integration, pending-auth shutdown, metadata, actual
Snowflake million-row behavior, resource bundling, complete notices,
signing/notarization, installation and owner acceptance remain open. No real
Snowflake login/query, billable work, remote write, push, publication or
deployment occurred. The latest stable DataGrip release API was rechecked on
10 November: still 2026.3, build `263.7307.216`.

## 10 November 2026: explicit SDK profiles and retained cleanup

Current integration status and bounded proof are in
[Snowflake SDK application integration](snowflake-sdk-integration.md).
The required all-applicable-DataGrip target, including Snowflake writes, DDL,
transactions and one-million-row results, remains open.

The local application source implements explicit authentication modes, immutable
native runtime selection, reserved cleanup permits, identity-retaining removal
authority and command-facing cleanup receipts. New Snowflake profiles select SDK
external browser; old missing-mode profiles retain OAuth. Focused tests cover
restart and metadata backup/restore, failure without native resources, cancellation
during actual-helper synthetic authentication, abandoned futures and shutdown.
No automatic authentication fallback or SQL replay was introduced.

`pnpm validate` passes **604 frontend / 297 standard Rust tests**, frontend build,
desktop compilation, formatting and strict all-target/all-feature Clippy.
Additional focused evidence:

| Command/check | Result | Local evidence |
|---|---|---|
| Runtime factory units | 18 pass | `sdk-runtime-factory.log` |
| Cleanup authority/registry units | 15 focused pass | `sdk-cleanup-owned-final.log` |
| Actual Node/helper through Studio | Four pass | `sdk-studio-after-apfs.log` |
| Actual-child supervisor/read adapter | 31 / 11 pass | `sdk-child-adapter.log` |
| Profile persistence | Six pass | `sdk-profile-interim.log` |
| Focused browser | Six pass | `sdk-browser-focused-final.log` |
| Preparation/startup script regressions | 48 pass | `sdk-runtime-scripts.log` |
| Resource validator/copy regressions | Seven pass | `sdk-native-resources-final.log` |
| Actual staged resources | Pass; 15,104 entries / five contained links | `sdk-stage-resources.json` |
| Actual staged SDK hello/EOF | Pass; zero authentication/SQL requests | `sdk-stage-startup.log` |
| Actual isolated database fixtures | 124 pass | `sdk-real-fixtures.log`; eight sequential suites on synthetic local engines |
| Normal SDK macOS bundle | Pass; 223.41 MiB, strict ad-hoc signature, resources and actual SDK hello/EOF | `sdk-bundle.log`, `sdk-bundle-artifact.json`; no authentication/SQL |
| Full browser suite | All 143 pass in 9.5 minutes | `sdk-browser-all-final.log`; 264-file source comparison unchanged |
| Hidden SDK native application | All 19 checks / ten cleanup cycles pass | `sdk-native-runs.json`, `sdk-native-summary.json`; packaged runtime available at all six checkpoints, zero connect/execution calls |
| Native recovery runner | Six tests pass | `sdk-native-recovery-runner.log` |
| Standalone hidden native PostgreSQL recovery | Two pairs / four processes pass | `sdk-native-recovery-runs.json`; pending-close exit 74.24ms / 50.61ms, actual rollback, restored drafts/preferences, zero reconnect/replay before observation |

These logs are in `.cache/verification/connection-attempts/`; `sdk-validate.log`
records full validation. The four actual Node/helper tests use a synthetic SDK;
the two offline browser-profile flows use actual persistence but the harness
refuses Snowflake authentication. Other Snowflake browser flows explicitly
intercept the API boundary. Neither is an actual account qualification.

The APFS receipt defect was reproduced: successful `remove_dir_all` did not make
an open directory handle's link count zero. The corrected receipt accepts the
successful removal after original-directory/parent identity checks. Replacements
remain untouched; a failed removal retains retry ownership after child reaping.
Initial failures are preserved in `sdk-cleanup-owned-interim.log` and
`sdk-studio-interim.log`. A subsequent rerun caught a test-local missing variable
before the final four tests passed. An initial browser run correctly failed the
local-only harness guard; the offline Snowflake fixture now permits one synthetic
profile shape while continuing to forbid both authentication routes.

Independent review cleared the runtime/profile/lifecycle integration after the
APFS repair. Packaging review found unchecked source, supplements/provenance,
Node LICENSE and permission bits plus the regular bundle route bypassing checks.
Those findings were fixed and reviewed again. A root wrapper return-shape error
was corrected before the actual stage gate passed. Source and manifest checks
do not authenticate all dependency bytes or resolve the remaining proxy notice
applicability gap. Initial filesystem deletion is synchronous, so async deadlines
do not bound blocked local filesystem I/O. Same-user hostile pathname mutation
is outside the trusted-parent boundary. Forced promotion rollback has source
review but no injected rename-failure test.

Notice preparation is committed separately as `daa4203`. Its offline stage has
manifest SHA-256 `e99ae96378b685b7a49027c77fe36039d6a32a6f2f470b9353ae36ad27307d76`;
the unchanged production lock has 183 dependencies. See
[notice evidence](snowflake-sdk-licenses.md). Those local integration checks ran
without Snowflake authentication or SQL. The separately approved live read
canary is recorded below. No remote write, installation, push, publication or
deployment ran.

Application integration is committed as `521b086`; packaging is `cdfba88`. The
normal and hidden native build provenance predates these commits and records
`daa4203` with `sourceDirty=true`. The normal application source was unchanged;
the first browser comparison saw only package-script changes. After correcting
the old sign-in-copy assertion, the final browser comparison found no changed
paths among its 264 recorded files. New canary preparation files and documentation
were independent of the compiled application.

The normal SDK bundle is 223.41 MiB; its executable SHA-256 is
`a6b09c0b0a73d9626cbf7bab9e9a5bdb1c69ffeda0172f0f04dd96b5f50ad1f2`.
The hidden bundle executable SHA-256 is
`2fad04be58bc700ab3807cb9240c52b843030c3668efcc463030a4a26879235b`.
The runtime manifest is the same `e99ae963…07d76` above. Normal Tauri packaging
dereferenced the five dependency links, retaining 15,104 validated entries;
hidden preparation preserved them. Both actual packaged SDK startup checks
reported zero authentication/SQL requests. The hidden native smoke passed in
4374ms; all six checkpoints reported runtime availability. Its 97.98MiB peak
main-process RSS is a smoke observation, not representative performance evidence.
The standalone PostgreSQL recovery route does not qualify bundled SDK discovery
or Snowflake transactions. Normal-owner-state startup/restart is still open.
Both release builds succeeded with the existing `rust-objcopy`/`libLLVM.dylib`
stripping warning; no global toolchain change or hook bypass was used.

## 10 November 2026: approved two-query Snowflake read canary

The owner approved the concrete plan committed as `f43f24f`, including possible
warehouse billing and SDK retransmission. The scope is the existing connection,
external-browser SSO and exactly two fixed SELECT requests: session/context and
synthetic scalar values. It excludes object reads, writes, DDL, transactions,
metadata enumeration and million-row workloads. See
[the plan and command](snowflake-read-canary.md).

The first live invocation at `2026-11-10T05:39:15.569Z` failed with the safe code
`resultMismatch`. It authenticated and submitted two helper query requests;
the child was reaped with a clean exit and its private local authentication
directory was removed. This is a failed qualification, not an account-readiness
pass. The SDK may retransmit, so the helper-request count is not an assertion
about the number of physical server submissions or charges.

Safe evidence: `.cache/snowflake-read-canary/0ad5c2eb-079f-4bb5-8add-ddf095053939.json`;
runner output: `.cache/verification/connection-attempts/sdk-read-canary-live.log`.
Runtime manifest SHA-256 is `e99ae96378b685b7a49027c77fe36039d6a32a6f2f470b9353ae36ad27307d76`;
context fingerprint is `e0298912a552c903ee975103eb0549d55076f22cea478d0b5310aaad1815eec7`.
No profile values, query rows, session IDs, URLs or raw SDK errors are retained
in this evidence. Consequently the original failure cannot be conclusively
attributed from its discarded result values.

Source inspection found a local validator contract defect: it expected a fixed
numeric session ID, but Snowflake documents `CURRENT_SESSION()` as an alphanumeric
VARCHAR. A synthetic regression with text-typed `session-A9b7` reproduced failure
before correction. This establishes the local defect, not the sole cause of the
live failure. [Official function contract](https://docs.snowflake.com/en/sql-reference/functions/current_session).

The corrected runner was independently reviewed, passed all 11 synthetic tests
and actual default PLANONLY, and was committed as `9e39e51`. Both approved SQL
strings were verified byte-for-byte unchanged against `f43f24f`. The same-scope
live rerun at `2026-11-10T05:49:42.148Z` **passed**, using the same runtime and
context fingerprints above. The owner had already explicitly approved this
scope and possible billing; no new query scope was introduced.

All recorded booleans passed: connection, per-query byte bounds/row
shapes/column names/types/principal/session, configured context, principal/session
continuity, exact synthetic scalar strings/NULL, logout acknowledgement, actual
child reaping, clean exit and original private authentication-state removal.
Safe evidence: `.cache/snowflake-read-canary/437fd1f6-c2b7-4164-a360-b31782e26b14.json`;
runner log: `.cache/verification/connection-attempts/sdk-read-canary-session-fix-live.log`.
Offline evidence: `sdk-read-canary-session-fix-tests.log` (11 pass),
`sdk-read-canary-session-fix-plan.json` (PLANONLY, no helper/auth/SQL).

This is actual account evidence for the two-query SDK helper read workflow. It
does not qualify normal packaged application login/restart, object metadata or
data, cancellation/interruption, expired authentication, million-row results,
writes, DDL, transactions or owner replacement acceptance. Logout acknowledgement
is the helper/SDK response, not an independent server-session inventory check.
No object reads or remote writes were requested, and no additional account
queries are authorized by this canary plan.
