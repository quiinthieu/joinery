# Proposed Snowflake SDK browsing and session context

This is a reviewed proposal, not implemented behavior or Snowflake account
qualification. The current SDK slice provides live-context relation completion
and lazy zero-row column suggestions; it deliberately does not provide complete
inspection, table browsing or session-state observation. Preserve that distinction
and the remaining [replacement requirements](snowflake-datagrip-replacement.md).
The [metadata design](snowflake-sdk-metadata-design.md) records the existing slice.

## Smallest coherent workflow

Implement **current-database catalog → inspect → bounded preview → refresh**,
with explicit current-schema switching and truthful session observation on the
same SDK connection. Enable `tableBrowsing` only after this whole workflow works.
Keep writes, DDL, transaction controls, filter/order support and continuation
disabled. Cross-database targets and switching remain required later work.

Reuse the pure builders/parsers in
[`snowflake_metadata.rs`](../src-tauri/src/drivers/snowflake_metadata.rs), the
bounded collector and driver lane in
[`snowflake_external.rs`](../src-tauri/src/drivers/snowflake_external.rs), and
Studio's revision/invalidation patterns in
[`execution.rs`](../src-tauri/src/execution.rs). Lock order stays Studio session
lane → SDK driver lane. Hold the driver lane across each operation's observations,
data collection and context recheck; private readers must not reacquire it.
Do not introduce an OAuth session or independent metadata lane.

## Target identity and DTO recommendation

[`TableRef`](../src-tauri/src/domain.rs) contains only schema/name. Recommended
new browsing DTOs bind actions to a server-held catalog:

```text
BrowseCatalog { revision, identity, database, relations }
Relation { targetId, table, tableType, traits, availability }
InspectRequest { revision, targetId }
PreviewRequest { revision, targetId, inspectionRevision }
```

Behind each target retain database, session generation, context fingerprint and
fully qualified spelling. Reject stale/missing targets before SQL. These are
recommended seams, not mandatory established contracts. A smaller first slice
could retain `TableRef` if it proves and enforces an immutable initial-database
invariant on every operation, keeps database switching disabled and invalidates
all targets on context/session drift. It must never apply an old reference to a
new database. Cross-database support will still need qualified identity or an
equivalent target contract.

