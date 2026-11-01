# Query plans

R6 local `feat/estimated-query-plans` adds **Explain** for PostgreSQL and
MySQL/MariaDB (`fad0341`), then **Analyze** for PostgreSQL (`fa1ff17`). Both
use the current statement or explicit selection without changing editor text.
Explicit bounded plan snapshot comparison follows in `ba984f2`; execution timing
and observed context in `cb1ca1a`; reviewed MariaDB actual plans in `c660bc4`
and MySQL TREE analysis in `c54ecab`. Snowflake estimated logical plans follow
in `48dfb21`. Redis has no plan action. R6 remains partial and unreleased.

## Preparation and authorization

Rust parses the entire selected input before wrapping it. Exactly one
`Statement::Query` (SELECT/WITH) is accepted. Batches, mutations, existing
EXPLAIN, malformed/empty SQL, MySQL executable comments and oversized input
are rejected. The generated statement is capped at 1 MiB. Original query bytes
are retained after `EXPLAIN (FORMAT JSON)` (PostgreSQL),
`EXPLAIN FORMAT=JSON` (MySQL/MariaDB), `EXPLAIN USING JSON` (Snowflake),
`EXPLAIN (ANALYZE, FORMAT JSON)` (PostgreSQL actual measurements),
`ANALYZE FORMAT=JSON` (capability-checked MariaDB actual measurements), or
`EXPLAIN ANALYZE FORMAT=TREE` (capability-checked MySQL actual measurements).

`Studio::prepare_explain` resolves the actual session engine and delegates the
generated SQL to existing `prepare_execution`. The immutable plan, Safe Mode,
production passphrase, one-use authorization, owned session and cancellation
path remain authoritative. A query containing a modifying CTE, side-effect
function or locking clause still receives normal Rust classification; tests
verify ReadOnly denial and production review. Estimated planning is not a
guarantee of zero database work or side effects: database grants remain the
access boundary.

**Analyze executes the query**, including its ordinary side effects, and can
add profiling overhead. Rust requires explicit review even on Silent local
profiles. This also covers recognized manually entered `EXPLAIN ANALYZE` and
parenthesized `EXPLAIN (ANALYZE, ...)` options; the check is applied before
the plan is stored and fingerprinted. PostgreSQL `ANALYZE FALSE`, `OFF` or `0`
does not execute analysis. MySQL/MariaDB token checks also require review for
statement-leading `ANALYZE` and `EXPLAIN` / `DESC` / `DESCRIBE` containing
unquoted `ANALYZE`,
including parser fallback and later batch statements. Comments and quoted
text do not trigger this check; EXPLAIN cases are conservatively reviewed.
ReadOnly rejects executing analysis; production
profiles retain passphrase requirements. Review displays the exact generated
SQL and execution warning. Cancelling review sends no execution request.

Approval is single use and bound to the exact plan/session. A replacement
preview or disconnected session invalidates it. Queries run on the owned
console session, preserving temporary objects and transaction state. A query
error uses normal recovery; cancellation retires the session rather than
replaying SQL. MySQL can return a partial TREE result after KILL QUERY; the
driver checks the cancellation request for recognized analysis even when the
server returns success. It reports Cancelled and retires the session. This does
not undo side effects or imply rollback. Ordinary write acknowledgements retain
their existing outcome handling. No analysis runs automatically on editor changes.

The command is enrolled in the desktop Tauri ACL and test harness. The hidden
native check verifies that both estimated and actual preparation reach session
validation through real WebView IPC rather than failing ACL resolution.

## MariaDB and MySQL capability contracts

MariaDB sessions advertise `mariaDbJson` after a server-family check and
successful protocol preparation of `ANALYZE FORMAT=JSON SELECT 1`. Other MySQL
sessions advertise `mysqlTree` after preparing the fixed
`EXPLAIN ANALYZE FORMAT=TREE SELECT 1`. Both statements are closed without
execution. Unsupported preparation leaves Analyze unavailable. This detects
syntax/support, not permissions for an arbitrary query; normal authorization
and database grants still apply. No analysis runs during connection, and no
optimizer or output setting is changed to enable the action.

Disposable qualification is bounded to MariaDB **11.4.13** and MySQL **8.4.13**.
MySQL 8.4.13's default JSON format version 1 rejects JSON analysis; version 2
returns a separate JSON-v2 contract. The implemented action explicitly requests TREE
instead. Fixtures verify it works with the session's `explain_format=JSON` and
preserves both that setting and `explain_json_format_version=1`. Wider versions,
forks, privileges and formats require independent qualification.

## Raw, tree and grid views

