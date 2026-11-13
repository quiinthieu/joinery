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

| Kind | Fixed operation | Maximum rows | Aggregate bytes |
| --- | --- | ---: | ---: |
| `context` | Full context observation | 1 | 65,536 |
| `quotedIdentifierPolicy` | Session quoted-name parameter | 1 | 65,536 |
| `currentTransaction` | Current transaction ID | 1 | 65,536 |
| `exactRelation` | Exact information-schema relation lookup | 3 | 2,097,152 |
| `columns` | Ordered complete column projection | 1,001 | 1,048,576 |
| `describe` | DESCRIBE TABLE columns | 1,001 | 1,048,576 |
| `zeroRowResolution` | Fully qualified SELECT with LIMIT 0 | 0 | 1,048,576 |

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

This is a single-read primitive. The driver still needs a lane-owned complete
freshness bracket: full context, quoted-name policy and transaction observation;
exact relation plus all inspection sources; repeated observations and final
owner/revision/transaction checks. Partial, discarded, interrupted, ambiguous or
changed observations cannot authorize mutation. No complete manual refresh or
data-preview route is enabled by these seven metadata kinds.

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

## Required next integration and acceptance

Connect the typed compiler/prepared snapshots to a lane-bearing driver adapter,
the atomic recording handoff and a TaskTracker-owned Studio run owner. Retain the
native manual owner even when recording fails or the initiating caller vanishes.
Implement the complete pending-owner freshness bracket and data refresh before
enabling later submissions. Then expose staging/review/submit and truthful pending
commit/rollback states, CSV batching and broader lossless codecs.

Run real disposable-table acceptance only under a fresh reviewed account plan;
earlier approvals are consumed or expired. The owner identified
`SANDBOX.BILLIE_TESTING` as the disposable location. Finish normal packaged-app,
restart, interruption, million-row and hands-on acceptance. Do not enable generic
committed-success editing as a substitute for pending manual work.

Source-scoped results belong in the [verification ledger](replacement-verification.md).
The normal packaged app and private project hub remain unchanged until an
authorized packaging/publication milestone.
