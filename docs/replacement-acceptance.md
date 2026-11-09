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

## Target definition and open questions

Replacement means the owner's required daily workflows succeed on their target
host, with reliable cancellation, transaction boundaries and local recovery.
Feature existence, unit tests, or this checklist alone do not establish readiness.
Full DataGrip parity is a separate, much broader claim and is not established.

Repository context suggests PostgreSQL, MySQL/MariaDB, standalone Redis and
Snowflake browser SSO. README documents password/TLS/SSH for SQL engines,
Snowflake read-only OAuth, and Redis limitations. These are candidate workflows,
not confirmation of the owner's requirements. Never read `.env`, OS keychains,
DataGrip settings or private database state to infer requirements.

Pending owner input (requested at Goal start):

- Required engines/versions, target OS and authentication (password, SSO, IAM,
  certificates, SSH, VPN), including whether Snowflake writes are essential.
- Essential daily read/write, transaction, import/export, schema, performance,
  saved-script, keyboard and accessibility workflows; representative sanitized
  examples and acceptable performance expectations.
- Whether pending edits/imports must survive concurrent migrations that drop,
  rename or recreate their target tables. Replacement on all three SQL engines is now a
  reproduced safety gap; see [investigation and repair design](table-identity-safety.md).

No unanswered feature is silently optional. Existing engine exclusions become
replacement blockers if the owner requires them. New engines, AI, collaboration,
visual designers and advanced administration are candidate enhancements only
after requirements are confirmed.

## Required acceptance gates

All rows are provisional requirements until the owner confirms applicability.
Each applicable engine/authentication pair needs its own evidence. Existing
historical evidence is linked in the engineering matrix; this run must record
fresh outcomes separately.

| Workflow | Observable acceptance | Initial state / remaining evidence |
|---|---|---|
| Connect securely | Create profile, connect, inspect identity/context; reject wrong CA/hostname/SSH trust; reconnect after interruption without replay | Implemented in bounded adapters; external auth/version combinations unconfirmed |
| Navigate metadata | Find table/view/routine, inspect columns, keys, DDL and dependencies; refresh after external DDL; handle denied privileges | Implemented with bounds; daily catalog/privilege qualification pending |
| Write SQL | Complete quoted names in active context; format without changing literals/binds; show Unicode diagnostic at correct statement | Bounded implementation; unsupported semantic forms remain explicit |
| Execute SQL | Current statement, selection, reviewed batch and typed parameters produce expected results; cancel long read and safely reconnect | 103 real fixtures and all 124 browser workflows pass within the ledger scopes; required external combinations remain open |
| Inspect and move data | Filter/sort/page, copy exact numeric/NULL values, export complete file; cancelled export clearly incomplete | Implemented with bounds; representative dataset sizes unconfirmed |
| Edit safely | Review SQL/bindings; edit/insert/delete disposable rows; conflict rejects stale edits atomically; ineligible results stay read-only | Provenance/snapshot/inspector repairs have real regressions; queued writes after closure and uncertain CSV reporting repaired. Permanent drop/recreate identity fails 27 actual fixture cases across PostgreSQL/MySQL/MariaDB, including CSV; repair remains incomplete |
| Transactions | Two consoles demonstrate shared versus isolated ownership; rollback leaves original rows; commit survives reconnect; cancellation reports unknown outcomes honestly | Manual/reviewed COMMIT uncertainty and closed-session cleanup repaired; 12 real transport tests, toolbar checks and four new grid/CSV fault browser cases pass |
| Reuse work | Save/reopen scripts and history according to opt-in policy; draft/profile/preferences survive backend/app restart without autoexecute/reconnect | Abrupt backend restart and two isolated actual native close/reopen pairs pass, including pending-save flush and real PostgreSQL rollback; normal packaged/credential evaluation remains open |
| Investigate performance | Explain appropriate engine query; actual execution requires review; cancel safely; compare compatible captures without rerunning SQL | Bounded plan implementation; Snowflake live qualification open |
| Change schema | Review generated migration, reject stale target snapshot, demonstrate engine-specific implicit commits and refresh metadata | Implemented SQL workflow; unsupported objects disclosed |
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
5. Permanent table identity investigation: a durable opt-in diagnostic reproduces
   27 failures across PostgreSQL/MySQL/MariaDB (nine each), including deletes,
   empty-result inserts and CSV across batch commits. No repair is claimed. The [design and reproducible steps](table-identity-safety.md)
   retain original-read, writer-target, privilege and CSV lifecycle requirements.
   Namespace/partition behavior and future guard lifecycle need separate qualification.
   Confirm the owner's concurrent-migration workflow before declaring applicability.
6. Pending owner input: confirm required engines/versions, authentication,
   transports, daily workflows, target hosts and performance/accessibility needs.
7. Use those answers to implement or qualify required gaps with isolated data;
   obtain explicit scope before external/production access or paid services.
   Unsupported required capabilities remain blockers, not optional omissions.
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
