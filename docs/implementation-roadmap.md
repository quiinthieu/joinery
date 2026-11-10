# Joinery implementation roadmap

## Confirmed owner target (9 November 2026)

The owner requires **Snowflake on macOS with SSO `authenticator=externalbrowser`**,
and **everything DataGrip currently provides for that workflow**, explicitly
including reads, writes, schema changes and multi-statement transactions.
The [Snowflake implementation queue](snowflake-datagrip-replacement.md) records
the required workflow inventory and qualification sequence.
This is not a read-only-pilot or MVP target. All applicable capabilities in the
official DataGrip baseline remain required until the owner explicitly excludes
one; existing exclusions are blockers, not optional enhancements.

The owner also requires the **latest stable DataGrip release** and Snowflake
query results up to **1,000,000 rows**. Current comparison: DataGrip 2026.3,
released 5 November 2026. Complete million-row paging/export with bounded memory
is required. The [opt-in temporary result store](large-results.md) now targets
one million rows with bounded memory/disk. Earlier actual PostgreSQL paging/export
and browser/native preference checks pass; their source and artifact identities
are recorded in the verification ledger.
Actual Snowflake million-row behavior remains unqualified.

Current Joinery implements explicit local-application OAuth/SQL API and stateful
SDK external-browser profile routes. Both enforce ReadOnly. SDK same-session
completion is implemented and locally qualified separately. A distinct
revision-bound relation catalog/inspection/read-only preview is now implemented
locally; see the [browsing contract](snowflake-sdk-browsing-design.md) and
[current qualification](replacement-verification.md#sdk-relation-browsing-10-november-2026).
Generic editable browsing, complete object/constraint inspection, writes, DDL
and transactions remain incomplete. Full account/session compatibility remains
unqualified. These are primary replacement blockers. Local
PostgreSQL/MySQL safety repairs remain useful application work but do not close
Snowflake acceptance. External account qualification needs explicit non-production
scope and permission for any billable activity/writes; no private DataGrip state
or credentials will be inspected.

The current [session observer](snowflake-sdk-session-observation.md) implements
bounded, same-connection AUTOCOMMIT/policy/opaque-transaction observations,
nullable roles and explicit Unknown states. Stable observations preserve cached
plans; changed or unavailable observations revoke them. Local qualification
passes; the [ledger](replacement-verification.md#sdk-session-observation-10-november-2026)
records current tests, packaging and native evidence. SDK schema switching and
transaction controls remain disabled;
actual account observation remains unqualified. Its ten/eleven additional SQL
executions are outside the earlier two-query canary approval.

An [offline qualification manifest](snowflake-qualification-manifest.md) is now
implemented and locally verified: 12 synthetic tests and actual CLI checks with
synthetic identifiers, current source/runtime validation and private exclusive
output. The distinct observer/options/scalar workflow has a complete 24-SQL
ledger. An executable runner and fresh account approval remain open; this tooling
does not qualify the account or normal application.

The preceding relation-browsing increment is locally qualified: **619 frontend / 338
standard Rust** tests, **29** synthetic-SDK actual-helper adapter tests, the
application/helper browsing flow, **11** focused browser workflows and an actual
PostgreSQL context/transaction regression pass. Fresh normal SDK macOS packaging,
signature/startup and **19** hidden native check categories with ten cleanup
cycles pass. The ledger records source hashes, artifact identities and explicit
account/installed-application limits. This does not qualify Snowflake account
metadata or enable writes, DDL or transactions.

The current [SDK application integration](snowflake-sdk-integration.md) adds explicit
authentication modes, fixed native runtime selection, retained cleanup authority,
pending launch permits and a reviewed opt-in resource-packaging route. Focused
checks and combined validation pass (604 frontend / 297 standard Rust tests),
as do 124 actual fixture tests and normal SDK resource-bearing macOS packaging,
ad-hoc signature and packaged SDK hello/EOF startup. All 143 browser workflows and 19 hidden SDK native checks with ten cleanup
cycles pass, as do two standalone PostgreSQL native recovery pairs.
The subsequent [SDK same-session completion](snowflake-sdk-metadata-design.md)
passes local qualification: 607 frontend / 312 standard Rust tests, 124 real-engine
regressions, all 145 browser workflows, fresh SDK-bearing packaging/startup and
19 hidden native check categories. Complete browsing/session context and the
required write/transaction workflows remain in the queue. No target capability
was made optional.
A separately approved two-query SDK helper canary now passes actual SSO,
configured context, session continuity, synthetic scalar values and local
cleanup. The ledger preserves its first validator failure and corrected rerun;
broader account SQL and writes still require concrete authorization.

The default Snowflake SQL starter repair reproduces the exact script's
`PolicyBlocked` failure in both policy and actual-helper tests, then adds strict
Snowflake-only zero-argument `CURRENT_USER()` / `CURRENT_VERSION()` observers.
Qualified/quoted functions, unknown functions and relative-history forms remain
blocked. All 19 safety and 19 synthetic-SDK actual-helper tests pass, as do
607 frontend / 314 standard Rust tests and full validation; see the
[repair evidence](replacement-verification.md#snowflake-starter-repair-10-november-2026-local-checks).
Complete browsing and authoritative session state remain required.

The reviewed [browsing/session proposal](snowflake-sdk-browsing-design.md)
defines the catalog → inspect → bounded preview → refresh workflow, qualified
targets and truthful unknown transaction/context states. The separate
[submission guard](snowflake-sdk-submit-guard-design.md) now implements a
process-local boundary against SDK retransmission and passes local qualification:
159 helper tests, 607 frontend / 314 standard Rust tests, focused browser flows,
fresh SDK-bearing macOS packaging/startup and 19 hidden native check categories.
The browsing contract now has a local catalog/column-inspection/preview
implementation. Truthful session observation is now locally qualified;
typed schema switching remains planned under the
reviewed [session observation/switching contract](snowflake-sdk-session-design.md);
the complete browsing/session workflow is not finished. Neither is
account qualified. The guard's actual pinned-SDK fault, redirect, proxy and cleanup
evidence establishes a bounded submission foundation; that foundation alone
must not enable writes or transactions.

The preceding stateful transport foundation is in the repository: a pinned SDK helper
passes 31 local tests (six use the actual SDK against loopback HTTP), and its
Rust supervisor passes eight unit and 30 integration tests. The latter includes
explicit cleanup receipts for unused/failed launches and actual child reaping.
The read adapter adds 11 integration tests, including exact typed binding and
terminal/cancellation ordering. Nine runtime-preparation tests, actual offline
macOS arm64 staging and staged SDK startup pass; the stage is not an app bundle.
The [driver design](snowflake-driver-design.md) records the observed SDK retry
behavior and remaining gates. Those foundation counts predate the current profile/cleanup integration. Current
resource packaging now passes locally. Normal packaged SSO/session behavior,
writes and transactions retain separate open qualification gates.

The preceding lifecycle increment added reserved connection-attempt identifiers, cancellation
before/during authentication, exact retirement of late responses, and a native
shutdown publication barrier. Focused UI tests pass; independent review exposed
startup socket/task ownership gaps in PostgreSQL, SSH and MySQL. Their focused
repairs and 124 actual database fixtures now pass. Combined qualification passes
598 frontend / 262 standard Rust tests, all 141 browser workflows, three native
smoke runs, two actual PostgreSQL recovery pairs and the normal ad-hoc macOS
bundle. This is prerequisite lifecycle work for
SDK integration; it does not implement externalbrowser SSO. See the
[active verification record](replacement-verification.md#connection-startup-increment-10-november-2026-locally-qualified).
The [connection ownership contract](connection-lifecycle.md) records cancellation,
preflight, resource bounds and shutdown limits. The current SDK integration implements profile routing and retained cleanup;
hidden native qualification now passes. SDK completion now has an implementation;
complete session metadata, browsing and the required write/transaction workflows
remain open.

The active 9 November 2026 Goal and next qualification queue are in
[daily replacement acceptance](replacement-acceptance.md). The required Snowflake externalbrowser/macOS target is confirmed above;
historical slice completion does not establish daily replacement acceptance.

The earlier P0 repairs cover query-result provenance, truthful bounded snapshots,
abrupt backend restart, manual transaction outcomes and delayed result inspection.
Their historical qualification remains below and in the ledger. The current
identity-safety increment has separate, narrower evidence and open gates.
The first pinned run passed validation, 91 fixtures, bundle and native smoke;
116/118 browser workflows passed and the two failures were reproduced. The
repaired runtime passed normal macOS packaging/signature, three isolated
native processes and 120/120 browser workflows. The repairs have independent
reviews; no material finding remains. The native-test-only follow-up `a1ab58b`
passes full validation (542 frontend / 180 standard Rust), two isolated actual
native close/reopen pairs against PostgreSQL, six runner acceptance tests and
ordinary smoke compatibility. Pending drafts/preferences survive close, manual
writes roll back and native invocation counters show no replay before observation.
Normal packaged restart and credentials remain unqualified. Next: confirm
required workflows and engine/authentication pairs, qualify authorized external combinations, and
obtain the owner walkthrough acceptance. Follow the
[verification ledger](replacement-verification.md); source presence and old
slice test counts do not close these gates.

The reviewed-writer safety follow-up repairs synchronous PostgreSQL retirement,
blocks queued writes after session closure, and fixes CSV unknown-outcome text
and editor retirement. Real regressions reproduced the defects before repair;
independent reviews found no material issues. Validation (545 frontend / 180
standard Rust), 103 real fixtures and four new fault browser workflows pass.
Combined packaging/signature, two native recovery pairs, ordinary smoke and all
124 browser workflows pass for that earlier source. A separate diagnostic on
`ff59ca6` reproduced 27 drop/recreate identity failures (nine each on PostgreSQL,
MySQL and MariaDB), including deletes, empty-result inserts and CSV between batch
commits. The PostgreSQL identity repair is committed locally and the current
MySQL/MariaDB increment implements bounded retained metadata-lock guards,
fail-closed original-generation checks and exact CSV preview ownership.
Preliminary integrated validation passed 580 frontend / 193 standard Rust,
123 real fixtures, 27/27 strict three-engine identity scenarios and all 137 browser
workflows. A later actual PostgreSQL regression exposed tokenless CSV association
with a newer preview; the legacy path is removed and named IPC requires the exact
token. Verified TLS-over-SSH now exercises actual guarded browse and approved
writes on both MySQL and MariaDB. Final source checks for local code `997680e`
pass 581 frontend / 193 standard Rust, 124 actual fixtures, all 27 strict scenarios
and all 137 browser workflows. Clean-source `6218e60` hidden native recovery,
ordinary smoke and normal ad-hoc macOS bundle qualification also pass; see the
ledger for artifact identities and limits. Historical `5762ee5` native/bundle
qualification belongs to the earlier PostgreSQL increment. See the
[identity investigation](table-identity-safety.md) and
[retained-guard contract](mysql-table-identity-design.md).

Next after local integration: implement the confirmed Snowflake externalbrowser
stateful transport and million-row result path; qualify all applicable DataGrip
2026.3 capabilities, performance and accessibility;
complete normal packaged restart/credentials and the hands-on walkthrough.
The Goal remains active until acceptance. No push, installation or publication
is authorized. The owner authorized reuse of the existing Snowflake connection;
only its non-secret configuration was inspected. Other private application state
and all external query/write qualification remain outside that authorization.

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
[Snowflake request-context slice](snowflake-completion-context.md) shipped in
preview.15; broader search-path/quoted-policy support and live qualification remain
open. External metadata changes still require refresh.
The [isolated query-session slice](isolated-query-sessions.md) merged on `main`
via PR #26 (`877904b`, all six PR checks passing with 79 frontend tests and 31
browser workflows) for PostgreSQL/MySQL/MariaDB using existing Rust sessions. It
remains unreleased; clean-source release provenance and grouped-FK JOIN
generation follow.
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

### R1 batch 1 result (merged in PR #25 `6445b0e`; unreleased)

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
Merged on `main` (`6445b0e`), unreleased.

### R1 batch 2 result (2026-10-30, local branch `feat/table-data-editing`)

- Optional raw `WHERE` and `ORDER BY` clauses on PostgreSQL and MySQL/MariaDB
  table tabs are synthesized and classified via `safety::prepare` (blocked in
  `ReadOnly` when non-read/unparseable; gated by one-use `ReviewDialog` approval
  in writable modes; raw `ORDER BY` disables keyset continuation cleanly).
- Permission-filtered inbound `referencing_foreign_keys` metadata powers `⇧⌘G`
  navigation to referencing rows in a new filtered tab across single, composite
  and multi-constraint relationships.
- Column hide (non-PK), reorder, resize and reset persist per
  `connectionId + schema + table` in bounded `SavedWorkspace.table_layouts`
  (purged on profile delete and reconciled on schema refresh).
- Multi-cell TSV paste validates every target cell and budget atomically before
  mutating `DirtyStore`, respecting reordered visible columns.
- Draft row inserts (`<default>` / `<required>`) and row deletes (`FOR UPDATE`
  full-row conflict checks) execute alongside updates in one atomic reviewed
  `BEGIN … COMMIT` transaction across PostgreSQL, MySQL 8.4 and MariaDB 11.4.
- `ValueDialog` provides 16-byte-per-row binary Hex/ASCII dumps, 1-based indexed
  PostgreSQL array element viewers (multi-dimensional/custom bounds, SQL `NULL`
  vs `"NULL"` vs `""`) and lossless token-level formatted JSON preserving
  64-bit integers and numeric object key order.

See matrix rows G03/G05/G07–G10 and `IMPLEMENTATION_STATUS.md` for evidence
(`99` frontend tests, `102` standard Rust tests, `41` disposable integration
tests, `37` Playwright workflows and clean-commit macOS native gates). Merged on
`main` via PR #27 (`94f63ba`); unreleased.

### R2 Data movement result (merged in PR #28 `c8d8aa1`; unreleased)

- Grid selection Copy-as (`CSV`, `TSV`, `JSON`, `NDJSON`, `SQL INSERT`,
  `Markdown`; max 5,000 rows / 2 MiB), capability-gated Rust `.partial` streaming
  export (`CSV`, `TSV`, `JSON`, `NDJSON`; max 1,000,000 rows / 512 MiB), upfront-validated
  capability-gated CSV/TSV import into reviewed 500-row `ChangePlan` batches, and
  key-aware bounded `LiveTable` / `RetainedResult` data comparison (including
  cross-engine PostgreSQL ↔ MySQL/MariaDB).
- Merged on `main` via PR #28 (`c8d8aa1`, all six PR checks green); unreleased.

### R3 Explorer and object search result (merged in PR #29 `2625933`; unreleased)

- Categorized WAI-ARIA keyboard object explorer (`Tables`, `Views`, `Materialized
  views`, `Functions`, `Procedures`, `Sequences`, `Triggers`, `Indexes`) with
  kind filter chips, bounded 256 KiB read-only DDL (`Copy DDL`, `Open in SQL
  editor`), bounded 200-edge 1-hop `Uses` / `Used by` dependency chips, and
  cross-connection `SearchEverywhere` (`⌘K` / `⇧⌘O`).
- Merged on `main` via PR #29 (`2625933`, all six PR checks green); unreleased.

### R4 Consoles, sessions and files result (merged in PR #30 `a0ba1a6`; unreleased)

- Per-session active schema/database switcher (`M05`: PostgreSQL `search_path`,
  MySQL/MariaDB `USE`, Snowflake stateless request-scoped schema verification)
  invalidating and refreshing live CodeMirror completion per session.
- Named & attachable multi-console session pool (`M06`: up to four total sessions
  per connection, multi-console attachment to named secondary sessions or `Shared
  session`, rename/disconnect controls, last-owner close lifecycle).
- Per-session `Auto-commit` vs `Manual commit` transaction mode and explicit
  `Commit` / `Rollback` controls (`X04`) driven by wire-level transaction state
  (`Idle` | `InTransaction` | `Failed`) on PostgreSQL and MySQL/MariaDB while
  keeping table-grid edits isolated on separate lanes.
- Capability-gated `.sql` file Open (`⌘O`), atomic in-place Save (`⌘S` on query
  tabs; `reviewEdits` preserved on table tabs) and Save As (`⇧⌘S`) via reusable
  `GrantKind::SqlFile` tokens (`E14`: ≤ 4 MiB UTF-8, dirty indicator, unsaved-close
  guard, zero raw paths over IPC, excluded from `SavedWorkspace.drafts`).
- Local SQLite saved-queries library (`P03`: max 256, ≤ 256 KiB each / 4 MiB
  total, privacy-safe detached default when `storeQueryText === false`,
  `SearchEverywhere` `Saved` scope + `LibraryModal`), persistent object &
  saved-query bookmarks (`P02`: `⌘D`, max 256, profile-delete cascade), and a
  50-item in-memory Recent locations ring buffer (`⇧⌘E`).
- Merged on `main` via PR #30 (`a0ba1a6`) on 31 October; unreleased.

## Assessment and next slices (2 November 2026)

R1–R4 are merged but absent from the last verified preview. Recorded R5 commit
`bb77b0f` remains readable; its former worktree/branch are now absent and former
dirty documentation is not recovered. Historical counts (146 frontend, 139 Rust,
56 fixture and 49 browser passes plus macOS gates) are not current qualification.
Integration is proceeding by reviewed feature group on the R6 branch.

Current candidate `51ad451` integrates six R5 slices plus formatting, named
outputs, console-bound inspectors, bounded alias/CTE rename, qualified-column
inspection/correction across ordered direct-table JOINs, and context-aware live
templates, alongside seven R6 slices. R5/R6 remain partial, local, unmerged,
unreleased and not installed. Templates expand observed database/schema
identifiers and literal selected SQL from the active console, with tab stops
and one undo, and never prepare or execute SQL. The focused template checks are
31 frontend tests, one Rust snippet-backup test, and six PostgreSQL/MySQL/MariaDB
browser flows. The prior JOIN candidate passed 379 frontend / 156 standard Rust
tests and its focused browser flows; those full checks were not rerun here.
Next: broader CTE/derived/unqualified inspection and semantic template placement,
then remaining R5/R6/R7/R8 and delivery gates. See
[current evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-context-aware-live-templates-2-november-2026).

### R5 integration slice 1: live FK JOIN completion (local `e0dad5f`)

Recorded `40c59a7` plus the SQL alias correction from `41957ea` now coexist with
the seven R6 slices. Permission-filtered outbound/inbound metadata preserves
composite pairs and temporary shadows. Scoped JOIN/initial ON suggestions handle
multiple/self relationships, aliases and quoting without executing SQL.
Snowflake FK metadata and advanced output inference remain outside this slice.

Validation passes 152 frontend / 136 standard Rust tests, two live metadata
fixture tests spanning PostgreSQL/MySQL/MariaDB, and fourteen completion/
Snowflake browser flows, including nine inserted-and-executed composite joins.
The browser check caught and fixed the PostgreSQL built-in-token alias defect.
Native/package gates and full integration suites were not rerun on this source.
R5 remains partial, local, unmerged and unreleased; see
[exact evidence](IMPLEMENTATION_STATUS.md#r5-integration-live-fk-join-completion-1-november-2026).

### R5 integration slice 2: Unicode diagnostics (local `ce7a7fa`)

Recorded `581c527` now coexists with the current R6 safety and cancellation
paths. Error DTOs and engine adapters retain UTF-16 ranges, statement identity
and available server diagnostics. PostgreSQL positions are character-based;
MySQL/MariaDB use best-effort syntax token/line ranges or whole-statement
fallback. Leading whitespace and unclassified-batch origins are covered by
live regression tests. Direct Snowflake wrapper validation shifts locations
once; Snowflake server diagnostics remain synthetic qualification.

All 54 SQL fixture tests and 22 diagnostic/FK/plan browser flows pass, including
mandatory analysis review and cancellation. Standard validation and three hidden
native runs pass for the exact code/tree. Inline markers/navigation and typed
parameters remain pending, including editor-selection and generated-wrapper
origin mapping. See [exact evidence](IMPLEMENTATION_STATUS.md#r5-integration-structured-unicode-diagnostics-1-november-2026).

### R5 integration slice 3: typed parameters (local `39359e7`)

Recorded `9cfefa2` now uses the common Rust plan store for query execution,
Explain and Analyze. Named/positional scanning excludes SQL strings/comments;
all values/types/NULL are validated before review. Bindings are immutable and
part of approval identity. Parameter drafts stay transient per tab, and history
keeps templates only. Snapshot comparison rejects different canonical bindings.
PostgreSQL uses typed SQL PREPARE/EXECUTE with encoded constants; MySQL/MariaDB
use protocol parameters; Snowflake SQL API bindings have synthetic coverage only.

Rewritten error locations map back to original placeholders; generated Explain
prefixes map to their input query. Inline markers and selection-to-editor origin
mapping remain pending. All 58 SQL fixture tests and 23 parameter/plan/timing/
diagnostic browser flows pass, plus 156 frontend and 154 standard Rust tests.
Clean-source native evidence and precise scope are in
[implementation evidence](IMPLEMENTATION_STATUS.md#r5-integration-typed-query-parameters-1-november-2026).

### R5 integration slice 4: inline diagnostics/navigation (local `65530c0`)

Recorded `655d8b7` now coexists with parameters and R6 preparation/approval paths.
Wavy marks, error gutters, SQLSTATE/location details and Jump to error use exact
submitted source identity. Current-statement and selection offsets pass through
parameter/review dialogs. Original placeholder and generated Explain input
coordinates are shifted once; CRLF/bare CR map to CodeMirror newline units.
Changed documents and malformed/out-of-slice locations cannot receive marks.
Statement navigation uses the active dialect parser and has no lexical guess
while parsing remains incomplete. Redis command behavior is preserved.

Twelve focused unit cases and six diagnostic browser flows pass, including
parameterized Run/Explain/Analyze and a late error after an edit. Twenty existing
parameter/plan/timing browser flows and standard validation also pass. See
[exact evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-inline-diagnostics-and-navigation-1-november-2026).

### R5 integration slice 5: definitions/usages/documentation (local `8ef9dfb`)

Recorded `b5a744d` now coexists with R6 and exact execution origins. F12 and
Cmd/Ctrl-click navigate CTE/table-alias declarations and primary-session object
structure; Shift+F12 lists scoped editor references; F1 shows current-session
metadata. Nested shadows, correlated bindings, unknown/ambiguous columns and
routine overloads are handled conservatively. Projection inference and complete
semantic SQL analysis remain open. Database dependents are object-level results.

Edits, refreshes, session/context changes and close reject obsolete replies.
Secondary consoles retain quick docs/local declaration jumps; object Structure
navigation is withheld because Structure currently uses the primary session.
Reference results have a visible 500-entry / 10,000-name-token bound. Standard
validation and 46 browser flows pass, including five new symbol flows; clean-source
macOS baseline passes three fresh processes / 30 cleanup cycles. See
[exact evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-sql-definitions-usages-and-documentation-1-november-2026).

### R5 integration slice 6: formatting/settings/snippets (local `db5cd1b`)

Recorded `3ca2ad2` now coexists with R6, with serialized format saves addressing
`41957ea` and relevant `bb77b0f` native guards. Selection formatting preserves
unselected text/newlines, undo and bind syntax; obsolete worker replies are
rejected after any document edit or session change. Global style preferences
persist with visible errors and rollback to the last successful save. Per-dialect
style overrides remain open.

Built-in and custom SQL templates use completion and keyboard tab stops without
automatic execution. Custom engine scope overrides global/built-in triggers.
SQLite bounds/validates templates and includes them in metadata recovery.
Context-aware variables and complete semantic placement remain open. Full
standard validation, all 86 local browser flows and clean-source macOS hidden
baseline (three processes / 30 cleanup cycles) pass. See
[evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-formatting-and-snippets-2-november-2026).

### R5 style follow-up (local `d39b89b`)

Preferences supports global defaults and independent PostgreSQL, MySQL/MariaDB
and Snowflake styles. First customization snapshots the global case/indent/
spacing; reset resumes inheritance. The console dialect automatically selects
the style. Rust migrates legacy settings and validates all styles on save and
metadata recovery. Three real database browser flows verify reload, automatic
selection and reset; standard checks and the clean-source native persistence/
worker baseline pass. Broader style controls and context-aware template variables
remain open. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-per-engine-formatting-styles-2-november-2026).

### R5 named-output follow-up (local `c32f0c2`)

Directly named SELECT columns now resolve through CTEs and derived tables using
current-session metadata. Full explicit CTE column lists are authoritative.
Definitions jump to local declarations, usages retain scoped identities and docs
label source metadata. Nested/quoted names, ambiguity, unavailable metadata and
unsupported shapes have regressions; wildcards, expressions and set-operation
outputs remain withheld. Metadata follows bodies within the existing fetch cap.
Inference has explicit recursion/body/output/token limits and action-local memoization.
Standard checks, five real navigation browser flows and the native synthetic
baseline pass. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-named-query-outputs-2-november-2026).

### R5 console-bound Structure follow-up (local `fe518fb`)

Primary and isolated consoles now open separate read-only Structure/DDL tabs
retaining the originating session and observed catalog revision. Metadata never
falls back to the primary session; pre/post revision checks and source lifecycle
guards reject stale replies. Column highlighting, explicit detail-denial fallback
and supported non-tabular inspectors are retained without browsing table data or
executing SQL. Existing inspector links and DDL-to-editor actions are withheld in
this path until they preserve the origin. Standard checks, six real navigation
flows and the native synthetic baseline pass. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-console-bound-structure-2-november-2026).

### R5 context-preserving inspector actions (local `34c001a`)

Foreign-key, dependency, owner-table, index and trigger links now retain the
frozen originating console/session/context. Explicit DDL append preserves source
SQL, file dirty tracking and session ownership; it never executes. Fresh catalog
checks reject stale/denied actions, source lifecycle changes and obsolete replies;
queued/running consoles cannot receive DDL. Standard checks, six navigation flows,
one real inspector fixture and the native synthetic baseline pass. See
[evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-context-preserving-inspector-actions-2-november-2026).

### R5 bounded alias rename (local `e052626`)

Explicit preview / Apply changes only the selected SELECT alias declaration and
proven qualified references, with dialect quoting, nested shadowing, correlated
references, CTE/derived boundaries, capture checks and one-step undo. Stale
text/session/context and unsupported shapes are withheld; no SQL executes.
Standard checks, nine focused browser flows and the native synthetic baseline
pass. See [bounds and evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-bounded-alias-rename-2-november-2026).

### R5 ordered JOIN alias visibility (local `1edc2d7`)

Each ON clause and its correlated subqueries use the JOIN's actual operands;
later aliases and earlier comma groups cannot hide valid outer references.
Pending JOIN conditions preserve nested association; completed CROSS/NATURAL/
USING joins retain their operands. MySQL CROSS conditions stay supported.
Capture simulation uses the same environments. The full preview/Apply, quoted
identifier, stale-document/context and one-undo guarantees remain in place.
Standard checks and twelve focused browser flows pass; three new flows execute
only explicit test reads before/after rename across all three disposable engines.
No new native, full-suite, distribution or external qualification is claimed.
See [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-ordered-join-alias-visibility-2-november-2026).

### R5 bounded CTE rename (local `072e4ff`)

Shift-F6 / Rename alias / CTE previews the declaration, proven relation uses and
unaliased qualifiers; explicit relation aliases retain their independent identity.
Ordered WITH visibility, PostgreSQL nested shadowing, physical-table separation,
quoted delimiters, output lists and proposed-name capture have regressions.
Recursive and unsupported shapes stay withheld. The shared MySQL/MariaDB dialect
also withholds nested WITH beneath an outer CTE scope: direct probes on the
11.4 fixture fail to resolve outer CTEs in nested definitions where MySQL 8.4
and PostgreSQL 18 succeed. No version-specific semantic rule is guessed.
Standard checks, fifteen focused browser flows and the clean native synthetic
baseline pass. Browser checks compare decoded result values before/after explicit
rename; preview/Apply never prepares or executes SQL. See
[exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-bounded-cte-rename-2-november-2026).

### R5 bounded qualified-column inspections (local `cfb6139`)

Inspect columns checks qualified references in one direct-table SELECT on
PostgreSQL/MySQL/MariaDB. Fresh inspection metadata is stricter than completion:
partial column grants, missing visibility, unsupported DDL and stale contexts
withhold claims. The user selects a real column, previews its exact quoted edit,
then explicitly applies one occurrence with one-step undo. Editor SQL is never
prepared/executed. Joins, CTEs, derived relations and broader inference remain open.
See [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-qualified-column-inspections-2-november-2026).

### R5 ordered direct-table JOIN inspections (local `46912f5`)

Inspect columns now checks qualified references over flat direct-table JOINs
and comma groups on PostgreSQL/MySQL/MariaDB. ON clauses see only their actual
operands, including pending JOIN association; later relations and earlier comma
groups cannot leak into their scope. Self-joins share one fresh metadata read
while retaining separate aliases. The correction chooser follows each issue's
owning table. A failed or stale table read prevents the whole inspection result.
Limits are 32 relations, 2,048 columns per table, 8,192 columns / 4 MiB aggregate,
alongside existing statement/token/expression limits. Nested/derived/CTE inference,
unqualified-column diagnostics and broader semantic fixes remain open. See
[exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-ordered-join-column-inspections-2-november-2026).

### R5 context-aware live templates (local `51ad451`)

1. Extend existing built-in/custom templates with a small documented variable
   vocabulary for current database/schema and selected text. Resolve values from
   the active console's observed context; quote SQL identifiers by engine and
   distinguish literal selected text from identifier values. Do not guess missing
   context or read another console's catalog.
2. Preserve numbered tab stops, final cursor stop, engine/global precedence,
   custom-template CRUD/reload and bounded backup/restore. Unsupported variables
   need explicit feedback; expansion must never prepare or execute SQL. Resolve
   asynchronous context before insertion and reject changed documents/sessions.
3. Reproduce missing/stale context and quoted-name cases, exercise primary and
   isolated console expansion on PostgreSQL/MySQL/MariaDB, compare an explicit
   harmless read after expansion, and prove zero expansion execution requests.
   Implemented locally on `51ad451`. See
   [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-context-aware-live-templates-2-november-2026).
   Broader CTE/derived inspection and semantic placement stay partial. Then
   continue remaining R5/R6 integration and R7/R8; merge/release/install and
   external/native/platform qualification remain separate gates.

### R6 slice 1: estimated query plans (local `feat/estimated-query-plans`)

Commit `fad0341`, based on fetched `origin/main` `a0ba1a6`, adds a single-query
estimated-plan action for PostgreSQL and MySQL/MariaDB, routed through existing
Rust authorization and cancellation. Bounded tree/raw views share the retained
JSON result; optimizer costs, estimated rows and actual per-loop metrics retain
distinct labels. See [query-plans.md](query-plans.md) for the contract and bounds.

Validation passes 126 frontend tests and 130 standard Rust tests, plus seven
relevant browser workflows (three live-engine plan flows and four session/file
regressions), macOS bundling and three hidden native runs / 30 cleanup cycles.
This branch does not contain R5. It is unmerged and unreleased; R6 is partial.

### R6 slice 2: reviewed PostgreSQL actual plans (local `fa1ff17`)

PostgreSQL single-query `EXPLAIN (ANALYZE, FORMAT JSON)` is implemented with
mandatory Rust review even on Silent profiles. Review shows exact SQL and an
execution warning; ReadOnly denial and production passphrases remain enforced.
Recognized direct SQL and parenthesized ANALYZE options hit the same gate.
Owned-session execution, cancellation and one-use approval are unchanged.

Standard validation passes 126 frontend and 132 Rust tests. Two PostgreSQL
fixtures prove denied analysis never executes, temporary state remains attached,
approvals expire and are single use, disconnect invalidates plans, and errors
and cancellation recover safely. Five plan browser flows and four console/file
regressions pass. Clean-source macOS bundle and three hidden native runs /
30 cleanup cycles pass. Full ignored integration/full browser suites were not
rerun. R5 integration, merge, application publication and installation remain open.

### R6 slice 3: bounded snapshots and comparison (local `ba984f2`)

Explicit captures retain raw plans and immutable executed query/context identity;
comparison never reruns SQL. A shared ten-capture / 2 MiB UTF-8 payload budget
includes SQL and context, with FIFO eviction and tab/session/connection cleanup.
Only matching engine/version, session, observed context and exact underlying
query bytes compare. PostgreSQL estimated and actual plans may be viewed side
by side; incompatible snapshots show a disabled reason. Independent trees and
raw JSON preserve labels and exact text without guessed node pairing or deltas.
Snapshots remain memory-only and are excluded from workspace persistence.

Standard validation passes 134 frontend and 132 Rust tests. Six plan browser
flows and four console/file regressions pass against disposable PostgreSQL 18,
MySQL 8.4 and MariaDB 11.4. The snapshot unit/lifecycle suite covers eviction,
invalid input, mismatch, exact numeric text, cleanup and persistence exclusion.
See [verification evidence](IMPLEMENTATION_STATUS.md#r6-bounded-plan-snapshots-and-comparison-1-november-2026).
This branch remains separate from R5, unmerged and unreleased.

### R6 slice 4: execution timing and observed plan context (local `cb1ca1a`)

Query runs separately measure queue wait, backend active/total, first nonempty
row batch and client request-to-finish time. Missing values remain unavailable;
zero remains visible. Driver batch delivery is not first-wire-row latency or
render time. Server planning/execution values stay separate in PostgreSQL plans.
Plan capture now uses context observed by Rust after acquiring the editor lane;
unavailable observations withhold capture rather than reusing frontend state.
Database state and unobserved settings remain outside this identity guarantee.

Standard validation passes 136 frontend tests and 132 standard Rust tests.
All 26 PostgreSQL fixture tests and twelve relevant browser flows pass; these
include queued context changes, empty/error/multi-result timing, active/queued
cancellation and unavailable-context capture denial. Clean-source macOS gates
are recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-execution-timing-and-observed-plan-context-1-november-2026).
This branch remains separate from R5, unmerged and unreleased.

### R6 slice 5: reviewed MariaDB actual plans (local `c660bc4`)

MariaDB's Analyze action uses `ANALYZE FORMAT=JSON` only when a prepare-only
server capability check succeeds. It never executes analysis on connection
or changes optimizer/output settings. Explicit review applies even on Silent,
including directly entered SQL that falls outside the AST grammar. ReadOnly
and production policy remain authoritative; one-use approval, owned temporary
state, error recovery, cancellation retirement and observed-context comparison
are verified. MariaDB inclusive node time, average rows per loop, filtering and
optimization remain distinct from PostgreSQL's metrics.

Standard validation passes 138 frontend and 133 standard Rust tests. All 26
PostgreSQL and 24 MySQL/MariaDB base fixtures pass, plus fifteen relevant browser
workflows. The separate case-folded fixture was excluded. Exact macOS gates are
recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-reviewed-mariadb-actual-plans-1-november-2026).
MariaDB qualification is bounded to 11.4.13. R6 stays partial, local and separate
from the preserved R5 candidate.

### R6 slice 6: reviewed MySQL TREE actual plans (local `c54ecab`)

MySQL Analyze now explicitly uses `EXPLAIN ANALYZE FORMAT=TREE` after a
non-executing prepare-only capability probe. Session JSON/output settings remain
unchanged. A separate bounded TREE adapter preserves engineering tokens and
unexecuted iterators; costs, rows, loops and inclusive per-loop times retain
their scopes. JSON estimates can compare with TREE actuals using the existing
exact executed query and observed-context identity, without replay.

Reproduced review bypass for `DESC ANALYZE ... FOR SCHEMA` is fixed before AST
fallback, including DESC/DESCRIBE aliases. Reproduced partial-result success
after MySQL KILL QUERY now honors cancellation and retires the session. This
does not roll back side effects or change ordinary write acknowledgements.

Validation passes 142 frontend and 133 standard Rust tests, all 26 PostgreSQL
and 25 MySQL/MariaDB base fixtures, and eighteen relevant browser workflows.
The separate case-folded fixture remains excluded. Capability/settings, one-use
approval, temporary state, ReadOnly, errors, cancellation, comparison and cleanup
are qualified on MySQL 8.4.13 / MariaDB 11.4.13 only. Clean-source macOS gates are
recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-reviewed-mysql-tree-actual-plans-1-november-2026).
R6 stays partial, local and separate from the preserved R5 candidate.

### R6 slice 7: Snowflake estimated logical plans (local `48dfb21`)

Explicit Explain wraps one complete Snowflake SELECT/WITH query with
`EXPLAIN USING JSON`; read-only classification still rejects unsafe/unknown
input. A step-local DAG adapter expands shared operators once and retains
all other parent edges as explicit references. Pruning estimates stay separate
from actual timing/rows; unsafe numeric estimates defer to exact raw JSON.
Malformed, cyclic, orphaned and excessive graphs retain raw/grid access.

Standard validation passes 148 frontend / 136 Rust tests, and four synthetic
Snowflake browser flows pass. Loopback Rust transport verifies the exact SQL
wrapper and configured request context without a live account. Snowflake
snapshots are unavailable until warehouse and session context are independently
observed and qualified. No Analyze or warehouse setting change is introduced.
Clean-source macOS evidence is in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-snowflake-estimated-logical-plans-1-november-2026).
R6 remains partial, local, unmerged and unreleased.

