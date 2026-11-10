# Snowflake SDK session observation

**Status: locally qualified; account and owner acceptance open.** This
increment observes runtime state on the existing SDK connection. It does not
switch schemas, enable transaction controls, qualify Snowflake account behavior,
or complete the [DataGrip replacement scope](snowflake-datagrip-replacement.md).
The earlier approval for two fixed SELECT statements does not cover these
additional observer or schema-list statements. No account SQL is needed for the
offline checks below.

For the design rationale and Snowflake references, see the
[session observation and schema-switching proposal](snowflake-sdk-session-design.md),
[SDK integration guide](snowflake-sdk-integration.md), and
[relation browsing guide](snowflake-sdk-browsing-design.md). This document covers
the observer and schema options. The dependent typed schema-switch adapter, IPC
and selector are implemented with separate
[application qualification](replacement-verification.md#sdk-guarded-schema-selection-10-november-2026).

## Read contract

The observer reuses the current SDK connection and its serialized console lane.
It does not start another session, change a setting, begin or recover a
transaction, or replay a statement. A driver lane wait is bounded to 1.5 seconds;
the metadata operation has a 65-second overall deadline.

Each observation sample makes five fixed reads on that lane:

1. The existing context query for principal, role, database, schema, warehouse,
   session ID, search path, and secondary-role information.
2. The quoted-identifier policy query.
3. `SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION`.
4. `SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION`.
5. `SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID`.

Two samples are compared before state is published. `session_state` therefore
uses ten fixed statement executions. The schema-options request places one
bounded current-database `INFORMATION_SCHEMA.SCHEMATA` read between the samples,
for eleven executions total. It returns at most 1,000 visible schemas and a
fresh opaque options revision. These counts exclude SDK connection startup and
do not estimate Snowflake billing or history behavior.

Studio automatically requests a state observation after each qualifying
serialized SQL action while the run was not cancelled and the session/driver
remain open. This can add up to ten executions after an individual SQL action,
including some actions ending in a database error; an early observation failure
can stop before all ten. These reads are additional to the submitted user SQL
and explicit editor/context refreshes. Account planning and authorization must
cover this cadence, not just manual refresh requests. The old two-query helper
canary bypasses this Studio path and does not authorize it.

Every observation result must be complete, untruncated, exactly one row, and at
most 64 KiB including metadata. Required headers must be unique and Text typed;
parameter keys must exactly match the requested name, and the current `value`
must be `true` or `false` (the `default` is not used). The transaction ID must
be SQL NULL or nonempty bounded text. It stays opaque; it is never parsed as a
number. The samples must agree on the full observed context, policy, settings,
and transaction ID.

The published fields have distinct meanings:

- `transactionMode` reports observed AUTOCOMMIT state.
- `transactionState` reports observed activity: SQL NULL means `idle`; a
  nonnull transaction ID means `inTransaction`.
- `transactionObservation` retains the nullable opaque ID, the
  `TRANSACTION_ABORT_ON_ERROR` policy, and `usability: "unknown"`. An open ID
  does not prove later statements will succeed, and the abort-on-error setting
  does not prove the transaction has failed.
- `role` remains nullable. A SQL NULL role is not replaced with the profile's
  configured role.

These semantics follow Snowflake's
[CURRENT_TRANSACTION](https://docs.snowflake.com/en/sql-reference/functions/current_transaction),
[SHOW PARAMETERS](https://docs.snowflake.com/en/sql-reference/sql/show-parameters),
[CURRENT_ROLE](https://docs.snowflake.com/en/sql-reference/functions/current_role),
[TRANSACTION_ABORT_ON_ERROR](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error),
and [transaction behavior](https://docs.snowflake.com/en/sql-reference/transactions).

## Failure, publication, and controls

An inconsistent or failed state-only observation publishes `transactionMode:
"unknown"`, `transactionState: "unknown"`, `role: null`, and an unavailable
observation. It never falls back to profile values or an assumed Idle/AutoCommit
state. Session closure or a changed session generation remains a connection
error, not a successful Unknown observation. In the combined context-options
request, observation or schema-list failure is an error; an incomplete schema
list is never returned as a successful empty list.

The observer has its own outcome. It does not rewrite the result or database
error from a separately acknowledged SQL statement. A fresh changed or
unavailable SDK observation revokes backend completion data, relation targets,
and prepared plans. An identical stable observation preserves them. The
options-generation token is revoked before lane waiting, and late responses
cannot restore an older generation. On the frontend, an SDK refresh first clears
old choices and shows Unknown; errors keep Unknown and do not revive stale
context. Closing a console with an open or unknown transaction requires
confirmation. The message asks users to verify remote state; closing locally
does not promise immediate server rollback.

The schema options are opaque, bounded choices from the current database. Their
visibility is privilege filtered; a missing schema can mean it was not listed
or is no longer available. The current schema may be displayed as unavailable,
but an omitted schema is not inserted into the selectable choices. The SDK
schema selector requires current opaque revision authority and observed idle
activity; revoked authority stays visibly disabled until explicit refresh.
The separate typed command does not widen public SQL admission. `USE`, `ALTER SESSION`,
writes, DDL, BEGIN, COMMIT, and ROLLBACK remain blocked by the read-only SQL
policy; `switch_session_context` does not enable SDK schema switching.

## Offline regression commands

Use the repository-pinned Node executable by absolute path; do not substitute a
private profile or credentials. These fixtures use synthetic SDK responses and
local test infrastructure only.

```sh
NODE_BIN="/absolute/path/to/repository-pinned-node-v24.22.2/bin/node"
export PATH="${NODE_BIN%/node}:$PATH"

cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib snowflake_session
JOINERY_TEST_NODE="$NODE_BIN" cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib session_tests
JOINERY_TEST_NODE="$NODE_BIN" cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib sdk_observation_drift_revokes_plans_and_completion_but_stable_state_preserves_them
pnpm test -- src/state/studio.test.ts
pnpm test:e2e -- tests/e2e/snowflake-sdk-session.spec.ts
```

The Rust helper fixture runs a child Node process with a synthetic SDK; it does
not import the installed Snowflake SDK or perform sign-in. The browser test
intercepts RPCs for the actual UI and likewise performs no SSO or Snowflake SQL.
These checks qualify local implementation behavior only. Actual account
observation shapes, history effects, transaction behavior, and schema
visibility remain unqualified and require separate approval and evidence.
