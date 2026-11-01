# Query plans

R6 local `feat/estimated-query-plans` adds **Explain** for PostgreSQL and
MySQL/MariaDB (`fad0341`), then **Analyze** for PostgreSQL (`fa1ff17`). Both
use the current statement or explicit selection without changing editor text.
Explicit bounded plan snapshot comparison follows in `ba984f2`.
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

## Explicit snapshots and comparison

**Keep snapshot** retains the current completed plan and its execution identity.
It never executes SQL. Up to ten snapshots share a global 2 MiB UTF-8 serialized
payload budget, including raw JSON, exact SQL and context; oldest captures are
evicted at either limit. Each raw plan still obeys the 512 KiB/depth/node parser
limits. Invalid, oversized or unsupported captures fail before eviction. These
are payload limits, not total JavaScript memory measurements.

Identity comes from the immutable prepared execution, not the current editor.
Only recognized single-query generated wrappers with known session context can
be captured. Engine/version, connection/session, observed database, role,
search path, active context, transaction mode/state and context revision must
match; underlying query bytes must match exactly. The known estimated/actual
wrapper is removed only for this query identity comparison. Incompatible
snapshots remain listed with a disabled reason. Arbitrary manually entered
EXPLAIN results remain viewable but are not assumed comparable.

Compatible plans display independent trees or exact raw JSON side by side.
Estimates, per-loop measurements and optimizer costs keep their original labels;
no node pairing by position, guessed delta or cost-to-millisecond conversion.
The context is the frontend's last observation at the execution request, not
an atomic server-context snapshot; queued or unobserved session changes can
differ. The comparison does not prove equal database state or settings.
Captures preserve unknown fields and exact raw numeric tokens, survive ordinary
result replacement, and clear with their owning tab, session retirement or
connection disconnect. Individual removal and clear-all are explicit actions.
Snapshots and execution metadata are excluded from workspace persistence.

## Verification and remaining work

Four frontend adapter tests cover engine shapes, metric labels, unknown
formats and complexity limits. Four Rust wrapper tests and Safe Mode
regressions cover byte preservation, complete-query rejection, nested risks
and mandatory analysis review, including PostgreSQL's parenthesized options.
Two PostgreSQL fixture tests from the prior unchanged-backend slice prove
no unapproved execution, one-use/stale
approval, owned temporary state, disconnect invalidation, errors and cancellation.
Six disposable-engine plan browser flows and four console/file regressions
pass. Six snapshot unit tests and two studio lifecycle tests cover exact query
identity, context/engine mismatch, count/byte eviction, invalid input, cleanup
and persistence exclusion. Browser flows verify estimated/actual comparison,
editor changes, no execution on capture/comparison, mismatch rejection and
cancellation/disconnect/reconnect cleanup. Exact standard validation and
macOS bundle/hidden-native evidence are in
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md#r6-bounded-plan-snapshots-and-comparison-1-november-2026).

Next: distinct queue, first-row, client/server timing,
and independent engine/version contracts for MySQL/MariaDB actual plans and
Snowflake plans. See the [roadmap](implementation-roadmap.md#r6-next-slices).
This local candidate is unmerged, unpublished, not installed and not externally
qualified. PostgreSQL's execution and measurement semantics are documented in
the [official EXPLAIN reference](https://www.postgresql.org/docs/current/sql-explain.html).
