# Implementation and release status

Updated 2026-10-26. The source includes PostgreSQL, MySQL/MariaDB,
standalone Redis and the read-only Snowflake SQL/catalog pilot. Preview.6 added
metadata startup/version protection. The current hardening slice adds Snowflake
OAuth/cancellation/disconnect fixes, dialog keyboard fixes and process-crash
recovery coverage. The canonical Site records exact verification, merge and
published download states. Full multi-engine v1.0 qualification remains pending.

## Recovery and accessibility continuation

Connection setup now separates successful sign-in from table discovery. A
connected session opens its workspace immediately; discovery failures produce a
notification and can be retried with Refresh schema. Retrying an unchanged
production profile does not require a write passphrase. Actual production
setting changes remain gated, and a newly saved profile becomes editable if
its first connection attempt fails. These paths are verified with synthetic
Snowflake/browser fixtures; this does not qualify a live production account.

The SQL editor suggests catalog schemas, tables, and columns as users type.
Snowflake completions include `database.schema.table` names, and referenced
tables in that form load their columns for alias suggestions. Suggestions use
the connected session's catalog and lazy table-schema lookup; they are verified
with local fixture metadata and a browser typing flow.

Snowflake result partitions now support the SQL API's gzip encoding. A local
HTTP regression reproduces the partial-result failure with 85 initial rows and
15 compressed rows, then verifies all 100 rows complete with exact numeric text
and nulls preserved. The 16 MiB response limit applies after decompression;
oversized and corrupt compressed responses remain errors and never report a
complete result. Live account retesting remains pending.

Keyboard workspace and result tabs now support wrapping arrows and Home/End,
roving focus and labelled panels. The editor splitter supports keyboard resizing.
Dialogs remove background controls from assistive navigation and restore prior
attributes on close; descriptions, explorer expansion, inspector announcements,
focus indicators and reduced-motion handling are included. Two frontend
regressions and a real keyboard browser flow pass. Assistive-technology and
packaged platform qualification remain open.

Preferences now provides app-owned SQLite metadata snapshots, guarded restore,
undo snapshots and explicit deletion. Snapshots include committed WAL data and
are published as closed single files with private Unix permissions. Integrity,
schema version/shape, typed records, unexpected schema objects and selected-file
digests are checked before restore. Paths never cross IPC. Limits are 20 files
and 64 MiB per file. Restore serializes connection/configuration changes, requires
an idle studio, exact confirmation and the current configured passphrase, and
retains current OS credentials/passphrase state. It never reconnects or replays
statements. Seven Rust regressions and a real backup/restore/undo/delete/reload
browser flow pass. A child process is killed after the first restore backup step;
reopening retains the complete pre-restore metadata and the undo snapshot. Other
platforms have not been exercised. The UI needs a store that can open;
incompatible-startup recovery still requires a compatible app and manual copy.

Query draft recovery is separately opt-in per profile and defaults off when
opening older profiles. The backend filters every save against current policy;
disabling draft storage or deleting a profile removes its current saved text in
the same transaction. Results, sessions, edits and execution state cannot enter
the typed workspace snapshot. Bounds are 32 drafts, 1 MiB each and 2 MiB total.
Saves are serialized and native close/quit flushes the 400 ms debounce. Startup
merges recovered drafts with any tabs opened during bootstrap. Recovery never
reconnects or executes. Theme, layout and grid density persist too. Three Rust
tests, two serialized-writer tests, a startup-race regression and a real browser
restart/opt-out flow pass. Keyboard cell navigation updates the announced
inspector, tab closure restores focus, and mode buttons expose their selection.

Local baseline validation, including strict Clippy, passes. All thirteen browser
workflows pass against disposable databases or synthetic Snowflake responses.
The canonical project Site records the exact merge, published preview and
download-verification states for this continuation.

The local macOS packaging check reproduced a linker-only signature that failed
strict resource verification. The bundle command now inspects signing, seals
ad-hoc local bundles when needed, and verifies the final app. It does not replace
real signing identities or claim notarization. Native validation passed in three
clean-source processes with 30 cleanup cycles; detailed local evidence is under
`.cache/p7-clean-verification/`. External release gates remain open.

The preview also contains a **Snowflake read-only SQL pilot**:
system-browser OAuth with PKCE, explicit account context, async SQL API polling,
bounded JSON partitions, exact text values, own-query cancellation, and a
read-only connection form. Mocked callback and SQL API tests, the native macOS
build, and a headless Snowflake form check pass. A live read-only account canary
also passed browser SSO, context identity verification, and `SELECT 1 AS CANARY`
with one result row. The canary exposed a redirect-path mismatch and an
unsupported SQL API `AUTOCOMMIT` request parameter; both were fixed and retested.
Preview.3 adds database-scoped catalog discovery, lazy column completion, structure, and bounded read-only table previews. Live table qualification and the remaining P6 criteria are not complete. The
previous `v0.1.0-preview.1` release remains available as the PostgreSQL-only preview.

