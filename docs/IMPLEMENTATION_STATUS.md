# Implementation and release status

Updated 2026-11-02. R1–R4 are merged but unreleased. The local R6 branch now
integrates R5 FK JOIN completion, structured Unicode diagnostics, typed
query parameters, inline error/statement navigation and SQL definitions/usages/
quick documentation, selection formatting/settings and snippets. Format saves
are serialized with rollback to the last successful settings. Global and
per-engine styles persist with validated recovery. Named CTE/derived-table
outputs now resolve to local declarations and proven source metadata. Wildcards,
expressions and unsupported shapes remain withheld. Console object/column jumps
now retain the originating session and observed catalog revision in read-only
Structure/DDL tabs; linked objects retain that origin and explicit DDL append
preserves the source SQL/session without execution. Late metadata cannot adopt
the primary session. The prior combined source passed 66 ignored local fixtures
and a normal ad-hoc macOS bundle build. R5 and R6 remain partial, unmerged,
unpublished and not installed. Preview.15 is the last verified release;
its current installed revision was not requalified. The canonical owner-private
Site records exact tested, merged, published and installed states. Full
multi-engine v1.0 qualification remains pending.

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
- File inputs, SQL files, library and exports: bounded reads of regular CA/key files, capability-granted `.sql` files (**4 MiB** UTF-8, max 64 active `GrantKind::SqlFile` grants, atomic sibling `.tmp` + `sync_all` + `rename`), capability-granted CSV/TSV import files (**16 MiB**, **5,000 rows**, 256 columns), capability-granted `.partial`-staged streaming exports (**1,000,000 rows** or **512 MiB** per job, at most two concurrent jobs), SQLite saved queries (**256 entries**, **256 KiB** each, **4 MiB** total), SQLite bookmarks (**256 entries**), and in-memory recent locations (**50 entries**); no arbitrary frontend filesystem path IPC.

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
Preview.14 was installed after the prior app exited. The installed executable
matches the published digest; AppKit verified launch from Applications and one
visible window. The same PID remained running during follow-up; its window was
later offscreen/hidden and user state was preserved. The prior bundle is retained
as a backup. Release receipts are under `.cache/ship-mysql-context/`.

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

