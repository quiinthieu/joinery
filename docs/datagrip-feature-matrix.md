# DataGrip workflow comparison and Joinery evidence

Audit date: 2026-10-28. Baseline: `bbba42122df2c8303af5a275671144d9337a4454`.
This is an engineering gap inventory, not a shipping tracker. The owner-private
[project hub](https://joinery-project-hub.otohime-nokimi.chatgpt.site) remains the
canonical release record. Its version 18 source (`63dd98e3`) was matched to the
local hub checkout: preview.11 is the application release; later native-memory
instrumentation is source tooling. Installation and release assets were not
independently requalified in this audit.

## Method and verification scope

Compared source, generated DTOs, tests, CI, README, and the development blueprint
with official DataGrip **2026.2** documentation accessed on the audit date. The
graph project was `Users-billiesilvaner-orca-joinery`, generation
`2026-10-28T10:10:41Z`. Tier 2 queries, call traces, and coverage checks guided
discovery. Files marked metadata-changed were read directly. Generated IPC types
are intentionally excluded from the graph. Fixture SQL parse gaps (MySQL lines
8–10 and PostgreSQL line 27) were inspected directly. Graph coverage is a
best-effort signal, not proof that every construct was indexed.

Negative findings are bounded to the checked-in application surfaces, driver
trait, domain types, commands, and tests at this revision. Generic SQL access does
not count as a dedicated object editor, plan visualizer, or administration tool.
Library support alone does not establish an integrated workflow.

| Check | Outcome at baseline |
|---|---|
| `pnpm validate` | Passed with `.tools/env.sh`: 29 frontend tests, Rust tests, frontend build, formatting, native compilation and strict Clippy |
| Initial validation attempt | Blocked by shell configuration: Node 22 and Cargo absent from PATH; resolved using existing local Node 24/Rust toolchains |
| `pnpm test:e2e tests/e2e/snowflake.spec.ts` | Both synthetic workflows passed; first attempt was blocked before app load by Chromium macOS sandbox process registration |
| Disposable PostgreSQL/MySQL/MariaDB/Redis integration | Not executed in the initial audit; ignored tests in baseline validation are not passing integration evidence |
| Native credential-store qualification | Skipped by baseline, explicitly ignored |
| Native smoke, desktop build and macOS bundle | Not executed in the initial audit |
| Windows/Linux runtime, live Snowflake accounts, signing, performance targets | Not executed; remain external qualification gates |

`VERIFIED_COMPLETE` means the stated bounded acceptance workflow passed on the
recorded revision/environment, not DataGrip parity. `IMPLEMENTED_UNVERIFIED`
means an implementation exists but this audit did not run its complete workflow.
`PARTIAL` means a required behavior is absent or only a subset exists. `BROKEN`
requires a reproduced failure. `MISSING` means no integrated implementation was
found in the bounded source scope. `INTENTIONALLY_UNSUPPORTED` preserves an
explicit product restriction. Confidence H/M means high/medium source-evidence
confidence, independently of verification status.

Each row is a feature record. The final section records the implemented batch;
status rows reflect that bounded evidence, while the baseline table stays historical. The evidence keys below provide source files,
symbols, existing tests, observed implementation, and limitations. The last
column names additional required tests and release eligibility: **hold** means
the whole capability is incomplete/unverified; **existing** means consult the
hub for its existing release, without claiming new release qualification;
**excluded** means outside the supported scope. No new application release is
authorized by this document.

Engines: **P** PostgreSQL, **M** MySQL/MariaDB, **S** Snowflake, **R** Redis.
Security: **read** bounded metadata/results only; **gate** mutations must use
Rust plans/approval; **local** local privacy/persistence; **transport** TLS,
SSH and authentication; **engine** capability/privilege restrictions.
Complexity is Low/Medium/High. Priority P0–P3 is separate from blueprint milestones.

## Source and test evidence

| Key | Implementation and important symbols | Existing tests and observed limits |
|---|---|---|
| C | `src/features/connections/ConnectionDialog.tsx` (`ConnectionDialog`, `ConnectDialog`), `GroupManager.tsx`; `src-tauri/src/execution.rs` (`Studio.connect`, profile/group validation); `secrets.rs`, `ssh.rs` | ConnectionDialog/SshHostTrust tests, `src-tauri/tests/{groups,credential_store,ssh,snowflake_profile}.rs`, `tests/e2e/connections.spec.ts`. One bastion, explicit trust, OS secrets. Native locked-store and external auth qualification open |
| D | `src-tauri/src/drivers/{mod,postgres,mysql,redis,snowflake}.rs`, engine catalog modules; `domain.rs` (`EngineCapabilities`, `SessionInfo`, `TableSchema`, `ForeignKey`) | `src-tauri/tests/{postgres,mysql,redis,redis_transport}.rs`, Snowflake catalog/lifecycle/compression fixtures. SQL engines and Redis have distinct behavior; Snowflake is read-only. FK records are grouped constraints with ordinal-paired columns (table-data batch) |
| E | `src/features/studio/SqlEditor.tsx`, `sql-context.ts` (`selectedStatement`, `referencedTables`, `tableLookup`, `catalogCompletions`), `completion-session.ts` (`CompletionSession`, `CompletionSchemas`), `sql-identifiers.ts`; `src/data/{completion-index,completion-protocol,sql-identifier}.ts` (`CompletionWorkerIndex`), completion/formatter workers | Context/session/protocol/index unit tests and browser completion/studio/mysql/snowflake specs. Baseline regex defects reproduced and repaired; current token reader and `completion-session.ts` cover bounded direct-base-table scopes. Default context is configured, not live SQL-session context |
| X | `src-tauri/src/execution.rs` (`prepare_execution`, `store_plan`, `execute_plan`), `safety.rs` (`prepare`, AST visitor); `src/app/App.tsx` (`runSql`, `executePrepared`) | Safety unit tests and engine integration tests. Immutable expiring plans, one-use approval, cancellation quarantine, no replay. Plain EXPLAIN can use SQL; ANALYZE is not assumed read-only |
| G | `src/features/grid/{DatabaseGrid,ValueDialog,TableFilterBar,ColumnConfigPopover,CopyAsMenu,ExportDialog,ImportCsvDialog,DataCompareDialog}.tsx`, `{paste,table-layout,table-query,value-format,copy-format}.ts`; `src/data/{dirty-store,result-cache,columnar}.ts`; table paging in `src/state/studio.ts`; backend `browse_query.rs`, `file_capability.rs`, `export.rs`, `import.rs`, `compare.rs`, `prepare_changes_batch` and driver `apply` | Dirty/cache/columnar/paste/layout/viewer/copy-format/file-capability/export/import/compare tests and database/browser edit & data-movement tests. Glide canvas, sparse overlay, primary-key provenance, atomic insert/update/delete plans, typed server sort/filter, Safe-Mode-classified raw WHERE/ORDER BY, outbound & inbound FK navigation, persisted column hide/reorder/width, atomic multi-cell paste, binary hex/ASCII, PostgreSQL array & lossless JSON viewers (R1 batches 1–2, merged #25/#27), plus Copy-as (`CSV`/`TSV`/`JSON`/`NDJSON`/`SQL INSERT`/`Markdown`), capability-gated `.partial` streaming exports, CSV/TSV import validation/remapping into reviewed 500-row `ChangePlan` batches, and key-aware bounded table/snapshot comparison (R2, merged #28 `c8d8aa1`). No arbitrary-result editing, PK editing on existing rows, or Snowflake table sort/filter |
| N | `src/features/connections/{Sidebar.tsx,explorer-tree.ts}`, `src/features/studio/{UtilityViews,ObjectInspectorView,SearchEverywhere,LibraryModal}.tsx`, `search-everywhere.ts`; backend `drivers/{postgres,mysql}/explorer.rs`, `drivers/snowflake/catalog.rs` (`object_details`), `domain.rs` (`DatabaseObjectKind`, `DatabaseObjectRef`, `ObjectDetails`, `ObjectDdlStatus`, `DependencyEdge`, `SavedQuery`, `SavedBookmark`) | Explorer tree, object inspector, Search Everywhere unit/component tests, PG/MySQL/MariaDB integration tests (`object_explorer_*`), and `tests/e2e/explorer.spec.ts`. Categorized object explorer (`Tables`, `Views`, `Materialized views`, `Functions`, `Procedures`, `Sequences`, `Triggers`, `Indexes`) with WAI-ARIA keyboard tree traversal, kind filter chips, Structure constraints/indexes/triggers, bounded 256 KiB read-only DDL (`Copy DDL`, `Open in SQL editor`), bounded 200-edge `Uses`/`Used by` dependencies, and cross-connection Search Everywhere (`⌘K` / `⇧⌘O`, merged #29 `2625933`) plus `Saved` queries scope, bookmarks and recent locations (R4 local on `feat/consoles-sessions-files`). No DDL mutation UI (`N08`) or graphical execution plans (`A01`) |
| W | `src/state/{studio,workspace}.ts`, `src/app/{App,Workbench}.tsx`, `src/features/studio/LibraryModal.tsx`, `Preferences.tsx`; backend `persistence/{workspace,library}.rs`, `file_capability.rs` (`GrantKind::SqlFile`), `execution.rs` (`session_contexts`, `switch_session_context`, `set_transaction_mode`, `commit_transaction`, `rollback_transaction`, `open_sql_file`, `save_sql_file`) | Workspace/studio/library/file-capability unit tests, PG/MySQL/MariaDB transaction & context integration tests, and `tests/e2e/{sessions,consoles-files}.spec.ts`. Shared primary sessions plus named multi-console session pool (up to 4 per connection), per-session `Auto-commit`/`Manual commit` transaction controls, per-session schema/database switcher, capability-gated `.sql` file Open/Save/Save As (excluded from opt-in workspace drafts), saved queries library, persistent bookmarks (`⌘D`) and in-memory recent locations (`⇧⌘E`) (R4 local on `feat/consoles-sessions-files`) |
| L | `src-tauri/src/persistence.rs`, `persistence/{workspace,recovery}.rs`, `commands/recovery.rs`; `MetadataRecovery.tsx` | Persistence/recovery tests and `tests/e2e/recovery.spec.ts`. Version/schema guards, WAL crash recovery, bounded snapshots and undo. Does not restore results, sessions, edits or execute SQL |
| B | `src-tauri/src/results.rs` (`ResultStore`), `commands.rs` (`result_range`), `src/ipc/{client,generated}.ts`, Arrow worker, result cache | Result/cache/columnar unit tests; native smoke fixture. Rust 64 MiB, frontend 32 MiB/512 pages, dirty edits 8 MiB, result 100,000 rows, pages 500 rows/2 MiB, IPC 4 MiB. Generated DTOs but `call<T>(command: string, ...)` is not a closed command map |
| Q | `scripts/{native-smoke.py,runtime_memory.py,verify-macos-bundle.mjs}`, `src/native-smoke.ts`, `.github/workflows/ci.yml`, `docs/IMPLEMENTATION_STATUS.md` | Python attribution tests, hidden WebView fixture, standard three-platform CI compilation. Hidden DOM readiness and synthetic cleanup are not visible startup, total representative idle memory, or FPS qualification |

## Official DataGrip references

Reference links are the behavior benchmark, not a promise that every engine has
every feature. Do not copy JetBrains code, icons, or proprietary assets.

- [DG-C: data sources, drivers, authentication and SSH/SSL](https://www.jetbrains.com/help/datagrip/data-sources-and-drivers-dialog.html)
- [DG-S: sessions and client/session binding](https://www.jetbrains.com/help/datagrip/managing-connection-sessions.html)
- [DG-N: database explorer and object actions](https://www.jetbrains.com/help/datagrip/database-explorer.html)
- [DG-E: scoped completion and JOIN generation](https://www.jetbrains.com/help/datagrip/auto-completing-code.html)
- [DG-EC: search-path and qualification completion settings](https://www.jetbrains.com/help/datagrip/settings-code-completion.html)
- [DG-I: inspections](https://www.jetbrains.com/help/datagrip/code-inspection.html)
- [DG-F: SQL formatting](https://www.jetbrains.com/help/datagrip/configure-the-sql-code-style.html)
- [DG-R: refactoring and rename review](https://www.jetbrains.com/help/datagrip/refactoring-source-code.html)
- [DG-T: live templates](https://www.jetbrains.com/help/datagrip/using-live-templates.html)
- [DG-X: execution and script/playground resolution](https://www.jetbrains.com/help/datagrip/run-a-query.html)
- [DG-P: query parameters](https://www.jetbrains.com/help/datagrip/settings-tools-database-user-parameters.html)
- [DG-G: data editor](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html)
- [DG-V: rich value viewers](https://www.jetbrains.com/help/datagrip/tables-view-data.html)
- [DG-FK: foreign keys and navigation](https://www.jetbrains.com/help/datagrip/foreign-keys.html)
- [DG-IM: import](https://www.jetbrains.com/help/datagrip/import-data.html)
- [DG-EX: export](https://www.jetbrains.com/help/datagrip/export-data.html)
- [DG-D: database diagrams](https://www.jetbrains.com/help/datagrip/creating-diagrams.html)
- [DG-M: schema comparison and migration review](https://www.jetbrains.com/help/datagrip/schema-comparison-and-migration.html)
- [DG-Q: estimated/actual execution plans, tree/raw/graph views](https://www.jetbrains.com/help/datagrip/query-execution-plan.html)
- [DG-SE: object, code, data and global search](https://www.jetbrains.com/help/datagrip/search-in-ide.html)
- [DG-B: bookmarks](https://www.jetbrains.com/help/datagrip/bookmarks.html)
- [DG-RL: recent locations](https://www.jetbrains.com/help/datagrip/recent-files-and-changes.html)
- [DG-SQ: saved queries](https://www.jetbrains.com/help/datagrip/store-your-queries.html)
- [DG-DOC: context documentation](https://www.jetbrains.com/help/datagrip/documentation-tool-window.html)

## Database management

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
|---|---|---|---|---|---|---|
| M01 Connections and environment profiles — DG-C | C, D | IMPLEMENTED_UNVERIFIED / H | Existing forms/core; rerun connect/test/disconnect on fixture matrix | P0 / Medium | Drivers; PMSR; transport | Engine/browser integration / existing |
| M02 Authentication and credential storage — DG-C | C | PARTIAL / H | Password/keychain and Snowflake OAuth exist; broader auth and real locked-store cases need qualification | P1 / High | OS stores, engine auth; PMSR; transport | Native denial, token lifecycle and no-secret logging / hold |
| M03 SSH/TLS and engine security — DG-C | C, D | IMPLEMENTED_UNVERIFIED / H | One-hop tunnels and hostname verification exist; rerun CA/name/trust/drop/cleanup cases | P0 / High | Transport; PMR (S uses HTTPS); transport | SSH/engine integration / existing |
| M04 Hierarchical connection groups — DG-C | C | IMPLEMENTED_UNVERIFIED / H | Group policy and hierarchy exist; rerun UI moves and policy inheritance | P1 / Low | Profiles; PMSR; gate | Group/browser tests / existing |
| M05 Multiple databases/schemas — DG-C, DG-EC | `execution.rs` (`session_contexts`, `switch_session_context`), `drivers/{postgres,mysql,snowflake}.rs`, `SqlEditor.tsx` (`Active schema` / `Active database`) | VERIFIED_COMPLETE / H (bounded R4) | Per-session active schema switcher on PostgreSQL (`set_config('search_path', …, false)`, preserving explicit `pg_catalog`) and Snowflake (stateless request-scoped schema verification), plus active database switcher on MySQL/MariaDB (`USE`), bumping completion revisions and refreshing live CodeMirror completion immediately without affecting sibling sessions | P1 / High | Session context; PMS (`none` on R); read | Rust PG/Snowflake context unit tests, PG/MySQL/MariaDB live integration tests, and browser per-session schema/database switcher workflow; local on `feat/consoles-sessions-files` / hold until merged & released |
| M06 Session management — DG-S | `studio.ts` (`sessionPool`, `bindQuerySession`, `attachQuerySession`, `renameQuerySession`, `disconnectQuerySession`), `Workbench.tsx`, D, W | VERIFIED_COMPLETE / H (bounded R4) | Named & attachable multi-console session pool (up to 4 total sessions per connection, primary + 3 secondary): multiple consoles attach to a named secondary session or `Shared session`, rename/disconnect sessions, isolate cancellation, and disconnect a pooled session on tab close only when closing its last attached owner | P1 / High | Context identity; PM, restricted S/R; engine | `studio.test.ts` multi-console pool/last-owner/retirement tests, `tests/e2e/sessions.spec.ts` (PG/MySQL/MariaDB) and `tests/e2e/consoles-files.spec.ts` (PG/MySQL shared named session); local on `feat/consoles-sessions-files` / hold until merged & released |
| M07 Driver capabilities/configuration — DG-C | D | PARTIAL / H | Six capability booleans exist; finer object/analysis/auth capabilities need contracts | P2 / Medium | Driver trait/DTOs; PMSR; engine | Unsupported controls disabled and backend rejects / hold |

## Database navigation

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| N01 Hierarchical schema/table/view explorer — DG-N | `Sidebar.tsx`, `explorer-tree.ts`, `database_objects`, D | VERIFIED_COMPLETE / H (bounded R3) | Categorized schema tree (`Tables` incl. partitioned tables, `Views`, `Materialized views`, `Functions`, `Procedures`, `Sequences`, `Triggers`, `Indexes`) with WAI-ARIA keyboard traversal (`ArrowRight`/`ArrowLeft`/`ArrowDown`/`ArrowUp`/`Home`/`End`/`Enter`), roving `tabIndex`, and dedicated `ObjectInspectorView` tabs for non-tabular objects (`objectExplorer: false` on Redis) | P1 / Medium | Catalog; PMS; read | 3 `explorer-tree` Vitest cases, `studio` `openObject` deduplication tests, and browser keyboard tree + categorized explorer workflow; local on `feat/explorer-object-search` / hold until merged & released |
| N02 Object filtering/search — DG-N, DG-SE | `Sidebar.tsx`, `explorer-tree.ts`, `SearchEverywhere.tsx`, `search-everywhere.ts` | VERIFIED_COMPLETE / H (bounded R3) | Sidebar object-kind filter chips (`All`/`Tables`/`Views`/`Routines`/`Other`) + substring filter across schema/name/qualified `schema.name`/signature/owner table, paired with cross-connection Search Everywhere (`⇧⌘O` `Objects` scope) and explicit connected vs offline profile counts | P1 / Medium | Object index; PMS; read | `explorer-tree` & `search-everywhere` unit tests and browser PG filter chips + PG/MySQL + offline profile Search Everywhere workflows; local on `feat/explorer-object-search` / hold until merged & released |
| N03 Columns, PKs and FKs — DG-N, DG-FK | D, N | VERIFIED_COMPLETE / H (bounded R1 batches 1–2) | Outbound and permission-aware inbound FK constraints carry name and ordinal-paired columns; Structure lists constraints | P1 / Medium | Metadata DTOs; PM (S declares none); read | Outbound/inbound composite/multi-FK and unreadable-source fixture tests on PG/MySQL/MariaDB; merged in #25 (`6445b0e`) and #27 (`94f63ba`) / hold until released |
| N04 Indexes and constraints — DG-N | `drivers/{postgres,mysql}/explorer.rs`, `StructureView`, `ObjectInspectorView.tsx` | VERIFIED_COMPLETE / H (bounded R3) | Bounded `ConstraintInfo` (`PRIMARY KEY`/`FOREIGN KEY`/`UNIQUE`/`CHECK`/`EXCLUDE` with clickable FK targets) and `IndexInfo` (method, uniqueness, PG key expressions & partial `WHERE` predicates, MySQL prefix lengths & functional expressions) in `StructureView` and `Index` inspector tabs | P1 / Medium | Object metadata; PMS by capability; read | Rust index/DDL unit tests, `ObjectInspectorView` component tests, PG/MySQL/MariaDB fixture tests, and browser PG/MySQL index & constraint inspection flows; local on `feat/explorer-object-search` / hold until merged & released |
| N05 Triggers, functions, procedures, sequences — DG-N | `drivers/{postgres,mysql}/explorer.rs`, `database_objects`, `object_details`, `ObjectInspectorView.tsx` | VERIFIED_COMPLETE / H (bounded R3) | Bounded discovery (10,000 objects) and inspection of PG overloaded functions/procedures (`pg_get_function_identity_arguments`), triggers (`decode_pg_trigger_type`) and sequences (`pg_sequence` + privilege-checked `pg_sequences.last_value`), MySQL 8.4 routines/triggers/indexes, and MariaDB 11.4 routines/triggers/indexes/sequences (excluding `SEQUENCE` from base-table `catalog`) | P2 / High | Object identity/capabilities; PMS; read | Rust PG trigger/DDL unit tests, PG overload + unprivileged sequence role integration test, MySQL 8.4 + MariaDB 11.4 routine/trigger/sequence integration test, and browser overloaded-routine/trigger flow; local on `feat/explorer-object-search` / hold until merged & released |
| N06 DDL viewing/object navigation — DG-N | `drivers/{postgres,mysql}/explorer.rs`, `snowflake/catalog.rs`, `DdlPanel`, `Workbench.tsx` | VERIFIED_COMPLETE / H (bounded R3) | Backend-owned read-only DDL (`DDL` sub-view on table/view/materialized-view tabs and `DdlPanel` on object tabs) capped at 256 KiB UTF-8 (`cap_ddl`) with `ObjectDdlStatus::Unavailable { reason }` privilege/capability fallback, `Copy DDL`, and `Open in SQL editor` (never auto-executed) | P1 / Medium | Object metadata; PMS; read | Rust UTF-8 DDL capping & quoting unit tests, `DdlPanel` component test, PG/MySQL/MariaDB quoted-identifier DDL integration tests, and browser PG/MySQL DDL/Copy/Open-in-editor workflows; local on `feat/explorer-object-search` / hold until merged & released |
| N07 Object dependency inspection — DG-N | `drivers/{postgres,mysql}/explorer.rs`, `normalize_dependency_edges`, `ObjectInspectorView.tsx` | VERIFIED_COMPLETE / H (bounded R3) | Bounded 1-hop `Uses` / `Used by` dependency edges (max 200 per direction, self-cycle and duplicate filtered) across PG `pg_depend`/`pg_rewrite`/`pg_constraint`/`pg_trigger` and MySQL/MariaDB `KEY_COLUMN_USAGE`/`VIEW_TABLE_USAGE`/`VIEW_ROUTINE_USAGE`/`TRIGGERS`, with click-through tab navigation | P2 / High | N05/N06; PMS by capability; read | Rust cycle/dedup/bound unit test, PG cyclic-view + overload/table/sequence dependency integration test, MySQL/MariaDB FK/view/trigger dependency test, and browser routine→view→table dependency click-through; local on `feat/explorer-object-search` / hold until merged & released |
| N08 Object creation/modification UI — DG-N, DG-M | X only supports explicit SQL | MISSING / H | Generate reviewable engine-aware DDL through existing authorization | P2 / High | N04–N07, plans; PM; gate | Preview/execution match and no automatic destructive operation / hold |

## SQL editing

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| E01 Dialect highlighting/parsing — DG-E | E | PARTIAL / H | PG/MySQL dialects plus Snowflake dollar strings; bounded token reader preserves Unicode names. Full procedural/dialect semantics remain open | P1 / Medium | CodeMirror; PMS (R native console); read | Quoting/comments/procedural boundaries per dialect / hold |
| E02 Schema-aware autocomplete — DG-E, DG-EC | E, D | PARTIAL / H | PostgreSQL context ships in preview.13, MySQL/MariaDB in preview.14 and bounded Snowflake request context in preview.15; broader path/policy support and external metadata monitoring remain open | P1 / High | M05/M06; PMS; read | Context changes and stale schema refresh / hold |
| E03 Direct-base-table statement/block/alias scope — DG-E | E | VERIFIED_COMPLETE / H (bounded repair) | Caret block/UNION arm isolated; comments/strings ignored; CTE/derived shadows suppress physical columns. Advanced output inference remains outside this acceptance | P1 / Medium | E01; PMS; read | 24 context cases plus PM accepted-and-executed SELECTs and synthetic S browser flow; see batch evidence / shipped in preview.12; see hub |
| E04 Standard-delimiter identifier completion — DG-E | E | VERIFIED_COMPLETE / H (bounded repair) | Case/segments/escaped delimiters/Unicode preserved; accepted identifiers execute on PM fixtures. PostgreSQL U& escape decoding is deliberately declined | P1 / Medium | Identifier-aware lookup; PMS; read | Unit case twins/Unicode/dots/escaping; PM accepted-and-executed fixtures, synthetic S qualification / shipped in preview.12; see hub |
| E05 Completion invalidation — DG-EC | E | VERIFIED_COMPLETE / H (known input context) | Document/caret/config/index revisions reject stale replies; partial failures preserve successful metadata; eight retained/in-flight reads. Live session discovery remains E02/I04 | P1 / Medium | E03/E04; PMSR; local | Seven session/cache tests, three worker-protocol tests, browser regression / shipped in preview.12; see hub |
| E06 FK JOIN suggestions — DG-E, DG-FK | D, E | MISSING / H | Add grouped constraint metadata, then explicit JOIN/ON insertion | P1 / High | E03/E04/N03; PM first; read | Composite FKs, aliases, multiple candidate relationships / hold |
| E07 Syntax diagnostics and execution error locations — DG-I | E, X; AppError has code/message only | MISSING / H | Structured positions and UTF-8/UTF-16 mapping, navigate to offending statement | P1 / High | Error DTOs and adapters; PMS; read | Multibyte text, later-batch errors, driver missing position / hold |
| E08 Inspections/quick fixes — DG-I | X safety checks at execution, no editor inspections | MISSING / H | Local diagnostics with explicit fixes; backend safety stays authoritative | P2 / High | E07 and scope analysis; PMS; gate | Fix preserves intent and cannot authorize writes / hold |
| E09 SQL formatting — DG-F | E, formatter.worker.ts | PARTIAL / H | Whole-document worker formatting exists; selection formatting and per-dialect settings missing | P2 / Medium | Editor selections; PMS; local | Comments/literals unchanged and stale formatter results ignored / hold |
| E10 Rename and SQL refactoring — DG-R | E | MISSING / H | Preview scope-correct text edits; database renames separately authorized | P2 / High | Scope/definition graph; PMS; gate | Alias shadowing and reviewed DDL / hold |
| E11 Go to definition and usages — DG-SE | E, N | MISSING / H | Resolve identifiers to metadata/editor locations | P1 / High | E02/N06; PMS; read | Ambiguity and inaccessible objects / hold |
| E12 Snippets/live templates — DG-T | E | MISSING / H | Local dialect-aware templates with keyboard placeholders | P2 / Medium | Existing editor/persistence; PMS; local | Expansion/undo and no automatic execution / hold |
| E13 Statement navigation — DG-X | E selectedStatement | PARTIAL / H | Caret/selection execution exists; next/previous statement navigation missing | P2 / Low | Syntax boundaries; PMS; read | Comments, quoted semicolons, large documents / hold |
| E14 Multiple consoles/files — DG-S, DG-SQ | `file_capability.rs` (`GrantKind::SqlFile`), `studio.ts` (`openSqlFile`, `saveSqlFile`, `isTabFileDirty`), `workspace.ts`, `Workbench.tsx`, `App.tsx` | VERIFIED_COMPLETE / H (bounded R4) | Multiple consoles with named session attachment plus capability-gated `.sql` file Open (`⌘O`), atomic in-place Save (`⌘S` on query tabs; `reviewEdits` preserved on table tabs) and Save As (`⇧⌘S`) via reusable `GrantKind::SqlFile` tokens (max 64 grants, ≤ 4 MiB UTF-8, symlink/NUL rejection, sibling `.tmp` + `sync_all` + `rename`, dirty dot, unsaved-close/quit guard, zero raw paths over IPC, excluded from `SavedWorkspace.drafts`) | P1 / High | M06, native file capability; PMSR; local | Rust `sql_file_capabilities` unit test, `studio.test.ts` dirty/draft-exclusion tests, and browser Open/dirty-guard/Save/Save-As workflow; local on `feat/consoles-sessions-files` / hold until merged & released |
| E15 Query parameters — DG-P | X; no typed editor parameter workflow | MISSING / H | Bind values through engine adapters and include bindings in reviewed plan | P1 / High | Typed execution request; PMS by capability; gate | Literal/comment exclusion, NULL/types, approval digest / hold |
| E16 Context documentation — DG-DOC | E, N | MISSING / H | Metadata documentation at caret with permission-aware fallback | P2 / Medium | E11; PMS; read | Correct object, stale/denied metadata / hold |

## Query execution

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| X01 Statement/script execution — DG-X | X, E | IMPLEMENTED_UNVERIFIED / H | Preflight and sequential execution exist; rerun exact statement/selection/batch fixtures | P0 / Medium | Drivers/safety; PMS, R command grammar; gate | Partial failures, comments, unknown SQL / existing |
| X02 Multiple result sets — DG-X, DG-G | X, B, W | IMPLEMENTED_UNVERIFIED / H | Results/subtabs exist; verify prior results survive later failures | P0 / Medium | Retention; PMS; read | Empty/multiple result sets and later errors / existing |
| X03 Async execution/cancellation — DG-X | X, D | IMPLEMENTED_UNVERIFIED / H | Owned cancellation/quarantine exist; rerun races and cleanup | P0 / High | Jobs/session lifetime; PMSR; engine | Queued/active/late cancellation, lost acknowledgements / existing |
| X04 Transactions/session controls — DG-S | `drivers/{postgres,mysql}.rs`, `execution.rs` (`set_transaction_mode`, `commit_transaction`, `rollback_transaction`), `Workbench.tsx`, W | VERIFIED_COMPLETE / H (bounded R4) | Per-session `Auto-commit` vs `Manual commit` mode on PostgreSQL & MySQL/MariaDB, wire-observed `TransactionState` (`idle` / `inTransaction` / `failed`), explicit `Commit` (`ReadOnly` blocked, audit-logged) and `Rollback` (`Failed` recovery, audit-logged), `ErrorCode::Conflict` refusal on `Manual` → `Auto-commit` while open, MySQL phantom-tx cleanup, and independent grid commits. Savepoint UI and S/R transactions excluded | P1 / High | M06; PM; gate | Rust PG auto-BEGIN unit test, PG (`25P02` failed state) & MySQL/MariaDB manual-tx/grid-isolation integration tests, `studio.test.ts`, and browser PG/MySQL shared manual-tx Commit/Rollback/grid-isolation workflows; local on `feat/consoles-sessions-files` / hold until merged & released |
| X05 Execution metrics — DG-X, DG-Q | QueryEvent, W | PARTIAL / H | Client elapsed/affected rows exist; queue/first-row/server timing missing | P2 / Medium | Driver metrics; PMSR; read | Correct units and no client/server-time conflation / hold |
| X06 Query history — DG-SQ | N, L, X | IMPLEMENTED_UNVERIFIED / H | Search/reopen and opt-in text exist; rerun privacy and replay paths | P1 / Low | Persistence/plans; PMSR; local | Metadata-only history and explicit rerun authorization / existing |
| X07 Raw EXPLAIN — DG-Q | X, safety AST visitor | PARTIAL / H | SQL entry point exists; capability-aware action and plan result typing missing | P2 / Medium | A01; PMS by capability; gate | Nested writes and ANALYZE approval / hold |
| X08 Error reporting/navigation — DG-X | AppError, QueryEvent.Error, W | PARTIAL / H | Error panel exists; statement source ranges/navigation absent | P1 / Medium | E07; PMSR; read | Unicode offsets and missing server positions / hold |

## Data exploration and editing

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| G01 Virtualized tabular browsing — DG-G | G, B | IMPLEMENTED_UNVERIFIED / H | Glide/million-row synthetic grid exists; rerun responsiveness and native evidence | P0 / Medium | Arrow/cache; PMS; read | Fast scrolling, empty/wide values; FPS separate / existing |
| G02 Pagination — DG-G | G, D | PARTIAL / H | PM keyset paging exists; keyless and S previews intentionally bounded | P1 / High | Stable row identity; PMS; read | Composite keys, evicted pages, changing live rows / hold |
| G03 Server sorting/filtering — DG-G | `drivers/browse_query.rs`, `Driver::browse_query`, `execution.rs` (`classify_raw_browse_sql`), TableFilterBar | VERIFIED_COMPLETE / H (PostgreSQL & MySQL/MariaDB) | Typed multi-key sort with PK tie-breakers and explicit NULL order; bound typed filters; raw WHERE/ORDER BY clauses classified via `safety::prepare` with Safe Mode & one-use approval gating. Snowflake sort/filter and MySQL ENUM/SET/JSON/binary/float sort remain open | P1 / High | Paging/provenance/safety; PM; gate | Paged-vs-reference fixtures, hostile values, Safe Mode raw-clause classification & browser sort/filter/lock/approval flows / hold until released |
| G04 Cell editing — DG-G | G, X | IMPLEMENTED_UNVERIFIED / H | Typed table edits exist; rerun write/readback and unsupported-type gates | P0 / High | Provenance/change plans; PM; gate | Constraint/conflict/rollback tests / existing |
| G05 Multiple-cell paste — DG-G | `paste.ts`, `DirtyStore.setBatch`, `DatabaseGrid.tsx` | VERIFIED_COMPLETE / H (bounded R1 batch 2) | Complete TSV paste validated across visible/reordered columns, editability, numeric precision, NULL tokens and dirty byte/row budgets before mutating the sparse overlay | P1 / Medium | Dirty budget/type validation; PM; gate | Four paste unit tests, DirtyStore batch budget/all-or-nothing tests and browser reordered-column atomic paste flow / hold until merged |
| G06 Dirty tracking and review/commit/rollback — DG-G | G, X | IMPLEMENTED_UNVERIFIED / H | Sparse edits and atomic server commits exist; verify across retained pages | P0 / High | Original identity; PM; gate | Revert, discard, conflict, lost commit acknowledgement / existing |
| G07 JSON/array/binary/large-value viewers — DG-V | `ValueDialog.tsx`, `value-format.ts`, columnar, D | VERIFIED_COMPLETE / H (bounded R1 batch 2) | 16-byte-per-row binary Hex/ASCII dump (bounded 16 KiB preview + exact byte count), 1-based indexed PostgreSQL array element viewer (multi-dimensional/custom bounds, SQL NULL vs `"NULL"` vs `""`), and lossless token-level formatted/raw JSON viewer preserving 64-bit integers and numeric key order. Binary columns and non-PG arrays remain read-only | P2 / Medium | Lossless value metadata; PMSR; read | Seven formatter/decoder unit tests and browser binary/array/JSON inspection & formatting workflow / hold until merged |
| G08 FK data navigation — DG-FK | App `followKey`, `referencedRowsQuery`, `referencingRowsQuery`, `referencing_foreign_keys` | VERIFIED_COMPLETE / H (outbound & inbound) | Outbound (`⇧⌘B`, merged in #25) and permission-aware inbound (`⇧⌘G`) FK navigation open filtered tabs with composite ordinal pairing, multi-constraint chooser and NULL refusal | P1 / Medium | N03/G03; PM; read | PG/MySQL/MariaDB inbound catalog + unreadable-source tests and browser outbound/inbound single/composite/chooser/NULL flows / hold until merged |
| G09 Multi-row insert/delete — DG-G | `DirtyStore`, `DatabaseGrid.tsx`, `ReviewDialog.tsx`, `prepare_changes_batch`, PG/MySQL `apply` | VERIFIED_COMPLETE / H (bounded R1 batch 2) | Explicit `insert`/`update`/`delete` `RowChange` union, draft rows with `<default>`/`<required>` states, `FOR UPDATE` full-row conflict checks, and one atomic reviewed `BEGIN … COMMIT` transaction | P2 / High | Defaults/generated values/identity; PM; gate | DirtyStore insert/delete unit tests, PG/MySQL/MariaDB atomic mixed-batch & stale-delete rollback tests, and browser insert/delete/undo/review/commit workflow / hold until merged |
| G10 Column management — DG-G | `ColumnConfigPopover.tsx`, `table-layout.ts`, `DatabaseGrid.tsx`, `SavedWorkspace.table_layouts` | VERIFIED_COMPLETE / H (bounded R1 batch 2) | Hide (non-PK), reorder, resize and reset columns per `connectionId + schema + table` in bounded local workspace metadata (128 tables, 256 columns, purged on profile delete); reordered paste and schema refresh preserve alignment | P2 / Medium | Stable column identity; PMS; local | Three layout reconciliation tests, Rust workspace bounds/purge test, and browser hide/reorder/resize/reload/paste workflow / hold until merged |
| G11 Export — DG-EX | `copy-format.ts`, `CopyAsMenu.tsx`, `ExportDialog.tsx`, `file_capability.rs`, `export.rs` | VERIFIED_COMPLETE / H (bounded R2) | Bounded selection Copy-as (`CSV`, `TSV`, `JSON`, `NDJSON`, `SQL INSERT`, `Markdown`; max 5,000 rows / 2 MiB) respecting column order/visibility and lossless 64-bit numbers/NULLs, plus native-save-dialog capability-gated Rust streaming export (`CSV`, `TSV`, `JSON`, `NDJSON`) to `<dest>.partial` (`0600`) with atomic rename on completion, explicit `INCOMPLETE` trailer on cancellation/failure, Snowflake truncation disclosures, and zero SQLite row/path retention | P1 / High | Native file capability/stream ownership; PMS (`LiveTable` streaming on PM; `RetainedResult` on PMS); local | 6 `copy-format` Vitest cases, 6 Rust `file_capability`/`export` unit tests, PG/MySQL/MariaDB/Snowflake integration tests, and browser Copy-as + 1,500-row streaming export + mid-stream cancellation flows; merged in PR #28 (`c8d8aa1`) / hold until released |
| G12 Import — DG-IM | `ImportCsvDialog.tsx`, `file_capability.rs`, `import.rs`, `prepare_changes_batch` | VERIFIED_COMPLETE / H (bounded R2) | Capability-gated regular-file CSV/TSV/TXT import (max 16 MiB / 5,000 rows / 256 columns), upfront full-file schema/type/precision/PK/read-only validation, column remapping, preview, and reviewed 500-row / 8 MiB `RowChange::Insert` `ChangePlan` transactions via `ReviewDialog` with zero automatic retries | P2 / High | G09, native file capability; PM; gate | 3 Rust `import` unit tests, PG/MySQL/MariaDB read-only/remapping/conflict-rollback integration tests, and browser invalid-row rejection + remapping + reviewed commit workflow; merged in PR #28 (`c8d8aa1`) / hold until released |
| G13 Data comparison — DG-G | `DataCompareDialog.tsx`, `compare.rs` | VERIFIED_COMPLETE / H (bounded R2) | Key-aware bounded comparison across `LiveTable` (PM, honoring Safe-Mode raw-clause approvals) and `RetainedResult` snapshots (PMS, including cross-engine) up to 25,000 rows / 16 MiB per side and 5,000 returned diff/anomaly entries, distinguishing SQL `NULL` vs `""`, exact numeric text, and duplicate/`NULL`-key anomaly buckets without populating `ResultStore` | P2 / High | Identity and streaming; PMS snapshots, PM live tables; read | 2 Rust `compare` unit tests, PG/MySQL/MariaDB + cross-engine PG↔MySQL integration tests, and browser live-table/snapshot diff & anomaly workflow; merged in PR #28 (`c8d8aa1`) / hold until released |
| G14 Arbitrary SQL result editing | G, X | INTENTIONALLY_UNSUPPORTED / H | Preserve read-only joins/views/aggregates and unsupported keys/types. The owner approved relaxing this under roadmap milestone RX with Rust-proven single-table provenance | P3 / High | Future proof of provenance; PMS; gate | Existing rejection regressions / excluded |

## Database design

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| D01 ER diagrams — DG-D | D has basic FK metadata; no diagram UI | MISSING / H | Interactive bounded relationship graph | P2 / High | N03/N07; PMS by capability; read | Large schemas, cycles, keyboard access / hold |
| D02 Schema snapshots/comparison — DG-M | L snapshots app metadata only | MISSING / H | Engine schema snapshots with version/context and deterministic diff | P2 / High | N04–N07; PM first; read | Reordered metadata, quoted names, unsupported objects / hold |
| D03 DDL generation — DG-N, DG-M | X executes explicit SQL only | MISSING / H | Generate accurate object definitions and changes in Rust | P2 / High | N06/D02; PM first; gate | Round trips in disposable schemas / hold |
| D04 Migration review/synchronization — DG-M | X existing plan gateway | MISSING / H | Explicit selection and approval of immutable generated changes | P2 / High | D02/D03; PM; gate | Drift, destructive diffs, implicit commits, no unattended apply / hold |
| D05 Index/constraint editing — DG-N | D/N lack management workflows | MISSING / H | Preview engine-specific additions/changes and authorize exact DDL | P2 / High | N04/D03; PM; gate | Composite/partial indexes, lock impact and failures / hold |

## Developer productivity

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| P01 Search Everywhere/object symbol search — DG-SE | `SearchEverywhere.tsx`, `search-everywhere.ts`, `App.tsx` | VERIFIED_COMPLETE / H (bounded R3 + R4 `Saved` scope) | Unified Search Everywhere (`⌘K` `All` scope, `⇧⌘O` `Objects` scope) with `All` / `Objects` / `Saved` / `Actions` / `History` tabs, deterministic ranking across commands, open tabs, saved queries, all connected sessions' `catalog` + `objects` (excluding offline profiles with explicit connected/offline count note), and recent `HistoryEntry[]` (capped at 80 results) | P1 / Medium | Object identity/index; PMSR; read | 4 `search-everywhere` unit tests, existing `studio.spec.ts` palette focus/trap regression, and browser PG + MySQL + Saved query Search Everywhere workflows; R3 merged in #29 (`2625933`), R4 `Saved` scope local on `feat/consoles-sessions-files` / hold until released |
| P02 Recent locations/bookmarks — DG-RL, DG-B | `persistence/library.rs`, `LibraryModal.tsx`, `studio.ts` (`bookmarks`, `recentLocations`, `toggleBookmark`, `openLocation`), `Sidebar.tsx`, `Workbench.tsx` | VERIFIED_COMPLETE / H (bounded R4) | Persistent object & saved-query bookmarks (`⌘D`, max 256 in SQLite `settings` `'bookmarks'`, deduplicated by canonical target, purged on `delete_profile` / `delete_saved_query`, `onNeedsConnect` routing for offline profiles) plus a 50-item in-memory `Recent locations` ring buffer (`⇧⌘E`, never persisted to SQLite) | P2 / Medium | Stable document/object IDs; PMSR; local | Rust `library.rs` & backup-validation unit tests, `studio.test.ts` bookmark/50-item ring buffer/profile-delete tests, and browser bookmark + recent-locations jump & reload persistence workflow; local on `feat/consoles-sessions-files` / hold until merged & released |
| P03 Saved queries/scratch files — DG-SQ | `persistence/library.rs`, `LibraryModal.tsx` (`SaveQueryDialog`, `LibraryModal`), `studio.ts` (`savedQueries`), `SearchEverywhere.tsx` | VERIFIED_COMPLETE / H (bounded R4) | Named SQLite saved-queries library (`settings` `'saved-queries'`, max 256 entries, ≤ 256 KiB SQL each / 4 MiB total, optional `connectionId` defaulting detached when `storeQueryText === false`, production/`!storeQueryText` purge vs non-production detach on `delete_profile`) paired with capability-gated `.sql` files (`E14`) and opt-in drafts | P2 / Medium | E14/persistence; PMSR; local | Rust `library.rs` bounds/cascade/backup-validation tests, `studio.test.ts` privacy-default & profile-delete cascade tests, and browser Save-to-library + SearchEverywhere reopen + reload persistence workflow; local on `feat/consoles-sessions-files` / hold until merged & released |
| P04 Keyboard navigation — DG-SE | W, Modal, tabs helpers | PARTIAL / H | Existing tab/dialog/resize flows; full explorer/grid accessibility open | P1 / Medium | Accessible controls; PMSR; local | Keyboard end-to-end and native assistive tech / hold |
| P05 Command palette/custom shortcuts — DG-SE | App command list | PARTIAL / H | Fixed actions exist; discoverable contextual actions and rebinding missing | P2 / Medium | Command registry; PMSR; local | Conflicts, focus scopes, disabled capabilities / hold |
| P06 Workspace layouts/tab management — DG-S | W | PARTIAL / H | One-window panes/tabs and resize exist; richer persisted layout/tab controls absent | P2 / Medium | Existing Zustand/workspace DTO; PMSR; local | Restart, resize, dirty close, session independence / hold |
| P07 Settings/themes — DG-C, DG-F | W, Preferences | IMPLEMENTED_UNVERIFIED / H | Dark/light/contrast and density exist; rerun persisted UI behavior | P2 / Low | Local metadata; PMSR; local | Restart/theme/editor/grid consistency / existing |
| P08 Draft/session recovery — DG-SQ | W, L | PARTIAL / H | Opt-in drafts/settings recover; sessions/results/edits intentionally do not | P0 / High | Persistence; PMSR; local | Crash/opt-out/close flush; no connect or execution / existing |

## Advanced tools

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| A01 EXPLAIN visualizer — DG-Q | X raw SQL, no plan UI/DTO | MISSING / H | Raw and normalized tree views with explicit engine support | P2 / High | Plan adapters/capabilities; PM first, S separately; gate | Raw/tree consistency, ANALYZE authorization / hold |
| A02 Query analysis/profiling/plan comparison — DG-Q | X05 basic elapsed metrics | MISSING / H | Estimated/actual metrics and comparable plan snapshots | P2 / High | A01/X05; engine-specific; gate | Units, estimates vs actuals, unknown metrics and budgets / hold |
| A03 Index/storage statistics — DG-N, DG-Q | TableInfo estimatedRows/totalBytes | PARTIAL / H | Some estimates exist; index utilization/storage tools missing | P2 / Medium | N04, permission-aware stats; PM first; read | No automatic COUNT, stale estimates and permission denial / hold |
| A04 Session/process monitoring — DG-S | ProcessesView, Driver.processes | PARTIAL / H | PM process listing and guarded PG cancellation exist; broader monitoring not implemented | P2 / High | Stable process identity; PM; gate | Permission denial, reused IDs, own-session races / hold |
| A05 Lock inspection | D process data only | MISSING / M | Add engine-specific bounded lock/wait inspection; dedicated DataGrip lock-tool equivalence not established by cited docs | P2 / High | Process/lock metadata; PM; read | Blocking chains, races, partial privileges / hold |
| A06 Engine administration | X explicit SQL; R command allowlist | PARTIAL / H | Dedicated admin workflows absent; benchmark each engine rather than assume uniform DataGrip support | P3 / High | Explicit capabilities/plans; engine-specific; gate | Privilege boundaries and operation-specific rollback limits / hold |
| A07 Snowflake writes, Redis Cluster/Sentinel/streams/PubSub, external MySQL process kill | D, README restrictions | INTENTIONALLY_UNSUPPORTED / H | Preserve current restrictions until redesigned; the owner approved Snowflake DML under roadmap milestone RX with independent capability/security design | P3 / High | Separate compatibility projects; SMR; engine | Existing rejection tests / excluded |

## Internal architecture and qualification

| ID / feature | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| I01 Rust ownership and Safe Mode | X, C, D | IMPLEMENTED_UNVERIFIED / H | Core enforcement exists; retain all bypass and integration checks | P0 / High | Existing gateway; PMSR; gate | All entry points, stale approval, nested writes / existing |
| I02 Result retention and binary IPC | B | PARTIAL / H | Explicit budgets and binary transport exist; representative transient-copy/plateau measurements open | P0 / High | Native profiling; PMSR; local | Native binary payloads, wide rows, cleanup plateau / hold |
| I03 Typed command calls | B generated DTOs / generic client.call | PARTIAL / H | Add closed command/request/response map without replacing IPC | P2 / Medium | Rust DTO export; PMSR; gate | Compile-time invalid command/argument cases / hold |
| I04 Metadata cache ownership/invalidation | E, D | PARTIAL / H | Shipped PostgreSQL/MySQL/Snowflake snapshots follow actual session/request context; local isolated-console metadata epochs use actual session IDs; external subscriptions and broader policies remain open | P1 / High | M05/M06; PMS; read | Role/session changes, DDL and external refresh / hold |
| I05 Startup, completion and grid performance | Q, E, B | PARTIAL / H | 100k-name warm lookup p95 0.10 ms; explicit visible completion p95 55–61 ms misses 50 ms target. Representative native load, startup and FPS remain unqualified | P0 release gate / High | Target hardware, measured scenarios; PMSR; local | p95 startup/completion/display and attributable memory/FPS / hold |
| I06 Packaging/signing/updates/platforms | Q | PARTIAL / H | macOS ad-hoc preview exists; signing/notarization, updater and packaged Windows/Linux open | P0 release gate / High | Credentials and platform hosts; PMSR; transport | Installer/update integrity and native runtime checks / hold |
| I07 Recovery and metadata compatibility | L | PARTIAL / H | Version guards, WAL recovery and undo exist; broader historical upgrades/disk-loss cases open | P0 / High | Versioned persistence; PMSR; local | Migration fixtures, interrupted restore, unsupported stores / hold |

## Reproduced first-slice failures (baseline; repaired)

Baseline direct calls to `referencedTables` returned `customers` for
`SELECT 'FROM customers secret' FROM orders o` and for a `-- FROM customers`
comment. Two statements reusing alias `o` produced both tables in one completion
index. `WITH orders AS (SELECT 1 AS synthetic) SELECT * FROM orders o` resolved
the CTE to the physical catalog table. With both `Orders` and `orders` in the
catalog, `FROM "public"."Orders" x` resolved to lowercase `orders`.

The selected repair is E03–E05 plus the required E01 lexical correction. It does
not close E02/I04 live session-context tracking, E06 JOIN completion, or derived
query-output inference. See [implementation-roadmap.md](implementation-roadmap.md)
for the dependency order and verification contract.


## Implemented batch: scoped completion repair

Work is local on `fix/scoped-sql-completion`, based on fetched `bbba421`.
The audit/roadmap commit is `16a49e3`. Application push, PR, merge, publication
and installation are not authorized by this batch; preview.11 remains the
existing download. **Local candidate** means tested implementation only.

The workflow is: choose the connected SQL console, write a direct base-table
reference, request a column at the caret, accept it, and execute a harmless
SELECT. Disposable PostgreSQL 18, MySQL 8.4 and MariaDB 11.4 return the expected
value after accepting identifiers containing spaces/dots, mixed case and doubled
quote delimiters. Snowflake browser responses are synthetic. Redis command
completion stays on its existing grammar.

`referencedTables` now reads CodeMirror token trees with limits of 65,536 UTF-16
units per statement, 4,096 tokens and 64 nested groups; synchronous parsing gets
10 ms. Larger statements/token scopes decline completion; unsupported semantic scopes
decline contextual columns.
CodeMirror's ASCII identifier lexer is supplemented only at Unicode error leaves,
never inside strings/comments. Standard quoted names retain their segments.
PostgreSQL folds unquoted names down, Snowflake up; MySQL resolves exact spellings
first and rejects ambiguous folded matches. Short catalog names require an
unambiguous lookup under the configured default. The reader never infers CTE or
derived output columns, correlated outer bindings, renamed relation outputs,
mixed INSERT/SELECT projections, final set-operation ordering, or PostgreSQL
`U&` identifier escapes. These are explicit future inference work, not guessed
physical-table bindings. MySQL server case settings and Snowflake session
identifier parameters are not discovered live.

`CompletionSession` is local to each existing CodeMirror instance. It keeps the
catalog index stable across ordinary typing and replaces only scoped columns.
Typed worker messages identify the document, caret, known connection/catalog
context and actual installed index revision. Every obsolete/aborted/disposed
request settles; metadata waiters are removed from the pending map on cancellation
instead of retaining per-query callbacks and old documents. The total completion wait is bounded to two seconds. Metadata
is prefetched after 100 ms, with at most eight retained entries and eight
concurrent IPC reads. Retired queued reads are discarded, failed old entries
cannot evict replacements, and one denied table does not erase other metadata.
The backend `table_schema` command, privilege checks and Rust execution/Safe Mode
ownership are unchanged. No credentials, new IPC endpoint or persistence schema
are introduced. Live role/search-path tracking remains E02/I04.

Current check evidence (2026-10-28, macOS Apple Silicon, local Node 24/Rust tools):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 59 frontend tests, 83 Rust tests; frontend build, formatting, type checking, native check and strict Clippy |
| Passed | Full `pnpm test:e2e`: 20 workflows, including new PG/MySQL/MariaDB escaped-name SELECT acceptance, existing editing/Safe Mode/cancellation/recovery/Redis and synthetic Snowflake |
| Passed | After the final scope-boundary corrections, `pnpm test:e2e tests/e2e/completion.spec.ts tests/e2e/snowflake.spec.ts`: six workflows |
| Passed | Targeted ignored Rust fixtures: PostgreSQL ten connect/run/release/disconnect cycles; MySQL/MariaDB catalog/keys/paging/read-only tables; PostgreSQL TLS-through-SSH correct-host success and wrong-host rejection |
| Passed | Final-source `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature), `pnpm desktop:smoke:build`, `pnpm test:native`: three hidden runs/30 cleanup cycles including actual completion worker acceptance |
| Failed then repaired | Pre-fix lexical/CTE regressions, ambiguous catalog names, unfinished nested blocks and unsupported projection cases. The intermediate Snowflake browser assertion expected unquoted PUBLIC; it now checks the valid quoted insertion and subsequent suggestions |
| Skipped | Baseline Rust runner reports 29 ignored tests; three were run separately above. The entire ignored suite was not run: some fixtures issue DROP statements, excluded by the user's test constraint. Real credential-store interactions remain unrun |
| Blocked by environment | Initial shell-toolchain and Chromium sandbox attempts were resolved; no outstanding environment blocker for this bounded slice |
| Not executed | Production/account qualification, packaged Windows/Linux runtime, signing/notarization, installed-app update and broad native performance qualification |

The latest local logs are ignored artifacts under `.cache/completion-*.log`.
Native bundle/smoke outcomes are recorded alongside the final implementation
entry in `IMPLEMENTATION_STATUS.md`.

Performance evidence is reproducible with `tests/e2e/completion.spec.ts` and saved
under `.cache/benchmarks/`. Headless Chromium 156, one local machine: 100,000 names,
1,000 warm prefix samples, p95 **0.10 ms** (5 ms target); index construction
**9.8 ms**. Twenty warm explicit-popup samples per fixture measured p95
**60.6 / 54.9 / 57.0 ms** for PostgreSQL/MySQL/MariaDB, missing the 50 ms visible
target. Installed `@codemirror/autocomplete` 6.20.4 schedules explicit starts
with a fixed 50 ms timer; no dependency patch or editor replacement was made.
Automatic typing retains CodeMirror's 100 ms activation delay. These figures
exclude cold metadata latency from the warm sample and do not establish native
WebView, large-catalog visible latency, total memory, startup or FPS targets.

Identifier rules were cross-checked against official
[PostgreSQL lexical documentation](https://www.postgresql.org/docs/current/sql-syntax-lexical.html),
[MySQL object naming](https://dev.mysql.com/doc/refman/8.4/en/identifiers.html), and
[Snowflake identifier requirements](https://docs.snowflake.com/en/sql-reference/identifiers-syntax).


## PostgreSQL live completion context — 29 October 2026

The PostgreSQL slice of E02/I04 shipped in preview.13 through PR #21 (`e8f1cec`),
from implementation `c870579` based on preview.12. The broader
multi-engine capabilities remain PARTIAL. See [the live context contract](live-completion-context.md)
and [architecture decisions](architecture-decisions.md).

| Field | E02-PG bounded workflow |
|---|---|
| Feature | PostgreSQL live session-aware schema/column completion |
| DataGrip reference | DG-E / DG-EC above; SQL resolution follows the selected session context |
| State / confidence | VERIFIED_COMPLETE for the bounded fixture workflow / High; multi-engine E02/I04 remain PARTIAL |
| Sources / symbols | `drivers/postgres.rs`: completion_identity/catalog/schema, schema_on; `execution.rs`: Studio.completion_catalog/completion_schema, Session.completion, run_plan invalidation; `domain.rs`: CompletionIdentity/CompletionCatalog; SqlEditor, CompletionSession, tableLookup and Workbench contextEpoch |
| Observed behavior | Shows actual database/effective role/ordered schemas; refreshes after execution and explicit refresh; accepts and executes an alternate-schema column; clears obsolete responses; uses server-resolved unqualified identities, including system/temporary objects and inaccessible earlier-name shadows |
| Missing behavior | Other live adapters, independent console binding, server-pushed external DDL/grant invalidation and advanced projection inference |
| Priority / complexity | P1 / High |
| Dependencies / scope | Existing Rust editor connection/execution lane, typed IPC, CodeMirror worker and bounded cache; PostgreSQL 18.6 disposable fixture |
| Security / resources | Same-session grants and column SELECT checks; fixed pg_catalog-qualified SQL; revision validation; completion metadata read-only; 10,000 relations / 2 MiB, 128 schemas, 4,096 columns / 1 MiB; eight frontend reads/cache entries and 100 suggestions. No authorization bypass or replay |
| Acceptance | SET search_path, SET ROLE, SET LOCAL/ROLLBACK, failed-transaction recovery and temporary schemas reflect the actual connection; denied and stale metadata never supplies old columns; concurrent unchanged readers share a valid revision; metadata cannot block active-query cancellation; existing grid keys/browsing pass |
| Tests | Three selected Rust fixture tests in postgres.rs; ordered/server-resolved lookup and delayed old-privilege unit tests; completion.spec.ts accepted SELECT/role/rollback workflow; native command guards through Tauri |
| Release eligibility | Shipped and installed in preview.13 via PR #21; six PR checks and five main jobs passed. External/platform/signing/performance gates remain open |

| Outcome | Current-batch evidence |
|---|---|
| Passed | `pnpm validate`: 62 frontend tests, 83 standard Rust tests, formatting/types/build/Cargo check/strict Clippy |
| Passed | `cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres live_ -- --ignored --test-threads=1`: all three PostgreSQL workflows |
| Passed | Full `pnpm test:e2e`: 21 workflows, including completion, legacy grid edits, Safe Mode, cancellation, recovery, MySQL/MariaDB, Redis and synthetic Snowflake |
| Passed | `pnpm desktop:build`, `pnpm desktop:bundle:mac` with strict ad-hoc signature verification, `pnpm desktop:smoke:build` and `pnpm test:native`: three hidden runs / 30 cleanup cycles, including actual Tauri command guards |
| Failed then repaired | Overlapping metadata reads initially reported busy; bounded metadata serialization fixes this. Distinct tokens invalidated unchanged concurrent readers; they now share a revision. Qualified multi-array unnest broke legacy grid FK reads; qualified ROWS FROM plus a regression fixes it. A new assertion reproduced missing implicit system relations; server-resolved names now also prevent denied-table shadows from binding later tables. The cancellation test now waits for actual query activity |
| Environment interruption | An earlier full run stopped after Vite root/module requests stalled during concurrent contract/native builds. Final checks run sequentially. After resumption, stopped fixture databases caused connection failures; the task-owned fixture project was restarted and its healthy state verified before the passing rerun. Exact orphan test servers were stopped; user servers and installed app were preserved |
| Skipped | Standard Rust runner reports 32 ignored tests; three selected non-destructive fixtures were executed separately. The entire ignored suite was not run because some fixtures issue DROP; real credential-store tests remain unrun |
| Blocked by environment | No outstanding blocker for this bounded local workflow |
| Not executed | Production/live account qualification, Windows/Linux packaged runtime, Developer ID/notarization, new release/install and representative native performance qualification |

Final native packaging outcomes are recorded in `IMPLEMENTATION_STATUS.md`.
Local logs are `.cache/live-context-*-final.log`. The passing browser rerun measured
warm explicit-popup p95 **53.8 / 53.5 / 53.3 ms** for PostgreSQL/MySQL/MariaDB;
the 50 ms target remains missed. Warm 100,000-name lookup p95 was **0.10 ms**
(1,000 samples; index build 6.8 ms). These are headless Chromium observations,
not native startup/memory/FPS qualification. No production connections,
destructive runtime SQL or application release form part of this batch.


## MySQL/MariaDB continuation — preview.14 and case-mode follow-up

E02-MM extends the same typed snapshot/worker contract from preview.13's merged
`e8f1cec`; the [adapter contract](mysql-live-completion-context.md) records source
symbols, acceptance and limits. E02/I04 remain PARTIAL across all engines.

| Field | E02-MM bounded workflow |
|---|---|
| Feature / scope | Effective database/account/roles, server table-name case policy and column-grant completion; MySQL 8.4 / MariaDB 11.4 mode-0 and mode-1 fixtures |
| DataGrip reference | DG-E / DG-EC; selected session and database control SQL resolution |
| State / confidence | VERIFIED_COMPLETE for the bounded mode-0 workflow / High. Mode 1 server qualification passes on both disposable engines in the follow-up; mode 2 remains IMPLEMENTED_UNVERIFIED |
| Sources | drivers/mysql.rs completion_identity/fetch_text/collect_metadata; mysql/catalog.rs live_completion_catalog/live_completion_schema; domain.rs; SqlEditor, CompletionSession, tableLookup |
| Observed behavior | USE and active role changes refresh context; valid accepted suggestions execute against the current database; UPDATE-only columns are withheld; known temporary shadows use actual columns; USE persists through transaction rollback; busy metadata returns promptly before query cancellation |
| Missing behavior | Temporary-only object discovery, mode-2 runtime qualification, external metadata subscriptions and live Snowflake context; saved SQL profiles require a default database |
| Priority / complexity | P1 / High |
| Dependencies | Shipped Rust session revision/lane, fixed metadata SQL, CodeMirror worker/cache; no new commands, global state or executor |
| Security / budgets | Existing Rust Safe Mode unchanged; same-session metadata, escaped identifiers, SELECT column privileges and read-only metadata. Existing 10,000 relations / 2 MiB and 4,096 columns / 1 MiB caps; identity 64 KiB; eight frontend reads/cache entries and 100 suggestions |
| Acceptance | Actual USE/roles/grants follow the connection; no stale prepared DATABASE(), old-revision columns, wrong-case relation binding or MySQL three-part insertion; existing grid and cancellation remain usable |
| Tests | mysql_live_completion_tracks_database_roles_and_column_grants plus existing catalog/paging and cancellation fixtures on both engines; strict/folded/path unit test; two browser acceptance flows. Follow-up mysql_live_completion_uses_server_case_policy runs on case-mode-1 fixtures |
| Release eligibility | Initial slice shipped and installed as preview.14 after PR #22 and five main jobs; public assets, clean-source native evidence and ad-hoc signature verified. Mode-1 qualification merged separately through PR #23 |

Passed: `pnpm validate` (63 frontend, 83 Rust), three selected MySQL fixture tests
across both engines, and all 23 browser workflows. Failed then repaired: initial
Unsupported and prepared-statement DATABASE() retaining its original context.
Existing bound-parameter grid reads share the same collector and still pass.
Three PostgreSQL context regression tests also passed on this revision, for six
selected Rust tests total. `pnpm desktop:build`, `pnpm desktop:bundle:mac` with
strict ad-hoc signature, `pnpm desktop:smoke:build` and `pnpm test:native` pass;
three hidden runs complete 30 cleanup cycles. Native provenance records the
pre-commit working tree (sourceDirty=true); no new installation occurred.
Logs are `.cache/mysql-context-*.log`; detailed native evidence is preserved in
native-smoke. The task-owned fixture project and volume were removed.
Skipped: 33 standard-runner ignored tests; selected fixtures are recorded separately.

Release evidence: PR #22 passed six checks and squash-merged as `dc220b0`,
with an identical source tree to tested `7697c6e`. All five main jobs passed
([run 40817452645](https://github.com/quiinthieu/joinery/actions/runs/40817452645)).
[Preview.14](https://github.com/quiinthieu/joinery/releases/tag/v0.1.0-preview.14)
assets, checksums, public download, extracted app and strict ad-hoc signature
verify. Clean-source native evidence passed three runs/30 cleanup cycles.
The installed preview.14 executable matches its release digest and launched
with a visible native window after the prior app exited. No production SQL or
automatic reconnect was performed during installation verification.

The separate mode-1 follow-up uses disposable MySQL 8.4 and MariaDB 11.4
servers initialized with lower_case_table_names=1. Both engines passed the
same Rust test for case policy, role-visible catalog, unqualified names,
canonical columns, stale revision rejection and uppercase SQL. Mode 2 needs
a suitable case-insensitive filesystem; the current Linux containers do not
qualify it. The fixture project was removed after the test.
Not executed: production/live accounts, destructive local runtime SQL, other
packaged platforms, real case mode 2, signing/notarization and broad performance
qualification. No environment blocker remains for the tested local workflow.

## Snowflake request-context continuation — preview.15

E02-SF uses the existing E02/I04 snapshot and worker. The overall capability
remains PARTIAL; this scope does not qualify a live account or full search-path
resolution. See [the bounded contract](snowflake-completion-context.md).

| Field | E02-SF bounded workflow |
|---|---|
| DataGrip reference | DG-E / DG-EC; selected context drives name resolution |
| State / confidence | PARTIAL / High source and synthetic evidence; live account behavior IMPLEMENTED_UNVERIFIED |
| Sources | snowflake/completion.rs live_completion_identity/live_completion_catalog; catalog.rs context_rows; Driver/Studio completion_catalog; SqlEditor |
| Observed behavior | Server identity/path and quoted policy are read under the same request fields as SQL; first-schema table names bind; qualified names remain; unsupported policy and failed context show errors and withhold stale metadata |
| Missing behavior | Later-schema fallback, complex/other-database path discovery, case-insensitive quoted policy and live-account qualification |
| Priority / complexity | P1 / Medium |
| Dependencies | Existing Rust session lane/revision, SQL API authentication, read-only metadata adapter and completion worker |
| Database scope | Snowflake configured database, explicit request role/schema/warehouse; primary user read from the server |
| Security / bounds | Safe Mode and Snowflake read-only restrictions retained; fixed bounded context SQL, one row / 64 KiB each; existing catalog/column/result limits; no reconnect/replay |
| Acceptance | Verified role/database/schema/warehouse; request fields match execution; conservative unqualified binding; qualified/alias lazy columns; visible error and refresh recovery; old revisions rejected by existing Studio path |
| Tests | Three Rust HTTP tests for context/request fields/path scope/quoted policy/malformed data/byte limits; browser context display/failure/recovery/revision-bound completion; existing database/browser regressions |
| Release eligibility | Shipped and installed in preview.15 via PR #24 after six PR checks/five main jobs, clean-source native evidence and public asset verification. Live, platform, signing and performance gates remain open |

Passed on the current slice: `pnpm validate` (63 frontend, 86 standard Rust),
three new HTTP fixture tests, all 23 browser workflows in the final fixed-file
run, desktop build, macOS bundle/strict ad-hoc signature and three native
runs/30 cleanup cycles. The initial browser run had 22 passes and a Redis form
timeout during page reloads; the trace and final rerun are recorded separately
in IMPLEMENTATION_STATUS.md. Native provenance is pre-commit sourceDirty=true;
that local build was not release provenance. The slice subsequently shipped and
installed in preview.15 from an identical clean source tree. The fixture project
was removed.

## Isolated query sessions — local M06-Q slice

| Field | M06-Q bounded workflow |
|---|---|
| Feature / reference | Explicit tab-owned SQL session, shared/default session preserved; DG-S |
| State / confidence | VERIFIED_COMPLETE for the bounded local workflow / High. External auth/platform qualification is unverified; broader M06/E14/X04 remain PARTIAL |
| Sources | studio.ts querySessionForTab/bindQuerySession/retireSession/closeTab; App runSql/executePrepared/health/close; Workbench session controls and scoped completion epoch; ConnectDialog handoff; existing Rust connect/disconnect |
| Observed behavior | PM/MariaDB state stays isolated; shared query completes while isolated query runs; owned cancellation retires only that socket; explicit reconnect, close confirmation and backend session cleanup pass. Changed-session prepared response is never submitted |
| Missing behavior | Named/reused multi-client sessions, session/transaction recovery, transaction status/autocommit/savepoint controls; no new SQL-session semantics for Snowflake/Redis |
| Priority / complexity | P1 / Medium, reusing existing backend capacity and credential flow |
| Dependencies / engines | Existing Rust-owned four-session limit, typed connect/disconnect, Safe Mode, completion revisions; PostgreSQL/MySQL/MariaDB |
| Security / limits | No credential cloning/persistence, automatic reconnect/replay or closed-session fallback. Exact-ID retirement and stale-plan guard. Existing result budgets, engine authorization and grid metadata retained |
| Acceptance | Two-console state/concurrency isolation; cancel/reconnect affects owned tab; late responses cannot rebind or execute; close releases session and preserves siblings; drafts contain SQL without live IDs |
| Tests | Eight new frontend units; three actual engine/browser flows; final pnpm validate 71 frontend/86 Rust passes. Final full browser suite 26/26, desktop build/bundle and three native runs/30 cleanup cycles pass. Earlier network, helper and host-pressure failures are recorded separately |
| Release eligibility | Merged on `main` via PR #26 (`877904b`, all six PR checks green), unreleased. Clean-source release provenance remains before shipping; see isolated-query-sessions.md |

## Table data view batch (R1 batch 1) — merged in PR #25 (unreleased)

Branch `feat/table-data-view` from fetched `799e600` merged on `main` as
`6445b0e` via PR #25 (six PR checks and five `main` validation jobs passed). It
implements the first batch of the approved DataGrip-replacement program in
[implementation-roadmap.md](implementation-roadmap.md) and remains unreleased.

| Field | Table data view workflow |
|---|---|
| DataGrip reference | DG-G data editor sorting/filtering; DG-FK navigation to referenced data |
| State / confidence | N03, G03 (typed) and G08 (outbound) VERIFIED_COMPLETE for the bounded workflow / High |
| Sources | `domain.rs` ForeignKey::group, SortKey/FilterOp/ColumnFilter/BrowseQuery; `drivers/browse_query.rs` build/seek; PostgresDialect, MysqlDialect; `Driver::browse_query`; Studio.browse_with; `browse_table` query argument; TableFilterBar, `table-query.ts`, studio setTableQuery/openTable, App followKey/goToReferenced |
| Observed behavior | Header click sorts ascending → descending → off, and Shift adds a key. PK tie-breakers and fixed NULL order make keyset pages equal a reference ORDER BY across three pages. Filters bind every value. PostgreSQL text search uses ILIKE and MySQL follows the column collation; LIKE metacharacters are escaped. Controls lock while edits are pending. FK navigation opens a new filtered tab, pairs composite columns by ordinal, offers a chooser for multi-key columns and refuses NULL keys |
| Missing behavior | Raw WHERE/ORDER BY text, inbound referencing rows, Snowflake sort/filter, persisted per-table views, ENUM/SET/JSON/binary/float ordering on MySQL |
| Security / bounds | Identifiers come only from the current schema; at most 8 sort keys, 16 filters, 64 KiB per value; existing 500-row/2 MiB pages and memory budgets. Generated reads use the existing metadata lane; Safe Mode and approvals unchanged; filter values are not persisted |
| Defects found and fixed | MySQL read-only profiles failed to browse editable tables (masked editability treated as schema drift); the frozen primary-key column stayed blank for pages loaded after scrolling (Glide's visible region excludes frozen columns). Each has its own commit and regression test |
| Tests | 9 browse_query unit tests, 2 FK grouping unit tests; PG/MySQL/MariaDB fixture tests for grouped FKs, paged-vs-reference sort/filter and read-only browsing; 8 new vitest cases; 4 new browser workflows |

Evidence (2026-10-30, macOS Apple Silicon, local Node 24 / Rust 1.99.1):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 71 frontend tests, 97 standard Rust tests, formatting, contracts, types, build, Cargo check and strict Clippy |
| Passed | Ignored integration suite on task-owned disposable fixtures (`postgres`, `ssh`, `mysql`, `redis`): 35 tests, including the new read-only, grouped-FK and sort/filter reference tests |
| Passed | Browser suite: the first full run passed 26 of 27. The failed MariaDB edit flow had reloaded to the welcome screen while documentation was being edited under the live Vite server. The MySQL spec rerun and a final full run with files held fixed passed all 27 workflows |
| Passed | `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature), `pnpm desktop:smoke:build`, `pnpm test:native --runs 3 --cycles 10`: three hidden runs, 30 cleanup cycles, main-process peak RSS 100–106 MiB. Fixture SHA-256 `f2d9bafd…5d9b` from `76f04be` with only docs uncommitted (sourceDirty=true) |
| Failed then repaired | Seek bind order (unit test); MySQL Conflict on read-only browsing (reproduced on HEAD in an isolated worktree, then fixed); test helper used an unsupported mouse modifier; frozen-column regression detected 2/3 unfixed runs, 3/3 pass fixed |
| Blocked by environment | Case-mode-1 MariaDB fixture: port 53407 is held by the agent host process, so `mysql_live_completion_uses_server_case_policy` was skipped |
| Not executed | Production accounts, Snowflake live, Windows/Linux packages, signing/notarization, representative performance |

## Table data editing batch (R1 batch 2) — PR #27 (unreleased)

Branch `feat/table-data-editing` (`2a59fae..b62e475`) from fetched `origin/main`
`877904b` (PR #26). It completes Milestone R1 (Data view parity) of
[implementation-roadmap.md](implementation-roadmap.md) and remains unreleased.

| Field | Table data editing workflow |
|---|---|
| DataGrip reference | DG-G data editor (raw WHERE/ORDER BY, column management, multi-cell paste, row insert/delete); DG-FK inbound referencing rows; DG-V rich value viewers |
| State / confidence | G03 (PM), G05, G07, G08 (outbound + inbound), G09 and G10 VERIFIED_COMPLETE for the bounded workflow / High |
| Sources | `browse_query.rs` (`raw_where`, `raw_order_by`), `execution.rs` (`classify_raw_browse_sql`, `prepare_browse`, `prepare_changes_batch`), `domain.rs` (`InboundForeignKey`, `RowChange`, `TableColumnLayout`), PG/MySQL `catalog.rs` (`referencing_foreign_keys`) and `changes.rs` (`apply`); `TableFilterBar.tsx`, `ColumnConfigPopover.tsx`, `table-layout.ts`, `paste.ts`, `DirtyStore`, `DatabaseGrid.tsx`, `ReviewDialog.tsx`, `ValueDialog.tsx`, `value-format.ts` |
| Observed behavior | Raw WHERE/ORDER BY clauses classify via `safety::prepare` (pure reads execute directly; side-effecting/unparseable SQL is blocked in ReadOnly and requires one-use `ReviewDialog` approval in writable modes; raw ORDER BY disables keyset continuation cleanly). `⇧⌘G` opens referencing rows in a new filtered tab with permission-filtered inbound constraints, composite ordinal pairing and multi-constraint chooser. Column hide (non-PK), reorder and widths persist per `connectionId + schema + table` across reloads and purge on profile delete. Multi-cell TSV paste validates every target cell and budget atomically before mutating `DirtyStore`, respecting reordered visible columns. Draft row inserts (`<default>`/`<required>`) and row deletes (`FOR UPDATE` full-row conflict checks) execute alongside updates in one reviewed `BEGIN … COMMIT` transaction. `ValueDialog` adds 16-byte-per-row binary Hex/ASCII dumps, 1-based indexed PostgreSQL array viewers (1D/2D/3D, custom bounds, SQL NULL vs `"NULL"` vs `""`) and lossless token-level formatted JSON preserving 64-bit integers and numeric key order |
| Missing behavior | Snowflake table sort/filter, arbitrary-result editing (RX), PK editing on existing rows, editable binary columns, non-PostgreSQL array viewers, streaming file export/import/compare (R2) |
| Security / bounds | Raw clauses capped at 16 KiB each, wrapped in newline-terminated guards, multi-statement inputs rejected, one-use approval token scoped to exact session/table/query. `SavedWorkspace.table_layouts` capped at 128 tables and 256 columns per table (no filter/clause literals persisted). Paste and row mutations enforce existing 8 MiB / 500-row dirty budgets and immutable expiring `ChangePlan` digests. Binary preview bounded to 16 KiB while reporting exact byte length; PG array element table bounded to 2,000 elements |
| Defects found and fixed | During Commit 5 (`G09`) browser verification, clicking `Insert row` updated `selection` in a passive `useEffect` without updating `selectionRef.current` synchronously, allowing a sibling `useEffect` to revert Inspector/paste focus to the prior cell; switching `focusRowRequest` to `useLayoutEffect` with eager `selectionRef` synchronization resolved the race. During post-implementation Tauri capability review, `referencing_foreign_keys`, `prepare_browse_table` and `prepare_table_changes` were found missing from `src-tauri/build.rs` and `src-tauri/capabilities/main.json`; enrolled all three and added typed real-WebView IPC guard checks in `src/native-smoke.ts` |
| Tests | 21 new Vitest cases (`table-query`, `table-layout`, `paste`, `dirty-store`, `value-format`, `studio`), 5 new Rust unit tests (raw-clause safety classification, inbound FK grouping, workspace layout bounds/purge), 6 new disposable PG/MySQL/MariaDB integration tests (raw clauses, inbound FKs, atomic insert/update/delete + stale-delete rollback), and 6 new Playwright E2E workflows |

Evidence (2026-10-30, macOS Apple Silicon, local Node 24 / Rust 1.99.1):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 99 frontend tests (19 files), 102 standard Rust tests, formatting, contracts, types, production build, Cargo check and strict Clippy; script tests (`git-identity.test.mjs` 6/6, `test_runtime_memory.py` 4/4) |
| Passed | Ignored integration suite on task-owned disposable fixtures (`postgres` 17, `mysql` 16 across MySQL 8.4 & MariaDB 11.4, `redis` 6, `ssh` 2): 41 tests passed, 0 failed |
| Passed | Full `pnpm test:e2e` with repository files held fixed: all 37 browser workflows passed in one run (`1.7m`), including all 9 `table-data.spec.ts` workflows |
| Passed | Clean-commit (`b62e475`, `sourceDirty=false`) `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature, `Joinery.app` 19.91 MiB), `pnpm desktop:smoke:build` and `pnpm test:native --runs 3 --cycles 10`: three hidden runs (3,725 / 2,927 / 2,925 ms), 30 cleanup cycles, main-process peak RSS 105.12–105.83 MiB, fixture SHA-256 `dcc7e495…2b55` (including WebView IPC ACL guards for all three new R1 Batch 2 commands) |
| Blocked by environment | Case-mode-1 MariaDB fixture: port 53407 is held by the agent host process, so `mysql_live_completion_uses_server_case_policy` was skipped locally (covered by CI `integration` job) |
| Not executed | Production accounts, live Snowflake accounts, Windows/Linux packaged runtime, Developer ID signing/notarization, representative performance qualification |

## Data movement batch (Milestone R2) — merged in PR #28 (unreleased)

Branch `feat/data-movement` (`a3020d7..34c51db`) from fetched `origin/main`
`94f63ba` (PR #27) merged on `main` via PR #28 (`c8d8aa1`, all six PR checks
passing). It implements Milestone R2 (Data movement: `G11`, `G12`, `G13`) of
[implementation-roadmap.md](implementation-roadmap.md) and remains unreleased.

| Field | Data movement workflow |
|---|---|
| DataGrip reference | DG-EX export and Copy-as extractors; DG-IM CSV/TSV import mapping and transactional apply; DG-G key-aware table and query-result data comparison |
| State / confidence | G11 (PMS), G12 (PM) and G13 (PMS snapshots + PM live tables) VERIFIED_COMPLETE for the bounded workflow / High |
| Sources | `file_capability.rs` (`FileDialogPort`, `RfdFileDialog`, `SandboxFileDialog`, `FileCapabilityStore`), `export.rs` (`start_export`, `cancel_export`), `import.rs` (`preview_csv_import`, `prepare_csv_import_batch`), `compare.rs` (`compare_data`), `commands.rs`, `build.rs`, `capabilities/main.json`; `copy-format.ts`, `CopyAsMenu.tsx`, `ExportDialog.tsx`, `ImportCsvDialog.tsx`, `DataCompareDialog.tsx`, `DatabaseGrid.tsx`, `Workbench.tsx`, `App.tsx`, `native-smoke.ts` |
| Observed behavior | Grid selection Copy-as (`CSV`, `TSV`, `JSON`, `NDJSON`, `SQL INSERT`, `Markdown`) formats visible/reordered columns while preserving SQL `NULL` vs `""`, unrounded 64-bit integers/decimals and dialect identifier quoting (`"` PG/Snowflake, `` ` `` MySQL/MariaDB). Streaming exports write through native save-dialog grants to `<dest>.partial` (`0600`) in 500-row pages without retaining rows in `ResultStore`, atomically rename on completion, append an explicit `INCOMPLETE` trailer on cancellation or error, and surface Snowflake 100,000-row / 64 MiB retention truncation warnings. CSV/TSV imports validate the entire file upfront (header mapping, required non-default columns, integer/float/boolean/JSON types, binary/generated rejection, duplicate in-file PKs and session `SafeMode::ReadOnly`) before generating reviewed 500-row / 8 MiB `RowChange::Insert` `ChangePlan` batches routed through `ReviewDialog`. Key-aware data comparison diffs two `LiveTable` or `RetainedResult` sources (including cross-engine PostgreSQL ↔ MySQL/MariaDB) with explicit key pairing, `NULL` vs `""` fidelity, and separate duplicate-key / `NULL`-key anomaly buckets |
| Missing behavior | Arbitrary-path CLI/headless export without dialog capability grant, non-CSV/TSV import formats (Parquet/Excel/SQL dump), direct Snowflake unretained multi-million-row streaming export beyond retained result snapshots, automatic sync/merge DDL/DML generation from data-compare diffs |
| Security / bounds | Zero raw filesystem paths cross IPC (`FileWriteGrant` one-use 15-min TTL, `FileReadGrant` 30-min TTL for regular files ≤ 16 MiB). Copy-as capped at 5,000 rows / 2 MiB; streaming export capped at 1,000,000 rows / 512 MiB and at most 2 concurrent jobs; CSV import capped at 16 MiB / 5,000 rows / 256 columns / 256 KiB cell and 500 rows / 8 MiB per reviewed `ChangePlan`; data comparison capped at 25,000 rows / 16 MiB per side and 5,000 returned diff/anomaly entries. Live-table export and comparison reuse `classify_raw_browse_sql` + `BrowseApproval` for raw clauses. Zero exported rows, imported values or file paths are written to SQLite (`history`, `audit_log`, `workspace`) |
| Defects found and fixed | During Playwright E2E verification of CSV import commit, `onImported()` triggered `refreshTable(targetTab.id)`, which transiently clears `tab.schema = null` while fetching fresh schema/pages (`src/state/studio.ts:445`). `App.tsx` previously guarded `<ImportCsvDialog />` on `targetTab.schema`, unmounting and remounting the dialog after commit (wiping completion state and multi-batch progress). Fixed in `34c51db` by keeping `<ImportCsvDialog />` mounted across transient schema refreshes, caching the last non-null `TableSchema` inside `ImportCsvDialog`, and lifting `importRawRefreshTabRef` to a component-level `useRef` |
| Tests | 6 new Vitest `copy-format` cases, 11 new Rust unit tests (`file_capability` 3, `export` 3, `import` 3, `compare` 2), 7 new Rust integration tests (`postgres` 3, `mysql` 3 across MySQL 8.4/MariaDB 11.4 + cross-engine PG↔MySQL, `snowflake_profile` 1), 4 new Playwright E2E workflows (`tests/e2e/data-movement.spec.ts`), and 7 real-WebView Tauri capability ACL assertions in `src/native-smoke.ts` |

Evidence (2026-10-30, macOS Apple Silicon, local Node 24 / Rust 1.99.1):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 105 frontend tests (20 files), 115 standard Rust tests, formatting, contracts, types, production build, Cargo check and strict Clippy; script tests (`git-identity.test.mjs` 6/6, `test_runtime_memory.py` 4/4) |
| Passed | Ignored integration suite on task-owned disposable fixtures (`postgres` 20, `mysql` 19 across MySQL 8.4 & MariaDB 11.4, `redis` 6, `ssh` 2): 47 tests passed, 0 failed |
| Passed | Full `pnpm test:e2e` with repository files held fixed: all 41 browser workflows passed in one run (`1.7m`), including all 4 `data-movement.spec.ts` workflows |
| Passed | Clean-commit (`34c51db`, `sourceDirty=false`) `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature, `Joinery.app` 20.41 MiB, bundle executable SHA-256 `872ed281…580b`), `pnpm desktop:smoke:build` and `pnpm test:native --runs 3 --cycles 10`: three hidden runs (4,050 / 2,988 / 2,962 ms), 30 cleanup cycles, main-process peak RSS 100.56–105.36 MiB, fixture SHA-256 `72cf5899…fc21` (including WebView IPC ACL guards for all 7 new R2 commands) |
| Blocked by environment | Case-mode-1 MariaDB fixture: port 53407 is held by the agent host process, so `mysql_live_completion_uses_server_case_policy` was skipped locally (covered by CI `integration` job, which passed on PR #28) |
| Not executed | Production accounts, live Snowflake accounts, Windows/Linux packaged runtime, Developer ID signing/notarization, representative performance qualification |

## Explorer and object search batch (Milestone R3) — merged in PR #29 (unreleased)

Branch `feat/explorer-object-search` (`7595db4..26f5136`) from fetched
`origin/main` `c8d8aa1` (PR #28) merged on `main` via PR #29 (`2625933`, all six
PR checks passing). It implements Milestone R3 (Explorer & object
search: `N01`, `N02`, `N04`, `N05`, `N06`, `N07`, `P01`) of
[implementation-roadmap.md](implementation-roadmap.md) and remains unreleased.

| Field | Explorer and object search workflow |
|---|---|
| DataGrip reference | DG-N database explorer, object structure/DDL and dependency inspection; DG-SE Search Everywhere across objects, actions and query history |
| State / confidence | N01, N02, N04, N05, N06, N07 and P01 VERIFIED_COMPLETE for the bounded workflow / High |
| Sources | `domain.rs` (`DatabaseObjectKind`, `DatabaseObjectRef`, `DatabaseObjectSummary`, `ObjectDetails`, `ObjectDdlStatus`, `IndexInfo`, `ConstraintInfo`, `TriggerInfo`, `RoutineInfo`, `SequenceInfo`, `DependencyEdge`, `cap_ddl`, `normalize_dependency_edges`), `drivers/postgres/explorer.rs`, `drivers/mysql/explorer.rs`, `drivers/snowflake/catalog.rs`, `commands.rs` (`database_objects`, `object_details`), `build.rs`, `capabilities/main.json`; `explorer-tree.ts`, `Sidebar.tsx`, `ObjectInspectorView.tsx`, `UtilityViews.tsx`, `search-everywhere.ts`, `SearchEverywhere.tsx`, `Workbench.tsx`, `App.tsx`, `studio.ts`, `native-smoke.ts` |
| Observed behavior | Schema explorer categorizes `Tables` (including PostgreSQL partitioned tables), `Views`, `Materialized views`, `Functions`, `Procedures`, `Sequences`, `Triggers` and `Indexes` with WAI-ARIA tree keyboard traversal (`ArrowRight`/`ArrowLeft`/`ArrowDown`/`ArrowUp`/`Home`/`End`/`Enter`), roving `tabIndex`, kind filter chips (`All`/`Tables`/`Views`/`Routines`/`Other`) and substring search. Overloaded PostgreSQL functions/procedures disambiguate via `pg_get_function_identity_arguments`; MariaDB sequences (`TABLE_TYPE = 'SEQUENCE'`) move out of base-table `catalog` into `Sequences`. Table/view tabs gain a `DDL` sub-view and enriched `StructureView` (`Constraints`, `Indexes` with expressions/partial `WHERE` predicates/prefix lengths, `Triggers`); non-tabular objects open dedicated `ObjectInspectorView` tabs with routine/sequence/trigger/index metadata, bounded 1-hop `Uses`/`Used by` dependency chips (click-through navigation) and read-only `DdlPanel` (`Copy DDL`, `Open in SQL editor`, explicit `Unavailable` privilege fallback). `SearchEverywhere` (`⌘K` `All`, `⇧⌘O` `Objects`) searches across commands, open tabs, all connected sessions' `catalog` + `objects` (excluding offline profiles with connected/offline count disclosure) and recent `HistoryEntry[]` |
| Missing behavior | Interactive DDL creation/modification wizards (`N08`), ER diagrams (`D01`), schema diff/migration generation (`D02`–`D05`), Snowflake routine/sequence/DDL discovery beyond configured-database tables/views, multi-hop transitive dependency graphs |
| Security / bounds | Read-only fixed metadata queries on existing bounded metadata lanes; zero automatic reconnects or background polling of offline profiles; zero object names, DDL text or dependency edges persisted to SQLite. `database_objects` capped at 10,000 objects per session; `cap_ddl` truncates definitions at 256 KiB on a valid UTF-8 char boundary with `truncated: true`; `normalize_dependency_edges` removes self-edges/duplicates and caps `uses`/`used_by` at 200 edges per direction; `SearchEverywhere` caps rendered results at 80. Unprivileged roles receive `sequence.last_value = None` and `ObjectDdlStatus::Unavailable` rather than failing inspection; `Open in SQL editor` never auto-executes |
| Defects found and fixed | PostgreSQL `pg_get_function_identity_arguments(p.oid)` preserves parameter names on named-argument routines (e.g. `"a integer"`) and `pg_get_expr(indpred, indrelid, true)` omits outer parentheses (`"qty > 1"`); isolated `r3_Order "Items"` FK target onto disposable `r3_parent_orders` and aligned Rust/Playwright assertions (`4058784`). MySQL 8.4 enables binary logging by default and blocks non-`SUPER` `CREATE TRIGGER`/`CREATE FUNCTION` unless `--log-bin-trust-function-creators=1` is set on the disposable Compose services (`4058784`), preventing a CI `integration` failure |
| Tests | 11 new Vitest cases (`explorer-tree` 3, `ObjectInspectorView` 3, `search-everywhere` 3, `studio` `openObject` 2), 5 new Rust unit tests (UTF-8 DDL capping, dependency cycle/dedup/bound normalization, PG trigger bitmask decoding, PG quoted table/sequence DDL synthesis, MySQL backtick index DDL synthesis), 2 new Rust integration tests (`postgres` overloads/quoted DDL/partial-expression index/cyclic views/unprivileged sequence role; `mysql` routines/triggers/prefix & functional indexes/views/MariaDB sequences across MySQL 8.4 & MariaDB 11.4), 2 new Playwright E2E workflows (`tests/e2e/explorer.spec.ts`), and 2 real-WebView Tauri capability ACL assertions (`database_objects`, `object_details`) in `src/native-smoke.ts` |

Evidence (2026-10-31, macOS Apple Silicon, local Node 24 / Rust 1.99.1):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 116 frontend tests (23 files), 120 standard Rust tests, formatting, contracts, types, production build, Cargo check and strict Clippy; script tests (`git-identity.test.mjs` 6/6, `test_runtime_memory.py` 4/4) |
| Passed | Complete ignored integration suite on task-owned disposable fixtures (`postgres` 21, `mysql` 21 across MySQL 8.4, MariaDB 11.4 & case-mode-1 folded servers on 53406/53407, `redis` 6, `ssh` 2): **50 of 50 tests passed, 0 skipped** (`joinery-r3` torn down via `down -v`) |
| Passed | Full `pnpm test:e2e` with repository files held fixed: **all 43 browser workflows passed** in one run (`1.8m`), including both new `explorer.spec.ts` workflows and existing `studio.spec.ts` palette focus-trap regressions |
| Passed | Clean-commit (`26f5136`, `sourceDirty=false`) `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature, `Joinery.app` `20.66 MiB`, bundle executable SHA-256 `04d4ae86…b000`), `pnpm desktop:smoke:build` and `python3 scripts/native-smoke.py --runs 3 --cycles 10`: three hidden runs (`6,622 / 3,010 / 3,034 ms`), 30 cleanup cycles, main-process peak RSS `106.89–107.41 MiB`, fixture SHA-256 `cc044b44…b83c` (including WebView IPC ACL guards for `database_objects` and `object_details`) |
| Not executed | Production accounts, live Snowflake accounts, Windows/Linux packaged runtime, Developer ID signing/notarization, representative performance qualification |

## Consoles, sessions, SQL files and library batch (Milestone R4) — local on feat/consoles-sessions-files (unmerged)

Branch `feat/consoles-sessions-files` (`78a5283..79878e9`) from fetched
`origin/main` `2625933` (PR #29). It implements Milestone R4 (Consoles,
sessions & files: `M05`, `M06`, `X04`, `E14`, `P02`, `P03`) of
[implementation-roadmap.md](implementation-roadmap.md) and is locally verified,
unmerged and unreleased.

| Field | Consoles, sessions, SQL files and library workflow |
|---|---|
| DataGrip reference | DG-S multi-console session pool, client attachment and manual/auto-commit transaction controls; DG-EC per-session schema/database switching; DG-SQ `.sql` file workflows and saved queries; DG-B bookmarks; DG-RL recent locations |
| State / confidence | M05, M06, X04, E14, P02 and P03 VERIFIED_COMPLETE for the bounded workflow / High |
| Sources | `domain.rs` (`TransactionMode`, `TransactionState`, `SessionContextKind`, `SessionRuntimeState`, `OpenedSqlFile`, `SavedSqlFile`, `SavedQuery`, `SavedQueryInput`, `BookmarkTarget`, `SavedBookmark`, `QueryEvent::SessionState`), `drivers/{postgres,mysql,snowflake}.rs`, `file_capability.rs` (`GrantKind::SqlFile`, `read_sql_file_text`, `write_sql_file_atomic`), `persistence/library.rs`, `persistence/recovery.rs`, `execution.rs`, `commands.rs`, `build.rs`, `capabilities/main.json`; `studio.ts`, `workspace.ts`, `SqlEditor.tsx`, `Workbench.tsx`, `LibraryModal.tsx`, `SearchEverywhere.tsx`, `search-everywhere.ts`, `Sidebar.tsx`, `App.tsx`, `native-smoke.ts` |
| Observed behavior | PostgreSQL and MySQL/MariaDB consoles can attach to named secondary sessions in `sessionPool` (up to four total sessions per connection) or share the primary session, rename/disconnect sessions, and disconnect a pooled session on tab close only when closing its last attached owner. Per-session active schema (`PostgreSQL` `set_config('search_path', …, false)`, `Snowflake` stateless request-scoped schema verification) and active database (`MySQL/MariaDB` `USE`) selectors update live CodeMirror completion immediately without affecting sibling consoles. Per-session `Auto-commit` vs `Manual commit` controls track wire-level `TransactionState` (`Idle` / `InTransaction` / `Failed` on PG `ReadyForQuery` `I`/`T`/`E` and MySQL `SERVER_STATUS_IN_TRANS`), provide explicit `Commit` (`ReadOnly` blocked, audit-logged) and `Rollback` (`Failed` recovery, audit-logged), reject `Manual` → `Auto-commit` switches while a transaction is open (`ErrorCode::Conflict`), and keep table-grid edits isolated on separate lanes. Query tabs open (`⌘O`), save in-place (`⌘S`, reusing `GrantKind::SqlFile`), and save-as (`⇧⌘S`) regular UTF-8 `.sql` files with dirty indicators and unsaved-close/quit guards. `SaveQueryDialog`, `LibraryModal` and `SearchEverywhere` (`Saved` scope) manage persistent saved queries, object/saved-query bookmarks (`⌘D`) and a 50-item in-memory `Recent locations` ring buffer (`⇧⌘E`) |
| Missing behavior | Savepoint UI, persistent live-session/transaction recovery across restarts, Snowflake/Redis interactive transactions, external filesystem file-watcher auto-reload |
| Security / bounds | `GrantKind::SqlFile` capped at 64 active grants and 4 MiB valid UTF-8 regular files (symlinks and NUL bytes rejected); saves write to a sibling `.joinery-sql-<uuid>.tmp` (`0600` on Unix for new files, preserving permissions on existing files), `sync_all`, and atomically `rename`; zero raw filesystem paths cross IPC and file-backed tabs are excluded from `SavedWorkspace.drafts`. Saved queries (`settings` `'saved-queries'`) capped at 256 entries, 256 KiB SQL each and 4 MiB total, defaulting detached when `storeQueryText === false`; bookmarks (`settings` `'bookmarks'`) capped at 256 entries; both are schema-validated during `recovery::validate` and cascade cleanly on `delete_profile`. Recent locations stay in memory (max 50) |
| Defects found and fixed | MySQL `SET SESSION autocommit = 0` can itself set `SERVER_STATUS_IN_TRANS` on an idle connection; `Mysql::close_phantom_transaction` immediately issues `ROLLBACK` when toggling into `Manual` mode from `Idle` so newly switched idle consoles do not display a phantom open transaction. Rendered `<select aria-label="Active schema">` as a sibling before `.completion-context` in `SqlEditor.tsx` so `<option>` labels never leak into `.completion-context` assertions |
| Tests | 6 new Vitest cases (`studio.test.ts` 5 covering session pool/transactions/context revisions, file-backed dirty & draft exclusion, saved queries/bookmarks/50-item recents/profile-delete cascade; `search-everywhere.test.ts` 1), 6 new Rust unit tests (`postgres` auto-BEGIN & `active_schema`, `snowflake` stateless schema switch, `file_capability` `SqlFile` round-trip/limits/symlink/path-hiding, `library` bounds/dedup/cascade & backup-corruption rejection), 2 new Rust integration tests (`postgres` manual-tx/`25P02` failed-state/grid-isolation/schema-switch; `mysql` manual-tx/grid-isolation/database-switch across MySQL 8.4 & MariaDB 11.4), 4 new Playwright E2E workflows (`tests/e2e/consoles-files.spec.ts`), and 12 real-WebView Tauri capability ACL assertions in `src/native-smoke.ts` |

Evidence (2026-10-31, macOS Apple Silicon, local Node 24 / Rust 1.99.1):

| Outcome | Evidence |
|---|---|
| Passed | `pnpm validate`: 122 frontend tests (23 files), 126 standard Rust tests, formatting, contracts, types, production build, Cargo check and strict Clippy; script tests (`git-identity.test.mjs` 6/6, `test_runtime_memory.py` 4/4) |
| Passed | Ignored integration suite on task-owned disposable fixtures (`postgres` 22, `mysql` 21 across MySQL 8.4 & MariaDB 11.4, `redis` 6, `ssh` 2): **51 of 51 executed tests passed, 0 failed** (`joinery-r4` torn down via `down -v`) |
| Passed | Full `pnpm test:e2e` with repository files held fixed: **all 47 browser workflows passed** in one run (`2.3m`), including all 4 new `consoles-files.spec.ts` workflows and all 3 `sessions.spec.ts` + 7 `completion.spec.ts` regressions |
| Passed | Clean-commit (`79878e9`, `sourceDirty=false`) `pnpm desktop:build`, `pnpm desktop:bundle:mac` (strict ad-hoc signature, `Joinery.app` `21.07 MiB`, bundle executable SHA-256 `2d16256a…f054`), `pnpm desktop:smoke:build` and `python3 scripts/native-smoke.py --runs 3 --cycles 10`: three hidden runs (`5,842 / 3,182 / 3,274 ms`), 30 cleanup cycles, main-process peak RSS `102.44–107.66 MiB`, fixture SHA-256 `06af4333…5f5a` (including WebView IPC ACL guards for all 12 new R4 commands) |
| Blocked by environment | Case-mode-1 MariaDB fixture: port 53407 is held by the host agent process, so `mysql_live_completion_uses_server_case_policy` was skipped locally (covered by CI `integration` job); host port 55432 is held by an external process so local PostgreSQL fixtures ran on `JOINERY_POSTGRES_PORT=55433` |
| Not executed | Production accounts, live Snowflake accounts, Windows/Linux packaged runtime, Developer ID signing/notarization, representative performance qualification |
