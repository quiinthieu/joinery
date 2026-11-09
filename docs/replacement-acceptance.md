# Daily DataGrip replacement acceptance

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
one million rows with bounded memory/disk. Actual PostgreSQL paging/export and
all 138 browser workflows pass; native preference/restart checks follow.
Actual Snowflake million-row behavior remains unqualified.

Current Joinery Snowflake uses local-application OAuth authorization-code/PKCE and
the SQL API, enforces ReadOnly, and does not implement a stateful externalbrowser
session with writes/transactions. Browser login alone does not prove equivalent
SSO setup or session behavior. These are primary replacement blockers. Local
PostgreSQL/MySQL safety repairs remain useful application work but do not close
Snowflake acceptance. External account qualification needs explicit non-production
scope and permission for any billable activity/writes; no private DataGrip state
or credentials will be inspected.

## Target definition and open questions

Replacement means the owner's required daily workflows succeed on their target
host, with reliable cancellation, transaction boundaries and local recovery.
Feature existence, unit tests, or this checklist alone do not establish readiness.
Full DataGrip parity is a separate, much broader claim and is not established.

The owner's target above supersedes the initial repository-derived candidate
engine list. PostgreSQL, MySQL/MariaDB and Redis are secondary existing adapters;
they are not substitutes for required Snowflake behavior. No feature is optional
merely because it is currently unsupported or externally unqualified.

Remaining details: Snowflake account/version/environment, role/warehouse policy,
representative sanitized tasks/column widths, CSV import sizes, accessibility/performance expectations,
and an authorized non-production qualification scope. These refine acceptance;
they do not narrow the confirmed all-applicable-DataGrip target.

## Required acceptance gates

All DataGrip capabilities applicable to the confirmed Snowflake/macOS target are
required; rows below organize acceptance and are not an exhaustive parity claim.
Each applicable engine/authentication pair needs its own evidence. Existing
historical evidence is linked in the engineering matrix; this run must record
fresh outcomes separately.

| Workflow | Observable acceptance | Initial state / remaining evidence |
|---|---|---|
| Connect securely | Existing Snowflake externalbrowser SSO; inspect identity/context; reject TLS/auth failures; reconnect deliberately without replay | Required externalbrowser path missing; current local-application OAuth pilot is not equivalent |
| Navigate metadata | Find table/view/routine, inspect columns, keys, DDL and dependencies; refresh after external DDL; handle denied privileges | Snowflake partial catalog pilot; required object explorer/DDL/dependency coverage and daily role qualification incomplete |
| Write SQL | Complete quoted names in active context; format without changing literals/binds; show Unicode diagnostic at correct statement | Bounded implementation; unsupported semantic forms remain explicit |
| Execute SQL | Snowflake statement/selection/reviewed batch, typed parameters, writes and session commands produce expected outcomes; cancellation/interruption and reconnect are truthful | Read-only SQL API pilot; required stateful write/transaction path missing. Other-engine fixture evidence does not qualify it |
| Inspect and move data | Browse and export complete results up to 1,000,000 rows with bounded memory; filter/sort/page, copy exact values; cancelled export clearly incomplete | Opt-in temporary result storage passes actual PostgreSQL million-row paging/export and browser acceptance. Native preference/restart checks follow. Actual Snowflake types/transport/size, wide-result export and independent copy/import limits remain open; see [large results](large-results.md) |
| Edit safely | Review SQL/bindings; edit/insert/delete disposable rows; stale/replaced targets reject atomically; ineligible results stay read-only | PostgreSQL original relation identity and MySQL/MariaDB retained guards implemented. Final source checks pass 581 frontend / 193 Rust, 124 actual fixtures, 27/27 strict scenarios and 137/137 browser workflows, including exact CSV ownership and TLS-over-SSH. Snowflake grid/CSV editing is still disabled and requires separate identity proof. Normal packaged restart/credentials and owner applicability/acceptance remain open |
| Transactions | Snowflake consoles demonstrate shared versus isolated ownership; rollback leaves original rows; commit survives deliberate reconnect; DDL implicit commits and interrupted acknowledgements are truthful | Required Snowflake stateful transaction path missing. Fifteen local transaction transport tests qualify existing SQL adapters only |
| Reuse work | Save/reopen scripts and history according to opt-in policy; draft/profile/preferences survive backend/app restart without autoexecute/reconnect | Abrupt backend restart and two isolated actual native close/reopen pairs pass, including pending-save flush and real PostgreSQL rollback; normal packaged/credential evaluation remains open |
| Investigate performance | Explain appropriate engine query; actual execution requires review; cancel safely; compare compatible captures without rerunning SQL | Bounded plan implementation; Snowflake live qualification open |
| Change schema | Review generated migration, reject stale target snapshot, demonstrate Snowflake implicit commits and refresh metadata | Existing other-engine workflow does not qualify Snowflake; required Snowflake DDL/schema diff/object coverage incomplete |
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
