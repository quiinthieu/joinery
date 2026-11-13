# Snowflake grid editing and CSV import

## Required workflow and implementation boundary

The owner requires latest DataGrip functionality for Snowflake on macOS,
including reads, writes, schema changes and transactions. Safe grid insertion,
update, deletion, CSV import, pending changes, review, submit, commit and rollback
remain required. A generated-SQL compiler is a prerequisite; it does not make
these application workflows available or qualify account effects.

DataGrip's [data editor](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html)
supports pending-change previews, row insertion/deletion, submission and automatic
or manual transaction modes. Manual submission can leave work pending until
explicit commit or rollback. Joinery's existing generic `Action::Changes` route
assumes success means committed. It preserves original result rows and invalidates
editability after a successful commit; submitted values do not establish server
postimages. Its
`Driver::apply_target` return type cannot represent pending Snowflake work or
per-row/commit acknowledgement evidence. Do not enable the SDK's `table_editing`
capability by reusing that contract unchanged.

## Current compiler implementation

`src-tauri/src/drivers/snowflake_mutations.rs` provides the pure, closed compiler.
It emits ordered units with quoted identifiers, bound values, explicit casts and
an expected affected count of one. UPDATE and DELETE compare every retained
original column, including NULLs; text comparisons use explicit UTF-8 binary
conversion so insensitive collation cannot hide a changed original. Decimal
validation uses strings rather than floating point and rejects precision or
scale loss before emitting SQL. No key declaration establishes row identity.

The current codecs are text, BOOLEAN and fixed-point NUMBER (precision 1–38,
scale 0–37, with scale no greater than precision). Decimal input accepts an
optional minus sign, integer digits and an optional nonempty fractional part.
Exponents, leading plus signs and fractional places beyond the declared scale
are rejected, including extra zero places. Binary, floating point, dates/times,
semi-structured and spatial values remain required codec work; the compiler
rejects them explicitly rather than emitting an approximate comparison.

Compilation permits at most 500 units, 128 columns, 1 MiB per cell and 8 MiB of
accounted input/output. The accounting includes original values, emitted SQL,
identifiers and bindings; it is not a measurement of total process memory.
Repeated indistinguishable originals, recognizable postimage/original overlaps
and incomplete originals conflict. UPDATE to DEFAULT is currently unsupported.
An insert whose default/generated postimage is unknown cannot be mixed with an
UPDATE/DELETE; insert-only defaults remain supported. These are current limits,
not a narrowing of the replacement target.

The descriptor and original-row constructors validate shape and values; they do
not establish metadata provenance, snapshot freshness, permissions, uniqueness or
immutable target identity. The future runtime must derive them from retained
server-side inspection and lossless snapshots, bind the plan to current authority
and check actual database counts inside its owned transaction. Native snapshot
preparation now calls the compiler, but there is no dispatch caller and it grants
no execution authority. Snowflake grid editing and CSV import remain disabled.

