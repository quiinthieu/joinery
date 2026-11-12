# Snowflake declared key column mappings

## Scope and acceptance

The ordered-mapping increment is locally verified on 54f59b6; source, browser,
hidden native and package checks pass for that source. Referenced-table
selection is a separate increment below; source, browser and hidden native
checks pass, with normal packaging verification pending. The
[verification ledger](replacement-verification.md) records completed checks and
their source identity. Actual Snowflake and
normal packaged acceptance remain open. This extends the
[declared key inspector](snowflake-sdk-constraints-design.md) with a separate
read-only **View key column mappings** action. The existing declaration inventory
must remain available if a mapping request fails.

The required workflow is to select a retained relation, inspect ordered primary,
unique and foreign-key columns, retain the local/referenced pairing of composite
foreign keys, distinguish unavailable mappings from absent declarations, recover
from a denied request, and discard stale results after refresh or context changes.
Referenced tables are displayed with their database and schema. Local selection
of retained referenced tables is described below. Cross-database navigation,
inbound references, dynamic-table mappings, CHECK constraints and
complete object inspection remain required work. Mappings do not enable grid
editing or establish safe row identity.

## Source contract

Reviewed 12 November 2026:

- Snowflake's [SQLAlchemy reflection documentation](https://docs.snowflake.com/en/developer-guide/sqlalchemy/sqlalchemy-reflection)
  supports per-table primary, unique and foreign-key reflection through SHOW
  PRIMARY KEYS, SHOW UNIQUE KEYS and SHOW IMPORTED KEYS. A schema-wide fallback
  would introduce a different scope and potential SHOW truncation; it is excluded.