A retained one-cell `QUERY PLAN` / `EXPLAIN` / `ANALYZE` result exposes **View plan**.
Snowflake `content` exposes it only for a recognized single-query generated
Explain execution; ordinary CONTENT query results do not become plans.
The tree and raw view use that same cached JSON or TREE cell. Original text
is preserved, including unknown fields, iterator descriptions and numeric tokens;
tree metrics are a bounded display projection. Unknown or malformed formats leave raw/grid access
available. Tree parsing stops at 512 KiB, depth 40 or 2,000 visited nodes/items.
Normal result-store and display-cache bounds still apply; no new persistence
or credentials are introduced.

PostgreSQL nodes show schema/relation, optimizer costs and estimated rows.
Actual plans display rows and times per loop, loop counts and server
planning/execution milliseconds. MySQL/MariaDB adapters accept their distinct
nested table/access/index/cost shapes and MariaDB `r_*` measurements. Optimizer
costs never receive millisecond labels; estimates and actuals stay separate.
MariaDB `r_rows` is the average per loop; `r_filtered` is an actual percentage.
`r_total_time_ms` is inclusive of child nodes, not a per-loop value; node times
must not be summed. `query_optimization.r_total_time_ms` is labeled separately
as server optimization, including zero. Constant plans may report optimization
without row/execution metrics: absent metrics remain absent. No statement-wide
execution or commit time is inferred. These labels follow the
[official MariaDB ANALYZE FORMAT=JSON contract](https://mariadb.com/docs/server/reference/sql-statements/administrative-sql-statements/analyze-and-explain-statements/analyze-format-json).

MySQL TREE parsing recognizes four-space iterator indentation, decimal and
engineering numeric tokens, cost ranges, missing costs and `never executed`.
Unknown iterator descriptions stay intact; unsupported continuations or broken
hierarchy fall back to raw/grid access. Actual first-row and inclusive total
times are milliseconds **per loop**; actual rows are per loop and loops are
separate. Child-inclusive times must not be summed. Unexecuted iterators receive
no invented zero rows/times/loops, and no statement-wide duration is inferred.
The adapter follows the [official MySQL EXPLAIN contract](https://dev.mysql.com/doc/refman/8.4/en/explain.html).

## Snowflake estimated logical plans

Explicit Explain compiles one validated query with `EXPLAIN USING JSON`; it
does not execute the target or change warehouse/session settings. The parser
uses the Snowflake dialect, including supported QUALIFY syntax. Because the SQL
parser does not understand USING JSON, Rust recognizes only the exact generated
wrapper, validates its entire inner query and applies the existing read-only
classification. Prefix lookalikes, batches, mutations and unclassified SQL remain
blocked in ReadOnly. The owned SQL API request carries its configured database, schema,
role, warehouse and query tag, without separately submitting the target query.

The bounded adapter preserves each step and every parent edge in its logical
DAG. Operator IDs are local to a step. Each operator expands once per step;
additional parents contain clearly labeled metric-free references. Duplicate
IDs/edges, missing parents, cycles, malformed shapes and unsupported data keep
raw/grid access. Payload is capped at 512 KiB, depth at 40 and a shared budget
at 2,000 step/operator/edge items, preventing exponential shared-node expansion.
Object names, aliases and expressions remain text. Unknown fields and numeric
tokens remain exact in raw JSON. Numeric estimates outside JavaScript's exact
integer range display a raw-view notice rather than a rounded value.

Global/node assigned partitions and bytes are compile-time upper-bound pruning
estimates; runtime pruning can reduce them. They are not actual rows, durations
or optimizer cost units. Logical structure does not imply execution order.
Warehouse size can affect the plan, and compilation can consume Cloud Services
credits. These semantics follow the [official Snowflake EXPLAIN reference](https://docs.snowflake.com/en/sql-reference/sql/explain).

Snowflake snapshot capture and comparison are explicitly unavailable. Its
current session state is cached configuration and does not observe warehouse
identity; it must not be presented as comparable execution context. The view
shows exact executed SQL and this limitation; the store rejects captures even
if called directly. No Snowflake Analyze action or live-account qualification
is claimed. The following comparison contract applies to PostgreSQL/MySQL/MariaDB.

## Explicit snapshots and comparison

**Keep snapshot** retains the current completed plan and its execution identity.
It never executes SQL. Up to ten snapshots share a global 2 MiB UTF-8 serialized
payload budget, including raw plan text, exact SQL and context; oldest captures are
evicted at either limit. Each raw plan still obeys the 512 KiB/depth/node parser
limits. Invalid, oversized or unsupported captures fail before eviction. These
are payload limits, not total JavaScript memory measurements.

Identity comes from the immutable prepared execution, not the current editor.
Only recognized single-query generated wrappers with known session context can
be captured. Engine/version, connection/session, observed database, role,
search path, active context, transaction mode/state must
match; underlying query bytes must match exactly. The known estimated/actual
wrapper is removed only for this query identity comparison. Incompatible
snapshots remain listed with a disabled reason. Arbitrary manually entered
EXPLAIN results remain viewable but are not assumed comparable.

Compatible plans display independent trees or exact raw plan text side by side.
Estimates, per-loop measurements and optimizer costs keep their original labels;
no node pairing by position, guessed delta or cost-to-millisecond conversion.
Context is observed by Rust after acquiring the serialized editor lane,
immediately before executing a recognized single-plan wrapper. This includes
preceding queued session changes. The identity uses that run's observation,
never frontend cached context or the current editor. Failed or unavailable
observation disables capture while retaining ordinary plan viewing. The probe
adds a metadata round trip and is included in backend active time. It is not a
transactional database snapshot: data and unobserved settings can still differ.
Captures preserve unknown fields and exact raw numeric tokens, survive ordinary
result replacement, and clear with their owning tab, session retirement or
connection disconnect. Individual removal and clear-all are explicit actions.
Snapshots and execution metadata are excluded from workspace persistence.

## Execution timing

The query results **Execution timing** disclosure shows independently measured
scopes in milliseconds. Backend durations are monotonic, floored and saturated
to `u32::MAX`; client elapsed is monotonic and rounded for display. Zero is a
valid measurement, distinct from unavailable.

| Metric | Scope |
|---|---|
| Queue wait | Backend run-task entry to acquisition of the editor execution lane; excludes scheduling before that task starts |
| Backend active | Lane acquisition through query execution, observed context and terminal session cleanup/state checks |
| Backend total | Task entry to that same terminal checkpoint; excludes history/audit persistence and event delivery |
| First row batch | Lane acquisition to the first nonempty batch delivered by a driver to the result sink, across all statements in the run; excludes queue wait |
| Client elapsed | Immediately before the execute request through application of its finished event, including IPC/poll delivery and event batching; failed execute requests measure through rejection |

First row batch is not time to the first wire row or screen paint: drivers may
buffer an entire result before delivering a batch. It remains unavailable for
empty results, pre-row errors and cancellations without rows. A later statement
failure does not erase an earlier batch measurement. Transport failures without
backend metrics keep those values unavailable. Stale revisions and foreign-run
metrics cannot overwrite current measurements. Timings are transient and clear
when a console is rebound; they are excluded from workspace restoration.

PostgreSQL plan `Planning Time` / `Execution Time` remain separately labeled
server values. No general server execution time, network duration, rendering
time, per-loop total, or timing delta is inferred from these measurements.

## Verification and remaining work

Snowflake-only code `48dfb21`: `pnpm validate` passes 148 frontend tests in
28 files and 136 standard Rust tests (115 core + 21 integration), with complete
build/types/formatting/check/strict Clippy. Four synthetic Snowflake browser
flows pass: two new plan workflows plus existing catalog/completion and
discovery-failure flows. Rust loopback HTTP verifies exact wrapper submission,
request context and lossless raw plan transport without SSO or a live account.
Adapter tests cover multiple steps/parents, order-independent edges, zero and
unsafe numeric estimates, invalid graphs, byte/depth/item/edge bounds, capture
refusal and result identity reset. Existing PostgreSQL/MySQL/MariaDB unit and
snapshot tests pass in the full frontend/core suites.

The preceding MySQL iteration's 26 PostgreSQL fixtures, 25 MySQL/MariaDB base
fixtures and eighteen plan/timing/console browser workflows remain prior
evidence; they were not rerun for this Snowflake-only transport slice. Current
native provenance and unrun gates are in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-snowflake-estimated-logical-plans-1-november-2026).

The first R5 integration slice `e0dad5f` adds live FK JOIN completion. Current
standard validation passes 152 frontend / 136 Rust, two live metadata fixture
tests and fourteen completion/Snowflake browser flows. Other R6 plan/timing
browser suites and native/package gates were not rerun for this slice; the
earlier native provenance remains specific to `48dfb21`.

The next R5 integration slice `ce7a7fa` adds structured backend Unicode
diagnostics while preserving R6 safety/cancellation. Current validation passes
152 frontend / 146 Rust, all 54 PostgreSQL/MySQL/MariaDB fixture tests, 22
diagnostic/FK/plan browser flows and three hidden native runs / 30 cleanup cycles.
Fourteen plan/timing and two synthetic Snowflake plan browser flows are included.
Normal distribution bundle verification and full browser/Redis/SSH suites were
not rerun. Explain validation reports parser positions in its unwrapped input;
runtime positions still refer to prepared SQL. Original editor selection and
generated-wrapper mapping must be integrated with parameters/markers. See
[exact diagnostics evidence](IMPLEMENTATION_STATUS.md#r5-integration-structured-unicode-diagnostics-1-november-2026).

The typed-parameter integration `39359e7` now routes Explain and Analyze through
the same immutable Rust binding plan as query execution. Exact reviewed types,
values and NULL participate in one-use approval; mandatory Analyze review and
ReadOnly/production gates remain enforced. PostgreSQL prepares the underlying
query for EXPLAIN EXECUTE; MySQL/MariaDB use protocol binds, and Snowflake SQL
API binding coverage remains synthetic. Snapshots require matching canonical
binding digests and never retain a separate value copy. Generated Explain error
prefixes map back to input query coordinates; full editor-selection markers
remain pending. Validation passes 156 frontend / 154 Rust, all 58 SQL fixture
tests and 23 relevant browser flows; see
[exact parameter evidence](IMPLEMENTATION_STATUS.md#r5-integration-typed-query-parameters-1-november-2026).

Inline integration `65530c0` now carries exact document/slice origins through the
parameter and review dialogs for Run, Explain and Analyze. Structured errors keep
their backend input-query coordinates; the editor applies the slice offset once
and converts file line endings into CodeMirror newline units. Error marks and jumps
reject changed documents and invalid source spans. Real
PostgreSQL/MySQL/MariaDB flows cover selected parameterized Explain/Analyze errors
and a delayed error after editing. All 26 relevant browser flows and standard
validation (168 frontend / 154 Rust) pass; the prior 58 ignored SQL fixture passes
were not rerun for this frontend-only change. Clean-source macOS arm64 hidden
build and three WebView runs / 30 cleanup cycles pass; synthetic baseline only. See
[inline evidence](IMPLEMENTATION_STATUS.md#r5-integration-inline-diagnostics-and-navigation-1-november-2026).

SQL symbol integration `8ef9dfb` adds scoped definitions/usages and quick
documentation without changing plan preparation or authorization. Final
validation passes 183 frontend / 154 Rust and 46 relevant real database browser
flows, including all fourteen plan/timing and six parameter flows. Clean-source
macOS baseline passes three processes / 30 cleanup cycles. See
[symbol evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-sql-definitions-usages-and-documentation-1-november-2026).

Formatting/snippet integration `db5cd1b` preserves every R6 preparation,
authorization and exact-source contract. Selection formatting and serialized
global preferences coexist with scoped templates. All 86 local browser flows
pass, including parameter/plan/navigation flows and simulated Snowflake; standard
validation passes 194 frontend / 155 Rust. Clean-source macOS hidden baseline
passes three processes / 30 cleanup cycles. See [formatting evidence](IMPLEMENTATION_STATUS.md#r5-integration-formatting-and-snippets-2-november-2026).

Combined candidate `b80b6c7` subsequently passed 66 ignored local SQL/Redis/SSH
fixtures and a normal ad-hoc macOS bundle build. Style follow-up `d39b89b` adds
per-engine preferences with 196 frontend / 156 Rust, three expanded real editor
browser flows and a clean-source native baseline (three processes / 30 cycles).
It does not change R6 authorization; the prior full 86-flow browser run and
combined fixture/bundle counts retain their recorded revisions. Next continue
secondary-session Structure navigation and broader semantic inference; keep
the earlier metadata-startup flake open and design observed Snowflake warehouse/
session identity before enabling snapshots. See the
[roadmap](implementation-roadmap.md#r6-next-slices).
This local candidate is unmerged, unpublished, not installed and not externally
qualified. PostgreSQL's execution and measurement semantics are documented in
the [official EXPLAIN reference](https://www.postgresql.org/docs/current/sql-explain.html).

Named-output follow-up `c32f0c2` adds bounded CTE/derived-table local navigation
without changing R6 execution, preparation, approval or exact-source contracts.
Standard checks pass 228 frontend / 156 Rust and five SQL navigation browser flows;
the clean-source native synthetic baseline passes three processes / 30 cycles.
The new output interactions are browser-tested. Prior full plan/browser/fixture/
normal-bundle qualification retains its recorded revisions; no new external or
release qualification is implied. See [current evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-named-query-outputs-2-november-2026).

Console-bound Structure follow-up `fe518fb` preserves R6 authorization, immutable
bindings and exact selected-source origins. Its read-only inspectors use the
originating console/session/context and never execute SQL or browse table data.
Validation passes 237 frontend / 156 Rust and six navigation browser flows; the
clean-source hidden native baseline passes three processes / 30 cycles. New
inspector interactions remain browser-tested. Prior broad plan/browser/fixture/
normal-bundle evidence retains its exact revisions. Context-preserving metadata
links and bounded editor refactoring are next; Snowflake capture remains gated
on observed identity and explicit external qualification. See [current evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-console-bound-structure-2-november-2026).
