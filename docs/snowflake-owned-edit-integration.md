# Snowflake owned edit integration

This integration underpins the reviewed Snowflake relation grid and CSV staging
workflow described in [the editing contract](snowflake-grid-editing-design.md).
Public Tauri IPC commands (`prepare_snowflake_grid_plan`,
`cancel_snowflake_grid_plan`, `execute_snowflake_grid_plan`,
`preview_snowflake_grid_transaction_relation`, and
`finish_snowflake_grid_transaction`) and `RelationBrowser` inline editing,
lossless scalar/temporal/binary/JSON codecs (`<= 1,000` columns), server-side
slice sort/filter/offset paging, `<ReviewDialog>` plan approval and cancel
recovery, multi-batch CSV import (`<= 100,000` rows across `500`-row batches),
and manual `Pending` -> `Reload pending preview` -> continuation -> `Commit` /
`Rollback` controls are now wired (`c033d00`, `682b1ed`, `a794426`) on
explicitly writable `externalBrowser` `reviewedDml` profiles. Fresh-plan live
[`SANDBOX.BILLIE_TESTING` qualification](snowflake-sandbox-qualification-plan.md),
million-row live-account acceptance, Developer ID signing/notarization, and
hands-on owner acceptance remain required before retiring DataGrip.

## Pending-owner metadata and data-preview transport

`mutationRead` accepts `owner`, `expected`, `kind`, `target`, optional
identifier `columns` (for `"preview"` only), and `credits`.
The expected context has explicit user, role, database, schema, warehouse and
session fields. A canonical private owner is required; a caller cannot supply
SQL, bindings, result limits or transaction commands. Targeted descriptors accept
only exact database/schema/name components and base/temporary table types (with
`schema !== "INFORMATION_SCHEMA"` enforced for `"preview"`).

The helper constructs one fixed statement for each kind:

| Kind                     | Fixed operation                                                                                | Maximum rows | Aggregate bytes |
| ------------------------ | ---------------------------------------------------------------------------------------------- | -----------: | --------------: |
| `context`                | Full context observation                                                                       |            1 |          65,536 |
| `quotedIdentifierPolicy` | Session quoted-name parameter                                                                  |            1 |          65,536 |
| `currentTransaction`     | Current transaction ID                                                                         |            1 |          65,536 |
| `exactRelation`          | Exact information-schema relation lookup                                                       |            3 |       2,097,152 |
| `columns`                | Ordered complete column projection                                                             |        1,001 |       1,048,576 |
| `describe`               | DESCRIBE TABLE columns                                                                         |        1,001 |       1,048,576 |
| `zeroRowResolution`      | Fully qualified SELECT with LIMIT 0                                                            |            0 |       1,048,576 |
| `preview`                | Quoted 1..=1,000 column SELECT on exact table with bounded WHERE/ORDER BY/OFFSET and LIMIT 501 |          501 |       2,097,152 |

The row ceilings include overflow sentinels where applicable. Serialized column
metadata and emitted batches count toward the byte ceiling. Existing frame,
batch, column and global credit limits still apply. The native supervisor checks
stream shape, cumulative counts/bytes and terminal consistency independently.
An honest truncated result is transport evidence only, never complete metadata.

Receive, dequeue and the synchronous SDK dispatch boundary require the exact
retained owner object. Reads queued behind a completed finish cannot execute.
Ordinary SQL and metadata stay excluded during manual ownership. Reads consume
ordinary submission capacity and leave reserved finish/rollback capacity intact.
Queued cancel/discard removes only that read; active cancellation retires the
helper without issuing a transaction boundary. Reaping establishes local child
retirement, not remote rollback.

The native driver now brackets preparation with eleven fixed observations: full
context, quoted-name policy and transaction; exact relation; ordered columns,
DESCRIBE and zero-row resolution; exact relation again; then context, policy and
transaction again. All sources must match the native binding. Idle preparation
requires no transaction; owner reads require the exact retained transaction.
Commit brackets the full owner context with six observations. A bracket holds
the same lane through final admission. Abandonment or protocol loss retires the
helper; coherent drift refuses new work. No automatic retry repairs drift.
Partial, discarded, interrupted, ambiguous or changed observations cannot
authorize mutation. Owner-scoped pending relation preview (`grid_fresh_preview`,
`408c275`) brackets the bounded `"preview"` read inside twelve fixed
observations on the same lane (initial context/policy/transaction, exact
relation, columns/DESCRIBE/zero-row resolution, `"preview"` rows, exact
relation again, and final context/policy/transaction) before minting a fresh
one-use continuation `editSnapshotId`, and `SnapshotStore::revoke_helper_session`
scopes revocation to the exact `HelperGeneration`. Test-only continuation hooks
have been removed.