Preserve `TABLE_TYPE` and transient, temporary, Iceberg, dynamic, hybrid and
immutable flags independently. `IS_IMMUTABLE` identifies the `READ ONLY` table
property; it is not a dynamic table's frozen region. None of these flags enables
editing. Keep permission errors and unknown kinds explicit rather than flattening
everything to an ordinary table. [TABLES fields and visibility](https://docs.snowflake.com/en/sql-reference/info-schema/tables).

## Fixed SQL, bounds and inspection

In the templates below, `D`, `S`, `T` and projected column names are separately
validated and double-quoted identifier components; `d`, `s`, `t` are separately
escaped string literals. They are never caller SQL fragments. Metadata uses a
private trusted path, without expanding the public SQL allowlist.

Reuse the current fixed context/policy reads and selective SCHEMATA/per-schema
TABLES queries. Retain limits of 1,000 schemas, 10,000 aggregate relations and
2 MiB per complete catalog. Select `CREATED` and `LAST_DDL` as additional
best-effort change indicators. Overflow fails refresh rather than returning a
partial catalog. Information Schema is privilege-filtered, requires selective
predicates and does not guarantee consistency against concurrent DDL. Label the
result as visible relations, not an authoritative inventory.
[Information Schema guidance](https://docs.snowflake.com/en/sql-reference/info-schema).

For complete columns on an unambiguous target, use:

```sql
SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME,
       COLUMN_NAME, ORDINAL_POSITION, COLUMN_DEFAULT, IS_NULLABLE,
       DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, CHARACTER_OCTET_LENGTH,
       NUMERIC_PRECISION, NUMERIC_PRECISION_RADIX, NUMERIC_SCALE,
       IS_IDENTITY, IDENTITY_GENERATION, IDENTITY_START,
       IDENTITY_INCREMENT, IDENTITY_ORDERED, COMMENT, EXPRESSION, KIND
FROM "D".INFORMATION_SCHEMA.COLUMNS
WHERE TABLE_CATALOG = 'd' AND TABLE_SCHEMA = 's' AND TABLE_NAME = 't'
ORDER BY ORDINAL_POSITION
LIMIT 1001;

DESCRIBE TABLE "D"."S"."T" TYPE = COLUMNS;

SELECT * FROM "D"."S"."T" LIMIT 0;
```

Bound each collected column result to 1,000 rows/1 MiB and the entire operation
to the existing 65-second metadata deadline. The sentinel, terminal row count,
shape/types and duplicate names must validate before publication. Require positive,
unique, increasing `ORDINAL_POSITION` values and compare their ordering with the
resolved column order. Do not require contiguous values without qualifying
Snowflake's behavior after `DROP COLUMN`.
`DESCRIBE` supplies the resolved full type and column details; the zero-row read
checks actual ordered result columns and SELECT access. Compare SDK type families
through an explicit normalization map: wire `FIXED` alone is not a complete
`NUMBER(p,s)` type. Preserve full type text. Do not invent temporal precision from
`DATETIME_PRECISION`, documented as not applicable.
[COLUMNS](https://docs.snowflake.com/en/sql-reference/info-schema/columns),
[DESCRIBE TABLE](https://docs.snowflake.com/en/sql-reference/sql/desc-table).

Recommend a separate `RelationInspection`/`ColumnInspection` DTO for actual
default expressions, nullability, lengths/precision/scale, identity attributes,
column kind/expression and comments. `ColumnInfo` remains a read-only grid
projection. Represent unavailable/not-applicable details explicitly. A failed
query is not an empty column list; unfetched keys are not empty authoritative
key arrays. Constraint details and enforcement require their own status and
qualification; do not infer standard-table enforcement or edit eligibility.

Temporary and permanent same-name rows must remain distinct. The temporary table
shadows the permanent table even under fully qualified spelling; mark the latter
shadowed and reject its actions. Never merge duplicate COLUMNS rows or pick one
arbitrarily. Complete inspection of a shadowing temporary target is an acceptance
gate: qualify resolved DESCRIBE/SHOW COLUMNS behavior and its identity/default
attributes under an actual collision. Until proven, report ambiguous inspection.
`SHOW COLUMNS IN TABLE "D"."S"."T"` is a candidate bounded, table-scoped source
for structured type and autoincrement details, not an assumed equivalent fallback.
[Temporary precedence](https://docs.snowflake.com/en/user-guide/tables-temp-transient),
[SHOW COLUMNS](https://docs.snowflake.com/en/sql-reference/sql/show-columns).

Preview uses the verified inspection's explicit quoted projection:

```sql
SELECT "column1", "column2" FROM "D"."S"."T" LIMIT 501;
```

Retain 500 rows/2 MiB, report truncation from the extra row, return no continuation
key and fail clearly if the first row exceeds the byte bound. Reject unsupported
filters/order/continuation. Recheck context/session generation, catalog membership
and ordered columns before publishing. Bracket inspection with selected relation
metadata checks and reject observed changes. These checks detect drift; they do
not guarantee atomic object identity against concurrent external DDL.

## Schema switching and refresh

Recommend a typed helper operation such as
`switchSchema { database, schema, expectedServerSession }`. The existing helper
`context` operation has setup assertions and simple-identifier restrictions; it
is not a generic live observer. Do not send arbitrary SQL labelled as read.
The only switch statement is:

```sql
USE SCHEMA "D"."S";
```

Under the existing lanes, validate current context and selected schema, invalidate
completion/browse/inspection snapshots and prepared plans **before dispatch**,
switch once, then read back the full context and identifier policy. Acknowledge
only the intended database/schema on the same server session with unchanged
identity fields. Unknown outcome or failed readback means unknown context or
session retirement, never a replay or an asserted restoration of old state.
[USE SCHEMA](https://docs.snowflake.com/en/sql-reference/sql/use-schema).

Generic integration needs two bounded repairs: Studio currently invalidates
completion/plans only after `switch_context` succeeds, and
`build_session_contexts` turns a `list_contexts` error into an empty list. Invalidate
before attempted switching and distinguish unavailable choices from a successful
empty visible set. Failed refresh removes the old snapshot; stale replies cannot
restore it or clear a newer revision. A per-table denial can remain local only
when context observation succeeds and the session remains valid.

## Truthful session observation and unknown state

Recommended minimal representation is `Unknown` in transaction mode/state and an
optional textual transaction ID; a separate typed observation result is another
valid design. The existing mandatory AutoCommit/Idle defaults cannot represent
failed observations truthfully. Keep mode independent from activity: autocommit
enabled does not exclude an explicit transaction.

Use fixed private reads, bracketed by context/session checks:

```sql
SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION;
SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION;
SELECT CURRENT_TRANSACTION()::VARCHAR AS TRANSACTION_ID;
```

Bound each observation to one row/64 KiB, validate exact parameter keys/Boolean
values and retain transaction IDs as text rather than JavaScript numbers. Server
session identity remains opaque VARCHAR. Map a missing transaction ID to idle
only after qualifying its real no-transaction behavior. Never set AUTOCOMMIT as
an observation: even setting its existing value can commit an active transaction.
[SHOW PARAMETERS](https://docs.snowflake.com/en/sql-reference/sql/show-parameters),
[CURRENT_TRANSACTION](https://docs.snowflake.com/en/sql-reference/functions/current_transaction),
[transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

An ordinary statement failure does not always abort a Snowflake transaction.
With `TRANSACTION_ABORT_ON_ERROR=true`, statements in an aborted transaction fail
until transaction completion. The reviewed CURRENT_TRANSACTION, SHOW TRANSACTIONS
and DESCRIBE TRANSACTION references do not establish a complete read-only
aborted-session discriminator. Observation failure must remain unknown, never
infer idle or trigger automatic commit/rollback/recovery. Metadata errors can
themselves matter under abort-on-error; read-only does not promise unchanged
transaction state after failure.
[Abort-on-error](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error),
[SHOW TRANSACTIONS](https://docs.snowflake.com/en/sql-reference/sql/show-transactions),
[DESCRIBE TRANSACTION](https://docs.snowflake.com/en/sql-reference/sql/desc-transaction).

Display unknown context/transaction state explicitly. Keep write/TX controls
disabled; future write admission must reject unknown state. Close confirmation
must handle uncertainty conservatively without claiming confirmed rollback.
Do not automatically commit, roll back or change autocommit to make browsing work.
All these metadata/session reads advance statement history: preserve the existing
LAST_QUERY_ID/RESULT_SCAN rejection until a separately qualified explicit query-ID
workflow exists.

## Phased integration and acceptance

1. Add pure inspection SQL/parsers and the chosen target/observation contracts.
   Test identifier injection, quoted names, type/null/default validation,
   duplicate targets, bounds and unavailable states before enabling capabilities.
2. Add SDK catalog/inspect/preview operations on the existing helper connection,
   then Studio snapshot admission and frontend invalidation. Actual-helper tests
   must cover drift at every phase, denied/malformed results, preview limits,
   stale targets submitting no SQL and generation replacement without reconnect.
3. Add typed schema switching and session observation. Regress generic list
   failure and pre-dispatch invalidation, including failed/lost switch response,
   unknown state and conservative close/admission behavior. Browser acceptance
   exercises catalog → inspect → preview → refresh and switch → verified refresh.
4. Before claiming account behavior, obtain a bounded disposable-object statement
   plan and authorization for warehouse use. Qualify actual metadata shapes,
   quoted names, dropped-column ordinal gaps, temporary collisions, privilege
   omissions, supported relation traits, external DDL and transaction observers
   with autocommit on/off,
   explicit transactions and abort-on-error both ways. Transaction mutation tests
   need separate authorization; this proposal enables none of them.

Synthetic/local tests establish application behavior only. Incomplete temporary
inspection or aborted-state observation remains an explicit gate, not a reason to
claim this workflow complete. Full cross-database browsing, writes, DDL,
transactions and broader object inventory remain in the replacement scope.

The read-only investigation used Tier 2 graph search/trace/snippets plus direct
source reads, generation `2026-11-10T06:16:25Z`; the relevant source paths/scopes
reported no recorded coverage gaps. That signal is best effort. No builds,
authentication or account queries were performed for this recommendation.
