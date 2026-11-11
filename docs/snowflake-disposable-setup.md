# Disposable Snowflake acceptance setup

Prepared 11 November 2026 at the owner's request. **Proposal only: no account
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
setup statements, each invoked at most once and in this order:

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

Before requesting execution permission, implement and independently review a
bounded setup runner with a complete ledger including connect-time SQL,
context/settings checks, these six statements and any permitted recovery or
cleanup. Bind it to source/runtime hashes, a one-use approval and admission
window; verify private receipts, result limits, interruption, no replay, and
cleanup ownership using synthetic tests. The existing
[24-read runner](snowflake-qualification-runner.md) cannot execute this setup and
must not be repurposed by passing arbitrary SQL. The application's reviewed
DML opt-in does not enable database/schema/table DDL. The
[DDL design](snowflake-sdk-ddl-design.md) remains proposed.

The broader [account qualification plan](snowflake-account-qualification-plan.md)
and [replacement acceptance](replacement-acceptance.md) remain open. Preparing
this plan required no private application-state access or Snowflake activity.
