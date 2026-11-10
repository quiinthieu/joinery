# Snowflake SDK metadata and completion increment

This is an implementation plan, not Snowflake account qualification or a parity
claim. The full requirements in
[snowflake-datagrip-replacement.md](snowflake-datagrip-replacement.md) remain open.
No authentication, account SQL, application changes or builds were performed for
this investigation.

## Smallest complete next workflow

Implement **live context → relation suggestions → lazy resolved column
suggestions → stale-context rejection**, using the existing externalbrowser
connection throughout. Start with relations in the observed current database.
Keep table browsing, editing, transactions, object explorer, DDL editing, schema
diff and diagram capabilities disabled. Completion has no separate capability
flag today and already runs for SQL engines independently of `tableBrowsing`.

This provides a complete editor workflow without presenting incomplete table
inspection or starting an OAuth metadata session. Follow it with the separate
catalog/schema/read-preview workflow described below. Neither slice reduces the
remaining cross-database, write, transaction or object-inventory requirements.

## Existing seams to preserve

- `src-tauri/src/drivers/snowflake_external.rs`: `run` validates public read SQL
  and bindings, takes the driver's `lane`, and calls the bounded helper stream.
  All metadata must use this same lane and helper connection. `catalog`, `schema`
  and `browse` currently return `Unsupported`.
- `src-tauri/src/drivers/snowflake/catalog.rs`: reusable identifier quoting,
  catalog/column result validation and preview policy exist, but transport is the
  legacy SQL API's `metadata_lane` plus REST partitions.
- `src-tauri/src/drivers/snowflake/completion.rs`: reusable conservative
  unqualified-name policy exists. Its context assertions compare against fixed
  SQL API request configuration and must not become SDK context assertions.
- `src-tauri/src/execution.rs`: `Studio::completion_catalog` already serializes
  through the session lane, clears the previous snapshot before refreshing,
  compares identity and ordered relation names/kinds, and issues a revision.
  `editor_schema` checks that revision, re-reads identity and verifies visibility
  before fetching columns. `run_plan` and explicit context switching clear cached
  completion. `Studio::catalog` does not acquire the session lane, so the driver
  lane remains essential.
- `src/features/studio/SqlEditor.tsx`, `completion-session.ts` and
  `src/state/studio.ts`: lazy schema loading, bounded caches and stale-response
  rejection already exist. Extend their context comparison rather than adding
  another completion cache or refresh lifecycle.

Lock order remains Studio session lane → SDK driver lane. Private helpers called
inside a held driver lane must not acquire it recursively. Hold one lane across
each multi-query metadata operation, including its context validation. Do not
copy the legacy independent metadata lane onto the stateful connection.

## First implementation slice

1. Add a small transport-neutral `drivers/snowflake_metadata.rs` containing
   identifier validation/quoting, fixed SQL builders, typed result parsers and
   conservative completion-name selection. Keep legacy authentication, REST
   submission and partition handling in the existing legacy modules. Avoid an
   executor framework for two adapters; private adapter methods can invoke the
   shared pure helpers.
2. Add a private SDK metadata reader and collector. Reuse stream cancellation,
   terminal outcome and cleanup handling, but pass operation-specific row/byte
   budgets instead of the public query's million-row/8-GiB limits. Only fixed
   trusted metadata SQL and validated quoted identifiers reach this path. Do not
   loosen the global read classifier or accept caller SQL merely labelled Read.
   Initial `CURRENT_VERSION()` already demonstrates a private internal read.
3. Read live effective context on that connection: user, primary role, database,
   schema, warehouse, server session ID, resolved ordered `CURRENT_SCHEMAS()`,
   secondary-role state and quoted-identifier policy. Do not return the initial
   verified identity as current state. Validate nulls according to each function
   rather than treating an unset warehouse/database as an invented value.
4. Add an optional context fingerprint to `CompletionIdentity` and
   `SessionRuntimeState`, defaulted for existing drivers. Its typed canonical
   inputs include all fields above; its comparison must include schema even when
   an explicit search path does not change. Extend frontend context invalidation
   to compare it. Keep display role separate from machine identity. Recheck
   context before publishing a collected snapshot; a changed server session
   generation closes/rejects the operation and never triggers reconnect/replay.
5. Query current-database relations with explicit selected columns, schema
   predicates, deterministic ordering and an overflow sentinel. Produce only a
   complete bounded snapshot. Keep the existing first-resolved-current-database
   schema rule for unqualified names; other schemas remain qualified. Do not
   derive absence of a shadowing object from privilege-filtered metadata.
