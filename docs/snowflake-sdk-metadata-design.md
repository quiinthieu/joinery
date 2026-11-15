# Snowflake SDK metadata and completion increment

This records the implemented completion slice and its remaining qualification,
not Snowflake account acceptance or a parity claim. The full requirements in
[snowflake-datagrip-replacement.md](snowflake-datagrip-replacement.md) remain open.
The initial investigation performed no authentication or account SQL. Subsequent
implementation and local tests are distinguished below; no metadata SQL has been
sent to the owner's account.

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

The first slice is implemented and locally qualified in the working branch.
Pure parsers pass 15 tests, the actual-helper adapter suite
passes 18 tests using a synthetic SDK, and nine actual-runtime Studio tests pass.
Four focused browser flows and the full 145-flow browser suite pass; guard
removal reproduces obsolete metadata in the new regression. Combined validation,
124 real-engine regressions, fresh SDK-bearing packaging and 19 hidden native
check categories pass; exact source/artifact identities are in the
[ledger](replacement-verification.md#sdk-completion-increment-10-november-2026-locally-qualified).
These establish local behavior only;
the earlier approved account canary did not include any metadata SQL.

Independent review reproduced missing global invalidation on a lazy column
failure. Errors now carry an optional `completionInvalidated: true` signal while
preserving their original code/message. Failed identity observations, observed
context drift and retired SDK sessions invalidate the complete catalog and all
column/navigation state. The frontend applies that signal only to the exact
session, revision and cache generation that issued the request. A late failure
cannot discard a replacement context. A definitive per-table permission denial
keeps other successful columns. Studio conservatively invalidates on any failed
identity observation, including a transient busy error; retrying requires an
explicit refresh. A busy column-read lane alone does not invalidate context.

Same-session metadata advances Snowflake statement history. Snowflake's
`LAST_QUERY_ID()` indexes all statements and `RESULT_SCAN()` defaults to the
previous statement, so metadata and health reads affect those relative references.
The existing read-only classifier blocks history-relative forms; actual-helper
regressions verify no dispatch for `LAST_QUERY_ID()` with default, negative or
positive indexes; `RESULT_SCAN()` with default, negative or positive indexes;
and `RESULT_SCAN(LAST_QUERY_ID())`. No classifier relaxation is
part of this slice. An explicit query-ID/history-token workflow and its full
Snowflake acceptance remain required rather than silently returning metadata
results under an owner's relative query reference.
[LAST_QUERY_ID contract](https://docs.snowflake.com/en/sql-reference/functions/last_query_id),
[RESULT_SCAN defaults](https://docs.snowflake.com/en/sql-reference/functions/result_scan).

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

The next state observer must distinguish open, idle and aborted transactions,
not map every SQL error to PostgreSQL-style failed state. Snowflake's
`TRANSACTION_ABORT_ON_ERROR` defaults to false; when true, a non-autocommit
transaction can remain aborted until explicit completion. `CURRENT_TRANSACTION()`
identifies an open transaction, but its short reference alone does not establish
a complete failed/idle-state protocol. Validate the observer against real
disposable transactions before enabling transaction controls. Read-only SELECT
metadata must not be conflated with the DML that starts a transaction when
AUTOCOMMIT is disabled.
[Abort-on-error parameter](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error),
[current transaction](https://docs.snowflake.com/en/sql-reference/functions/current_transaction),
[implicit transaction rules](https://docs.snowflake.com/en/sql-reference/transactions#autocommit).

## Budgets, privileges and refresh

Use explicit budgets, initially aligned with the existing bounded workflow:

| Operation            | Complete-result bound (preview remains planned)                             |
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

## Reproducible local acceptance

These commands use the repository's pinned Node/Rust toolchains, disposable
fixtures and synthetic SDK. Set `JOINERY_TEST_NODE` to the absolute pinned Node
binary used by the prepared runtime; this is a test-only seam, not an application
runtime override.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib snowflake_metadata
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --test snowflake_external
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib snowflake_connection_tests
pnpm exec playwright test tests/e2e/snowflake-sdk-completion.spec.ts tests/e2e/snowflake.spec.ts
pnpm validate
```

The Studio tests exercise one actual helper connection from connect through
cleanup: stable refresh retains its revision, hidden context changes replace it,
old revisions submit no SQL, column reads resolve fully quoted names, global
context errors remove stale state, and a definitive denial for one table keeps
another table available. The browser tests exercise editor suggestions, delayed
schema replies, invalidation and explicit recovery with synthetic SDK responses.
A separate disposable PostgreSQL flow fetches actual object metadata, injects a
marked schema failure, and holds a settled editor refresh's object reply to check
that obsolete navigation cannot reappear. These are local application
checks; they do not demonstrate the owner's Snowflake grants, session policies,
catalog size, latency or real-account metadata shapes.

For later account acceptance, first review an explicit bounded statement/object
plan and obtain authorization for its possible warehouse use. Use the prepared
runtime and a disposable application-data context, inspect actual relation and
column suggestions, compare their context to independently observed session
state, then disconnect and verify cleanup. No metadata plan is authorized by the
preceding two-query scalar canary, and no remote completion result is recorded
for this increment yet.

## Second slice: complete current-database read browsing

The reviewed [browsing/session proposal](snowflake-sdk-browsing-design.md)
expands this slice's target contracts, inspection sources, bounds and acceptance
gates. Its typed context switching depends on the separately proposed
[submission guard](snowflake-sdk-submit-guard-design.md); neither proposal is
implemented or account-qualified by this completion increment.

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
