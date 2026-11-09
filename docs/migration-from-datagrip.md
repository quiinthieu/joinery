# Trying Joinery for daily database work

This is a local evaluation guide, not a declaration of DataGrip parity. Confirm
your required engines, authentication and workflows in
[replacement acceptance](replacement-acceptance.md). Current test/build outcomes
and limits are in the [verification ledger](replacement-verification.md).
Keep DataGrip available until your required workflows are accepted.

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

## Prove writes and recovery before switching

On disposable fixture rows, change the profile from ReadOnly to a reviewed write
mode. Inspect SQL and bindings in **Review pending changes** before applying a
grid edit. Query-result edits require proven base-table/primary-key provenance;
unsupported results explain why editing is unavailable. After commit, that query
result remains an explicitly stale, read-only snapshot. Rerun to see actual
defaults, generated keys or trigger changes and restore editing.

The `ff59ca6` diagnostic reproduced permanent table replacement failures across
PostgreSQL, MySQL and MariaDB, including CSV. The final PostgreSQL 18.6 strict
fixture diagnostic passes all nine expected rejections, and the formal example
regression passes separately. Latest validation passes 561 frontend / 186 Rust
tests with strict Clippy; one clean fixture command passes 112. The focused identity browser run passes 9/9; after test-coordinate repair and
UUID fixture isolation, the full browser suite passes 129/129 and focused
table-data/raw-browse tests pass 11/11. Earlier full/focused failures are historical
test-coordinate and fixture-isolation failures. The 18
MySQL/MariaDB cases remain unsafe; see the bounded
[guard design proposal](mysql-table-identity-design.md). Until the relevant
engine and workflow are qualified, discard pending edits and CSV
previews after external DDL, read or preview again, and coordinate DDL throughout
review and submission. This reduces exposure; a fresh read cannot protect
against a later replacement. See the [identity investigation](table-identity-safety.md)
for the original 27 cases, partial PostgreSQL repair, and remaining tests.

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