6. Implement `completion_schema` using the same-session, fully quoted resolved
   relation and a zero-row read. It proves the columns actually resolved for this
   session and whether SELECT is permitted. Keep `inspection_schema` unsupported
   until nullability, defaults, constraints and completeness are obtained;
   current SDK wire columns contain only name/type. Do not present its fabricated
   `nullable=true` or missing keys as authoritative inspection metadata.

`CURRENT_SECONDARY_ROLES()` returns structured JSON with active/requested roles;
capture it as context, not an unparsed display label. The initial helper
`context` operation asserts configured role/database/warehouse and optionally
executes `USE SCHEMA`; it is not a generic live observer after context changes.
Use private fixed reads or deliberately revise that protocol operation instead.
[Secondary-role function](https://docs.snowflake.com/en/sql-reference/functions/current_secondary_roles),
[resolved search path](https://docs.snowflake.com/en/sql-reference/functions/current_schemas).

Retain the existing conservative rejection of
`QUOTED_IDENTIFIERS_IGNORE_CASE=TRUE` until the editor's identifier model supports
it. Snowflake documents different stored/resolved spelling under that setting;
silently using case-preserving rules is incorrect.
[Identifier parameter](https://docs.snowflake.com/en/sql-reference/parameters#quoted-identifiers-ignore-case).

For `session_state`, observe actual autocommit/transaction state before claiming
AutoCommit/Idle. Observing state does not enable transaction actions. Metadata
must not issue DDL, change AUTOCOMMIT, commit, roll back, switch roles or change
context as a side effect. In particular, setting AUTOCOMMIT can implicitly commit
an active transaction even if its value stays unchanged. Test metadata with an
active transaction when the transaction increment is enabled.
[Transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

## Budgets, privileges and refresh

Use explicit budgets, initially aligned with the existing bounded workflow:

| Operation            | Proposed complete-result bound                                              |
| -------------------- | --------------------------------------------------------------------------- |
| Context              | One row, 64 KiB; at most 64 search-path entries of 1024 bytes each          |
| Completion relations | 10,000 relations; 2 MiB serialized snapshot; bounded intermediate collector |
| Lazy columns         | 1 MiB serialized schema; effective helper ceiling of 1,000 columns          |
| Later read preview   | 500 retained rows, 501st overflow sentinel; 2 MiB                           |

Studio's 4,096-column check is only an outer ceiling: the helper currently rejects
more than 1,000 columns. Raising that limit requires its own protocol/memory tests.
Overflow, malformed shape, partition/result mismatch or helper truncation fails a
completion snapshot with `ResultLimitReached`; it must not become an empty or
apparently complete catalog. Preview truncation may be explicit. Helper row
budgets currently discard excess incoming rows while draining, so they bound
retention rather than remote query work. Include selective predicates and SQL
limits as well as a deadline/cancellation path.

Information Schema is read-only, privilege-filtered and not a consistent snapshot
against concurrent DDL. Broad queries can fail for insufficient selectivity;
LIMIT alone is not the documented selectivity solution. It requires a warehouse.
Use schema/object scope and expose a clear engine error rather than automatically
switching warehouses or falling back to delayed Account Usage metadata.
[Information Schema guidance](https://docs.snowflake.com/en/sql-reference/info-schema).

Distinguish an empty visible set from a failed metadata query and a denied column
read. Visibility does not prove SELECT permission. SHOW and Information Schema
also differ for MANAGE GRANTS; do not merge them into an authoritative inventory
without an explicit policy. The latter difference is documented for TABLES.
[TABLES visibility](https://docs.snowflake.com/en/sql-reference/info-schema/tables).

Stable ordering makes revisions deterministic. On denied/failed refresh, the
existing previous-snapshot removal prevents serving an old authorization context.
Keep stale frontend responses ignored. Do not claim the revision protects writes
or identifies an original object generation: external DDL/grant changes can race
these metadata reads.

## Second slice: complete current-database read browsing

Implement `catalog`, complete `schema`, bounded `browse`, live `session_state`,
`list_contexts` and verified explicit schema switching. Enable `tableBrowsing`
only when this whole sidebar → inspect → preview → refresh workflow works. Keep
editing/filtering/continuation capability claims separate. Read COLUMNS metadata
for actual nullability/defaults/type precision and validate it against resolved
columns; keys require separate treatment and must not imply standard-table
uniqueness enforcement or editability.
[COLUMNS fields](https://docs.snowflake.com/en/sql-reference/info-schema/columns).

Explicit switching is a typed validated operation, followed by a live readback
before acknowledging success; it clears completion/plans. A lost acknowledgement
must not report the old context as current. Metadata never switches schema merely
to inspect an object.

`TableRef` currently contains schema/name only. Before permitting database
switching or cross-database browse targets, add a database-qualified target or
equivalent snapshot-bound target token. Never apply an old table reference to the
new current database. Snowflake resolves omitted databases through current
context, and temporary tables can shadow non-temporary tables with the same
fully qualified spelling in one session. Duplicate catalog rows must not be
silently collapsed into one actionable target.
[Name resolution](https://docs.snowflake.com/en/sql-reference/name-resolution),
[temporary-table precedence](https://docs.snowflake.com/en/user-guide/tables-temp-transient).

The existing Vec catalog DTO cannot communicate paged partial results. For larger
inventories add a scoped page DTO with cursor, hasMore and explicit truncation;
keep completion's bounded complete-snapshot contract. SHOW OBJECTS has documented
scoping/cursor syntax and a 10,000-record limit, does not require a warehouse, and
lists tables/views rather than every database object. It is a separate potential
metadata strategy, not a transparent fallback with identical semantics.
[SHOW OBJECTS](https://docs.snowflake.com/en/sql-reference/sql/show-objects).

## Required inventory after these slices

The replacement matrix requires every applicable object type, DDL, keys,
dependencies, denied-role behavior and external refresh. Its exact inventory is
still open; this plan does not declare that inventory complete. Existing
`DatabaseObjectKind` expresses tables/partitions/views/materialized views,
functions/procedures/sequences/triggers/indexes. It cannot faithfully represent
all Snowflake candidates such as stages, file formats, pipes, streams, tasks,
policies, tags, roles/database roles, warehouses, types and semantic views.

Even relation classification needs more than TABLE_TYPE: TABLES exposes separate
temporary, Iceberg, dynamic and hybrid flags. Preserve those distinctions in
typed metadata rather than flattening everything to ordinary table/view.
[Relation fields](https://docs.snowflake.com/en/sql-reference/info-schema/tables).

Add object-kind-specific enumeration/details with explicit permission and
unavailable states, then DDL/dependency navigation. GET_DDL supports many of these
kinds but needs argument signatures for overloaded routines, restricts secure
objects, and can omit/redact protected information. Returned DDL is not always
the original source; it is not proof of complete recreate capability.
`Studio::database_objects` currently truncates its Vec to 10,000 silently and
needs an explicit completeness contract before broad explorer claims.
[GET_DDL restrictions](https://docs.snowflake.com/en/sql-reference/functions/get_ddl).

## Tests and evidence needed

- Pure helper tests: quoted/dotted/mixed-case identifiers, invalid names and SQL
  injection strings; exact query scope/order/sentinels; typed context and metadata
  shape/null validation; object flags and duplicate-name handling.
- Real helper plus FakeSDK protocol tests: every metadata read uses the existing
  connection; concurrent SQL/metadata never overlaps; cancellation/backpressure,
  overflow and malformed metadata; no hidden USE/SET/COMMIT/reconnect; cleanup on
  timeout/close. These establish local protocol behavior, not Snowflake semantics.
- Studio regressions: identical refresh retains revision; role/schema/database/
  warehouse/secondary-role/policy changes invalidate it; stale lazy requests and
  old-session responses fail; denied refresh removes old suggestions; busy
  execution is not interleaved; selection survives only its matching revision.
- Browser workflow: open SDK session, load relation/qualified suggestions, request
  lazy columns, switch context and reject stale suggestions; sidebar remains
  disabled until the second slice is complete. No OAuth request is introduced.
- Later explicitly authorized Snowflake account qualification: quoted names,
  visible-but-not-selectable objects, denied roles, secondary roles, unset or
  unavailable warehouse, large metadata, temporary/permanent shadowing, same
  session continuity, external DDL/grants and metadata during open transactions.
  Include all required object kinds and cross-database targets when implemented.

Suggested ownership: one worker may own the new pure helper module and tests;
root/shared ownership covers registration, Driver overrides, domain/IPC type
regeneration, Studio revision behavior and frontend invalidation. Changes to
helper limits/context protocol require the helper owner and updated runtime
source pins. Preserve legacy OAuth profile routing and its request semantics.

Investigation used Tier 2 graph search/trace/snippets plus direct source reads.
Project generation `2026-11-10T05:16:00Z` was ready; coverage for the cited source
and design paths reported metadata matches and no recorded gaps. This is a
best-effort index signal, not proof of completeness. No application/test source
was edited and no build or account qualification was run.
