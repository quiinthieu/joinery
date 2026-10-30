# Joinery implementation roadmap

This delivery sequence supplements [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).
The [feature matrix](datagrip-feature-matrix.md) records engineering evidence;
the owner-private [project hub](https://joinery-project-hub.otohime-nokimi.chatgpt.site)
records shipping status. Priority P0–P3 is not the blueprint's P0–P7 milestone
number. Existing released capabilities and external gates retain their history.

## Agreed first batch

The user selected SQL correctness, then a bounded repair across existing SQL
engines before live role/search-path tracking or JOIN generation. Baseline source
is `bbba421`; baseline validation and two synthetic Snowflake browser workflows
passed. Initial shell-toolchain and Chromium sandbox blockers were environmental
and resolved. No production access is needed or authorized.

| Slice | Value, priority, complexity | Dependencies and observable acceptance |
|---|---|---|
| Completion scope/quoting/invalidation (E01, E03–E05) | Trust the identifiers inserted while writing SQL; P1, Medium | Existing CodeMirror and worker. Ignore comments/strings, isolate caret query block, respect CTE/derived shadows, preserve quoted identity and insertion, reject obsolete responses. Regression corpus and browser acceptance pass |
| Live completion context (E02, I04, M05) | Match the actual SQL session; P1, High | Rust-owned context DTO/revision and engine adapters. Database, authenticated identity, role and search-path changes invalidate caches; no metadata reuse across contexts or automatic replay |
| Console/session binding (M06, E14, X04) | Make transaction ownership visible; P1, High | Existing four-session backend limit. Explicit console binding; cancel/disconnect affects only its owned session; grid commits remain independent |
| FK metadata and JOIN completion (N03, E06) | Write correct joins quickly; P1, High | Constraint identity plus ordered source/target columns, then scoped identifier insertion. Composite/multiple/self-referencing relationships produce correct reviewed text without executing it |
| Diagnostics and parameters (E07, E15, X08) | Find errors and rerun safely; P1, High | Driver error positions, Unicode offset mapping, typed binding contracts. Exact statement navigation and bindings included in immutable authorization plans |

### First-slice implementation contract

- Keep Rust connections, metadata, credentials, execution and approval ownership.
  No Tauri command or persistence migration is required for this repair.
- Use CodeMirror dialect token trees and a bounded identifier/scope reader.
  Current query blocks expose direct base-table bindings only. CTE and derived
  aliases shadow physical tables; derived outputs and outer correlated bindings
  are deliberately not inferred. Incomplete/unsupported analysis suppresses
  contextual columns rather than guessing.
- Preserve original identifier segments. PostgreSQL folds unquoted identifiers
  to lowercase; Snowflake to uppercase. MySQL matches exact catalog spellings
  first and declines ambiguous case-folded matches. Known configured default
  schemas retain precedence; ambiguous unresolved table names require qualification.
- Keep keyword/catalog lookup in the existing worker. Distinguish document,
  context and installed-index revisions; do not rebuild the catalog on ordinary
  keystrokes. Worker responses must identify the installed index, and the editor
  must reject stale document/caret/session/metadata results.
- Retain at most eight active referenced-table requests and 100 returned
  suggestions. Clear obsolete cache entries; an older failed request must not
  delete its replacement. A denied table must not erase successful metadata for
  other current bindings.
- Insert valid dialect quoting, preserving surrounding SQL and UTF-16 ranges.
  PostgreSQL/MySQL/Snowflake share the existing architecture; Redis keeps its
  native grammar and command completion.

### First-slice result (2026-10-28)

The bounded repair is implemented locally. See the matrix's final evidence
section for scope and outcomes: 59 frontend tests, 83 Rust tests, 20 browser
workflows and the six-workflow final completion/Snowflake rerun pass. Three
selected read-only Rust integration workflows additionally verify cleanup,
metadata paging and TLS hostname checks. Native packaging results are recorded
in `IMPLEMENTATION_STATUS.md`. That repair subsequently shipped in preview.12
through PR #20 (`06146bc`); its original local evidence remains historical.

The 100,000-name index meets the warm 5 ms lookup target in the measured browser
sample (p95 0.10 ms). Warm explicit visible completion misses the 50 ms target
(p95 55–61 ms); the installed CodeMirror scheduler adds a fixed 50 ms before
requesting explicit suggestions. Treat visible scheduling/typing latency as a
measured follow-up, without patching dependency internals or claiming native
performance qualification. Keep cold metadata waits and large-catalog visible
measurements separate.

The PostgreSQL live-context slice shipped in preview.13 (PR #21, `e8f1cec`); see
[live-completion-context.md](live-completion-context.md) for the actual contract,
resource bounds and verification scope. E02/I04 remain PARTIAL across all engines.
The [MySQL/MariaDB live adapter](mysql-live-completion-context.md) shipped in
preview.14 (PR #22, `dc220b0`), with mode-0 fixtures and clean-source release
evidence. PR #23 adds mode-1 qualification on both disposable engines.
Mode 2 needs a case-insensitive filesystem. The bounded
[Snowflake request-context slice](snowflake-completion-context.md) is implemented
locally; broader search-path/quoted-policy support and live qualification remain
open. External metadata changes still require refresh.
Independent console/session binding and grouped-FK JOIN generation follow.
External release gates remain open and need their own authorization/scope.

### Acceptance and verification contract

1. Reproduce and then fix references in comments, strings, dollar quotes and
   separate statements; cover semicolons in literals, CTE/derived shadows, UNION
   arms, nested blocks, aliases and malformed/incomplete documents.
2. Exercise case twins, embedded dots/delimiters, Unicode, qualified names,
   same-name schemas and quoted-prefix insertion. Accept a suggestion and run a
   harmless SELECT against disposable PostgreSQL/MySQL/MariaDB fixtures; use
   synthetic Snowflake API responses and disclose that boundary.
3. Delay metadata/worker responses while editing, moving the caret, refreshing
   metadata, switching connections and closing tabs. Verify obsolete responses
   disappear, promises/timers settle, and partial metadata errors degrade safely.
4. Measure warm prefix lookup on 100,000 names and visible suggestions separately.
   Compare with the blueprint's p95 5 ms lookup/50 ms visible targets; report
   environment and any failures without generalizing browser numbers to WebView.
5. Run `pnpm validate`, relevant integration/browser tests, `pnpm desktop:build`,
   `pnpm desktop:bundle:mac`, and hidden native smoke. Record Passed, Failed,
   Skipped, Blocked by environment, and Not executed separately.

## DataGrip replacement program (approved 2026-10-30)

The owner asked for Joinery to replace DataGrip on the existing engines
(PostgreSQL, MySQL/MariaDB, Snowflake, Redis), and approved this
milestone order. It supersedes the order of the "Subsequent phases" table
below without changing its exit criteria. Console/session binding moves to R4
so table-data parity, which also supplies grouped FK metadata for JOIN
completion and diagrams, ships first. Each batch needs its own approved plan,
fixture evidence and PR.

| Milestone | Scope (feature IDs) | Exit criteria |
|---|---|---|
| R1 Data view parity | Batch 1: N03, G03 typed, G08 outbound. Batch 2: G05, G09, G10, G07, raw WHERE/ORDER BY text classified by `safety::prepare`, inbound referencing rows | Sorted/filtered pages equal reference queries; composite FKs navigate; paste all-or-nothing; inserts/deletes via one-use plans |
| R2 Data movement | G11 export, copy-as formats, G12 CSV import, G13 data compare | Exact values round-trip; cancelled exports marked partial; no result rows in app data |
| R3 Explorer and search | N01, N02, N04–N07, P01 | Keyboard tree; quoted/overloaded names; per-object permission degradation |
| R4 Consoles, sessions, files | M05, M06, X04, E14, P02, P03 | Isolated console transactions/cancellation; unsaved-file guard |
| R5 Editor intelligence | E06, E07/X08, E15, E11, E13, E09, E12, E16 | Correct composite JOIN text; multibyte error carets; bindings in approval digest |
| R6 Query plans | X07, A01, A02, X05 | ANALYZE explicitly authorized; estimates and actuals distinct |
| R7 Database design | D01–D05, N08 | DDL round-trips on disposable schemas; no unattended apply |
| R8 Productivity and monitoring | P05, P06, A03–A05, E08, E10 | Shortcut conflicts detected; monitoring permission-aware |
| RX Restriction relaxation | G14 single-table result editing, PK-less row editing, Snowflake DML (A07) | Rust-proven provenance; one-use plans and production passphrase retained; Snowflake writes qualified only on an approved non-production account |
| v1.0 gates | I05–I07 | Unchanged external signing, platform and performance gates |

### R1 batch 1 result (2026-10-30, local branch `feat/table-data-view`)

- FK constraints carry name and ordinal-paired columns; Structure lists them.
- `browse_table` accepts an optional typed `BrowseQuery`. Rust rebuilds the SQL
  from the current schema with bound values, PK tie-breakers, explicit NULL
  order (ascending last, descending first) and a NULL-safe keyset seek.
  Engines without `tableFiltering` (Snowflake, Redis) reject non-empty queries.
- Header clicks sort (Shift adds a key); a filter bar builds typed predicates;
  controls lock while edits are pending. ⇧⌘B, the palette or the inspector
  opens the referenced row in a new tab, with a chooser for multi-key columns.
- Two pre-existing defects found by the new tests are fixed separately:
  MySQL read-only profiles could not browse editable tables, and the frozen
  primary-key column stayed blank for pages loaded after scrolling.

See the matrix rows N03/G03/G08 and `IMPLEMENTATION_STATUS.md` for evidence.
Not pushed, merged or released.

## Subsequent phases and blueprint cross-reference

| Phase | Feature IDs / user value | Priority / complexity | Dependencies and exit criteria | Blueprint relationship |
|---|---|---|---|---|
| 0 Reliability | M01–M03, X01–X03, G04/G06, I01/I02/I07; preserve working workflows | P0 / High | Fixture engine matrix, cancellation/cleanup, Safe Mode, budgets and recovery pass on current revision; external gates remain explicit | Continue existing qualification batch B/E, do not reopen shipped engines as greenfield work |
| 1 SQL editor | First-batch slices above, then E09–E13/E16 | P1–P2 / High | Session metadata and error contracts before context/navigation; snippets/formatting stay local and never execute automatically | Extends SQL Studio/schema indexing; scoped defects first |
| 2 Explorer | N01–N08, A03; inspect objects without hand-written catalog SQL | P1–P2 / High | Typed engine object identities, bounded catalog/DDL, permission states; object changes use reviewed Rust plans | Extends existing catalog; grouped FKs pulled forward as JOIN prerequisite |
| 3 Grid | G03/G05/G07–G13; filter, edit and move data safely | P1–P2 / High | Typed sort/filter requests with PK tie-breakers and NULL ordering; atomic paste; row mutation types before insert/import; explicit bounded Rust export | Retains existing provenance, overlays, transactions and memory-only results |
| 4 Analysis | X05/X07/A01/A02/A04/A05; explain execution behavior | P2 / High | Engine capabilities and raw/normalized plans agree; estimated and actual metrics distinct; ANALYZE explicitly authorized, never auto-run | Advances query-plan stretch goal after safety/metadata prerequisites |
| 5 Design | D01–D05/N07; understand and review schema changes | P2 / High | Metadata → snapshots → deterministic diff → immutable reviewed migration; no automatic schema synchronization | Advances diagrams/migration stretch goals |
| 6 Productivity | P01–P07/E12/E14; navigate and reuse work | P1–P2 / Medium | Extend existing palette/Zustand/persistence; keyboard workflows, opt-in SQL storage and dirty-state protection | Retains single-window Joinery identity and default shortcuts |
| 7 Hardening/expansion | I03/I05/I06, new engines; prove performance and distribution | P0 release gates, P3 new engines / High | Measure throughout, not only at the end. Actual platform installers/UI, signing/update integrity and attributable memory/startup/FPS. Every new engine gets independent dialect, metadata, capability, security and fixture contracts | Existing P7 gates remain; SQLite/SQL Server/Oracle/BigQuery are separate projects |

## Release and architecture constraints

All application changes go on a task branch from fetched `origin/main`; use the
repository owner-identity scripts, inspect diffs, and make small passing
Conventional Commits with regression tests. Do not push, merge, publish or install
an application release without authorization. Update matrix evidence and the
private hub after implementation/verification; record local-only implementation
as unmerged and unreleased. Keep hub access owner-private.

No new state manager, execution system, editor, grid or adapter architecture is
introduced. The scoped repair fits the blueprint; the subsequent live context
contract is recorded in [architecture-decisions.md](architecture-decisions.md).
Future interface changes must preserve Rust validation,
one-use approvals and explicit memory budgets. No live credentials, production
tables, signing setup or external accounts are prerequisites for the first slice.

Snowflake remains read-only; Redis Cluster/Sentinel/streams/PubSub and external
MySQL process killing remain unsupported. Arbitrary SQL results remain read-only.
Full derived/CTE output inference, live session-context tracking for remaining
engines and JOIN generation remain open after the PostgreSQL context workflow.
