# Snowflake SDK reviewed DML proposal

**Status: proposed next increment; not implemented or account-qualified.**
The SDK currently admits public SQL through a single classified-read gate.
Explicit transaction controls have their own typed workflow; they do not enable
DML. This proposal adds a usable, bounded INSERT/UPDATE/DELETE workflow on the
same SDK session. It authorizes no account activity and does not complete the
[DataGrip replacement scope](snowflake-datagrip-replacement.md). Broader DML, grid
editing, cross-database operations, DDL and full transaction acceptance remain
required. Read alongside the
[explicit transaction contract](snowflake-sdk-transactions-design.md).

## Policy prerequisite and supported scope

Do not reinterpret existing non-ReadOnly SDK profiles silently. Their current UI
describes authorization for explicit transaction boundaries. Recommended
migration: add an explicit SDK DML opt-in, default false for existing and new
profiles, separate from SafeMode. The owner enables it with copy describing table
mutations, transaction consequences and possible warehouse billing. SafeMode
still determines per-plan approval, confirmation and passphrase requirements;
ReadOnly always denies DML. An alternative migration needs equivalent explicit
consent before previously transaction-only profiles gain DML permission.

Keep OAuth/legacy profiles ReadOnly and prevent authentication changes from
restoring elevated policy or opt-in automatically. Preserve verified HTTPS/443,
no SSH, no stored password and existing production/group normalization. Changing
profile policy, authentication or passphrase invalidates pending plans. A new
independent reviewed-DML capability must not enable table editing, DDL editing,
arbitrary transaction SQL or AUTOCOMMIT mode changes.

The first supported grammar should be explicit:

- One INSERT INTO a named table, with an explicit column list and bounded VALUES
  rows containing supported scalar expressions or binds.
- One UPDATE of a named table with supported SET expressions and optional WHERE.
- One DELETE FROM a named table with optional WHERE.

Require one successfully parsed top-level statement. A `Write` classification is
insufficient: SELECT INTO and other forms can receive that classification.
Reject parser fallback, nested mutations, scripting, CALL/EXECUTE, COPY, PUT/GET,
MERGE, multi-table inserts, RETURNING and unreviewed dialect modifiers. Initially
reject INSERT SELECT, INSERT OVERWRITE, UPDATE FROM, DELETE USING, subqueries and
unsupported expressions rather than accepting them through a broad Write gate.
Retain all AST visitor checks; classification must never become permissive merely
because a plan was approved or SafeMode is Silent. Preserve unrestricted
UPDATE/DELETE warnings and exact-name confirmation under applicable policies.

