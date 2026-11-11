# Snowflake SDK reviewed DDL

**Status: generated CREATE TABLE, nullable ADD COLUMN and DROP TABLE are
implemented locally, with helper and pinned-SDK loopback verification. Real
account DDL and normal application external-browser SSO are not qualified.**
This integrated local contract does not authorize account activity or establish
full DataGrip replacement. Read alongside the
[reviewed DML contract](snowflake-sdk-dml-design.md),
[explicit transaction contract](snowflake-sdk-transactions-design.md), and
[replacement inventory](snowflake-datagrip-replacement.md).

## Earlier pure syntax checkpoint (11 November 2026)

The first checkpoint implemented only `safety/snowflake_ddl.rs`, with no
application execution route. Its eleven inline tests passed; full validation at
that checkpoint passed 671 frontend and 437 standard Rust tests, with 129 fixture
tests ignored. Review reproduced and fixed scale-38 admission and plain DROP's
default CASCADE behavior. DROP explicitly emits RESTRICT. Those counts describe
that earlier generator checkpoint, not validation totals for the integrated
workflow. See the [verification ledger](replacement-verification.md#pure-snowflake-ddl-syntax-foundation-11-november-2026).

## Supported workflow and grammar

The relation browser opens a dedicated `SnowflakeDdlDialog` for the exact owning
console session. The form previews generated SQL and submits a typed, reviewed
plan. It does not choose a primary session or use generic console execution.
One plan admits one action:

- **CREATE TABLE:** an exact database, schema and new name, with 1–256 columns.
  NOT NULL is allowed on the new empty table. The adapter observes schema
  availability and exact-name absence; a concurrent collision still depends on
  Snowflake rejecting CREATE.
- **ADD COLUMN:** one nullable column, with no default, on the exact observed and
  inspected supported table. Existing rows receive NULL; see
  [ALTER TABLE](https://docs.snowflake.com/en/sql-reference/sql/alter-table).
- **DROP TABLE:** the exact observed table, with explicit RESTRICT and a separate
  destructive confirmation repeating the fully qualified target. There is no
  CASCADE, PURGE or IF EXISTS. RESTRICT does not protect dependent views or prove
  dependency absence; see [DROP TABLE](https://docs.snowflake.com/en/sql-reference/sql/drop-table).

The compiler accepts NUMBER, VARCHAR, BINARY, BOOLEAN, FLOAT, DATE, TIME and
TIMESTAMP_NTZ, with bounded parameters and output. Numeric precision is 1–38;
scale is 0–37 and cannot exceed precision. See Snowflake's
[numeric limits](https://docs.snowflake.com/en/sql-reference/data-types-numeric).
Each identifier component is independently quoted and the complete statement is
round-tripped through strict AST admission. Quoted dots and case remain part of
the identifier. There is no raw-SQL entry point or parser fallback.

Defaults, expressions, identity, keys, policies, tags, comments, table options,
rename/drop/type changes to columns, multi-action ALTER, CREATE OR ALTER, CREATE
OR REPLACE, IF NOT EXISTS, CTAS, cloning, temporary/dynamic/external/hybrid/Iceberg
tables, views, schema/database DDL, grants and ownership changes are excluded.
Broad `ddlEditing` stays false; the narrow reviewed capability grants no general
DDL or write-classification bypass.

ADD and DROP require an available `BASE TABLE` with known permanent or transient
status and known false temporary, Iceberg, dynamic, hybrid and immutable traits.
Unknown traits or incomplete/denied catalog evidence fail closed. CREATE emits
ordinary CREATE TABLE syntax, but a transient parent schema/database can make the
new table transient. Transient tables have zero or one day of Time Travel and
**no Fail-safe**. After retention expires, Snowflake cannot recover their
historical data. The workflow warns about this and does not verify the target's
configured retention or provide UNDROP/recovery. See
[temporary and transient tables](https://docs.snowflake.com/en/user-guide/tables-temp-transient)
and [transient schema semantics](https://docs.snowflake.com/en/sql-reference/sql/create-schema).

## Policy, review and authority

`reviewedDdl` is an independent persistent opt-in, default false for new and
missing values. Reviewed DML consent, transaction controls and a writable policy
do not supply it. Only Snowflake SDK external-browser profiles can enable it;
ReadOnly denies DDL. Authentication and policy transitions clear the opt-in and
revoke pending authority; switching back does not restore consent. Existing
production-profile and Password-Gated normalization rules still apply.

Every statement requires deliberate approval of a fresh plan, including Silent
mode. Review shows the operation, fully qualified target, canonical SQL, column
changes and transaction warnings. Safe Mode's connection-name confirmation and
Password-Gated passphrase remain independent of DROP's exact-target confirmation.

DDL executes in its own transaction and can implicitly commit pre-existing work;
ROLLBACK cannot undo it. Joinery requires a coherent idle SDK session with
AUTOCOMMIT true, no observed active transaction and unrestricted authority. It
rejects manual, open, failed, unknown or stale transaction state and never changes
AUTOCOMMIT to make DDL eligible. See
[Snowflake transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

Preflight owns the serialized session lane. A single expiring adapter grant binds
the structured action, canonical SQL, all three target components, current
principal/session, catalog and inspection evidence, transaction generation and
coherent idle observation. Studio binds this to the current persisted policy and
one-use reviewed plan. The adapter grant expires after 60 seconds. Dispatch
consumes it once, rechecks target evidence, takes another complete idle sample
immediately before submission and checks generation/expiry again at that point.
Intervening metadata, session, authentication or policy changes revoke authority.

Names and catalog observations are not immutable object identities. Constraints
remain `NotFetched`; dependencies, hidden objects and temporary shadows are not
exhaustively inspected. Concurrent rename/replacement can change what an exact
name resolves to between observation and execution. The first account workflow
therefore requires a dedicated disposable schema with least-privilege ownership
and no concurrent DDL actors. This boundary reduces exposure without making the
name-resolution race atomic or proving dependency absence.

## Submission, evidence and lifetime

The private helper payload is closed to `kind`, canonical `sql` and expected
`user`/`session`, with no binds. Rust owns grammar, policy and catalog authority;
the helper checks payload bounds and exact connected identity. The native guard
allows only the exact reviewed query submission and blocks repeated query POSTs
for that identity. There is no automatic retry, reconnect/replay, resubmission by
request ID, or automatic post-DDL catalog refresh.

Statement evidence is `succeeded`, `rejected` or `unknown`, independently of later
session/catalog visibility. The helper emits the exact DDL acknowledgement before
draining its private bounded status result; it publishes no query rowset or
affected-row count. An acknowledgement survives drain/protocol failure,
cancellation, disconnect, dropped IPC waiters and shutdown. Retirement before
acknowledgement after possible dispatch is unknown; retirement afterwards
preserves success while reporting the unusable session. A definitive server
rejection stays rejected even if later observation fails. An acknowledged DDL
is not rollback-protected or proof of later catalog visibility.

A tracked native owner continues through completion when the UI or IPC waiter
goes away. Studio records the operation, target, outcome and retirement evidence
in history/audit, subject to the existing query-text storage policy. A failed
local history/audit write retains the receipt in memory and blocks normal
shutdown until pending receipt writes succeed. Retrying persistence never
resubmits SQL. If SQLite cannot write, this retention does **not** provide crash,
forced-exit or power-loss durability; keep the application open and restore local
storage before closing. Local persistence errors do not erase an acknowledged
statement outcome.

## Catalog hold and explicit reconciliation

Before dispatch, Studio establishes a catalog-specific metadata hold, invalidates
completion, browse/inspection grants and pending plans, advances owning-tab
revisions and blocks conflicting work on tabs sharing the session. Prior query
results remain prior evidence rather than being presented as the DDL result.
No automatic catalog query follows success, rejection or unknown delivery.

The hold clears only through the explicit DDL catalog-reconciliation command:
coherent idle session/options observation, bounded catalog refresh, then a second
coherent idle observation matching the catalog context. The hold revision is
captured before waiting for the lane; an older queued refresh cannot clear a
newer hold. Ordinary runtime/options refresh cannot clear a catalog hold. Denial,
overflow, session drift or incomplete observation retains it. A retired session
requires reconnecting, and an unknown statement requires external outcome
inspection before any new reviewed mutation. Reconciliation describes current
visibility; it cannot retroactively prove exactly-once remote execution.

## SQL budgets and billing boundary

These are complete successful-path SQL submission budgets for each named command,
excluding earlier browser/catalog/column inspection and initial connection work.
There are no hidden transaction or session-setting statements:

| Command | CREATE | ADD | DROP |
| --- | ---: | ---: | ---: |
| Preflight | 12 reads | 15 reads | 11 reads |
| Execute an approved plan | 17 reads + 1 DDL | 20 reads + 1 DDL | 16 reads + 1 DDL |

Each idle sample is five reads (context, quoted-identifier policy, AUTOCOMMIT,
abort-on-error and current transaction). Preflight takes two samples and target
checks of 2/5/1 reads. Execute repeats those checks between two samples, submits
one DDL, then takes a five-read terminal sample. A terminal rejection can also
take that final sample; early failures may stop sooner. No automatic retry adds
another mutation.

Explicit reconciliation costs `27 + S` reads, where `S` is the visible schema
count: two eleven-read session/options observations and a `5 + S` catalog read.
The schema bound is 1,000, so the maximum is 1,027 reads. Catalog output is also
bounded at 10,000 relations and 2 MiB. This can inspect the current database, not
just the changed table. Review account/billing approval against this complete
budget; metadata and DDL can resume the already selected billable warehouse.

## Local verification and remaining qualification

The complete pinned helper suite passes **356/356**, with evidence at
`.cache/verification/connection-attempts/ddl-helper-all-pinned.log`. It covers
strict payload/identity admission, DDL acknowledgement and bounded private status
results, definitive rejection, pre/post-ACK retirement, reentrant close and
single-submission behavior. Actual SDK 3.4.1 loopback cases exercise HTTP,
verified TLS and CONNECT transport with synthetic local login and preconnected
production Helper fixtures. The unguarded SDK control demonstrates internal
retries; guarded cases show the query guard preventing a second submission.

These tests establish the pinned local path, not Snowflake account deduplication,
remote DDL effects or normal application SSO. Helper EOF-seam tests using
`Helper.stop()` do not qualify every framed-process EOF path. Source pins cover
selected SDK/Axios/follow-redirects seams rather than every dependency or an
operating-system network sandbox. See [runtime checks](snowflake-runtime.md).

Before account qualification, approve the exact non-production account, role,
warehouse, disposable target, full observer/DDL/reconciliation budget and cleanup
scope. Stop on unexpected identity, privilege denial, collision or metadata
ambiguity without escalating privileges or selecting a fallback target. Record
statement IDs, statement outcomes, independent catalog evidence and cost
observations without secrets. No local test, opt-in or previous read canary grants
that approval. Real-account CREATE/ADD/DROP and normal-app external-browser SSO
remain unqualified.

This is a bounded table lifecycle. Object editors, dependencies, schema compare,
multi-object migration preview/selection, drift checks, synchronization, broader
DDL and authorized account acceptance remain open in the
[replacement acceptance inventory](replacement-acceptance.md) and
[DataGrip replacement scope](snowflake-datagrip-replacement.md).
