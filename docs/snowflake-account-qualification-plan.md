# Snowflake account qualification permission plan

Draft for review, 10 November 2026. **This document authorizes no connection,
browser authentication, SQL or billable work.** It proposes bounded later approvals
using the existing connection configuration. No credentials or actual connection
identifiers belong in this document. The configured `STAGING` context is not
write-authorized merely because it is already configured or was used for the
[two-query canary](snowflake-read-canary.md).

The target remains all applicable Snowflake/DataGrip workflows, including writes,
DDL, transactions and results up to 1,000,000 rows. These stages are an incremental
qualification sequence, not a reduced acceptance target. Existing local evidence
and remaining gaps are in the [integration record](snowflake-sdk-integration.md)
and [required inventory](snowflake-datagrip-replacement.md).

## What can run, and what must be built first

| Route                      | Implemented boundary                                                                                             | Qualification still needed                                                                                                                                                                        |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Existing fixed read canary | Two fixed helper SELECTs; prior approved run passed on its recorded source                                       | A changed guarded runtime needs its own recorded provenance; that historical pass does not qualify the current guard, app or additional SQL                                                       |
| Current SDK adapter        | Read-only execution, scalar binding, cancellation, serialized same-session completion, guarded query submissions | Account behavior, complete results, exact types and normal packaged application acceptance                                                                                                        |
| Stages A1–A3 below         | Proposed additional fixed read scope                                                                             | A bounded runner/statement ledger and tests must be implemented and reviewed before execution; do not turn this document into an ad hoc SQL script                                                |
| Stages B1–B2 below         | Proposed disposable-object scope                                                                                 | Write/DDL/transaction state, affected-row/outcome handling, fault controls and cleanup must first be implemented and locally reviewed; current app gates must continue rejecting these operations |
| Packaged app acceptance    | Separate run against a recorded package                                                                          | Must enforce the approved SQL/metadata budget, use the actual app path and exercise its UI, result storage and lifecycle; SDK-only evidence cannot substitute                                     |

No developer switch, alternate OAuth route or SQL API fallback may bypass the app's
ReadOnly gate for qualification. An independently reviewed SDK qualification tool
can establish engine behavior before app integration, but its evidence must say
which app gates it bypasses. Neither the
[submission guard](snowflake-sdk-submit-guard-design.md) nor one successful write
enables general mutations. It bounds admitted submissions for reviewed transport
sources; it does not prove delivery, exactly-once effects, commit or rollback.

## Approval record and shared boundaries

Before requesting approval, produce a private local 0600 offline manifest
containing the exact rendered SQL, bind types/values, maximum invocation counts, runtime/package hashes,
runner source revision, selected stages and the cleanup sequence. Keep rendered
identifiers and binds out of tracked/shared artifacts; approve its hash through
the private conversation and retain the manifest only in the owned private root.
Local dry-run must not spawn the helper, authenticate, allocate auth state or submit SQL. The
approval names that manifest, one run window and the following local aliases:

- `DB`, `ROLE`, `WH`, and initial schema: selected from the existing connection or
  explicitly chosen by the owner through a private local configuration. Retain
  the existing account/host/TLS route; do not copy its values into tracked files.
- `READ_SCHEMAS`: at most two explicitly selected schemas for relation metadata.
  Database schema names are a separate, explicit metadata disclosure below.
- For B only, `Q = JOINERY_QUAL_<RUN>`, where `RUN` is one fresh locally generated
  32-character uppercase hexadecimal token, in an owner-selected disposable
  **standard Snowflake database**. `Q` is absent before the run and reserved
  exclusively for this run. Existing schemas are not adopted.

Identifier components are validated and quoted independently; the placeholders
below are not SQL expressions or a caller-supplied SQL suffix. `DB_LIT`, `S_LIT`
and `Q_LIT` mean properly escaped literals for those same exact identifiers. `?`
means a scalar bind with a fixed manifest value, not string interpolation. Review
the actual compiler-produced wire SQL and positional bind order before approval.
No arbitrary SQL, functions, object names or bind values are accepted at run time.

