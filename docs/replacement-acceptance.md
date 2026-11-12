# Daily DataGrip replacement acceptance

## Retained reviewed SQL console (12 November 2026, local integration)

The [retained-console walkthrough](snowflake-sdk-console-workflow.md) describes
the new opt-in mixed-SQL editor workflow and app-wide original receipts. Local
checks pass with an isolated synthetic SDK; full native application and account
acceptance remain open. The [ledger](replacement-verification.md) records source
and results separately from the preceding private compiler/adapter/journal work.

Require one physical session across separately reviewed SET/read,
SHOW/RESULT_SCAN, temporary-object and BEGIN/write/ROLLBACK/COMMIT runs, with no
automatic application SQL between them. Prove cancellation and result/recording
failures preserve original acknowledgements and stop the untouched suffix. A
lost start response must recover only by SQL-free correlation, without replay.
Rows must remain openable after editor closure while their process still owns
them; backend restart must restore honest SQL-free outcomes without signing in
or implying unavailable rows were empty. A received exact pre-admission refusal
may settle; a lost refusal still needs durable negative recovery. Configurable
long-running query behavior and practical journal retention are required.

Real Snowflake effects, one-million-row paging/export, normal packaged external-
browser SSO/restart and the owner's complete daily evaluation remain open. The
permitted disposable target is `SANDBOX.BILLIE_TESTING`; further account activity
still needs a fresh exact reviewed plan and possible-billing approval. Every
other applicable DataGrip capability remains in scope. The Goal is incomplete.

## Reviewed table DDL (11 November 2026, local integration)

The SDK external-browser route now includes a separate opt-in for generated
CREATE TABLE, nullable ADD COLUMN and DROP TABLE. Every statement uses its own
short-lived review and one-use authority; DROP also requires the exact fully
qualified target name. ReadOnly and OAuth deny this route. Admission requires a
coherent idle AUTOCOMMIT session, supported catalog traits and fresh context.
Permanent and transient ordinary base tables are supported; transient objects
have no Fail-safe. DDL cannot be rolled back and can race with external actors.
This is a bounded implemented subset, not complete Snowflake DDL parity.

