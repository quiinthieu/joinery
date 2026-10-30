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
| G | `src/features/grid/DatabaseGrid.tsx`, `ValueDialog.tsx`; `src/data/{dirty-store,result-cache,columnar}.ts`; table paging in `src/state/studio.ts`; backend `prepare_changes_batch` and driver `apply` | Dirty/cache/columnar tests and database/browser edit tests. Glide canvas, sparse overlay, primary-key provenance, atomic update plans, typed server sort/filter and outbound FK navigation (table-data batch). No arbitrary-result editing, PK editing, insert/delete grid workflow or atomic multi-cell paste handler |
| N | `src/features/connections/Sidebar.tsx`, `src/features/studio/UtilityViews.tsx` (`StructureView`, `ProcessesView`, `HistoryView`) | Browser studio/mysql/snowflake specs. Selected connection's schema/table tree, substring filter, columns/types/PK/FK display. No general object kinds, DDL/dependency API, or graphical plans |
| W | `src/state/{studio,workspace}.ts`, `src/app/{App,Workbench}.tsx`, `Preferences.tsx`; backend `persistence/workspace.rs` | Workspace/studio/modal/tab unit tests and browser specs. Shared primary sessions plus local tab-owned SQL sessions; opt-in drafts exclude live bindings. File I/O and named/multi-client session management remain open |
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
| M05 Multiple databases/schemas — DG-C, DG-EC | D, N, E | PARTIAL / H | Configured database plus discovered schemas; add explicit context selection and invalidation | P1 / High | Session context; PMS; read | Same-name objects across databases/roles / hold |
| M06 Session management — DG-S | D, W | PARTIAL / H | Local PM isolated-tab ownership uses existing four-session limit; named sessions, multi-client attachment and broader controls remain open | P1 / High | Context identity; PM, restricted S/R; engine | Isolation/cancellation/close, full browser and native gates pass locally; named-session controls remain open / hold |
| M07 Driver capabilities/configuration — DG-C | D | PARTIAL / H | Six capability booleans exist; finer object/analysis/auth capabilities need contracts | P2 / Medium | Driver trait/DTOs; PMSR; engine | Unsupported controls disabled and backend rejects / hold |

## Database navigation

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| N01 Hierarchical schema/table/view explorer — DG-N | N, D | PARTIAL / H | Tables/views under selected connection; distinguish object kinds and keyboard tree semantics | P1 / Medium | Catalog; PMS; read | Keyboard traversal and view-specific actions / hold |
| N02 Object filtering/search — DG-N, DG-SE | N | PARTIAL / H | Substring table filter exists; object-kind filters and cross-connection search missing | P1 / Medium | Object index; PMS; read | Search matches, empty/disconnected/denied states / hold |
| N03 Columns, PKs and FKs — DG-N, DG-FK | D, N | VERIFIED_COMPLETE / H (bounded table-data batch) | FK constraints carry name and ordinal-paired columns; Structure lists constraints. Inbound referencing-row navigation remains open | P1 / Medium | Metadata DTOs; PM (S declares none); read | Composite/multi-FK fixture tests on PG/MySQL/MariaDB; see table-data batch / hold until merged |
| N04 Indexes and constraints — DG-N | D, N | MISSING / H | Add bounded metadata and inspect definitions before mutation tools | P1 / Medium | Object metadata; PMS by capability; read | Unique/partial/expression indexes and denied metadata / hold |
| N05 Triggers, functions, procedures, sequences — DG-N | D, N | MISSING / H | Add engine-specific discovery and inspection | P2 / High | Object identity/capabilities; PMS; read | Overloads, quoting and permission restrictions / hold |
| N06 DDL viewing/object navigation — DG-N | N, D | MISSING / H | Fetch backend-owned definitions and navigate from explorer/editor | P1 / Medium | Object metadata; PMS; read | Quoted definitions and unavailable DDL / hold |
| N07 Object dependency inspection — DG-N | D, N | MISSING / H | Add bounded dependency edges, cycles and navigation | P2 / High | N05/N06; PMS by capability; read | Cycles, missing objects and partial permissions / hold |
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
| E14 Multiple consoles/files — DG-S, DG-SQ | W | PARTIAL / H | Query tabs plus local PM tab-owned sessions; file I/O and named session attachment remain open | P1 / High | M06, native file capability; PMSR; local | Unsaved changes, session isolation, file permissions / hold |
| E15 Query parameters — DG-P | X; no typed editor parameter workflow | MISSING / H | Bind values through engine adapters and include bindings in reviewed plan | P1 / High | Typed execution request; PMS by capability; gate | Literal/comment exclusion, NULL/types, approval digest / hold |
| E16 Context documentation — DG-DOC | E, N | MISSING / H | Metadata documentation at caret with permission-aware fallback | P2 / Medium | E11; PMS; read | Correct object, stale/denied metadata / hold |

