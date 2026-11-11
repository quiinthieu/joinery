# Snowflake qualification in an existing disposable schema

Prepared 11 November 2026. **Proposal only: unexecuted and not executable until
a new runner, its regression tests, and current source/runtime pins are reviewed.**
The owner permits creating fresh tables under `SANDBOX.BILLIE_TESTING`. That
target permission persists; it does not supply the new SQL, metadata-disclosure,
authentication, transaction, DROP, or possible-billing approval described here.
No account connection, credential access, or account SQL was used to prepare this
document. No approval question is pending while the executable boundary is absent.

The harness-only [offline definition](../src-tauri/src/drivers/snowflake_table_qualification.rs)
and its [tests](../src-tauri/src/drivers/snowflake_table_qualification_tests.rs)
now implement the generated target, production-compiled DDL/DML, ordered phases,
typed outcome/state/readback validators and bounded census expansion. Fourteen focused
Rust tests pass; an independent bounded source review found no material issue in
this offline scope. The snapshot hash detects changes to this definition; it is
not an executable approval manifest. The later
[attachment increment](snowflake-qualification-attachments.md) implements physical
admission/resource plumbing and validated raw census/creation-marker observation
hooks. The [shared SQL contract and offline ledger](snowflake-qualification-sql-ledger.md)
now export bootstrap/typed SQL and expand exact physical statement order/bounds.
The later owning policy implements physical ledger enforcement, frozen lifecycle
observations and durable intent/original-outcome/phase receipts, with local fault
and synthetic composition coverage. The full 32-phase Studio/adapter/helper route
is now composed in a synthetic-only harness; initial S=1/32 success runs match
393/610 SQL requests. The first expanded qualification run passes 119/122; focused
reruns pass success/cancellation. The post-ACK storage-failure case initially
closes early; a separately reproduced finalization race is repaired and that
case now passes, as do all 122 qualification and 131 process/adapter tests. Strict
Rust checks pass separately; both earlier full application runs fail the recorded
PostgreSQL startup timeouts. A subsequent timing reproduction and bounded repair
pass the normal parallel protocol target and 45 real PostgreSQL/SSH/TLS fixtures.
Two new tests pass all 14 actual helper-child-exit scenarios across the seven
mutations, before and after ACK, preserving original outcomes without SQL replay.
The complete combined-source qualification module passes all 124 tests.
See the verification ledger for current full validation and failed attempts.
Remaining full-route census/context drift and final integrated fault verification,
source/runtime/private connection binding and the account entry remain incomplete.
No normal desktop command exposes this module.
See the
[verification ledger](replacement-verification.md) for checks and remaining gates.

The earlier [database setup attempt](snowflake-disposable-setup.md) stopped on
the definitive `CREATE DATABASE` rejection at 05:41:35.426 UTC. Its plan hash and
run token remain consumed. Do not retry it, edit its target, fall back through
that runner, or reuse its approval. The existing setup runner admits only its
closed 28-statement database/schema/table workflow. The application’s reviewed
DDL support does not turn that helper-direct runner into an application runner.

## Target and operating conditions

One new table is sufficient for this first complete lifecycle. A new offline
compiler generates a cryptographically random, nonzero, 32-character uppercase
hexadecimal `RUN` and the sole table name `JOINERY_QUAL_<RUN>`. In this document,
`T` means that exact identifier value, and `F` means the independently quoted
three-part name `"SANDBOX"."BILLIE_TESTING"."T"`. Neither is an SQL fragment or
a caller-selected object. The private manifest replaces the placeholders once.

Use one fresh external-browser SDK session against the existing account/host,
username, primary role and warehouse. The connection’s initial database/schema
must be `SANDBOX`/`BILLIE_TESTING`; validate these as connection options and then
against startup observations. If the saved connection names another context,
prepare a separately reviewed qualification connection before manifest emission;
the runner never edits the working profile, borrows a working console, or issues
`USE`. Preserve the existing role, warehouse and secondary-role configuration.
Pin the returned principal/session and all sampled settings/search-path context.

