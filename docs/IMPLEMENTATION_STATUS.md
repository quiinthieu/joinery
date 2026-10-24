# PostgreSQL implementation checkpoint

Updated 2026-10-24. This checkpoint implements the PostgreSQL studio and SSH portion of the approved roadmap. It is a local macOS development build; it is not the complete multi-engine v1.0 release.

The current development branch also contains a **Snowflake read-only SQL pilot**:
system-browser OAuth with PKCE, explicit account context, async SQL API polling,
bounded JSON partitions, exact text values, own-query cancellation, and a
read-only connection form. Mocked callback and SQL API tests, the native macOS
build, and a headless Snowflake form check pass. A real Snowflake account canary
is still required before calling this pilot
usable against production. Catalog/table browsing and the remaining P6 criteria
are not complete. The published preview remains PostgreSQL-only.

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
3. Run the configured Windows/Linux/macOS CI matrix and platform UI smoke checks. Only macOS was executed in this workspace.
4. Add Developer ID signing/notarization and release/update recovery. The delivered local macOS bundle uses ad-hoc signing.
5. Continue P4–P6: MySQL/MariaDB parity, Redis console/key browser, and Snowflake catalog, live OAuth qualification, result/cancellation edge cases, and read-only grid browsing. The Snowflake development pilot is not part of the published preview.
6. Complete P7 accessibility, cross-platform releases, crash/upgrade recovery, and sustained performance qualification.

The approved blueprint remains in [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md) and the [recorded ChatGPT Page](https://chatgpt.com/space/page_c4b9e62285c481918eec36b706cca3a7).
