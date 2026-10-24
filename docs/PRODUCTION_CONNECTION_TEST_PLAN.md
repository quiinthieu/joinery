# Local testing against production databases

Use the packaged macOS Joinery app to qualify each production connection with
small, read-only queries. PostgreSQL is available now. Snowflake connections
remain blocked on the adapter and authentication work described below. No
production connection has been tested as part of writing this plan.

## Connection inventory and local secrets

Record every PostgreSQL database and Snowflake account/context separately.
For each, identify the owner, engine/server version, direct/VPN/SSH route,
authentication method, read-only role, approved schema/table/columns, expected
TLS identity, and any query or warehouse budget. Start with one PostgreSQL
canary, then expand one connection at a time.

The repository's `.env.example` provides numbered PostgreSQL and Snowflake
blocks. A local `.env` has been created with blank secret fields and mode
`0600`; it is ignored by Git. Fill it locally and duplicate numbered blocks
for the rest of the inventory. Use only the fields needed for each
authentication method. Keep private keys in their existing protected files;
record paths rather than copying key contents. Do not put secrets in chat,
the plan Page, screenshots, issue bodies, or committed files. `.env` is a
plaintext local staging file, not encrypted storage. Never use `VITE_`
variables for secrets or source an untrusted `.env` as shell code.

This test procedure transfers fields manually into native connection forms;
the file is not a working connection importer. Prefer session-only passwords
initially. Exercise optional OS credential storage separately with a dedicated
test credential. Snowflake fields capture future requirements and do not
enable Snowflake in the current app.

| Inventory field | PostgreSQL | Snowflake |
|---|---|---|
| Identity | Host, port, database, login | Account hostname, user, database, schema, role, warehouse |
| Network | Direct, VPN, or one SSH bastion | HTTPS endpoint, VPN/PrivateLink/proxy requirements |
| Authentication | Password; SSH password/key/agent if used | Approved OAuth or key-pair flow; identify SSO/MFA needs |
| Trust | Verified DB certificate, optional CA, SSH fingerprint | Verified HTTPS and account hostname |
| Test scope | Read-only role and approved relations/columns | Read-only grants and approved warehouse/data |
| Limits | DBA-approved timeouts and connection count | Timeout, warehouse spend budget, result partition budget |

## Stage 0: establish a repeatable local baseline

1. Record the app version, Git revision, macOS version, and test timestamp.
2. Start the disposable fixture with `docker compose up -d --wait`.
3. Confirm connect/query, browse, edit/review/apply/readback, and cancellation
   with reconnection using the fixture. Test destructive-query warnings,
   malformed SQL, changed SSH fingerprints, and negative TLS cases only with
   disposable fixtures, not production systems.
4. Preserve the local regression results in
   [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md). Run additional tests
   when fixing failures rather than treating a screenshot as functional proof.

Existing evidence covers 28 Rust tests, 13 frontend tests, four browser
workflows, and native WebView smoke checks. Actual native credential-store
success/denial/locked-store behavior remains to be qualified. Total idle RAM,
fully cold first paint, and 100+ FPS remain open performance gates.

## Stage 1: PostgreSQL canary connection

Use a database-enforced read-only login, ideally against an approved replica.
Select the **Production** group and the read-only Safe Mode policy. Require
verified TLS; for SSH, compare the displayed fingerprint with the owner's
independently supplied fingerprint before trusting it. Retain the database's
real hostname so TLS verifies the database identity through the bastion.

