# Architecture decisions

## 2026-10-29 — Read live completion metadata in the editor session

PostgreSQL completion context and columns use the actual editor connection under
its existing Rust execution lane. They do not use the separate login-configured
metadata client used by table browsing. Replicating SET ROLE/search_path on a
second client would miss transaction-local settings and temporary objects, and
could use different effective privileges.

A typed catalog snapshot carries an opaque revision and effective identity.
Column reads require the revision and revalidate identity and visibility. Every
execution invalidates the previous snapshot before SQL submission. The frontend
uses existing execution/tab state to refresh, clears pending old metadata, and
keeps its existing CodeMirror worker and bounded cache. No second global state
manager, SQL executor or persistence schema is introduced.

The PostgreSQL adapter supplies server-resolved unqualified table identities
using `pg_table_is_visible`. The frontend intersects suggestions with that set
instead of resolving only over permitted catalog entries: an inaccessible table
can still shadow an accessible later table. System and temporary schemas follow
the server's actual resolution order; fully qualified accessible names remain.

The tradeoff is explicit unavailability while a user query owns the lane or its
transaction is failed. Metadata callers may wait up to 1.5 seconds for each other;
active query/cancellation ownership is preserved. PostgreSQL is the first live
adapter. Other engines keep their configured completion paths until separately
implemented and qualified. External DDL/grant monitoring remains a later feature.

Unchanged concurrent catalog reads share a revision so React lifecycle overlap
cannot invalidate a current reader. App executions invalidate revisions even if
the eventual identity returns to its previous value. Qualified `ROWS FROM` keeps
the legacy grid foreign-key reader equivalent to PostgreSQL multi-array unnest.

## 2026-10-29 — Extend the same context lane to MySQL/MariaDB

The existing completion identity/revision contract now carries an optional server
relation-case policy. MySQL/MariaDB observe current database, effective account,
active roles and lower_case_table_names on their editor connection. PostgreSQL
retains its established rules; other adapters do not pretend to supply live data.

Live metadata uses text protocol after a reproduced stale DATABASE() result from
a cached prepared statement. The existing bound-parameter grid path is preserved;
both protocols share bounded row collection. SHOW FULL COLUMNS follows temporary
shadows and filters SELECT privileges, avoiding execution of user objects just to
infer columns. All completion metadata is read-only, independent of the proven
grid's edit metadata. No additional execution or state-management system is added.

The catalog cannot discover temporary-only names. Case modes 0/1 are fixture-tested
(mode 1 follows through PR #23); mode 2 still needs platform qualification. These
limits remain explicit rather than being presented as full DataGrip parity.

## 2026-10-29: Snowflake completion uses verified request context

Snowflake SQL API submissions carry explicit database/schema/role/warehouse
fields, while inherited search-path and quoted-identifier defaults can affect
name resolution. Reuse the existing completion snapshot and pass its identity
into Driver completion_catalog; do not introduce an adapter context cache.
PostgreSQL/MySQL continue reading their actual socket session.

Snowflake verifies fixed metadata statements under the same request fields.
Unqualified binding is restricted to known first-schema tables; later-schema
fallback is withheld because visible catalog entries cannot prove that an
inaccessible earlier table is absent. Qualified lookup remains available.
Unsupported quoted-name policy fails with a visible error. This trades broader
inference for explicit, bounded correctness; live-account qualification and
additional identifier policies remain separate work.
