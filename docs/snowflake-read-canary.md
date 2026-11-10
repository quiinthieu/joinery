# Snowflake SDK read canary

This is a prepared, opt-in qualification runner. No real account authentication
or SQL execution was performed while implementing it. Synthetic tests establish
the runner's protocol checks and local cleanup behavior, not Snowflake account
behavior or application replacement readiness.

Run the default command to review its exact plan:

```sh
node scripts/snowflake-read-canary.mjs
```

The default validates the fixed local runtime and ignored connection-context file,
then prints the two SQL strings, bounds, hashes and interaction/billing warnings.
It does not spawn a process, open a browser, authenticate, contact the network,
submit SQL, allocate auth state or write evidence. It does read and hash local
runtime files. It prints a connection fingerprint instead of account/user/context
values. A successful plan is not permission to execute it.

Only after explicit user approval of this exact scope, browser interaction and
possible warehouse billing, the execution command is:

```sh
node scripts/snowflake-read-canary.mjs --run
```

Do not run that command merely to validate this implementation. Approval is a
human workflow requirement; the flag does not verify that approval was granted.
There are no CLI options for SQL, connection values, executable paths, environment,
runtime paths, retries or timeouts.

## Fixed connection and runtime

The only configuration input is the ignored repo-local
`.cache/owner-snowflake-context.json`. The exact object shape is:

```json
{
  "profile": "nonsecret profile label",
  "source": "nonsecret source description",
  "connection": {
    "account": "organization-account",
    "user": "CONFIGURED_LOGIN_NAME",
    "database": "DATABASE",
    "schema": "SCHEMA",
    "warehouse": "WAREHOUSE",
    "role": "ROLE",
    "authenticator": "externalbrowser"
  },
  "scope": "nonsecret prior context description"
}
```

