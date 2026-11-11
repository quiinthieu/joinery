# Snowflake SDK reviewed DDL proposal

**Status: execution workflow proposed; pure syntax foundation implemented
locally and verified. No application DDL execution route or account
qualification.**
This is a bounded design for the next Snowflake SDK schema-change workflow. It
does not authorize account activity, enable DDL execution, or complete the Snowflake
DataGrip replacement. The full replacement still requires broader object
creation and editing, schema comparison, reviewed migration and synchronization
workflows, and hands-on qualification. Read alongside the
[reviewed DML contract](snowflake-sdk-dml-design.md), the
[explicit transaction contract](snowflake-sdk-transactions-design.md), and the
[Snowflake replacement inventory](snowflake-datagrip-replacement.md).

## Pure syntax foundation (11 November 2026)

`safety/snowflake_ddl.rs` compiles the existing structured table action and an
explicit database into one canonical statement. It has no raw-SQL entry point
and grants no execution, catalog, transaction or object-identity authority.
Closed types are NUMBER, VARCHAR, BINARY, BOOLEAN, FLOAT, DATE, TIME and
TIMESTAMP_NTZ; parameters and output are bounded. Numeric precision is 1–38,
scale is 0–37 and no greater than precision. Aliases, expressions, defaults,
identity, keys, comments, unsupported actions and supplied forbidden fields fail
closed. CREATE may specify NOT NULL on its empty new table; ADD remains nullable.
Identifier values are quoted independently and round-tripped through the whole
AST; quoted case semantics still require runtime qualification before execution.