These exclusions define the initial grammar, not completion of Snowflake DML.
INSERT SELECT, joins/subqueries, bulk operations and MERGE need subsequent
reviewed support. INSERT OVERWRITE deletes existing rows and needs specific
destructive-operation treatment, even though Snowflake permits it inside the
current transaction. UPDATE FROM can produce nondeterministic multi-joined
updates. Do not silently change session parameters to change those semantics.
See [INSERT](https://docs.snowflake.com/en/sql-reference/sql/insert) and
[UPDATE](https://docs.snowflake.com/en/sql-reference/sql/update).

For the first current-database slice, require an explicitly database-qualified
target and compare its parsed identifier components with the pinned database.
Do not split quoted names on literal dots, concatenate SQL fragments or rewrite
the reviewed target after approval. Reject known temporary/permanent ambiguity
or unsupported relation kinds; browsing tokens are not immutable object IDs.
Concurrent server DDL or hidden objects can still change name resolution. Either
prove and document the selected-name invariant under least-privilege metadata,
or retain a target-qualification gate; do not claim an atomic object binding.

## Native admission, binds and helper contract

Reuse the SQL editor's parameter/review UI, but store a distinct SDK-DML action.
Recommended private request fields are DML kind, original classified SQL,
canonical bound statement, session ID and server-held observation revision. The
stored fingerprint covers all fields, exact wire SQL/binds and effective policy.
After owning the Studio lane, consume the plan once under the configuration
gate; rederive current persisted policy, check deadline/approval/passphrase and
session closure, then validate the adapter authority. Do not hold that gate over
network awaits. A queued plan remains revocable until consumption.

Reuse `params::validate_inputs` and canonical recompilation. Require exact
recompiled wire SQL and ordered bind equality, including repeated named slots;
never trust a public BoundStatement independently. Keep supported scalar values,
server numeric/boolean casts, precision, SQL/frame/aggregate-byte bounds and at
most 1,000 parameter occurrences for this proposed DML gate. The general
parameter compiler currently permits 1,024; apply the narrower DML cap explicitly
rather than claiming the existing compiler already enforces it. Do not interpolate values, accept raw SQL
fragments or admit array binds/staged binding. The existing positive SDK
`arrayBindingThreshold` prevents hidden staged-binding setup for this scalar
path. Snowflake documents positional bindings and their size limitations in
[Executing statements](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#binding-statement-parameters).

Recommended helper operation: strict `dml` payload with kind
`insert | update | delete`, exact reviewed SQL, scalar binds and expected private
principal/session identity. Unlike fixed transaction commands, DML necessarily
contains reviewed SQL: the kind enum is not an independent proof of its grammar.
Rust owns AST admission; the helper validates the closed payload, established
identity pin and bounds at admission and dispatch. Match this operation only to
`Impact::Mutating`; leave public read admission unchanged. Never adopt a supplied
pin or use profile login/role as a fallback for observed identity.

One operation owns one guarded SDK execute, one exact SDK request ID and its
allowance through terminal/drain. Keep mandatory transport-source verification,
the native one-submission latch and redirect restrictions. No retry, reconnect,
request-ID resubmission or replacement session is part of this workflow. SDK
documentation describes request-ID resubmission on the same connection, but that
is a separate recovery design, not permission for automatic replay here.
[Resubmitting requests](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#resubmitting-requests)

## Statement evidence and exact affected counts

Retain successful statement acknowledgement and its exact registered identity
before callbacks, status parsing, draining or event delivery can fail. A typed
result should separately contain statement outcome (`succeeded`, `rejected`,
`unknown`), optional exact affected count, summary-validation status, final
runtime, retirement and a bounded safe error. These are recommended DTO concepts,
not established public field names. Pre-admission failures can return Err;
post-admission results must preserve known evidence, including when the UI or
history writer is gone. ACK means statement success, not independently verified
transaction disposition.

The current read adapter returns helper `rowsSeen` through QuerySink::complete
and Driver::execute's u64. That would report one for a DML status row. Add a typed
driver result instead of forcing ACK, drain failure and unavailable counts through
`AppResult<u64>`. QuerySink may still render the bounded status result, but viewer
closure, retention failure or result-store failure cannot replace remote evidence.
Studio's generic connection-loss-to-OutcomeUnknown rewrite must consult that
evidence rather than overwrite acknowledged success.

Recommended pure summary parser, with exact descriptors qualified before release:

| Kind   | Required status           | Separate observation                                 |
| ------ | ------------------------- | ---------------------------------------------------- |
| INSERT | `number of rows inserted` | No invented update/delete count                      |
| UPDATE | `number of rows updated`  | `number of multi-joined rows updated`, when reported |
| DELETE | `number of rows deleted`  | No invented insert/update count                      |

Require one complete, untruncated row; unique names; a reviewed descriptor set;
authoritative fixed numeric wire types; and nonnegative integral decimal text.
Proposed cap: two columns, one row and 64 KiB including descriptors/cells. Normalize
only reviewed wire-type casing, not arbitrary aliases. Parse with checked integer
arithmetic and publish decimal strings. Reject null, fractional, negative,
duplicate, unexpected, oversized or overflowing fields. A failed parser makes the
count unavailable while preserving prior success ACK. Do not use zero to mean
unavailable. Exact SDK descriptor casing/optional-column variants remain an
account acceptance gate; unexpected variants fail closed without replay.

Do not use the Node SDK's `getNumRows()` as affected rows or confuse it with the
JavaScript stored-procedure API. Node SDK 3.4.1 exposes `getNumUpdatedRows()`, but
its implementation converts and sums every numeric status cell. That loses
precision outside JavaScript's safe integer range and can add multi-joined rows
to affected rows. Parse each status explicitly instead. See the official pinned
[Statement implementation](https://github.com/snowflakedb/snowflake-connector-nodejs/blob/v3.4.1/lib/connection/statement.js)
and [Result implementation](https://github.com/snowflakedb/snowflake-connector-nodejs/blob/v3.4.1/lib/connection/result/result.js).
The documented [INSERT](https://docs.snowflake.com/en/sql-reference/sql/insert),
[UPDATE](https://docs.snowflake.com/en/sql-reference/sql/update) and
[DELETE](https://docs.snowflake.com/en/sql-reference/sql/delete) examples establish
the distinct status meanings, not qualification of every live descriptor variant.

## Same-session observation, recovery and scheduling

Use the existing full observation sample and separate transaction recovery owner.
Consume one-use DML authority before dispatch; reject restricted rollback-only
state, unavailable baseline, stale revision, different principal/session or
changed full context. Hold one adapter lane through the result and final sample.
Recommended budget: five baseline statements, one DML, five final statements:
**at most eleven explicit statement executions**, ending early on failure. The
initial coherent observer may separately cost ten statements when authority is
created. Target inspection or later supported grammar must have its own declared
budget, not be hidden inside eleven. These counts exclude authentication/session
setup and do not promise a server execution-cost cap.

Skip Studio's generic ten-read end-run observer for this typed result. Do not
issue hidden USE, BEGIN, COMMIT, ROLLBACK, ALTER SESSION or count/history queries.
Interpret observed state conservatively:

- An existing explicit transaction remains on the same observed opaque ID; that
  ID does not establish usability. A failed DML alone does not prove abortion.
- With AUTOCOMMIT true and no explicit transaction, documented statement success
  ordinarily completes its own transaction. Describe that mode without claiming
  an independent check of committed data effects.
- With AUTOCOMMIT false and no transaction, DML can start an implicit transaction.
  Publish its observed ID and offer existing reviewed commit/rollback controls;
  never auto-BEGIN or auto-COMMIT.
- After known ACK or definitive database rejection, ordinary observer denial can
  leave Unknown runtime with retained same-session rollback-only recovery. Never
  require a successful baseline observer before recovery ROLLBACK. Recovery is
  generation-bound and survives unrelated completion/options invalidation.
- Unknown delivery, foreign identity, transport/protocol corruption, abandoned
  operation ownership or deadline retirement closes the local session. Preserve
  known ACK if one exists; absent it, report uncertainty rather than success or
  rejection. Child reap does not prove remote rollback.

Snowflake's [transaction rules](https://docs.snowflake.com/en/sql-reference/transactions)
describe single-session transactions, implicit DML transactions, failed-statement
rollback and DDL/AUTOCOMMIT implicit commits. Continue leaving observed usability
Unknown, and retain recovery when abort-on-error prevents ordinary observation;
see [TRANSACTION_ABORT_ON_ERROR](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error).

Cancellation before admission reports no submission. After possible dispatch,
cancel acknowledgement is only control evidence; retain the original statement
terminal/ACK, then conservatively retire without replay. A successful statement
followed by cancellation must not become a cancelled or unknown mutation. Keep
the exact-operation RAII lease and cleanup receipts through all await/drop paths.
Cancellation during connection setup can never publish an owned DML action.

Before DML, revoke schema/completion/browse/transaction grants and stale plans,
advance owning-tab event revisions, and block conflicting queries or controls on
every tab sharing the session. Preserve existing results. Set the durable
metadata-refresh hold before dispatch; health, completion and relation-browser
effects must not add SQL after DML. Publish the returned runtime directly and
require owner-requested refresh. Preserve acknowledged statement evidence after
tab closure, disconnect, local audit failure and late event delivery. No global
lock may span authentication or server work.

## Limits of SQL effect classification

Reject explicit unreviewed UDF, external-function and procedure calls in every
expression. Plain reviewed built-ins are defensible: Snowflake says user-defined
functions cannot share system-defined names, but namespace/quoting/arity still
need explicit validation. See
[function naming](https://docs.snowflake.com/en/developer-guide/udf-stored-procedure-naming-conventions).

This cannot prove that a named table has no indirect external effects. Row-access
policies can invoke external functions, including on rows selected by UPDATE or
DELETE. Views and other dependencies can conceal calls too. External side effects
can survive transaction rollback and can repeat through Snowflake's own remote
service retries. Native one-submission protection cannot prevent those retries.
Do not claim that only the displayed table changes or that external effects are
exactly once. A stronger guarantee requires separately reviewed target/dependency
policy and permissions, with unavailable metadata treated honestly. See
[CREATE ROW ACCESS POLICY](https://docs.snowflake.com/en/sql-reference/sql/create-row-access-policy),
[row-access policy behavior](https://docs.snowflake.com/en/user-guide/security-row-intro)
and [external-function side effects](https://docs.snowflake.com/en/sql-reference/external-functions-best-practices).

## Pinned implementation map

The inspected checkout uses sqlparser **0.63.0** with `visitor`. A pure child
module `src-tauri/src/safety/snowflake_dml.rs` can reuse the existing private
classification veto and unrestricted-write analysis without exposing them.
Suggested APIs are `analyze(original_sql, pinned_database)` and
`compile(original_sql, inputs, pinned_database)`, returning private-field
candidate/canonical types rather than a caller-constructible admission token.
This is a proposed implementation map, not parser execution evidence.

- INSERT/UPDATE/DELETE are tuple `Statement` variants. INSERT targets use
  `TableObject::TableName`; `columns` is `Vec<ObjectName>`. VALUES rows use
  `Parens<Vec<Expr>>` and `row.content`, with nonempty bounded rows and exact
  column arity. Require INTO, explicit unique columns and exactly a VALUES body.
- Require the INSERT source Query's WITH, ordering, limits, fetch, locks,
  FOR/settings/format clauses and pipe operators absent. Reject every INSERT
  modifier, alias, conflict/returning/output clause and multi-table field unless
  deliberately admitted later.
- UPDATE/DELETE require one plain `TableWithJoins`, no joins/alias/arguments,
  hints/version/ordinality/partitions/JSON path/sample/index hints. UPDATE
  assignments must be unique one-component `AssignmentTarget::ColumnName`s.
  Reject FROM/USING, returning/output, ordering/limit and optimizer modifiers.
- `ObjectName` contains `ObjectNamePart`, including dynamic Function parts.
  Accept exactly three Identifier parts, double quotes or unquoted names only;
  preserve quoted values and uppercase unquoted lookup components. Reject dynamic
  IDENTIFIER targets, empty/NUL/oversized names and foreign pinned databases.
  Existing provenance name helpers implement PostgreSQL/MySQL semantics and are
  unsuitable for Snowflake identifier matching.
- Use closed expression allowlists by position. Do not reuse the common read
  function allowlist: it includes aggregate/window forms. A function-free first
  grammar should reject every Function and special function-like AST form,
  subquery, sequence reference, wildcard and unreviewed cast/operator. An
  exhaustive fallback rejects future parser variants.

The existing parameter pipeline is `extract_query_params`,
`rewrite_for_classification`, `validate_inputs` and `compile_statement` in
[params.rs](../src-tauri/src/params.rs). Bound original input first; rewrite only
placeholders for strict parsing; validate inputs and compile the original SQL
with positional offset zero. Compare the exact canonical wire SQL and ordered
TypedBind fields against any claimed bound representation. Compiler-generated
CAST/PARSE_JSON wrappers do not authorize equivalent user-supplied functions.
Independently bound original/rewritten/wire SQL and the final helper frame;
the compiler's existing eight-MiB bind budget alone is insufficient. Keep the
separate proposed 1,000-occurrence DML limit described above.

## Future effect-qualification runner

Keep the existing 24-read manifest/protocol immutable. A mutation qualification
needs a distinct workflow and fresh approval binding exact objects, templates,
binds, effects and any recovery permissions. Reuse source/runtime pins, the
exclusive consumed grant, strict framing, reserve-before-write child ownership,
deadlines, redacted receipts and identity-checked authentication-root cleanup.
Add typed boundary-ACK retention, future DML count parsing, bounded synthetic-row
comparisons and transition-specific observation checks. The read runner's stable
sample comparator would reject legitimate transaction-ID changes.

Do not defeat its permanent failure latch with automatic finally-ROLLBACK SQL.
Recovery requires explicit approved reservations and trusted same-session
eligibility; malformed protocol or foreign identity stops SQL entirely. Local
child/root cleanup does not prove remote rollback or object removal.

A first effect plan should use a uniquely named, owner-provisioned disposable
table with synthetic columns, no policies/indirect dependencies and no concurrent
writers. Bind the qualified name and expected schema; do not adopt existing
objects or use OR REPLACE. Compile an empty baseline, BEGIN plus exact mutations,
projected-row checks, ROLLBACK and unchanged-baseline checks; then a separate
COMMIT sequence and result verification. Include every observer and verification
statement in a new ledger, with its actual budget. Require accepted initial
settings instead of silently changing AUTOCOMMIT or abort-on-error.

A temporary table cannot prove visibility from a second session. Independent
commit visibility requires another explicitly approved connection or later read
plan. Provisioning, drop and uncertain-outcome recovery also need exact approved
scope. If outcome is uncertain, report retained remote objects rather than
inventing successful cleanup. Helper-direct effects still do not qualify Studio
policy, scheduling or UI behavior. Implement and synthetically fault-test the new
compiler/protocol, then review its executable boundary before asking for approval.

## Delivery order and verification gates

1. **Policy and pure contracts.** Settle explicit opt-in migration; add strict
   SDK-DML grammar and typed outcome/summary parsers. Own `domain.rs`, `safety.rs`,
   `params.rs`, profile normalization/form/persistence tests and a focused pure
   driver module. Prove AST/batch/quoted-name escapes, unsupported modifiers,
   malformed counts, precision and bind mismatch fail before submission.
2. **Helper/supervisor transport.** Own `helper.mjs`, `snowflake_process.rs` and
   their process/actual pinned-SDK loopback fixtures. Prove one guarded native
   dispatch, exact impact/pin/ACK association, synchronous callback safety, lost
   ACK, rejected SQL, malformed trailing frames, cancel/drop and bounded cleanup.
3. **Adapter and Studio.** Own `snowflake_external.rs`, `drivers/mod.rs`, the
   stored DML action/execution branch and fixtures. Prove eleven-statement budget,
   one SDK connection, stale authority/policy rejection, pending/implicit
   transaction support, rollback recovery and no generic extra observer. Verify
   status count differs from status-row count and multi-joined count is separate.
4. **Application integration.** Own contracts/IPC, store/editor/review UI and
   browser/native tests. Prove zero automatic metadata/health SQL, shared-console
   exclusion, delayed events, tab/session retirement and truthful command/count/
   transaction messages. Audit or history failure cannot rewrite known outcomes.
5. **Separate account acceptance.** After local validation, package/source-pin
   qualification and independent review, obtain a new bounded approval for a
   disposable isolated table and exact mutations. Test INSERT/UPDATE/DELETE
   effects, zero/multiple rows, exact descriptors, explicit BEGIN plus rollback
   and commit, failed DML/abort behavior and real cancellation uncertainty.
   Design the cleanup/verification SQL in that approval; do not borrow an expired
   read canary's authority or create objects implicitly.

Broader grammar, bulk/large-value binds, joins, MERGE, grid editing, cross-database
targets, relation-kind coverage, DDL implicit-commit handling, recovery UX and
owner hands-on acceptance remain open after this initial workflow. Local
synthetic fixtures establish transport/application behavior, not Snowflake data
effects or complete DataGrip replacement acceptance.
