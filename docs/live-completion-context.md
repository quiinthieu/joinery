# Live PostgreSQL completion context

Implemented on `feat/live-completion-context` from merged `06146bc` (preview.12).
Shipping status remains on the owner-private project hub. This batch is local
implementation; no new release is claimed here.

## Workflow and ownership

A connected PostgreSQL SQL editor displays the database, effective role and
ordered schema search path. On opening the editor, explicit refresh, catalog
refresh, or an execution transition in any console for that connection, it reads
`completion_catalog` from Rust. Pending old responses are discarded. The editor
clears old catalog/columns while loading or unavailable and offers an explicit
Refresh action. Changes made through the app invalidate previous context tokens.

Rust reads `current_database`, `current_user` and `current_schemas(true)` on the
actual PostgreSQL editor connection under its existing execution lane. Implicit
and temporary schemas participate in unqualified name resolution. PostgreSQL's
`pg_table_is_visible` supplies the exact unqualified relation set, including
system catalogs and inaccessible earlier-name shadows. Qualified accessible
relations remain available even when a denied relation shadows their short name.
An opaque revision identifies the effective snapshot; unchanged concurrent
readers share it, while execution invalidation or observed identity/catalog
changes produce a new revision. `completion_schema` requires that revision,
rechecks identity and table visibility, then reads visible columns on that same
connection/lane. Every execution invalidates the snapshot before submitting SQL.
Internal catalog functions are schema-qualified to `pg_catalog`; no user SQL is
replayed and no role/search-path mutation is used to synchronize another client.

The existing table browser/editing metadata and independent write transactions
retain their established ownership. Live completion metadata is read-only.
Connection/query credentials and Safe Mode authorization remain in Rust. No
persistent schema or second execution/state system is introduced.

## Bounds and failure states

- Up to 10,000 visible relations, with a 2 MiB serialized snapshot budget.
- Up to 128 effective search-path schemas; oversized resolution is declined.
- Column responses: at most 4,096 columns and 1 MiB serialized data.
- Existing frontend eight-request/cache and 100-suggestion limits remain.
- Active user execution returns a busy error. Concurrent metadata callers have
  a bounded 1.5-second lane wait, avoiding spurious failure from initial React
  lifecycle overlap. Driver queries retain PostgreSQL session timeout settings.
- Failed transactions, denied/missing objects, obsolete tokens and disconnected
  sessions return real errors. No configured fallback is presented as live data.

MySQL/MariaDB and Snowflake retain their existing configured completion behavior;
this live adapter currently supports PostgreSQL. Redis retains native command
completion. External DDL/grant changes require explicit refresh or a subsequent
app execution; there is no server-side metadata event subscription. Independent
console/session binding and JOIN generation remain subsequent phases.

## Source and tests

`domain.rs`: CompletionIdentity/CompletionCatalog; `execution.rs`: Session
snapshot ownership, execution invalidation and completion methods; `drivers/mod.rs`
and `drivers/postgres.rs`: live adapter and shared schema reader. Commands,
contract exporter, browser harness and native capability manifest expose the
same typed DTOs. `SqlEditor.tsx`, `completion-session.ts`, `sql-context.ts` and
`Workbench.tsx` bind context and metadata to frontend lifecycle revisions.

The graph service was unavailable during initial implementation (transport
closed). On resumption, the graph was available for symbol/snippet checks; final
coverage generation was `2026-10-29T01:35:26Z`. Coverage reported partial SQL
fixture parsing; direct source reads cover the fixture and final working-tree
changes. Graph coverage is best-effort and does not establish completeness itself.

PostgreSQL fixture tests exercise search-path priority, SET LOCAL and rollback,
role reduction and visibility, stale tokens, failed transactions, and metadata
reads during cancellation, concurrent readers, system names, inaccessible-name
shadows, column grants and legacy grid foreign keys. Unit tests check ordered
and server-resolved lookup and out-of-order old privilege results. The browser
accepts and executes an alternate-schema column,
then checks role/rollback transitions. Native smoke checks that new commands
reach Rust's disconnected-session guard through the actual capability boundary.
Final command outcomes are recorded in the batch evidence section of the feature
matrix and IMPLEMENTATION_STATUS.md.

## References

- [PostgreSQL system information functions](https://www.postgresql.org/docs/current/functions-info.html)
- [MySQL information functions](https://dev.mysql.com/doc/refman/8.4/en/information-functions.html) for the next adapter slice
