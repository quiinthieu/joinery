# Snowflake SDK relation browsing and session context

The current working source implements a narrow, read-only **Browse relations**
flow for SDK Snowflake sessions. It lists visible relations in the configured
current database, inspects a selected target, can show its generated definition,
and can show a bounded unordered preview. This is not complete DataGrip browsing, Snowflake account qualification,
or permission to use the old two-SELECT canary for metadata. The required
[replacement scope](snowflake-datagrip-replacement.md) remains open.

Joinery reports `relation_browsing` separately from generic `table_browsing`;
the SDK enables relation browsing while generic table browsing, grid editing,
and table filtering remain off. Separately reviewed transactions, SQL writes and
generated table changes have their own opt-in contracts; relation metadata grants
none of that authority. The relation modal has a
local name filter, but relation browsing does not enable grid writes or infer
that displayed metadata is authoritative. The route stays on the existing SDK
connection and serialized session lane; it does not create a second OAuth or
metadata session.

**Account scope warning:** merely opening a query editor may issue the existing
same-session completion metadata request. Opening **Browse relations** then
explicitly loads schemas and relation metadata. Both use the selected account,
role, and warehouse. The prior approval for two fixed SELECTs did not include
these operations. Do not connect an account for this workflow until separate
approval names the metadata scope and package. Previewing an existing relation
also reads up to 500 rows; it needs separate approval for the exact target and
data scope. The current
[account qualification plan](snowflake-account-qualification-plan.md) excludes
existing-table data reads.

## Reproduce the UI without Snowflake

The synthetic browser flow intercepts the SDK RPCs and supplies fixture results;
it performs no SSO or Snowflake SQL:

```sh
pnpm exec playwright test tests/e2e/snowflake-sdk-browsing.spec.ts
```

This checks the UI path and request shapes against synthetic responses. It does
not establish account metadata shapes, real SSO, warehouse behavior, or remote
data access.

## Account walkthrough after separate approval

Use this only after a reviewed approval covers (a) the schema and relation
metadata statements issued by query-editor completion and Browse relations,
(b) the selected role, database, warehouse, and run window, and (c) if preview is
approved, the exact relation whose rows may be read. Do not reuse the prior
two-query canary approval. If approval covers metadata but not existing-table
rows, stop after inspection and do not press **Preview rows**.

1. Connect the approved SDK profile and open a query tab. Opening the editor can
   run completion metadata; the approved scope must already cover it.
2. Choose **Browse relations** in that console. The modal loads the current
   database's visible schema and relation list. Use **Find relation** and select
   a listed target. Only targets marked available can be inspected.
3. Choose **Inspect relation**. Review the type, nullability, defaults, identity,
   kind, and comment observations. Column inspection does not fetch constraints.
   The separate **View key constraints** action reads declared primary, unique
   and foreign keys only when the approved scope covers those metadata views;
   see the [key inspection contract](snowflake-sdk-constraints-design.md).
   It does not resolve key column mappings or establish edit eligibility.
   The separate **View key column mappings** action resolves ordered declared
   primary/unique/foreign-key columns and qualified referenced tables only when
   the approved scope includes its declaration and per-table SHOW/pipe reads;
   see the [mapping contract](snowflake-sdk-key-mappings-design.md). This action
   remains independent of columns, definitions and the declaration view.
   If the approval includes `GET_DDL` on this target, **View definition** can be
   used before inspection. Review the warning and use **Copy DDL** to copy the
   complete returned text. This view has no execution action. A definition read
   needs its own approved scope even when no table rows are previewed.
4. Only for an explicitly approved row target, choose **Preview rows**. The
   preview is unordered, may show up to 500 rows, and has no server continuation.
   The 25-row and 20-column page controls operate on the retained local result.
5. Use **Refresh relations** after a known context or schema change. Refresh
   drops prior targets before loading; a failed or ambiguous refresh leaves old
   results unavailable. Resolve the permission or context issue under its own
   approval before retrying.

Without account approval, use the synthetic browser flow above. Do not substitute
the configured `STAGING` context or the earlier canary as implicit authorization.

## Implemented local contract

### Catalog and target identity

An explicit browse request reads schema names and then table metadata for each
visible schema in the session's configured database. It is privilege-filtered,
not an authoritative inventory. The bounded collector allows at most 1,000
schemas, 10,000 aggregate relations, and 2 MiB of catalog data. It rejects
truncation, missing schemas, permission errors, malformed rows, and aggregate
overflow instead of publishing a partial list. `CREATED` and `LAST_DDL` are
best-effort drift indicators, not immutable object IDs.

Each catalog carries the observed context identity, an opaque revision, and
opaque target IDs retained by the server for that session. Inspect and preview
must present the current revision and target; preview also requires the matching
inspection revision. The IDs bind server-held database/schema/name and traits,
but do not make name-based Snowflake reads atomic against external DDL.

Known relation traits are displayed independently: transient, temporary,
Iceberg, dynamic, hybrid, and immutable (`READ ONLY`). Unknown traits, unsupported
kinds, temporary/permanent shadows, and ambiguous same-name targets are marked
unavailable. The UI disables inspection for those entries. No target status
enables editing.

