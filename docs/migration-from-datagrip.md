# Trying Joinery for daily database work

This is a local evaluation guide, not a declaration of DataGrip parity. Confirm
your required engines, authentication and workflows in
[replacement acceptance](replacement-acceptance.md). Current test/build outcomes
and limits are in the [verification ledger](replacement-verification.md).
Keep DataGrip available until your required workflows are accepted.

The confirmed target is Snowflake externalbrowser SSO on macOS, with all
applicable DataGrip functionality including writes, DDL and explicit transactions.
The current SDK external-browser route is integrated for read-only SQL; its local
browser, native and package qualification passes. Existing profiles retain their OAuth/SQL API
route. Neither route satisfies the complete target: SDK metadata, writes, DDL,
transactions and actual account qualification remain open. The fixture walkthrough
below qualifies existing paths only; keep DataGrip until the required walkthrough
is accepted. See [SDK integration and limits](snowflake-sdk-integration.md).

For the confirmed million-row workload, see the
[large-result walkthrough](large-results.md). Temporary result storage requires
explicit opt-in before a query runs, has bounded disk use, and preserves the
memory-only default. Its common-store PostgreSQL qualification does not establish
Snowflake connector performance or replacement readiness.

## Build and run

Follow [README setup](../README.md#development) with Node 24, pnpm 11 and the
tracked Rust toolchain. From a clean dependency installation:

```sh
pnpm install
pnpm validate
pnpm desktop:bundle:mac
```

The macOS application is
`src-tauri/target/release/bundle/macos/Joinery.app`. Open it manually for evaluation;
this build does not install or replace an application in `/Applications`.
`pnpm desktop` runs development mode. A browser-only `pnpm dev` preview does not
provide the normal desktop database service.

The normal bundle uses Joinery's standard application data and OS credential
store. For fully isolated packaged restart/credential evaluation, use a separate
disposable macOS test account. `JOINERY_NATIVE_SMOKE_ROOT` applies only to the
hidden smoke executable; it does not redirect the normal application's data or
keychain. Automated checks in the ledger used isolated harnesses, not existing
private application state.

Local ad-hoc signature verification is separate from Apple Developer ID signing
and notarization. No automatic updater or cross-platform installation qualification
is established by this guide. For a local update, rebuild and evaluate the new
bundle; retain your prior working application until acceptance succeeds.

## Start with disposable data

Use the README fixture setup, not a production database:

```sh
python3 scripts/mysql-tls-fixture.py
docker compose --profile mysql --profile mysql-folded --profile redis up -d --wait
```

In **New connection**, choose the engine and **Local** environment. For the
loopback PostgreSQL fixture use host `127.0.0.1`, port `55432`, database/user
`joinery`, password `joinery-local-only`, and disable TLS for this fixture only.
Start with **ReadOnly** Safe Mode. MySQL/MariaDB fixtures use ports `53306`/
`53307` and the same database/user/password; see README for their custom CA and
the Redis fixture. Never reuse these credentials or fixture keys externally.

Open a query tab and execute:

```sql
SELECT id, status FROM public.orders ORDER BY id LIMIT 10;
```

Try statement versus selection execution, completion, a deliberate syntax error,
result inspection, filtering/sorting in a table tab, and export. Compare exact
large numbers, NULL and empty strings. A SELECT result is a retained snapshot;
run SQL again when you need current server data.

## Bring over your work deliberately

- Create connection profiles manually. Re-enter credentials through the app;
  do not copy DataGrip credential stores, tokens or private application files.
  Verify engine, host, database/schema, role, TLS verification and environment
  before running work. SSH trust must be checked explicitly.
- Open selected `.sql` files through the application's file workflow. Keep your
  script source under your existing version control. Review database context
  and parameters before execution; do not assume DataGrip macros, drivers,
  custom extractors or dialect settings transfer automatically.
- Decide whether to opt into stored query text and restored drafts. They are
  separate from transient parameter values, database sessions and results.
  Restored drafts do not reconnect or execute automatically.
- Learn the displayed shortcuts and command palette; use Preferences for custom
  shortcuts and layout. Verify your own keyboard and accessibility workflow.

The active connection-lifecycle increment adds **Cancel** while connecting.
Cancelling or closing that dialog prevents a delayed response from becoming an
active session; failed cleanup is reported and retried when closing the app.
Closing Joinery asks for a decision when work is pending, saves workspace state,
and awaits local connection retirement. This is not a guarantee about a remote
statement's outcome: verify interrupted writes or transactions on the database.
Local combined lifecycle checks pass, including browser workflows and isolated
native recovery. Normal installed-app restart, OS credentials and actual
Snowflake authentication remain unqualified; consult the ledger.

Connection setup is bounded to 32 open/pending sessions overall and four per
profile. An unstarted reservation expires after 60 seconds; an active setup
keeps its capacity until its owner unwinds. These are current implementation
limits, not evidence of DataGrip parity or accepted limits for your workflow.

## Prove writes and recovery before switching

On disposable fixture rows, change the profile from ReadOnly to a reviewed write
mode. Inspect SQL and bindings in **Review pending changes** before applying a
grid edit. Query-result edits require proven base-table/primary-key provenance;
unsupported results explain why editing is unavailable. After commit, that query
result remains an explicitly stale, read-only snapshot. Rerun to see actual
defaults, generated keys or trigger changes and restore editing.

PostgreSQL verifies original relation/attribute identity. MySQL/MariaDB retain a
read transaction that protects the original table with a metadata lock for up to
60 seconds; at most 32 targets can be live per session. Preparing or applying an
expired target fails closed: refresh/rerun the table or preview the CSV again,
review the new plan and submit deliberately. Preview ownership is exact; an old
or released CSV preview cannot attach to a newer preview of the same file/table.
Every batch carries its original preview token. Closing the import dialog releases
its unused plan and target; a submitted batch keeps its truthful outcome.

External DDL can wait for a live MySQL/MariaDB target. Application DDL revokes retained MySQL/MariaDB guards across the
backend before submission, invalidating old edits and previews.
A writer that obtained its own metadata lock and freshly verified the original
guard may finish after later lease expiry. Coordinate migrations and refresh after
DDL. A successful raw query whose guard expires still displays rows read-only;
it must not be reported as failed merely because editing eligibility ended.

Preliminary fixture qualification passes all 27 original replacement scenarios
across PostgreSQL, MySQL and MariaDB, and 137 browser workflows. Final integrated
qualification for mandatory CSV ownership and verified TLS-over-SSH guard/writer
coverage is pending. See the [identity investigation](table-identity-safety.md),
[guard contract](mysql-table-identity-design.md) and
[verification ledger](replacement-verification.md) for evidence and exact limits.
These fixtures do not qualify external engines/authentication or owner suitability.

Use a console's Manual transaction mode to update a fixture row, inspect it in
that session, then Rollback and verify from another session. Repeat with Commit.
Grid commits use a separate transaction and login role; they do not join the
console's transaction. Check shared versus isolated console bindings explicitly.
Open snapshots in other tabs are not automatically refreshed by your writes.

If a reviewed grid edit or CSV batch loses its COMMIT response, the write may
already be committed. PostgreSQL retires the editor and cancels changes already
queued behind that session; reconnect and inspect the actual table before
retrying. CSV imports halt, disable another submission and distinguish earlier
committed batches from the uncertain current batch. A failed auxiliary MySQL
writer may leave its healthy editor available; editor retirement follows an
explicit session-closure event. Neither case automatically retries the write.

If a console Commit loses its acknowledgement, the app closes that session and
reports an unknown outcome. Reconnect and inspect the data from a fresh session before
deciding what to do; do not submit the write again automatically. If the server
acknowledged Commit but context refresh failed, the message explicitly says the
transaction committed and requires reconnection. An ordinary deferred-constraint
error can end a PostgreSQL transaction while leaving its connection usable; check
the refreshed transaction indicator and context before continuing.

Cancel a long fixture read and verify the session/reconnect behavior before
retrying work. Never automatically retry an uncertain write. Save a draft and
preferences, quit/relaunch, and check recovery; sessions and results should not
be restored. Automated checks include abrupt backend restart and actual isolated
native close/reopen with pending drafts/preferences and PostgreSQL rollback
(`pnpm test:native:recovery`; see README). They use a hidden test build and an
in-memory secret store. They do not substitute for your normal packaged native
quit/restart evaluation or OS credential-store qualification.

Snowflake is a read-only browser-OAuth pilot with open external qualification;
Redis supports a bounded standalone command/key workflow, not Cluster, Sentinel,
streams or Pub/Sub. A required unsupported engine/authentication/operation is a
replacement blocker, not a reason to silently omit that daily workflow.

Record each required task, engine/version, authentication/transport, expected
result, observed behavior and accept/reject in the acceptance document. Ask for
missing workflows to be implemented before retiring DataGrip. No production
writes, paid services, publication or deployment are authorized by this guide.