Twelve focused regressions cover hostile names/values, ordering, full predicates,
NULL/empty/default distinctions, exact decimals, byte-exact text, overlapping
mutations, invalid/generated columns and aggregate limits. SQL parser checks
establish the generated single-statement shape only. The
[verification ledger](replacement-verification.md#snowflake-mutation-compiler-13-november-2026)
records current commands and results; neither these tests nor the official SQL
references replace actual Snowflake semantic and account acceptance.

## Owned-batch runtime integration

The internal helper protocol adds `mutationBatch` and `mutationFinish` on one
retained physical connection. The Rust supervisor retains separate typed batch
evidence. This work is not yet an application editing route: the compiler has no
dispatch caller, durable batch recording is not integrated, and grid/CSV
capabilities remain disabled. The
[ledger](replacement-verification.md#snowflake-owned-batch-protocol-13-november-2026)
records passing helper, parser, actual child-process, actual-SDK loopback,
resource/startup and full application checks, with their exact source scopes.
None establishes remote Snowflake effects or new UI readiness.

The helper admits at most 500 generated units and 510 independently identified
steps, including fixed context, settings and boundary observations. The complete
encoded request must fit the existing 1 MiB frame before queue admission. The
compiler's 8 MiB logical budget and 1 MiB cell ceiling do not promise transport
admission; larger generated requests still need a reviewed framing solution.
Each generated unit uses scalar binds and `MULTI_STATEMENT_COUNT=1`. The native
compiler must establish the SQL grammar before dispatch; helper shape checks do
not authorize browser-supplied SQL or prove metadata provenance.

Initial admission verifies idle state and exact observed context before BEGIN,
then retains the actual transaction ID. Quoted-name execution requires observed
`QUOTED_IDENTIFIERS_IGNORE_CASE=false`. Manual continuation requires the same
private owner, physical session, context and transaction ID, plus explicit
consent that a later whole-transaction rollback includes prior submitted batches.
Both request receipt and queued dispatch exclude unrelated SQL while owned work
is pending. Whole-batch submission capacity is reserved before the first SQL;
manual ownership also retains a separate five-statement finish allowance.
The guard's 65,536 spent-ID process ceiling still applies. Protected finish
capacity does not permit unlimited batches. Exhaustion must refuse further work;
it cannot silently reconnect or replay an open manual transaction. Large-import
capacity and a safe lifecycle policy remain replacement acceptance requirements.

DML summaries require exact `FIXED` metadata and decimal strings. Only affected
count one advances; a present UPDATE multi-join count must be zero. An omitted
optional multi-join count stays absent: the native generated UPDATE has no join.
No status-row count or earlier uniqueness query substitutes for affected rows.
Statement ACK, complete drain, exact count and transaction disposition are
separate facts. Interrupted execution/cancellation never authorizes blind cleanup
on a possibly active statement, and connection retirement never proves rollback.

A boundary ACK and an idle observation cannot alone prove commit or rollback.
The helper also checks the owned ID, session and state from
[`DESCRIBE TRANSACTION`](https://docs.snowflake.com/en/sql-reference/sql/desc-transaction),
whose output shape is illustrated in the official
[monitoring example](https://docs.snowflake.com/en/sql-reference/transactions#monitoring-transactions-and-locks).
Missing or contradictory disposition is unknown. A
[`COMMIT` can have no open transaction to commit](https://docs.snowflake.com/en/sql-reference/sql/commit#usage-notes).
Explicit transactions do not require silently changing AUTOCOMMIT; its value
does not affect statements inside BEGIN/COMMIT. Server aborts and observation
failures still require truthful unknown outcomes.

The compact versioned evidence contract retains ordered step roles, exact IDs,
ACK/drain/count states and the closed terminal summary without SQL or bindings.
Its strict decoder must reject malformed or reordered evidence, and maximum-size
proof must include the future durable descriptor/base64 wrapper within 32 KiB.
Raw binary fitting that limit does not establish whole-payload admission.

Before application dispatch is enabled, complete and verify the separate schema8
batch archive under the existing exclusive admission/owner fence. Schema7
settlement deletes the captured payload, so the new per-unit proof must be
retained atomically in a bounded durable record. Earlier pending manual receipts
remain immutable; a separate finish record resolves their explicitly bound
transaction membership. Reserve recording capacity for rollback before accepting
manual work. Restore imports historical proof without helper ownership, replay
or live execution authority. Capture/settle faults, caller loss, child death,
ordinary-quota exhaustion and restart require actual SQLite/process tests.
The [durable batch-outcome contract](snowflake-batch-outcomes-design.md) records
the locally verified schema8/API foundation and its remaining gates. Focused
SQLite/owned-child cases, full application validation, actual preserved-schema7
migration and existing PostgreSQL/browser workflows pass. Its provisional
capacities are not accepted daily workflow limits or established by protocol or
logical quota tests. No batch adapter, application staging or CSV dispatch is
enabled by storage alone.

## Database correctness constraints

Snowflake [does not enforce primary or unique keys on standard tables](https://docs.snowflake.com/en/sql-reference/constraints-overview).
Declared keys may help the user understand a table; they cannot authorize a
single-row edit. Keyless tables remain in scope. An arbitrary `ROW_NUMBER`,
`LIMIT`, or a pre-edit uniqueness count cannot manufacture a durable row identity.
Indistinguishable duplicate originals must cause a single-row edit to conflict;
the application must never silently choose one duplicate.

Snowflake's [transaction documentation](https://docs.snowflake.com/en/sql-reference/transactions)
specifies READ COMMITTED visibility: successive statements can observe different
committed data. An UPDATE's actual cardinality must be checked before committing;
an earlier SELECT count is insufficient. DDL can commit a transaction implicitly.
Failed statements do not necessarily abort prior work. Multiple users of one
connection share its transaction, so a grid rollback must not undo unrelated
console work. The documentation also cautions against concurrent INSERT/COPY and
DDL on the same object. A metadata fingerprint detects observed drift; it is not
an immutable object identifier or a lock against replacement.

[`FOR UPDATE`](https://docs.snowflake.com/en/sql-reference/constructs/for-update)
is supported only for hybrid tables and does not establish read stability even
there. Requiring hybrid tables would narrow the owner's target and is not an
accepted solution for standard-table editing.

## Ordered implementation

1. **Closed mutation compiler.** Compile ordered INSERT/UPDATE/DELETE units from
   a validated server-side descriptor and lossless originals. Quote each name
   component, bind cell values and generate casts only from a closed type model.
   Use every comparable original column in null-safe UPDATE/DELETE predicates;
   preserve NULL, empty text and DEFAULT separately. Each single-row unit expects
   exactly one affected row. Reject incomplete originals, duplicate assignments,
   indistinguishable repeated originals, unsupported codecs and resource overflow.
   Text originals need byte-exact UTF-8 comparison or an explicit unsupported
   collation result: a collated equality can match one concurrently changed row
   without detecting conflict. Decimal casts must never round an original or
   silently change a reviewed value; reject unrepresentable precision/scale.
   See Snowflake's [collation rules](https://docs.snowflake.com/en/sql-reference/collation)
   and [NUMBER conversion behavior](https://docs.snowflake.com/en/sql-reference/functions/to_decimal).
   Compiler output and fingerprints convey no execution or immutable-target
   authority. Do not deserialize a browser-supplied descriptor as trusted metadata.
2. **Owned batch execution and truthful receipts.** Introduce a dedicated typed
   SDK operation and durable outcome contract. Bind the reviewed batch to the
   retained session, context/catalog/inspection revisions and original snapshots.
   Corroborate `QUOTED_IDENTIFIERS_IGNORE_CASE=false` before relying on quoted
   spelling: Snowflake's [identifier resolution](https://docs.snowflake.com/en/sql-reference/identifiers-syntax)
   otherwise resolves quoted alphabetic names differently. Do not change that
   setting silently or assume its account default.
   Verify idle admission before BEGIN, own the lane through all statements and
   rollback/commit, and admit no unrelated SQL into that transaction. Check each
   actual DML count before committing. Any zero/multiple count or statement error
   stops the suffix and attempts rollback of the entire owned transaction.
   Report rolled back only after confirmation; failed/lost rollback leaves the
   executed prefix uncertain. Retain attempted, successful and unattempted unit
   states with committed, pending-manual, confirmed-rollback or unknown outcomes.
   Preserve original BEGIN/DML/COMMIT/ROLLBACK
   acknowledgements separately from verified transaction disposition; loss of a
   commit acknowledgement is unknown, never an automatic retry. Do not roll back
   an existing transaction on an admission failure. Cancellation, detached work,
   caller loss, shutdown and local persistence failures require explicit handling.
   Manual submission must not issue a fresh BEGIN or automatic COMMIT into an
   already owned pending transaction. Explain that a subsequent whole-transaction
   rollback also discards earlier submitted grid batches in that transaction.
   Ordinary helper operations retain their two-statement identity ceiling. Use
   the separate bounded batch admission/ID budget including control and metadata
   reads; do not relax the ordinary ceiling or loop the old operation unchanged.
   Size durable evidence against schema-7's compact payload limit before choosing
   the runtime batch size; the compiler's row limit is not that proof.
3. **Application grid and import workflow.** Retain comparison-grade originals,
   allow staged inserts/updates/deletes and show the exact generated SQL/bindings
   for approval. Represent pending manual-transaction work separately from
   committed results. Revert, submit, commit and rollback must update the correct
   snapshots and invalidate affected tabs truthfully. CSV preview and every batch
   must retain the same target/preview authority and exact Snowflake coercions.
   The SDK relation preview now retains a bounded native edit-snapshot token for
   supported codecs on an explicitly writable reviewed-DML profile, linking exact
   originals, target, inspection/catalog revisions, context and physical helper.
   Native preparation resolves row references; browser-supplied originals confer
   no authority. The token enables no grid/CSV submission or pending UI workflow.
   The generic preparation route also deduplicates by declared primary-key
   columns, which rejects distinct keyless originals. Use the separate Snowflake
   compiler and typed action rather than enabling that generic route.
   While a manual transaction is pending, the helper intentionally excludes
   ordinary preview/metadata operations. A complete manual refresh workflow needs
   an explicitly reviewed owner-scoped read contract. Do not relax ordinary SQL
   exclusion or manufacture postimages from submitted values.
   The current import integer codec routes non-MySQL values through PostgreSQL
   i64 limits; it cannot be reused for Snowflake NUMBER(38,0). Duplicate keys are
   not automatically invalid for standard Snowflake tables. Default expressions,
   policies, identity columns, temporary shadows and concurrent DDL remain visible
   hazards, not inferred guarantees.
   Snowflake's [INSERT reference](https://docs.snowflake.com/en/sql-reference/sql/insert)
   restricts structured/semi-structured expressions in VALUES; the required
   future JSON/ARRAY/OBJECT codec must use a supported generated shape rather
   than adding `PARSE_JSON` blindly to the scalar VALUES template.
4. **Real database, native and owner acceptance.** Execute the cases below on
   newly created synthetic tables in an explicitly approved scope. Finish normal
   packaged keyboard/UI, restart and owner acceptance. Offline protocol fixtures
   cannot establish Snowflake SQL semantics, row counts or commit effects.

A helper-owned sequence on a retained connection is the initial runtime design
candidate. An anonymous Scripting block is not automatically a transaction and
would need its own reviewed guard and acknowledgement model. If used later,
[SQLROWCOUNT must be captured immediately after DML](https://docs.snowflake.com/en/developer-guide/snowflake-scripting/dml-status):
the default 2025_01 behavior can clear it after non-DML statements. The compiler
does not choose or confer authority on either execution strategy.

## Next native integration contract

The source review on 13 November establishes the following sequence. The native
snapshot/codec foundation is now implemented; the typed dispatcher and application
editing callers remain incomplete. The external adapter retains
one native snapshot per target in a process-global bounded store, with opaque tokens,
exact physical helper identity, application session, full observed context,
catalog/inspection revisions, retained relation and ordered column metadata.
Move the exact lossless preview rows into that store; do not retain a second
native result copy or accept browser originals. Replacing a preview, refreshing
metadata, changing context, disconnecting or expiring the token revokes it.
`drivers/snowflake_edit_snapshot.rs` owns private
`NativeEditBinding`, `SnapshotStore::retain/prepare` and non-cloneable
`PreparedSnowflakeEdits`; `SnapshotMutation` contains only row indices and
closed assignments. Preview retention and an optional `editSnapshotId` response
field are integrated. There is no new IPC mutation command or editing capability.
A proposed separate `snowflake_external_grid.rs` adapter will own the lane-bearing
`PreparedGridOperation` and mandatory admission/original-recording callbacks.
Shared driver fields and Studio execution/outcome/group registries remain lead
integration work.

Preparation accepts only that token, row indices and closed cell assignments.
Resolve originals and consume a matching token atomically, then create a non-cloneable
compiled plan that continues owning its resource reservation. Validate both text
character and byte dimensions and corroborate closed NUMBER/BOOLEAN metadata;
reject ambiguous defaults, identity or unsupported codecs explicitly. Proposed
count/byte/expiry bounds require separate capacity acceptance and do not define
the owner's workload. The existing compiler remains the final shape/overlap guard.

The current store admits at most 64 live snapshots/plans and 16 MiB of logical
charged storage across sessions; a preview is at most 500 rows / 2 MiB. Snapshot
expiry is five minutes and prepared-plan expiry is one minute. Outstanding plans
retain their budget after token consumption, even if revoked, until dropped.
Invalid matching intent consumes its token and requires a fresh preview. Foreign
or stale requests cannot consume the owner's token. These are provisional bounds,
not RSS measurements or acceptance of large-import capacity. Unsupported codecs
and capacity refusal leave the preview read-only with no token; contradictory
supported metadata or malformed original values are errors.
Preparation reserves selected native mutation clones plus the compiler's full
8 MiB allowance while retaining the preview charge, so concurrent retained work
can refuse even a small preparation below the count ceiling. Caller-owned
response/intent copies, transient current bindings and allocator overhead are
outside this logical retained-store accounting. Their actual memory and lifetime
capacity still require measurement before workflow acceptance.

Every replacement preview advances an attempt generation and revokes its target
before its first await, including failed or abandoned replacements. Final native
retention checks that generation under the same browsing/store lock order. An
older in-flight preview cannot recreate authority after a newer attempt is
abandoned. Metadata refresh, replacement inspection, context invalidation and
shutdown revoke prepared plans as well as stored tokens. Preparation owns the
native session lane and submits no SQL. A prepared plan is not dispatch authority:
the future adapter must call `validate_current` while holding its lane and must
still obtain fresh remote observations before submission.

The adapter must hold its lane from fresh preflight through durable admission
and original capture, validate the entire encoded request against the transport
frame before SQL, and serialize final dispatch with shutdown. Reserve outcome
memory before spawning, reserve durable capacity and initial rollback credit,
arm the outcome owner, and pin the exact helper before invoking the adapter.
Retain exact compact evidence before callbacks, disconnect or publication.
Persistence failure keeps the original bytes, gate and reservation for SQL-free
retry. A successful driver call must never collapse pending work into generic
committed success.

Studio-owned outcome cells and manual-group owners must outlive tabs and
disconnected sessions. Groups retain their lease, safety-slot ownership and a
native retirement observer captured before dispatch. Reaping permits local
release only after every retained original settles. An obligation with no
retained original may instead be explicitly settled as no-original by its owner;
that path must never discard known evidence. Reaping proves no remote rollback.
Recovery, cancellation and shutdown must include these registries. Caller loss
cannot drop an acknowledged outcome.

Initial automatic/manual preflight may use ordinary metadata while idle. Preparing
new edits for manual continuation cannot promise fresh target inspection through
those same paths: the pending helper correctly excludes them, and its existing
owned checks do not establish fresh complete column/target metadata. Review and
add a closed owner-scoped verification/read contract before enabling that workflow.
Preserving old catalog revisions alone does not prove freshness. Commit requires
the separate owned-session observation and disposition checks. Safety rollback
uses the exact private same-session owner without requiring fresh pre-boundary
context/settings or an edit snapshot, then verifies post-boundary disposition.
These dependencies do not make manual mode, broader codecs or CSV optional.

## Reproducible acceptance cases

| Case | Required evidence |
| --- | --- |
| Quoted names and hostile cell text | Exact component quoting and placeholders in unit tests; real table readback shows text stored literally, with no extra statements |
| Keyless distinct row | Edit one retained row by original values; exactly one row changes after commit |
| Declared but duplicate key | Two rows sharing a declared PK never count as uniquely identified; full originals and actual affected counts govern |
| Identical duplicates | Single-row UPDATE/DELETE affecting two rows rolls back, including earlier mutations in that batch |
| Stale original | Concurrent non-key update causes conflict and whole-batch rollback; no silent overwrite |
| Collation and decimal casts | Case/accent/trim changes remain conflicts despite collated equality; fractional overflow cannot round to a different row/value |
| Atomic mixed batch | Insert/update/delete succeeds together; a late constraint/count failure leaves no earlier mutation committed |
| Lossless values | NUMBER(38), fixed scale, NULL/empty/default, Unicode, supported dates/times/binary/semi-structured values retain their exact intended meaning; unsupported types are explicit blocking gaps |
| Competing transaction | Existing console work survives a rejected grid admission; separate grid rollback cannot discard it |
| Manual mode | Submit leaves visible pending work; other sessions see it only after commit; rollback restores results without calling it committed |
| Network/cancel/ACK loss | Deterministic barriers around BEGIN, each DML and COMMIT preserve known/unknown evidence; no automatic SQL replay on reconnect or restart |
| DDL/context/policy drift | Retained plans expire when observed context/metadata changes; externally concurrent name replacement is not misrepresented as prevented |
| CSV batches | Same target across preview/prepare/submit; exact large decimals, validation errors, cancellation and partial prior-batch commits are shown truthfully |
| Normal app | Actual UI staging/review/submit/revert/commit/rollback and keyboard paths work end to end after restart; screenshots alone do not qualify them |

The owner's permission to create tables under `SANDBOX.BILLIE_TESTING` identifies
the eventual disposable scope. It does not revive an expired plan or authorize
new billable activity. Prepare the exact SQL, names, request budget, cleanup and
window for fresh approval before account execution. Existing application-table
data is outside that scope. No account SQL is issued by this design work.