Use an existing least-privilege role. An administrator can arrange test-role
defaults for a short statement timeout, lock timeout, and idle-transaction
timeout across all connections. Proposed initial budgets are 15 seconds,
2 seconds, and 30 seconds respectively, subject to the database owner's
limits. Role defaults or server controls matter because grid browsing uses
separate connections from the SQL editor. Editor `SET` commands do not
configure those connections. PostgreSQL documents these settings in
[Client Connection Defaults](https://www.postgresql.org/docs/18/runtime-config-client.html).

Run these identity/settings checks in the editor:

```sql
SELECT current_database(), current_user, version();
SHOW transaction_read_only;
SHOW statement_timeout;
SHOW lock_timeout;
SHOW idle_in_transaction_session_timeout;
SELECT 1 AS connection_ok;
```

Then exercise the following cases. Record expected, observed, duration, and
any sanitized error; mark unrun cases explicitly.

| ID | Case | Pass condition |
|---|---|---|
| PG-01 | Open and reconnect | Correct database/login; no silent TLS downgrade; health reaches connected |
| PG-02 | Schema discovery | Approved schemas/tables and PK/FK metadata load; permission failures are clear |
| PG-03 | Editor completion | Suggestions match the selected database/schema and refresh without stale columns |
| PG-04 | Bounded read | An owner-approved projection/filter with `LIMIT 50` returns expected values |
| PG-05 | Multiple statements | Two small SELECT statements yield separate result tabs and metrics |
| PG-06 | Value fidelity | Synthetic bigint, decimal, NULL/empty, Unicode, timestamps, JSON, and binary displays match expectations |
| PG-07 | Grid browsing | First page and a few subsequent pages load; PK order and row labels remain consistent |
| PG-08 | Read-only protection | Editing controls and policy behavior match read-only mode; attempt no production DML |
| PG-09 | History and audit | SQL-text history remains off unless explicitly chosen; audit contains metadata rather than result rows |
| PG-10 | Session lifecycle | Disconnect/reconnect and app restart work; saved profiles contain no unexpected plaintext secrets |
| PG-11 | Network lifecycle | Ordinary reconnect after VPN/SSH interruption works without replaying a query; disruptive failure injection stays in fixtures |
| PG-12 | Own-query cancellation | If permitted, cancel a short `SELECT pg_sleep(5)`; confirm terminal state and reconnect before the next query |

Use synthetic SELECT literals for value testing so no sensitive production
rows need to be copied. Choose indexed, owner-approved predicates and narrow
columns for data checks. `LIMIT` caps returned rows, not query cost. Avoid
full counts, broad sorts, unreviewed views/functions, `EXPLAIN ANALYZE`,
unbounded exports, and cancelling other users' queries during this pass.
The app's retained-result limit does not cap server work.

Read-only mode supplements database grants; it does not prove arbitrary SQL
or user-defined functions cannot have side effects. Keep queries explicit
and limited to the approved test set.

## Stage 2: expand across the PostgreSQL list

Repeat PG-01 through PG-12 for each connection, marking optional cancellation
and permission-dependent process inspection as skipped when unavailable.
Cover every route actually present: direct TLS, VPN, custom CA, and SSH
password/encrypted key/agent. Identify unsupported database authentication
requirements such as automated IAM token refresh as gaps rather than claiming
coverage from a one-time password login.

For a modest usage pass, open two SQL tabs, run five small queries, browse a
few pages, and disconnect. Sample empty idle, connected idle, and active-grid
memory separately, including WebKit helper processes. Record first paint
and interaction latency independently of query execution. Use a local
synthetic grid and a 120 Hz display for frame-time qualification; production
databases are not a stress-test fixture.

Run editable-table and concurrent-update tests only against disposable local
tables or a separately authorized staging sandbox. Test cross-page rollback,
stale/deleted rows, constraints, open editor transactions, pending-work close
protection, and uncertain commit acknowledgement there. Do not create test
objects or perform trial writes in production under this plan.

## Stage 3: Snowflake implementation and fixture gate

Current status: **not supported in the shipped PostgreSQL milestone**.
Inventory accounts now; connection attempts in Joinery start only after a
tested adapter exists. Keep this as an explicit follow-on implementation
milestone, with the approved P0 performance gate still applicable.

1. Add a Rust Snowflake SQL API adapter with explicit capability reporting;
   reuse the existing execution gateway, immutable plans, and bounded result
   protocol. Initially advertise SQL reads, catalog, and own-query
   cancellation; keep editing unavailable until separately proven.
2. Implement the authentication methods actually required by the inventory.
   Support approved OAuth and/or encrypted key-pair signing, secure token/key
   handling, refresh/expiry, and actionable MFA/SSO errors. An existing CLI
   `externalbrowser` login is not proof that a native OAuth integration works.
   Follow Snowflake's [SQL API authentication documentation](https://docs.snowflake.com/en/developer-guide/sql-api/authenticating).
3. Specify account, role, database, schema, warehouse, request timeout, and
   query tag explicitly. Do not silently fall back to a broader default role
   or warehouse. Implement asynchronous handles, polling, cancellation,
   bounded partition retrieval, exact numeric text, and uncertain submission
   outcomes. These controls are exposed by the
   [SQL API reference](https://docs.snowflake.com/en/developer-guide/sql-api/reference).
4. Test account-hostname validation, expired tokens, wrong role, denied
   objects, unavailable warehouses, asynchronous results, partition limits,
   cancellation races, and lost responses with mocks and a restricted
   Snowflake test account. Prevent arbitrary credential-bearing URL redirects
   and never automatically replay a statement with an unknown outcome.
5. Verify system-WebView IPC, result fidelity, bounded memory, and profile
   isolation before enabling any production Snowflake profile.

Estimated relative scope: high; use the existing P6 planning range of 3–4
engineer-weeks for full adapter qualification, refined after inventory and
authentication requirements are known. A limited read-only pilot is a
smaller milestone with separate exit criteria, not full P6 completion.

## Stage 4: Snowflake production canary and expansion

Use an approved role with only the necessary database/schema access and
SELECT grants, plus warehouse usage. Keep secondary-role activation and
privilege expansion outside the pilot. Use a small dedicated warehouse when
available, with an agreed timeout/spend budget and an owner-provided query
tag. Do not modify warehouse settings or grants as part of connecting.

For each account/context, verify the chosen authentication and network route,
then run:

```sql
SELECT CURRENT_ACCOUNT(), CURRENT_USER(), CURRENT_ROLE(),
       CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE();
SELECT 1 AS CONNECTION_OK;
```

Follow with an approved projection/filter and `LIMIT 50`. Verify catalog
visibility, separate results, NUMBER precision, timestamp variants,
VARIANT/NULL display, bounded result partitions, token expiry behavior,
own-query cancellation, query IDs/tag, and reconnect behavior. Compare the
same small query in an approved existing client. Keep async/large-result and
failure injection tests in the test account. Record any network policy,
PrivateLink, proxy, OAuth integration, or unsupported feature as a blocker.

## Evidence and exit criteria

Maintain a private local test receipt per connection; do not commit filled
inventories. Store receipts under ignored `.cache/production-connection-tests/`
without secrets, SQL literals containing private data, or row screenshots.

```text
Connection alias / engine / server version:
App version / Git revision / OS / timestamp:
Route / auth method / effective role:
Approved relation alias / query budget:
Case ID | expected | observed | duration | pass/fail/skipped
Own query ID or sanitized error code:
Memory/latency observations:
Cleanup / open issues / owner sign-off:
```

A connection passes when identity and verified transport are correct, the
approved read queries and metadata work, values are faithful, cancellation
and reconnect behavior are understood, and no secrets or rows enter logs or
persistence. Track unsupported and unrun cases separately. Any wrong database,
wrong effective role, certificate/fingerprint mismatch, secret leakage,
unexpected mutation, or ambiguous execution outcome blocks further testing
of that connection until resolved.

The deliverable is a per-connection compatibility matrix and a prioritized
issue list. A successful PostgreSQL canary does not validate Snowflake, every
network path, production writes, or the remaining release performance gates.
