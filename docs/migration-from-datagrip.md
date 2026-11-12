# Trying Joinery for daily database work

This is a local evaluation guide, not a declaration of DataGrip parity. The
confirmed target and required workflows are recorded in
[replacement acceptance](replacement-acceptance.md). Current test/build outcomes
and limits are in the [verification ledger](replacement-verification.md).
Keep DataGrip available until your required workflows are accepted.

The new [reviewed retained-console walkthrough](snowflake-sdk-console-workflow.md)
describes opt-in connection setup, mixed-script review, retained session context,
explicit transaction boundaries and SQL-free receipt/result recovery. It is a
locally verified implementation with a synthetic SDK, including durable refusal
response-loss/restart recovery and a bounded receipt panel. This does not
establish real Snowflake effects, current packaged native UI behavior or daily
replacement readiness; packaging this increment remains pending.

Console receipts retain both admitted runs and committed pre-admission refusals.
After a lost response, use **Reconcile receipt (no SQL)** to retrieve the original local
decision. A refused request needs a new explicit review before a different
request can run. A missing receipt or recording error remains uncertain; never
retry SQL merely to discover its outcome. Restart restores opaque request IDs,
then retrieves their decisions without reconnecting or submitting SQL. Earlier
versions' discarded refusal IDs cannot be reconstructed. See the walkthrough
for recording holds and recovery limits. Receipt history opens in a bounded,
keyboard-scrollable panel; its pending notices stay visible when settled history
is collapsed. The panel can cover part of the lower results area while open,
without displacing the editor or run toolbar.

This increment upgrades local metadata to schema 3. Earlier Joinery versions
cannot open that format. Preserve a pre-upgrade metadata backup if you need to
return to an earlier app; never run both versions against the same data directory.