## Query execution

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| X01 Statement/script execution — DG-X | X, E | IMPLEMENTED_UNVERIFIED / H | Preflight and sequential execution exist; rerun exact statement/selection/batch fixtures | P0 / Medium | Drivers/safety; PMS, R command grammar; gate | Partial failures, comments, unknown SQL / existing |
| X02 Multiple result sets — DG-X, DG-G | X, B, W | IMPLEMENTED_UNVERIFIED / H | Results/subtabs exist; verify prior results survive later failures | P0 / Medium | Retention; PMS; read | Empty/multiple result sets and later errors / existing |
| X03 Async execution/cancellation — DG-X | X, D | IMPLEMENTED_UNVERIFIED / H | Owned cancellation/quarantine exist; rerun races and cleanup | P0 / High | Jobs/session lifetime; PMSR; engine | Queued/active/late cancellation, lost acknowledgements / existing |
| X04 Transactions/session controls — DG-S | X, D, W | PARTIAL / H | Explicit SQL transactions work on PM; local isolated session actions and close warnings exist; server transaction state/autocommit/savepoint UI remain absent; S/R transactions excluded | P1 / High | M06; PM; gate | Targeted two-console state/concurrency/cancellation/close tests pass; final regression/native gates blocked / hold |
| X05 Execution metrics — DG-X, DG-Q | QueryEvent, W | PARTIAL / H | Client elapsed/affected rows exist; queue/first-row/server timing missing | P2 / Medium | Driver metrics; PMSR; read | Correct units and no client/server-time conflation / hold |
| X06 Query history — DG-SQ | N, L, X | IMPLEMENTED_UNVERIFIED / H | Search/reopen and opt-in text exist; rerun privacy and replay paths | P1 / Low | Persistence/plans; PMSR; local | Metadata-only history and explicit rerun authorization / existing |
| X07 Raw EXPLAIN — DG-Q | X, safety AST visitor | PARTIAL / H | SQL entry point exists; capability-aware action and plan result typing missing | P2 / Medium | A01; PMS by capability; gate | Nested writes and ANALYZE approval / hold |
| X08 Error reporting/navigation — DG-X | AppError, QueryEvent.Error, W | PARTIAL / H | Error panel exists; statement source ranges/navigation absent | P1 / Medium | E07; PMSR; read | Unicode offsets and missing server positions / hold |

## Data exploration and editing

