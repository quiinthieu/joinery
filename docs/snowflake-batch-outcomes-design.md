# Durable Snowflake batch outcomes: schema8 storage

Status: locally verified storage foundation; runtime integration remains open.
See the [source-scoped ledger](replacement-verification.md#snowflake-batch-storage-13-november-2026-local-foundation). This document scopes the
persistence foundation. It does not enable generated mutations in the application
or establish DataGrip replacement readiness. The capacities below are provisional
internal refusal limits, not accepted daily-workflow limits. The existing normal
app remains the separately recorded schema-7 package.

## Schema7 baseline and missing storage

The preceding implementation uses schema7:

| Source | Relevant contract or limitation |
| --- | --- |
| [persistence.rs](../src-tauri/src/persistence.rs) | `SCHEMA_VERSION = 7`; metadata tables use `_v7`; migration excludes live Studio owners before taking the SQLite writer. |
| [outcomes.rs](../src-tauri/src/persistence/outcomes.rs) | `reserve_owned_operation`, `capture_original_outcome`, `settle_original_outcome` and `abandon_reserved_operation` provide immutable reservation/capture and atomic settlement. `settle` clears descriptor and payload; only compact boundary evidence survives. |
| [history_owner.rs](../src-tauri/src/persistence/history_owner.rs) | OS owner proof and the shared/exclusive admission gate coordinate recovery, migration and restore. Dead-owner recovery settles captured originals before uncaptured-ticket fallback. |
| [snowflake_batch.rs](../src-tauri/src/drivers/snowflake_batch.rs) | `BatchCapture::encode/decode` provides versioned, closed, SQL-free evidence: request shape, sequential steps, exact identities/counts, unavailable-count flags and optional terminal summary. It has no persistence integration. |
| [snowflake_batch_tests.rs](../src-tauri/src/drivers/snowflake_batch_tests.rs) | The maximum capture test proves base64 capture fits within 30KiB. It does not prove a complete durable envelope fits within 32KiB. |
| [outcomes.rs](../src-tauri/src/persistence/outcomes.rs) | `merge_for_restore` removes imported operation/binding rows and preserves current authority. A batch archive needs a separate historical merge contract. |

The existing payload bound is 32KiB. Active operation limits are 128 per root
and 32 per owner **per pool**, with a separate 65,536-marker lifetime ceiling.
Those limits do not reserve a future manual-transaction finish or rollback slot.

Adding batch bytes to the current payload alone would lose them on settlement.
Removing the last operation binding after a settled manual `pending` receipt
would also leave an idle, live remote transaction without restore exclusion.

## Records and authority

The focused `persistence/batch_outcomes.rs` module adds five tables:

| Table | Responsibility |
| --- | --- |
| `batch_operations_v8` | Separate reservation/capture/settlement markers and bounded transient payload; preserves the existing ordinary outcome format. |
| `batch_captures_v8` | Immutable complete envelope and digest, keyed by operation UUID; separate provenance identifies imported historical evidence. |
| `manual_transactions_v8` | Stable native transaction UUID; connection, local session and helper-generation identities; observed helper owner/remote transaction ID; derived disposition bookkeeping. |
| `manual_transaction_members_v8` | Immutable operation-to-transaction membership, attempt ordinal and frozen prior-pending count. |
| `manual_transaction_bindings_v8` | Current-only owner nonce and reserved final-rollback capacity. Historical records alone never confer execution authority. |

Allocate the native transaction UUID before the first dispatch. The helper owner
and remote transaction ID may be learned later; bind each once and reject a
conflicting observation. Never derive live authority from a descriptor, digest,
imported record or remote transaction ID.

Attempt membership and pending-batch count are different. A rejected attempt
still has stable membership but does not increase the successful-pending count.
For a manual batch, the helper's `priorBatches` describes preceding pending
batches; a successful `pending` result adds this batch afterward. Finish reports
the preceding pending count. Validate these relationships against the frozen
reservation and the existing [helper contract](../snowflake-helper/src/mutation-batch.mjs).

Earlier `pending` captures and their stable audits remain byte-identical after a
later rollback or finish. Readers combine original member evidence with a later
verified disposition linked to the same transaction. Preserve the whole-group
rollback disclosure and the continuation's explicit rollback-prior permission.

Keep the group OS-owner binding between batches, without holding an idle shared
admission gate. Restore must prove that binding dead or refuse replacement.
Restart does not adopt or reconnect a manual transaction: death without verified
disposition records local uncertainty, not a remote commit or rollback.

## API and recording sequence

Keep existing ordinary/typed APIs compatible. Add opaque, non-IPC-deserializable
`BatchPlan`, `BatchReservation` and `ManualTransactionLease` types:

```rust
reserve_owned_batch(owner, gate, descriptor, plan, optional_transaction_lease)
    -> AppResult<BatchReservation>;
reserve_owned_batch_finish(owner, gate, descriptor, transaction_lease, action)
    -> AppResult<BatchReservation>;
OriginalBatchOutcome::capture_in(reservation, evidence, termination, finished_at, buffer)
    -> AppResult<OriginalBatchOutcome>;
capture_batch_outcome(owner, gate, reservation, original)
    -> AppResult<OutcomePhase>;
settle_batch_outcome(owner, gate, reservation, original)
    -> AppResult<OutcomePhase>;
batch_evidence(operation_id)
    -> AppResult<Option<HistoricalBatchEvidence>>;
manual_transaction_evidence(transaction_id, after_ordinal, limit)
    -> AppResult<ManualTransactionPage>;
```

`BatchPlan` freezes the closed request fingerprint, unit count, membership intent
and a driver-computed worst-case capture size. The fingerprint binds request
mode, owner, rollback-prior permission and ordered unit kinds; the operation
digest also binds the generated request and pinned context without retaining
SQL, binds or context strings. A caller-provided size estimate is insufficient.

Reserve the operation, stable started audit, membership and required capacity
atomically before dispatch. Capture the original compact evidence before
observer/publication callbacks. Accept ACK-without-identity and interrupted
prefixes as evidence of uncertainty; never manufacture missing query IDs,
counts, helper summaries or disposition proof. A closed local termination reason
may accompany an incomplete prefix without replacing a known helper result.

Extend existing settlement so one SQLite transaction archives the exact envelope,
writes idempotent audits, updates derived group bookkeeping, removes operation
bindings and clears the transient payload. Release the group binding only after
verified closure, initial rejection before a transaction was opened, or OS-proven
owner death. Unknown and no-original outcomes refuse continuation and retain
their live binding until that proof. Explicit retirement while the Studio remains
alive needs a separate authority contract. Any failure rolls back the entire
settlement; identical retries succeed and changed bytes conflict.

An initial history-less receipt path can follow existing boundary recording.
Any generated-query history integration must define a distinct closed projection;
manual `pending` must not be represented as generic committed success.

## Whole-envelope and capacity requirements

The **complete** canonical envelope, including descriptor, membership reference,
timestamps, base64 evidence and syntax, must fit within 32KiB. Stream into the
existing supplied buffer; do not allocate a second base64 payload. Extend the
driver with a trustworthy maximum-size calculation and validate the complete
worst-case envelope before any SQL. Test maximal legal escaped metadata and all
500 units, not an assumed 2KiB allowance. Reject overflow without truncation.

The following are provisional implementation limits, **requiring capacity and
workflow validation**. They are not accepted replacement limits:

| Provisional capacity | Purpose |
| --- | --- |
| 64MiB ordinary retained envelopes per root | Bound historical payload growth independently of active-operation memory. |
| 128 live manual groups/root, 32/owner | Bound idle bindings and held rollback credits; these provisional group limits are distinct from active-operation pool limits. |
| One operation marker plus 32KiB final-rollback credit per new group | Reserve cleanup capacity before BEGIN; ordinary work and commit cannot consume it. |
| At most 64 metadata records/page; decode one envelope at a time | Bound read/recovery working sets; measure actual scratch allocations. |

Charge reservations and retained envelopes, transferring their logical charge
at settlement rather than counting one obligation twice. Define separate bounded
metadata/membership accounting. SQLite pages, journals and transient physical
copies require disk-headroom validation; 64MiB is not a disk or RSS promise.
Retain the existing marker ceiling until an explicit replacement is approved;
reserved rollback markers must be protected from ordinary exhaustion. A bounded
final cleanup attempt is not a promise of unlimited retries or successful remote
rollback.

A protected rollback rejection consumes its one reserved attempt. Settlement
records local uncertainty and retains the original rejection without
minting another credit. Rejected continuations and commits may retain the prior
open transaction and its unused rollback credit. These distinctions require the
focused regressions recorded in the ledger, which now pass. Runtime integration
remains separate.

An OS-gated volatile rollback token must require a genuine current group lease
and its pre-reserved credit. It can permit cleanup while SQLite is unavailable,
then promote that exact obligation when writing recovers. It cannot turn missing
or corrupt ordinary rows into new reservations. Disk failure can still prevent
durable capture; retain the bounded memory obligation and report that limit.

## Migration, recovery, privacy and restore

Raise the schema version only with version-specific schema validation and backup
allowlists. Preserve schema7 definitions and validate backups from versions1–8.
Fence cached schema7 privacy writers by renaming profiles/history/audit to `_v8`;
keep unrelated lifecycle and console tables unchanged. Prove all Studio leases
under admission EX before the migration writer, and retain exact-version checks
after acquiring an operation's SH gate.

The implementation updates `persistence.rs`, `outcomes.rs`, `history_owner.rs`,
`history/lifecycle.rs`, `recovery.rs` and console retention's explicit range to
`5..=8`. Encrypted manifests/streams use `recovery_crypto.rs` and
`recovery_stream.rs`. Driver capture bounds and focused persistence tests are
included; runtime integration requires a separate implementation assignment.

Dead-owner recovery settles captured originals before abandoning uncaptured
reservations. Group death adds local disposition uncertainty without rewriting
earlier captures, fabricating finished ACKs or issuing remote SQL.

Restore unions validated immutable historical captures and memberships, rejecting
same-ID/different-content conflicts before publication. Imported captured but
unsettled evidence remains historical; do not apply its completion or fabricate
finished audits. Strip imported runtime/group bindings and quota credits. Only
valid current local authority survives replacement, and live idle groups block
restore. Retain acquired dead-owner proofs through merge/copy.

Keep existing privacy lineage and history suppression authoritative. No archive
contains SQL, cell values, binds, credentials, arbitrary errors or raw expected
context. Historical evidence must never reconstruct erased query text. Retention
or erasure rules for the remaining SQL-free metadata need an explicit policy;
never erase live cleanup authority as a side effect of clearing history.

## Required implementation gates

Current evidence includes real SQLite fault/idempotence/validation cases, three
owned-child death scenarios, two genuine preserved-schema7 migrations and an
actual encrypted schema8 roundtrip with three settled originals and a live idle
group. The encrypted case preserves historical bytes/membership/audits and strips
imported authority/credit while retaining the live source unchanged. It does not
cover encrypted captured-only restore or undo restoration. The
[ledger](replacement-verification.md#snowflake-batch-storage-13-november-2026-local-foundation)
distinguishes these passes from logical quota guards, unmeasured full-capacity/RSS,
the unproved delayed preclaim race and the remaining runtime/remote/native gates.

- Real SQLite: complete maximal envelope and one-byte overflow; strict malformed
  decoding; reserve/capture/settle faults at archive, audit and binding writes;
  identical retry and conflicting retry; no partial settlement.
- Membership: rejected attempts versus pending count; two pending members followed
  by continuation rollback or explicit finish; earlier capture/audit bytes remain
  unchanged; wrong owner, session, helper generation and count are rejected.
- Capacity: ordinary byte/marker exhaustion preserves the reserved rollback path;
  commit cannot spend its credit; group/metadata/import budgets fail before SQL
  or restore publication. Validate the proposed capacities before adopting them.
- Process: actual SIGKILL after durable capture but before settlement, after ACK
  but before capture, and while manually idle; preserve known evidence, report
  unknown when warranted, and perform no replay/reconnect. Use disposable/offline
  fixtures; Snowflake account qualification requires separate authorization.
- Backup: schema7 migration and cached-writer fencing; versions1–8 and encrypted
  roundtrips; live idle-group restore refusal; foreign captured/settled evidence
  remains readable but grants no local authority; conflict/privacy/size refusal
  leaves the current store intact.

Passing these gates would establish bounded durable recording within the tested
scope. UI editing, target/inspection authority, transaction cardinality enforcement,
account qualification, lifetime compaction, installation and DataGrip acceptance
remain separate requirements.
