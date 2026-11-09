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
| Execute SQL | Current statement, selection, reviewed batch and typed parameters produce expected results; cancel long read and safely reconnect | Implemented; fresh real-engine/browser run pending |
| Inspect and move data | Filter/sort/page, copy exact numeric/NULL values, export complete file; cancelled export clearly incomplete | Implemented with bounds; representative dataset sizes unconfirmed |
| Edit safely | Review SQL/bindings; edit/insert/delete disposable rows; conflict rejects stale edits atomically; ineligible results stay read-only | Implemented; focused correctness review in progress |
| Transactions | Two consoles demonstrate shared versus isolated ownership; rollback leaves original rows; commit survives reconnect; cancellation reports unknown outcomes honestly | Implemented SQL sessions; actual engine tests required |
| Reuse work | Save/reopen scripts and history according to opt-in policy; draft/profile/preferences survive backend/app restart without autoexecute/reconnect | Persistence implementation exists; page reload alone insufficient |
| Investigate performance | Explain appropriate engine query; actual execution requires review; cancel safely; compare compatible captures without rerunning SQL | Bounded plan implementation; Snowflake live qualification open |
| Change schema | Review generated migration, reject stale target snapshot, demonstrate engine-specific implicit commits and refresh metadata | Implemented SQL workflow; unsupported objects disclosed |
| Everyday access | Complete daily workflow by keyboard; focus visible; dialogs traversable; required assistive technology usable | Keyboard tests exist; canvas accessibility/hands-on evaluation open |
| Run locally | Build normal app, verify bundle, launch using isolated app data, quit/relaunch; preserve profiles/preferences; prove supported update/install path | Local packaging pending; signing/notarization/platform qualification open |

## Execution queue

1. Establish official non-commercial baseline; confirm owner requirements.
2. Requalify combined `bcb606f` source: baseline validate, real PostgreSQL,
   MySQL/MariaDB/Redis/SSH fixtures, full browser suite, normal bundle and hidden
   native smoke. Record failures before fixing; do not count ignored tests as run.
3. Review/fix query-result editing correctness, then rerun affected checks and
   request independent review of substantial changes.
4. Add missing daily workflow acceptance coverage where it tests real behavior.
5. Prioritize uncovered required features using owner answers and measured
   failures. Advance independent safe work while external qualification waits.
6. Prepare the migration walkthrough below and obtain owner hands-on acceptance.

## Migration and hands-on walkthrough

Keep DataGrip available until acceptance is resolved. Do not copy credential
stores or private settings automatically. Build/run with README commands; create
profiles manually, initially in ReadOnly mode. Re-enter secrets through the app.

1. On an isolated fixture, create a connection using the required transport;
   verify identity, role, database/schema and Safe Mode before opening work.
2. Find a familiar table and inspect DDL/keys. Open a SQL console; complete a
   qualified query, bind a parameter, run selection and a multiple-statement batch.
3. Filter/sort/page results; copy/export exact NULL, empty text and large numbers.
4. On disposable rows, review an edit and roll it back in a manual console.
   Commit a separate edit and verify from a fresh connection. Demonstrate a
   concurrent conflict leaves no partial change.
5. Run/cancel a long read, reconnect and verify context. Explain a query; review
   before actual-plan execution. Never retry an uncertain write automatically.
6. Save a script; opt into drafts only if desired. Quit and relaunch using test
   app data; verify recovery without automatic database connection or execution.
7. Repeat the owner's normal daily tasks with keyboard/assistive tools and
   representative sanitized data. Record engine/version, elapsed time, missing
   behavior and explicit accept/reject for every required matrix row.

Acceptance is pending until the owner confirms suitability; external access,
signing and platform gates remain separate from tested local behavior.
