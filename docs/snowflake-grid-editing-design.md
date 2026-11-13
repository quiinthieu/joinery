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
assumes success means committed and patches retained rows immediately. Its
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
and check actual database counts inside its owned transaction. The compiler has
no dispatch or application caller and grants no execution authority. Snowflake
grid editing and CSV import remain disabled.

Twelve focused regressions cover hostile names/values, ordering, full predicates,
NULL/empty/default distinctions, exact decimals, byte-exact text, overlapping
mutations, invalid/generated columns and aggregate limits. SQL parser checks
establish the generated single-statement shape only. The
[verification ledger](replacement-verification.md#snowflake-mutation-compiler-13-november-2026)
records current commands and results; neither these tests nor the official SQL
references replace actual Snowflake semantic and account acceptance.

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
   The helper currently rejects a third statement ID in one request; introduce
   a separate bounded batch admission/ID budget including control and metadata
   reads. Do not relax that global ceiling or loop the old operation unchanged.
   Size durable evidence against schema-7's compact payload limit before choosing
   the runtime batch size; the compiler's row limit is not that proof.
3. **Application grid and import workflow.** Retain comparison-grade originals,
   allow staged inserts/updates/deletes and show the exact generated SQL/bindings
   for approval. Represent pending manual-transaction work separately from
   committed results. Revert, submit, commit and rollback must update the correct
   snapshots and invalidate affected tabs truthfully. CSV preview and every batch
   must retain the same target/preview authority and exact Snowflake coercions.
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