Extra keys, secrets, non-string values, control characters, malformed account
names and non-uppercase simple SQL context identifiers are rejected. The account
is a bare Snowflake account identifier; the runner constructs the hostname with
the fixed `.snowflakecomputing.com` suffix. The `user` value is supplied to the
SDK as the authentication login. Snowflake distinguishes user `NAME` from
authentication `LOGIN_NAME`; they can differ, as documented in
[ALL_USER_NAMES usage notes](https://docs.snowflake.com/en/sql-reference/functions/all_user_names).
[CURRENT_USER](https://docs.snowflake.com/en/sql-reference/functions/current_user)
returns the current user's name. The runner checks that this principal is a
nonempty string of at most 256 bytes, without control characters, and remains
exactly the same across both queries. It does not independently verify the
binding between the returned principal and the configured login. The existing
file's `scope` text is provenance, not live-execution authorization.

Execution uses only `.cache/snowflake-runtime/macos-arm64/bin/node` and its
original `helper/main.mjs`, with the actual official Snowflake SDK 3.4.1. It reuses
the packaging validator's Node/source/lock/notice hashes and safe-tree checks.
There is no runtime fallback, download, installation or staging. The supported
execution host is macOS ARM64. A caller-writable development stage is not an
immutable security boundary against another process running as the same user.

## Exact remote scope

The runner issues one helper `connect`, then at most two helper `query` requests
on that same connection, followed by helper `close`. Both queries are hardcoded:

```sql
SELECT CURRENT_USER() AS CANARY_USER, CURRENT_ROLE() AS CANARY_ROLE, CURRENT_DATABASE() AS CANARY_DATABASE, CURRENT_SCHEMA() AS CANARY_SCHEMA, CURRENT_WAREHOUSE() AS CANARY_WAREHOUSE, CURRENT_SESSION() AS CANARY_SESSION
```

```sql
SELECT CURRENT_USER() AS CANARY_USER, CURRENT_SESSION() AS CANARY_SESSION, CAST(42 AS NUMBER(10,0)) AS CANARY_INTEGER, CAST('joinery-read-canary' AS VARCHAR) AS CANARY_TEXT, CAST(TRUE AS BOOLEAN) AS CANARY_BOOLEAN, CAST(NULL AS VARCHAR) AS CANARY_NULL
```

The first checks exact configured role/database/schema/warehouse values, a
bounded nonempty principal and a numeric session identifier. The second requires
the same principal and session identifier, exact column names/types, and the
scalar strings `42`, `joinery-read-canary`, `TRUE`, and SQL NULL. Numbers and
booleans are fetched as strings by the original helper. Each response must
contain exactly six columns and one row, with no truncation, at most 8192 result
bytes and one initial batch credit. Unexpected values/types, a changed principal,
an extra row, malformed frames or failed terminals fail qualification.

These statements contain no object references. The runner sends no `USE`,
`ALTER`, DML, DDL, transaction, catalog, table-read or warehouse-management
request. It does not use helper `context`, which can issue `USE SCHEMA`.
The original official SDK separately performs connection authentication and
session setup. Two is a bound on runner-issued helper query requests, not on
physical SQL/HTTP submissions or internal SDK work. The original SDK can
retransmit requests after a lost response; the runner neither adds retries nor
changes that behavior or proves server deduplication.

`EXTERNALBROWSER` can open the system browser and require the user's SSO/MFA
interaction. Browser URLs, tokens and credentials must never be shared or added
to evidence. Cancelling the runner does not close a browser window or prove that
an identity-provider action was reversed.

The runner does not inspect or change warehouse suspension/auto-resume settings.
Submitting SELECT may use or automatically resume the configured warehouse and
incur Snowflake charges, including its configured billing minimums or other
account costs. These synthetic expressions do not establish a zero-cost
guarantee. There is no server-side cost cap or statement-timeout change in this
scope. Do not execute before approval that explicitly includes possible warehouse
resume and billing. Local termination does not prove already submitted remote
work stopped, nor does it establish remote logout or rollback.

## Local deadlines, ownership and evidence

Hello has a two-second deadline, connect/authentication 120 seconds, each query
30 seconds, and close two seconds. SIGINT/SIGTERM cancels local waiting. After
success or failure the runner ends child input and waits two seconds for actual
child close; it escalates to SIGTERM with a two-second wait, then SIGKILL with a
three-second wait. A close acknowledgement alone cannot pass: the actual child
must close cleanly. SDK logout acknowledgement is recorded separately from local
process reap. These are local asynchronous deadlines, not hard wall-clock bounds
on synchronous filesystem hashing/removal or an unresponsive operating system.

The runner creates a private `0700` auth directory below the repo-local
`.cache/snowflake-read-canary/`. Only `JOINERY_SNOWFLAKE_STATE_DIR` is passed in the
child environment; inherited credential, proxy, Node preload and home overrides
are excluded. The unchanged helper owns its ephemeral SDK configuration and
isolates home/certificate caches there. The runner retains the directory identity
until the actual child `close` event, then checks device/inode before removal.
If reap or removal cannot be confirmed, the run fails and retains the remaining
state instead of claiming cleanup. No retry or later automatic deletion is
implemented. Do not remove retained auth state while its child may remain alive.

Only an executed run writes a new exclusive `0600` JSON evidence file beneath
`.cache/snowflake-read-canary/`. Evidence contains runtime/context fingerprints,
timestamps, query-request counts, fixed failure codes and boolean checks. It
omits account/context values, rows, session/query/request IDs, raw frames,
browser URLs, credentials, tokens and helper error messages. Child stderr is
drained and discarded. No new tracked file receives connection information.

## Offline verification

```sh
node --test scripts/snowflake-read-canary.test.mjs
```

Tests use synthetic framed Node children and injected runtime-validation seams.
They cover plan-only behavior, rejected inputs, the exact query sequence,
distinct login/principal acceptance, principal/session/type/value mismatches,
non-string context rejection, oversized results, sanitized failures,
cancellation, timeout escalation, actual child exit and private-state cleanup.
The seams are programmatic test interfaces; the CLI never accepts them.
These tests do not invoke the real helper, authenticate or contact Snowflake.