| Status | Scope |
|---|---|
| Shipped in preview.5 | PostgreSQL studio, SSH/TLS, editing/review, Safe Mode, themes, history/audit, bounded grid/Arrow results |
| Shipped since preview.4 | MySQL/MariaDB studio; fixture-qualified on MySQL 8.4 and MariaDB 11.4 |
| Shipped in preview.5 | Standalone Redis 8.2 workspace with SCAN/type/TTL, bounded typed viewers and reviewed commands |
| Shipped pilot in preview.5 | Read-only Snowflake browser OAuth, SQL, catalog/columns and bounded table previews |
| Shipped since preview.6 | Local metadata schema/version guard and transactional initialization |
| Current hardening slice | Snowflake credential lifecycle and cancellation/disconnect; keyboard focus; WAL crash recovery and closed-store restoration tests |
| Pending | Broader live account/version qualification, signing/notarization, packaged Windows/Linux, measured performance and full recovery/update qualification |

## Implemented

| Area | Current behavior |
|---|---|
| Shell | Tauri v2 system WebView; React/TypeScript; lazy editor/grid/worker bundles; explicit command capabilities and CSP |
| Connections | Private local SQLite metadata; nested groups with inherited, immutable environments; protected production profile edits; session passwords and optional native credential storage; saved connection deletion uses a dedicated confirmation without a passphrase |
| SSH | Password, encrypted private key, and agent authentication; optional agent socket path; explicit fingerprint trust; changed-key rejection; reference-counted tunnel lifecycle; loopback listeners; bounded forwarding channels |
| TLS | System trust plus a custom PEM CA; production requires certificate verification; tunneled database connections preserve the original hostname for verification |
| SQL | CodeMirror tabs; selection/current-statement/all-statement execution; worker completion index and formatting; per-statement result tabs and metrics; cancellation; query history |
| Browsing | Keyset paging in native PK order; byte-bounded pages; evicted-page reloading; PK/FK structure; table statistics; schema-change and stale-request guards |
| Editing | Sparse dirty state; NULL-aware and type-aware controls; immutable generated SQL plus bindings; review before apply; one transaction across pages; row locking and original-value conflict detection |
| Safety | Rust-enforced policies; production passphrase gates; expiring, one-use approvals; readonly PostgreSQL session defaults; explicit uncertain outcomes; no automatic write retries |
| Operations | Health, process inspection, query-identity checks before process cancellation, metadata history, digest-only local audit viewer |
| UX | Dark, light, and high-contrast themes; keyboard actions; command palette; inspector; pending-work close/quit confirmation |

Grid browsing and commits use the login role independently of SQL-editor transactions and session settings. Grid commits use a separate transaction and cannot commit an editor transaction. Arbitrary query results, views, generated columns, and unsupported primary-key types remain read-only.

## Resource bounds

- Rust result buffers: **64 MiB globally**, **100,000 rows per query result**, 100 retained result descriptors.
- Frontend Arrow buffers: **32 MiB globally**, 512 cached pages, four in-flight requests.
- Arrow IPC: **4 MiB per frame**, checked before encoding; wide ranges trigger smaller frontend requests.
- Table pages: **500 rows or 2 MiB**, with keyset continuation from the last retained PK.
- Raw result rows: **2 MiB viewer limit**; large write results are discarded while execution is drained to a known outcome.
- Pending edits and review originals: **8 MiB**, at most 500 edited rows per commit.
- File inputs: bounded reads of regular CA/key files; no arbitrary frontend filesystem API.

These bounds constrain owned buffers; they do not include WebView, operating-system frameworks, driver receive buffers, transient encoding, or JavaScript object overhead.

## Validation performed locally

### Build policy

GitHub Actions is enabled for public-repository validation on standard Linux,
Windows, and macOS runners, with no artifact uploads, caches, larger runners,
custom images, or paid external services. Every job skips if the repository is
private. Pull requests and pushes to `main` run `pnpm validate`; a Linux job
runs disposable PostgreSQL/SSH/browser integration. No production credentials
are used. Release packaging and native platform UI checks remain local.

Run `pnpm validate` locally before shipping, followed by integration and
native/package checks appropriate to the change (commands in the README).
CI compilation does not qualify packaged Windows/Linux app behavior.

