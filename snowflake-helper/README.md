# Snowflake helper

This private package provides the bounded Node helper for the planned stateful
Snowflake connection. It is not yet connected to the application profile flow.
The current product continues to use the existing read-only OAuth/SQL API route.
The complete replacement target still requires external-browser SSO, writes,
schema changes and transactions; this infrastructure does not qualify them.

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
remain integration requirements. Neither process termination nor a logout
acknowledgement proves remote cancellation or rollback.

The helper accepts caller-classified reads only. Its wrapper does not resubmit
SQL, but the pinned SDK contains its own statement retry middleware. Local
loopback transport tests distinguish retransmission from simulated server
deduplication; actual Snowflake outcomes remain unqualified. Mutation, DDL and
transaction execution must remain gated until their retry and outcome contracts
are demonstrated. See [driver design](../docs/snowflake-driver-design.md),
[license evidence](../docs/snowflake-sdk-licenses.md) and
[replacement acceptance](../docs/replacement-acceptance.md).
