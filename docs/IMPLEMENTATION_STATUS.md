# Implementation and release status

Updated 2026-10-24. Release scope for `v0.1.0-preview.3`: the PostgreSQL studio,
SSH support, and read-only Snowflake SQL/catalog/table previews on macOS Apple Silicon. This remains
a development preview; full multi-engine v1.0 qualification is pending.

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
| Included in preview.3 | PostgreSQL studio, SSH/TLS, editing and review, Safe Mode, themes, history/audit, bounded grid/Arrow results |
| Included as a pilot in preview.3 | Snowflake browser OAuth, identity checks, read-only SQL results, async polling, own-query cancellation, catalog/column completion and bounded table previews |
| Pending | Broader Snowflake live table/value/cancellation/token qualification, MySQL/MariaDB, Redis, signing/notarization, cross-platform installers, full performance and recovery qualification |

## Implemented

| Area | Current behavior |
|---|---|
| Shell | Tauri v2 system WebView; React/TypeScript; lazy editor/grid/worker bundles; explicit command capabilities and CSP |
| Connections | Private local SQLite metadata; nested groups with inherited, immutable environments; protected production profiles; session passwords and optional native credential storage |
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

The later processes benefit from warm OS services and caches. These are integration-test timings, not visible first-paint measurements. WebKit helper memory is excluded; peak main-process figures are not total idle app RAM. The **50–80 MB idle target, fully cold sub-second startup target, and 100+ FPS target remain unqualified**. The million-row test verifies virtualization and scrolling, not a display-frame-rate guarantee. Per the roadmap, complete those measurements before adding broad engine scope.

The reproducible runner writes detailed local evidence to `.cache/native-smoke/runs.json` and identifies the exact test executable by SHA-256.

## Remaining release work

1. Qualify total idle memory including WebKit helpers, visible first paint, and frame times on representative hardware; address any regressions before P4.
2. Exercise native credential-store success/denial/locked-store flows. Current connection tests inject a memory store and never access personal credentials.
3. Run packaging and platform UI smoke checks on Windows and Linux as well as macOS. Only macOS was executed in this workspace; CI compilation does not qualify packaged app behavior.
4. Add Developer ID signing/notarization and release/update recovery. The delivered local macOS bundle uses ad-hoc signing.
5. Continue P4–P6: MySQL/MariaDB parity, Redis console/key browser, and broader Snowflake live table/OAuth/result/cancellation qualification. Preview.3 includes read-only catalog/grid browsing; preview.2 covered SQL only.
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

## Continuation batch B: runtime qualification (local, partially implemented)

The next branch adds test-only shell/loading/cycling/release checkpoints, source
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

These batch B additions are not part of preview.3. Complete helper attribution,
representative visible startup/frame-time qualification, real locked-store cases,
and sustained cleanup checks before adding MySQL/MariaDB and Redis scope.

The detailed blueprint remains in [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).
The private [ChatGPT Site](https://joinery-project-hub.otohime-nokimi.chatgpt.site)
is the canonical plan, implementation-progress, and shipping-status destination.
Update it after implementation, verification, merge, and release milestones;
the former plan and testing Pages are retired.
