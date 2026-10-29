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
