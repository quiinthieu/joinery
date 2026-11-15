# Snowflake SDK reviewed DML workflow

**Status: implemented and locally tested; actual Snowflake DML and full
DataGrip replacement acceptance remain open.** The SDK admits only public
classified reads; a separate typed action now supports a bounded, reviewed
single INSERT/UPDATE/DELETE statement on the same SDK session when the owner
explicitly opts in. Its tests use a synthetic SDK and do not establish remote
data effects or authorize account activity. Broader DML, grid editing,
cross-database operations, DDL and actual-account transaction acceptance remain
required. Read alongside the
[explicit transaction contract](snowflake-sdk-transactions-design.md).

## Policy and supported scope

The Snowflake profile has a separate SDK DML opt-in, `reviewedDml`, defaulting
false for new profiles and for old profiles without the field. It is independent
of Safe Mode and explicit transaction controls; a non-ReadOnly boundary policy
does not imply DML consent. The checkbox names reviewed INSERT, UPDATE and DELETE
and warns about possible warehouse billing and transaction effects. Safe Mode
still controls per-plan confirmation and passphrase requirements. ReadOnly,
OAuth/local-application profiles and unsupported authentication routes deny DML.
Changing authentication or switching into OAuth clears the opt-in; switching
back does not restore it automatically. Production-profile password-gated
normalization is preserved. Authentication and policy changes invalidate
pending plans.

The DML opt-in does not enable grid/table editing, DDL, arbitrary transaction
SQL or AUTOCOMMIT mode changes. Preserve verified HTTPS/443, no SSH, no stored
password and existing production/group normalization.

The supported grammar is deliberately narrow:

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

Require an explicit, fully qualified SQL name and compare its parsed database
component with the pinned current database. The implementation preserves quoted
identifier components and never splits a dot inside a quoted name, concatenates
SQL fragments or rewrites the reviewed target after approval. This is
name-based targeting: it does not bind an immutable Snowflake object identity,
and it does not qualify relation-kind identity for grid editing. Review copy
warns that concurrent DDL, temporary-name shadowing, object defaults, policies
and dependencies can affect name resolution or indirect effects. A matching
browser label or catalog name is not an immutable object ID; concurrent server
changes can still create a time-of-check/time-of-use race. Do not claim an
atomic object binding or that only the named table can be affected.

## Native admission, binds and helper contract

The SQL editor's parameter/review UI stores a distinct SDK-DML plan. The plan
binds original SQL, validated scalar parameters, a canonical bound statement,
session ID and server-held observation revision; its stored fingerprint covers
the exact wire SQL/binds and effective policy. After owning the Studio lane,
execution consumes the plan once under the configuration gate, rederives current
policy, checks deadline/approval/passphrase and session closure, then validates
adapter authority. The configuration gate is not held over network awaits. A
queued plan remains revocable until consumption.