All eleven inline tests pass in the complete crate. Independent review found the
scale-38 admission and lead review found plain DROP's default CASCADE behavior;
new regressions reproduced both before the fixes. DROP now explicitly emits
RESTRICT. The full validation passes 671 frontend / 437 standard Rust tests;
129 fixture tests are ignored, not passes. No UI or account DDL route is enabled.
See the [verification ledger](replacement-verification.md#pure-snowflake-ddl-syntax-foundation-11-november-2026).
Snowflake's [numeric limits](https://docs.snowflake.com/en/sql-reference/data-types-numeric)
and [DROP semantics](https://docs.snowflake.com/en/sql-reference/sql/drop-table)
were verified against current official documentation.

## Database contract and first workflow

Snowflake DDL has its own transaction semantics. Each DDL statement executes as
a separate transaction. If a transaction is already active, Snowflake commits
that transaction before executing the DDL. A later `ROLLBACK` cannot undo the
DDL. Do not place DDL inside a user transaction, describe it as rollbackable,
or treat the transaction toolbar state as protection. Initially require a fresh,
coherent observation of an idle SDK session in automatic-commit mode; reject
open, failed, unknown, stale, or manually managed transaction state. Do not
change `AUTOCOMMIT` as a side effect. See Snowflake's
[transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

The initial usable flow should cover one operation at a time against a
user-prepared disposable schema:

1. **Create a table.** Choose the exact database, schema and new table name;
   enter a bounded list of columns using an allowlist of supported scalar types.
   Require at least one column. Explicit NOT NULL is allowed on this newly
   created empty table. The first version should not accept defaults, other
   constraints, policies, tags, comments, identity/sequence behavior, table
   options or expressions. Require that the target table name is currently
   unused, then show the fully qualified generated statement.
2. **Add a column.** Select an existing supported table in that same schema and
   add exactly one nullable column with an allowlisted scalar type and no
   default. Snowflake documents that a newly added column without a default
   appears as `NULL` for existing rows. Do not expose drop/rename/type-change,
   multi-action `ALTER`, or `CREATE OR ALTER` in this first slice. See
   [ALTER TABLE](https://docs.snowflake.com/en/sql-reference/sql/alter-table).
3. **Drop a table.** Select one exact table and require a separate destructive
   confirmation that repeats its fully qualified name. Do not generate
   `CASCADE`, `PURGE`, or `IF EXISTS`; generate explicit `RESTRICT` because
   standard tables otherwise default to CASCADE for foreign-key references.
   RESTRICT does not protect dependent views or prove complete dependency
   absence. The action must fail if the reviewed
   target is absent or no longer matches the observed catalog. Explain that a
   dropped table may be recoverable only during its configured Time Travel
   retention, and that recovery is separate from transaction rollback. Snowflake
   notes that dropping a table can invalidate views that reference it and that
   retention and recovery depend on the object's type and settings. See
   [DROP TABLE](https://docs.snowflake.com/en/sql-reference/sql/drop-table).

Use one exact DDL statement per reviewed plan. Initially reject raw SQL input,
multiple statements, parser fallback, comments/directives that change meaning,
unqualified identifiers, `CREATE OR REPLACE`, `IF NOT EXISTS`, `IF EXISTS`, CTAS,
cloning, temporary/dynamic/external/hybrid/Iceberg tables, views, schema/database
DDL, grants, ownership changes and any syntax outside the three operations
above. `CREATE OR REPLACE` is explicitly excluded because Snowflake describes it
as equivalent to dropping the existing database object and creating a new one.
This generated-only slice is an explicit usability limit, not a claim of DDL
parity.

The intended UI pattern follows the useful parts of DataGrip 2026.3: create or
modify an object from the database explorer, review generated SQL in a preview,
then make an explicit submission. DataGrip also supports structure comparison,
selecting migration changes, reviewing a generated migration script, and
executing or opening it in a console. The bounded Joinery flow should adopt
review-before-submit while retaining its narrower grammar; it does not deliver
the full DataGrip migration workflow. See JetBrains'
[Create and Modify dialogs](https://www.jetbrains.com/help/datagrip/2026.3/create-and-modify-dialogs.html),
[schema comparison and migration](https://www.jetbrains.com/help/datagrip/2026.3/schema-comparison-and-migration.html),
and [updating database object source](https://www.jetbrains.com/help/datagrip/2026.3/update-source-code-of-database-objects.html).

## Policy, review and execution

DDL needs a separate persistent opt-in, such as `reviewedDdl`, defaulting to
false for both new profiles and profiles that lack the field. Do not infer DDL
consent from a non-ReadOnly boundary policy, the reviewed-DML opt-in, or prior
transaction controls. Only the supported Snowflake SDK external-browser route
may expose the opt-in. ReadOnly always denies DDL; OAuth/local-application and
legacy/missing-auth profiles remain read-only. Switching authentication or
policy clears the opt-in and revokes pending plans. Switching back never
restores it automatically. Preserve the existing production-profile and
password-gated normalization rules.

The opt-in is only an eligibility gate. Every statement still requires a fresh
plan and deliberate per-plan approval, including in Silent mode. The review
must display the operation, exact fully qualified target, generated SQL,
column changes and transaction warning. `DROP TABLE` gets its own destructive
confirmation that repeats the exact target. Apply the existing Safe Mode
confirmation rules, including passphrase entry in Password-Gated mode; never
weaken approval because SQL was generated by a form. Bind the one-use plan to
the exact SQL, structured action, effective policy, database/schema/table
identifier components, observed principal/session, transaction and catalog
revision, and expiry. Revalidate these at consumption and invalidate the plan
after any intervening session, policy, authentication or metadata change.

Use a dedicated typed SDK operation and private plan shape. Rust performs strict
AST admission for exactly one supported action and emits the canonical SQL;
the helper validates the closed payload, exact session identity and bounds.
Do not turn the DDL opt-in into a general `Write`/`DDL` SQL classification bypass.
Consume authority once, serialize the SDK lane through completion, and retain a
native one-submission latch for the exact statement identity. Never automatically
retry, reconnect and replay, or resubmit by request ID after possible dispatch.
The Node SDK's internal retry behavior remains a release gate: fault-test the
pinned execution path for DDL and keep the feature unavailable if a lost
response can cause an unreviewed replay. The existing DML proposal describes
the related SDK request-ID and retry concern.

Represent terminal evidence as `succeeded`, `rejected` or `unknown` separately
from later session/catalog observation. Preserve a known statement
acknowledgement if draining, event delivery, local history or catalog refresh
fails. A successful DDL acknowledgement does not independently prove what a
later catalog observer will see; do not label it `committed` or use a later
`ROLLBACK` as recovery. Once dispatch may have occurred, cancellation or
disconnect may leave the outcome unknown and must never trigger replay. Any
retry is a new user-reviewed plan after explicit state reconciliation.

## Object identity and metadata effects

Bind all actions to a fully qualified and losslessly parsed
`database.schema.object` identifier. Preserve quoted spelling and case; do not
split dots inside quoted names, normalize away case, or assemble SQL by string
concatenation. Bind the database to the fresh observed current database and the
schema/object to the corresponding catalog observation. A browser label or a
matching name is not an immutable Snowflake object ID. Between observation and
execution, another actor can rename, replace or create an object with the same
name, and name resolution can therefore identify a different object. Plans
cannot make that race atomic. Keep the first workflow restricted to a dedicated
disposable schema with least-privilege ownership and no concurrent DDL actors;
state that this operational boundary reduces exposure but does not prove
immutable object identity. Fail closed on ambiguous object kinds, hidden or
denied metadata, incomplete catalog results, a changed revision or observed
dependency outside the disposable target.

DDL changes catalog state, so set a durable metadata-refresh hold before
dispatch. Revoke completion, browse, inspection and stale plan grants that
depend on the affected schema; advance owning-tab revisions and block
conflicting work on every tab sharing the session. Do not issue background
catalog queries while the statement is pending or its outcome is unknown. After
a known terminal result, keep the hold until the owner explicitly requests a
fresh catalog refresh. A catalog readback may describe current visibility but
cannot retroactively prove that the original DDL executed exactly once.

## Tests and account qualification

Before enabling the feature, add focused coverage for:

* Profile persistence and migration: new and missing `reviewedDdl` values are
  false; DML consent does not imply DDL; OAuth and policy transitions clear it;
  restored credentials or profile normalization cannot elevate it.
* Grammar and identity: exact positive cases for supported CREATE, one-column
  ADD, and DROP; rejection of all excluded forms, multiple statements,
  unqualified/quoted-dot edge cases, parser fallback, identifier collisions,
  wrong catalog revision and wrong object kind.
* Approval and single use: ReadOnly denial; per-plan review under each Safe Mode;
  exact destructive DROP confirmation; stale/revoked/expired plans; one-use
  consume and no second SDK submission.
* Transaction and delivery behavior: active/manual/failed/unknown transaction
  blocks; no hidden `BEGIN`, `COMMIT`, `ROLLBACK`, `USE`, or `ALTER SESSION`;
  DDL is never described as rollback-protected; rejection, acknowledged success,
  response loss, cancellation, disconnect, malformed trailing data and helper
  exit preserve truthful evidence without a retry.
* Metadata and history: hold precedes dispatch and survives all terminal/error
  paths; no hidden refresh SQL runs; owner refresh restores grants only against
  a new observation; durable history keeps operation and outcome without
  recording credentials or unrelated SQL text.
* End-to-end UI: create, add column and deliberately drop only in disposable
  schema fixtures, with accessible review copy, exact SQL preview, resulting
  catalog state and prior-results labeling. Include conflicts from a second
  actor to demonstrate the documented object-name race and safe failure path.

Local pure, mock-helper and disposable-database tests establish grammar,
policy, evidence, and UI behavior only. Before any real-account qualification,
obtain explicit approval for a named non-production account, role, warehouse,
fresh disposable database/schema, statements and cleanup scope. Verify effective
role privileges, transaction state and the exact target before the first
statement; stop on unexpected account, object collision, privilege mismatch or
metadata ambiguity without escalating privileges or selecting a fallback
target. Capture statement IDs and outcome evidence without secrets, inspect
resulting catalog state independently, and record storage/warehouse/cloud-service
cost observations. Exercise lost-response and cancellation cases without
replaying mutations. Do not use a production schema or infer account approval
from this proposal, local fixtures, previous read canaries, or a DDL opt-in.

This slice is only an initial table lifecycle in a disposable schema. It does
not remove the full replacement requirements for object browsing/inspection,
DataGrip-style table and object editors, dependencies, schema compare/diff,
multi-object migration preview and selection, drift checks, synchronization,
broader DDL, or authorized account acceptance. Keep those gaps visible in the
[replacement acceptance inventory](replacement-acceptance.md) and
[DataGrip replacement scope](snowflake-datagrip-replacement.md).

## Implementation seams found in current source

The following is a proposed implementation plan, not implemented DDL support
or authorization to connect to an account. Preserve the current SDK read gate
and the PostgreSQL/MySQL DDL paths while adding this bounded workflow.

* **Policy and capability:** Add independent `reviewedDdl` persistence and
  transition handling in `domain.rs`, `drivers/snowflake.rs` and
  `ConnectionDialog.tsx`. Add a narrow capability for these reviewed table
  actions; keep SDK `ddlEditing` false because that flag also exposes index,
  constraint and migration editing.
* **Structured input and compiler:** Reuse the supported `TableObjectAction`
  variants and `TableColumnDraft`, but wrap them in an SDK request with an
  explicit observed database and server-held catalog/transaction revisions.
  `TableRef` and `DatabaseObjectRef` have no database component; never pack one
  into `schema` or split quoted identifiers on dots. A dedicated pure compiler
  must reject unsupported variants and fields, enforce the scalar type
  allowlist, and emit exactly one canonical, strictly admitted statement.
* **Constrained form and execution:** Reuse `TableObjectDialog.tsx` and
  `ReviewDialog`, with SDK entry points from `RelationBrowser.tsx` and the exact
  owning session. Remove identity, primary-key, default, comment and arbitrary
  type controls in SDK mode; ADD remains nullable. The existing form executes
  generic SQL and automatically calls `refreshCatalog` after success. Its SDK
  branch instead needs dedicated preview/prepare/execute commands and typed
  result handling; opening generated SQL in a console must not bypass the raw
  DDL rejection. Register contracts in `commands.rs`, `lib.rs`,
  `bin/test-harness.rs`, the contract exporter and `ipc/command-map.ts`.
* **Independent confirmations:** Always require per-plan approval, including
  Silent mode. Preserve Safe Mode's connection-name confirmation and
  Password-Gated passphrase check. `PreparedPlan.confirmationText` currently
  represents that policy confirmation; add a separate bound and validated
  exact-target confirmation for DROP rather than overwriting it.
* **Composite observation grant:** A new `execution/snowflake_ddl.rs` path
  should bind the canonical action, current persisted policy, principal/session,
  target scope, catalog state and full idle/autocommit observation in one
  expiring server-held grant. Browsing revokes normal transaction grants, so a
  browse revision plus cached Studio runtime is insufficient. Existing
  `RelationInspection.constraints` is `NotFetched`; it is not evidence that
  constraints or dependencies are absent. Ownership, hidden metadata, unused
  names and dependency completeness require explicit bounded qualification;
  unknown or incomplete evidence keeps the feature unavailable. Revalidate
  the grant and consume it once after acquiring the execution lane, while
  retaining the documented non-atomic object-name semantics.
* **Typed evidence and owner lifetime:** Extend the driver trait, SDK adapter,
  process protocol and helper with a dedicated closed DDL operation and exact
  statement acknowledgement. Reject generic execution of its private plan;
  retain succeeded/rejected/unknown evidence separately from later observation
  and avoid affected-row summaries. Use a tracked Studio owner task so dropped
  waiters and graceful shutdown cannot discard retained ACK/history evidence;
  a bounded shutdown timeout must keep the owner alive and the app open.
* **Catalog-aware metadata resume:** Reuse the native hold and shared-session
  pending/revision invalidation before dispatch. Add a hold kind or dedicated
  catalog-resume command: current `session_contexts_with_refresh(..., true)`
  clears the hold after runtime/options observation alone. A DDL hold must
  survive that path and clear only after an explicit owner request completes
  the bounded affected-catalog refresh and coherent observation against the
  captured hold revision. No automatic refresh follows the DDL result.

Focused regressions should cover forbidden action/form fields, quoted-dot
identifiers, both DROP and policy confirmations, stale composite grants,
manual/open/unknown transaction denial, second-actor drift, ACK preservation
through cancellation/drop/shutdown, and failed catalog resume retaining the
hold. Local fixtures do not close the account qualification or dependency
visibility gates above.