| ID / feature / DataGrip ref | Joinery evidence | State / confidence | Missing behavior → acceptance | Priority / complexity | Dependencies; engines; security | Tests needed / release |
| G01 Virtualized tabular browsing — DG-G | G, B | IMPLEMENTED_UNVERIFIED / H | Glide/million-row synthetic grid exists; rerun responsiveness and native evidence | P0 / Medium | Arrow/cache; PMS; read | Fast scrolling, empty/wide values; FPS separate / existing |
| G02 Pagination — DG-G | G, D | PARTIAL / H | PM keyset paging exists; keyless and S previews intentionally bounded | P1 / High | Stable row identity; PMS; read | Composite keys, evicted pages, changing live rows / hold |
| G03 Server sorting/filtering — DG-G | `drivers/browse_query.rs`, `Driver::browse_query`, TableFilterBar | VERIFIED_COMPLETE / H (typed filters) | Typed multi-key sort with PK tie-breakers and explicit NULL order; bound typed filters. Raw WHERE/ORDER BY text (R1 batch 2) and Snowflake remain open | P1 / High | Paging/provenance; PM; read | Paged-vs-reference fixtures, hostile values, browser sort/filter/lock flows / hold until merged |
| G04 Cell editing — DG-G | G, X | IMPLEMENTED_UNVERIFIED / H | Typed table edits exist; rerun write/readback and unsupported-type gates | P0 / High | Provenance/change plans; PM; gate | Constraint/conflict/rollback tests / existing |
| G05 Multiple-cell paste — DG-G | DatabaseGrid.onCellEdited only | MISSING / H | Validate complete paste before changing the sparse overlay | P1 / Medium | Dirty budget/type validation; PM; gate | Mixed invalid cells, NULL/empty and all-or-nothing local update / hold |
| G06 Dirty tracking and review/commit/rollback — DG-G | G, X | IMPLEMENTED_UNVERIFIED / H | Sparse edits and atomic server commits exist; verify across retained pages | P0 / High | Original identity; PM; gate | Revert, discard, conflict, lost commit acknowledgement / existing |
| G07 JSON/array/binary/large-value viewers — DG-V | ValueDialog, columnar, D | PARTIAL / H | Text/JSON and bounded previews exist; dedicated array/binary viewers absent | P2 / Medium | Lossless value metadata; PMSR; read | Fidelity, size caps and unsupported edits / hold |
| G08 FK data navigation — DG-FK | App `followKey`, `referencedRowsQuery` | VERIFIED_COMPLETE / H (outbound) | Outbound referenced-row tabs with composite pairing, multi-key chooser and NULL refusal; inbound referencing rows remain open | P1 / Medium | N03/G03; PM; read | Browser single/composite/chooser/NULL flows / hold until merged |
| G09 Multi-row insert/delete — DG-G | G; backend changes are updates | MISSING / H | Add explicit row mutation kinds and immutable reviewed plans | P2 / High | Defaults/generated values/identity; PM; gate | Atomic batch, returned identities and uncertain outcomes / hold |
| G10 Column management — DG-G | G | PARTIAL / H | Resize and first-PK freeze exist; hide/reorder/persist configuration missing | P2 / Medium | Stable column identity; PMS; local | Schema refresh and reordered paste alignment / hold |
| G11 Export — DG-EX | B/G bounded clipboard copy only | MISSING / H | Explicit destination and bounded Rust streaming export | P1 / High | Native file capability/stream ownership; PMS; local | Cancellation, exact values, partial files, no implicit disk retention / hold |
| G12 Import — DG-IM | X, no import workflow | MISSING / H | Mapping/validation/preview and backend-authorized batches | P2 / High | G09, native file capability; PM; gate | Invalid rows, budgets, rollback and no automatic retries / hold |
| G13 Data comparison — DG-G | G | MISSING / H | Key-aware bounded comparison with explicit snapshot/live semantics | P2 / High | Identity and streaming; PM first; read | Duplicates, NULL/type differences and bounded memory / hold |
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
| P01 Search Everywhere/object symbol search — DG-SE | N substring filter, W command palette | PARTIAL / H | Unified object/action/query navigation with scoped metadata | P1 / Medium | Object identity/index; PMSR; read | Keyboard search and inaccessible/offline scopes / hold |
| P02 Recent locations/bookmarks — DG-RL, DG-B | W history and tab state only | MISSING / H | Bounded location history and named bookmarks | P2 / Medium | Stable document/object IDs; PMSR; local | Reopen/missing-object and privacy behavior / hold |
| P03 Saved queries/scratch files — DG-SQ | W opt-in drafts/history | PARTIAL / H | Named saved-query library and explicit file operations | P2 / Medium | E14/persistence; PMSR; local | Opt-in content, overwrite and recovery / hold |
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
| Release eligibility | Locally verified, unreleased. Review and clean-source release provenance remain before shipping; see isolated-query-sessions.md |

## Table data view batch (R1 batch 1) — local branch

Branch `feat/table-data-view` from fetched `799e600`. It implements the first
batch of the approved DataGrip-replacement program in
[implementation-roadmap.md](implementation-roadmap.md). It has not been pushed,
merged, released or installed.

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