Snowflake documents SELECT and transaction behavior in its
[transaction reference](https://docs.snowflake.com/en/sql-reference/transactions),
and describes [SHOW PARAMETERS](https://docs.snowflake.com/en/sql-reference/sql/show-parameters)
and [DESCRIBE TABLE](https://docs.snowflake.com/en/sql-reference/sql/desc-table).
The inspected command pages do not establish a command-specific guarantee that
these metadata commands preserve every pending transaction. Actual same-session
pre/post transaction and eventual commit/rollback effects remain an external
qualification gate. Official references and synthetic SDK fixtures do not prove
that gate.

## Studio-owned original recording

`execution/batch_outcome_recording.rs` retains each admitted candidate before
SQLite reservation. It retains outcome memory, the operation gate, exact helper
observer and any compact original independently of the originating caller/tab.
Local storage failures retry the same reservation/capture/settlement; they never
reconnect or replay SQL. Known originals and encoder failures cannot select the
no-original abandonment path.

Initial manual groups pin the exact helper before dispatch and retain the same
pre-authentication rollback safety slot through pending/unknown state. Ordinary
pool exhaustion or session-map removal cannot allocate away that protected
buffer. This is local recording ownership; the driver must separately retain its
lane-bearing private manual transaction owner.

The synchronous dispatch handoff must arbitrate abandonment under the cell lock
and retain the actual terminal watch before returning. Immutable native operation
identity, kind, parsed batch request and physical helper must match the admitted
candidate. A mismatch or dispatch unwind must preserve the obligation and block
abandonment. Only a verified pre-dispatch refusal or a genuine appropriate native
terminal with no batch evidence permits a no-original settlement.

The compact original replaces the live terminal report after encoding. Capture
and settlement remain distinct, and both must finish before memory/gates release.
Idle group release requires exact local reap proof and no outstanding originals;
uncertain remote disposition stays unknown. Shutdown waits for owned work and
child cleanup before flushing these registries. Metadata restore refuses to
replace storage while a retained recording or manual group remains.

This layer is bounded by the existing ordinary/safety memory pools, 64 retained
cells and 32 groups. These logical bounds are not measured allocator/RSS or
lifetime-capacity acceptance. Actual SQLite fault and owned-child tests qualify
local behavior only; synthetic batch receipts do not qualify Snowflake effects.

## Typed native dispatch and protected rollback

`PreparedGridOperation` consumes one native snapshot and retains an owned lane
guard, exact compiled payload, operation ID and digest. Shared logical memory is
reserved before JSON allocation; a bounded borrowed frame computes the digest
without cloning the payload. Final native binding/expiry validation shares the
shutdown and browsing gates with the synchronous admission callback. A callback
cannot dispatch twice. Actual supervisor acceptance determines whether a request
was queued; a local callback error cannot turn an accepted request into proof of
no dispatch.

The caller retains `GridManualTransaction` before dispatch. Its private owner
excludes ordinary SQL and metadata, survives recording failure and allows only
the same physical helper to finish. Recording captures the actual native terminal
synchronously. Recording failure blocks commit and further mutations; safety
rollback performs no freshness SQL. SQL-free settlement must name the exact
blocked operation; settling an older receipt cannot unblock a newer attempt.
Unknown outcomes or abandoned execution retire the helper without SQL replay.

Compact pending authority is minted while the genuine native Pending operation
is available. It retains no live operation or full terminal report. Protected
rollback may retain an immutable predecessor original across capture/settlement
faults and settles that original before promoting the rollback receipt. A queued
zero-step admission refusal must retain the exact prior transaction and its own
original; it is never classified as no dispatch.

Only the native adapter can mint `GridNoDispatch`, after actual nonadmission.
It binds the exact helper, operation and request. A refused Continue/Commit can
precede protected rollback using that proof. The unreleased schema8 contract adds
a settled `noDispatch` membership witness, with no fabricated original; restored
witnesses are historical only and can authorize no new execution. This changes
the unpublished schema8 validation contract, not the packaged schema7 database.
Compatibility with older evaluation schema8 writers is not claimed.

If protected rollback itself is refused before queueing, its exact recording
credit may already be claimed. The adapter retains proof through the recording
callback, then retires the helper rather than suggesting that a new ID can reuse
the credit. Reaping proves local cleanup, never remote rollback.

The Studio run owner must retain the native manual handle, recording authority,
originals and safety slot independently of the IPC caller. If initial admission
has already reserved/pinned a group and then proves no dispatch, retire that
exact helper, preserve no-original membership, and release only after genuine
reap. A refusal before any durable reservation has no such obligation. Neither
case may silently discard known originals. These application policies remain
integration work; the adapter is not a public grid command.

## Studio TaskTracker grid run owner

`execution/snowflake_grid.rs` now owns generated-edit execution on `dml_tasks`
independently of the initiating IPC caller or tab:

1. Sessions retain the typed `Arc<SnowflakeExternal>` alongside `Arc<dyn Driver>`
   so generated-edit preparation, retirement observation and physical helper
   closure remain available without widening the generic driver trait.
2. Each run claims logical outcome memory and recovery activity before spawning
   into `dml_tasks`, acquires the session/manual-owner lane, runs native
   preparation and freshness bracketing, reserves and pins the exact immutable
   operation ID/request/digest in SQLite, and registers `ManualOwner` before
   dispatch. Unpolled or detached callers leave admitted runs and durable
   evidence alive.
3. `StudioGridRecorder` encodes the compact original synchronously while the
   genuine native `Operation` is borrowed, minting compact pending or
   queued-admission-rejection rollback authority without retaining `Operation`
   or cloning `OriginalBatchOutcome`. Storage faults retain both the predecessor
   and rollback originals across ordered settlement.
4. Initial manual `NoDispatch` after durable reservation retires the exact
   physical helper before abandoning the no-original membership so post-reap
   reconciliation releases the group truthfully. Post-`Pending` `Continue` or
   `Commit` `NoDispatch` retains proof for protected SQLite rollback; queued
   zero-step `Admission` rejection retains its compact original and prior
   transaction authority for protected rollback.
5. `reconcile_snowflake_grid_owners` notifies `GridManualTransaction` only for
   the exact settled blocked operation ID, clears stale `RejectedAdmission`
   recovery once its cell settles, drains/retires owners across profile save,
   disconnect and shutdown, counts active grid runs and open manual owners in
   `resource_usage`, blocks metadata restore while any run or manual owner
   remains, and blocks ordinary Studio SQL, transaction controls, context/schema
   switches and relation browsing before they can invalidate `BrowseState` on a
   live `Pending` owner (`f74ac0d`). Protected rollback admission and global
   `flush_batch_outcomes` serialize under `flush_lock` while accepting non-adjacent
   manual member ordinals (`m.ordinal <= prior.ordinal`) across intervening
   `NoDispatch` witnesses.

## Reviewed IPC, RelationBrowser staging, and manual transaction UI

Public Tauri IPC commands (`prepare_snowflake_grid_plan`,
`cancel_snowflake_grid_plan`, `execute_snowflake_grid_plan`,
`preview_snowflake_grid_transaction_relation`,
`finish_snowflake_grid_transaction`) and desktop ACL capabilities (`c033d00`,
`682b1ed`, `a794426`) expose reviewed Snowflake grid mutations through `Studio`
and `SafetyGovernor`:

1. `prepare_snowflake_grid_plan` consumes one linear `editSnapshotId` (from
   idle `preview_relation` or owner-scoped
   `preview_snowflake_grid_transaction_relation` across server-paged, sorted,
   and filtered `PreviewSlice` windows), compiles the closed
   `DELETE`/`UPDATE`/`INSERT` batch without dispatching SQL, and stores
   `Action::SnowflakeGrid` behind `PreparedPlan { reviewed_snowflake_grid: true }`
   for `<ReviewDialog>` confirmation, passphrase, and one-use approval token
   validation. Cancelling `<ReviewDialog>` invokes `cancel_snowflake_grid_plan`
   to atomically restore the linear `editSnapshotId` without requiring a network
   reload when the session generation and context remain intact.
2. `RelationBrowser` enables inline cell updates (`NULL` toggle, `Revert`),
   whole-original row deletion, paginated new-row insertion (`DEFAULT` / `NULL`
   / text), RFC-4180 multi-batch CSV row staging up to `100,000` rows in
   `500`-row batches (`2,000` scalar binds per batch, file upload or pasted text
   with case-sensitive header matching, progress tracking, explicit cancel, and
   mixed-batch unknown-postimage warnings), `"Resume session metadata"`
   recovery, and a **Pending Snowflake grid transaction** banner exposing
   `Reload pending preview` (preserving active sort/filter/page slices),
   `Commit transaction`, and protected `Rollback transaction` (which remains
   enabled even when `recordingError` blocks continuation and commit).

## Required next integration and acceptance

Lossless Snowflake edit codecs now cover `TEXT` / `VARCHAR`, `BOOLEAN` (with
wire `"TRUE"`/`"FALSE"` normalized to canonical `"true"`/`"false"` in the native
driver), exact `NUMBER(1..=38, 0..=38)` (including `NUMBER(38, 38)`), finite
`FLOAT` / `DOUBLE` (preserving `-0.0` vs `0.0` via `TO_BINARY(TO_VARCHAR(...))`),
even-hex `BINARY`, `DATE`, `TIME(0..9)`, `TIMESTAMP_NTZ(0..9)` /
`TIMESTAMP_LTZ(0..9)` / `TIMESTAMP_TZ(0..9)`, and canonical `PARSE_JSON`
`VARIANT` / `ARRAY` / `OBJECT` on base/temporary tables up to `1,000` columns
(`682b1ed`), alongside server-paged, sorted, and filtered browsing (`a794426`).
Spatial (`GEOGRAPHY`, `GEOMETRY`) and unstructured file-reference columns remain
explicitly read-only in grid editing.

Run real disposable-table acceptance only under the fresh reviewed
[`SANDBOX.BILLIE_TESTING` qualification plan](snowflake-sandbox-qualification-plan.md);
earlier approvals are consumed or expired. Finish live account, restart,
interruption, million-row, Developer ID/notarization, and hands-on owner
acceptance on the verified schema-8 `Joinery.app` bundle.

Source-scoped results belong in the [verification ledger](replacement-verification.md).
The private project hub and `/Applications` installation remain unchanged until
an authorized publication/installation milestone.
