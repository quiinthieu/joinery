# Disposable Snowflake acceptance setup

Prepared 11 November 2026 at the owner's request. **No account
connection, SQL, object creation or cleanup has been executed or authorized.**
The prior read approvals are consumed. This setup does not grant write, DDL,
transaction, interruption or billing permission.

Use the existing external-browser connection and its configured role and
warehouse. Create a fresh transient database named `JOINERY_ACCEPTANCE_<RUN>`,
where `RUN` is one newly generated 32-character uppercase hexadecimal token.
Create only its `SCRATCH` schema and empty `ROW_CHECKS` table. Never use the
existing configured database/schema as a fallback. A missing `CREATE DATABASE`
privilege or name collision stops setup; do not switch roles or grant privileges.
The owner can instead have an administrator create this exact isolated setup.

## Exact proposed SQL

These aliases describe the privately rendered statements. Quote each identifier
component independently. `DB` is the one generated database, and `LABEL` is
`joinery-disposable-<RUN>`; neither accepts arbitrary SQL. There are six proposed
object statements, each invoked at most once and in this relative order. The
separate helper runner's full 28-statement ledger also includes the session
observations listed below:

```sql
CREATE TRANSIENT DATABASE "DB"
  DATA_RETENTION_TIME_IN_DAYS = 0 COMMENT = 'LABEL';

CREATE SCHEMA "DB"."SCRATCH"
  DATA_RETENTION_TIME_IN_DAYS = 0 COMMENT = 'LABEL';

CREATE TABLE "DB"."SCRATCH"."ROW_CHECKS" (
  "ID" NUMBER(38,0), "NOTE" VARCHAR(128), "ACTIVE" BOOLEAN
) DATA_RETENTION_TIME_IN_DAYS = 0;

SELECT DATABASE_NAME, DATABASE_OWNER, IS_TRANSIENT, COMMENT, CREATED
FROM "DB".INFORMATION_SCHEMA.DATABASES
WHERE DATABASE_NAME = 'DB';

SELECT CATALOG_NAME, SCHEMA_NAME, SCHEMA_OWNER, IS_TRANSIENT, COMMENT
FROM "DB".INFORMATION_SCHEMA.SCHEMATA
WHERE CATALOG_NAME = 'DB' AND SCHEMA_NAME = 'SCRATCH';

SELECT COUNT(*) AS "ROW_COUNT" FROM "DB"."SCRATCH"."ROW_CHECKS";
```

The result contract requires exactly the new database/schema, their expected
owner and labels, transient status, and one exact zero row count. No existing
application table is read. Stop after any rejection, unknown acknowledgement,
unexpected result or ownership mismatch. Do not replay a possibly submitted
statement. A successful CREATE acknowledgement is evidence to retain separately
from subsequent catalog visibility; names and comments are not immutable object
identities. Do not invite other users or concurrent DDL into this database.

