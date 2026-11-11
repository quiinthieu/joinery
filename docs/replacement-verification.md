# Replacement verification ledger

Run started 9 November 2026 on macOS arm64, source `bcb606f`, task branch
`feat/datagrip-replacement` from fetched `origin/main`. Local work is unmerged,
unreleased and not installed. Actual-account evidence is limited to the explicit
canary scopes recorded below; complete external workflow qualification is open.
See [acceptance](replacement-acceptance.md) for required gates and migration.

## Environment and commands

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