The local-only build-policy change passed `pnpm validate` on macOS: 13 frontend
tests, 27 Rust tests, frontend production build, Rust compilation, formatting,
and strict Clippy. The 10 disposable database integration tests were ignored by
that baseline command; no new production connection checks were run.

### Recorded application checks

- **28 Rust tests passed**, including live PostgreSQL and SSH/TLS integration tests.
- **13 frontend tests passed**, including exact Arrow values, wide-page adaptation, dirty tracking, SQL statement boundaries, host trust, and stale schema refreshes.
- **4 headless browser workflows passed** against the real Rust service and disposable PostgreSQL: connect/query; edit/review/commit/readback; guarded writes/cancellation/million-row scrolling; nested groups/audit viewing.
- Rust Clippy passed with all targets/features and warnings denied.
- Release frontend and native macOS bundle builds passed.
- Hidden native WebView checks passed in three fresh processes: binary `ArrayBuffer` channels, Arrow worker decoding, exact bigint text, NULL/empty/Unicode, formatter worker, CodeMirror, Canvas grid, and native close protection.

The browser HTTP bridge and native fixture commands require separate test features. They are outside the normal desktop release. The contract exporter also requires a separate feature and is excluded from the app bundle.

Notable regressions covered: numeric keys accidentally sorted as text; stale async schema responses; changed column positions between schema and page fetch; cross-page transaction rollback; grid commits alongside an open SQL transaction; queued cancellation targeting an active run; changed SSH keys; database TLS hostname verification through SSH; writes completing after viewer closure or result-budget exhaustion.

## Native timing and memory observations

Platform: Apple Silicon, macOS 27.2. The hidden fixture loads 20,000 result rows and opens the editor/grid. Samples include startup peaks and a close-guard interaction.

| Run | Native start to completed checks | Main peak RSS | Main peak physical footprint |
|---|---:|---:|---:|
| First | 3,338 ms | 99.72 MiB | 29.06 MiB |
| Subsequent fresh process | 668 ms | 105.36 MiB | 30.20 MiB |
| Subsequent fresh process | 619 ms | 105.02 MiB | 30.03 MiB |

The later processes benefit from warm OS services and caches. These are integration-test timings, not visible first-paint measurements. WebKit helper memory is excluded; peak main-process figures are not total idle app RAM. The **50–80 MB idle target, fully cold sub-second startup target, and 100+ FPS target remain unqualified**. The million-row test verifies virtualization and scrolling, not a display-frame-rate guarantee. Engine implementation continues while these release-qualification gates remain open.

The reproducible runner writes detailed local evidence to `.cache/native-smoke/runs.json` and identifies the exact test executable by SHA-256.

## Remaining release work

1. Qualify total idle memory including WebKit helpers, visible first paint, and frame times on representative hardware; address regressions before v1.0 qualification.
2. Exercise native credential-store success/denial/locked-store flows. Current connection tests inject a memory store and never access personal credentials.
3. Run packaging and platform UI smoke checks on Windows and Linux as well as macOS. Only macOS was executed in this workspace; CI compilation does not qualify packaged app behavior.
4. Add Developer ID signing/notarization and release/update recovery. The delivered local macOS bundle uses ad-hoc signing.
5. Complete P4/P5 production/version qualification, and broader Snowflake live table/OAuth/result/cancellation qualification. Preview.3 includes read-only catalog/grid browsing; preview.2 covered SQL only.
6. Complete P7 accessibility, cross-platform releases, crash/upgrade recovery, and sustained performance qualification.

## Continuation batch A: Snowflake browsing (shipped in preview.3)

PR #7 (`846dd0a`) adds catalog discovery and estimates for the
configured database, lazy column metadata feeding existing completion workers,
structure inspection, and read-only table previews. Previews stop at 500 rows
or 2 MiB, are explicitly unordered, and do not expose continuation or editing.
Catalog discovery rejects incomplete/oversized results rather than presenting a
partial inventory. Metadata uses a separate bounded lane and local statement
handles so it cannot replace the editor cancellation target. Heartbeats remain
query-free. New Snowflake tabs use Snowflake identity/version functions.

This code is included in **preview.3**, published after all five PR checks passed.
The downloaded ZIP, all three release assets, executable digest and ad-hoc
signature were verified against the source manifest. The build source
`0b1f22f` and merged release `846dd0a` have identical complete Git trees.
Live account/table qualification is
pending; the earlier identity/SELECT 1 canary did not test these additions.
Seven new adapter regression tests use a synthetic loopback SQL API. They cover
multipart catalog/preview responses, object quoting, exact values, NULL/empty,
row/byte limits, schema changes, invalid continuation, cancellation isolation,
and write rejection. The browser workflow separately uses synthetic API/Arrow
responses; PostgreSQL browser/integration tests continue to use disposable data.

