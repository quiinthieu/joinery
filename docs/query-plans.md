# Estimated query plans

The first R6 slice (`fad0341`, local `feat/estimated-query-plans`) adds an
**Explain** action for PostgreSQL and MySQL/MariaDB query tabs. It uses the
current statement or explicit selection without modifying editor text. It
does not request executing ANALYZE. Snowflake and Redis have no action.

## Preparation and authorization

Rust parses the entire selected input before wrapping it. Exactly one
`Statement::Query` (SELECT/WITH) is accepted. Batches, mutations, existing
EXPLAIN, malformed/empty SQL, MySQL executable comments and oversized input
are rejected. The generated statement is capped at 1 MiB. Original query bytes
are retained after `EXPLAIN (FORMAT JSON)` (PostgreSQL) or
`EXPLAIN FORMAT=JSON` (MySQL/MariaDB).

`Studio::prepare_explain` resolves the actual session engine and delegates the
generated SQL to existing `prepare_execution`. The immutable plan, Safe Mode,
production passphrase, one-use authorization, owned session and cancellation
path remain authoritative. A query containing a modifying CTE, side-effect
function or locking clause still receives normal Rust classification; tests
verify ReadOnly denial and production review. Estimated planning is not a
guarantee of zero database work or side effects: database grants remain the
access boundary.

The new command is enrolled in the desktop Tauri ACL and test harness. The
hidden native check verifies that its real WebView IPC reaches session
validation rather than failing ACL resolution.

## Raw, tree and grid views

A retained one-cell `QUERY PLAN` / `EXPLAIN` result exposes **View plan**.
The tree and raw view use that same cached JSON cell. Raw JSON is preserved as
text, including unknown fields and exact numeric tokens; tree metrics are a
bounded display projection. Unknown or malformed formats leave raw/grid access
available. Tree parsing stops at 512 KiB, depth 40 or 2,000 visited nodes/items.
Normal result-store and display-cache bounds still apply; no new persistence
or credentials are introduced.

PostgreSQL nodes show schema/relation, optimizer costs and estimated rows.
Existing raw actual plans can display actual rows and times per loop plus
server planning/execution milliseconds. MySQL/MariaDB adapters accept their
distinct nested table/access/index/cost shapes and MariaDB `r_*` measurements.
Optimizer costs never receive millisecond labels; estimates and actuals stay
separate. This does not qualify an executing actual-plan action or MySQL
ANALYZE output compatibility.

## Verification and remaining work

Four frontend adapter tests cover engine shapes, metric labels, unknown
formats and complexity limits. Three Rust wrapper tests plus the Safe Mode
regression cover byte preservation, batch/mutation/comment rejection and
nested-risk authorization. Three disposable-engine browser flows exercise
isolated sessions, unchanged SQL, tree/raw/grid views and batch rejection.
Four existing console/file browser regressions also pass. Complete standard
validation and macOS bundle/hidden-native evidence are recorded in
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md#current-assessment-and-r6-estimated-plans-1-november-2026).

Next is an explicitly reviewed PostgreSQL ANALYZE action, including Silent
local profiles, followed by independent MySQL/MariaDB capability contracts,
bounded in-memory comparison and distinct client/server timing. The
[roadmap](implementation-roadmap.md#r6-slice-2-explicitly-reviewed-actual-plans)
defines acceptance. R6 remains partial; this candidate is not merged,
published, installed or externally qualified.