The UI binds drafts and approval to the owning console, retains acknowledgement
separately from metadata reconciliation, pauses automatic observers, and requires
an explicit successful catalog/context refresh before normal metadata resumes.
Unknown delivery retires without replay. Original typed outcome receipts survive
a dropped caller; failed local history/audit recording blocks further DDL and
normal shutdown until a local-only retry succeeds. Abrupt loss while SQLite cannot
write is not claimed crash-durable. See the [DDL contract](snowflake-sdk-ddl-design.md),
[walkthrough](migration-from-datagrip.md#reviewed-sdk-table-ddl-prepared-acceptance-walkthrough)
and [current verification](replacement-verification.md).

The earlier approved database-creation setup was consumed: SSO and initial
observations passed, CREATE DATABASE was rejected, and no later schema/table SQL
was dispatched. The owner now permits newly created tables in
`SANDBOX.BILLIE_TESTING`. The [new lifecycle proposal](snowflake-existing-schema-qualification.md)
records the complete budget and remaining runner work. A fresh exact plan and
possible-billing approval are required before account activity; the prior setup must not be replayed. Actual
account writes/DDL/transaction effects, normal packaged SSO, million-row account
performance, broader DDL/grid/import/authoring workflows and hands-on acceptance
remain required. The Goal stays active and incomplete; nothing is merged,
published or installed.

## Recorded SQL-write increment (11 November 2026, sources `eae1e81` / `fb091db`)

The SDK external-browser route now implements a separately opted-in, individually
reviewed single INSERT/UPDATE/DELETE workflow with scalar parameters, exact status
counts, one-use submission authority and durable outcome history. It targets the
fully qualified SQL name shown in review; concurrent DDL, temporary-name shadows,
defaults, policies and dependencies can change resolution/effects. There is no
immutable object-identity or safe grid-edit claim. Acknowledged success is separate
from committed data effects. Unknown outcomes retire without replay; ordinary
observer denial retains rollback recovery. Metadata remains paused until an
explicit coherent refresh. ReadOnly and OAuth still deny writes.

Local commits `eae1e81` and `fb091db` pass full `pnpm validate`: 671 frontend
and 426 standard Rust tests, with 129 fixture tests explicitly ignored. Focused
checks pass 51 Rust DML cases, then 15 final Studio cases including the reproduced
and repaired graceful-shutdown history gap. All 21 Studio/actual-helper synthetic
SDK fixtures, seven browser DML/transaction flows and 43 actual disposable
PostgreSQL regressions pass. Helper coverage is 255 passing cases across the 107
actual-SDK loopback and 148 remaining-helper runs. The recorded DML-source normal
macOS bundle (224.33 MiB), strict ad-hoc signature/resources/startup checks and
three hidden native runs pass with zero startup authentication/SQL. See the
[verification ledger](replacement-verification.md) for source pins and limits.
These tests do not qualify actual Snowflake writes, transaction effects or a
million-row account result. Broader DML, grid editing, DDL, AUTOCOMMIT changes and
all remaining applicable DataGrip capabilities remain required gaps.

The owner requested a [disposable setup plan](snowflake-disposable-setup.md).
A separate helper-only runner now implements a fixed 28-SQL ledger with three
CREATEs and durable one-use receipts. The one approved account attempt is recorded above and in the ledger.
The original pure DDL syntax foundation has since gained the separately reviewed
application execution route described above. Actual account setup and mutation effects remain
required gates.
The Goal remains active and incomplete; nothing is merged, published or installed.

## Recorded transaction increment (10 November 2026, source `929ae06`)

Local source now includes separately reviewed SDK BEGIN/COMMIT/ROLLBACK controls,
one-use authority, policy rechecks and rollback-only recovery. SDK profiles still
default ReadOnly; only explicitly selected external-browser policies can permit
BEGIN/COMMIT. OAuth remains ReadOnly. On that recorded source, DML, DDL and AUTOCOMMIT
changes remained blocked. The UI retains command acknowledgement separately from transaction
activity and verified data effects, and pauses automatic metadata after a control.
See the [transaction contract](snowflake-sdk-transactions-design.md).

Full `pnpm validate` passes: 659 frontend and 384 Rust tests, formatting,
production frontend build, Rust checks and strict all-target/all-feature Clippy.
The standard run reports 129 fixture tests ignored. Separate checks pass 31
actual-helper/synthetic-SDK internal adapter tests, 31 adapter integration tests
and 16 Studio/helper fixtures; all six affected cases also pass against the final
source. Seven Studio policy/race tests and actual local SQLite profile restart/
backup round trips pass. Three transaction browser flows pass, including keyboard
approval and 31-second quiet-window checks; seven related UI cases passed in the
prior run. These UI tests intercept RPCs and do not qualify account behavior.

The rebuilt isolated SDK native app passes three runs, 20 check categories and
ten cleanup cycles each, including typed IPC admission guards. The normal SDK
macOS bundle passes packaging, strict ad-hoc signature/resource checks and guarded
SDK startup: 224.14 MiB, 15,106 resource entries, no symlinks, zero authentication
or SQL requests. All 324 frozen inputs stay unchanged. This source is committed
as `929ae06`; it is unmerged, unpublished and not installed. No account transaction
statements or writes are authorized or executed for this increment. Actual
account effects, Developer ID/notarization and owner acceptance remain open.

Started 9 November 2026. Goal remains **active, not accepted**. This document
coordinates the persistent Goal; the [engineering matrix](datagrip-feature-matrix.md)
retains historical evidence and the [roadmap](implementation-roadmap.md) retains
implementation history. The [official baseline](datagrip-official-baseline.md)
records current DataGrip research. No push, publication, deployment or merge is
authorized. The private hub is not updated because this task forbids publication.

Fresh commands, outcomes and qualification limits are recorded in the
[verification ledger](replacement-verification.md).
The [migration guide](migration-from-datagrip.md) provides build, connection,
script, transaction and recovery steps for hands-on evaluation.

## Confirmed owner target (9 November 2026)

The owner requires **Snowflake on macOS with SSO `authenticator=externalbrowser`**,
and **everything DataGrip currently provides for that workflow**, explicitly
including reads, writes, schema changes and multi-statement transactions.
The [Snowflake implementation queue](snowflake-datagrip-replacement.md) records
the required workflow inventory and qualification sequence.
This is not a read-only-pilot or MVP target. All applicable capabilities in the
official DataGrip baseline remain required until the owner explicitly excludes
one; existing exclusions are blockers, not optional enhancements.

The owner also requires the **latest stable DataGrip release** and Snowflake
query results up to **1,000,000 rows**. Current comparison: DataGrip 2026.3,
released 5 November 2026. Complete million-row paging/export with bounded memory
is required. The [opt-in temporary result store](large-results.md) now targets
one million rows with bounded memory/disk. Earlier actual PostgreSQL paging/export
and browser/native preference checks pass; the ledger identifies their source.
Actual Snowflake million-row behavior remains unqualified.

Explicit SDK external-browser profiles and retained helper lifecycle are now
implemented. New profiles select the SDK; existing profiles retain OAuth/SQL API.
Both restrict public SQL to reads; SDK typed boundary actions have a separate
policy. The preceding SDK integration passes 604 frontend / 297 standard
Rust tests; focused profile, actual-child lifecycle and browser checks pass.
All 124 actual fixtures and normal SDK resource-bundle/signature/startup checks
pass. All 143 browser workflows, 19 hidden SDK native checks with ten cleanup cycles
and two standalone PostgreSQL native recovery pairs pass.
See [SDK integration evidence](snowflake-sdk-integration.md). Actual Snowflake
account behavior, metadata, writes, DDL and transactions remain required and open.

The subsequent completion increment is locally qualified: 607 frontend /
312 standard Rust tests, 124 real-engine regressions, all 145 browser workflows,
fresh SDK-bearing packaging/startup and 19 hidden native check categories pass.
This provides same-session relation/column suggestions, not browsing or complete
inspection. Actual account metadata and the full required workflow remain open;
the [ledger](replacement-verification.md) records exact evidence and limits.

Later local source adds revision-bound relation browsing and the
[session observer](snowflake-sdk-session-observation.md): bounded same-connection
settings/activity/context observations with explicit Unknown state. Current
qualification is recorded in the ledger, separately from the earlier counts.
Guarded schema selection now has a typed adapter, IPC and selector; its application
qualification passes. Generic editable browsing, complete object inspection,
other session switches, write/DDL/transaction workflows and account/owner acceptance
remain required and open. Account scope must include automatic post-SQL observations and editor
metadata; the earlier two-query canary does not authorize them.

One separately approved helper-only 24-read/SHOW qualification attempt ended on
10 November at 12:31:29 UTC. SSO and its two startup checks passed; Snowflake
rejected the third session-context query. The runner stopped, acknowledged logout,
reaped the child and removed its own temporary authentication state. The approval
is consumed and was not retried. This failed attempt does not close account
observation, metadata, application or owner acceptance; the
[ledger](replacement-verification.md#approved-account-qualification-attempt-10-november-2026)
records its exact plan, scope and evidence.

A freshly approved corrected attempt passed at 12:59:21 UTC: all 24 fixed SQL
statements validated on the actual connection, with logout/reaping/private-state
cleanup. This qualifies helper context/settings, bounded schema discovery and
scalar results. It does not qualify normal application behavior, metadata beyond
that ledger, million-row results, writes, DDL, transactions or owner acceptance.
Both approvals are consumed. See the
[account pass](replacement-verification.md#corrected-account-readmetadata-qualification-10-november-2026).

## Target definition and open questions

Replacement means the owner's required daily workflows succeed on their target
host, with reliable cancellation, transaction boundaries and local recovery.
Feature existence, unit tests, or this checklist alone do not establish readiness.
Full DataGrip parity is a separate, much broader claim and is not established.

The owner's target above supersedes the initial repository-derived candidate
engine list. PostgreSQL, MySQL/MariaDB and Redis are secondary existing adapters;
they are not substitutes for required Snowflake behavior. No feature is optional
merely because it is currently unsupported or externally unqualified.

The owner authorized reusing the current Snowflake connection. Its allowlisted
non-secret account/role/warehouse/database/schema settings have been located in
an ignored owner-only repository cache; they need not be provided again. The owner
approved the bounded two-query read canary including possible billing. Its first
run authenticated and submitted both reads, then failed local validation. After
a reproduced validator-contract repair, the same-scope live rerun passed context,
session continuity, scalar values and cleanup. [The ledger](replacement-verification.md)
records both attempts. This does not authorize object reads or writes or qualify
packaged application sign-in.
The owner permits creating new test tables under `SANDBOX.BILLIE_TESTING`.
The earlier disposable-database setup approval was consumed by the privilege
rejection; no database/schema was created. The existing-schema table lifecycle
still needs a fresh concrete execution plan and one-attempt possible-billing
approval before account activity. The provided table scope need not be requested
again and does not authorize writes to existing application tables.

Remaining details: server version/environment,
warehouse policy,
representative sanitized tasks/column widths, CSV import sizes, and
accessibility/performance expectations. These refine acceptance;
they do not narrow the confirmed all-applicable-DataGrip target.

## Required acceptance gates

All DataGrip capabilities applicable to the confirmed Snowflake/macOS target are
required; rows below organize acceptance and are not an exhaustive parity claim.
Each applicable engine/authentication pair needs its own evidence. Existing
historical evidence is linked in the engineering matrix; this run must record
fresh outcomes separately.

| Workflow | Observable acceptance | Current implementation / remaining evidence |
|---|---|---|
| Connect securely | Existing Snowflake externalbrowser SSO; inspect identity/context; reject TLS/auth failures; reconnect deliberately without replay | SDK profile/lifecycle integration implemented with focused local evidence; actual externalbrowser account/native acceptance remains open |
| Navigate metadata | Find table/view/routine, inspect columns, keys, DDL and dependencies; refresh after external DDL; handle denied privileges | Partial Snowflake catalog/columns/generated DDL, declared PK/UQ/FK, [ordered column mappings and retained reference selection](snowflake-sdk-key-mappings-design.md) implemented locally. Reference-selection source/browser/hidden native and local package checks pass. Cross-database/inbound/related-row navigation, dynamic mappings, CHECK constraints, complete object explorer/dependencies and daily role/account qualification remain incomplete. See the source-specific [ledger](replacement-verification.md) |
| Write SQL | Complete quoted names in active context; format without changing literals/binds; show Unicode diagnostic at correct statement | Bounded implementation; unsupported semantic forms remain explicit |
| Execute SQL | Snowflake statement/selection/reviewed batch, typed parameters, writes and session commands produce expected outcomes; cancellation/interruption and reconnect are truthful | SDK reads, bounded completion/browsing, separately reviewed single SQL DML, typed boundaries and generated table DDL are integrated locally. Complete inspection, broader SQL/DDL/session behavior and actual account outcomes remain unqualified. Other-engine fixtures do not qualify Snowflake |
| Inspect and move data | Browse and export complete results up to 1,000,000 rows with bounded memory; filter/sort/page, copy exact values; cancelled export clearly incomplete | Opt-in temporary result storage passes actual PostgreSQL million-row paging/export and browser acceptance. Native preference/restart checks pass. Actual Snowflake types/transport/size, wide-result export and independent copy/import limits remain open; see [large results](large-results.md) |
| Edit safely | Review SQL/bindings; edit/insert/delete disposable rows; stale/replaced targets reject atomically; ineligible results stay read-only | PostgreSQL original relation identity and MySQL/MariaDB retained guards implemented. Final source checks pass 581 frontend / 193 Rust, 124 actual fixtures, 27/27 strict scenarios and 137/137 browser workflows, including exact CSV ownership and TLS-over-SSH. Snowflake grid/CSV editing is still disabled and requires separate identity proof. Normal packaged restart/credentials and owner applicability/acceptance remain open |
| Transactions | Snowflake consoles demonstrate shared versus isolated ownership; rollback leaves original rows; commit survives deliberate reconnect; DDL implicit commits and interrupted acknowledgements are truthful | SDK explicit controls and rollback recovery locally qualified on `929ae06`; see the verification ledger. Actual Snowflake data effects, mode changes, DML/DDL interaction and owner acceptance remain open. Other-engine tests cannot close this gate |
| Reuse work | Save/reopen scripts and history according to opt-in policy; draft/profile/preferences survive backend/app restart without autoexecute/reconnect | Abrupt backend restart and two isolated actual native close/reopen pairs pass, including pending-save flush and real PostgreSQL rollback; normal packaged/credential evaluation remains open |
| Investigate performance | Explain appropriate engine query; actual execution requires review; cancel safely; compare compatible captures without rerunning SQL | Bounded plan implementation; Snowflake live qualification open |
| Change schema | Review generated migration, reject stale target snapshot, demonstrate Snowflake implicit commits and refresh metadata | SDK reviewed CREATE TABLE, nullable ADD COLUMN and DROP TABLE are integrated locally with catalog holds and retained outcomes. Actual account DDL, schema diff/migration, broader objects and concurrent actor behavior remain unqualified; other-engine fixtures cannot close these gates |
| Everyday access | Complete daily workflow by keyboard; focus visible; dialogs traversable; required assistive technology usable | Keyboard tests exist; canvas accessibility/hands-on evaluation open |
| Run locally | Build normal app, verify bundle, launch using isolated app data, quit/relaunch; preserve profiles/preferences; prove supported update/install path | Normal local bundle/signature and hidden native smoke pass; packaged restart/install/signing/platform qualification open |

## Execution queue

1. Completed: official non-commercial baseline, original-source investigation,
   reproduced failures, reviewed provenance/snapshot/inspector/transaction
   repairs and focused regression coverage. Historical runs remain in the ledger.
2. Completed: repaired-source validation (542 frontend / 179 standard Rust),
   97 real fixture tests, normal macOS bundle/signature, three isolated native
   runs and 120/120 browser workflows. Runtime source is `e481e62`; subsequent
   documentation records evidence rather than changing application behavior.
3. Completed: reviewed isolated native close/cancel/reopen qualification
   (`a1ab58b`), two real PostgreSQL recovery pairs, six runner acceptance tests,
   ordinary smoke compatibility and full validation (542 frontend / 180 standard
   Rust). This is a native-test-only increment; normal packaged/credential gates
   remain open. See the ledger for pending-save timing and replay counters.
4. Reviewed-writer safety follow-up: synchronous PostgreSQL retirement, rejection
   of already queued writes after closure, truthful uncertain CSV batch reporting
   and frontend retirement have reproduced regressions and independent reviews.
   Validation (545 frontend / 180 standard Rust), 103 real fixtures and four new
   real browser workflows pass. Combined normal bundle/signature, two native
   recovery pairs, ordinary smoke and all 124 browser workflows pass. Local
   commits are `71d95b6` and `cd5bee0`; see the ledger for source/artifact scope.
5. Permanent table identity: the opt-in diagnostic on `ff59ca6` reproduced 27
   failures across PostgreSQL/MySQL/MariaDB. PostgreSQL original identity is
   committed; current MySQL/MariaDB retained guards pass preliminary integrated
   qualification (580 frontend / 193 Rust, 123 fixtures, 27 strict scenarios,
   137 browser workflows). Actual tokenless CSV reassociation was then reproduced
   and removed by requiring exact preview ownership at Rust and named IPC boundaries.
   Verified TLS-over-SSH guards/writers pass both engines. Code is locally
   committed as `997680e`; final source checks pass 581 frontend / 193 Rust,
   124 actual fixtures, 27 strict scenarios and all 137 browser workflows.
   Finish clean-source native/bundle qualification. Historical native evidence on `5762ee5` predates this increment.
   See [design and reproducible steps](table-identity-safety.md) and
   [retained-guard contract](mysql-table-identity-design.md).
6. Confirmed priority: Snowflake externalbrowser SSO on macOS, all applicable
   DataGrip capabilities, including writes/DDL/explicit transactions. Research
   official driver routes and implement stateful session ownership; the existing
   OAuth/SQL-API read-only pilot does not meet this requirement.
7. Qualify each required capability locally where possible, then against an
   explicitly authorized non-production Snowflake account. Obtain scope before
   external/billable work or writes. Missing live evidence remains blocked;
   synthetic transports never establish actual Snowflake behavior.
8. Pending owner evaluation: follow the concrete walkthrough below, including
   packaged native restart and credential behavior. Record acceptance per required
   workflow and resolve failures before retiring DataGrip.

## Migration and hands-on walkthrough

Keep DataGrip available until acceptance is resolved. Do not copy credential
stores or private settings automatically. Build/run with README commands; create
profiles manually, initially in ReadOnly mode. Re-enter secrets through the app.

1. On an isolated fixture, create a connection using the required transport;
   verify identity, role, database/schema and Safe Mode before opening work.
2. Find a familiar table and inspect DDL/keys. Open a SQL console; complete a
   qualified query, bind a parameter, run selection and a multiple-statement batch.
3. Filter/sort/page results; copy/export exact NULL, empty text and large numbers.
4. Review and apply a disposable grid edit, observing its separate writer.
   In a Manual console, execute an UPDATE, then Rollback and verify from another
   connection. Repeat with Commit. Demonstrate a concurrent grid conflict leaves
   no partial change.
5. Run/cancel a long read, reconnect and verify context. Explain a query; review
   before actual-plan execution. Never retry an uncertain write automatically.
6. Save a script; opt into drafts only if desired. Quit and relaunch using test
   app data; verify recovery without automatic database connection or execution.
7. Repeat the owner's normal daily tasks with keyboard/assistive tools and
   representative sanitized data. Record engine/version, elapsed time, missing
   behavior and explicit accept/reject for every required matrix row.

Acceptance is pending until the owner confirms suitability; external access,
signing and platform gates remain separate from tested local behavior.
