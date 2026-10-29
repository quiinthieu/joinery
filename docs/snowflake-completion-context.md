# Snowflake SQL API completion context

Feature E02-SF extends the existing Rust completion snapshot and CodeMirror
worker. It is a bounded local continuation of E02/I04, not full Snowflake
qualification. The private hub remains the shipping-status record.

## Workflow

The SQL editor loads a server-verified request context, displays database,
effective user/role and resolved search path, and requests revision-bound
columns. Loading, failure and Refresh use the same UI and cache invalidation
as PostgreSQL/MySQL. Failed context reads withhold stale suggestions.

Every SQL API submission already supplies the configured database, schema,
role and warehouse. Context reads use those same request fields and the existing
authentication, expiry, cancellation and disconnect machinery. This feature
does not model the API as a persistent socket session or enable USE, ALTER
SESSION, transactions, temporary objects, writes, reconnect or SQL replay.

Two fixed metadata statements read CURRENT_USER/ROLE/DATABASE/SCHEMA/WAREHOUSE,
CURRENT_SCHEMAS and the effective QUOTED_IDENTIFIERS_IGNORE_CASE setting.
Database/schema/role/warehouse must match the configured request. Invalid or
missing metadata is an error. Case-insensitive quoted identifiers are explicitly
unsupported by the current editor and produce a visible error instead of using
the wrong identifier policy.

Catalog and columns remain limited to the configured database and existing
read-only adapter. Qualified database.schema.table and schema.table names remain
available. Unqualified names bind only to known tables in the first resolved
search-path schema when its database/schema identity is an uppercase simple
name in the configured database. Later-schema fallback is withheld: metadata
visibility cannot establish that an inaccessible earlier object is absent.
External-database and quoted complex search-path entries retain qualified lookup
within the configured database; they do not trigger cross-database discovery.

## Ownership and limits

The existing Driver completion_catalog receives the already-read identity from
Studio. This avoids an extra context request or a second adapter cache.
PostgreSQL/MySQL retain their same-session metadata implementations. Typed IPC
is unchanged; no second executor or global state is added.

Each context statement is limited to one row and 64 KiB using the existing
bounded metadata reader. Search paths allow at most 64 entries of 1,024 bytes.
Existing snapshot limits (10,000 relations / 2 MiB, 4,096 columns / 1 MiB),
frontend concurrency/cache budgets and operation deadlines remain in force.
Column discovery reuses the existing quoted, zero-row schema query and its
database-enforced SELECT authorization; arbitrary SQL results remain read-only.
External grants/defaults/DDL require refresh or subsequent app execution.

## Acceptance and evidence

- Synthetic Rust HTTP tests verify request fields on every metadata submission,
  effective identity, first-schema binding, qualified fallback, malformed
  responses, mismatched roles, unsupported quoted policy and byte limits.
- The browser fixture verifies the context display, failure/refresh recovery,
  revision-bound lazy columns and existing qualified/alias suggestions.
- Existing PostgreSQL/MySQL completion, cancellation, editing and recovery
  workflows remain regression gates.

Source: drivers/snowflake/completion.rs, catalog.rs context_rows,
catalog_tests.rs, Driver/Studio completion_catalog, SqlEditor and the Snowflake
browser fixture. Final command outcomes are in IMPLEMENTATION_STATUS.md.
Graph coverage is best-effort; source reads supplement the indexed call paths.
No live Snowflake account, SSO or production query is part of this batch.
HTTP fixtures do not qualify live grants, inherited session defaults or latency.

## Official references

- [SQL API request fields and supported parameters](https://docs.snowflake.com/en/developer-guide/sql-api/reference)
- [SQL API context limitations](https://docs.snowflake.com/en/developer-guide/sql-api/intro)
- [Object name resolution](https://docs.snowflake.com/en/sql-reference/name-resolution)
- [CURRENT_SCHEMAS](https://docs.snowflake.com/en/sql-reference/functions/current_schemas)
- [Quoted identifier policy](https://docs.snowflake.com/en/sql-reference/parameters#quoted-identifiers-ignore-case)
