# Snowflake declared key inspection

## Scope and acceptance

This increment is implemented locally; source-specific checks and open gates are
recorded in the [verification ledger](replacement-verification.md). Account
acceptance remains open. It adds a separate read-only **View key constraints**
action to the SDK relation browser, independent of column inspection and generated
definitions.
The required user workflow is: select a current visible relation, request its
declared keys, inspect names, types, enforcement and foreign-key reference rules,
recover from a denied request and refresh safely after context or catalog changes.

The separate [key column mapping increment](snowflake-sdk-key-mappings-design.md)
tracks ordered local/referenced columns without changing this declaration view.
The full replacement still requires verification of key column mappings on the
actual account, inbound and outbound table navigation, CHECK constraints,
complete object inspection and
database-qualified editing/concurrency behavior. Column nullability remains in
column inspection; this view is not a complete constraint inventory. Nothing in
this change enables writes or establishes edit eligibility.

## Official metadata contract

Reviewed 12 November 2026:

- [TABLE_CONSTRAINTS](https://docs.snowflake.com/en/sql-reference/info-schema/table_constraints)
  reports primary, unique and foreign-key declarations, enforcement, deferrability,
  RELY and comments. It does not enumerate CHECK or NOT NULL constraints.
- [REFERENTIAL_CONSTRAINTS](https://docs.snowflake.com/en/sql-reference/info-schema/referential_constraints)
  reports a foreign key's referenced unique-constraint identity and match/update/
  delete rules. That identity is a constraint name, not a referenced table or
  column mapping. Both views are filtered by the session's role visibility.
- [CHECK_CONSTRAINTS](https://docs.snowflake.com/en/sql-reference/info-schema/check_constraints)
  is a distinct view and remains required work.

An empty successful response means no visible declared primary/unique/foreign
keys were returned. It is distinct from a request that was not made or failed.
Declared metadata alone does not establish uniqueness of existing values or safe
row identity. The UI displays Snowflake's reported ENFORCED value rather than
inferring enforcement from a key name.

## Implementation boundary

`inspect_relation_constraints` takes only session ID, retained catalog revision
and opaque target ID. The backend resolves its own exact database/schema/name,
serializes through the existing relation lane, revalidates admission after waiting,
and checks observed session and relation metadata before and after the read.
These checks detect observed drift; they cannot make concurrent external DDL
atomic or establish immutable object identity.

A fixed query selects explicit columns from the quoted database's
INFORMATION_SCHEMA.TABLE_CONSTRAINTS with an exact table filter. Values are bound;
caller SQL or caller-supplied identifiers are not accepted. A LEFT JOIN to
REFERENTIAL_CONSTRAINTS uses constraint catalog, schema and name. It preserves
visible foreign keys when the reference row is unavailable and reports that
status separately. Duplicate or ambiguous joined results must be rejected rather
than selecting an arbitrary referenced key. Constraint names can be reused across
tables and must not be treated as globally immutable IDs.

The typed response retains the request's target identity and key metadata.
`referenceStatus` distinguishes `notApplicable`, `available` and `notVisible`.
A present reference row with malformed or partial fields must not be converted
to `notVisible`. Complete row shape, identity, kinds, YES/NO flags, byte/row bounds
and duplicate results are checked before publishing. Truncated responses fail;
no partial key inventory is presented as complete.

The frontend validates the returned target/revision, clears stale data on
selection, filter, refresh, session ownership, context and metadata/transaction
holds, and discards late responses. A failed constraints request removes older
keys and leaves an explicit retry path. Other read-only views remain independent.

## Required verification

- Fixed SQL/binds and quoting; complete valid/empty/foreign-key results; hidden
  reference; malformed/null/duplicate/truncated/oversized results.
- SDK context/relation drift, permission denial, operation serialization and
  session/transaction holds; no arbitrary SQL or inferred write authority.
- UI request identity, empty versus error, retry, returned-target validation,
  transient invalidation and late responses; keyboard browser walkthrough.
- Typed IPC, desktop command/ACL registration and missing-session rejection.
- Full repository validation and independent review of the final source.

Synthetic SDK and browser fixtures establish local behavior only. Live Snowflake
visibility, result shapes, hybrid/standard enforcement and normal packaged usage
require a separately approved account plan. The expired frozen table-lifecycle
attempt does not exercise or authorize this new command. Completed commands and
results belong in the [verification ledger](replacement-verification.md).