All stages exclude existing application-table reads, including `SELECT * LIMIT 0`
against them, and exclude existing-table writes. Reads of the specifically listed
Information Schema views are metadata permission, not table-data permission.
There are no grants, role creation, account/user configuration changes, warehouse
creation/resizing/suspend/resume commands, stages, PUT/GET, COPY, external functions,
stored procedures, tasks, integrations, clones, unloads or cross-database data reads.
The sole proposed CALL exception is B2's fixed SYSTEM$WAIT system-function call.
Session AUTOCOMMIT changes are allowed only in the separately approved B1 case.
The selected role is not elevated automatically after a denial.

Each stage has its own fresh session unless its test explicitly needs a second
observer. Do not attach tests to a user's working console or an open transaction.
No automatic reconnect, runner retry, replay, resubmission by request ID or
unreviewed recovery SQL is allowed. A scope/result/identity mismatch stops further
test statements; only the reserved, safe cleanup sequence may follow. Hitting a
cap fails the stage rather than returning partial qualification.

## Resource and billing envelope

These are proposed **enforced local admission/retention caps**, not existing
runner features, a server cost cap or proof remote work stopped. Implement and
test them before execution. Count every SQL request, including connect-time
`SELECT CURRENT_VERSION()`, health `SELECT 1`, context rechecks and background
metadata. A queued request must reserve budget before dispatch. SDK authentication,
logout, cancellation and renewal HTTP work is counted separately; a SQL count does
not claim to bound all physical HTTP requests.

| Stage, per approved run                   | SQL cap, including housekeeping         | Sessions                          | Result/object limits                                                                                                                             |
| ----------------------------------------- | --------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| A1 context + scalar                       | 12                                      | 1                                 | At most one row per SELECT; 64 KiB per small result                                                                                              |
| A2 selective metadata                     | 24                                      | 1                                 | Schema-name inventory: at most 1,000 rows; at most two relation queries; 10,000 relations / 2 MiB aggregate; no lazy columns on existing objects |
| A3 generated results + drain cancellation | 10                                      | 1                                 | At most two executions of the one 1M-row template; at most 2M generated rows / 512 MiB retained per result                                       |
| B1 fixtures + DML/DDL/TX + cleanup        | 160, reserving the final 24 for cleanup | At most 2 simultaneously, 3 total | One schema; at most 8 simultaneous relations; at most 64 rows across fixture tables; 1 MiB per fixture read                                      |
| B2 interruption / uncertainty + cleanup   | 40, reserving the final 16 for cleanup  | At most 2 simultaneously, 4 total | Existing B1 fixture only; at most two deliberately interrupted submissions; at most two 10-second waits; at most 64 fixture rows                 |

A stages total at most 46 SQL requests and three sessions if approved together;
B stages total at most 200 requests and seven sessions. App reruns consume a
separately named approval/budget, not another automatic copy of these allowances.
No stage retries a failed run to obtain a pass.

These bounds apply to every packaged-app stage, including object-free A1 and
generated-result A3: mounting the current SQL editor automatically requests
completion metadata (SCHEMATA plus per-schema TABLES reads). A packaged-app run
must first enforce a matching metadata permission/budget or suppress that
automatic dispatch through a reviewed implementation. A1/A3 approval alone does
not authorize those background reads. Use the helper-only route until the app's
complete automatic SQL scope is enforceable and approved.

Authentication wait is at most 120 seconds per session. Small qualification SQL
gets a 30-second local wait; current adapter read/metadata operations also have a
65-second operation deadline, so the earlier effective limit applies. No live
attempt silently raises those bounds. A stages have a ten-minute admission window;
B stages have a twenty-minute window, after which only reserved cleanup may start.
These bounds do not stop remote work already submitted. In particular, killing
the child does not establish server cancellation, logout or rollback.

Use a new private local result/auth root. Limit simultaneous result retention plus
export to 1 GiB; enable the existing opt-in disk result store deliberately for A3,
with a 64 MiB memory retention budget. Measure actual Node/app peak memory, local
disk and elapsed time; these retention caps do not prove SDK heap usage is bounded.
One complete CSV export is permitted to that private root, with a 512 MiB limit.
Verify and release that export/result before retaining the second generated result;
the aggregate 1 GiB limit takes precedence over individual allowances.
Paging and export must read the retained snapshot without another remote query.
Remove owned local data only after actual child reaping and directory-identity
verification; retain and report failed cleanup instead of claiming success.