Snowflake documents the [database syntax](https://docs.snowflake.com/en/sql-reference/sql/create-database),
[schema syntax](https://docs.snowflake.com/en/sql-reference/sql/create-schema),
[database metadata](https://docs.snowflake.com/en/sql-reference/info-schema/databases)
and [schema metadata](https://docs.snowflake.com/en/sql-reference/info-schema/schemata).
New databases also contain system-created schemas; setup does not alter them.
DDL implicitly commits an active transaction and cannot be undone with a later
ROLLBACK. Setup must use a fresh idle session, never a working console with
pending edits. See [transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

## Cleanup and charges

Keep the setup for separately approved synthetic DML, transaction and schema
acceptance. This proposal includes no inserts, updates, deletes or acceptance
queries. Later work needs its own exact statement/observer/metadata budget and
approval; this setup is not blanket authority over the new database.

The proposed eventual cleanup statement is:

```sql
DROP DATABASE "DB" RESTRICT;
```

Cleanup needs separate explicit approval and an inspection of the exact
database's ownership, creation receipt and complete object inventory. Refuse
cleanup if a CREATE outcome is uncertain, a name has been replaced, unrelated
objects or users appear, or ownership cannot be established. Preserve evidence
and report the required owner intervention. `RESTRICT` checks foreign-key
references; it does **not** mean the database must be empty and is not a
substitute for this ownership review. Dropping a database removes its contained
objects. See [DROP DATABASE](https://docs.snowflake.com/en/sql-reference/sql/drop-database).

Transient objects and retention zero reduce recovery/storage exposure; they
do not promise immediate purge or zero cost. There is no rollback safety net.
Authentication, metadata, the empty-table count and subsequent qualification
may incur cloud-service/storage charges or resume the configured warehouse.
No warehouse creation, resizing or policy change is proposed. Review Snowflake's
[transient storage](https://docs.snowflake.com/en/user-guide/tables-temp-transient),
[Time Travel](https://docs.snowflake.com/en/user-guide/data-time-travel) and
[compute billing](https://docs.snowflake.com/en/user-guide/cost-understanding-compute)
rules before execution approval.

## Private proposal and execution gate

The rendered offline proposal is owner-private (0600), under the ignored
`.cache/snowflake-qualification/owner-disposable-setup-proposal.json`. It contains
no credentials. Its SHA-256 is
`619ed15a8447898f084424a11fb9852b9d180d5065a36dcad7872328222a44d6`.
It declares `offlineOnly: true`, `runner: null` and no authorization. It is
deliberately **not** an executable approval manifest.

The separate bounded setup runner is implemented locally. Its qualification and
source/runtime pins must be recorded before requesting execution permission.
The original six-statement proposal above remains historical preparation, not
the runner's approval manifest. The runner derives every SQL string from a
closed ledger and the single run token; it has no arbitrary-SQL flag.

| Index | Statement or fixed observation |
| --- | --- |
| 1 | Six-column startup identity/context SELECT |
| 2 | `SELECT CURRENT_VERSION()` |
| 3–7 | Initial five-observation sample |
| 8 | CREATE TRANSIENT DATABASE |
| 9–13 | Five-observation sample, expecting the new database and PUBLIC schema |
| 14 | CREATE SCHEMA |
| 15–19 | Five-observation sample, expecting the new database and SCRATCH schema |
| 20 | CREATE TABLE |
| 21–23 | Exact database/schema readback and zero table count shown above |
| 24–28 | Final five-observation sample |

Each five-observation sample is exactly:

```sql
SELECT CURRENT_USER() AS PRINCIPAL, CURRENT_ROLE() AS ROLE_NAME,
  CURRENT_DATABASE() AS DATABASE_NAME, CURRENT_SCHEMA() AS SCHEMA_NAME,
  CURRENT_WAREHOUSE() AS WAREHOUSE_NAME, CURRENT_SESSION() AS SESSION_ID,
  TO_VARCHAR(CURRENT_SCHEMAS()) AS SEARCH_PATH,
  CURRENT_SECONDARY_ROLES() AS SECONDARY_ROLES;
SHOW PARAMETERS LIKE 'QUOTED_IDENTIFIERS_IGNORE_CASE' IN SESSION;
SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION;
SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION;
SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID;
```

The first context SELECT is the helper's existing startup identity operation:
`SELECT CURRENT_USER(), CURRENT_ROLE(), CURRENT_DATABASE(), CURRENT_SCHEMA(),
CURRENT_WAREHOUSE(), CURRENT_SESSION()`. Connect itself sends zero SQL. The
private execution manifest contains all 28 rendered strings and their bounds.
The runner pins principal/session, primary role, warehouse and secondary roles;
requires AUTOCOMMIT true, no active transaction, quoted-case policy false and
stable settings before each CREATE. CREATE's documented context changes are
expected explicitly. Search-path samples do not prove atomic identity; every
object statement uses its fully qualified generated name.

One attempt permits one helper/SSO session, at most 28 SQL statements including
three CREATEs, one outstanding SQL command, and no retries or cleanup SQL.
Explicit cancel and close controls may overlap the active command. Each result
is bounded to one row, 64 KiB and at most 32 columns. Authentication is bounded
to 120 seconds, each SQL operation to 30 seconds, and the workflow to 360 seconds
before bounded close/EOF/TERM/KILL cleanup. Receipt persistence has a separate
bounded wait. Admission and SQL stop at the exact approved window's end; setup
accepts a window of at most 24 hours so a short approval delay does not require a
new plan. This is neither a spending cap nor a bound on SDK authentication,
result-fetch or logout HTTP traffic.

Before authentication, exclusive private markers consume both the plan hash and
the account/host/run-token combination. They are never removed for another try.
The private hash-chained journal syncs each intent before dispatch and each
CREATE acknowledgement before any later SQL. Failure stops subsequent SQL;
known acknowledgement, unknown delivery, readback validation and complete
qualification remain distinct. Failed receipt persistence prevents progress,
but cannot undo a CREATE. Preserve the journal and require owner intervention
after uncertain delivery; never create a new approval for the same object token.
The retained final receipt deliberately has `qualificationPassed: false`,
`evidencePersisted: false` and `finalizationStatus: requiresRunnerConfirmation`:
it records execution facts and a qualification candidate without certifying its
own final directory sync. Only the runner's returned report can state
`runnerConfirmed` after that sync succeeds. Keep both forms of evidence; a
provisional receipt alone never establishes complete qualification. Journal-close
or final-sync failures preserve known CREATE outcomes and report failed
qualification. No receipt is overwritten to manufacture success.
Local source checks are pathname-based and do not protect against hostile
same-user mutation or remote concurrent changes.

Offline preparation and verification use:

```sh
pnpm snowflake:qualification:setup --input .cache/snowflake-qualification/owner-setup-input.json
pnpm snowflake:qualification:setup --input .cache/snowflake-qualification/owner-setup-input.json --emit .cache/snowflake-qualification/owner-setup-execution-plan.json
pnpm snowflake:qualification:setup --input .cache/snowflake-qualification/owner-setup-input.json --plan .cache/snowflake-qualification/owner-setup-execution-plan.json
```

These commands do not launch the helper or authenticate. Inputs/outputs are
owner-private; do not paste the connection profile or journal into public logs.
Only a separately authorized `--run` with the exact plan and one-use approval
file can start account activity. Do not manufacture that approval from this
document or reuse a consumed read approval. The existing
[24-read runner](snowflake-qualification-runner.md) cannot execute this setup and
must not be repurposed by passing arbitrary SQL. The application's reviewed
DML opt-in does not enable database/schema/table DDL. The
[DDL design](snowflake-sdk-ddl-design.md) has a pure syntax foundation but no
application execution route.

The broader [account qualification plan](snowflake-account-qualification-plan.md)
and [replacement acceptance](replacement-acceptance.md) remain open. Preparing
this plan required no private application-state access or Snowflake activity.