Reuse `params::validate_inputs` and canonical recompilation. Require exact
recompiled wire SQL and ordered bind equality, including repeated named slots;
never trust a public BoundStatement independently. Keep supported scalar values,
server numeric/boolean casts, precision, SQL/frame/aggregate-byte bounds and at
most 1,000 parameter occurrences for this DML gate. The general
parameter compiler currently permits 1,024; apply the narrower DML cap explicitly
rather than claiming the existing compiler already enforces it. Do not interpolate values, accept raw SQL
fragments or admit array binds/staged binding. The existing positive SDK
`arrayBindingThreshold` prevents hidden staged-binding setup for this scalar
path. Snowflake documents positional bindings and their size limitations in
[Executing statements](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#binding-statement-parameters).

The dedicated helper `dml` operation carries exact reviewed SQL, scalar binds
and expected private principal/session identity. Rust owns AST admission; the
helper validates the closed payload, established identity pin and bounds at
admission and dispatch. It matches only to `Impact::Mutating`; public read
admission remains unchanged. A DML kind does not independently prove its SQL
grammar. Never adopt a supplied pin or use profile login/role as a fallback for
observed identity.

One operation owns one guarded SDK execute, one exact SDK request ID and its
allowance through terminal/drain. Keep mandatory transport-source verification,
the native one-submission latch and redirect restrictions. No retry, reconnect,
request-ID resubmission or replacement session is part of this workflow. SDK
documentation describes request-ID resubmission on the same connection, but that
is a separate recovery design, not permission for automatic replay here.
[Resubmitting requests](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#resubmitting-requests)

## Statement evidence and exact affected counts

Retain statement acknowledgement and its exact registered identity before
callbacks, status parsing, draining or event delivery can fail. The typed result
separates statement outcome (`succeeded`, `rejected`, `unknown`), optional exact
affected count, summary-validation status, final runtime, retirement and a
bounded safe error. Pre-admission failures return an error; post-admission
results preserve known evidence, including when the UI or history writer is
gone. ACK means statement success, not independently verified transaction
disposition or committed data effects.

The read adapter returns helper `rowsSeen` through QuerySink::complete and
Driver::execute's u64, which would report one for a DML status row. The dedicated
typed DML driver result preserves ACK, summary validity and drain failure
separately rather than forcing them through `AppResult<u64>`. QuerySink can still
render the bounded status result, but viewer closure, retention failure or
result-store failure cannot replace remote evidence. Studio's generic
connection-loss-to-OutcomeUnknown rewrite consults that evidence rather than
overwriting acknowledged success.

The pure summary parser accepts only the reviewed descriptors below. The local
contract publishes counts as exact nonnegative decimal strings, never JavaScript
numbers:

| Kind   | Required status           | Separate observation                                 |
| ------ | ------------------------- | ---------------------------------------------------- |
| INSERT | `number of rows inserted` | No invented update/delete count                      |
| UPDATE | `number of rows updated`  | `number of multi-joined rows updated`, when reported |
| DELETE | `number of rows deleted`  | No invented insert/update count                      |

Require one complete, untruncated row; unique names; a reviewed descriptor set;
authoritative fixed numeric wire types; and nonnegative integral decimal text.
The implemented cap is two columns, one row and 64 KiB including descriptors
and cells. Normalize only reviewed wire-type casing, not arbitrary aliases.
Parse with checked integer arithmetic and publish decimal strings. Reject null,
fractional, negative, duplicate, unexpected, oversized or overflowing fields.
A failed parser makes the count unavailable while preserving prior success ACK.
Do not use zero to mean unavailable. Exact live SDK descriptor casing and
optional-column variants remain an account acceptance gate; unexpected variants
fail closed without replay.

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
The DML execution path uses five baseline statements, one DML statement and five
final statements: **at most eleven explicit statement executions**, ending early
on failure. The initial coherent observer may separately cost ten statements
when authority is created. Target inspection or later grammar must have its own
declared budget, not be hidden inside eleven. These counts exclude
authentication/session setup and do not promise a server execution-cost cap.

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
every tab sharing the session. Preserve existing results. The durable
metadata-refresh hold is set before dispatch; health, completion and
relation-browser effects do not add SQL after DML. The result and durable history
retain statement evidence after tab closure, disconnect, local audit failure
and late event delivery. The hold persists until the owner explicitly selects
Refresh and a new observation succeeds. No global lock spans authentication or
server work. History stores operation/outcome/count evidence, not bind values.

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

The implementation uses sqlparser **0.63.0** with `visitor`. The pure admission
module is `src-tauri/src/safety/snowflake_dml.rs`; it reuses the existing private
classification veto and unrestricted-write analysis without exposing them. Its
candidate/canonical types keep admission authority private rather than exposing a
caller-constructible token.

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
  grammar rejects every Function and special function-like AST form,
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
separate 1,000-occurrence DML limit described above.

## Remaining effect qualification

Keep the existing 24-read manifest/protocol immutable. An account effect
qualification still needs a distinct workflow and fresh approval binding exact
objects, templates, binds, effects and any recovery permissions. Reuse
source/runtime pins, the exclusive consumed grant, strict framing,
reserve-before-write child ownership, deadlines, redacted receipts and
identity-checked authentication-root cleanup. Local DML now retains typed
boundary acknowledgement, parses bounded status counts and compares observed
transitions; the read runner's stable sample comparator would reject legitimate
transaction-ID changes and cannot stand in for mutation qualification.

Do not defeat the read runner's permanent failure latch with automatic
finally-ROLLBACK SQL. Any account mutation recovery still requires a separate
approved action and trusted same-session eligibility; malformed protocol or
foreign identity stops SQL entirely. Local child/root cleanup does not prove
remote rollback or object removal.

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
inventing successful cleanup. The synthetic/helper fault tests exercise the
integrated Studio policy, scheduling and UI evidence path, but they do not
qualify actual Snowflake data effects. Independent review and separate account
authorization are still required before an account write.

## Local implementation and remaining gates

The local increment implements the false-by-default profile opt-in, strict AST
and bind admission, typed SDK helper operation, one-use review plan, exact count
parser, durable metadata hold/explicit Refresh, statement outcome notice,
restart-safe history evidence and no-replay retirement for unknown outcomes.
Focused Rust DML coverage passes 51 tests. The full Node/helper/synthetic SDK
Studio suite passes 21 cases, including five new DML cases. After a later
graceful-shutdown history repair, all 15 Studio DML unit cases and full
`pnpm validate` pass: 671 frontend and 426 standard Rust tests, with 129 fixture
cases explicitly ignored. Seven headless browser DML/transaction flows and
43 actual disposable PostgreSQL regressions also pass. Helper coverage is 255
cases across the 107 actual-SDK loopback and 148 other helper runs. Three hidden
SDK native runs pass 20 categories and ten cleanup cycles each. These counts
establish local behavior only; exact commands and source limits are in the
[verification ledger](replacement-verification.md).

Independent reviews resolved material findings, including dropped-waiter evidence,
native metadata races, incoherent refresh and graceful shutdown. Current-source
normal macOS packaging, signature/resources/startup checks and the 336-input
source comparison pass. Before account testing, finish and review the bounded
disposable setup runner and obtain fresh execution/billing approval for the
isolated objects and exact statements. Test actual
INSERT/UPDATE/DELETE effects, zero/multiple rows, live status descriptors,
explicit BEGIN plus rollback and commit, failed DML/abort behavior and real
cancellation uncertainty. Design cleanup and verification SQL in that approval;
do not borrow an expired read canary's authority or create objects implicitly.

Broader grammar, bulk/large-value binds, joins, MERGE, grid editing, cross-database
targets, relation-kind coverage, DDL implicit-commit handling, recovery UX and
owner hands-on acceptance remain open after this initial workflow. Local
synthetic fixtures establish transport/application behavior, not Snowflake data
effects or complete DataGrip replacement acceptance.