The owner must explicitly accept possible automatic warehouse resume, compute,
cloud-service/result-access and synthetic-object storage charges. The runner
neither changes warehouse policy nor promises zero cost. Snowflake documents
per-second warehouse billing with a 60-second minimum on each start/resume;
using a small or already running warehouse does not establish an account-wide
currency ceiling. If a hard spend ceiling is required, the owner must provide an
appropriate pre-existing enforcement arrangement; this plan does not create one.
[Compute billing](https://docs.snowflake.com/en/user-guide/cost-understanding-compute).

## A1: same-session context and guarded scalar reads

The only additional object-free templates are:

```sql
SELECT CURRENT_VERSION()
```

```sql
SELECT 1
```

```sql
SELECT CURRENT_DATABASE(), CURRENT_USER(), CURRENT_VERSION();
```

```sql
SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME,
       CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME,
       CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID,
       TO_JSON(CURRENT_SCHEMAS()) AS SEARCH_PATH,
       CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES
```

```sql
SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION
```

```sql
SELECT CAST(? AS NUMBER(38,0)) AS EXACT_INTEGER,
       CAST(? AS BOOLEAN) AS EXACT_BOOLEAN,
       CAST(? AS VARCHAR) AS EXACT_TEXT,
       CAST(? AS VARCHAR) AS EXACT_NULL
```

The scalar vector is the exact integer `9007199254740993`, Boolean true, text
`joinery-qualification`, and typed SQL NULL. Preserve exact canonical strings
where the compiler/helper requires them; do not pass the integer through a
JavaScript Number. Use the context and parameter templates before and after the
scalar read within the 12-statement cap; no `USE` or `ALTER` is included in A.

Require exact configured role/database/schema/warehouse, the supported identifier
policy, stable principal, opaque nonempty session string, ordered search path,
secondary-role JSON contract, one complete result and exact scalar values/types.
`CURRENT_SESSION()` returns VARCHAR, not a numeric identifier. `CURRENT_USER()`
is a principal NAME and need not equal authentication LOGIN_NAME; independently
binding them requires a separately verified expected principal, not case folding.
Nullable context/roles and unavailable values must follow the engine contract and
the implementation's fail-closed support boundary, not invented synthetic
expectations. No raw principal/session/context values go into shared evidence.
[CURRENT_SESSION](https://docs.snowflake.com/en/sql-reference/functions/current_session),
[CURRENT_USER](https://docs.snowflake.com/en/sql-reference/functions/current_user),
[name versus login](https://docs.snowflake.com/en/sql-reference/functions/all_user_names),
[secondary roles](https://docs.snowflake.com/en/sql-reference/functions/current_secondary_roles).

## A2: selective metadata without application-table data

Approve schema **names** visible in exactly `DB`, then relation metadata in only
`READ_SCHEMAS`. Do not inspect columns, definitions, comments, grants, query
history, ACCOUNT_USAGE or other databases. The fixed templates are the current
metadata shapes, with quoted identifiers/literals substituted offline:

```sql
SELECT CATALOG_NAME, SCHEMA_NAME
FROM "DB".INFORMATION_SCHEMA.SCHEMATA
WHERE CATALOG_NAME = DB_LIT AND SCHEMA_NAME <> 'INFORMATION_SCHEMA'
ORDER BY SCHEMA_NAME LIMIT 1001
```

```sql
SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE,
       IS_TRANSIENT, IS_TEMPORARY, IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE
FROM "DB".INFORMATION_SCHEMA.TABLES
WHERE TABLE_CATALOG = DB_LIT AND TABLE_SCHEMA = S_LIT
ORDER BY TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, IS_TRANSIENT, IS_TEMPORARY,
         IS_ICEBERG, IS_DYNAMIC, IS_HYBRID, IS_IMMUTABLE
LIMIT 10001
```

Take A1 context/policy before and after the metadata sequence, with no automatic
session mutation. A sentinel row, truncation, context drift, ambiguous duplicate
temp/permanent spelling or size overflow must fail without publishing a partial
catalog. A permission denial must be reported, not repaired by granting access or
switching roles. Visibility is relative to privileges; a successful empty result
is not proof the account has no other objects.
[SCHEMATA](https://docs.snowflake.com/en/sql-reference/info-schema/schemata),
[TABLES](https://docs.snowflake.com/en/sql-reference/info-schema/tables).

**Current app acceptance gate:** `completion_catalog` currently enumerates every
visible non-system schema in `DB` (up to 1,000) and queries each, whereas this
permission permits at most two relation reads. Do not open an auto-refreshing app
session under schema-only permission. First implement/enforce the approved
dispatch budget and schema scope, or obtain a separately rendered database-wide
metadata plan with explicit counts. A preflight inventory alone does not enforce
that boundary against later changes. Current lazy completion also issues
`SELECT * ... LIMIT 0`; it remains excluded on all existing application objects.

## A3: one-million-row result, paging and export

No existing table is referenced. Use exactly this deterministic template, at most
twice: one complete run and one cancellation/result-drain run.

```sql
WITH SYNTHETIC AS (
  SELECT ROW_NUMBER() OVER (ORDER BY SEQ4()) AS N
  FROM TABLE(GENERATOR(ROWCOUNT => 1000000))
)
SELECT N AS ORDINAL,
       CAST(9007199254740992 + N AS NUMBER(38,0)) AS EXACT_INTEGER,
       CASE WHEN MOD(N, 2) = 0 THEN CAST(NULL AS VARCHAR) ELSE '' END AS NULL_OR_EMPTY,
       REPEAT('x', 96) AS PAD
FROM SYNTHETIC ORDER BY N
```

`GENERATOR(ROWCOUNT => constant)` bounds cardinality. `SEQ4` alone is not promised
gap-free, so the oracle uses `ROW_NUMBER`; there is no time-limit-only generator,
random workload, cross join or repeated polling.
[GENERATOR](https://docs.snowflake.com/en/sql-reference/functions/generator),
[sequence gaps](https://docs.snowflake.com/en/sql-reference/functions/seq1).

For the complete run require exactly 1,000,000 rows, no truncation, ordinals
1–1,000,000, exact large integer strings, SQL NULL versus empty string and exact
padding. Inspect first, middle, last and fixed seeded local sample pages; verify
the complete exported CSV with an independent local parser/count/digest against
the formula, without more SQL. The width deliberately exceeds 64 MiB overall.
Record memory/disk/backpressure and distinguish full retention from discarded or
truncated rows. Duplicate-column-name and the wider type inventory remain separate
required follow-ups, not claims established by these four columns.

In the second run cancel after a fixed local retained-row threshold of 10,000 or
while paused for credit after acknowledgement, whichever the reviewed runner
specifies. A completed result racing cancellation remains completed; a cancel ACK
does not prove rollback or that execution stopped. Do not rerun if the race does
not exercise the intended phase. Current classifier support for GENERATOR,
SEQ4/ROW_NUMBER and the expression forms must be checked offline; a rejection is
a blocked app test, not permission to loosen the classifier broadly.

## B1: unique schema, synthetic objects, mutations and transactions

This needs **separate write/DDL/cleanup approval** after local implementation.
The owner chooses a disposable database and confirms that the selected role may
create, own and later drop `Q` and its synthetic contents. CREATE SCHEMA requires
the corresponding database privilege; the test does not grant it. Prefer a
dedicated test database/role, with no other actor writing this unique schema.
If the role lacks authority, stop and ask the owner to arrange it outside this
run. Do not substitute the existing configured schema.
[CREATE SCHEMA privileges](https://docs.snowflake.com/en/sql-reference/sql/create-schema).

First run the A1 identity checks and an exact-name SCHEMATA lookup for `Q` (same
projection as A2, adding `AND SCHEMA_NAME = Q_LIT`, `LIMIT 2`). Require zero rows.
Create once, with no OR REPLACE or IF NOT EXISTS, then verify its owner/comment
through an exact-name SCHEMATA query selecting
`CATALOG_NAME, SCHEMA_NAME, SCHEMA_OWNER, COMMENT`, `LIMIT 2`:

```sql
CREATE SCHEMA "DB"."Q" DATA_RETENTION_TIME_IN_DAYS = 0
  COMMENT = 'joinery-qualification:RUN'
```

`RUN` here is the exact manifest token. The comment and name aid provenance; they
do not cryptographically pin an object generation or authorize deleting a
replacement. Stop on conflicting ownership/identity or unexpected objects. The
scope assumes exclusive use of this newly created namespace; a same-name
replacement by another actor invalidates cleanup authority.

Only these relation names may be created or targeted: `Q_STATE`, `Q_COLLISION`,
`Q_ORDINAL`, `Q_TYPES`, `Q_DDL`, `Q_DDL_RENAMED`, `Q_VIEW`. `Q_COLLISION` additionally
has one session temporary table with the same spelling. All references below
expand `T` into one exact `"DB"."Q"."<name>"`; no caller-selected table is allowed.
All permanent tables specify `DATA_RETENTION_TIME_IN_DAYS = 0`.

| Case                     | Fixed SQL templates and permitted values                                                                                                                                                                                                                                                                                                              | Required observation                                                                                                                                                                                                                                                                     |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| State / DML              | `CREATE TABLE T (ID NUMBER(38,0), MARK VARCHAR(32), ENABLED BOOLEAN, VERSION NUMBER(38,0)) DATA_RETENTION_TIME_IN_DAYS = 0`; `INSERT INTO T (ID,MARK,ENABLED,VERSION) VALUES (?,?,?,?)`; `UPDATE T SET MARK=?, VERSION=VERSION+1 WHERE ID=? AND VERSION=?`; `DELETE FROM T WHERE ID=? AND VERSION=?`                                                  | `T=Q_STATE`; IDs 1–16 only; MARK in `seed,updated,rollback,commit,ddl,uncertain`; VERSION 0–4; Boolean/NULL fixed in manifest. Exact affected rows and stale-version zero-match, then observer-visible state; standard-table primary keys are not relied on for enforcement              |
| MERGE                    | `MERGE INTO T AS D USING (SELECT CAST(? AS NUMBER(38,0)) AS ID, CAST(? AS VARCHAR) AS MARK) AS S ON D.ID=S.ID WHEN MATCHED THEN UPDATE SET MARK=S.MARK WHEN NOT MATCHED THEN INSERT (ID,MARK,ENABLED,VERSION) VALUES (S.ID,S.MARK,TRUE,0)`                                                                                                            | `T=Q_STATE`, same bounded IDs/marks; fixed insert and update cases, exact counts and independent state                                                                                                                                                                                   |
| Temp/permanent collision | `CREATE TABLE T (ID NUMBER(38,0), PERMANENT_MARK VARCHAR(16)) DATA_RETENTION_TIME_IN_DAYS = 0`; `INSERT INTO T VALUES (1,'permanent')`; `CREATE TEMPORARY TABLE T (ID NUMBER(38,0), TEMP_MARK VARCHAR(16))`; `INSERT INTO T VALUES (2,'temporary')`; `SELECT * FROM T ORDER BY ID LIMIT 65`; `SELECT * FROM T LIMIT 0`; `DROP TABLE T`                | `T=Q_COLLISION`. Own-session resolved columns/data show temporary object; another session sees permanent object. After explicitly dropping the temporary table, original session sees permanent. Completion must reject ambiguous metadata rather than silently selecting a write target |
| Dropped-column ordinals  | `CREATE TABLE T (A NUMBER(38,0), REMOVED VARCHAR(16), Z VARCHAR(16)) DATA_RETENTION_TIME_IN_DAYS = 0`; `ALTER TABLE T DROP COLUMN REMOVED`; `INSERT INTO T (A,Z) VALUES (1,'last')`; `SELECT * FROM T LIMIT 0`                                                                                                                                        | `T=Q_ORDINAL`. Compare resolved order A,Z with own-schema COLUMNS metadata. Record engine-reported ordinal positions; do not assume dropping a column must preserve or renumber an undocumented gap                                                                                      |
| DDL / replacement        | `CREATE TABLE T (ID NUMBER(38,0)) DATA_RETENTION_TIME_IN_DAYS = 0`; `ALTER TABLE T ADD COLUMN NOTE VARCHAR(16)`; `ALTER TABLE T RENAME COLUMN NOTE TO RENAMED_NOTE`; `ALTER TABLE T RENAME TO "DB"."Q"."Q_DDL_RENAMED"`; `DROP TABLE "DB"."Q"."Q_DDL_RENAMED"`; create `Q_DDL` again with `(ID NUMBER(38,0), NEW_GENERATION BOOLEAN)` and retention 0 | `T=Q_DDL`. Refresh metadata after each transition. A stale reviewed grid/CSV target must stay blocked until a separately implemented generation/conflict safeguard exists; reusable fully qualified names do not prove original-object identity                                          |
| View                     | `CREATE VIEW "DB"."Q"."Q_VIEW" AS SELECT ID, MARK FROM "DB"."Q"."Q_STATE"`; `SELECT * FROM "DB"."Q"."Q_VIEW" LIMIT 65`                                                                                                                                                                                                                                | View flagged truthfully; authoritative inspection/editing capabilities remain unsupported unless separately implemented                                                                                                                                                                  |

The collision requires ownership of the permanent table under Snowflake's
documented rule. A temporary object takes precedence for the owning session even
with the same qualified spelling; explicitly remove it before targeting the
permanent one. Temporary objects can incur storage charges while alive.
[Temporary-table behavior](https://docs.snowflake.com/en/user-guide/tables-temp-transient),
[collision privilege requirement](https://docs.snowflake.com/en/release-notes/bcr-bundles/2023_03/bcr-427).

The exact own-schema column oracle is:

```sql
SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, ORDINAL_POSITION,
       DATA_TYPE, IS_NULLABLE
FROM "DB".INFORMATION_SCHEMA.COLUMNS
WHERE TABLE_CATALOG = DB_LIT AND TABLE_SCHEMA = Q_LIT
  AND TABLE_NAME = 'Q_ORDINAL'
ORDER BY ORDINAL_POSITION LIMIT 1001
```

It reads at most 1,000 metadata rows / 1 MiB, not application rows. Zero-row
resolved columns are completion evidence, not keys/defaults/authoritative DDL.
[COLUMNS](https://docs.snowflake.com/en/sql-reference/info-schema/columns).

For `Q_TYPES`, use this one fixed table and insert, then `SELECT * FROM T LIMIT 65`
and zero-row column metadata, at most twice each:

```sql
CREATE TABLE "DB"."Q"."Q_TYPES" (
  EXACT_INTEGER NUMBER(38,0), EXACT_DECIMAL NUMBER(38,18), APPROXIMATE FLOAT,
  FLAG BOOLEAN, EMPTY_TEXT VARCHAR(64), NULL_TEXT VARCHAR(64), UNICODE_TEXT VARCHAR(64),
  RAW_BYTES BINARY, DAY_VALUE DATE, CLOCK_VALUE TIME(9),
  LOCAL_TIME TIMESTAMP_NTZ(9), ZONED_TIME TIMESTAMP_TZ(9), INSTANT_TIME TIMESTAMP_LTZ(9),
  JSON_VALUE VARIANT, ARRAY_VALUE ARRAY, OBJECT_VALUE OBJECT
) DATA_RETENTION_TIME_IN_DAYS = 0
```

```sql
INSERT INTO "DB"."Q"."Q_TYPES"
SELECT CAST('9007199254740993' AS NUMBER(38,0)),
       CAST('12345678901234567890.123456789012345678' AS NUMBER(38,18)),
       CAST('0.1' AS FLOAT), TRUE, '', NULL, 'snowman-☃', TO_BINARY('00FF','HEX'),
       CAST('2026-01-02' AS DATE), CAST('12:34:56.123456789' AS TIME(9)),
       CAST('2026-01-02 03:04:05.123456789' AS TIMESTAMP_NTZ(9)),
       CAST('2026-01-02 03:04:05.123456789 +05:30' AS TIMESTAMP_TZ(9)),
       CAST('2026-01-02 03:04:05.123456789 +05:30' AS TIMESTAMP_LTZ(9)),
       PARSE_JSON('{"n":null,"s":"x","i":9007199254740993}'),
       CAST(PARSE_JSON('[1,null,"x"]') AS ARRAY),
       CAST(PARSE_JSON('{"k":null,"v":"x"}') AS OBJECT)
```

Verify exact numeric strings and temporal nanoseconds/offset/instant semantics,
SQL NULL versus JSON null versus empty string, UTF-8 and binary encoding. FLOAT
is approximate; do not demand exact decimal arithmetic. JSON serialization order
is not object identity, and parsing the large JSON integer through a JavaScript
Number is not an exactness oracle. Unimplemented type support fails qualification
or stays explicitly unsupported; these types do not exhaust Snowflake's full
required inventory. Fixed conversion syntax and output assumptions need offline
review plus engine observation, without changing session formats to hide a bug.
[Snowflake data types](https://docs.snowflake.com/en/sql-reference-data-types).

Transaction controls are individual serialized statements on the same session:
`BEGIN TRANSACTION`, `COMMIT`, `ROLLBACK`,
`SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION`, and, only for the named case,
`ALTER SESSION SET AUTOCOMMIT = FALSE` / `TRUE`. Require the fresh session to
report AUTOCOMMIT TRUE before fixture DML; do not assume its inherited default or
automatically repair a different value. Fixed cases use Q_STATE inserts:

1. Explicit BEGIN + insert ID 3 (`rollback`) + ROLLBACK: no observer-visible row.
2. Explicit BEGIN + insert ID 4 (`commit`) + COMMIT: exactly one committed row.
3. BEGIN + insert ID 5 (`ddl`) + first CREATE Q_DDL + ROLLBACK: the insert remains
   committed; DDL is not reported as rollbackable.
4. BEGIN + insert ID 6 (`commit`) + ALTER SESSION SET AUTOCOMMIT = FALSE: verify
   the documented implicit commit. Insert ID 7 (`rollback`) in implicit mode,
   ROLLBACK, then restore TRUE only after the transaction has ended. Observe no ID 7.

The observer is read-only, uses the same approved account/role/database/warehouse
in a separately authenticated session and reads only
`SELECT ID, MARK, ENABLED, VERSION FROM "DB"."Q"."Q_STATE" ORDER BY ID LIMIT 65`.
No metadata/health activity may silently change transaction controls. Snowflake
documents that DDL commits an active transaction and runs separately, that changing
AUTOCOMMIT commits an active transaction, and that a failed DML statement need not
end the whole transaction. Test outcomes accordingly; cleanup is not promised to
roll back committed work. [Transactions](https://docs.snowflake.com/en/sql-reference/transactions).

## B2: cancellation, interruption and commit uncertainty

Approve this only after the B1 cases and local fault tool are reviewed. A combined
B1+B2 approval may reserve the same run's fixtures for B2 and defer B1 cleanup until
B2 ends. If B1 has already cleaned its schema, B2 requires a newly approved B1 setup
with a fresh namespace; it does not adopt an old schema or recreate one implicitly.
No account-wide network impairment, TLS interception, certificate bypass, dependency patch or
fault to another process is permitted. Fault controls affect only the owned helper
or its own transport, using an explicit local qualification seam absent from
production IPC/profile inputs. Render the fault phase and trigger before approval.

- Before dispatch: cancel a prepared attempt/queued Q_STATE read and prove no
  query submission. During a known bounded execution, optionally submit
  `CALL SYSTEM$WAIT(10, 'SECONDS')` at most twice and issue the
  existing SDK cancellation operation after one second. This template is a future
  supported-mode/classifier gate, not a current app allowlist expansion. If the
  result wins the race, report success; cancel acknowledgement is separate from
  the original terminal; SYSTEM$WAIT itself checks cancellation periodically, so an
  immediate remote stop is not promised. [SYSTEM$WAIT](https://docs.snowflake.com/en/sql-reference/functions/system_wait),
  [SDK cancellation](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#canceling-statements).
- Interrupt result draining of the A3 template only within its two-execution
  allowance, or the bounded own-table read. Preserve acknowledged execution versus
  incomplete results, retire the session, and prove no automatic replay. Do not
  treat interrupted fetch as a rolled-back statement.
- Two mutation faults maximum: (i) the autocommit insert of ID 15 (`uncertain`),
  and (ii) BEGIN + insert ID 16 (`uncertain`) + COMMIT. Lose the owning process or
  transport response at a reviewed dispatch/acknowledgement boundary. Preserve any
  definitive acknowledgement already observed. Otherwise report OutcomeUnknown,
  retire, and do not retry the insert or COMMIT, even with the same request ID.
- Use the separate observer's exact Q_STATE read to determine visible committed
  rows. An absent row observed once while work may still run is not proof of
  rollback. Permit at most three observer reads per fault, five seconds apart,
  within the SQL budget. If still unresolved, retain the unknown outcome and
  namespace for owner-directed follow-up; do not issue destructive cleanup against
  possibly running work. A later observed committed row establishes that state,
  not which transport delivery timing occurred.

A killed child before COMMIT is not proof that the server processed COMMIT; a
synthetic post-ACK suppression is not proof of remote lost-ACK behavior. Record
which boundary was actually hit and leave unexercised cases open. No exactly-once,
remote deduplication, immediate rollback-on-disconnect or deterministic cancellation
claim follows from this test. Admission of one native submission is a different
property from one server effect.

## Own-schema cleanup and evidence

Cleanup permission is part of B's approval, not authority over any existing
schema. Reserve its SQL budget before tests begin. First finish or truthfully
resolve active work and explicitly ROLLBACK a known usable owned transaction;
never send rollback into a replacement session and describe it as the old rollback.
Do not replay uncertain mutations to make cleanup easier.

In the original collision session, drop its temporary Q_COLLISION before the
permanent object. Drop Q_VIEW first, then only the recorded run-owned tables by
exact qualified names with individual `DROP TABLE`; include Q_DDL_RENAMED only
when that name was actually created. Track acknowledged drops and which name
generation is expected. An uncertain create/drop, unexpected object, changed
owner/marker or other actor invalidates automatic cleanup until resolved.

Recheck the exact schema provenance and its synthetic inventory using the approved
SCHEMATA/TABLES templates. Once recorded relations are gone, the only final schema
statement is `DROP SCHEMA "DB"."Q" RESTRICT`, followed by the exact-name SCHEMATA
lookup requiring no active schema. **RESTRICT is a foreign-key dependency rule,
not an empty-schema guard**; it does not replace the exclusive-namespace and
inventory checks. Do not add CASCADE, adopt an unknown schema or clean an unrelated
prefix match. [DROP SCHEMA](https://docs.snowflake.com/en/sql-reference/sql/drop-schema).

Explicitly drop temporary objects and request logout, then await actual local
child reap and root removal. A failed logout/unknown session is not evidence the
temporary object disappeared remotely. Zero retention on new objects does not
guarantee immediate physical erasure or eliminate permanent-table Fail-safe/storage
costs. Report active-object absence, logout acknowledgement, process reap, local
state removal and any unresolved remote cleanup separately.
[Temporary/permanent storage](https://docs.snowflake.com/en/user-guide/tables-temp-transient).

Evidence uses a new private 0600 local file and contains source/runtime/package
fingerprints, SQL-template IDs/counts, bounds, timings, exact-count/hash assertions,
known/unknown outcomes and individual cleanup booleans. Never persist/share raw
connection settings, credentials, browser URLs, tokens, query text containing
private identifiers, returned metadata names, rows, raw errors or session/query/
request IDs. A narrowly scoped private transient observer map may need actual IDs
for correlating the run; it is not copied into the report and is removed only when
safe to discard. Retain enough nonsecret failure categories to diagnose validators
without erasing the first failure when a corrected approved rerun later passes.

## Minimum input and staged approval wording

Before any request to run, finish the relevant tool/gates, tests and exact offline
manifest. Then obtain these minimum nonsecret choices privately:

1. Which stages and route: SDK/helper qualification, normal packaged app acceptance,
   or separately budgeted runs of both; the run window and reviewed source/package.
2. The existing connection alias and intended ROLE/WH, initial context and whether
   the owner provides an independently verified expected principal. No password,
   token or SSO URL is requested.
3. Approval for possible warehouse auto-resume and billing/storage, or an existing
   enforceable spend restriction. No account/warehouse settings are changed here.
4. For A2, DB schema-name visibility permission and at most two READ_SCHEMAS;
   permission for actual app-wide auto-completion must match its enforced dispatch
   scope. A3 separately includes 1M-row generation, private disk storage/export and
   the fixed cancellation attempt.
5. For B, an owner-selected disposable standard database, authority to create/own
   and drop the exact new Q namespace and its bounded synthetic contents, exclusive
   use, plus the named cleanup and session-AUTOCOMMIT scope. B2 additionally names
   the two fault triggers, observer session and retained-state escalation contact.

Concrete approval requests can then say: “Run manifest HASH, stages A1/A2 only,
using connection alias X, approved metadata scope and stated billing/deadlines;
no table data or writes.” A3 adds its explicit generated-result/export scope.
B1 needs a separate “create, mutate and clean only DB.Q and listed synthetic
objects, within the 160-statement cap”; B2 separately adds its exact interruption
scope. These are review templates, not a permission question being asked now.

Within a previously approved exact manifest, a validator-only repair and rerun may
be covered by that approval when its scope allows it; elapsed time or this document
does not grant approval. Expanded SQL, objects, faults, resource caps or routes
require a revised concrete scope. Each pass records whether it establishes SDK
engine behavior, app behavior, package/lifecycle behavior or owner acceptance.
None alone closes the remaining full-replacement requirements.