The qualification profile must already satisfy the normal production policy:
writable Safe Mode, explicit reviewed DDL and DML opt-ins, and the usual plan
authorization. The first runner increment may support Confirm mode only and fail
offline for other modes; it must not silently lower a policy or bypass a required
passphrase. Profile/policy changes revoke the attempt’s pending plans. The wider
Safe Mode matrix remains covered separately by local tests and later acceptance.

Require the existing schema to be available, every DDL phase to observe idle
AUTOCOMMIT=true state, and quoted-identifier policy false. Observe rather than
change AUTOCOMMIT and TRANSACTION_ABORT_ON_ERROR. Missing privilege, hidden
metadata, conflicting names, unexpected settings or unsupported object traits
stop the run. No grants, ownership transfers, session-parameter changes, warehouse
commands, database/schema creation, temporary objects, or alternate target are
allowed. CREATE itself remains the privilege check; no unbudgeted privilege
enumeration is implied.

Reserve the generated name exclusively for this attempt. The owner must exclude
other actors from that name and from concurrent DDL in `BILLIE_TESTING` during
qualification. Require the exact new CREATE acknowledgement before later work,
then verify its owner equals the pinned primary role and its catalog traits and
creation marker remain consistent, following Snowflake's
[primary-role ownership rules](https://docs.snowflake.com/en/user-guide/security-access-control-overview).
Names, timestamps, owner metadata and
matching column shapes do not establish immutable identity or eliminate
replacement/temporary-shadow races. If exclusive use cannot be established,
leave this proposal blocked rather than adopt an existing relation.

## Closed statement templates

The three DDL actions use the production generated-action compiler and dedicated
review/authorization/execute route. The emitted canonical SQL, including spacing,
is frozen in the private manifest; no arbitrary SQL field or parser fallback is
added to qualification. All initial columns are nullable. ADD has no default,
constraint, expression, backfill or NOT NULL option.

```sql
CREATE TABLE "SANDBOX"."BILLIE_TESTING"."T" (
  "ID" NUMBER(38, 0), "NOTE" VARCHAR(128), "ACTIVE" BOOLEAN
);
ALTER TABLE "SANDBOX"."BILLIE_TESTING"."T" ADD COLUMN "EXTRA" VARCHAR(32);
DROP TABLE "SANDBOX"."BILLIE_TESTING"."T" RESTRICT;
```

The compiler may render these single-line statements without a trailing
semicolon. The manifest must contain its actual byte-for-byte output, never the
illustrative line wrapping. DROP additionally requires the independent exact
target confirmation `"SANDBOX"."BILLIE_TESTING"."T"`, after substitution, plus
any normal connection-policy confirmation. No IF EXISTS, IF NOT EXISTS,
OR REPLACE, CASCADE, CREATE AS SELECT, CLONE or retention-setting syntax is added.
The table can inherit transience from its existing parent. The runner must admit
only known non-temporary BASE TABLE traits, with a known permanent/transient
value, and report the resulting recovery/storage implications.

Use these four exact, fully qualified DML statements, each once, through the
production DML compiler and dedicated reviewed route. There are no parameters
or bind values in this first ledger; literals are fixed synthetic values.

```sql
-- I1: inside the transaction that will be rolled back
INSERT INTO "SANDBOX"."BILLIE_TESTING"."T" ("ID", "NOTE", "ACTIVE")
VALUES (1, 'rollback-only', TRUE);

-- I2: inside a distinct transaction that will be committed
INSERT INTO "SANDBOX"."BILLIE_TESTING"."T" ("ID", "NOTE", "ACTIVE")
VALUES (2, 'committed', TRUE);

-- U1 and D1: subsequent AUTOCOMMIT writes on this same synthetic row
UPDATE "SANDBOX"."BILLIE_TESTING"."T"
SET "NOTE" = 'updated', "ACTIVE" = FALSE WHERE "ID" = 2;
DELETE FROM "SANDBOX"."BILLIE_TESTING"."T" WHERE "ID" = 2;
```

Freeze the compiler’s canonical/wire SQL and empty bind vectors in the manifest.
Require acknowledged statement outcomes and an available affected-row summary
of exactly one for each DML statement; status-row counts do not substitute for
affected rows. Do not replay a statement if summary retrieval or validation fails.

The only transaction SQL is two typed `BEGIN` operations, one typed `ROLLBACK`
and one typed `COMMIT`, at the ordered phases below. Use freshly observed native
transaction revisions and normal typed plan authorization. Require BEGIN to
observe an active transaction, each in-transaction DML/read observer to retain
that exact transaction, and each terminal boundary to observe idle state. Keep
the configured AUTOCOMMIT value unchanged. Boundary acknowledgement, observed
state and verified row effects are distinct evidence; DDL is never inside these
transactions. Snowflake documents the independent commit behavior of DDL in
[Transactions](https://docs.snowflake.com/en/sql-reference/transactions).

One ownership readback, through the normal read route, is fixed to the new table.
The selected traits follow the official
[TABLES view reference](https://docs.snowflake.com/en/sql-reference/info-schema/tables)
(rechecked 11 November 2026):

```sql
SELECT TABLE_OWNER, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY,
       CREATED::VARCHAR AS CREATED
FROM "SANDBOX".INFORMATION_SCHEMA.TABLES
WHERE TABLE_CATALOG = 'SANDBOX' AND TABLE_SCHEMA = 'BILLIE_TESTING'
  AND TABLE_NAME = 'T'
ORDER BY TABLE_NAME LIMIT 2;
```

Substitute only the generated name’s escaped literal. Require one row, pinned
owner, BASE TABLE, non-temporary status, known transience and a nonempty creation
marker. The six data readbacks all use this single fixed SELECT, with unchanged
wire SQL and no binds:

```sql
SELECT "ID", "NOTE", "ACTIVE", "EXTRA"
FROM "SANDBOX"."BILLIE_TESTING"."T" ORDER BY "ID" LIMIT 2;
```

Require the exact column/types/nullability contract, no extra rows and the
expected contents in the phase table. Two-row bounds detect an unexpected
second row; the isolated fixture contains at most one row. No data read,
DESCRIBE or zero-row schema-resolution query may name an existing table. All
column inspection and `SELECT * ... LIMIT 0` adapter work names only `F`.

## Metadata expansion and complete SQL budget

The production adapter has broader metadata prerequisites than the new table’s
data readbacks. `relation_catalog()` lists every visible non-INFORMATION_SCHEMA
schema in the pinned current database and reads relation names, kinds, traits,
creation and last-DDL markers once per returned schema. It currently cannot
restrict that catalog to `BILLIE_TESTING`. Explicitly disclose and approve this
bounded metadata visibility across `SANDBOX`; table-creation permission alone
does not cover it. No other database, existing table data, existing column
inspection, grants inventory or account-wide object inventory is included.

For this new workflow, propose **S <= 32** visible user schemas. The first schema
list, during CREATE preflight, freezes their exact ordered set, must contain
`BILLIE_TESTING`, and must pass the bound before CREATE can be admitted. Every
subsequent schema list must match it exactly. The physical SQL gate admits
per-schema catalog templates only for that frozen set, once per scheduled
catalog pass. An overflow or changed set stops the attempt; never widen the cap
or derive another target under its approval. This is a new runner restriction,
not a claim that the production adapter’s existing 1,000-schema bound is 32.

The private manifest must expand fixed SQL and closed generation rules from the
pinned production functions, not accept a runtime SQL string. Its allowed
families are:

| Family                      | Production derivation and bounds                                                                                                   |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Startup                     | One six-field identity/context SELECT and `SELECT CURRENT_VERSION()`                                                               |
| Observation sample          | Five fixed reads: extended context, quoted-identifier policy, AUTOCOMMIT, abort-on-error, current transaction                      |
| Session/options             | Sample + current-database schema list + sample = 11 reads                                                                          |
| Catalog                     | Two-read context/policy before and after; one schema list; one relation list per schema = `5 + S` reads                            |
| New-table inspection        | Two-read context/policy twice; exact relation check twice; columns, DESCRIBE and zero-row resolution = 9 reads                     |
| DDL target checks           | CREATE schema/collision reads = 2; ADD exact relation/columns/DESCRIBE/resolution/relation reads = 5; DROP exact relation read = 1 |
| Reviewed DML                | Five-read sample + one DML + five-read sample = 11 SQL executions                                                                  |
| Typed boundaries            | BEGIN/COMMIT: sample + boundary + sample = 11; ROLLBACK: boundary + sample = 6                                                     |
| Normal Studio read          | One exact SELECT + the automatic two-sample runtime observation = 11 reads                                                         |
| Coherent DDL reconciliation | Session/options + catalog + session/options = **`27 + S` reads**                                                                   |

The five-read sample is exactly the current constants in
[metadata](../src-tauri/src/drivers/snowflake_metadata.rs) and
[session observation](../src-tauri/src/drivers/snowflake_session.rs). Schema and
relation lists use the exact templates in
[browsing](../src-tauri/src/drivers/snowflake_browsing.rs), with database fixed to
SANDBOX and schema names obtained only from the bounded census. Preserve their
sentinels and parser checks: at most 10,000 total catalog relations/2 MiB, at
most 1,000 inspection columns/1 MiB, exact relation lookups bounded to three
rows, and one-row/64 KiB session samples. This run’s table inspections must
resolve exactly three then four columns. The schema-list SQL still has its
production 1,001-row sentinel; the qualification gate additionally rejects more
than 32 returned schemas before authorizing downstream SQL. Every fixed SQL
string, generated template, substitution rule and maximum invocation count must
be available privately before approval. Server-returned schema names cannot be
rendered offline: record each permitted expansion privately before its physical
write, under the already approved bounded rule, rather than accepting SQL input.

The complete successful path below assumes that stable schema set. An error can
consume a prefix only; no diagnostic, recovery or cleanup SQL is added afterward.
Prepare/authorize/local persistence and result-stream credits add zero SQL.

| Phase     | Action and required result                                     |              SQL executions |
| --------- | -------------------------------------------------------------- | --------------------------: |
| 1         | Fresh connection: identity and version, exact expected context |                           2 |
| 2         | CREATE preflight: schema available, fresh name absent, idle    |                          12 |
| 3         | Reviewed CREATE, acknowledged success                          |                          18 |
| 4         | Explicit coherent catalog reconciliation                       |                    `27 + S` |
| 5         | New-table ownership/creation/traits readback                   |                          11 |
| 6         | Fresh catalog; select only exact new table for ADD             |                     `5 + S` |
| 7         | Inspect new table: exact three nullable columns                |                           9 |
| 8         | ADD preflight with current catalog and inspection revisions    |                          15 |
| 9         | Reviewed nullable ADD, acknowledged success                    |                          21 |
| 10        | Explicit coherent catalog reconciliation                       |                    `27 + S` |
| 11        | Fresh catalog; select only exact new table for shape check     |                     `5 + S` |
| 12        | Inspect: original three plus nullable EXTRA VARCHAR(32)        |                           9 |
| 13        | Data readback: no rows; publishes fresh idle authority         |                          11 |
| 14        | Typed BEGIN A; observe active transaction A                    |                          11 |
| 15        | Reviewed I1; affected rows 1; still transaction A              |                          11 |
| 16        | Readback: `(1, rollback-only, true, NULL)` in A                |                          11 |
| 17        | Typed ROLLBACK A; observe idle                                 |                           6 |
| 18        | Readback: no rows                                              |                          11 |
| 19        | Typed BEGIN B; observe active transaction B                    |                          11 |
| 20        | Reviewed I2; affected rows 1; still transaction B              |                          11 |
| 21        | Typed COMMIT B; observe idle                                   |                          11 |
| 22        | Readback: `(2, committed, true, NULL)`                         |                          11 |
| 23        | Reviewed U1; affected rows 1; remain idle AUTOCOMMIT           |                          11 |
| 24        | Readback: `(2, updated, false, NULL)`                          |                          11 |
| 25        | Reviewed D1; affected rows 1; remain idle AUTOCOMMIT           |                          11 |
| 26        | Readback: no rows                                              |                          11 |
| 27        | Explicit session/options metadata resume after DML hold        |                          11 |
| 28        | Fresh catalog; select only exact new table for DROP            |                     `5 + S` |
| 29        | DROP preflight: fresh exact target, idle                       |                          11 |
| 30        | Reviewed exact-target DROP RESTRICT, acknowledged success      |                          17 |
| 31        | **Final explicit coherent catalog reconciliation**             |                **`27 + S`** |
| 32        | Fresh catalog; require exact generated name absent             |                     `5 + S` |
| **Total** | One session; three DDL, four DML, four boundaries              | **`386 + 7S`, at most 610** |

The 610 cap includes startup, every target recheck, both inspections, seven
normal readbacks and all their runtime observers, three coherent refreshes, and
four additional catalog requests. A reconciliation returns session options,
not the public catalog DTO; do not assume it supplies the later ADD/DROP target
IDs or final absence assertion for free. Each DDL execute budget includes its
last pre-dispatch five-read sample and its final five-read sample. Each readback
waits for its automatic runtime observer and verifies coherent state before
continuing. Freeze these counts in tests against the actual production route;
any extra startup/health/completion query or changed cadence invalidates the
plan. At S=32 the cap is 599 reads plus 11 DDL/DML/boundary statements.

## Failure, DROP and evidence boundaries

Never advance to a later phase after a rejected/unknown operation, inconsistent
result, context or policy drift, denied/overflowing metadata, expired grant,
admission deadline, or failed durable receipt. The active production call may
finish only its frozen, already-budgeted read-only post-observation suffix,
including after a statement rejection. Its SQL/deadline bounds still apply;
these reads are not a new diagnostic branch or a retry. Validate the completed
call and sync its original receipt before any phase advancement. Never retry,
reconnect-and-replay, poll for completion, perform automatic rollback, or continue
to DROP after a failed phase. Planned ROLLBACK A is a qualification step, not
generic failure cleanup. A possibly submitted mutation permanently consumes that
phase and the run; no new approval may adopt its same table token for another try.

DROP is authorized only as phase 30 of a fresh fully approved ledger and only
for the table whose CREATE receipt and following checks passed. It is not
general cleanup permission. Do not drop the schema/database, an existing object,
a name/prefix match, or any object after uncertain identity/delivery. RESTRICT
does not prove dependency absence or object identity. On failure, preserve the
remaining new table and privately report the exact known/unknown state for owner
intervention; this document supplies no later cleanup execution permission.

Persist a synced intent before each physical SQL write. Retain known
DDL/DML/boundary acknowledgements during the active call, and sync the original
receipt before admitting any later phase, including its first SQL. Preserve the first
failure and known statement outcomes if observers, IPC delivery, audit/history,
journal close or final directory sync fail. Local pending-receipt retries may
write only original local evidence; graceful shutdown must report failure while
that evidence remains unrecorded. They never repeat remote SQL. Forced process
loss and unrecoverable storage failure remain risks, not claimed durability.

Keep credentials, SSO URLs, private connection values, returned schema/relation
names, raw rows/errors and session/query IDs out of shared logs. The owner-private
manifest/journal may contain the rendered identifiers and minimal typed evidence
needed to correlate this run, but no credential. Reports expose safe hashes,
phase/count bounds, typed outcomes, validation booleans and cleanup status. Final
qualification requires source revalidation and confirmed durable final receipt;
an ACK, catalog visibility and full workflow pass remain distinct. Same-session
readbacks do not establish independent-session visibility or remote exactly-once
effects. This initial lifecycle does not qualify cancellation/fault injection,
large results, all DataGrip workflows, the packaged app, or owner replacement
acceptance.

## Billing and fresh approval boundary

The new approval must name one exact private execution-plan hash, newly generated
table, account/connection alias, pinned principal/role/warehouse/context, S<=32
metadata disclosure, all 32 phases and the 610-SQL maximum. It must expressly
cover external-browser SSO, possible warehouse auto-resume, cloud services and
storage/retention charges, the four typed transaction boundaries, and the
single conditional end-of-successful-run DROP. It grants no second attempt,
existing data access or later cleanup. Old canary/read/setup approvals remain
consumed; consent to fresh tables alone is not this execution/billing grant.
Explicitly disclose the active call's bounded read-only post-observation suffix
after rejection; no later phase or mutation is permitted after that failure.

No resource settings are changed to control spend. Snowflake bills warehouse
compute per second with a 60-second minimum when it starts; metadata/readbacks
can resume the selected warehouse. See
[compute costs](https://docs.snowflake.com/en/user-guide/cost-understanding-compute).
The plain CREATE inherits the existing schema/database’s permanence or transience;
permanent tables can retain Time Travel/Fail-safe storage after DROP, while
transient tables have narrower recovery and no Fail-safe. See
[table types and recovery](https://docs.snowflake.com/en/user-guide/tables-temp-transient).
Neither an SQL cap nor a local deadline is a currency ceiling, guarantee of
immediate purge, HTTP-request cap or proof that remote work stopped. Cost-query
SQL, a second observer session and warehouse inspection are not in this ledger.

After the implementation below passes review, emit/verify a new owner-private
0600 manifest under the ignored 0700 `.cache/snowflake-qualification` root. The
future approval must bind a proposed 30-minute UTC admission window, a 15-minute
monotonic workflow deadline, 120-second authentication wait and 30-second SQL
admission waits; preserve stricter production operation limits. Source/runtime
and window checks must occur again before authentication and each SQL admission.
Use the actual implemented/tested limits in the final manifest; changes require
a new review and hash. Bounded close/EOF/TERM/KILL and actual local reap are
allowed local lifecycle cleanup, with zero extra SQL and no remote rollback
guarantee. Authentication, renewal, result fetch and logout network work remain
separately disclosed. Do not ask for execution approval until those concrete
artifacts and checks exist.

## Smallest complete runner increment, separately implemented

Keep the existing read and disposable-database setup workflow identifiers,
ledgers and consumed markers unchanged. Add a distinct
`existing-schema-table-lifecycle-v1` workflow and approval format; no `--sql`,
`--schema`, fallback target or unrestricted helper operation is introduced.
The minimum complete increment has these parts:

1. **Offline compiler.** Generate the sole name and canonical action/read/DML
   SQL from the closed definitions above; include ordered phases, exact empty
   binds, typed validators, bounded census expansion, caps and disclosures.
   Dry-run/emission/verification never spawn a helper, allocate auth state,
   authenticate or execute SQL. Proposed result caps are two rows/64 KiB for
   fixed readbacks, existing production metadata bounds, 32 MiB aggregate result
   content, 64 MiB physical inbound bytes, 1 MiB/frame, 32,768 frames and 64 KiB
   counted stderr. The reviewed runner must enforce rather than merely print
   these caps.
2. **One fixed Rust application-core qualification entry point.** Drive the
   production Studio/SnowflakeExternal session, DDL preflight/prepare/dedicated
   authorization/execute, DML, typed boundaries, read event completion and
   explicit metadata reconciliation in the exact order above. Keep UI health,
   automatic completion, other consoles and background catalog work disabled
   for this owned run. No direct helper setup commands substitute for these
   paths. This qualifies the recorded application-core/adapter path; a normal
   packaged UI run remains separate.
3. **Physical admission and durable ownership.** A qualification-only policy
   at the production SnowflakeProcess write boundary reserves/checks the next
   permitted SQL/template expansion and syncs its intent before writing. It
   rejects unexpected operation kinds, bytes, binds, schema/object identities,
   duplicate/extra SQL, excess counts or overlap. It does not rely solely on a
   command’s nominal budget. Startup and automatic observers pass the same gate.
   Permit only the active call's fixed observer suffix after rejection, never
   phase advancement. Add a narrow callback after production schema-list parsing
   to freeze/compare S<=32; CREATE preflight does not expose that list publicly.
   Count inbound frames/bytes/results/stderr at their receiving paths. A general
   inbound semantic/journal gate is unnecessary under this bounded suffix policy.
   The retained workflow owner validates the completed call and syncs its original
   receipt before any later phase. Reuse the established one-submission SDK guard;
   do not add a generic classification bypass. DML/transaction terminal local
   failures must be visible and retained rather than discarded. The owner task
   outlives dropped callers and drains receipts before graceful shutdown succeeds.
4. **Execution envelope and local lifecycle.** Reuse the established private
   file, exclusive marker, hash-chained journal and identity-checked cleanup
   rules where suitable, without pretending the old helper-direct child driver
   already drives Rust’s helper ownership. Consume both the execution-plan hash
   and account/host/database/schema/run-token key durably before authentication,
   even if local launch/SSO later fails. Bind dirty-source content hashes, not
   HEAD alone: the new compiler/runner/gate/validators, all relevant Rust
   production code and Cargo.lock/build features, the actual qualification
   executable, helper sources, verified Node binary, SDK/guard dependency tree,
   runtime resource manifest, policy/profile digest and workflow templates.
   Reject source/binary/resource drift, symlinks, permissive files and reused
   output/approval markers; revalidate after execution while retaining outcomes.
5. **Meaningful local regressions before any account approval.** Run the full
   32-phase production-adapter workflow against a synthetic SDK subprocess at
   S=1 and S=32; assert exact SQL bytes/counts `393` and `610`, final refresh
   `28` and `59`, typed row/state assertions, one execution of each mutation,
   final absence and zero unlisted SQL. Test S=33 rejection before CREATE,
   schema-set drift, collision/owner/traits/shape mismatch, wrong role/context,
   missing opt-ins, ReadOnly, generic-route bypass, changed SQL/binds, expiry,
   reused tokens and tampered sources/runtime. Include rejection and lost-ACK
   stops at every mutation, allowing exactly the active call's approved read-only
   suffix but zero later-phase SQL. Test final-observer/catalog failure preserving ACK,
   dropped waiters, history/audit/journal/final-sync failure, local-only receipt
   retry, failed launch, cleanup identity replacement, actual reaping, redaction
   and offline modes having zero account side effects. Run relevant repository
   checks and independently review the complete manifest and guard placement.

Implemented prerequisites now include the offline definition/ledger, physical
admission and original completion observation, frozen lifecycle/runtime policy,
and private durable journal barriers. The same-call Studio reconciliation bridge
retains original context options and catalog without a second catalog request.
The synthetic-only runner now composes all 32 actual Studio/adapter/helper phases.
Final fault checks and real wire interruption qualification remain open. A closed
account entry and source/runtime/profile execution envelope remain required before
an executable approval plan can be prepared.
No new execution manifest, approval file, authentication, account test, cleanup
or account qualification is claimed. See the current verification ledger for
the scope and results of each implemented prerequisite.