### R6 next slices

1. Six R5 slices and seven R6 slices now include per-engine styles and named
   CTE/derived-table outputs, console Structure, inspector continuation and
   bounded alias/CTE rename and ordered JOIN column inspection/correction (`46912f5`).
   Standard checks and 21 focused browser flows pass. The clean-source
   native synthetic baseline passes three processes / 30 cycles on this revision.
   Prior combined-source qualification passed all 66 ignored local SQL/Redis/SSH
   fixtures and a normal ad-hoc macOS bundle on `b80b6c7`; the full 86-flow browser
   suite passed on `db5cd1b`.
   Next: the remaining R5/R6/R7/R8 scope.
   A non-recursive CTE UNION is inspected from the anchor projection. A recursive arm can read its CTE.
   A derived table UNION is inspected from the anchor projection.
   INTERSECT and EXCEPT use that same anchor projection inside a CTE or derived table.
   A select list that is only * or alias.* for one relation expands to that relation's columns.
   An unaliased schema.table.* expands to that same relation.
   A star beside simple columns keeps those names as well. A repeated output name still withholds.
   A lone star on every arm of a set operation expands when each star is the same relation.
   An unaliased schema.table.* may be that star on every arm.
   A later arm of a recursive CTE may be that same lone star, including a star of the CTE.
   Every arm of a set operation may add the same number of simple columns beside that star.
   A star of another relation is accepted when metadata gives every arm the same width.
   A full explicit column list renames a star when the list matches that star's width.
   A top-level UNION, INTERSECT, or EXCEPT of flat SELECT arms uses that same width check.
   One pair of parentheses may wrap a flat arm of a CTE or derived-table set operation.
   `{{lower(selection)}}` and `{{upper(selection)}}` change the selected text's case.
   A recursive CTE of more than two flat SELECT arms is inspected from the anchor projection.
   A full explicit CTE column list renames that CTE's projected outputs. A partial list still withholds.
   A parenthesized query in a function argument uses the same template placement.
   A parenthesized statement after EXPLAIN uses the same template placement.
   A custom snippet can declare its completion site. A built-in trigger keeps its place.
   Completion classifies the statement inside an EXPLAIN wrapper.
   A recursive CTE of two flat SELECT arms is inspected from the anchor projection.
   Completion places statement, JOIN and LIMIT templates inside a parenthesized query after FROM, JOIN, AS, IN, EXISTS, SELECT, a comparison, or a comma.
   A derived table can read an earlier CTE in the same WITH list.
   Several non-recursive CTEs use the same projected-name rule. A CTE can see earlier CTEs in that WITH list.
   Parenthesized select-list expressions use the same one-owner column rule.
   Function arguments use the same one-owner column rule.
   Bare names in WHERE, GROUP BY, HAVING, ORDER BY and ON use the same
   one-owner rule as the select list. Completion places statement templates at a statement
   start and JOIN or LIMIT templates after that clause. A top-level select-list name is
   inspected when one relation owns it. Several owners withhold the claim.
   One non-recursive CTE or derived query can be inspected from its projected
   names (`15089a1`). Multiple CTEs, recursive queries and stars still withhold.
   Context-aware templates are local on `51ad451`. Preserve conservative ambiguity
   handling, every
   preparation/approval path and exact editor origins. Keep the earlier startup
   flake open until its cause is known.
   Application merge, release and installation remain separate authorized gates.
