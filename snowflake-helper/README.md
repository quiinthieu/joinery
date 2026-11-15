# Snowflake helper

This private package provides the bounded Node helper for stateful Snowflake
connections. New application profiles select SDK external-browser SSO; existing
profiles preserve their OAuth/SQL API mode. Both routes remain read-only. Local
profile, resource and lifecycle integration passes; the approved two-query account
canary is helper-level evidence, not normal packaged application acceptance.
The complete replacement target still requires qualified account workflows,
writes, schema changes and transactions.

The package pins `snowflake-sdk` 3.4.1. Install its locked production dependencies
with a repository-local npm cache, lifecycle scripts disabled, and isolated npm
configuration. Run `pnpm test:snowflake:helper` from the repository root after
installation. `pnpm test:snowflake:process` exercises the Rust supervisor, read
adapter and real helper with an injected fake SDK, without importing the production SDK or
opening any network connection. These are separate qualifications. The
[runtime preparation guide](../docs/snowflake-runtime.md) covers staging the
actual pinned runtime and checking SDK import/startup without authenticating.

The production launcher must supply an absolute Node executable and helper entry
point, a fresh generation, and a parent-owned private `0700` state directory via
the fixed `JOINERY_SNOWFLAKE_STATE_DIR` environment field. Authentication and SQL
use framed stdin, never command arguments or environment fields. The helper
creates an exclusive `0600` client configuration owned by the current uid and
primary gid, isolates HOME/config/certificate caches, and disables platform
metadata probing before SDK import. It does not delete the parent-owned root.

The supervisor removes that root only after actually reaping the child. Cleanup
receipts distinguish logout acknowledgement, process exit and directory removal.
Runtime abandonment retains an uncertain root instead of deleting it before
confirmed exit. Application cleanup tracking and packaged resource resolution
are implemented and locally qualified. Normal packaged account acceptance,
installation, Developer ID signing and notarization remain separate gates.
Neither process termination nor a logout
acknowledgement proves remote cancellation or rollback.

The helper accepts caller-classified reads only. Before SDK import, production
main verifies 13 reviewed transport-source files and installs process-local
HTTP(S) query admission. Each execute registers its exact returned request ID;
only one native query submission is admitted. A violation irreversibly closes
admission and retires the helper. This pins a bounded SDK/Axios/follow-redirects
path, not every dependency or arbitrary future transport.

Local tests distinguish pure/native guard checks, synthetic SDK Helper/process
tests, actual SDK loopback transport, and the unchanged unguarded retry baseline.
The guarded matrix passes 49 actual SDK scenarios plus one network-boundary
self-check, including verified TLS and CONNECT. A test-only disabled guard
reproduces two POSTs/two simulated effects after acknowledgement loss. No account
authentication runs in these fixtures. See the
[guard contract](../docs/snowflake-sdk-submit-guard-design.md) and
[verification ledger](../docs/replacement-verification.md).

A lost first response still leaves the server outcome uncertain. Local admission
does not prove remote deduplication, cancellation or rollback. Mutation, DDL and
transaction execution must remain gated until their retry and outcome contracts
are demonstrated. See [driver design](../docs/snowflake-driver-design.md),
[license evidence](../docs/snowflake-sdk-licenses.md) and
[replacement acceptance](../docs/replacement-acceptance.md).
