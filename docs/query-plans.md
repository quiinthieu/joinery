# Query plans

R6 local `feat/estimated-query-plans` adds **Explain** for PostgreSQL and
MySQL/MariaDB (`fad0341`), then **Analyze** for PostgreSQL (`fa1ff17`). Both
use the current statement or explicit selection without changing editor text.
Snowflake and Redis have no plan action. R6 remains partial and unreleased.

## Preparation and authorization

Rust parses the entire selected input before wrapping it. Exactly one
`Statement::Query` (SELECT/WITH) is accepted. Batches, mutations, existing
EXPLAIN, malformed/empty SQL, MySQL executable comments and oversized input
are rejected. The generated statement is capped at 1 MiB. Original query bytes
are retained after `EXPLAIN (FORMAT JSON)` (PostgreSQL),
`EXPLAIN FORMAT=JSON` (MySQL/MariaDB), or
`EXPLAIN (ANALYZE, FORMAT JSON)` (PostgreSQL actual measurements).

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
the plan is stored and fingerprinted. Explicit `ANALYZE FALSE`, `OFF` or `0`
does not execute analysis. ReadOnly rejects executing analysis; production
profiles retain passphrase requirements. Review displays the exact generated
SQL and execution warning. Cancelling review sends no execution request.

Approval is single use and bound to the exact plan/session. A replacement
preview or disconnected session invalidates it. Queries run on the owned
console session, preserving temporary objects and transaction state. A query
error uses normal recovery; cancellation retires the session rather than
replaying SQL. No analysis runs automatically on editor changes.

The command is enrolled in the desktop Tauri ACL and test harness. The hidden
native check verifies that both estimated and actual preparation reach session
validation through real WebView IPC rather than failing ACL resolution.

## Raw, tree and grid views

A retained one-cell `QUERY PLAN` / `EXPLAIN` result exposes **View plan**.
The tree and raw view use that same cached JSON cell. Raw JSON is preserved as
text, including unknown fields and exact numeric tokens; tree metrics are a
bounded display projection. Unknown or malformed formats leave raw/grid access
available. Tree parsing stops at 512 KiB, depth 40 or 2,000 visited nodes/items.
Normal result-store and display-cache bounds still apply; no new persistence
or credentials are introduced.

PostgreSQL nodes show schema/relation, optimizer costs and estimated rows.
Actual plans display rows and times per loop, loop counts and server
planning/execution milliseconds. MySQL/MariaDB adapters accept their distinct
nested table/access/index/cost shapes and MariaDB `r_*` measurements. Optimizer
costs never receive millisecond labels; estimates and actuals stay separate.
Raw-field support does not qualify an executing MySQL/MariaDB ANALYZE action.

## Verification and remaining work

Four frontend adapter tests cover engine shapes, metric labels, unknown
formats and complexity limits. Four Rust wrapper tests and Safe Mode
regressions cover byte preservation, complete-query rejection, nested risks
and mandatory analysis review, including PostgreSQL's parenthesized options.
Two PostgreSQL fixture tests prove no unapproved execution, one-use/stale
approval, owned temporary state, disconnect invalidation, errors and cancellation.
Five disposable-engine plan browser flows and four console/file regressions
pass. Exact standard validation and macOS bundle/hidden-native evidence are in
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md#r6-postgresql-actual-plans-1-november-2026).

Next: bounded in-memory snapshots/comparison, distinct client/server timing,
and independent engine/version contracts for MySQL/MariaDB actual plans and
Snowflake plans. See the [roadmap](implementation-roadmap.md#r6-next-slices).
This local candidate is unmerged, unpublished, not installed and not externally
qualified. PostgreSQL's execution and measurement semantics are documented in
the [official EXPLAIN reference](https://www.postgresql.org/docs/current/sql-explain.html).