2. Design observed Snowflake warehouse/session identity before snapshot support.
   Keep capture explicitly unavailable meanwhile. Live read-only account
   qualification requires explicit external scope; mock evidence is local only.
3. Broaden supported MySQL/MariaDB versions and TREE shapes only with captured
   engine evidence. Unsupported capability/format combinations retain clear
   unavailable/raw states; do not silently change user settings.

The MariaDB adapter follows its
[official analysis contract](https://mariadb.com/docs/server/reference/sql-statements/administrative-sql-statements/analyze-and-explain-statements/analyze-format-json).
The separate MySQL TREE adapter follows the
[8.4 EXPLAIN manual](https://dev.mysql.com/doc/refman/8.4/en/explain.html).
The earlier JSON-v2 probe remains research evidence; no JSON-v2 adapter is enabled.

R7 schema/design and R8 remaining productivity follow these R6 slices. Release,
platform, security, accessibility, recovery and performance qualification run
throughout; application publication and installation remain separate actions.

### R7 slice 1: read-only ER diagrams (branch `feat/er-diagram`)

The owner asked on 3 November to continue until Joinery can replace DataGrip.
Read-only slices go first: D01 diagrams, then A05/A04 monitoring, A03 statistics,
D02 snapshots/diff, D03 DDL generation, D05/N08 reviewed editors, D04 migration
review, P05/P06, RX and the v1.0 gates. `schema_diagram` reads one schema's
relations, readable columns and outbound foreign keys in one bounded metadata call
on PostgreSQL and MySQL/MariaDB. It never prepares or executes editor SQL. See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-1-read-only-er-diagrams-3-november-2026).

### R8 slice 1: lock waits and session monitoring (branch `feat/lock-monitoring`)

`lock_waits` reads current waits and blockers on PostgreSQL, MySQL and MariaDB.
The Processes view derives root blockers and deadlock cycles locally and adds
filtering and optional refresh. Merged in PR #62 (`80de7aa`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-1-lock-waits-and-session-monitoring-3-november-2026).

### R8 slice 2: relation storage and index usage statistics (branch `feat/storage-statistics`)

`object_details` populates `TableStorageStats` (`estimated_rows`, `table_bytes`,
`index_bytes`, `free_bytes`, `total_bytes`, `live_tuples`, `dead_tuples`,
`sequential_scans`, `index_scans`, `last_analyze`, `last_vacuum`, `stats_stale`,
`restricted_reason`) and per-index `size_bytes`, `scans`, `tuples_read`,
`tuples_fetched` and `cardinality` on PostgreSQL and MySQL/MariaDB. It never
issues automatic `COUNT(*)` or `ANALYZE`, withholds PostgreSQL relation storage
and index counters when the session lacks `SELECT` privilege on the relation,
and degrades cleanly on MySQL/MariaDB when `mysql.innodb_index_stats` or
`performance_schema` is unreadable or disabled. Merged in PR #63 (`130e192`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-2-relation-storage-and-index-usage-statistics-3-november-2026).

### R7 slice 2: schema snapshots and deterministic schema diff (branch `feat/schema-diff`)

`schema_snapshot(sessionId, schema)` captures a bounded, canonical structural
snapshot (`SchemaSnapshot`) of a PostgreSQL schema or MySQL/MariaDB database
(tables, views, materialized views, partitioned tables, columns, constraints,
indexes, triggers, overloaded routines, sequences, and disclosed unsupported
objects such as custom PostgreSQL enum/domain/range/composite types, foreign
tables, non-view rewrite rules, and MySQL scheduled events). `compare_schemas`
computes a deterministic `SchemaDiffReport` independent of catalog row order,
with engine-quoted object and member names. `SchemaDiffView` compares any two
live schemas or up to 16 in-memory captured snapshots and never prepares or
executes DDL. Merged in PR #64 (`046d05c`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-2-schema-snapshots-and-deterministic-schema-diff-3-november-2026).

### R7 slice 3: Rust DDL generator and migration preview (branch `feat/ddl-generator`)

`generate_schema_ddl(snapshot, targetSchema)` and
`generate_migration_ddl(left, right, targetSchema)` generate dependency-ordered
PostgreSQL and MySQL/MariaDB full-schema and migration DDL scripts in Rust
(`src-tauri/src/ddl.rs`), retargeting self-schema references and flagging
destructive `DROP` statements and warnings without auto-executing SQL. Round-trip
disposable fixture tests on PostgreSQL 18, MySQL 8.4 and MariaDB 11.4 verify
zero structural diffs (`compare_schemas == 0`) after executing both full-schema
and migration DDL plans. Merged in PR #65 (`b67e65b`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-3-rust-ddl-generator-and-migration-preview-3-november-2026).

### R7 slice 4: index and constraint editor (branch `feat/index-constraint-editor`)

`preview_index_constraint_ddl(engine, action)` and
`prepare_index_constraint_ddl(sessionId, action)` build and authorize
engine-aware `CREATE INDEX`, `DROP INDEX`, `ADD CONSTRAINT` (`PRIMARY KEY`,
`UNIQUE`, `CHECK`, `FOREIGN KEY` with `ON DELETE`/`ON UPDATE` and PostgreSQL
`NOT VALID`) and `DROP CONSTRAINT` statements on PostgreSQL and MySQL/MariaDB.
`StructureView` and `ObjectInspectorView` wire `IndexConstraintDialog` with live
Rust DDL previews, lock/implicit-commit warnings, `Constraint-backed` index
guardrails, and mandatory `ReviewDialog` approval. Merged in PR #66 (`792aa99`).
See [evidence](IMPLEMENTATION_STATUS.md#r7-slice-4-index-and-constraint-editor-3-november-2026).

### R7 slice 5: table and column DDL editor (branch `feat/table-object-editor`)

`preview_table_object_ddl(engine, action)` and
`prepare_table_object_ddl(sessionId, action)` build and authorize engine-aware
`CREATE TABLE` (ordered columns, `NOT NULL`, defaults, PostgreSQL `GENERATED BY
DEFAULT AS IDENTITY` vs MySQL/MariaDB `AUTO_INCREMENT`, composite `PRIMARY KEY`
and table comments), `ADD COLUMN`, `ALTER COLUMN` (PostgreSQL `ALTER COLUMN …
DROP DEFAULT` before `TYPE … USING …`, `SET`/`DROP NOT NULL`, `SET`/`DROP
DEFAULT` and `RENAME COLUMN`; MySQL/MariaDB `MODIFY COLUMN` / `CHANGE COLUMN`),
`DROP COLUMN` (with optional PostgreSQL `CASCADE`), and `DROP TABLE` / `DROP
VIEW` / `DROP MATERIALIZED VIEW` (with optional PostgreSQL `CASCADE`).
`Sidebar.tsx` (`Create table in <schema>`) and `StructureView` (`Add column…`,
`Drop table…` / `Drop view…`, per-column `Edit…` and `Drop…`) open
`TableObjectDialog` with live Rust DDL previews, lock/implicit-commit/destructive
warnings, and mandatory `ReviewDialog` approval. Merged in PR #67 (`c855c04`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-5-table-and-column-ddl-editor-3-november-2026).

### R7 slice 6: migration review and schema synchronization workflow (branch `feat/migration-sync`)

`prepare_migration_ddl(sessionId, expectedSnapshot, statements)` verifies that
the live target schema has not drifted from `expectedSnapshot` (returning
`ErrorCode::Conflict` on any structural difference or truncation/privilege
change), classifies selected `DdlStatement`s as `StatementKind::Ddl` via
`safety::plan_for_statements` (`SafeMode` + one-use approval + production
passphrase), and attaches destructive, implicit-commit and lock warnings.
`SchemaDiffView` adds per-statement checkboxes (`Select all`, `Select
non-destructive`), **Review & apply migration…**, and `ReviewDialog` integration,
refreshing the catalog and schema comparison upon completion. Merged in PR #68
(`83b1260`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-6-migration-review-and-schema-synchronization-workflow-3-november-2026).

### R8 slice 3: contextual command palette, custom shortcuts and tab management (branch `feat/shortcuts-and-layouts`)

`shortcuts.ts` defines 16 canonical keyboard shortcut actions (`openPalette`,
`searchObjects`, `recentLocations`, `newQuery`, `openSqlFile`, `saveSqlFile`,
`saveSqlFileAs`, `toggleBookmark`, `preferences`, `closeActiveTab`,
`closeOtherTabs`, `closeCleanTabs`, `reopenClosedTab`, `togglePinTab`,
`toggleSidebar`, `toggleInspector`) with conflict and reserved-chord
(`Mod+C`/`V`/`X`/`Z`/`A`) validation, persisted in `SavedWorkspace.shortcuts`
(validated in `persistence/workspace.rs`). `SearchEverywhere` renders contextual
`contextBadge` pills and `disabledReason` subtitles for active-tab, tab-strip
and layout actions. `studio.ts` adds tab pinning (`QueryDraft.pinned`),
`closeOtherTabs`, `closeCleanTabs` (preserving pinned, dirty, running and
last-secondary-session-owner tabs), a bounded 10-entry `closedTabs` reopen stack
(`reopenClosedTab`, purged on connection removal), and persisted
`sidebarCollapsed` / `inspectorVisible` toggles. Merged in PR #69 (`8be971a`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-3-contextual-command-palette-custom-shortcuts-and-tab-management-3-november-2026).

### R8 slice 4: closed typed IPC command map and granular engine capabilities (branch `feat/typed-commands-capabilities`)

`src/ipc/command-map.ts` defines the closed `JoineryCommandMap` pairing every
backend command with its exact argument and result DTOs from
`src/ipc/generated.ts`, enforced by `StudioClient.call` in `src/ipc/client.ts`.
`EngineCapabilities` (`src-tauri/src/domain.rs`) adds `schemaDiagram`,
`schemaDiff`, `ddlEditing`, `lockInspection`, `explainAnalyze` and
`oauthBrowser`, populated by each engine driver and enforced in `Studio`
(`ErrorCode::Unsupported`) as well as in `Sidebar`, `ProcessesView`,
`StructureView`, `ObjectInspectorView` and `Workbench`. See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-4-closed-typed-ipc-command-map-and-granular-engine-capabilities-3-november-2026).

### R8 slice 5: engine administration and table maintenance tools (branch `feat/typed-commands-capabilities`)

`build_table_maintenance_sql` (`src-tauri/src/ddl.rs`),
`preview_table_maintenance`, and `prepare_table_maintenance` generate reviewed
PostgreSQL (`ANALYZE`, `VACUUM`, `VACUUM FULL`, `REINDEX TABLE`) and
MySQL/MariaDB (`ANALYZE TABLE`, `OPTIMIZE TABLE`, `CHECK TABLE`) maintenance
statements with lock, implicit-commit, and non-transactional warnings.
`Postgres::execute` / `execute_bound` reject non-transactional maintenance
statements (`VACUUM`, `CLUSTER`, `CONCURRENTLY`) with `ErrorCode::Conflict` when
a manual transaction is open. `StorageStatsSection` (`ObjectInspectorView.tsx`,
`UtilityViews.tsx`) adds a **Maintenance…** action opening
`TableMaintenanceDialog.tsx` and routing execution through `ReviewDialog`. See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-5-engine-administration-and-table-maintenance-tools-3-november-2026).

### RX slice 1: single-table query result editing with Rust-proven provenance (branch `feat/typed-commands-capabilities`)

`safety::analyze_single_table_select` and `safety::verify_result_provenance`
(`src-tauri/src/safety.rs`) inspect `SELECT` statements on PostgreSQL and
MySQL/MariaDB against the live `TableSchema` and driver-reported result columns,
proving single-base-table and primary-key provenance or recording an explicit
`ResultEditability::ReadOnly { reason }`. Proven results store a projected
`TableSchema` in `ResultStore` and expose `ResultEditability::Editable { table,
primary_key }` on `ResultInfo`, enabling inline cell edits, row inserts, row
deletes, TSV paste, and reviewed `prepare_table_changes` commits (`SELECT … FOR
UPDATE` full-row conflict checks + in-place `ResultStore::apply_committed_changes`
refresh) directly inside SQL query result grids. See
[evidence](IMPLEMENTATION_STATUS.md#rx-slice-1-single-table-query-result-editing-with-rust-proven-provenance-3-november-2026).

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
Full derived/CTE output inference and wider engine/context qualification remain
open. Direct-base-table PostgreSQL/MySQL/MariaDB FK JOIN assistance is locally
verified in the first R5 integration slice above.
