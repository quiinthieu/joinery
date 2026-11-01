# Query plans

R6 local `feat/estimated-query-plans` adds **Explain** for PostgreSQL and
MySQL/MariaDB (`fad0341`), then **Analyze** for PostgreSQL (`fa1ff17`). Both
use the current statement or explicit selection without changing editor text.
Explicit bounded plan snapshot comparison follows in `ba984f2`; execution timing
and observed context in `cb1ca1a`; reviewed MariaDB actual plans in `c660bc4`
and MySQL TREE analysis in `c54ecab`.
Snowflake and Redis have no plan action. R6 remains partial and unreleased.

## Preparation and authorization

Rust parses the entire selected input before wrapping it. Exactly one
`Statement::Query` (SELECT/WITH) is accepted. Batches, mutations, existing
EXPLAIN, malformed/empty SQL, MySQL executable comments and oversized input
are rejected. The generated statement is capped at 1 MiB. Original query bytes
are retained after `EXPLAIN (FORMAT JSON)` (PostgreSQL),
`EXPLAIN FORMAT=JSON` (MySQL/MariaDB),
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

Eight frontend adapter tests cover JSON/TREE engine shapes, inclusive versus
per-loop metrics, engineering numeric text, unexecuted iterators, unsupported
formats and complexity limits. Four Rust wrapper tests and Safe Mode regressions
cover byte preservation, complete-query rejection, executable comments, nested
risks and mandatory review, including parser fallback and DESC/DESCRIBE aliases.
Eight snapshot tests and two lifecycle tests cover identity, bounds and cleanup.

Standard validation passes 142 frontend and 133 standard Rust tests. All 26
PostgreSQL fixtures and 25 MySQL/MariaDB base fixtures pass; the separate
case-folded fixture was excluded. Fourteen plan/timing browser workflows and
four console/file regressions pass on disposable engines. MySQL and MariaDB
flows cover review cancellation without execute, unchanged SQL, raw/tree/grid,
direct-SQL review, ReadOnly denial, estimated/actual comparison without replay,
mismatch/cleanup, error recovery and cancellation retirement. MySQL also proves
JSON settings are preserved and server output contains unexecuted iterators.
Exact macOS evidence and unrun checks are recorded in
[implementation evidence](IMPLEMENTATION_STATUS.md#r6-reviewed-mysql-tree-actual-plans-1-november-2026).

Next: Snowflake estimated plans and integrated R5/R6 qualification. See the
[roadmap](implementation-roadmap.md#r6-next-slices).
This local candidate is unmerged, unpublished, not installed and not externally
qualified. PostgreSQL's execution and measurement semantics are documented in
the [official EXPLAIN reference](https://www.postgresql.org/docs/current/sql-explain.html).
