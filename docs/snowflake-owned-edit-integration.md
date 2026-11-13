# Snowflake owned edit integration

This internal integration supports the required grid/CSV workflow described in
[the editing contract](snowflake-grid-editing-design.md). It adds no application
editing command or capability. Full DataGrip replacement, broader codecs, native
staging/review and real Snowflake acceptance remain required.

## Pending-owner metadata transport

`mutationRead` accepts exactly `owner`, `expected`, `kind`, `target` and `credits`.
The expected context has explicit user, role, database, schema, warehouse and
session fields. A canonical private owner is required; a caller cannot supply
SQL, bindings, result limits or transaction commands. Targeted descriptors accept
only exact database/schema/name components and base/temporary table types.

The helper constructs one fixed statement for each kind:

| Kind                     | Fixed operation                          | Maximum rows | Aggregate bytes |
| ------------------------ | ---------------------------------------- | -----------: | --------------: |
| `context`                | Full context observation                 |            1 |          65,536 |
| `quotedIdentifierPolicy` | Session quoted-name parameter            |            1 |          65,536 |
| `currentTransaction`     | Current transaction ID                   |            1 |          65,536 |
| `exactRelation`          | Exact information-schema relation lookup |            3 |       2,097,152 |
| `columns`                | Ordered complete column projection       |        1,001 |       1,048,576 |
| `describe`               | DESCRIBE TABLE columns                   |        1,001 |       1,048,576 |
| `zeroRowResolution`      | Fully qualified SELECT with LIMIT 0      |            0 |       1,048,576 |

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
authorize mutation. A complete owner-scoped data-preview route remains required
before manual continuation can obtain new prepared snapshots.

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
   the exact settled blocked operation ID, drains/retires owners across profile
   save, disconnect and shutdown, blocks metadata restore while any run or manual
   owner remains, and blocks ordinary Studio SQL, context/schema switches and
   relation browsing before they can invalidate `BrowseState` on a live `Pending`
   owner.

## Required next integration and acceptance

Implement owner-scoped relation data preview/refresh so manual continuation can
prepare new snapshots without test-only hooks. Then expose application
staging/review/submit, truthful pending commit/rollback UI states, CSV batching
and broader lossless codecs beyond the current narrow scalar/text/integer
subset. None of the internal driver/Studio APIs alone enables public user
editing.

Run real disposable-table acceptance only under a fresh reviewed account plan;
earlier approvals are consumed or expired. The owner identified
`SANDBOX.BILLIE_TESTING` as the disposable location. Finish normal packaged-app,
restart, interruption, million-row and hands-on acceptance. Do not enable generic
committed-success editing as a substitute for pending manual work.

Source-scoped results belong in the [verification ledger](replacement-verification.md).
The normal packaged app and private project hub remain unchanged until an
authorized packaging/publication milestone.