The confirmed target is Snowflake externalbrowser SSO on macOS, with all
applicable DataGrip functionality including writes, DDL and explicit transactions.
The SDK external-browser route supports reads, separately reviewed bounded SQL
INSERT/UPDATE/DELETE, generated CREATE TABLE/nullable ADD COLUMN/DROP TABLE,
and explicit transaction controls locally. Account mutation
effects remain unqualified. The preceding
profile/runtime increment passed its local browser, native and package checks;
the later browsing increment has separate [qualification evidence](replacement-verification.md#sdk-relation-browsing-10-november-2026). Existing profiles retain their OAuth/SQL API
route. SDK same-session completion is implemented and locally qualified under its
[qualification contract](snowflake-sdk-metadata-design.md). Neither route satisfies
the complete target. SDK relation browsing, column inspection, generated DDL,
declared key inspection and read-only preview are implemented locally. Complete
object/constraint inspection, editable browsing, broader writes/DDL, complete
transactions and account qualification remain
open. The fixture walkthrough
below qualifies existing paths only; keep DataGrip until the required walkthrough
is accepted. See [SDK integration and limits](snowflake-sdk-integration.md).

For the confirmed million-row workload, see the
[large-result walkthrough](large-results.md). Temporary result storage requires
explicit opt-in before a query runs, has bounded disk use, and preserves the
memory-only default. Its common-store PostgreSQL qualification does not establish
Snowflake connector performance or replacement readiness.

The owner permits creating new tables under `SANDBOX.BILLIE_TESTING`; the earlier
database-creation approval was consumed by the rejected `CREATE DATABASE` attempt
and must not be replayed. The recorded internal full-runtime benchmark passes all
32 synthetic phases at S=1 (393 SQL requests). After a reproduced S=32
admission timeout, the accelerated-source S=32 rerun passes all 32 phases /
610 requests. The same-source S=1 rerun also passes all 32 phases / 393 requests;
on that source, the complete module (204 tests) and application validation (679 frontend / 467
standard Rust tests, strict checks) pass. These are internal results, not product or
account acceptance. Offline source/binary/full-runtime/profile binding and one-use
consumption are implemented. The private factory now owns exclusive persistent
attempt/workspace/state, construction exclusively from consumed roots and final
terminal evidence, including dropped waiters. The dedicated
[qualification command](snowflake-table-offline-command.md) can emit private review
files and verify the original bytes without connecting or launching the helper.
Its separately approved execute route restores those original bytes, consumes
fixed macOS account-home markers and retains the private factory through terminal
persistence. SIGINT/SIGTERM revoke future admissions without aborting the owner;
foreground Ctrl-C can also terminate the helper and leave an unknown original
outcome. Never replay a spent attempt or infer rollback from interruption. Local
process checks use synthetic homes and SDK responses; account and normal packaged
acceptance remain open. Before account activity, current source/runtime/executable
pins, a fresh exact table-only plan and fresh possible-billing approval are
required. See the [qualification attachments](snowflake-qualification-attachments.md),
[offline SQL ledger](snowflake-qualification-sql-ledger.md), and current
[verification ledger](replacement-verification.md).

## Script stops and statement counts

The [execution accounting contract](script-execution-accounting.md) distinguishes
planned statements, attempted statements, successful acknowledgements and the
unattempted suffix. The editor, query history and history search show the final
summary. A result-limit stop with remaining statements displays **Stopped**, the
limit reason and which statements were not attempted. A final limited read may
show **Completed** with a result-limit reason; inspect the result's truncation
label. Older history explicitly has no recorded counts.

An attempt does not prove the server received SQL. Completed statements may still
be uncommitted, and retained results may be incomplete. After a stop, inspect the
observed transaction state before choosing an explicit commit or rollback. Nothing
automatically resumes the suffix, retries SQL or changes transaction state.

For local acceptance, connect to the isolated PostgreSQL fixture documented in
README, turn SQL-text history off and leave temporary result storage off. Run
`SELECT n FROM generate_series(1,1000001) n; SELECT 'unattempted suffix'` using
**Run all statements**. Expect one truncated result and **Stopped**, 2 planned,
1 attempted, 1 completed, 1 not attempted. Query history and the command palette's
History scope must preserve that outcome without storing SQL text. A subsequent
`SELECT 7` must replace the previous summary. The automated browser case also
uses a uniquely named fixture table and a fresh observer connection to prove that
an UPDATE suffix did not execute; cleanup touches only that table.

The real Rust fixture additionally checks byte limits, cancellation, explicit
server termination, a successful two-statement control, durable restart accounting
and a skipped COMMIT that leaves the transaction open until explicit rollback.
These checks qualify the common executor locally; account Snowflake execution,
normal packaged UI usage and owner acceptance remain open. No account SQL is
authorized by this walkthrough.

## Snowflake CHECK constraints (prepared acceptance walkthrough)

The [CHECK contract](snowflake-sdk-check-constraints-design.md) and
[verification ledger](replacement-verification.md) distinguish implementation,
local checks and account acceptance. This walkthrough is prepared, not completed
owner acceptance. Account browsing and the new CHECK view require a separately
approved scope. Creating fixtures or modifying permissions requires exact
approved disposable SQL; the expired table-lifecycle plan does not cover this read.

1. In an SDK Snowflake console, open **Browse relations**, select a current
   available target and choose **View CHECK constraints**. Other metadata actions
   are independent: none needs to succeed first.
2. On an approved fixture with known CHECK clauses, compare the exact qualified
   table, constraint name and multiline clause text. Include quoted names,
   same-named tables in different schemas and Unicode text. The clause must
   appear as inert read-only text; no execution or validation-status claim follows.
3. A successful empty inventory says **No visible CHECK constraints returned**.
   Under an approved denied-privilege fixture, verify the explicit error/retry
   remains distinct and preserves columns, keys, mappings, definitions and preview.
4. Refresh, change selection/context or apply a metadata hold while the read is
   pending. Previous CHECK output must disappear; late replies must not restore
   it. Repeat after selecting a referenced table and after reconnecting.
5. Exercise the action by keyboard in the normal packaged macOS app. Record
   source/binary version, focus/accessibility behavior and observed account
   outcomes. Reopen the app and verify profile/script recovery separately.

The view does not expose enforcement or existing-row validation status; empty
role-visible metadata proves neither absence nor editing eligibility. Creating,
changing and validating CHECK constraints remains separate schema work. Keep
account visibility, interruption, object-family compatibility and owner acceptance
open until demonstrated.

## Declared Snowflake keys (prepared acceptance walkthrough)

Local focused/full validation, headless browser, hidden native IPC/cleanup and
normal SDK-bearing package checks pass on the source recorded in the ledger.
Normal packaged SSO/daily usage and account acceptance remain open. Use a
separately approved evaluation connection before sending account metadata reads.

1. In an SDK Snowflake console, open **Browse relations**, filter by schema/name
   and select a current available relation. Choose **View key constraints**;
   successful column inspection or definition retrieval is not a prerequisite.
2. Inspect the primary, unique and foreign-key declarations, reported enforcement,
   deferrability, RELY and comments. Foreign keys show a qualified referenced
   **constraint** and its rules when visible. This is not a table/column mapping;
   **Not visible** retains the distinction from **Not applicable**.
3. A successful empty response says no visible declared keys were returned. A
   denied request says the keys are unavailable and allows an explicit retry.
   Neither case proves that the table has no constraints.
4. Refresh relations or change the selection/context. The previous key inventory
   must disappear; a late response must not restore it. Check the same flow by
   keyboard and confirm the dialog returns focus when closed.

This declaration view excludes key column mappings, CHECK and NOT NULL constraint inventories;
column inspection separately shows nullability. Reported declarations and
enforcement do not establish safe editing or validate existing data. Keep the
remaining metadata acceptance gates open. See the
[key inspection contract](snowflake-sdk-constraints-design.md) and
[verification ledger](replacement-verification.md) for evidence and limits.

## Snowflake key columns (prepared acceptance walkthrough)

Implementation and local verification are tracked separately in the
[mapping contract](snowflake-sdk-key-mappings-design.md) and
[ledger](replacement-verification.md). This walkthrough is not completed account
acceptance. Use a separately approved evaluation scope covering browsing,
declaration reads and the three per-table SHOW/pipe projections. Existing
table-lifecycle approvals do not include the new commands. Any fixture creation
or data changes require their own approved disposable SQL plan.

1. Select a current available ordinary table with known composite primary,
   unique and foreign keys. Choose **View key column mappings**. Neither column
   inspection nor **View key constraints** is a prerequisite.
2. Compare the displayed order with the known declaration. For a foreign key
   whose referenced columns have a different order, verify each local column
   stays paired with its corresponding referenced column and that the full
   database/schema/table is correct. Quoted names must remain distinguishable.
3. Compare **View key constraints** with the independent mapping view. A hidden
   reference declaration can coexist with a visible SHOW mapping. **Not visible**
   mappings must not be described as a key with zero columns or no key; metadata
   visibility and propagation may differ. Dynamic tables report **Unsupported**
   mappings; targets with unknown traits remain unavailable for inspection.
4. Under an approved denied-privilege fixture, verify a mapping failure leaves
   earlier declarations, columns and definitions available. Restore only the
   approved fixture permission and explicitly retry. A successful empty inventory
   must remain distinct from the failed action.
5. Refresh, change the selection/context and apply a metadata hold while a
   request is pending. Old results must disappear and a late response must not
   restore them. Repeat by keyboard in the normal packaged macOS app and record
   focus behavior, source version and observed account outcomes.

The mapping workflow displays referenced identities. The local navigation
increment below selects retained destinations; cross-database navigation and
inbound references remain required. Mappings and reported enforcement do not
validate existing values or establish editing safety. Keep these account and
owner acceptance gates open until demonstrated.

## Referenced Snowflake tables (prepared acceptance walkthrough)

Implementation/verification status is recorded in the
[selection contract](snowflake-sdk-key-mappings-design.md#referenced-table-selection)
and [ledger](replacement-verification.md). This is a prepared walkthrough, not
completed account or owner acceptance. Loading the catalog, mappings and later
inspection still requires a separately approved account scope.

1. Use an approved source table with a foreign key to a known table in another
   schema of the same database. Include quoted names and a same-named table in a
   different schema so matching the wrong destination would be observable.
2. Filter the relation list to the source, select it and choose **View key column
   mappings**. Check the full referenced database/schema/table and column pairs.
3. By keyboard, activate **Select referenced table** for the intended destination.
   The filter must clear, the Relation selector must select the exact retained
   destination and receive focus, and old details must disappear. No metadata
   query or row preview should start automatically.
4. Explicitly choose **Inspect relation**. Confirm columns belong to the
   destination and that the console's active schema/context did not switch.
   Definitions, keys and preview remain separate explicit actions within the
   approved scope.
5. Check an external-database, missing, ambiguous or unavailable destination.
   The action must explain why it cannot select the table; it must not refresh,
   connect, switch context or choose a partially matching name.
6. Repeat after refresh, owner/session replacement and a transaction/metadata
   hold. Previous mapping actions must disappear and must not revive after the
   hold clears. Record normal macOS focus/keyboard behavior, source version and
   observed account requests. The automated same-batch stale-handler regression
   remains separate from this hands-on walkthrough.

Cross-database catalogs/navigation, inbound references and actual role/DDL
visibility remain required. Selection does not establish immutable table identity
or permit editing.

## Explicit SDK transaction controls (local acceptance walkthrough)

Keep an existing profile ReadOnly unless deliberately evaluating boundary controls
in an approved isolated scope. An external-browser SDK profile can select Confirm,
Guard, Silent or Password gated; production normalizes non-ReadOnly policy to
Password gated. Switching to OAuth resets ReadOnly. This authorizes only the
separate buttons. SQL writes require an additional reviewed-DML opt-in; generated
table DDL requires a separate reviewed-DDL opt-in. Raw transaction statements remain blocked. The Auto-commit/Manual dropdown remains
disabled for SDK sessions.

After a fresh context observation, an idle console offers **Begin transaction**.
An observed active transaction offers **Commit transaction** and **Rollback
transaction**. BEGIN/COMMIT use the selected review/passphrase policy; ROLLBACK
retains its recovery exemption. A pending action blocks queries, schema selection
and background metadata/health work. After completion, use **Refresh completion
context** deliberately to resume metadata loading. Acknowledged means the command
was accepted; verify actual data effects independently before repeating a write.

If the follow-up observer is rejected, the console can show **Only rollback
recovery is available**. Use **Rollback transaction** or disconnect. Rollback
needs no successful pre-read and cannot be trapped by normal review policy.
An unknown/retired result closes local ownership without claiming remote rollback;
inspect the remote state before reconnecting or repeating work. No automatic
retry or reconnect occurs. These steps are a prepared walkthrough, not account
transaction acceptance: no transaction/write approval has been consumed for this
increment. See the [contract and required checks](snowflake-sdk-transactions-design.md).

## Local outcome-recording recovery

A statement acknowledgement and local history/audit recording are separate
results. If Joinery reports incomplete outcome recording, keep it open and do
not rerun SQL to repair history. Pending original receipts block new reviewed
Snowflake writes, schema changes, BEGIN/COMMIT and normal shutdown until local
recording succeeds.
Explicit typed rollback remains available under its normal recovery policy and
retains its own outcome; it does not replay an earlier statement.
After a submitted rollback, its pending receipt must be recorded before another
rollback on that console.

Resolve an open transaction deliberately. Restore local storage availability,
then retry closing when ready; the receipt retry writes local records only.
Transaction boundaries retain their existing audit evidence, not a new script
history entry. While SQLite cannot write, pending receipts remain in memory:
forced termination or power loss can lose them. Neither a recorded receipt nor
an acknowledgement proves committed account data effects. Current verification
and remaining account gates are in the [ledger](replacement-verification.md).

## Reviewed SDK table DDL (prepared acceptance walkthrough)

These steps require a fresh exact account plan and possible-billing approval.
The owner permits **new test tables under `SANDBOX.BILLIE_TESTING`**; the consumed
CREATE DATABASE setup is not reusable. No existing table should be modified or
dropped. Keep DataGrip available for independent inspection and recovery.

1. Use an External browser SSO profile with a deliberately selected writable
   Safe Mode, current database `SANDBOX` and schema `BILLIE_TESTING`. Separately
   enable **Allow separately reviewed CREATE TABLE, nullable ADD COLUMN, and DROP
   TABLE**. This defaults off and is independent of reviewed SQL DML. OAuth and
   ReadOnly cannot enable it. Use a dedicated idle AUTOCOMMIT console; the app
   never changes AUTOCOMMIT to permit DDL.
2. Open **Browse relations** in that console. Choose **Create table…** and the
   fresh name from the approved plan. Enter only the approved scalar columns,
   preserving identifier case. **Generate & review SQL** performs bounded server
   checks; it can incur charges even before dispatch. Inspect the fully qualified
   SQL and transient/implicit-commit warnings. Approve once using the configured
   confirmation/passphrase. Even Silent mode requires DDL review.
3. While pending, **Disconnect to stop write** retires local ownership. It does
   not prove that Snowflake did not execute the statement. Never repeat a write
   based only on a closed dialog, lost connection or missing acknowledgement.
4. **Statement acknowledged** means Snowflake acknowledged the statement, not
   that the catalog is reconciled. Prior results remain snapshots. Background
   metadata stays paused. Use **Refresh completion context** explicitly; denied
   or inconsistent reconciliation retains the hold and can be retried locally
   by the owner. A runtime-only observation cannot release the catalog hold.
5. Browse and inspect the newly created table. Choose **Add nullable column…**
   for the exact approved column, review and authorize again, then explicitly
   reconcile. For **Drop table…**, also type the full quoted target name exactly;
   connection-name confirmation alone is insufficient. DROP uses RESTRICT and
   can fail on dependencies. DDL is never part of rollback recovery.
6. Confirm each new object/effect independently and check typed History after
   application restart. A failed local history/audit write retains the original
   receipt and blocks further DDL and normal exit until local recording works.
   It does not promise crash durability while local storage cannot write.

Only ordinary permanent/transient base tables with known supported traits are
admitted. Temporary, dynamic, hybrid, Iceberg and immutable tables, arbitrary
SQL, rename/type changes, defaults, keys/indexes, broader objects and migrations
remain outside this implementation and remain required replacement gaps.
Concurrent object replacement or temporary-name shadows can change effects;
reviewed names do not establish immutable remote object identity. Transient
objects have no Fail-safe and at most one day of Time Travel. Treat unknown
delivery as unknown: inspect remote state before reconnecting or planning another
action. Local synthetic/browser checks are not account or owner acceptance.

## Reviewed SDK SQL writes (prepared acceptance walkthrough)

Use this only after a fresh exact table-only account plan for the owner-permitted
`SANDBOX.BILLIE_TESTING` and the exact statement/billing scope have been separately
approved. The rejected `CREATE DATABASE` setup is consumed and cannot be reused.
These steps are a concrete evaluation sequence, not permission to run account SQL.
Local UI fixtures already exercise review, parameters and outcome presentation;
actual Snowflake data effects and owner acceptance remain open.

The proposed first [table lifecycle](snowflake-existing-schema-qualification.md)
uses fixed literal SQL. The parameterized example below is a separate follow-up
walkthrough: its statement and values need their own exact approval and must not
be substituted into that closed lifecycle.

1. Edit an **External browser SSO** profile, deliberately choose the appropriate
   non-ReadOnly Safe Mode, and check **Allow separately reviewed single INSERT,
   UPDATE, and DELETE statements**. New and existing profiles default this off.
   OAuth and ReadOnly cannot enable it. Production uses Password gated mode.
2. Connect with current database `SANDBOX` and schema `BILLIE_TESTING`, then
   obtain a coherent context observation. Use a separate console for this test.
   Begin a transaction with the explicit **Begin transaction** button; its review
   is independent of DML. Then use **Refresh completion context** to restore
   normal metadata authority.
3. Enter a single supported statement against the new table named by the fresh
   approved plan. For example:

   ```sql
   INSERT INTO "SANDBOX"."BILLIE_TESTING"."<TABLE_FROM_FRESH_APPROVED_PLAN>"
     ("ID", "NOTE", "ACTIVE") VALUES (:id, :note, :active)
   ```

   `<TABLE_FROM_FRESH_APPROVED_PLAN>` is a documentation placeholder, not a
   runnable or pre-approved table name; substitute only the exact generated table
   identifier in the fresh approved plan. Use Integer `9007199254740993`, Text
   `synthetic note` and Boolean `true` in the parameter dialog. Integer text stays
   exact.

4. Inspect the SQL, ordered typed parameters and warnings in **Review this
   operation**. Approve once with the policy's confirmation/passphrase. The review
   closes after approval. Running controls and background metadata pause; prior
   query results remain labeled as belonging to the earlier execution.
5. Inspect **Snowflake INSERT outcome**. Require acknowledged success and an
   exact affected-row string of `1`; unavailable counts must say unavailable,
   never zero. This is a statement receipt, not proof the row is committed.
   Exercise equivalent reviewed UPDATE and DELETE plans only in the separately
   approved synthetic sequence, checking exact counts and expected values.
6. Use **Rollback transaction** and independently verify the inserted row is
   absent. In a fresh separately approved transaction, repeat the insert and
   deliberately **Commit transaction**, then verify persistence from a distinct
   observer session. Never infer either effect from the button acknowledgement.
7. Inspect **History**, quit normally and reopen an isolated evaluation profile.
   Require the exact outcome/count receipt with no automatic connection or SQL
   replay. Bound values are not stored in DML history. SQL text is retained only
   when profile query-text storage is enabled; do not put secrets in SQL literals.

Every new write requires a fresh review. Supported syntax is deliberately bounded:
explicit INSERT columns with VALUES; plain UPDATE assignments and optional WHERE;
plain DELETE and optional WHERE; scalar binds/literals and admitted expressions.
MERGE, INSERT SELECT, functions/subqueries, joins, batches, bulk writes and DDL
remain gaps. The review targets the displayed SQL name. It cannot pin an immutable
Snowflake object: concurrent DDL, temporary-name shadows, defaults, policies and
dependencies can change resolution or indirect effects. Grid editing is still
disabled. These limits do not reduce the required DataGrip replacement scope.

After a known result, explicitly **Refresh completion context** when ready to
resume metadata; failed or incoherent refresh leaves the hold in place. If only
rollback recovery remains, use **Rollback transaction** or disconnect. During a
pending write, **Disconnect to stop write** ends local ownership but cannot prove
server cancellation or rollback. An **unknown** outcome retires the session;
inspect the remote state through an independently authorized route before
reconnecting or preparing another write. No automatic retry occurs. A graceful
close waits for retained outcome history and stays open if that drain times out;
force-killing the process cannot provide the same guarantee.

Record the app/package hash, approvals, exact SQL budget including all observers,
statement IDs privately, before/after row counts, transaction effects, error paths
and restart outcome in the [verification ledger](replacement-verification.md).
The [DDL proposal](snowflake-sdk-ddl-design.md) describes the next required schema
workflow; it is not currently available.

## Build and run

Follow [README setup](../README.md#development) with Node 24, pnpm 11 and the
tracked Rust toolchain. From a clean dependency installation:

```sh
pnpm install
pnpm validate
pnpm desktop:bundle:mac
```

For the confirmed Snowflake external-browser target, first follow
[runtime preparation](snowflake-runtime.md), then build with the SDK resources:

```sh
pnpm snowflake:prepare
pnpm desktop:snowflake:bundle:mac
pnpm test:snowflake:startup --bundle src-tauri/target/release/bundle/macos/Joinery.app
```

Runtime preparation requires the pinned official Node archive described in that
guide. The startup check performs hello/EOF only, without authentication or SQL.
The ordinary bundle command does not include SDK resources and cannot connect an
SDK profile. Neither build installs the application.

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

## Current Snowflake evaluation boundary

The corrected, separately approved helper-only read/metadata attempt on
10 November passed all 24 planned SQL statements at 12:59:21 UTC, including
external-browser SSO, context/settings, bounded schema discovery and synthetic
scalars. Logout, child reaping and private authentication-state cleanup passed.
The earlier failed attempt is retained in the ledger. Both approvals are consumed
and authorize no additional application workflows. See the
[account pass and limits](replacement-verification.md#corrected-account-readmetadata-qualification-10-november-2026).
The current `fb091db` SDK bundle (224.33 MiB) passes normal packaging, strict
ad-hoc signature, resources and guarded SDK startup without authentication/SQL.
Three hidden native runs pass 20 check categories with ten cleanup cycles each.
The reviewed DML workflow also passes local browser/helper and shutdown checks;
these do not qualify actual Snowflake write or transaction effects. Normal
application account acceptance remains open. The app is not installed or released.

Create a Snowflake profile with **External browser SSO**, the existing account
hostname and approved role/warehouse/database/schema. Existing OAuth profiles
keep their prior authentication mode; select the SDK explicitly rather than
expecting an automatic migration. Do not copy tokens or private DataGrip state.

Account metadata needs separate qualification authorization: opening a query
editor can issue same-session metadata SQL and use the warehouse. The approved
two-query helper canary did not include this workflow. After a reviewed account
scope is authorized, verify the displayed completion context, relation suggestions
and lazy alias-column suggestions against that scope. Use **Refresh completion
context** after a context failure; stale suggestions must stay unavailable until
refresh succeeds. The [contract](snowflake-sdk-metadata-design.md) records bounds,
permission handling and relative-query-history limitations.

The current source also has a distinct **Browse relations** action in an SDK
query console. It loads visible schemas and relations for the configured current
database, then offers bounded column inspection, generated relation definitions
and an unordered read-only preview. This is separate from generic table browsing
(`tableBrowsing` remains disabled) and provides no editing. Inspection reports
constraints as **not fetched**; metadata does not establish edit eligibility.
The feature is locally implemented but not qualified against an account.

After selecting an available relation, **View definition** works before column
inspection. Read the generated-DDL warning, then use **Copy DDL** to copy the
complete returned text. The panel offers no execution action. Snowflake may
normalize types, emit `CREATE OR REPLACE`, or hide policy/tag details; review the
text before using it elsewhere. Definitions that exceed the response budget are
rejected rather than shown partially. Selection, refresh and observed context
changes clear old definitions.

Do not use the old two-query canary approval for this flow. The separate
[browsing contract](snowflake-sdk-browsing-design.md) describes the exact local
limits and a synthetic UI check. For an account walkthrough, first obtain reviewed
approval covering the query-editor completion metadata and Browse relations
schema/relationship metadata. Definition viewing additionally needs `GET_DDL`
approval for the named target and its context/metadata checks. Previewing rows
from an existing table requires additional approval for that exact target and data scope; the current account
qualification plan excludes existing-table data reads. Without approval, use the
synthetic test flow documented in the contract and do not authenticate to
Snowflake for this walkthrough.

Guarded **Active schema** selection is now implemented for the SDK route;
application qualification is recorded in the
[ledger](replacement-verification.md#sdk-guarded-schema-selection-10-november-2026).
After separately authorizing the account workflow, refresh completion context,
require an observed idle transaction, and select a schema from the visible
current-database list. Quoted punctuation remains part of the selected name.
The selector says **Changing schema…** while pending and blocks concurrent query
or metadata admission across tabs. Success refreshes editor context; a final-
verification failure after known USE acknowledgement closes the session and asks
for reconnection. Do not repeat the switch to infer what happened remotely.

Metadata lookup, relation browsing, query preparation/execution and automatic
health can revoke the opaque selection authority. The disabled selector then
says **Refresh to change schema**; use the existing **Refresh completion context**
button deliberately. No automatic eleven-read observation is added after each
lazy lookup. The switch itself uses thirteen SQL executions; subsequent editor
completion and eleven-read options add their own SQL, including other open
editors. These must be included in any new account scope. Cross-database, role,
warehouse and active-transaction switching remain incomplete and required.

The SDK query console also observes settings and transaction activity on its
existing connection: ten fixed reads per state observation, or eleven with a
bounded schema-options read. These reads also require separate account scope.
Studio automatically requests the ten-read observation after each qualifying
serialized SQL action while the session stays open and the run was not cancelled,
including some database-error outcomes; an early observer failure can use fewer
reads. Include this cadence and editor refreshes in the approved account scope.
An unavailable observation displays **Transaction state unknown**; a nonnull
transaction ID displays **Open transaction · usability unknown**. Neither
means rollback has happened or that an open transaction can still accept work.
Schema selection is disabled unless fresh idle authority is available;
transaction controls remain disabled. Closing an open
or unknown console requires confirmation and verification of remote state;
local closure does not promise immediate remote rollback. See the
[observer contract](snowflake-sdk-session-observation.md).

The helper's [schema-switch transport](snowflake-sdk-session-design.md) now has
local regression coverage, with a separate typed USE/readback operation and
retained acknowledgement on uncertain final context. Its guarded application
adapter, IPC and selector are locally qualified, including focused browser and
hidden native checks. Do not use the helper operation manually as a migration workaround;
it has no account qualification
or inherited authorization from the earlier read canary.

The SDK SQL route remains ReadOnly, and authoritative constraints, complete session
switching, writes, DDL, transactions and the required million-row result/export
workflow remain open. Keep the existing DataGrip connection available while the
required account workflows are implemented and accepted.

The SDK helper now guards query submission against automatic retransmission for
its exact reviewed dependency sources. It verifies those sources before import
and retires on a second or ambiguous submission. A lost acknowledgement can
still mean the server executed the first request: an unknown outcome is not a
failed query or rollback. Do not rerun uncertain work automatically. This guard
does not enable writes or qualify account transaction behavior. See the
[guard contract](snowflake-sdk-submit-guard-design.md) and current ledger.

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

Snowflake has read-only external-browser SDK and local-application OAuth routes,
with full external workflow qualification open;
Redis supports a bounded standalone command/key workflow, not Cluster, Sentinel,
streams or Pub/Sub. A required unsupported engine/authentication/operation is a
replacement blocker, not a reason to silently omit that daily workflow.

Record each required task, engine/version, authentication/transport, expected
result, observed behavior and accept/reject in the acceptance document. Ask for
missing workflows to be implemented before retiring DataGrip. No production
writes, paid services, publication or deployment are authorized by this guide.