- [SHOW PRIMARY KEYS](https://docs.snowflake.com/en/sql-reference/sql/show-primary-keys)
  returns one row per key column with a sequence and table/constraint identity.
  The standalone UNIQUE/IMPORTED reference pages were unavailable during research.
  Their required field names are supported by Snowflake's
  [owned SQLAlchemy implementation](https://github.com/snowflakedb/snowflake-sqlalchemy/blob/28f731f03ceb7b09c4c1b89b3c2ddf88237780a3/src/snowflake/sqlalchemy/snowdialect.py#L795-L949)
  and [JDBC fixture](https://github.com/snowflakedb/snowflake-jdbc/blob/c7057fd89ca0706809b845c913ab4f3319c33ddb/src/test/resources/wiremock/mappings/databasemetadata/metadata_show_results.json),
  rather than an assumed complete public raw-output schema.
- The [pipe operator](https://docs.snowflake.com/en/sql-reference/operators-flow)
  binds a following SELECT to the preceding command's result, returns the final
  result, requires quoted lowercase SHOW field names, and does not support binds.
  Pipes execute as anonymous scripting blocks. Only closed generated SHOW/SELECT
  pairs are admitted here; arbitrary scripting or semicolon batches are not.
- [Dynamic-table keys](https://docs.snowflake.com/en/user-guide/dynamic-tables/input-data-optimization)
  can be derived from query structure rather than declared constraints. Dynamic
  tables are explicitly unsupported by this mapping action. Unknown traits keep
  the existing unavailable-target rejection; SHOW rows must never create invented
  declarations.
- Snowflake's [JDBC tests](https://github.com/snowflakedb/snowflake-jdbc/blob/c7057fd89ca0706809b845c913ab4f3319c33ddb/src/test/java/net/snowflake/client/internal/jdbc/DatabaseMetadataWiremockLatestIT.java)
  account for eventually consistent key metadata after DDL. A missing mapping
  therefore does not establish that a retained declaration has no columns.
- [Hybrid-table constraint metadata](https://docs.snowflake.com/en/sql-reference/sql/create-hybrid-table)
  does not establish whether validation of existing data failed. ENFORCED plus
  matching columns remains descriptive metadata, not editing authority.

## Closed request and bounded reads

`inspect_relation_constraint_mappings` accepts session ID and a request containing
only retained catalog revision and opaque target ID. Unknown fields are rejected.
The backend resolves the exact target, re-reads its declared constraints, and
performs all three per-table SHOW reads under the existing serialized relation
lane. Fully quoted identifiers come from the retained target, not caller SQL.
Available dynamic targets return unsupported entries without SHOW reads. Targets
with unknown traits remain unavailable and are rejected before account reads.

Each SHOW is followed by a fixed SELECT over `$1` projecting explicit fields with
`TO_VARCHAR`. Primary/unique reads retain local database, schema, table,
constraint, column and sequence. Imported-key reads retain the local identity,
foreign-key name/column, sequence, referenced database/schema/table/column and
referenced key name. Casting defines the text wire contract without assuming raw
SHOW timestamp/numeric types. There is no LAST_QUERY_ID, RESULT_SCAN or fallback
to a broader inventory. All mapping SHOW requests have empty binds.

Each SHOW has a 1,001-row sentinel for a 1,000-row product limit. Truncation,
overflow, incomplete rows or bytes, and a final serialized response over 1 MiB
fail the action; no partial inventory is published. These bounds do not prove
Snowflake's underlying output or role visibility is exhaustive. Actual SDK pipe
compatibility and account output shapes remain external qualification gates.

Pre/post session observations and relation checks detect observed drift. Catalog
revision/target checks surround the lane wait and publication. These observations
cannot make external concurrent DDL atomic or establish immutable object identity.
The frozen table-lifecycle account plan is unchanged and grants no authority for
these new reads.

## Mapping invariants and UI states

Only internally observed declarations are eligible for a mapping. Every SHOW
row must match the exact local table. An unexpected constraint, kind conflict,
malformed row, duplicate/gapped ordinal, duplicate local column or inconsistent
reference fails the action. Ordinals are parsed as positive integers, sorted
numerically and required to cover `1..N`. Foreign-key local/referenced columns
stay paired from the same row; independent sorting would corrupt composite keys.
Visible REFERENTIAL_CONSTRAINTS identity must match SHOW's referenced constraint
catalog/schema/name. A coherent SHOW mapping can be available even when the
separate reference declaration is hidden; those visibility observations differ.

The response retains the catalog revision, target ID, database and table. Each
declared constraint has a `status`:

- `available`: nonempty ordered columns; foreign keys additionally have a full
  referenced table and a referenced column for every local column.
- `notVisible`: a complete well-formed SHOW returned no mapping for that declaration.
  Columns are empty and the reference is null; the UI must not label it no key.
- `unsupported`: the table family is explicitly gated. No inferred columns or
  reference are returned.

The UI distinguishes not fetched, empty declarations, unavailable/unsupported
mappings and action failure. Selection, filter, refresh, owner/session changes,
context changes and transaction/metadata holds invalidate stored/outstanding
results. Late responses and mismatched returned identities cannot publish.

## Referenced-table selection

Implemented locally; source/browser/hidden native checks and independent review
pass. Normal packaging remains pending. **Select referenced table** is
a local navigation action for a complete, available foreign-key mapping. Resolve
the full database/schema/table identity against the entire retained catalog,
including entries hidden by the current text filter. Require exactly one match
before checking its availability; an unavailable duplicate must not be filtered
away to choose an available sibling. Preserve exact case and quoting. Never fall
back to a table name alone.

Selection clears the filter, previous metadata/error panels and pagination,
selects the destination's existing opaque target ID, and focuses the Relation
selector. It does not load metadata, preview data or issue any RPC. Subsequent
inspection, definitions, key reads and previews remain explicit actions using
their existing retained-target admission and context checks. No refresh,
connection, USE or session-context switch is performed by navigation.

Activation must recheck the live owner/session/context and a render-captured
invalidation generation. A transaction or metadata hold that starts and ends in
one React batch still revokes the old action, even if the context value returns
to its previous value before rendering. Refresh, owner replacement, unmount and
new work likewise prevent a captured old handler from selecting a target.

Explain missing, ambiguous, unavailable and external-database destinations.
The current catalog is scoped to the profile database and covers multiple
schemas. Cross-database navigation remains required and needs a separately
admitted database-scoped retained catalog; existing schema-switch authority is
not a substitute. Names remain observations, not immutable object identity or
edit authority. Inbound-reference discovery remains required.

The read-only design was reviewed on clean 54f59b6 with Tier 2 generation
2026-11-12T07:01:27Z, coverage for ten evidence paths and direct source reads.
Implementation-specific review and test evidence belong in the
[verification ledger](replacement-verification.md). Actual role visibility,
external DDL and normal packaged macOS keyboard/focus behavior remain acceptance
gates; this contract authorizes no account requests.

## Required verification

Pure tests cover closed SQL/quoting, composite primary/unique/foreign keys,
numeric order, paired references, visibility differences, malformed identities,
duplicate/gapped ordinals, unknown groups and byte/row bounds. SDK transport
fixtures cover shared-session admission, denial, context/catalog drift, held
reads and disconnects. UI and browser tests exercise independent actions,
quoted names, keyboard access, errors/retry and stale-response invalidation.
Typed IPC, native command/ACL registration, full validation and independent
review are required before committing the increment.

Synthetic fixtures establish local behavior only. Real role visibility,
eventual consistency, pipe execution, denial/interruption and normal macOS usage
need a fresh reviewed account plan and owner evaluation. No live account request
is authorized by this document.