The mode-1 follow-up shipped in source through PR #23 as `dec8f2b`, with
the same full tree as tested `b67a0fd`. All six PR checks and all five main jobs
passed ([main run 40821915951](https://github.com/quiinthieu/joinery/actions/runs/40821915951)).
Integration now starts and cleans up the folded-case fixtures and runs their
ignored test. This changes qualification infrastructure; preview.14 remains
the binary download.

## Snowflake completion request context (shipped in preview.15)

The [bounded contract](snowflake-completion-context.md) extends the existing
Rust snapshot, revision checks and CodeMirror worker. Fixed, bounded metadata
requests verify effective identity/search path and quoted-name policy under the
same database/schema/role/warehouse fields as execution. Known first-schema
tables bind unqualified; later-schema fallback is withheld because catalog
visibility cannot rule out inaccessible earlier shadows. Qualified lookup
remains available in the configured database. Unsupported quoted-name policy
produces a visible error with explicit refresh recovery.

`pnpm validate` passes 63 frontend and 86 standard Rust tests, formatting,
types, build, Cargo check and strict Clippy. Three new synthetic Rust HTTP
tests cover identity, request fields, path scope, malformed responses, quoted
policy and byte limits. The initial browser run passed 22/23; Redis form setup
was interrupted by page reloads observed in its trace while documentation
edits overlapped the live Vite run. With files held fixed, the complete rerun
passed all 23 workflows, including Redis and Snowflake context failure/refresh
and lazy revision-bound column completion. No application fix was required for
that reload. Logs: `.cache/sf-context-validate.log`, `-rust.log`,
`-browser.log` and `-browser-final.log`.

The initial local builds preceded the release described below. Live Snowflake accounts, actual inherited
defaults/grants, broader search paths/quoted policies and representative metadata
latency remain unqualified. Thirty-four ignored tests are skipped by ordinary
validation; no full ignored or destructive runtime SQL suite was run locally in
this batch. All database effects used disposable fixtures. No production query,
automatic reconnect/replay or Safe Mode change was introduced.

Final `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature),
`pnpm desktop:smoke:build` and `pnpm test:native --runs 3 --cycles 10` all pass.
Three hidden runs complete 30 cleanup cycles. Native fixture SHA-256:
`446ba9cbd3ae8065dd0b0ca210fae0810f86a19e2bddf8742738251d75fc1c52`.
This is a pre-commit working-tree build based on `dec8f2b` (sourceDirty=true),
not release provenance. Native JSON is preserved in
`.cache/sf-context-native-runs.json`; build/bundle/native logs use the
`.cache/sf-context-` prefix. Task-owned database containers, network and volume
were removed. The installed preview.14 was preserved during development.

PR #24 subsequently passed all six checks and merged as 799e600, with the same
complete tree as tested f8488f7. All five main jobs passed
([run 40825228002](https://github.com/quiinthieu/joinery/actions/runs/40825228002)).
[Preview.15](https://github.com/quiinthieu/joinery/releases/tag/v0.1.0-preview.15)
was built from clean committed source, including three native runs/30 cleanup
cycles (sourceDirty=false). Public asset downloads, checksums, extracted app
and strict ad-hoc signature verify. It was installed after the prior app exited;
the prior bundle is retained. Installed executable SHA-256:
`04b5ea7a2639fa1975e9f74a53beee798bc5d580a1177e8feb3b4c2244a193bd`.
AppKit verified Applications path, finished launch and one visible window.
Receipt: .cache/ship-sf-context/receipt.json. Live account, signing/platform and
performance gates remain open.

## Isolated query-tab sessions (locally verified; unreleased)

M06-Q reuses the existing Rust-owned connect/disconnect operations, engine
adapters, Safe Mode and four-session profile cap. The existing sign-in dialog
hands a new SessionInfo to its query tab without replacing the explorer's primary
session. Execution and completion resolve that exact tab binding. Cancellation
retains a disconnected binding with explicit reconnect; stale events cannot
retire a new session, and changed-session prepared plans are never submitted.
Close/switch/profile/native-exit confirmations address possible open
transactions. Draft snapshots exclude live bindings. No new IPC or credential
cloning API was introduced; see [the bounded contract](isolated-query-sessions.md).

Passed: final pnpm validate, 71 frontend tests (eight new ownership/cleanup/
credential-handoff/draft cases), 86 standard Rust tests, frontend build, formatting,
types, Cargo check and strict Clippy. Three actual browser/engine flows pass on
PostgreSQL 18, MySQL 8.4 and MariaDB 11.4, including concurrent consoles,
private/shared state, owned cancellation, explicit reconnect, close cancellation
and backend resource-count cleanup. A delayed PostgreSQL prepared response across
rebinding produced no execution submission.

Baseline: after correcting test result-set selection, the new acceptance flow
failed at the missing isolated-session control before implementation.
Full browser attempts passed 25/26 each. The first failure was a blank page
with ERR_NETWORK_CHANGED module downloads. The second was the existing grid
keyboard test after repeated selection activated its read-only value viewer;
the test now presses Escape before checking grid navigation. The next full retry
and PostgreSQL-only keyboard retry were interrupted when host load rose above
100 and then 126 with low free memory. They do not establish product failures
or a clean final full-suite pass.

Initial blocked/not-reached checks: desktop:build, desktop:bundle:mac,
desktop:smoke:build and test:native for this slice. The pipeline stopped at the browser gate. Earlier
preview.15 native evidence applies only to its released tree. Native and
clean full-regression qualification were open at that point; see the successful
continuation below. This slice is not released.
No production queries, destructive runtime SQL, locked-store interaction or
new SSH/production account qualification was performed. Ordinary validation
skips 34 ignored tests; the full ignored suite was not rerun locally.

Logs: .cache/isolated-validate-final.log, isolated-workflows.log,
isolated-browser-final.log, isolated-browser-verified.log,
isolated-browser-complete.log, isolated-keyboard-final.log and baseline logs.
The screenshot review preserved Joinery's existing layout and visible ownership
label; no proprietary assets were used. Only task-owned test processes and
fixture containers were stopped; user workloads and installed preview.15 were
preserved.

After host resources recovered, the final fixed-file browser suite passed all
26 workflows, including all three new session flows. The first recovered run
was 25/26 because the unconditional Escape helper cleared a normal grid
selection. The corrected helper dismisses only an open Glide overlay and
verifies the selected column before ArrowRight; the final full suite passes.
No application change was needed for these helper/network failures.

Passed continuation: desktop:build, desktop:bundle:mac with strict ad-hoc
signature, desktop:smoke:build and three native runs/30 cleanup cycles.
Native elapsed times: 3662/3237/3423 ms. Build sourceRevision=f556518,
sourceDirty=true (final test-helper edit before its commit); binary SHA-256
`a2d918af11f6910185f863ac9732c56de864b62bde02e5abbff47b562b5f953b`.
This is local candidate evidence, not clean-source release provenance.
Logs: .cache/isolated-browser-qualified.log, isolated-format-recovered.log,
isolated-desktop.log, isolated-bundle.log, isolated-native-build.log and
isolated-native-runs.log/json. The retry's containers, network and volume were
removed. The prior host-resource hold is resolved for these local checks.
External auth/account, packaged platform, signing and representative performance
gates remain open. This slice subsequently merged on `main` via PR #26 (`877904b`,
including `35b84c5` long-sidebar scroll containment, all six PR checks passing
with 79 frontend tests and 31 browser workflows); preview.15 remains installed
and this phase is unreleased.

## Table data view: sort, filter and FK navigation (merged in PR #25; unreleased)

R1 batch 1 of the approved DataGrip-replacement program (see
[implementation-roadmap.md](implementation-roadmap.md)) merged on `main` via
PR #25 (`6445b0e`, all six PR checks and five `main` validation jobs passing). It
remains unreleased (`v0.1.0-preview.15` is installed).

PostgreSQL and MySQL/MariaDB table tabs sort on the server by header click
(Shift adds a key) and filter with typed, bound predicates. Rust rebuilds every
page query from the current schema, adds primary-key tie-breakers and a fixed
NULL order, and continues with a NULL-safe keyset seek. Without user sort keys
the previous row-value fast path is kept. Snowflake and Redis advertise
`tableFiltering: false` and reject non-empty queries. Foreign keys are grouped
constraints with ordinal-paired columns; ⇧⌘B, the palette or the inspector opens
the referenced row in a new tab. Sort/filter controls lock while edits are
pending, so a reload cannot silently discard them.

New tests found two pre-existing defects, each fixed in its own commit with a
regression test. Read-only MySQL profiles could not browse editable tables
because masked editability was treated as schema drift. The frozen primary-key
column stayed blank for pages loaded after scrolling because Glide's visible
region excludes frozen columns.

`pnpm validate` passes 71 frontend and 97 standard Rust tests, formatting,
contracts, types, build, Cargo check and strict Clippy. The ignored suite
(`postgres`, `ssh`, `mysql`, `redis`) passes 35 tests on a task-owned disposable
Compose project. The case-mode-1 MariaDB fixture could not bind port 53407,
which the agent host process holds; `mysql_live_completion_uses_server_case_policy`
was skipped. The final full browser run passes all 27 workflows; an earlier
run's MariaDB failure reproduced a known reload while documentation was edited
under the live Vite server. Logs use the `.cache/table-data-` prefix.

`pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature),
`pnpm desktop:smoke:build` and `pnpm test:native --runs 3 --cycles 10` pass:
three hidden runs, 30 cleanup cycles. Native fixture SHA-256:
`f2d9bafdd1e551b354f520da3b07144de6c40edfb84b5fa3d08379cf504b5d9b`, built from
`76f04be` with documentation uncommitted (sourceDirty=true); not release
provenance. No production connection, Safe Mode change, replay or reconnect was
introduced.

## Table data editing, layouts, raw clauses, inbound FKs and viewers (local implementation)

R1 batch 2 of the approved DataGrip-replacement program is implemented on
branch `feat/table-data-editing` (`2a59fae..b62e475`), based on fetched
`origin/main` `877904b` (PR #26). It completes Milestone R1 (Data view parity)
and remains unreleased.

- **Raw table `WHERE` / `ORDER BY` classification (`2a59fae`)**: PostgreSQL and
  MySQL/MariaDB table tabs accept optional raw `WHERE` and `ORDER BY` clauses
  (capped at 16 KiB each, wrapped in newline-terminated comment guards). Rust
  synthesizes the full statement and classifies it through `safety::prepare`:
  multi-statement inputs are rejected, non-read or unparseable expressions are
  blocked in `ReadOnly` and gated by a one-use `ReviewDialog` approval token
  (plus production passphrase when configured) in writable modes, and raw
  `ORDER BY` cleanly disables keyset continuation rather than risking unstable
  seeks.
- **Inbound referencing-row navigation (`43da547`)**: Permission-filtered
  `referencing_foreign_keys` metadata groups composite inbound constraints in
  key-ordinal order on PostgreSQL and MySQL/MariaDB. `⇧⌘G`, the palette and the
  inspector open referencing rows in a new filtered tab, with a constraint
  chooser when multiple child constraints target the row and NULL-key refusal.
- **Column layout management (`0ca0146`)**: `ColumnConfigPopover` and
  `table-layout.ts` support hiding non-PK columns, reordering, resizing and
  resetting per `connectionId + schema + table`, persisted in bounded
  `SavedWorkspace.table_layouts` (at most 128 tables and 256 columns per table,
  reconciled against live schema refreshes and purged on profile deletion).
- **Atomic multi-cell TSV paste (`a026c44`)**: `paste.ts` and
  `DirtyStore.setBatch` parse RFC-4180-style TSV clipboards and pre-validate
  every target cell across visible (including reordered/hidden) columns against
  editability, integer/decimal precision, NULL tokens and the 8 MiB / 500-row
  dirty budgets before mutating the sparse overlay. Any invalid cell rejects the
  entire paste with zero partial mutations.
- **Reviewed multi-row insert and delete (`8a55a4c`)**: `RowChange` becomes an
  explicit `insert` / `update` / `delete` tagged union. Draft rows display
  `<default>` / `<required>` states, deletes lock and verify all non-binary
  original columns via `FOR UPDATE`, and mixed insert/update/delete batches
  execute atomically in one reviewed `BEGIN … COMMIT` transaction across
  PostgreSQL, MySQL 8.4 and MariaDB 11.4. Browser testing caught a React 18
  discrete-click vs passive-`useEffect` `selectionRef` race when focusing newly
  inserted draft rows; moving `focusRowRequest` to `useLayoutEffect` with eager
  `selectionRef` synchronization resolved it.
- **Binary, PostgreSQL array and lossless JSON viewers (`e3b13b4`)**:
  `ValueDialog` and `value-format.ts` add WAI-ARIA tabbed viewers for binary
  columns (16-byte-per-row Hex/ASCII dump over a bounded 16 KiB preview with
  exact byte length, decoding PG `\x`/escaped `bytea` and MySQL `0x`),
  PostgreSQL arrays (1-based `[1]`, `[1][1]` element table supporting custom
  `[lower:upper]=` bounds and distinguishing SQL `NULL` from `"NULL"` and `""`,
  bounded to 2,000 preview elements) and JSON/JSONB (token-level lossless
  pretty-printer + `Format JSON` action preserving 64-bit integers and numeric
  object key order).

Verification on macOS Apple Silicon (Node 24, Rust 1.99.1):

- `pnpm validate` passes **99 frontend tests** (19 files) and **102 standard
  Rust tests**, formatting, contracts, types, production build, Cargo check and
  strict all-target/all-feature Clippy. Script unit tests
  (`git-identity.test.mjs` 6/6 and `test_runtime_memory.py` 4/4) pass.
- The disposable Rust integration suite (`postgres` 17, `mysql` 16 across MySQL
  8.4 and MariaDB 11.4, `redis` 6, `ssh` 2) passes **41 of 41 executed tests**
  on task-owned Compose project `joinery-batch2`. As in R1 batch 1, port 53407
  is held by the host agent process, so case-mode-1
  `mysql_live_completion_uses_server_case_policy` was skipped locally.
- With repository files held fixed, the complete Playwright suite passes **all
  37 browser workflows** in a single run (`1.7m`), including all nine
  `table-data.spec.ts` workflows. The `joinery-batch2` containers, network and
  volume were removed immediately afterward (`down -v`).
- **Tauri v2 capability ACL enrollment (`951fca6`, `b62e475`)**: Enrolled
  `referencing_foreign_keys`, `prepare_browse_table` and `prepare_table_changes`
  in `src-tauri/build.rs` and `src-tauri/capabilities/main.json`, and added
  typed real-WebView IPC disconnected-session guard assertions for all three
  commands in `src/native-smoke.ts`.
- Clean-commit (`b62e475`, `sourceDirty=false`) `pnpm desktop:build`,
  `pnpm desktop:bundle:mac` (strict ad-hoc signature verification on
  `Joinery.app`, 19.91 MiB), `pnpm desktop:smoke:build` and
  `pnpm test:native --runs 3 --cycles 10` all pass: three hidden processes
  (`3,725 / 2,927 / 2,925 ms`), 30 cleanup cycles, main-process peak RSS
  `105.12–105.83 MiB`, native smoke binary SHA-256
  `dcc7e49561e0022e29a59927d6d3c3f5f724b2504544f5ada3a2a0719b8a2b55` (bundle
  binary SHA-256
  `0b73952da4025367e65746849c019df3c22283aa03b591932324590a06950691`).
- No production database connections, automatic SQL replay or Safe Mode
  weakening were introduced. Live Snowflake/production account qualification,
  Windows/Linux packaged runtime, Developer ID signing/notarization and
  representative performance gates remain open.

## Data movement: copy-as, streaming export, reviewed CSV import and key-aware comparison (merged in PR #28; unreleased)

Branch `feat/data-movement` (`a3020d7..34c51db`, from `origin/main` `94f63ba`)
implements Milestone R2 (Data movement: `G11`, `G12`, `G13`) and merged on
`main` via PR #28 (`c8d8aa1`, all six PR checks passing). `v0.1.0-preview.15`
remains the published and installed release; PRs #25, #26, #27 and #28 are
merged on `main` and unreleased:

- **Selection Copy-as (`a3020d7`)**: `copy-format.ts` and `CopyAsMenu.tsx`
  format grid selections and retained pages as `CSV`, `TSV`, `JSON`, `NDJSON`,
  `SQL INSERT` and `Markdown` (bounded to **5,000 rows** and **2 MiB**),
  respecting hidden/reordered columns, distinguishing SQL `NULL` from empty
  strings, preserving exact 64-bit integer and decimal text without IEEE-754
  rounding, and quoting identifiers per dialect (`"` on PostgreSQL/Snowflake,
  `` ` `` on MySQL/MariaDB).
- **Capability-gated Rust streaming export (`046f049`)**: `file_capability.rs`
  (`FileDialogPort`, `RfdFileDialog`, `SandboxFileDialog`,
  `FileCapabilityStore`) issues opaque expiring capability tokens (`FileWriteGrant`
  one-use 15-minute TTL, `FileReadGrant` 30-minute TTL) so raw filesystem paths
  never cross IPC. `export.rs` and `ExportDialog.tsx` stream `LiveTable` pages
  (PostgreSQL, MySQL/MariaDB, honoring Safe-Mode raw-clause `BrowseApproval`)
  and `RetainedResult` snapshots (PostgreSQL, MySQL/MariaDB, Snowflake, with
  explicit truncation disclosures when retained results hit the 100,000-row /
  64 MiB cap) directly to `<dest>.partial` (`0600` on Unix) up to **1,000,000
  rows** / **512 MiB** without populating `ResultStore`. Completed exports
  atomically rename `<dest>.partial` to `<dest>`; cancelled or failed runs leave
  `<dest>.partial` with an explicit `INCOMPLETE` trailer so partial files are
  never mistaken for complete exports. Zero exported rows or file paths are
  persisted to SQLite.
- **CSV/TSV import validation, remapping and reviewed `ChangePlan` batches
  (`039c580`, `34c51db`)**: `import.rs` and `ImportCsvDialog.tsx` read regular
  files up to **16 MiB** / **5,000 rows** / **256 columns** (256 KiB per cell)
  through `FileReadGrant`, auto-detect or override delimiters/headers/NULL
  tokens, allow column remapping or skipping, and validate the entire file
  upfront against session `SafeMode` (`ReadOnly` rejected before plan creation),
  table editability, required non-default columns, generated/binary column
  exclusions, integer/float/boolean/JSON syntax and duplicate in-file primary
  keys. Validated rows materialize in **500-row / 8 MiB** `RowChange::Insert`
  batches through `Studio::prepare_changes_batch` and `ReviewDialog`, preserving
  production passphrase gates, one-use expiring approvals and full transaction
  rollback on server constraint conflicts with zero automatic retries. Browser
  E2E verification caught a transient unmount bug when `refreshTable` cleared
  `tab.schema` after commit; `34c51db` keeps `ImportCsvDialog` mounted with a
  cached `TableSchema` across refreshes so completion state and multi-batch
  progress survive cleanly.
- **Key-aware bounded data comparison (`defd48e`)**: `compare.rs` and
  `DataCompareDialog.tsx` compare two `LiveTable` or `RetainedResult` sources
  (including cross-engine PostgreSQL ↔ MySQL/MariaDB) in 500-row pages up to
  **25,000 rows** / **16 MiB** per side and **5,000** returned diff/anomaly
  entries without adding `ResultStore` descriptors or retained bytes. Comparison
  pairs explicit key columns, distinguishes SQL `NULL` from `""` and exact
  numeric text, and isolates duplicate-key and `NULL`-key rows into dedicated
  anomaly buckets rather than silently coalescing them.
- **Tauri v2 capability ACL enrollment**: Enrolled `pick_export_destination`,
  `start_export`, `cancel_export`, `pick_import_source`, `preview_csv_import`,
  `prepare_csv_import_batch` and `compare_data` in `src-tauri/build.rs` and
  `src-tauri/capabilities/main.json`, and verified all seven commands over real
  Tauri WebView IPC in `src/native-smoke.ts`.

Verification on macOS Apple Silicon (Node 24, Rust 1.99.1):

- `pnpm validate` passes **105 frontend tests** (20 files) and **115 standard
  Rust tests**, formatting, contracts, types, production build, Cargo check and
  strict all-target/all-feature Clippy. Script unit tests
  (`git-identity.test.mjs` 6/6 and `test_runtime_memory.py` 4/4) pass.
- The disposable Rust integration suite (`postgres` 20, `mysql` 19 across MySQL
  8.4 and MariaDB 11.4, `redis` 6, `ssh` 2) passes **47 of 47 executed tests**
  on task-owned Compose project `joinery-r2` (including streaming multi-page
  exports and `.partial` cancellation trailers, CSV import remapping/read-only/
  rollback, and live-table/snapshot/cross-engine data comparison). Port 53407
  remains held by the host agent process, so case-mode-1
  `mysql_live_completion_uses_server_case_policy` was skipped locally. The
  `joinery-r2` containers, network and volume were removed immediately afterward
  (`down -v`).
- With repository files held fixed, the complete Playwright suite passes **all
  41 browser workflows** in a single run (`1.7m`), including all four
  `data-movement.spec.ts` workflows.
- Clean-commit (`34c51db`, `sourceDirty=false`) `pnpm desktop:build`,
  `pnpm desktop:bundle:mac` (strict ad-hoc signature verification on
  `Joinery.app`, 20.41 MiB, bundle executable SHA-256
  `872ed28177824c7b336f0156122ad1bbecedfa2cfe76e597b7dd6d2452cf580b`),
  `pnpm desktop:smoke:build` and `pnpm test:native --runs 3 --cycles 10` all
  pass: three hidden processes (`4,050 / 2,988 / 2,962 ms`), 30 cleanup cycles,
  main-process peak RSS `100.56–105.36 MiB`, native smoke binary SHA-256
  `72cf58993b99c75aa98b9c857fde477ad326950d916ae59051b8b0926195fc21`
  (`bundleBinarySha256`
  `44727e2f20eaa5be59d97ab50524d4e8a06164924266ef7520f361f8cbeda714`).
- No production database connections, automatic SQL replay, raw-path IPC or Safe
  Mode weakening were introduced. Live Snowflake/production account
  qualification, Windows/Linux packaged runtime, Developer ID
  signing/notarization and representative performance gates remain open.

## Explorer, object inspection, DDL/dependencies and Search Everywhere (merged in PR #29; unreleased)

Branch `feat/explorer-object-search` (`7595db4..26f5136`, from `origin/main`
`c8d8aa1`) implements Milestone R3 (Explorer & object search: `N01`, `N02`,
`N04`, `N05`, `N06`, `N07`, `P01`) and merged on `main` via PR #29 (`2625933`,
all six PR checks passing). `v0.1.0-preview.15` remains the published and
installed release; PRs #25, #26, #27, #28 and #29 are merged on `main` and
unreleased:

- **Bounded object discovery, DDL and 1-hop dependency adapters (`7595db4`,
  `4058784`)**: `domain.rs` defines `DatabaseObjectKind` (`table`,
  `partitionedTable`, `view`, `materializedView`, `function`, `procedure`,
  `sequence`, `trigger`, `index`), `DatabaseObjectRef`, `DatabaseObjectSummary`,
  `ObjectDetails`, `ObjectDdlStatus` (`available` with `truncated` flag up to
  **256 KiB** UTF-8 via `cap_ddl`, or `unavailable` with explicit `reason`),
  `IndexInfo`, `ConstraintInfo`, `TriggerInfo`, `RoutineInfo`, `SequenceInfo`
  and `DependencyEdge` (capped at **200 edges per direction** with self-edge and
  duplicate removal via `normalize_dependency_edges`).
  - **PostgreSQL (`drivers/postgres/explorer.rs`)**: Discovers up to **10,000**
    schema-accessible functions, procedures (disambiguated by
    `pg_get_function_identity_arguments`), sequences, non-internal triggers and
    indexes alongside partitioned tables (`relkind = 'p'`). Synthesizes
    deterministically quoted `CREATE TABLE` / `CREATE SEQUENCE` DDL, extracts
    view/routine/index/trigger definitions via `pg_get_*def`, decodes `tgtype`
    bitmasks, inspects expression and partial `WHERE` index predicates, checks
    `pg_sequences.last_value` without failing when `USAGE`/`SELECT` is denied on
    a sequence, and resolves 1-hop `Uses` / `Used by` edges across `pg_depend`,
    `pg_rewrite`, `pg_constraint` and `pg_trigger` (terminating cleanly on
    mutually cyclic views).
  - **MySQL 8.4 & MariaDB 11.4 (`drivers/mysql/explorer.rs`)**: Excludes MariaDB
    `TABLE_TYPE = 'SEQUENCE'` relations from base-table `catalog` and surfaces
    them as `Sequence` objects (`SHOW CREATE SEQUENCE` + sequence table read),
    discovers `information_schema` routines, triggers and indexes (including
    prefix lengths and MySQL 8.4 functional key expressions), extracts bounded
    `SHOW CREATE` definitions with privilege-denial fallback to
    `ObjectDdlStatus::Unavailable`, and computes 1-hop `Uses` / `Used by` edges
    across `KEY_COLUMN_USAGE`, `VIEW_TABLE_USAGE`, `VIEW_ROUTINE_USAGE` and
    `TRIGGERS`. Disposable Compose MySQL/MariaDB services now pass
    `--log-bin-trust-function-creators=1` so the non-`SUPER` fixture account can
    exercise trigger and function DDL under MySQL 8.4 binary logging locally and
    in CI.
  - **Snowflake & Redis**: Snowflake advertises `objectExplorer: true` for
    read-only table/view `Structure` inspection with explicit
    `ObjectDdlStatus::Unavailable` (no unverified `GET_DDL` calls); Redis
    advertises `objectExplorer: false`.
- **Categorized sidebar explorer & WAI-ARIA keyboard tree (`53d9216`)**:
  `explorer-tree.ts` and `Sidebar.tsx` group objects per schema into `Tables`,
  `Views`, `Materialized views`, `Functions`, `Procedures`, `Sequences`,
  `Triggers` and `Indexes`, add `All` / `Tables` / `Views` / `Routines` /
  `Other` filter chips plus qualified/signature substring filtering, and
  implement full roving-`tabIndex` WAI-ARIA keyboard tree navigation
  (`ArrowRight`, `ArrowLeft`, `ArrowDown`, `ArrowUp`, `Home`, `End`, `Enter`).
  `studio.ts` fetches `database_objects` alongside `catalog` without failing
  sign-in if object discovery errors, and deduplicates `kind: "object"` tabs by
  `connectionId + schema + kind + name + identity`.
- **Object structure, read-only DDL and dependency inspector (`a60d3cf`)**:
  `ObjectInspectorView.tsx`, `UtilityViews.tsx` (`StructureView`) and
  `Workbench.tsx` add a `DDL` sub-view to table/view/materialized-view tabs,
  enrich `StructureView` with `Constraints` (clickable FK targets), `Indexes`
  (expressions, partial `WHERE` predicates, clickable index links) and
  `Triggers` (clickable trigger links), and render dedicated inspector tabs for
  routines, sequences, triggers and indexes with `Copy DDL`, `Open in SQL
  editor` (never auto-executed) and clickable `Uses` / `Used by` dependency
  chips.
- **Cross-connection Search Everywhere (`26f5136`)**: `search-everywhere.ts` and
  `SearchEverywhere.tsx` upgrade the command palette (`⌘K` opens `All`; `⇧⌘O`
  opens `Objects`) with roving-`tabIndex` `All` / `Objects` / `Actions` /
  `History` scope tabs and deterministic ranking (capped at **80** items) across
  commands, open non-singleton workspace tabs, all currently connected sessions'
  in-memory `catalog` + `objects` (never auto-connecting offline profiles, while
  displaying connected vs offline counts in `Objects` scope), and recent
  `HistoryEntry[]`.
- **Tauri v2 capability ACL enrollment**: Enrolled `database_objects` and
  `object_details` in `src-tauri/build.rs` and
  `src-tauri/capabilities/main.json`, and verified both commands over real Tauri
  WebView IPC disconnected-session guards in `src/native-smoke.ts`. Zero object
  metadata, DDL text or dependency edges are written to SQLite.

Verification on macOS Apple Silicon (Node 24, Rust 1.99.1):

- `pnpm validate` passes **116 frontend tests** (23 files) and **120 standard
  Rust tests**, formatting, contracts, types, production build, Cargo check and
  strict all-target/all-feature Clippy. Script unit tests
  (`git-identity.test.mjs` 6/6 and `test_runtime_memory.py` 4/4) pass.
- The complete disposable Rust integration suite (`postgres` 21, `mysql` 21
  across MySQL 8.4, MariaDB 11.4 and case-mode-1 folded servers on 53406/53407,
  `redis` 6, `ssh` 2) passes **all 50 of 50 tests (0 skipped)** on task-owned
  Compose project `joinery-r3`, which was torn down with `down -v` immediately
  after browser verification.
- With repository files held fixed, the complete Playwright suite passes **all
  43 browser workflows** in a single run (`1.8m`), including both new
  `explorer.spec.ts` workflows and existing `studio.spec.ts` palette focus-trap
  regressions.
- Clean-commit (`26f5136`, `sourceDirty=false`) `pnpm desktop:build`,
  `pnpm desktop:bundle:mac` (strict ad-hoc signature verification on
  `Joinery.app`, `20.66 MiB`, bundle executable SHA-256
  `04d4ae8683ea8d8812a64848ed24956932ed02dd94755344200f86f3e3a2b000`),
  `pnpm desktop:smoke:build` and `python3 scripts/native-smoke.py --runs 3
  --cycles 10` all pass: three hidden processes (`6,622 / 3,010 / 3,034 ms`), 30
  cleanup cycles, main-process peak RSS `106.89–107.41 MiB`, native smoke binary
  SHA-256 `cc044b44582fbcf81b695d0b4734a24ca926da7660d61422ecaab351ac74b83c`
  (`bundleBinarySha256`
  `2de7c55277d78b3595d0c498a4dfe71759a2627a5f8658f9b89492fbb6be3da3`).
- No production database connections, automatic SQL execution/replay or Safe
  Mode weakening were introduced. Live Snowflake/production account
  qualification, Windows/Linux packaged runtime, Developer ID
  signing/notarization and representative performance gates remain open.

## Consoles, named session pool, transaction controls, capability-gated SQL files and query library (merged, unreleased)

Branch `feat/consoles-sessions-files` (`78a5283..79878e9`, from `origin/main`
`2625933`) implements Milestone R4 (Consoles, sessions & files: `M05`, `M06`,
`X04`, `E14`, `P02`, `P03`). It merged via PR #30 (`a0ba1a6`) on
31 October. The following local verification is historical; R1–R4 are now
merged and unreleased. `v0.1.0-preview.15` remains the latest published release;
installation was not requalified in the 1 November assessment:

- **Per-session transaction controls and active schema/database switching
  (`78a5283`)**:
  - `domain.rs` defines `TransactionMode` (`autoCommit`, `manual`),
    `TransactionState` (`idle`, `inTransaction`, `failed`),
    `SessionContextKind` (`postgresSchema`, `mysqlDatabase`, `snowflakeSchema`,
    `none`), `SessionRuntimeState` and `QueryEvent::SessionState`.
  - **PostgreSQL (`drivers/postgres.rs`)**: Tracks authoritative wire-level
    `ReadyForQuery` status bytes (`I` → `Idle`, `T` → `InTransaction`, `E` →
    `Failed`), auto-prepends `BEGIN` before transactional statements in `Manual`
    mode when `Idle` (skipping `VACUUM`, `CREATE INDEX CONCURRENTLY`, `SET
    SESSION` and explicit transaction control), executes `COMMIT` (`ReadOnly`
    blocked; audit-logged) and `ROLLBACK` (always allowed, including `25P02`
    failed-transaction recovery; audit-logged) on the session execution lane,
    lists visible non-internal schemas, and switches active schema via
    `select set_config('search_path', $1, false)` (preserving explicit
    `pg_catalog` precedence when present).
  - **MySQL 8.4 & MariaDB 11.4 (`drivers/mysql.rs`)**: Toggles `SET SESSION
    autocommit = 0/1`, observes `SERVER_STATUS_IN_TRANS` from wire packets,
    clears any phantom transaction opened by `SET SESSION autocommit = 0` itself
    (`close_phantom_transaction`), executes explicit `COMMIT` / `ROLLBACK` on
    the editor connection while keeping table-grid reads and reviewed
    `ChangePlan` commits isolated on auxiliary connections, and switches active
    database via escaped `USE`. Both PostgreSQL and MySQL refuse `Manual` →
    `Auto-commit` transitions while `transactionState != Idle` with
    `ErrorCode::Conflict`.
  - **Snowflake (`drivers/snowflake.rs`)**: Lists schemas in the configured
    database and verifies stateless request-scoped schema overrides without
    claiming interactive transaction support (`transactions: false`).
- **Capability-gated `.sql` file Open, Save and Save As (`25f8dd8`,
  `5af049e`)**: `file_capability.rs` adds reusable `GrantKind::SqlFile` grants
  (capped at **64** active tokens, **4 MiB** valid UTF-8 regular files,
  rejecting symlinks and NUL bytes) and atomic sibling temp-file writes
  (`.joinery-sql-<uuid>.tmp`, `0600` on Unix for new files or preserved existing
  permissions, `sync_all`, then `rename`). Query tabs support `⌘O` (Open), `⌘S`
  (in-place Save reusing `fileToken`, while `⌘S` on table tabs preserves
  `reviewEdits`), and `⇧⌘S` (Save As, releasing any replaced token), render a
  dirty indicator and unsaved-close/quit confirmation guard, never expose raw
  filesystem paths over IPC, and are excluded from `SavedWorkspace.drafts`.
- **Saved queries library, object bookmarks and recent locations (`c4135c4`,
  `5af049e`)**: `persistence/library.rs` stores bounded JSON arrays under
  `SCHEMA_VERSION = 1` `settings` keys `'saved-queries'` (**256** entries,
  **256 KiB** SQL each, **4 MiB** total) and `'bookmarks'` (**256** entries,
  deduplicated by canonical target), validated during `recovery::validate` and
  cascading on `delete_profile` (purging production or `!storeQueryText` saved
  queries while detaching non-production opted-in saved queries, and purging
  profile bookmarks). `SaveQueryDialog`, `LibraryModal`, `Sidebar.tsx` and
  `SearchEverywhere` (`Saved` scope) integrate saved queries, `⌘D` bookmark
  toggling on tables/objects/saved queries, and an ephemeral **50-item**
  in-memory `Recent locations` ring buffer (`⇧⌘E`).
- **Named multi-console session pool & 12-command Tauri v2 ACL enrollment
  (`c4135c4`, `8e5cf3d`, `5af049e`)**: `studio.ts` and `Workbench.tsx` manage a
  named `sessionPool` (retaining Rust's four-session-per-connection cap) so
  multiple consoles can attach to the same secondary session or `Shared
  session`, rename/disconnect sessions, and disconnect a pooled session on close
  only when closing its last attached owner. All 12 new R4 commands
  (`session_contexts`, `switch_session_context`, `set_transaction_mode`,
  `commit_transaction`, `rollback_transaction`, `open_sql_file`,
  `save_sql_file`, `release_sql_file_grant`, `save_saved_query`,
  `delete_saved_query`, `save_bookmark`, `delete_bookmark`) are enrolled across
  `commands.rs`, `lib.rs`, `build.rs`, `capabilities/main.json` and
  `bin/test-harness.rs`, and verified over real Tauri WebView IPC in
  `src/native-smoke.ts`.

Verification on macOS Apple Silicon (Node 24, Rust 1.99.1):

- `pnpm validate` passes **122 frontend tests** (23 files) and **126 standard
  Rust tests**, formatting, contracts, types, production build, Cargo check and
  strict all-target/all-feature Clippy. Script unit tests
  (`git-identity.test.mjs` 6/6 and `test_runtime_memory.py` 4/4) pass.
- The disposable Rust integration suite (`postgres` 22, `mysql` 21 across MySQL
  8.4 and MariaDB 11.4, `redis` 6, `ssh` 2) passes **51 of 51 executed tests**
  on task-owned Compose project `joinery-r4` (including manual-commit auto-BEGIN,
  PostgreSQL `25P02` failed-transaction rollback, MySQL phantom-transaction
  cleanup, grid-commit isolation, and per-session schema/database switching).
  Port 53407 remains held by the host agent process, so case-mode-1
  `mysql_live_completion_uses_server_case_policy` was skipped locally. The
  `joinery-r4` containers, network and volume were removed immediately after
  browser verification (`down -v`).
- With repository files held fixed, the complete Playwright suite passes **all
  47 browser workflows** in a single run (`2.3m`), including all four
  `consoles-files.spec.ts` workflows and all existing `sessions.spec.ts` and
  `completion.spec.ts` regressions.
- Clean-commit (`79878e9`, `sourceDirty=false`) `pnpm desktop:build`,
  `pnpm desktop:bundle:mac` (strict ad-hoc signature verification on
  `Joinery.app`, `21.07 MiB`, bundle executable SHA-256
  `2d16256ab3ddca4958b57ef479aa73955b53085bf3ba12274fc4d7e7867ef054`, built in
  an isolated target directory while preserving the user's running `Joinery.app`
  instance), `pnpm desktop:smoke:build` and `python3 scripts/native-smoke.py
  --runs 3 --cycles 10` all pass: three hidden processes (`5,842 / 3,182 /
  3,274 ms`), 30 cleanup cycles, main-process peak RSS `102.44–107.66 MiB`,
  native smoke binary SHA-256
  `06af43336d6d4ad388e24040575cfba959263ebfdada56eee000a9290ed65f5a`
  (`bundleBinarySha256`
  `5c8827496e7c1dcb0811710a4e3979ac9d96c0e76f77f6ebc85fab3252d100c9`).
- No production database connections, automatic SQL execution/replay, raw-path
  IPC or Safe Mode weakening were introduced. Live Snowflake/production account
  qualification, Windows/Linux packaged runtime, Developer ID
  signing/notarization and representative performance gates remain open.

## Current assessment and R6 estimated plans (1 November 2026)

Fetched `origin/main` is `a0ba1a6` (R4, PR #30). R1–R4 are merged but absent
from preview.15, confirmed against the GitHub release list. The installed app's
exact preview revision was not requalified. Joinery is an advanced preview, not
yet a demonstrated full DataGrip replacement.

The separate `feat/editor-intelligence` candidate at `bb77b0f` contains eight
R5 commits. Its four uncommitted documentation files and existing processes
were preserved. Its local record reports 146 frontend, 139 standard Rust,
56 disposable fixture and 49 browser passes plus clean-commit macOS gates.
These recorded R5 outcomes were not independently rerun here. The current
R6 branch does not contain that candidate; integration must reconcile the
typed-parameter `prepare_execution` signature before qualification.

The first R6 slice is implemented in `fad0341` on `feat/estimated-query-plans`
from `a0ba1a6`. [query-plans.md](query-plans.md) records its bounded scope.
It is local, unmerged, unpublished and not installed. This first slice left
executing ANALYZE, comparison, richer timing and Snowflake plans open; the
PostgreSQL actual-plan follow-up below supersedes its ANALYZE status.

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 126 frontend tests (24 files), 130 standard Rust tests (109 core + 21 others), formatting, contracts, types, production build, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Seven browser workflows on disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4: three `query-plans.spec.ts` flows plus four `consoles-files.spec.ts` regressions. Estimated action, isolated session, unchanged SQL, raw/tree/grid round-trip and explicit batch rejection pass |
| Passed | `pnpm test:git` 6/6; runtime-memory script tests 4/4; identity and diff checks |
| Passed | Clean-source `fad0341` macOS production bundle (21.07 MiB) and strict ad-hoc signature verification; executable SHA-256 `5e6e9f94ea4e02cc395978ca4e007251b31fc906dc17e8c6713d91140807e65f` |
| Passed | Clean-source `pnpm desktop:smoke:build` and three hidden native runs / 30 cleanup cycles: `3,827 / 3,083 / 3,139 ms`, main-process peak RSS `106.17–106.86 MiB`; actual WebView IPC reaches the `prepare_explain` session guard; fixture SHA-256 `2a562b46d1bc9f52bd30c1857d88f8cf4cdd45924e851275002572e09088dbcd` |
| Environment resolved | Node 26.7.0's experimental global web storage broke five existing jsdom suites. Validation passes with `NODE_OPTIONS=--no-experimental-webstorage` and `/opt/homebrew/bin` first on PATH; no application workaround was added |
| Not executed | Full ignored Rust integration suite and full browser suite for this R6 slice; complete independent R5 rerun; installed app revision; production/live Snowflake accounts; Windows/Linux packaged runtime; Developer ID signing/notarization; assistive technology and representative performance gates |

Native evidence is synthetic, hidden and main-process-only; it does not qualify
cold visible startup, total idle memory or display FPS. Disposable Compose
project `joinery-assess-r6` uses isolated PostgreSQL port 55433 and was removed
after the browser checks. No production connections or automatic SQL replay
were introduced.

## R6 PostgreSQL actual plans (1 November 2026)

Code `fa1ff17` follows the estimated-plan slice on `feat/estimated-query-plans`.
It adds PostgreSQL **Analyze**, with explicit one-use Rust review even on Silent
profiles, exact generated SQL and an execution warning. It also fixes a
reproduced safety regression: sqlparser stores parenthesized ANALYZE options
separately from the plain EXPLAIN analyze flag. Recognized executing forms now
require review; explicitly disabled analysis stays non-executing. ReadOnly
denial and production passphrases remain enforced.

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 126 frontend tests (24 files), 132 standard Rust tests (111 core + 21 others), formatting, contracts, types, production build, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Two ignored PostgreSQL fixture tests (`actual_plan_*`) executed against disposable PostgreSQL 18: no unapproved execution, exact reviewed SQL, temporary session state, single-use/stale approval, disconnect invalidation, error recovery and cancellation retirement |
| Passed | Nine browser workflows on PostgreSQL 18 / MySQL 8.4 / MariaDB 11.4: five `query-plans.spec.ts` flows plus four `consoles-files.spec.ts` regressions; review cancellation sends no execute request, accepted analysis leaves editor text intact and exposes actual metrics, direct parenthesized SQL requires review, ReadOnly rejects analysis, and errors/cancellation retain session policy |
| Passed | Clean-source `fa1ff1733abff10d16fc0b440b0c861e7529e66a` macOS production bundle with strict ad-hoc signature verification; executable 20.92 MiB, SHA-256 `2a707a9c04b1e415c45bab4da286f18d80b85625d463dee6749e96a7489ec3b8` |
| Passed | Clean-source hidden native build and three runs / 30 cleanup cycles; actual and estimated `prepare_explain` IPC reach Rust session guards. Native elapsed `6,026 / 3,189 / 3,177 ms`; main-process peak RSS `106.09–106.69 MiB`; fixture SHA-256 `7d11daed35b68c3e342df65c1721a74637f402f57eaca060c2de571f2a1df8a6` |
| Environment | Node 26.7.0 validation uses `NODE_OPTIONS=--no-experimental-webstorage`; no application workaround. A browser run was invalidated by a Vite full reload while source files were changing; the final nine-flow run held files fixed and passed |
| Not executed | Full ignored Rust integration/full browser suites; independent complete R5 rerun; installed revision; production/live Snowflake accounts; Windows/Linux packaged runtime; Developer ID signing/notarization; assistive technology and representative total-memory/cold-start/FPS qualification |

Native evidence at `.cache/native-smoke/runs.json` is hidden, synthetic and
main-process-only. It does not qualify total idle memory, visible cold startup
or display FPS. Fixtures use disposable Compose project `joinery-assess-analyze`
with PostgreSQL on port 55433; it was removed after browser verification.
No production access or automatic SQL replay.
The application candidate is local, unmerged, unpublished and not installed.
R6 remains partial; [next slices](implementation-roadmap.md#r6-next-slices)
cover comparison, metrics and independent engine adapters. The separate R5
worktree and its four unrelated documentation edits remain intact.

## R6 bounded plan snapshots and comparison (1 November 2026)

Code `ba984f2` adds explicit snapshots of completed retained plans and compatible
side-by-side trees/raw JSON. Captures use immutable executed SQL plus known
engine/version, connection/session and observed runtime context; editing the
SQL after execution cannot change a capture's identity. Exact underlying query
bytes must match. Estimated/actual wrappers may differ; incompatible choices
show a disabled reason. No capture/comparison executes SQL or guesses node
pairing. Context is the frontend observation at request time, not an atomic
server snapshot; queued/unobserved changes and database state may differ.

A global ten-capture / 2 MiB UTF-8 serialized payload budget includes raw JSON,
SQL and context. FIFO eviction and explicit removal/clear keep retention bounded;
invalid/oversized captures cannot evict good entries. Existing raw-plan parser
limits still apply. These are payload caps, not measured total JavaScript RAM.
Captures survive result replacement, clear on owner-tab/session/connection
cleanup, and are excluded from workspace persistence.

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 134 frontend tests (25 files), 132 standard Rust tests (111 core + 21 others), formatting, contracts, types, production build, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Six snapshot unit tests and two studio lifecycle regressions cover exact SQL and raw numeric tokens, engine/version/session/context mismatch, UTF-8 count/byte eviction, malformed/unsupported/oversized input, cleanup notifications and workspace persistence exclusion |
| Passed | Ten relevant browser workflows on disposable PostgreSQL 18 / MySQL 8.4 / MariaDB 11.4: six query-plan flows and four console/file regressions. Live flows exercise tree/raw comparison, explicit removal, editor changes after execution, PostgreSQL estimated-versus-actual plans, zero extra execution requests for capture/comparison, disabled mismatches, cancellation retirement and reconnect cleanup |
| Passed | Clean-source `ba984f20a993b9732c336fcc8bbbbc1a46dea6c2` macOS production bundle with strict ad-hoc signature verification; executable 20.93 MiB, SHA-256 `f20d43a48250856f8f95647d2c054079d7670210ed4fd9a35c738659461837a5` |
| Passed | Clean-source hidden native build and three runs / 30 cleanup cycles; native elapsed `4,630 / 3,773 / 3,813 ms`, main-process peak RSS `104.45–107.05 MiB`; fixture SHA-256 `7de17912a4d0b873b72f9e8a80876a60de8273b173d8bf5b12416dc615957b16`. Baseline IPC, result/cache and session cleanup checks pass |
| Environment | Node 26.7.0 validation uses `NODE_OPTIONS=--no-experimental-webstorage`; no application workaround. Initial browser failures exposed the missing comparison region semantics and assertion issues; corrected final ten-flow run held source fixed and passed |
| Not executed | Full ignored Rust integration/full browser suites; the two PostgreSQL actual-plan fixture tests passed in the prior unchanged-backend slice and were not rerun here. Independent complete R5 rerun; installed revision; production/live Snowflake accounts; Windows/Linux packaged runtime; Developer ID signing/notarization; assistive technology and representative total-memory/cold-start/FPS qualification remain open |

Native evidence at `.cache/native-smoke/runs.json` is hidden, synthetic and
main-process-only. It does not qualify total idle memory, visible cold startup
or display FPS. Snapshot flows are covered by the frontend/browser tests above.

Disposable Compose project `joinery-assess-compare` used PostgreSQL port 55433
and was removed after the browser checks. Existing R5 worktree and its four
unrelated documentation edits remain intact. The application is local,
unmerged, unpublished and not installed. R6 remains partial; the
[next slices](implementation-roadmap.md#r6-next-slices) cover distinct metrics
and independent engine adapters.

## R6 execution timing and observed plan context (1 November 2026)

Code `cb1ca1a` separates query queue wait, backend active/total, first
nonempty driver row batch and client request-to-finish duration. Zero remains
valid; unavailable rows/backend/server metrics remain unavailable. Batch timing
is not first-wire-row or render latency. PostgreSQL server plan times remain
separate. No server/network/render metrics are derived. See the exact
[measurement scopes](query-plans.md#execution-timing).

Plan capture identity now uses Rust-observed context immediately before the
recognized plan runs, after acquiring its serialized editor lane. Queued session
changes therefore precede observation. Failure withholds capture instead of
using frontend cache; ordinary plan viewing remains available. The extra
metadata probe is included in active time. Data state and unobserved settings
remain outside the comparison guarantee.

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 136 frontend tests (26 files), 132 standard Rust tests, formatting, contracts, types, production build, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Full disposable PostgreSQL 18 fixture suite: 26 tests, including new queued context/timing and empty/error/multi-result tests; active and queued cancellation now assert unavailable first batch when no rows arrive. Existing approval/session, safety, grid, data movement, context and cleanup tests pass |
| Passed | Twelve relevant browser workflows on disposable PostgreSQL 18 / MySQL 8.4 / MariaDB 11.4: eight plan/timing flows and four console/file regressions. Metrics are visible on all three engines; unavailable context withholds capture while preserving tree view; empty/error/multi-result timing is exercised end to end |
| Passed | New frontend state/presentation tests distinguish measured zero from unavailable, reject stale/foreign-run measurements, clear timings on rebinding and exclude them from workspace restoration |
| Passed | Clean-source `cb1ca1a65e1edb5ab06810024f943f267e49d688` macOS production bundle with strict ad-hoc signature verification; executable 20.93 MiB, SHA-256 `ec91600fbb003a9a440448264b802731295a490ffb08906617afb04ada0107a7` |
| Passed | Three hidden native synthetic integration runs / 30 cleanup cycles; 5805 / 3135 / 3146 ms, peak main-process RSS 106.17–107.25 MiB. Existing IPC/cache/session cleanup checks pass; this fixture does not exercise the timing disclosure/plan comparison UI or qualify cold visible startup, total idle memory or FPS |
| Provenance | Native fixture revision `cb1ca1a65e1edb5ab06810024f943f267e49d688`, tree `3c0a1f709d288656953a13610014e11040fcd826`, `sourceDirty: false`, binary SHA-256 `b710563e97c23f5b0f23caaf21228de8afee3513116a9ccf02416d73fc18d6ef`, ad-hoc bundle binary SHA-256 `e75c049f9f58f179a330731fe213ef607ab78289411d1c1718325e04fc82700b` |
| Environment | Node 26.7.0 uses `NODE_OPTIONS=--no-experimental-webstorage`; no application workaround. Final sources held fixed for validation/fixture/browser checks |
| Open | Full MySQL/MariaDB/Redis/SSH ignored suites and full browser suite for this slice; independent R5 rerun/integration; installed revision; live Snowflake; Windows/Linux packaged runtime; Developer ID signing/notarization; assistive technology and representative total-memory/cold-start/FPS qualification |

Disposable Compose project `joinery-assess-timing` uses PostgreSQL port 55433
and is removed after checks. The unrelated R5 worktree remains intact. This
application candidate is local, unmerged, unpublished and not installed.
R6 remains partial; independent engine adapters and integrated qualification
continue in the [roadmap](implementation-roadmap.md#r6-next-slices).

## R6 reviewed MariaDB actual plans (1 November 2026)

Code `c660bc4` adds MariaDB `ANALYZE FORMAT=JSON` through the existing owned
session, immutable review and one-use approval path. A server-family check plus
protocol preparation of a fixed constant SELECT enables the capability; the
statement is closed without execution. No optimizer or output settings change.
The action validates one complete SELECT/WITH query before wrapping it.

A reproduced regression showed directly entered MariaDB analysis could pass
Silent mode without review when sqlparser fell back to Unknown. Token detection
now enforces mandatory review for statement-leading ANALYZE and conservative
EXPLAIN/ANALYZE forms before fallback, including later batch statements. Quoted
text/comments remain unaffected. ReadOnly blocks executing analysis; production
retains its passphrase. Tests prove unapproved execution leaves a session-local
counter unchanged, approved analysis runs once on its temporary-table session,
stale/disconnected approvals expire, errors recover and cancellation retires
the session. MariaDB results use the ANALYZE column for existing plan views and
observed-context snapshot comparison.

MariaDB actual rows are averages per loop; actual filtering is a percentage;
node total time includes children and must not be summed. Optimization time is
separate; constant plans can report it without execution/row metrics. No missing
measurement is invented. See [the plan contract](query-plans.md).

| State | Evidence |
|---|---|
| Passed | `pnpm validate`: 138 frontend tests / 26 files, 133 standard Rust tests (112 core plus 21 integration), contracts/types/build, Prettier, Cargo formatting/check and strict all-target/all-feature Clippy |
| Passed | All 26 PostgreSQL fixture tests and 24 MySQL/MariaDB base fixture tests on PostgreSQL 18, MySQL 8.4.13 and MariaDB 11.4.13; includes three new capability/approval/session tests, TLS-through-SSH, live context, transactions, grid, explorer and data movement. Separate case-folded fixture excluded |
| Passed | Fifteen browser workflows: eleven plan/timing flows plus four console/file regressions. MariaDB covers review cancellation without execute, unchanged SQL, raw/tree/grid, direct-SQL review, ReadOnly denial, estimated/actual snapshots without replay, mismatch/cleanup, error recovery and cancellation retirement |
| Passed | Five display-adapter and seven snapshot identity/store tests include inclusive timing, optimization-only actuals with zero and generated MariaDB query identity |
| Passed | Clean-source `c660bc4c23b8c6b8a59a9a084d7910705b5177c3` macOS production bundle with strict ad-hoc signature verification; executable 21,950,016 bytes (20.93 MiB), SHA-256 `7b63b0eb9c798bd014d33f774f205d8c32ac650b87d1f5fea11f49022c486a7a` |
| Passed | Three hidden native synthetic runs / 30 cleanup cycles; 5754 / 3094 / 3081 ms, peak main-process RSS 106.64–107.36 MiB. Baseline IPC/cache/session cleanup passes. This fixture does not exercise MariaDB plan UI/transport or qualify cold visible startup, total idle memory or FPS |
| Provenance | Native fixture revision `c660bc4c23b8c6b8a59a9a084d7910705b5177c3`, tree `ece4030c7129decc40970b2c7e8d161078c581a2`, `sourceDirty: false`, binary SHA-256 `2556fda558a7c1455f247a93fc8b8acb89df9fbfd08a88c1c46228dbf7be263c`, ad-hoc bundle binary SHA-256 `2639673f4b703fc203e79da09abc7abbbe1a8fbee121814b9e4a983563850e6f` |
| Scope | MariaDB capability/action qualified on 11.4.13 only. MySQL 8.4.13 defaults to `explain_json_format_version=1` and rejects JSON analysis (1235); a separate disposable session using version 2 returns a different JSON-v2 contract. This probe does not enable a MySQL action or change application connection settings |
| Unmerged / unpublished | R6 is local on `feat/estimated-query-plans`, separate from the preserved R5 candidate. R1–R4 are merged but absent from preview.15. No application push, merge, release or installation in this slice |
| Open | Case-folded fixture, Redis/SSH ignored suites and full browser suite for this slice; independent R5 review/integration; installed revision; live Snowflake; Windows/Linux packaged runtime; Developer ID signing/notarization; assistive technology; representative total-memory/cold-start/FPS qualification |

Logs: `.cache/mariadb-validate.log`, `mariadb-all-base-fixtures-final.log`,
`mariadb-postgres-fixtures.log`, `mariadb-browser-final.log`,
`mariadb-engine-contracts.json`, `mariadb-macos-bundle.log`,
`mariadb-bundle-evidence.json`, `mariadb-native-build.log`,
`mariadb-native-runs.log`, and `.cache/native-smoke/{build,runs}.json`.
The initial fixture run used the browser port variable; rerunning with the Rust
suite's `JOINERY_TEST_PORT=55433` resolves those connection failures. Final
held-source MySQL/MariaDB base suite passes 24/24 in 5.67 s; browser 15/15 in
54.3 s. Disposable project `joinery-assess-actual` uses PostgreSQL port 55433.

## R6 reviewed MySQL TREE actual plans (1 November 2026)

Code `c54ecab` independently enables MySQL `EXPLAIN ANALYZE FORMAT=TREE` after
preparing and closing a fixed constant SELECT without executing it. No optimizer
or output setting changes. The existing complete-query wrapper, explicit review
even on Silent, production passphrase, ReadOnly denial, owned session, one-use
approval and observed-context comparison remain authoritative.

Two failures were reproduced before their fixes. `DESC ANALYZE FORMAT=TREE FOR
SCHEMA joinery SELECT 1` fell outside AST grammar and could bypass review; the
pre-parser now recognizes EXPLAIN's DESC/DESCRIBE aliases. MySQL KILL QUERY can
return a successful partial TREE result; recognized analysis now checks the
cancellation request even on successful result consumption, reports Cancelled
and retires its session. This does not imply rollback of side effects. Normal
write acknowledgement handling is preserved.

The bounded adapter preserves exact raw TREE text and engineering tokens,
four-space hierarchy, missing costs and never-executed iterators. Actual first
row / inclusive total times and rows are per loop; loops stay separate. No child
sum, statement duration or missing zero metric is invented. Unsupported shapes
keep raw/grid access. JSON estimates and TREE actuals compare with exact query
bytes and observed context, without rerunning SQL. See [the contract](query-plans.md).

| State | Evidence |
|---|---|
| Passed | `pnpm validate` on Node 26.7.0: 142 frontend tests / 26 files, 133 standard Rust tests (112 core + 21 integration), contracts/types/build, Prettier, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | 26 PostgreSQL fixtures and 25 MySQL/MariaDB base fixtures on disposable PostgreSQL 18, MySQL 8.4.13 and MariaDB 11.4.13. The latter includes four plan tests for capability, review/session effects, error/cancellation and JSON-settings preservation / unexecuted iterators. Case-folded fixture excluded |
| Passed | Eighteen browser workflows on Node 26: fourteen plan/timing + four console/file regressions. MySQL adds cancelled review without execute, unchanged SQL, raw/tree/grid, direct alias review, ReadOnly, JSON-estimate/TREE-actual comparison without replay, mismatch/cleanup, error recovery and cancellation retirement |
| Passed | Eight display-adapter and eight snapshot tests cover nested/zero/engineering/missing/unexecuted metrics, unsupported hierarchy/continuations, depth/node/byte bounds, exact raw retention and JSON/TREE identity |
| Passed | Clean-source `c54ecab3caa2f46022d7b70cf6d0873d6ada90b7` macOS production bundle with strict ad-hoc signature verification; executable 21,950,016 bytes (20.93 MiB), SHA-256 `5327e4215c6c49eafec5adb9800008b181f8379ddf5bb8690c60faf2b52f3a81` |
| Passed | Three hidden native synthetic runs / 30 cleanup cycles; 5756 / 3033 / 3026 ms, peak main-process RSS 106.02–106.44 MiB. Baseline IPC/cache/session cleanup passes; this does not exercise MySQL native plan UI/transport or qualify visible cold startup, total idle memory or FPS |
| Provenance | Native fixture revision `c54ecab3caa2f46022d7b70cf6d0873d6ada90b7`, tree `0c631c8e8499334977abf0b8bffb10d72e325742`, `sourceDirty: false`, binary SHA-256 `3765cd785dd36ea51e6268519f7470f414c419be4d91a966af99ce5455bee888`, ad-hoc bundle binary SHA-256 `09942ab4060a6244925fc88561878f7da68bd9ce487b33317c4579af542c89ae` |
| Scope | MySQL capability/TREE action qualified on 8.4.13 only; MariaDB regression on 11.4.13. Session `explain_format=JSON` and `explain_json_format_version=1` remain unchanged. No JSON-v2 adapter enabled |
| Local only | R6 remains unmerged, unpublished and not installed, separate from preserved R5. Fetched `origin/main` stays `a0ba1a6`; R1–R4 are merged but unreleased. Preview.15 is the last verified release; this slice's GitHub refresh was blocked by command approval policy |
| Open | Case-folded fixture, Redis/SSH ignored suites, full browser suite and independent R5 integration; installed revision; live Snowflake; Windows/Linux runtime; Developer ID signing/notarization; assistive technology; representative total-memory/cold-start/FPS qualification |

Environment and failures: Node 26.7 requires
`NODE_OPTIONS=--no-experimental-webstorage` for the existing localStorage test
environment. A shell-selected Node 22 run passed but was repeated on supported
Node 26. One full run failed the unrelated concurrent metadata startup test;
its targeted recheck and both subsequent full validation runs passed, without
persistence changes. An accidental all-ignored run included the absent mode-1
case fixture and failed that connection; the explicitly scoped base suite passes
25/25. Initial cancellation fixtures used a projection MySQL can omit; the final
constant WHERE/SLEEP filter ensures work and reproduced partial-plan success
before the driver fix. No tests were skipped to hide either product regression.

Logs: `.cache/mysql-tree-validate-node26.log`,
`mysql-tree-fixtures-base-final.log`, `mysql-tree-postgres-fixtures.log`,
`mysql-tree-browser-node26.log`, `mysql-tree-safety-before.log`,
`mysql-tree-fixtures-final.log`, `mysql-tree-persistence-recheck.log`,
`mysql-tree-macos-bundle.log`, `mysql-tree-bundle-evidence.json`,
`mysql-tree-native-build.log`, `mysql-tree-native-runs.log`, and
`.cache/native-smoke/{build,runs}.json`. The disposable Compose project is
`joinery-assess-mysql-tree`, using PostgreSQL port 55433. Next slices are
[Snowflake estimates and integrated qualification](implementation-roadmap.md#r6-next-slices).

## R6 Snowflake estimated logical plans (1 November 2026)

Code `48dfb21` adds explicit `EXPLAIN USING JSON` for a complete validated
Snowflake query. The exact-wrapper safety adapter resolves sqlparser's missing
USING JSON grammar without accepting prefix lookalikes or allowing a batch
escape. It preserves SQL bytes and existing read-only classification. The
owned SQL API request carries configured warehouse/role/database/schema/query
tag; the target query is never submitted separately. No session or warehouse
settings change and no executing Analyze action is introduced.

The bounded display preserves every logical parent edge and step-local ID.
Shared operators expand once per step with metric-free references; duplicate
IDs/edges, cycles, missing parents, malformed shapes and complexity excesses
retain raw/grid fallback. Assigned partitions and bytes are upper-bound pruning
estimates, never actual timing/rows. Unsafe integer estimates defer to exact raw
JSON. Snowflake CONTENT results require recognized execution identity and normal
queries clear it. Snapshot capture is disabled in both UI and store because
warehouse/session observations are not yet sufficient for comparison.

| State | Evidence |
|---|---|
| Passed | `pnpm validate` on Node 26.7.0 with `NODE_OPTIONS=--no-experimental-webstorage`: 148 frontend tests / 28 files, 136 standard Rust tests (115 core + 21 integration), contracts/types/build, Prettier, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Loopback synthetic HTTP verifies exact wrapper, configured request context and unchanged raw JSON including unsafe integer tokens; all Snowflake core/lifecycle/catalog/auth/compression regressions run in standard Rust tests. No SSO or live database account used |
| Passed | Four headless synthetic Snowflake browser workflows: two plan workflows plus existing catalog/lazy completion/read-only preview and discovery-failure recovery. Covers step/shared-reference rendering, raw/grid transitions, exact large numeric text, disabled snapshots, unchanged editor, no Analyze, ordinary CONTENT identity reset and malformed graph raw fallback; zero page errors |
| Passed | Four new DAG adapter tests and identity/result-detection tests cover order-independent/multistep/shared edges, zero/unsafe estimates, cycles/orphans/duplicate IDs/edges, invalid shapes, 512 KiB/depth 40/shared 2,000-item budget, capture refusal and exact engine mismatch |
| Passed | Clean-source `48dfb21cbeb2e84049f519b2b35ccf1873590b50` macOS production app bundle, strict ad-hoc signature verification; executable 21,950,016 bytes, SHA-256 `9495bad07dda69f8a4cc6b4dc45b025137a489cd7109dc5b176b936937d4a7c7` |
| Passed | Three hidden native synthetic runs / 30 cleanup cycles; 5780 / 3048 / 3135 ms, peak main-process RSS 102.08–107.33 MiB. Baseline IPC/cache/session cleanup only; does not exercise Snowflake native plan UI/transport or qualify cold visible startup, total idle memory or FPS |
| Provenance | Native fixture revision `48dfb21cbeb2e84049f519b2b35ccf1873590b50`, tree `e8fec66513d429d28f2988feb77e08d9cd2b4d99`, `sourceDirty: false`, binary SHA-256 `4579c734331349ca254f6d38684f1e267d158e7bb1c19051390469811966a1ca`, ad-hoc bundle binary SHA-256 `383cf5bb2555d5dc6293f1a7d9b17b611b310da80754b432a70ac7ff4fef6c64` |
| Not rerun | PostgreSQL/MySQL/MariaDB ignored fixture suites and prior eighteen plan/timing/console browser workflows, case-folded engine, Redis/SSH ignored suites and full browser suite. Previous iteration's counts remain dated evidence, not qualification of this source |
| R5 inventory drift | Current Git and complete local Orca inventories contain main and assess only. Former R5 worktree and `feat/editor-intelligence` branch are absent; commit `bb77b0fc785d7a0076a3aa6d2457ac4da09d5ce9` remains readable. Former dirty docs were not recovered; recorded R5 verification is not a current rerun. No worktree/branch recreation or changes to other checkouts in this slice |
| Local only | R6 remains unmerged, unpublished and not installed; fetched `origin/main` remains `a0ba1a6`. R1–R4 are merged but unreleased. Preview.15 remains last verified release; earlier GitHub refresh was blocked by command approval policy |
| Open | Integrated R5/R6 review and qualification; prior metadata-startup flake investigation; observed Snowflake warehouse/session identity and snapshots; live account qualification; installed revision; Windows/Linux runtime; Developer ID signing/notarization; assistive technology; representative memory/cold-start/FPS |

Reproduced before implementation: the frontend rejected Snowflake plan JSON;
read-only preparation rejected the generated USING JSON wrapper. Both pass after
the adapters. An initial browser run failed an ambiguous Run button test locator;
the corrected locator passes all four workflows. Strict Clippy rejected a
single-element test loop; its correction passes full validation. No safety
policy, hook or engine rejection was bypassed. The earlier concurrent metadata
startup failure did not recur in this completed run; its cause remains open.

Logs: `.cache/snowflake-plans-validate-complete.log`,
`snowflake-plans-browser-final.log`, `snowflake-plans-macos-bundle.log`,
`snowflake-plans-bundle-evidence.json`, `snowflake-plans-native-build.log`,
`snowflake-plans-native-runs.log`, and `.cache/native-smoke/{build,runs}.json`.
See [query-plan semantics](query-plans.md#snowflake-estimated-logical-plans) and
[the next slices](implementation-roadmap.md#r6-next-slices).

## R5 integration: live FK JOIN completion (1 November 2026)

Code `e0dad5f12461fdfdecae073be62fe74d3f57f004` restores the first R5 feature
group onto the R6 branch, using recorded `40c59a7` and the SQL alias correction
from `41957ea`. It does not incorporate the remaining editor-intelligence
commits. Former worktree/branch absence and unrecovered dirty docs remain as
recorded above; the source commit `bb77b0f` is readable.

PostgreSQL/MySQL/MariaDB completion now obtains permission-filtered outbound
and inbound constraints on the actual editor session. Ordered composite pairs
stay grouped; incomplete or denied relationships are withheld. Temporary
shadows do not inherit permanent-table relationships. MySQL metadata cleanup
retains the existing transaction-state behavior. Snowflake supplies no FK
relationships in this slice.

JOIN target and initial ON completion insert explicit equality predicates from
known relationships. Multiple constraints remain distinct; self joins choose
noncolliding aliases; quoting and CTE shadows are respected. CROSS/NATURAL JOIN
does not receive an ON clause. Existing query-block/token/depth/schema-read and
response-revision bounds remain. This is direct-base-table assistance, not
derived/CTE output inference. Accepting a suggestion never runs SQL.

| State | Evidence |
|---|---|
| Reproduced | Three added frontend tests failed before implementation. The first browser run exposed PostgreSQL's `ln` alias being tokenized as a built-in and discarded; the recorded alias correction and a dedicated unit regression fix it |
| Passed | `pnpm validate` on supported Node 26.7 with `NODE_OPTIONS=--no-experimental-webstorage`: 152 frontend tests / 28 files, 136 standard Rust tests (115 core + 21 integration), contracts/types/build, Prettier, Cargo fmt/check and strict all-target/all-feature Clippy |
| Passed | Two ignored live-completion fixture tests: PostgreSQL 18.6 and one test looping MySQL 8.4 / MariaDB 11.4. Covers outbound/inbound/composite metadata, temporary shadows and existing role/grant/search-path/database/rollback behavior |
| Passed | Fourteen headless browser workflows: three new FK flows (nine accepted-and-executed outbound/inbound/ON composite joins), seven existing completion flows and four synthetic Snowflake flows. Zero page errors in new flows. PostgreSQL safely generates `ln2` for a new alias; a reverse-join test that assumed `ln` was corrected before the passing run |
| Local only | Code is on `feat/estimated-query-plans`, unmerged, unpublished and not installed. Fetched `origin/main` remains `a0ba1a6`. R5 is partly integrated; R6 stays partial |
| Not rerun | Full ignored fixture/full browser suites, prior eighteen plan/timing/console browser workflows and native/package gates on this source. The preceding `48dfb21` native evidence is historical, not qualification of this revision |
| Open | Remaining R5 diagnostics, typed parameters and R6 authorization compatibility, editor navigation/formatting/snippets; earlier metadata-startup flake investigation; integrated native/fixture/browser qualification; Snowflake observed context and live accounts; release/install/platform/signing/accessibility/performance gates |

Logs: `.cache/r5-joins-before.log`, `r5-joins-validate.log`,
`r5-joins-fixtures.log` and `r5-joins-browser.log`. Disposable Compose project
`joinery-assess-r5-integration` uses isolated PostgreSQL port 55433; no production
account is used. The earlier metadata-startup failure did not recur in this
validation; its cause remains open. See the
[remaining integration sequence](implementation-roadmap.md#r6-next-slices).

## R5 integration: structured Unicode diagnostics (1 November 2026)

Code `ce7a7fa480e2bb8b3692801b935fd17d4b539366`, tree
`9bc2dd4ebb97d758bb0641fd42fbcf1f744aa107`, integrates recorded `581c527`
with the existing seven R6 slices and the preceding FK JOIN slice. Optional
`AppError.location` carries a half-open UTF-16 range, one-based Unicode scalar
line/column, statement index, SQLSTATE and available PostgreSQL detail/hint.
Generated contracts preserve compatibility with errors and plans lacking spans.

PostgreSQL server character positions map across accented text, CJK, emoji and
later batch statements. MySQL/MariaDB syntax messages supply best-effort token
or line ranges; unavailable positions use the failing statement. Their leading
whitespace normalization is mapped back explicitly. A server-sanitized snippet
falls back to its reported line instead of claiming token precision. Snowflake
compilation positions and SQLSTATE have synthetic adapter coverage only.
Cancellation retains its existing terminal behavior without SQL error markers.

ReadOnly parser/policy failures carry locations without changing classification
or authorization. Parser fallback still submits one complete unclassified batch;
its span starts at the submitted buffer origin, including leading whitespace.
Generated row-change plans carry no invented editor range. Explain validation
reports parser positions in the unwrapped input; directly entered Snowflake
wrappers shift inner parser/policy locations once. Runtime locations refer to
the prepared SQL buffer; editor selection and generated-wrapper origin handling
must be reconciled when integrating markers and typed parameter execution.
Inline editor markers and error navigation are not implemented in this slice.

| State | Evidence for this source |
|---|---|
| Reproduced | Two preimplementation serialized-location tests fail with absent `location`; both pass after integration. Additional regressions cover fallback origins and direct Snowflake wrapper offsets |
| Passed | `pnpm validate`: 152 frontend tests in 28 files; 146 standard Rust tests (125 core plus 21 integration/unit checks); contracts, TypeScript, production build, formatting, desktop compilation and strict all-target/all-feature Clippy |
| Passed | All 27 PostgreSQL and 27 MySQL/MariaDB ignored fixture tests, including both case-folded instances, review/one-use authorization, cancellation, context, FK completion and the new Unicode diagnostics. PostgreSQL 18.6, MySQL 8.4 and MariaDB 11.4; no production account |
| Passed | Twenty-two headless browser flows: three new real diagnostic/correction flows, three FK flows, fourteen plan/timing flows and two synthetic Snowflake plan flows. Serialized UTF-16 ranges and SQLSTATE survive transport; each failed query is corrected and completes on the same session. Zero page errors in new flows |
| Passed | Hidden native build records the exact code/tree above with `sourceDirty: false`; three macOS arm64 WebView runs / 30 cleanup cycles pass. Synthetic integration baseline only; no representative performance claim |
| Local only | `feat/estimated-query-plans`, unmerged, unpublished and not installed. Fetched `origin/main` remains `a0ba1a6`. No application push, merge, release or installation |
| Not rerun | Full browser suite, separate Redis/PostgreSQL-SSH ignored suites, normal distribution bundle verification and external/platform gates. Earlier `48dfb21` distribution evidence remains historical |
| Open | Typed parameters with all R6 preparation/approval paths, editor markers/navigation/formatting/snippets, metadata-startup flake cause, observed Snowflake snapshot context, live accounts, release/install/signing/platform/accessibility/performance qualification |

Integration corrections: the recorded PostgreSQL diagnostic test referenced a
MySQL-only fixture table; it now uses `orders`. The recorded MySQL "syntax"
query was actually a missing-column query; a parser-accepted trailing wildcard
now reaches a real server syntax error on both engines. The added leading-space
fixture reproduced a position error and passes after origin normalization.
Initial strict Clippy found two explicit loop counters in the imported mapper;
both now use indexed iteration. The first full MySQL run omitted
`JOINERY_TEST_PORT=55433` for its cross-engine PostgreSQL test (26 passed, one
connection failure); all 27 pass with the correct fixture port. No product gate
or hook was bypassed. The earlier metadata-startup failure did not recur during
this validation; its cause remains open.

Logs: `.cache/r5-diagnostics-before.log`, `r5-diagnostics-validate.log`,
`r5-diagnostics-postgres.log`, `r5-diagnostics-mysql.log`,
`r5-diagnostics-all-postgres.log`, `r5-diagnostics-all-mysql-final.log`,
`r5-diagnostics-browser.log`, `r5-diagnostics-native-build.log`,
`r5-diagnostics-native-runs.log` and `.cache/native-smoke/{build,runs}.json`.
Owned Compose project `joinery-assess-r5-diagnostics` (PostgreSQL port 55433),
its five containers and disposable volume were removed after checks. Other
running services were preserved. Continue with the
[remaining integration sequence](implementation-roadmap.md#r6-next-slices).

## R5 integration: typed query parameters (1 November 2026)

Code `39359e7cfd0dbc33bb9a12e08bd8ca69c04d3c89`, tree
`81e80804552df82933f72b164b32ac6c25575145`, integrates recorded `9cfefa2`
with the existing R5 diagnostics/JOIN and seven R6 slices. Named `:name`,
PostgreSQL `$n`, and MySQL/Snowflake `?` placeholders are scanned outside
strings, quoted identifiers and comments. Snowflake also accepts `$n`. PostgreSQL
casts, assignment and JSONB question operators remain SQL; MySQL executable
comments remain rejected. Positional question marks are numbered across batches.

A transient per-tab dialog accepts text, signed 64-bit integers, decimals,
booleans, UUIDs, dates, timestamps, JSON and explicit typed NULL. Rust validates
all values, rejects duplicates/missing/unexpected inputs and canonicalizes
bindings before storing an immutable execution plan. Limits are 64 distinct
parameters, 1,024 occurrences, 256 KiB total input value bytes and an 8 MiB
prepared-plan payload budget; repeated binds are checked before cloning. Invalid
Unicode calendar values cannot panic; excessive Snowflake decimal precision and
MySQL fractional timestamp precision are rejected rather than truncated.

The reviewed SQL template, types and canonical values participate in one-use
approval identity. Changed values expire the prior plan; execution accepts no
replacement bindings. ReadOnly, production passphrase and mandatory Analyze
review remain authoritative, including Silent profiles. Parameter drafts clear
when tabs close; values are absent from persisted workspace/history/audit/local
metadata in the regression checks. Returned results, plans and server messages
can naturally contain the submitted values.

PostgreSQL uses typed SQL PREPARE and EXECUTE with hex-encoded UTF-8 constant
expressions, preserving exact text output; this is not a claim of binary-protocol
binding. Generated plans prepare the underlying query and use EXPLAIN EXECUTE
with the same options and bindings. Prepared names are deallocated after success
or after failed-transaction recovery. MySQL/MariaDB use protocol parameters and
explicit casts; Snowflake uses SQL API bindings with typed expressions. Snowflake
transport and rewritten diagnostics have synthetic coverage only, with no live
account qualification. Redis has no SQL-parameter workflow.

Explain and Analyze use the common parameter dialog and Rust plan store. Plan
snapshots retain a canonical binding digest and reject comparisons for different
bindings without retaining a separate copy of the values. Rewritten backend
positions map to original placeholders; generated Explain prefix coordinates
map to its submitted input query. PostgreSQL errors referring to EXECUTE internals
fall back to the complete query rather than inventing token precision. Inline
markers and mapping from an editor selection to the full editor remain pending.

| State | Evidence for this source |
|---|---|
| Reproduced | A new snapshot regression accepted different bindings before the identity fix; it now rejects them. Additional regressions cover Unicode calendar input, repeated-bind budgets, prototype-shaped parameter names and closed-tab draft cleanup |
| Passed | `pnpm validate`: 156 frontend tests / 29 files, 154 standard Rust tests (133 core plus 21 integration/unit checks), contracts, TypeScript, production frontend build, formatting, desktop compilation and strict all-target/all-feature Clippy |
| Passed | All 29 PostgreSQL and 29 MySQL/MariaDB ignored SQL fixture tests, including both case-folded instances. New tests cover all parameter kinds/NULL, exact large values, reviewed writes, Unicode positions, parameterized estimates/analysis, stale approvals and generated-wrapper origins. PostgreSQL 18.6, MySQL 8.4 and MariaDB 11.4; no production account |
| Passed | Twenty-three relevant headless browser flows: six new parameter flows, fourteen existing plan/timing flows and three error/correction flows. New flows execute reviewed injection-shaped text and large integers, verify template-only history, retain editor SQL, require Analyze review and reject snapshots with different bindings. Zero page errors in new flows |
| Passed | Hidden macOS arm64 WebView build records the exact clean code/tree above (`sourceDirty: false`); three fresh runs / 30 cleanup cycles pass. Synthetic integration baseline only, with no representative performance or native parameter-dialog qualification claim |
| Local only | `feat/estimated-query-plans`, unmerged, unpublished and not installed. Fetched `origin/main` remains `a0ba1a6`. No application push, merge, release or installation |
| Not rerun | Full browser suite, dedicated FK/completion/Snowflake browser suites, separate Redis/PostgreSQL-SSH ignored suites, normal distribution bundle verification and external/platform gates. Earlier `48dfb21` normal distribution evidence remains historical |
| Open | Inline markers/navigation and selected-buffer origins, definition/usages/documentation, selection formatting/snippets/settings, metadata-startup flake cause, observed Snowflake snapshot context, live accounts, release/install/signing/platform/accessibility/performance qualification |

Integration corrections: PostgreSQL SQL PREPARE cannot prepare EXPLAIN directly;
the adapter now prepares its underlying query. The imported PostgreSQL boolean
expectation now matches its existing text protocol (`t`). Initial strict Clippy
reported an explicit counter and a helper below the test module; both are fixed.
The first browser run passed 20 flows but failed three assertions because
Playwright treated a disabled option as enabled; those checks now assert the
actual disabled attribute and all three pass. No product gate or hook was bypassed.
The earlier metadata-startup failure did not recur; its cause remains open.

Logs: `.cache/r5-params-{validate,pg-full,mysql-full,browser,browser-rerun}.log`,
`r5-params-snapshots-before.log`, `r5-params-native-build.log`,
`r5-params-native-runs.log`, `r5-params-native-evidence.json` and
`.cache/native-smoke/{build,runs}.json`.
Owned Compose project `joinery-assess-r5-params` (PostgreSQL port 55433), its
five containers and disposable volume were removed after checks. Other running
services were preserved. Continue with the
[remaining integration sequence](implementation-roadmap.md#r6-next-slices).

## R5 integration: inline diagnostics and navigation (1 November 2026)

Code `65530c035c25586415ce0b6ba40de6bf14817187`, tree
`3848d35d774b6e62953c7c027b165067a3ed49b1`, integrates recorded `655d8b7`
with the current R6/parameter branch. Error underlines, gutter badges, Unicode
line/column, submitted-batch statement index, SQLSTATE, detail/hint and **Jump to
error** preserve structured parser/policy/server diagnostics. Previous/next SQL
statement navigation uses the active CodeMirror dialect tree (`⌥⌘↑/↓` or
`Ctrl+Alt+↑/↓`) and command palette. It does not guess statement boundaries while
a large document is still parsing. Redis keeps its command console behavior.

Execution captures the exact submitted slice, full source and UTF-16 offset.
That origin travels through parameter entry, preparation and one-use review.
Backend coordinates already map substitutions and generated Explain wrappers to
input SQL; the frontend adds the editor offset once. CRLF/bare CR file positions
convert to CodeMirror newline units. Invalid/out-of-slice spans, absent origins
and changed documents have no marker or jump. Editing clears the captured origin;
an old error arriving after an edit may display its message but cannot mark the
new text. Origins are transient and do not enter workspace snapshots.

| State | Evidence for this source |
|---|---|
| Regression baseline | Recorded navigation tests could not import the absent module before integration. Twelve focused cases now cover dialect statement ranges, selection offsets, Unicode/CRLF/bare CR, source identity, malformed spans and EOF |
| Passed | `pnpm validate`: 168 frontend tests / 30 files, 154 standard Rust tests (133 core plus 21 integration/unit checks), contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Passed | All 26 relevant real PostgreSQL/MySQL/MariaDB headless browser flows: six diagnostics/navigation flows, six parameter flows and fourteen plan/timing flows. Diagnostics cover batches, current statements, explicit selections, parameterized Run/Explain/Analyze, exact jump selection, edit invalidation and a delayed PostgreSQL error arriving after a replacement edit; zero page errors in the new flows |
| Native | Hidden macOS arm64 build records the exact clean code/tree above (`sourceDirty: false`); three fresh processes / 30 cleanup cycles pass (3.77 / 3.06 / 3.05 seconds). Fixture SHA-256 `0c8cc85395f5c8cc5ad58faa850489e20d1a29013f2d53ada8caa682d8047e0f`. Synthetic baseline only; no native error-UI, total-memory, cold-start or display-FPS qualification |
| Local only | `feat/estimated-query-plans`, unmerged, unreleased and not installed. Fetched `origin/main` remains `a0ba1a6`. No application push, merge, release or installation |
| Prior evidence | All 58 ignored SQL fixture passes belong to `39359e7`; no ignored SQL/Redis/SSH fixture suite was rerun for this frontend-only slice. Current browser checks use fresh PostgreSQL 18, MySQL 8.4 and MariaDB 11.4 fixtures |
| Not rerun | Full browser suite, dedicated FK/completion/Snowflake suites, normal distribution bundle verification, live accounts and platform/accessibility/performance gates. Native baseline does not qualify the error UI or representative performance |
| Next | Definition/usages/context documentation (`b5a744d`), selection formatting/settings/snippets (`3ca2ad2`), format-save serialization (`41957ea`) and relevant recorded `bb77b0f` regressions. Investigate the earlier metadata-startup flake and qualify the combined source before authorized delivery |

The first and final 26-flow browser runs passed. The final run includes the
line-ending correction and delayed-error case. No hooks or product safety gates
were bypassed. The earlier metadata-startup failure did not recur; its cause
remains open. Logs are `.cache/r5-inline-{validate-final,browser-final,native-build,native-runs}.log`
and immutable `.cache/r5-inline-native-evidence.json`. Owned Compose project
`joinery-assess-r5-inline`, its three containers and disposable volume were removed.
Other services and the installed application were preserved.

## R5 integration: SQL definitions, usages and documentation (1 November 2026)

Code `8ef9dfb94c3c9a9962bb6e1761bb1979e0822334`, tree
`9114c4d221ae05af10efbd5aa8ed71c245a50212`, integrates recorded
`b5a744d` with the current R6/parameter/error-origin paths. **F12** or
Cmd/Ctrl-click navigates CTE/table-alias declarations and primary-session object
structure, highlighting the resolved column. **Shift+F12** lists editor references;
**F1**, the Docs button and command palette show quick metadata documentation.
No action prepares or executes the editor SQL. Opening an existing Structure tab
retains its normal bounded metadata/table-read behavior.

Resolution uses the configured dialect syntax tree and current console catalog,
search path, database and identifier case policy. Usage candidates are resolved
in their own scopes, so nested alias/CTE shadows and correlated references do
not share references merely because their spelling matches. Columns require
known metadata and unambiguous ownership; ambiguous bare self-join columns, unknown/derived bindings,
explicit projection aliases and overloaded routines are not guessed. Unique
routines retain catalog identity. Strings/comments are excluded. This is a
bounded resolver, not complete SQL semantic analysis or inferred projection
lineage. CTEs without an explicit column list do not claim inferred columns.

Quick docs include available column types/nullability/PK/FK, object comments,
routine/sequence metadata and object-level database dependents. Missing or denied
metadata stays unavailable; dependency results are not column lineage. Schema
loads are limited to eight relevant tables per action. Editor references stop
at 500 entries or 10,000 name tokens and display a limit notice. Parsing retains
its 75 ms availability bound; large-query/catalog responsiveness is unqualified.

Edits, refreshes, context/session changes, close and unmount invalidate pending
navigation. Refresh replaces the schema cache; old promises cannot populate its
replacement. Catalog and detail reads use the console's session. Structure tabs
currently use the profile's primary session, so secondary-console object/column
Structure navigation is explicitly unavailable rather than inspecting another
session. Quick docs and local declaration jumps remain available there.

| State | Evidence for this source |
|---|---|
| Regression baseline | Missing symbol APIs failed the recorded tests before integration. Added cases reproduced incorrect nested scopes, unknown/ambiguous columns, routine overload/search-path resolution and projection-alias ownership before correction |
| Passed | `pnpm validate`: 183 frontend tests / 31 files, 154 standard Rust tests, contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy. The navigation module has 25 cases (13 symbol cases plus 12 prior diagnostic/statement cases); two overlay/Structure cases also pass |
| Passed | Final 46 real database browser flows on PostgreSQL 18, MySQL 8.4 and MariaDB 11.4: five symbol flows, seven completion, three FK JOIN, six diagnostics, six parameters, fourteen plan/timing, three sessions and two explorer/search. New flows verify docs, focus/Escape, usage selection, declaration/column jumps, zero editor execution, edit/refresh/close invalidation, denied metadata and the secondary-session boundary |
| Native | Clean-source macOS arm64 hidden build records the code/tree above (`sourceDirty: false`); three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `8c794a99f55f1da76ff370df3bb5dbf8ce711c43bd87b71478bb9053ea587f46`. Synthetic baseline only; this does not qualify native symbol UI, total memory, cold start or display FPS |
| Local only | `feat/estimated-query-plans`, unmerged, unreleased and not installed; fetched `origin/main` remains `a0ba1a6`. No application push, merge, release or installation |
| Prior evidence | The 58 ignored SQL fixture passes belong to `39359e7`; no ignored SQL/Redis/SSH suite was rerun for this frontend slice. Completion/FK/session/explorer browser regressions were rerun on the final source |
| Open | Complete semantic binding, projected/derived columns, routine argument-type overload selection, secondary-session Structure navigation, full browser suite, dedicated Snowflake suite/live accounts, normal distribution bundle, Windows/Linux runtime, signing/notarization, accessibility and representative performance |
| Next | Selection formatting/settings/snippets (`3ca2ad2`), format-save serialization (`41957ea`), and relevant recorded `bb77b0f` regressions. Preserve R6 authorization and exact editor origins; investigate the earlier metadata-startup flake and qualify the combined source before authorized delivery |

Earlier browser attempts exposed test synchronization/count assumptions: background
completion can issue concurrent schema reads. The final stale-response regression
holds all old replies and injects a stale type marker; refreshed docs retain the
current type. The final 46-flow run holds source fixed and passes. No hooks or
product safety gates were bypassed. Logs are
`.cache/r5-symbol-validate-session-final.log`,
`.cache/r5-symbol-browser-final-46.log`, and
`.cache/r5-symbol-native-{build,runs}.log`; immutable native evidence is
`.cache/r5-symbol-native-evidence.json`. Owned disposable Compose project
`joinery-assess-r5-symbols` was removed after checks; user services and the installed
application were preserved. The earlier metadata-startup flake did not recur;
its cause remains open.

## R5 integration: formatting and snippets (2 November 2026)

Code `db5cd1b5cbdca127e407d012d129f1d0260b8a9a`,
tree `60a1b994235713177b4558b40279dbe81f01167c`, integrates recorded
`3ca2ad2`, robust save serialization for the ordering issue in `41957ea`, and
relevant native command checks from `bb77b0f`. The alias correction from
`41957ea` was integrated earlier. No former dirty documentation was recovered.

Format SQL uses the active selection, or the full document when the selection
is empty. The worker returns one undoable replacement and retains a selected
trailing newline. Any document edit/replacement or engine/session change rejects
the old reply. Preferences persist global keyword case (upper/lower/preserve),
indent width (2/4 spaces) and query spacing (1/2 blank lines). Saves reach Rust
in order; optimistic updates compose and a failed latest save restores the last
successful settings. Preferences displays errors and prevents close/recovery
while writes are pending. Formatting preferences are global, not per dialect.

The formatter recognizes Joinery's named binds in all three SQL dialects,
PostgreSQL/Snowflake dollar positions and MySQL/Snowflake question positions.
Covered regressions preserve bind-name case, quoted Unicode text and comments,
PostgreSQL casts/JSON operators and Snowflake colon paths. Browser flows format
and execute a named bind on all three local SQL engines; literal/comment colons
do not become parameters. This is compatibility coverage, not complete dialect
grammar or live Snowflake qualification.

Built-in and custom templates expand through completion without executing SQL.
Custom engine scope overrides global scope, which overrides built-ins for the
same case-insensitive trigger. CodeMirror provides tab stops; `${0}`
sets the final cursor position. Qualified/quoted identifier completions exclude
templates. PostgreSQL/MySQL have read/write/transaction built-ins; Snowflake has
read-oriented built-ins only; Redis has none. Custom text is still subject to
normal Rust execution policy when explicitly run. Automatic context-aware
template variables and complete semantic snippet placement remain open.

SQLite stores at most 128 custom snippets, with ASCII identifier triggers up to
32 characters, titles up to 120 characters, bodies up to 16 KiB and a 512 KiB
encoded collection bound on writes. Duplicated scoped triggers and invalid
records are rejected. Snippets/settings are included in validated metadata
backup/restore. Explicitly saved snippets may contain SQL text even when query
history/draft opt-ins are off; the UI explains this local storage behavior.

| State | Evidence for this source |
|---|---|
| Reproduced and fixed | A selected formatter reply survived an edit elsewhere; rapid settings saves reached IPC concurrently. New tests failed before the document-identity and write-queue fixes. MySQL rejected named binds and Snowflake rejected question binds before formatter configuration was corrected |
| Passed | `pnpm validate`: 194 frontend tests / 33 files, 155 standard Rust tests, contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Browser | All 86 local browser flows pass on fixed source, using disposable PostgreSQL 18, MySQL 8.4, MariaDB 11.4 and Redis 8.2 plus simulated Snowflake. Three new flows cover selection boundaries/undo, custom template expansion and Tab/Shift+Tab, explicit execution, formatted bind review, reload persistence, edit and deletion. Existing completion, files, sessions, navigation, plans, recovery, grid and data movement flows also pass |
| Native | Clean-source macOS arm64 hidden build records this code/tree (`sourceDirty: false`); three fresh WebView processes / 30 cleanup cycles pass, including settings/snippet persistence and command guards. Fixture SHA-256 `691c9acf4644b2295f644c5c47525762d44caec7534a8599083b3d77a21d6e55`. Synthetic IPC/worker baseline only; native Preferences/selection UI, total memory, cold visible startup and display FPS remain unqualified |
| Local only | `feat/estimated-query-plans`, unmerged, unreleased and not installed; fetched `origin/main` remains `a0ba1a6`. No application push, merge, release or installation |
| Prior evidence | The 58 ignored SQL fixture passes belong to `39359e7`; ignored SQL/Redis/SSH suites were not rerun for this slice. Existing distribution-bundle evidence remains specific to its recorded revision |
| Open | Per-dialect formatting overrides, richer template variables, complete semantic binding/projection inference, secondary-session Structure navigation, live external qualification, normal distribution bundle, Windows/Linux runtime, signing/notarization, accessibility and representative performance |
| Next | Qualify the combined source with full ignored local fixtures and a normal distribution build; investigate the earlier metadata-startup flake if it recurs. Preserve R6 authorization/exact origins, then continue the remaining R5/R6 gaps and R7/R8. Merge, release and installation remain separate gates |

Logs: `.cache/r5-format-validate-exact.log`,
`.cache/r5-format-browser-exact.log` and
`.cache/r5-format-native-{build,runs}.log`. Immutable native evidence is
`.cache/r5-format-native-evidence.json`. No hooks or policy gates were bypassed.
The earlier metadata-startup flake did not recur in this full browser run; its
cause remains open. Disposable services are task-owned; user services and the
installed application were preserved.

## Combined local qualification (2 November 2026)

Clean candidate `b80b6c72d961081ad962ab12ee0655625638498e`, tree `12bf0aceaa9257ba2105ab7cb9a776b3794d5c19`,
passed all 66 explicitly selected ignored fixtures: 29 PostgreSQL, 29
MySQL/MariaDB (including both identifier-folding modes), six Redis and two SSH.
These use disposable PostgreSQL 18, MySQL 8.4, MariaDB 11.4 and Redis 8.2.
This supersedes the earlier 58-fixture qualification for the combined candidate;
it does not qualify external accounts or additional engine versions.

`pnpm desktop:bundle:mac` built the normal macOS arm64 `Joinery.app` and passed
strict local ad-hoc signature verification. Bundle executable SHA-256:
`6cb4a716a616f4bc985a93de9b3dd33cc2644b7f22914dcf7cb60c4e7c695b9f`.
The app was not launched, installed, published, Developer ID signed or notarized.
The existing frontend chunk-size warning remains. Logs:
`.cache/r5-r6-combined-{fixtures,bundle}.log`; immutable provenance:
`.cache/r5-r6-combined-evidence.json`. These checks precede the style follow-up
below and must not be attributed to its later source revision.

## R5 follow-up: per-engine formatting styles (2 November 2026)

Code `d39b89b024acbe69f329022082788a376adc42c3`, tree `e7fd7bfb8d979322dcbd1388d20a02f3135068a0`, adds independent PostgreSQL,
MySQL/MariaDB and Snowflake formatting styles. Each inherits global defaults
until its first customization, which snapshots the three global fields. Later
global changes leave custom styles intact. **Use global defaults** removes that
override and resumes inheritance; removing the last override returns to the
legacy settings shape. The active console's dialect selects the formatter style;
an unbound console retains its existing PostgreSQL fallback. Redis has no SQL
formatter. Case, indentation and query spacing remain the supported style fields.

Rust retains backward-compatible reading of legacy global settings, validates
every override, rejects unsupported keys and includes styles in validated
metadata backup/restore. Ordered optimistic saves, visible errors and rollback
also cover customization/reset across scope switches. No query execution or R6
authorization behavior changed.

| State | Evidence for this source |
|---|---|
| Reproduced and fixed | A dialect-style regression failed before implementation because PostgreSQL used global settings. It now verifies independent PostgreSQL, MySQL and Snowflake formatting |
| Passed | `pnpm validate`: 196 frontend tests / 33 files, 156 standard Rust tests, contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Browser | Three expanded PostgreSQL/MySQL/MariaDB editor flows pass: persisted global/custom styles, explicit console binding after reload, automatic dialect selection and reset to global defaults, alongside selection/undo, snippets and parameter review. An initial test incorrectly used an unbound console after reload; it was corrected to select the test connection. These are three relevant flows on this source; the full 86-flow run belongs to `db5cd1b` |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`. Three fresh WebView processes / 30 cleanup cycles pass, including all three dialect styles through SQLite save/bootstrap and a PostgreSQL formatter-worker check. Fixture SHA-256 `883ef1029e3d3c2bf9d17e406e6fecdf8c2d15ad5a2e935f1791057b127dd961`. Synthetic IPC/worker/cleanup baseline only, not native Preferences UI or representative performance |
| Prior qualification | All 66 ignored fixtures and the normal ad-hoc distribution bundle passed on `b80b6c7` immediately before this follow-up; they were not rerun on this later source |
| Open | Broader style options, context-aware template variables, semantic/projection inference, secondary-session Structure navigation, observed Snowflake identity/live account qualification, native UI/accessibility, representative performance, Windows/Linux runtime, Developer ID signing/notarization and release/install gates |
| Next | Continue bounded semantic/projection inference and secondary-session metadata/navigation; preserve conservative ambiguity handling and exact R6 origins. Then implement the remaining R5/R6 and R7/R8 milestones with disposable fixtures. Application push, merge, release and installation remain separate authorized steps |

Logs: `.cache/r5-dialect-validate.log`, `.cache/r5-dialect-browser-exact.log`,
`.cache/r5-dialect-native-{build,runs}.log`. Immutable native evidence:
`.cache/r5-dialect-native-evidence.json`. Local, unmerged, unreleased and not
installed. No hooks or policy gates were bypassed. The earlier metadata-startup
flake did not recur in these three browser flows; its cause remains open.

## R5 follow-up: named query outputs (2 November 2026)

Code `c32f0c2da1740b1b05347efcb986ca7170d46ba2`, tree `ef633488c3cbb5cd5a129bff6b0e6df0c89ab828`, resolves directly named
SELECT columns through CTEs and derived tables. F12 selects the local output
declaration; Shift+F12 lists references belonging to that declaration; F1 shows
the proven source column and source type. Explicit full CTE column lists rename
outputs authoritatively. Nested CTEs, aliases, quoted names and engine-specific
column casing retain scoped identities. Source primary keys/nullability are not
asserted as constraints on a derived result.

Metadata dependencies now follow query bodies before the existing eight-schema
fetch limit. Per-action inference is memoized and bounded to 16 levels, 128 query
bodies, 256 outputs per body and 2,048 SELECT-list tokens. Existing 500-usage /
10,000-name-token bounds and edit/context/session stale-result guards remain.
Unknown and duplicate names are withheld; local bindings shadow catalog names.
Expression outputs, wildcards, set-operation outputs, table-valued functions,
partial CTE column lists and relation alias-column lists remain unsupported.
An unsupported output shape conservatively withholds that relation's inferred
outputs. No editor SQL is executed by navigation; R6 policy/origins are unchanged.

| State | Evidence for this source |
|---|---|
| Reproduced and fixed | `recent.order_id` returned no symbol before inference despite `SELECT o.id AS order_id FROM orders o`; the recorded regression now resolves its local declaration, source and three occurrences |
| Passed | `pnpm validate`: 228 frontend tests / 33 files, 156 standard Rust tests (135 core plus 21 other checks), contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Focused regressions | 31 resolver cases cover direct/renamed/nested outputs, explicit lists, local declarations, duplicate/shadowed names, unknown metadata, quoted/case behavior, unsupported shapes and bounded recursion. An overlay regression labels source metadata without implying output constraints. An initial combined run exposed missing DOM cleanup between overlay tests; cleanup is now explicit and the complete run passes |
| Browser | All five SQL navigation flows pass on disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4. Three expanded flows cover inferred CTE outputs, explicit CTE lists and derived-table outputs with docs/usages/local definition jumps and no execution requests. A delayed source-schema response after an edit is discarded; metadata-denial/refresh and isolated-console protections still pass. Zero page errors |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`; three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `5292adff4fb9c1714db98366fa396d4543440bfc7cd16f3eba31b649043666af`; isolated ad-hoc bundle executable SHA-256 `93972d0ec65d9313cbc2646e5241789e38863076626a3ffa468311cd93f7f504`. Existing synthetic IPC/worker/cleanup baseline only; the new navigation interactions are browser-tested, not native UI-qualified |
| Prior qualification | Full 86-flow browser suite belongs to `db5cd1b`. All 66 ignored SQL/Redis/SSH fixtures and the normal ad-hoc distribution bundle belong to `b80b6c7`; those broader checks were not rerun on this later source. Per-engine style checks remain recorded on `d39b89b` |
| Open | Broader semantic analysis/projection shapes, secondary-session Structure navigation, context-aware templates/style controls, observed Snowflake identity/live account qualification, native UI/accessibility, representative performance, Windows/Linux runtime, Developer ID signing/notarization and release/install gates |
| Next | Route Structure navigation through the originating console's session and context, with cache/stale-result isolation and disposable browser coverage, as the next bounded slice. Then continue remaining R5/R6 and R7/R8. Application push, merge, release and installation remain separate authorized steps |

Logs: `.cache/r5-projection-validate.log`, `.cache/r5-projection-browser.log`,
`.cache/r5-projection-native-{build,runs}.log`. Immutable native evidence:
`.cache/r5-projection-native-evidence.json`. Local, unmerged, unreleased and not
installed. No hooks or authorization gates were bypassed. The earlier metadata-
startup flake did not recur in these five browser flows; its cause remains open.

## R5 follow-up: console-bound Structure (2 November 2026)

Code `fe518fb5554f9975a29a878fae1eccfc08a1ba52`, tree `24dcdd37bfaef069c643ee2fc7d6e1df36da4121`, enables object/column Structure
navigation from primary and isolated SQL consoles. Inspectors retain source tab,
connection, session, frontend context epoch and the observed catalog revision.
Tab identity includes that origin; reopening a matching target refreshes metadata
and column highlighting. Source closure, session rebinding/disconnect, reconnect
and context changes never substitute the primary session. Inspectors are not
session owners, workspace drafts, bookmarks or connection-level recent locations.

Metadata is read from the originating session. The catalog revision is checked
before and after schema/details requests; column lookup uses the revision-checked
completion schema path. Result identity includes tab, session, context, target
and refresh revision. Cleanup and current-source guards reject obsolete replies.
Extended-detail denial retains successful columns. Capabilities still gate
extended object inspection. Local CTE/alias/output jumps and R6 execution origins
are unchanged. Navigation makes no execution/preparation or table-browse request.

These inspectors offer read-only Structure and DDL copying. Existing dependency,
owner/index/trigger click-through and **Open in SQL editor** actions are withheld
in this new path because their older actions select the primary session. The
normal explorer retains its existing behavior. Context-preserving metadata links
and explicit DDL-to-console insertion are the next bounded follow-up.

| State | Evidence for this source |
|---|---|
| Reproduced and fixed | A store regression initially failed because no session-aware object-opening action existed; a second regression reproduced the unbound DDL editor action and now confirms it is withheld; the former isolated-console browser test explicitly expected the withheld F12 jump. It now opens a highlighted inspector without primary table browsing |
| Passed | `pnpm validate`: 237 frontend tests / 34 files, 156 standard Rust tests (135 core plus 21 other checks), contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Focused regressions | Two store cases and seven inspector cases verify identity/deduplication, no data browsing, denied details, observed context changes before/during reads, late replies after closure/rebinding/context change and no primary fallback |
| Browser | All six SQL navigation flows pass on disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4. The new PostgreSQL flow opens distinct Structure/DDL inspectors from two isolated sessions plus the primary console, asserts each details request's session ID and sees no browse/prepare/execute requests. Existing named-output/local-jump, stale metadata and permission flows pass; zero page errors in the instrumented engine and three-session flows. Async selection assertions now wait for navigation completion. Earlier in-progress runs were interrupted by source/contract regeneration; the final unchanged-source run passes |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`; three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `fbf89916efb0e158bc010f0f9203bcbd4b6a9a37b945c3ffd98c4a96fd74a7b8`; isolated ad-hoc bundle executable SHA-256 `99092751829fa9b95f4c4a5eccf6d3081e65a51f34c2ce31d878becf39aea1ca`. Existing synthetic IPC/worker/cleanup baseline only; Structure interaction evidence is browser-based, not native UI qualification |
| Prior qualification | Full 86-flow browser suite belongs to `db5cd1b`. All 66 ignored SQL/Redis/SSH fixtures and the normal ad-hoc distribution bundle belong to `b80b6c7`; those broader checks were not rerun on this later source. Named-output and style evidence retain `c32f0c2` and `d39b89b` |
| Open | Context-preserving inspector links/DDL-to-console insertion, broader semantic analysis/projection shapes, context-aware templates/style controls, observed Snowflake identity/live account qualification, native UI/accessibility, representative performance, Windows/Linux runtime, Developer ID signing/notarization and release/install gates |
| Next | Preserve the inspector origin through dependency/owner/index/trigger navigation and explicit DDL insertion into a still-bound source console. Then continue bounded SQL refactoring/semantics and remaining R5/R6/R7/R8. Application push, merge, release and installation remain separate authorized steps |

Logs: `.cache/r5-console-structure-validate.log`,
`.cache/r5-console-structure-browser.log`,
`.cache/r5-console-structure-native-{build,runs}.log`. Immutable native evidence:
`.cache/r5-console-structure-native-evidence.json`. Local, unmerged, unreleased
and not installed. Owned database fixtures were stopped and removed; user
services were preserved. No hooks or execution gates were bypassed. The earlier
metadata-startup flake did not recur in these six flows; its cause remains open.
A fresh `origin/main` fetch remains `a0ba1a6`. The live GitHub release refresh was
blocked by automatic approval policy, so preview.15 remains **last verified**;
the installed application was not requalified.

## R5 follow-up: context-preserving inspector actions (2 November 2026)

Code `34c001a57019619a1da9db215e117c3eb16b15f7`, tree `bd6ca32dff2965ff8e4905cd2117e8d1906177cf`, continues console-bound Structure
through foreign-key, index, trigger, dependency and owner-table links. Every
linked inspector retains the frozen source tab, session, frontend context epoch
and observed catalog revision. It never owns a session, browses table data or
substitutes the primary session. Normal explorer navigation retains its routing.

**Append DDL to source console** checks the current catalog before explicitly
appending the displayed DDL after existing SQL and returning to the source tab.
It preserves the file token/name, saved baseline/dirty tracking, retained results
and query-session ownership. No preparation or execution request is made. Empty
DDL, a closed/rebound/disconnected/context-changed source, a queued/running
console, denied metadata, a changed catalog or an obsolete inspector action
cannot append. Current-origin checks run before and after the asynchronous
catalog read; pending actions are serialized and unmounted/refreshed results
are discarded. Database execution still follows the existing Rust approvals.

| State | Evidence for this source |
|---|---|
| Reproduced and fixed | Five new store regressions initially failed because linked-session navigation and source DDL append actions did not exist. They now verify preserved SQL/file dirty tracking/session ownership and rejected invalid/busy sources |
| Passed | `pnpm validate`: 253 frontend tests / 34 files, 156 standard Rust tests, contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy. The final browser-only assertion fixes also pass their formatting check |
| Focused regressions | 16 additional frontend cases cover linked target identity, all five link types, source closure/rebinding/context changes, denied/changed catalog replies, inspector refresh and DDL append without SQL loss or execution |
| Browser | All six SQL navigation flows pass on fresh disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4. The expanded PostgreSQL flow follows index → owner table → trigger → function, follows a foreign key and appends exact DDL back to each of two isolated source consoles; the primary console also appends to itself. Each linked details request uses the originating session, all three retain existing SQL/session selection, and the flow sees no browse/prepare/execute requests or page errors. Test assertions restore the caret after editor remount, await the intended object's request and compare CodeMirror lines with newlines preserved |
| Fixture | The PostgreSQL trigger/function chain is initialized by the tracked disposable fixture. The existing ignored object-explorer regression also passes against this database, covering overloaded identities, DDL, dependencies and permission limits; this is one additional fixture check, not a rerun of all 66 |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`; three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `199fcfc2c361e5e4ff912d4ddec5d9a083e5a79ca79317ee2833624d53732a94`; isolated ad-hoc bundle executable SHA-256 `58c8282b7492c9a22d75ff8cdb7f46ce0b39db7943d1de1915626ff4887a30c0`. This remains the synthetic IPC/worker/cleanup baseline; new inspector interaction evidence is browser-based, not native UI qualification |
| Prior qualification | Full 86-flow browser suite belongs to `db5cd1b`. All 66 ignored SQL/Redis/SSH fixtures and the normal ad-hoc distribution bundle belong to `b80b6c7`; those broader checks were not rerun on this source. Earlier formatting, named-output and initial console Structure evidence retains its revisions |
| Open | Scope-correct refactoring, broader semantic analysis/projection shapes, context-aware templates/style controls, observed Snowflake identity/live account qualification, native UI/accessibility, representative performance, Windows/Linux runtime, Developer ID signing/notarization and release/install gates |
| Next | Bounded alias rename preview (E10): prove the declaration/reference scope, preview edits and require explicit apply; preserve quoting/case and reject ambiguous/unsupported or stale document/session/context cases. One undo must restore the original text. Never rename a database object or execute SQL. Then continue remaining R5/R6/R7/R8 |

Logs: `.cache/r5-inspector-actions-validate.log`,
`.cache/r5-inspector-actions-browser.log`,
`.cache/r5-inspector-actions-postgres-inspector.log`,
`.cache/r5-inspector-actions-native-{build,runs}.log`. Immutable native evidence:
`.cache/r5-inspector-actions-native-evidence.json`. Local, unmerged, unreleased
and not installed. Owned database fixtures were stopped and removed; user
services were preserved. No hooks or execution gates were bypassed. The earlier
metadata-startup flake did not recur in these six flows; its cause remains open.
The fetched `origin/main` remains `a0ba1a6`. Release metadata was not refreshed
in this slice; preview.15 remains last verified, and the installed application
was not requalified. Application push, merge, release and installation remain
separate authorized steps.

## R5 follow-up: bounded alias rename (2 November 2026)

Code `e0526268fab3f9162b3bd890b1484a3d90b127db`, tree `2698b041469a6dfa2adab8ea464260367bee9f97`, adds **Rename alias** / Shift-F6.
The dialog lists every local declaration/qualified-reference edit, requires
explicit Apply, preserves quoted identifiers with escaped delimiters and applies
one isolated CodeMirror transaction. One undo restores the original SQL.
Comments, string literals, other statements and nested shadowing are preserved;
proven correlated references follow their declaration. CTE bodies and non-lateral
derived tables cannot adopt their containing query's FROM aliases. Renaming
neither changes a database object nor prepares/executes SQL. Run shortcuts are
isolated while the dialog owns keyboard focus.

This independent analysis never uses the capped navigation usages as edit proof.
It rejects name collisions, captured/unbound qualifiers, ambiguous whole-row/bare
names, stale document identity (even after restoring identical text), editor/session/
context/catalog changes, invalid or overlong names and partial scans. Bounds:
65,536 UTF-16 units per statement / 4,096 lexical tokens / 16 nested levels / 64 query bodies /
500 edits. New names stop at 128 UTF-16 units, with PostgreSQL's 63-byte and
MySQL/MariaDB's 64-character bounds also enforced. MySQL alias case checks are
conservative. SELECT/WITH with physical, CTE and SELECT-derived aliases is the
supported subset; DML/data-modifying CTEs, set operations, LATERAL/APPLY, table
functions, relation column lists, parenthesized joins and other unsupported
relation modifiers are withheld. Nested multi-relation queries that hide an outer
name are also withheld: earlier JOIN ON clauses have different visibility from
the complete query. The SQL lexer is not a complete semantic SQL validator.

| State | Evidence for this source |
|---|---|
| Reproduced | Missing rename analysis failed the initial behavior tests. A later dedicated regression reproduced unsafe whole-query name resolution for an earlier JOIN ON clause followed by a shadowing alias; this shape is now explicitly withheld |
| Passed | `pnpm validate`: 277 frontend tests / 35 files, 156 standard Rust tests, contracts, types, production frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Focused unit/UI | 24 new cases cover full edit lists, shadowing/correlation, CTE/derived boundaries, capture/collisions, quoted delimiters across PostgreSQL/MySQL/Snowflake dialects, identifier limits, unsupported shapes, stale documents, explicit Apply/cancel and dialog shortcut isolation |
| Browser | All nine focused flows pass against disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4: three new rename workflows plus all six SQL navigation regressions. Preview/cancel/apply, original SQL preservation, one-step undo, escaped quoted aliases, capture/unsupported rejection and document/context refresh invalidation pass, with no prepare/execute requests or page errors. A Vite module import drives a background document-change regression; the editor is correctly inert/hidden during preview |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`; three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `7410d2059eb94aa4c78cb1712305c1fea6995ba070389e989834a4c5a9209040`; isolated ad-hoc bundle executable SHA-256 `1f7374dcfb4748f785ffb792b1fbfd32148992d900c8cad0ba1ec863fee907e0`. Synthetic IPC/worker/cleanup baseline only; rename interaction evidence is browser-based, not native UI qualification |
| Prior evidence | The single PostgreSQL inspector fixture belongs to `34c001a`. The full 86-flow browser run belongs to `db5cd1b`; all 66 ignored fixtures and normal ad-hoc distribution bundle belong to `b80b6c7`. None is relabeled as rerun here. Snowflake rename checks are local dialect tests, not live account qualification |
| Open | Broader refactoring and semantic inspections/templates; R5/R6 depth, R7/R8, observed Snowflake identity/live qualification, native UI/accessibility, representative performance/recovery, Windows/Linux runtime, signing/notarization and release/install gates |
| Next | Model ordered JOIN visibility for alias rename, reproduce earlier-ON correlated/shadowing cases, and enable only proven scopes with the same preview/capture/staleness/undo guarantees. Then continue bounded CTE rename, semantic inspections/templates and remaining R5/R6/R7/R8 |

Logs: `.cache/r5-alias-rename-{reproduction,join-reproduction,validate,browser,native-build,native-runs}.log`.
Immutable native evidence: `.cache/r5-alias-rename-native-evidence.json`.
Initial browser assertions were corrected to inspect the modal-hidden editor via
its DOM locator. A development-page reload interrupted one intermediate run;
final focused runs pass. No known test failure was bypassed. Owned fixtures were
removed; user services remain running. Fetched `origin/main` is `a0ba1a6`.
Release metadata was not refreshed in this slice; preview.15 remains last verified.
Local, unmerged, unreleased and not installed; application push/merge/release/
installation remain separate authorized steps.

## R5 follow-up: ordered JOIN alias visibility (2 November 2026)

Code `1edc2d748d7fcac4bf59875249cbfecee82a1ce4`, tree `6b753a39e325f072a004cbe7b5648df945276be1`, removes the blanket rejection
of nested multi-relation queries that shadow an outer alias. Each ON clause now
owns the actual JOIN operands, including its correlated expression/subquery
children. Later aliases cannot hide earlier outer references; earlier comma
groups stay outside another JOIN's condition. A pending-JOIN stack preserves
`a JOIN b JOIN c ON ... ON ...` association. Completed CROSS/NATURAL/USING joins
retain their operands, and MySQL CROSS JOIN ON/USING conditions remain supported.
SELECT/WHERE/ORDER clauses retain the complete query scope. Proposed-name capture
checks use these same environments; later local aliases keep their own uses.

The existing explicit preview/Apply, dialect quoting, complete edit list,
identifier/statement/token/nesting/edit limits, stale document/session/context
rejection and isolated one-step undo remain in force. Parenthesized joined tables,
LATERAL, DML/set operations, table functions, relation-column lists and unsupported
modifiers remain withheld; PostgreSQL USING-column aliases are explicitly rejected.
This remains a bounded lexical analysis, not a full SQL semantic validator.

| State | Evidence for this source |
|---|---|
| Reproduced | Ten new failures on the previous source cover earlier-ON shadowing, comma-group visibility, nested JOIN association and unsupported USING-column aliases |
| Passed | `pnpm validate`: 290 frontend tests / 35 files, 156 standard Rust tests, contracts, types, frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Focused unit | Thirteen additional cases (91 navigation tests total) cover outer/later-local rename, capture/unbound qualifiers, comma groups, nested JOINs, completed CROSS/NATURAL/USING operands, MySQL CROSS conditions and local Snowflake dialect quoting/case |
| Browser | Twelve focused PostgreSQL 18 / MySQL 8.4 / MariaDB 11.4 flows pass: all nine previous rename/navigation flows plus three new ordered-JOIN workflows. Each new workflow checks three query shapes, explicit before/after test reads returning one row, exact preserved edits, capture rejection, zero execution requests during rename, and undo. No page errors |
| Timing regression | The first PostgreSQL browser run opened preview after result arrival but before final query/session events; that legitimate context transition invalidated it. The test now waits for completed-query UI and refreshed context. The final twelve-flow run passes; staleness guards remain enabled |
| Prior evidence | Native macOS hidden baseline belongs to `e052626`: three fresh processes / 30 cleanup cycles, synthetic IPC/worker/cleanup only. Inspector fixture: `34c001a`. Full 86-flow browser suite: `db5cd1b`. All 66 ignored SQL/Redis/SSH fixtures and normal ad-hoc bundle: `b80b6c7`. None was rerun or relabeled as current qualification |
| Open | Bounded CTE rename, semantic inspections/templates and remaining R5/R6/R7/R8; observed Snowflake identity/live qualification, native UI/accessibility, representative performance/recovery, Windows/Linux runtime, signing/notarization and release/install gates |
| Next | Prove declaration/relation scope for bounded non-recursive CTE rename, including ordered WITH entries, nested shadowing, implicit qualifiers and proposed-name capture, with the same complete preview/context/undo guarantees |

Logs: `.cache/r5-ordered-join-{before,unit,validate,browser,fixtures}.log`.
Direct disposable-engine checks independently confirmed the earlier-ON,
comma-group, nested JOIN and completed CROSS association used by the tests.
Reference semantics: [PostgreSQL SELECT](https://www.postgresql.org/docs/18/sql-select.html)
and [MySQL 8.4 JOIN](https://dev.mysql.com/doc/refman/8.4/en/join.html).
Snowflake remains local dialect evidence only. No external account was queried.
Fetched `origin/main` is still `a0ba1a6`; preview.15 remains the last verified
published application release, without a fresh release metadata check here.
Local, unmerged, unreleased and not installed. No application push, merge,
release or installation occurred.

## R5 follow-up: bounded CTE rename (2 November 2026)

Code `072e4ff3c9ce57988ce16c59f2dd4fd87aa2f78a`, tree `1f4ebf3f4327eec3adee933e0f7b2cfa032568bc`, extends
Shift-F6 / **Rename alias / CTE** to nonrecursive SELECT CTEs. Declaration,
unqualified FROM/JOIN relation uses and implicit qualified references share a
scope identity. Earlier WITH entries become visible after their body; later
siblings cannot capture an earlier body. Explicit aliases remain independent;
schema-qualified physical tables, comments, strings and other statements are
preserved. PostgreSQL nested WITH shadowing, explicit output lists and
MATERIALIZED / NOT MATERIALIZED are covered. Quoted identifiers retain escaped
delimiters. Proposed-name simulation checks every CTE/physical relation use and
every relevant qualifier, including unresolved names, for changed binding.

Preview lists the entire edit set and requires explicit Apply. One isolated
transaction gives one-step undo; stale document/editor/session/context/catalog
identity is rejected. Dialog run shortcuts cannot execute. This changes SQL text,
never database objects or authorization. Existing bounds remain 65,536 UTF-16
statement units / 4,096 tokens / 16 levels / 64 query bodies / 500 edits, with
128-unit replacement names plus PostgreSQL 63-byte / MySQL-family 64-character
limits. Recursive WITH, DML, set operations, whole-row/bare-name ambiguity,
LATERAL/APPLY, functions, alias-column lists, parenthesized joins and unsupported
modifiers remain withheld. This is bounded lexical analysis, not full SQL validation.

**Engine boundary reproduced:** PostgreSQL 18 and MySQL 8.4 execute the outer-CTE
read in an inner WITH definition; MariaDB 11.4 reports missing table `src`.
This occurs both with a later same-name inner CTE and with a distinct inner name.
The shared MySQL/MariaDB editor dialect therefore withholds nested WITH beneath
an outer CTE scope for both alias and CTE rename, even for some valid MySQL SQL.
It retains ordered top-level CTE entries and nested SELECT subqueries. The
browser suite explicitly asserts both unsupported forms offer no rename and send
no execution request. No backend-version detection or broader compatibility is claimed.

| State | Evidence for this source |
|---|---|
| Reproduced | Initial missing-feature suite: 20 failures / 98 passes. Two intermediate browser runs exposed the MariaDB boundary above; direct CLI probes confirmed it. The unsupported scope is now rejected, rather than applying edits under another engine's resolution rules |
| Passed | `pnpm validate`: 327 frontend tests / 35 files, 156 standard Rust tests, contracts, types, frontend build, formatting, compilation and strict all-target/all-feature Clippy |
| Focused unit/UI | 37 additional cases: 36 analyzer cases plus the CTE dialog case. Ordered WITH visibility, declaration/relation/qualifier entry points, explicit aliases, physical relations, nested PostgreSQL shadowing, capture/unresolved names, quoting/limits/stale documents, unsupported shared-MySQL scopes, recursive/DML/set/whole-row/LATERAL rejection and complete edit bounds |
| Browser | All fifteen focused flows pass on disposable PostgreSQL 18 / MySQL 8.4 / MariaDB 11.4. Three new CTE flows each compare three original/renamed query pairs through decoded Arrow values, including exact `9007199254740992` / `9007199254740993` text on MySQL-family fixtures. Preview/cancel/Apply/one undo, quoting, sibling/alias collision, shortcut isolation, document/context invalidation, unsupported/recursive rejection and zero requests during rename pass. All twelve previous alias/JOIN/navigation flows pass; zero page errors |
| Native | Clean-source macOS arm64 hidden build records this code/tree with `sourceDirty: false`; three fresh WebView processes / 30 cleanup cycles pass. Fixture SHA-256 `5e7efdfabcad243851d7b9d01b758e74c9ddf0917dfd0dc71f4441bd900697ec`; isolated ad-hoc bundle executable SHA-256 `d81323ba7b4c40522d427d995cf98da273bd11ff805755fd0167f713d4024814`. Synthetic IPC/worker/cleanup baseline only; CTE rename interaction evidence is browser-based, not native UI qualification |
| Prior evidence | Inspector fixture: `34c001a`. Full 86-flow browser suite: `db5cd1b`. All 66 ignored SQL/Redis/SSH fixtures and normal ad-hoc distribution bundle: `b80b6c7`. None is relabeled as rerun here. Snowflake checks are local dialect evidence only |
| Open | Broader refactoring, semantic inspections/templates and remaining R5/R6/R7/R8; observed Snowflake identity/live qualification, native UI/accessibility, representative performance/recovery, Windows/Linux runtime, signing/notarization and release/install gates |
| Next | Bounded on-demand missing qualified-column inspection using successful current-session direct-table schema metadata, with user-selected real-column correction, exact preview/Apply/stale checks/undo and no execution. Withhold ambiguous, denied, partial or unsupported metadata; then continue richer templates and remaining roadmap |

Logs: `.cache/r5-cte-rename-before.log`,
`.cache/r5-cte-rename-validate-qualified.log`,
`.cache/r5-cte-rename-browser-qualified.log`,
`.cache/r5-cte-rename-native-{build,runs}.log`.
Immutable native evidence: `.cache/r5-cte-rename-native-evidence.json`.
Reference semantics: [PostgreSQL SELECT](https://www.postgresql.org/docs/18/sql-select.html),
[MySQL 8.4 WITH](https://dev.mysql.com/doc/refman/8.4/en/with.html), and
[MariaDB WITH](https://mariadb.com/docs/server/reference/sql-statements/data-manipulation/selecting-data/common-table-expressions/with).
The precise MariaDB limitation above is fixture-derived, not inferred from those docs.
Owned fixtures were removed; user services were preserved. No external account
was queried. Fetched `origin/main` remains `a0ba1a6`; preview.15 is the last
verified release, without a fresh release/installed-version qualification here.
The earlier metadata-startup flake did not recur in fifteen focused flows; its
cause remains open. Local, unmerged, unreleased and not installed. No application
push, merge, release or installation occurred.
