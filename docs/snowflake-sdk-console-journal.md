# Retained Snowflake adapter and local console evidence

## Scope and status

This increment connects the [private transport/compiler](snowflake-sdk-console-transport.md)
to a retained adapter and adds a local SQLite journal. Local checks and bounded
review pass; see the [ledger](replacement-verification.md) for results, including
one unexplained intermittent synthetic helper-start failure. There is
no new application command or usable console UI. The
[complete console design](snowflake-sdk-console-scripts-design.md) remains the
required workflow, including native admission, durable execution ownership,
result/history integration, recovery and owner acceptance.

## Retained physical connection

A separate default-off `reviewedConsole` profile consent requires external-browser
SDK authentication and a writable policy. Existing generated DML/DDL permissions
do not imply this consent, and console consent does not imply those permissions.
Legacy profiles deserialize it as false. Closed account qualification profiles
continue to reject this authority.

The private adapter claims one physical helper session under its serialized lane.
Claiming performs no SQL; the connection's earlier identity/version bootstrap is
still disclosed. Claim clears generated grants/catalog/options and permanently
seals ordinary SQL, health probes, metadata and generated operations on that
connection. It does not clear rollback-only recovery. A second claim cannot
replace the existing owner. Dropping the owner closes the connection.

Each borrowed run verifies the canonical immutable compiled script and dispatches
only its contiguous next unit. The owner can record intent before calling next.
The final ownership/closed check and supervisor admission are serialized with
local revocation. Stop, cancellation, error, result limit and recording failure
prevent the same run's suffix. There is no reconnect, retry, implicit transaction
settlement or suffix resume. A settled run may leave the physical console alive
for another separately reviewed run; unfinished/abandoned ownership closes it.

The mandatory asynchronous acknowledgement recorder runs exactly once when the
sticky original ACK becomes visible, before publishing columns, rows or result
completion. Recording has a bounded deadline. Recording failure suppresses result
publication, drains/discards the original bounded stream and preserves original
ACK evidence separately from delivery/recording errors. This ordering seam alone
does not establish durability: the future execution owner must provide the actual
journal write, in-process outbox, cross-run admission hold and shutdown barrier.

Passive response context preserves individually unknown fields. It does not
establish coherent transaction/autocommit state or authorize generated actions.
Transport row counts are not affected-row or commit evidence.
The private adapter currently has a 65-second run-unit delivery deadline, followed
by bounded terminal draining. Configurable long-running query behavior and large
result delivery still need workflow-level implementation and qualification;
these fixed foundation limits do not establish DataGrip replacement suitability.

## SQL-free local journal

Metadata schema version 2 adds run and step tables in an atomic migration that
preserves existing settings/groups. A missing or incompatible current journal is
rejected rather than silently recreated. Opening metadata validates existing
evidence but does not recover or take ownership of another live console.

One private process-held OS lease owns all journal mutations across runs. A second
store may read evidence, but cannot mutate it, recover live work or restore
metadata while the lease is held. The macOS lock is private, non-symlinked and
bound to the held inode/root. Process death releases the OS lock; it does not
delete a permanent attempt record or establish a remote outcome.

Run admission atomically deduplicates a stable client request ID and binds it to
the original run, session, connection, plan digest and unit count. A retry with a
different proposed run ID/time returns the existing owner; changed authority is
rejected. SQL-free lookup can recover correlation without resubmitting SQL. Run
metadata, durable per-step intent, original outcome and monotonic observations
use committed transactions and revision checks. Original outcomes cannot be
replaced by cancellation, delivery loss or local recording failure.

Original outcomes distinguish acknowledgement, definitive rejection, proven local
non-dispatch and unknown. Non-dispatch needs authoritative evidence that the
adapter never admitted the operation to the supervisor. Absence of a report,
generic transport error or an unresolved persisted intent is insufficient.
After acquiring the exclusive lease, recovery seals unresolved intents as
unknown/interrupted, preserves recorded originals and never resumes execution.

Accounting distinguishes planned units, durable intents, acknowledged/rejected/
unknown originals, proven non-dispatch and the untouched suffix. Intent is not
proof of helper admission or server submission. A not-attempted total includes
the untouched suffix and proven local non-dispatch, with both components exposed.
Counts describe top-level compiled units, not statements inside procedures or
scripting bodies. Terminal reason and its observations are immutable; later
delivery/recording observations remain separate and monotonic. Unknown originals
take precedence over ordinary cancellation/failure labels.

The journal stores no SQL, binds, result rows, credentials or raw error messages.
It stores bounded identifiers, passive context and closed observations/errors.
The initial limit is 100 units per run, 10,000 retained run identities and 16 KiB
per JSON record. Capacity fails before new admission or restore; nothing is
evicted to make room. A practical archival/retention workflow that preserves
deduplication and recovery evidence remains required before daily replacement.

## Metadata restore

Restoring a version 1 or 2 metadata backup requires the exclusive journal lease.
The selected backup is validated and upgraded only in a private copy. Before
replacement, current original outcomes and client identities are merged into that
copy. Compatible current evidence supersedes older backup evidence. Conflicting
identity, contradictory/newer backup evidence or an oversized union rejects the
restore. Backup-only incomplete runs become interrupted history. Restoring
settings cannot roll back a known original outcome or erase a deduplication
identity. The selected backup bytes remain unchanged.

## Verification and remaining integration

Focused local checks use synthetic child SDK sessions and disposable SQLite
stores, including a real subprocess lock/crash test:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib console -- --test-threads=1
pnpm validate
```

These checks cannot establish actual Snowflake transaction effects, LAST_QUERY_ID
semantics, permissions, detached transaction cleanup, forced-power-loss durability
or normal packaged UI behavior. Existing account approvals do not authorize new
queries. The retained Studio owner still needs immutable review/admission,
policy-revocation checks, stable client correlation, mandatory durable recording
and outbox holds, local receipt reconciliation, truthful result/history events
and native/UI scheduling that suppresses hidden SQL across runs. SQL completion
and other authoring features remain required in permanent console mode. Actual
account qualification, one-million-row interaction/export, normal packaging and
hands-on acceptance stay open alongside all other required DataGrip capabilities.