Local verification on macOS Apple Silicon passed `pnpm validate` (13 frontend
and 34 Rust tests, production frontend build, formatting, compilation and strict
Clippy), all eight disposable PostgreSQL integration tests, all six headless
browser workflows, and three hidden native WebView runs. The native smoke
binary digest was `8a4f4c0190e352c2b8018a8802840167bb525ae1993f575d3646c2202dd1a536`.
Its native completion times were 2,810 / 663 / 531 ms; these remain hidden-fixture
timings with main-process-only memory, not cold visible-startup or idle-RAM gates.
The normal macOS application bundle also built locally and passed ad-hoc signature
verification after re-signing the stale resource seal left by the incremental
bundler. Developer ID signing/notarization remains pending. No production databases
were connected during this batch. [Download preview.3](https://github.com/quiinthieu/joinery/releases/tag/v0.1.0-preview.3).

## Continuation batch B: validation infrastructure (qualification incomplete)

The runtime harness adds test-only shell/loading/cycling/release checkpoints, source
provenance and dirty-tree flags for native builds, and phase-specific main-process
memory samples. Three hidden native runs passed ten tab/result cleanup cycles
each, including Arrow requests in flight at closure. Rust result counters and
frontend Arrow bytes returned to zero after every cycle. A new disposable
PostgreSQL regression also passed ten connect/run/release/disconnect cycles;
all nine PostgreSQL integration tests passed. The baseline `pnpm validate` also
passed: 36 Rust tests, 13 frontend tests, production frontend build, formatting,
Rust compilation, and strict Clippy. Twelve tests remain ignored in that baseline
(nine PostgreSQL, two SSH/database, and one real credential-store qualification).

The native OS credential-store round trip passed on macOS using one random
synthetic entry, followed by deletion and absence checks. It initially failed
inside the filesystem sandbox and passed with native Keychain access. No saved
connection credentials were accessed. Injected failure coverage verifies that an
unavailable store fails before network dialing, leaves no session, preserves
saved configuration when deletion is denied, and redacts OS error details.
Real OS denial/locked-store interactions remain pending.

The final native fixture binary digest is
`e43f658af8c043be4fa62bb8da7ceab0b30758119334ea15b20bb745efd3bea5`.
It was built from the batch B working tree, explicitly recorded as dirty in its
build manifest. Hidden shell DOM checkpoints were 1,320 / 393 / 402 ms. Median
main-process footprint during the empty-shell interval was 26.59 / 27.45 / 26.97
MiB, and after result release 27.91 / 29.75 / 28.42 MiB. Helpers are excluded;
these short observations do not establish total idle RAM or sustained stability.
The 50–80 MB total-idle, cold visible-startup and 100+ FPS gates stay unqualified.

This batch ships source validation tooling through [PR #8](https://github.com/quiinthieu/joinery/pull/8);
preview.3 remains the application download. The test-only harness is excluded
from that normal app. Complete helper attribution,
representative visible startup/frame-time qualification, real locked-store cases,
and sustained cleanup checks remain release gates while engine implementation continues.

The detailed blueprint remains in [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).
The private [ChatGPT Site](https://joinery-project-hub.otohime-nokimi.chatgpt.site)
is the canonical plan, implementation-progress, and shipping-status destination.
Update it after implementation, verification, merge, and release milestones;
the former plan and testing Pages are retired.


## Continuation batch C: MySQL/MariaDB studio

The new `mysql_async` adapter is verified against disposable MySQL 8.4 and
MariaDB 11.4. It supports connection health, verified system/custom-CA TLS,
original-hostname verification through SSH, CodeMirror execution/formatting/
completion, exact bigint/decimal values, individual result sets, catalog/PK/FK
inspection, keyset paging, and reviewed InnoDB edits. Auxiliary connections keep
grid commits separate from editor transactions. Generated, binary, floating-point
columns, views, MyISAM tables, and unsupported keys stay read-only.

MySQL-specific enforcement rejects executable comments and parser/charset mode
changes. Fixed backend session settings are restored before every editor
statement. Grid updates use typed bound parameters, metadata/row locks, full
original comparisons, matched-row counts, strict warnings, and one transaction.
Lossy integer/decimal edits are rejected before review. Cancellation holds the
owned connection identity until control requests finish and quarantines the
session. A server-acknowledged write remains completed if cancellation arrives
too late; uncertain writes are never retried. Full viewers discard output while
draining writes, so result-budget exhaustion does not misreport a committed write.

Process inspection is available; killing external MySQL processes is deliberately
unavailable because a process-list snapshot cannot authorize a later reused ID.
The SQL toolbar cancels Joinery's owned run. DDL may implicitly commit an editor
transaction. Broader server versions, live production accounts, OS/platform
qualification and signed distribution remain open. Redis is the next engine.

The final `pnpm validate` passed 38 Rust tests, 14 frontend tests, the production
frontend build, formatting, native compilation and strict all-feature Clippy.
Local database validation passed nine MySQL/MariaDB integration tests (each
exercising both engines), nine PostgreSQL integration tests and two SSH/database
tests. Eight browser workflows passed, including actual connect, catalog, inline
edit, generated SQL review, commit/readback and lazy completion for both new
engines. These use disposable synthetic data; no production databases were read
or modified. Build/native/release evidence is recorded on the project Site after
completion.


## Continuation batch D: Redis workspace

Standalone Redis has a native CodeMirror console, command completion, current
line/selection/batch execution, SCAN browser with type/TTL inspection, and
string/hash/list/set/sorted-set previews. Typed forms generate commands through
the same immutable plan, one-use approval and production passphrase gateway.
Every Redis write requires review. Newline/quoted-byte parsing is shared by
review and execution; the complete batch is preflighted before any command runs.
Commands execute sequentially without rollback or automatic write retries.

The adapter uses redis-rs encoding/parsing over owned async TCP/TLS streams.
RESP2 framing caps declared bulk lengths, array nodes and nesting before decode.
Replies and decoded viewers stop at 2 MiB / 10,000 nodes / 16 levels. String
previews stop at 64 KiB; other previews stop at 500 entries / 2 MiB. Binary
values carry explicit encoding metadata. Opaque browser cursors are scoped to
pattern/key/type and expire after five minutes. Empty SCAN pages retain their
continuation; duplicates and expiring/changing keys are explicitly live behavior.

Cancellation closes the owned socket and retires the session. A lost write
acknowledgement is outcome unknown. A full result registry still reports a
server-acknowledged write as completed. TLS verifies the original hostname
through SSH; custom ACL usernames authenticate explicitly even with an empty
password, with no default-user fallback. The browser uses a separate bounded
connection and 15-second deadline; health uses PING only.

Redis 8.2 disposable fixtures cover exact integers, NULL/empty/binary/Unicode,
preflight/approval/readonly policy, scoped list paging, expiry/KEEPTTL, large
strings, authenticated SSH, cleanup and full viewer budgets. Synthetic transport
tests cover cancelled-read socket closure, lost-write outcomes without retries,
oversized response headers, TLS-over-SSH hostname rejection and ACL identity.
The browser workflow covers real connect/scan/typed edit/review/commit/readback
and byte-safe forms. Blocking, scripting, transaction, administrative/session
commands and unbounded collection commands are unavailable. Cluster, Sentinel,
streams and Pub/Sub remain unsupported. Production accounts and broader versions
are unqualified. Final CI, native/package and release evidence is recorded on
the canonical project Site after completion.

Final local baseline validation passes 47 Rust tests and 16 frontend tests,
production frontend build, formatting, native compilation and strict Clippy.
All nine browser workflows pass across the four engine families. Six disposable
Redis integration tests pass alongside the earlier twenty PostgreSQL/MySQL/SSH
tests. These checks use synthetic fixtures and never read production tables.


## Continuation batch E: local metadata startup protection

Startup previously reset `user_version` to 1 regardless of the existing version,
and initialized tables/default groups through separate writes. Regression tests
reproduced acceptance of a newer schema and an incompatible unversioned store.
The fix checks version compatibility before persistent pragmas and again under
an immediate transaction, validates required column types/nullability/unique
keys, and commits initialization/version/default groups together. Newer or
incompatible stores fail without an automatic reset. Failed initialization
rolls back new schema while preserving existing rows. Missing default groups
are restored with insert-if-absent semantics; user-renamed groups stay intact.

Six new regressions cover newer-file preservation, invalid legacy rollback,
missing versioned tables, missing unique keys, partial legacy/default-group
recovery and concurrent application starts. Combined baseline validation passes
53 Rust and 16 frontend tests, build, formatting, native compilation and strict
Clippy. Native, integration, CI and publication evidence is recorded on the Site
after completion. This ships a bounded startup/downgrade guard; it does not
qualify a restore UI, arbitrary historical migrations, process-crash recovery,
signed updates or platform distribution. Those P7 gates remain open.

## Session and recovery hardening

Thirteen synthetic Snowflake lifecycle regressions cover serialized refresh,
refresh-token rotation/retention, short token lifetimes, expiry/revocation,
transient token-service failures, cancellation before SQL and during refresh,
submission and partition retrieval, and disconnect during HTTP/auth waits.
Eight failures were reproduced before the fixes. Closed or expired sessions
require reconnecting; statements are never replayed automatically. Disconnect
aborts local HTTP work; an already submitted remote read can remain until its
server timeout when no cancellable handle was received.

The keyboard browser regression reproduced search focus being stolen by the
modal timer. It now verifies focus preservation, Tab wrapping, Escape restoration
and Enter execution without reactivating the trigger. Focusable links and
nonzero tab indexes participate in the dialog trap; empty dialogs retain focus.
This covers the tested keyboard path, not a complete assistive-technology audit.

A subprocess recovery regression kills a disposable writer with committed WAL
records and an uncommitted transaction, reopens through Persistence, verifies
committed settings/history/audit/groups, checks pending writes rolled back and
runs SQLite quick_check. It also restores a closed/checkpointed store into a
fresh root. This qualifies the local metadata store path, not a backup/restore
UI, arbitrary historical upgrades or recovery of external database effects.

Local baseline: 67 Rust and 16 frontend tests, frontend production build,
formatting, native compilation and strict all-target/all-feature Clippy passed.
The baseline skips the child-process helper, disposable database tests and the
native credential-store check. Live production qualification is unchanged;
see the Site for release-specific integration, browser and native evidence.

## Sustained native fixture and helper-memory sampling

The hidden native fixture accepts 10–1000 cleanup cycles and 1–60 second idle
intervals, with matching bounded runner and Rust watchdog deadlines. Each cycle
closes tabs with Arrow requests in flight and verifies Rust result/session/run
counters and frontend retained bytes return to zero.

Optional macOS sampling launches a separately signed hidden fixture bundle through
LaunchServices. Helpers are attributed only when process responsibility points
to the fixture itself. Shared or unavailable responsibility, missing WebKit
helpers and unreadable processes remain explicit evidence gaps. RSS sums can
double-count shared pages; physical footprint is recorded separately. Python
regressions cover attribution isolation and incomplete samples; Rust tests cover
configuration bounds. CI runs the Python regressions alongside identity checks.

This ships validation tooling, excluded from the normal application build.
Preview.11 remains the app download. Representative total-idle memory, cold
visible startup and display FPS remain unqualified. Current validation and merge
evidence is recorded on the private project hub.


## Scoped SQL completion repair (local, unmerged)

On 2026-10-28, task branch `fix/scoped-sql-completion` implements the bounded
SQL-editor repair described in the [DataGrip feature matrix](datagrip-feature-matrix.md)
and [implementation roadmap](implementation-roadmap.md). The audit baseline is
`bbba421`; audit/roadmap commit `16a49e3` precedes the application change. This is
local implementation evidence, not a release or installation. Preview.11 remains
the existing app download and the private hub remains canonical shipping status.

- CodeMirror token scopes isolate statements, nested queries and set-operation
  arms, ignore comments/literals and suppress unsupported CTE/derived outputs.
  Structured identifiers preserve quoting, case, Unicode, embedded dots and
  escaped delimiters; ambiguous short names require qualification.
- Per-editor completion lifecycle retains at most eight metadata entries/reads,
  sends at most 100 suggestions and rejects stale document/caret/context/index
  responses. Cancelled waiters release old documents even if metadata is slow.
  Catalog indexing stays stable while typing; successful table metadata survives
  another table's denial. Existing Rust metadata/execution/Safe Mode paths remain.
- Passed `pnpm validate`: 59 frontend tests and 83 Rust tests, formatting, types,
  frontend build, Cargo check and strict Clippy. The full browser suite passed
  20 workflows; the final completion/Snowflake rerun passed six. Actual accepted
  SELECTs return expected values on PostgreSQL 18, MySQL 8.4 and MariaDB 11.4;
  Snowflake is synthetic, with no live account qualification.
- Three selected ignored Rust fixtures pass: repeated PostgreSQL resource
  cleanup, MySQL/MariaDB metadata/paging and TLS-through-SSH hostname checks.
  The full ignored suite was not executed because some fixtures issue DROP SQL;
  real OS credential-store checks also remain unrun. No production queries.
- Final-source `pnpm desktop:build`, `pnpm desktop:bundle:mac`,
  `pnpm desktop:smoke:build` and `pnpm test:native` pass on macOS 27.2 arm64.
  The macOS app bundle passes strict ad-hoc signature verification. Three hidden
  native runs pass (30 cleanup cycles total), including the real completion
  worker's escaped-name acceptance plus existing Arrow/editor/grid/close checks.
  Builds were made from the task working tree before its implementation commit
  (`sourceDirty=true`); logs and binary provenance remain in `.cache/`. No
  installed app was changed, and Developer ID/notarization remain open.

The measured 100,000-name warm index p95 is 0.10 ms. Warm explicit visible popup
p95 is 55–61 ms, above the blueprint's 50 ms target; CodeMirror 6.20.4 has a fixed
50 ms explicit-start delay. These are headless Chromium observations, not native
WebView, cold metadata, startup, total-memory or FPS qualification. The matrix
records samples and limitations. Live database/schema/search-path/role revisions,
full CTE/derived/correlated inference and FK JOIN generation remain separate work.
No dependency replacement, new state manager, privileged IPC command or backend
authorization change is introduced.


## PostgreSQL live completion context (shipped in preview.13)

The `feat/live-completion-context` branch extends preview.12's merged `06146bc`
with effective PostgreSQL completion context. Rust reads database, current role
and ordered schemas on the editor connection and execution lane; column metadata
requires a matching opaque snapshot revision and visible table/column grants.
Every app execution invalidates the old snapshot. The editor clears obsolete
metadata while refreshing and offers explicit unavailable/error/refresh states.
Temporary tables and transaction-local search paths use their actual session.

See [the contract and bounds](live-completion-context.md) and
[architecture decision](architecture-decisions.md). This is PostgreSQL first;
other engines retain configured completion behavior. External DDL/grant changes
need refresh or a subsequent app execution. SQL console/session binding, the
remaining live adapters and FK JOIN generation remain open.

Current-source verification passes: `pnpm validate` includes 62 frontend and
83 standard Rust tests, formatting, types, production frontend build, Cargo check
and strict Clippy. Three selected PostgreSQL fixture tests pass, including search
path/role/rollback, failed-transaction recovery, temporary/system relations,
read-only completion metadata, column grants, inaccessible-name shadows, stable
concurrent revisions, cancellation, and legacy grid keys/browsing. The full
browser suite passes all 21 workflows, including accepted-and-executed completion,
role/rollback changes and existing supported-engine editing/recovery/Safe Mode.
New native commands are registered in the command manifest, capability allowlist
and generated DTOs.

Final-source `pnpm desktop:build`, `pnpm desktop:bundle:mac`,
`pnpm desktop:smoke:build` and `pnpm test:native` pass on macOS arm64. The app
bundle passes strict ad-hoc signature verification. Three hidden native runs
pass 30 cleanup cycles, including actual IPC calls reaching Rust's disconnected-
session guards, completion-worker acceptance, Arrow/editor/grid/close checks.
Native fixture binary SHA-256 is
`86491f9e300eb6723cf1e9b0a5358b665f1aaeb6f8a2fba0f3a7347aa7dd3116`.

Reproduced and repaired during this batch: initial overlapping metadata reads
reported busy; bounded metadata serialization resolved this. Unchanged concurrent
readers invalidated each other's tokens; they now share a revision. Qualifying
multi-array unnest broke grid foreign-key metadata; qualified ROWS FROM plus a
regression preserves it. Missing implicit system relations were reproduced; the
adapter now supplies server-resolved names and declines an accessible later table
when an inaccessible earlier one shadows it. The cancellation fixture now waits
for actual query activity before cancelling.

Earlier verification interruptions are separate: Vite root/module requests
stalled during concurrent build/contract regeneration, while harness health stayed
HTTP 200. Final build and browser checks ran sequentially. On resumption, stopped
fixture databases caused connection failures; restarting the exact task project
and verifying health preceded the passing full rerun. Exact orphan test servers
were stopped. The task fixture project and volume were removed after verification;
unrelated user processes and the installed app were preserved.

Source builds use the pre-commit working tree (sourceDirty=true), with
logs/provenance under `.cache/live-context-*-final.log` and `.cache/native-smoke/`.
No production queries or destructive runtime SQL fixture commands were executed.
The standard Rust runner reports 32 ignored tests; three selected fixtures were
executed separately. The entire ignored suite, real credential-store behavior,
external accounts/platforms/signing and representative performance qualification
were not run. Warm browser popup p95 remains above the 50 ms target (53.8 / 53.5 /
53.3 ms for PostgreSQL/MySQL/MariaDB); warm 100,000-name lookup p95 is 0.10 ms.
This workflow subsequently shipped as preview.13 through PR #21 (`e8f1cec`).
All six PR checks and all five main validation jobs passed. Clean-commit native
verification repeated three runs/30 cleanup cycles (sourceDirty=false).
Unauthenticated public assets, manifest/checksums and strict ad-hoc signature
verified. The installed executable matches release SHA-256
`cfb5db5ef905148148c5dabddb081a9c24385771a9a2bc93a8c091a6ae7df6d1` and
launched from Applications with a visible window. The prior preview.12 bundle
is retained locally; the owner-private hub records the shipping receipt.


## MySQL/MariaDB live completion context (local implementation)

`feat/mysql-live-completion-context` starts from merged preview.13 `e8f1cec`.
The adapter now reads effective database/account/roles and server table-name case
rules on the editor connection. The same typed snapshot/revision and worker path
are reused. SHOW FULL COLUMNS supplies SELECT-granted columns and actual temporary
shadows of known tables. Arbitrary SQL results and completion metadata remain
read-only; legacy grid metadata and backend authorization are preserved.

See [the bounded contract](mysql-live-completion-context.md). Initial Unsupported
was reproduced, then cached prepared metadata retained the original DATABASE()
after USE while the editor's text query observed the new database. Text-protocol
live metadata fixes that regression; the existing parameterized grid path shares
the bounded decoder and retains its established ownership.

`pnpm validate` passes 63 frontend and 83 standard Rust tests, formatting, types,
production frontend build, Cargo check and strict Clippy. Three selected MySQL
fixture tests pass on both MySQL 8.4/MariaDB 11.4: new context/grants/transitions,
legacy keys/paging/read-only metadata and cancellation with busy guards. All 23
browser workflows pass, including both new accepted-and-executed USE/role/grant
flows and existing PostgreSQL, Redis, synthetic Snowflake, editing and recovery.

The standard runner reports 33 ignored tests. The full ignored suite was not run
locally because some fixtures issue destructive SQL; no production connections.
The fixture qualifies case mode 0. Modes 1/2 have frontend coverage and still need
real server/platform qualification; temporary-only catalog discovery and external
DDL/grant subscriptions remain open. Preview.13 remains shipped and installed;
this MySQL/MariaDB batch is local and unreleased.

Final PostgreSQL context regressions pass all three selected tests on this
revision, including concurrent snapshots, role/search path, cancellation and
legacy grid metadata. In total six selected Rust fixture tests passed. Final
`pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature),
`pnpm desktop:smoke:build` and `pnpm test:native` pass on macOS arm64; three hidden
native runs pass 30 cleanup cycles. These are pre-commit working-tree builds
(sourceDirty=true), native fixture SHA-256
`3d3cc79c469fa56e16bf47a387189db6bf7d07446471c8df19984ec9eb8f88f3`.
The task-owned fixture project/volume was removed and user app/processes preserved.
Logs and provenance remain under `.cache/mysql-context-*.log` and native-smoke.
Warm browser popup p95 is 53.6 / 53.4 / 53.3 ms for PostgreSQL/MySQL/MariaDB;
the 50 ms target remains missed. These checks do not qualify native performance,
other packaged platforms or case modes 1/2.

## MySQL/MariaDB live completion release and case-mode follow-up

The preceding local slice shipped in
[preview.14](https://github.com/quiinthieu/joinery/releases/tag/v0.1.0-preview.14).
PR #22 passed six checks and squash-merged as `dc220b0`, with the same complete
tree as tested `7697c6e`. All five main jobs passed in
[run 40817452645](https://github.com/quiinthieu/joinery/actions/runs/40817452645).
The clean-source macOS bundle and native smoke passed three runs/30 cleanup
cycles. Published and unauthenticated public assets match local digests; the
extracted app passes strict ad-hoc signature verification. Executable SHA-256:
`0bf283f622d6fac400f63043262e407c27d134fcc113ec8036c358f076e7ed55`.
The prior installed preview.13 remains running with a production Snowflake
connection; installation of preview.14 awaits safe closure. Release receipts
are under `.cache/ship-mysql-context/`.

The next task branch adds isolated case-mode-1 MySQL 8.4 and MariaDB 11.4
fixtures. `mysql_live_completion_uses_server_case_policy` passes on both:
effective mode, role-visible tables, unqualified resolution, canonical columns,
old-revision rejection and uppercase SQL. Mode 2 requires a suitable
case-insensitive filesystem and remains unqualified. Saved SQL profiles
currently require a nonempty database, so the no-default-database path is
not an exposed app workflow. The mode-1 fixture containers and network were
removed after testing.

Follow-up validation passes 63 frontend and 83 standard Rust tests, formatting,
types, production build, Cargo check and strict Clippy. The new selected ignored
fixture test passed separately on both engines. No runtime application code
changes in this follow-up; browser/native checks were not repeated.