### Inspection and preview

For an available target, inspection combines three separate observations:

1. `INFORMATION_SCHEMA.COLUMNS` for ordered column metadata and dimensions.
2. A relation-specific `DESCRIBE` result for full types and descriptor fields.
3. `SELECT * FROM <qualified target> LIMIT 0` for resolved result columns.

The backend checks the session context and relation metadata before and after the
reads, checks response shape and bounds, and rejects disagreement rather than
publishing a partial inspection. Column positions must be increasing and unique;
contiguous positions are not assumed. `COLUMN_DEFAULT`, the `DESCRIBE` default,
and a separately reported `write default` remain distinct observations. If
`write default` is absent, the UI reports **not reported**. Constraint status is
`NotFetched`; absent keys are not represented as empty, authoritative key lists.

An approved preview uses the inspected, quoted column projection and
`LIMIT 501`. The backend retains at most 500 rows and 2 MiB, using the extra row
only to report truncation. It has no sorting, filtering, or continuation. The UI
pages the retained rows and columns locally (25 rows and 20 columns per page).

Catalog refresh revokes old server-held targets before waiting for the session
lane; a failed refresh leaves no usable catalog. Each inspection attempt revokes
its previous column snapshot before reading. Global context/session invalidation
also revokes the affected catalog generation. A local inspection/preview failure
does not publish partial data; the UI clears its catalog, inspection and preview
and requires refresh after any current request error. These checks detect
observed changes; they cannot prevent an external actor from changing an object
between name-based SQL statements.

The source preserves independent COLUMNS, DESCRIBE, and write-default
observations. Review reproduced and repaired loss of independent Iceberg initial
and write defaults: COLUMNS expression defaults and DESCRIBE defaults are retained
separately, and a missing write-default field differs from reported NULL. The
local parser and UI regressions pass; actual account descriptor shapes remain
unqualified. See the [official default semantics](https://docs.snowflake.com/en/user-guide/tables-iceberg-default-values)
and [verification ledger](replacement-verification.md#sdk-relation-browsing-10-november-2026).

### Generated relation definitions

The separate `define_relation` command accepts only the current catalog revision
and opaque target ID. The backend resolves the qualified name from its retained
target, uses the existing console/metadata lanes, and checks session context and
relation metadata before and after the read. It does not require a successful
column inspection, so an unmapped column type need not prevent definition viewing.
These checks detect observed drift; they do not make name-based reads atomic.

A closed builder issues one aliased `GET_DDL` query with independently quoted
database, schema and relation components, encoded as a SQL string argument. The
documented type mapping covers the existing available table/view families,
including external, hybrid, Iceberg, dynamic, event and materialized relations.
Unknown or unavailable targets are rejected without guessing another type.
One complete, non-NULL text cell and the enclosing response must fit the bounded
1 MiB budget. Truncated, malformed or oversized definitions are rejected rather
than exposed as copyable partial SQL.

The UI presents the generated text read-only with **Copy DDL**. Selection,
filtering, refresh, observed context/session changes and current-request failures
clear the previous definition; late responses cannot republish it. Definition
viewing does not enable the generic object explorer, grid editing or SQL writes.

Snowflake's [official GET_DDL reference](https://docs.snowflake.com/en/sql-reference/functions/get_ddl)
describes generated output rather than an exact original script: types and
properties may be normalized, views include `CREATE OR REPLACE`, and hidden
policy/tag assignments may contain placeholders. Fully qualifying the recreated
object does not qualify every reference inside a view. Secure-view access can
require ownership. The visible warning explains why returned text is not proof
that replay is safe or complete. No session parameter is changed to obtain it.

Focused regressions, full validation, the changed browser flow and hidden native
IPC checks pass. Actual account qualification remains open. See the ledger for the
tested source and results; source presence alone closes no replacement acceptance gate.

## Still required

- Actual-account qualification of the separately implemented
  [session observations](snowflake-sdk-session-observation.md) and
  [guarded schema selection](snowflake-sdk-session-design.md), including
  database, schema, role, warehouse, secondary roles and search path.
- Actual account qualification for schema visibility, quoted identifiers,
  object traits, collisions, permission failures, COLUMNS/DESCRIBE differences,
  preview result types, context continuity, warehouse use, and cleanup.
- Cross-database browsing, authoritative constraints, complete DDL tooling, schema diagrams,
  writes, DDL editing, transactions, and the required million-row result/export
  workflow.
- Unmapped type families, including DECFLOAT and MAP, currently fail inspection
  rather than publishing guessed type information. Extending and qualifying all
  applicable Snowflake types remains required.
- Broader relation-type and Iceberg qualification, including defaults and
  temporary/permanent collisions. A stale or unknown target must remain
  unavailable until those behaviors are established.

Synthetic UI and helper fixtures establish local behavior only. The accepted
DataGrip workflow and account qualification gates remain in the
[replacement queue](snowflake-datagrip-replacement.md) and
[acceptance record](replacement-acceptance.md).
