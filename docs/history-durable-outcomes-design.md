# Durable original history outcomes: next increment

Status: design only. Schema 6 does not implement this outbox. This is a required
correctness prerequisite, not DataGrip replacement acceptance.

Generic and typed completion/audit retry payloads currently live in memory.
The history ticket and owner lease survive a crash, but an original result whose
history write failed does not. Proven dead-owner recovery must therefore keep its
outcome unknown unless completion was already durable. Typed transaction-boundary
receipts have no history ticket or owner binding today.

## Proposed ordering and authority

Use the existing metadata SQLite database for a bounded, SQL-free obligation
record, keyed by a stable operation identity. Reserve capacity and bind ordinary
operations before dispatch. Capture an immutable original outcome in its own
transaction before trying history, audit or ownership settlement. Then atomically
apply completion, exact idempotent audit tuples, binding release and outbox
settlement. Retries compare the original payload and reject conflicts.

Suggested APIs are `reserve_owned_operation`, `capture_original_outcome` and
`settle_original_outcome`. The payload is a strict whitelist of owner identity,
optional exact history ticket, original completion/evidence and frozen audit
tuples. It must exclude SQL, credentials, bound values, result rows, arbitrary
error strings, `HistoryEntry` and complete SDK result objects. Typed audits need
stable identities/timestamps and `record_audit_once`; the current `audit()` creates
a new identity on each retry. Preserve original acknowledgement, rejection and
unknown classifications without claiming verified database effects.

Keep initial capture separate from projection so an UPDATE, audit INSERT or
binding DELETE fault does not roll back the captured original. If the initial
outbox INSERT/commit also fails, retain the original in bounded memory, retain
ownership, warn that it is not durable and block ordinary admission. Capacity
reservation does not guarantee a later write succeeds during ENOSPC, I/O failure,
read-only storage or lock contention. A second journal adds cross-store recovery
and restore complexity and still cannot guarantee writes during disk loss.

## Crash, restore and privacy contract

| Durable point at death | Required restart behavior |
| --- | --- |
| No original capture | Existing proven dead-owner interruption/unknown fallback; no inferred remote result |
| Original captured; projection incomplete | Settle the exact original locally, with one stable audit obligation and no engine request |
| Settlement committed; memory not cleared | Exact retry is harmless and cannot duplicate audit or change original evidence |
| Corrupt, conflicting or expected-but-missing obligation | Fail safely; preserve current evidence and refuse replacement/replay |

Recover durable originals before the interruption fallback, holding the existing
owner proof and admission coordination. Restore must reconcile current local
obligations before undo/replacement and reject conflicts before publication.
Backup-only records may preserve historical evidence but grant no local execution
authority. Clear, opt-out, deletion and pruning still suppress/redact visible
history; settlement never recreates a missing row or SQL. This is logical privacy
authority, not physical erasure of old SQLite pages, WAL or backups.

Transaction boundaries need an explicit operation identity and owner binding
independent of history; do not invent history entries. Preserve emergency
rollback when recording is unavailable. Its result may remain volatile and must
be reported that way. Ordinary reservation failure must prevent dispatch.

The next schema migration must exclude older live publishers, including Studios
with a boundary operation but no history binding. Prove this before changing
storage authority; physical history/profile/audit fencing alone cannot capture
an acknowledgement still held by an older process. Test exclusion and admission
races using actual separate processes and the stable admission gate.

## Required resource decisions

The read-only field inventory found: history completion readers cap encoding at
65,536 bytes, without the matching write-size guard; audit strings have no
general size validation; typed retry maps retain full results containing unbounded
errors/session state. New outbox validation must use its own strict whitelist
without silently tightening legacy history decoding.

Proposed initial limits, requiring implementation and exact boundary tests:

| Resource | Proposed limit / behavior |
| --- | --- |
| Entire canonical obligation | 32 KiB encoded UTF-8, including identities/evidence/audits; bounded serialization, no truncation |
| Ordinary reservations | 32 per Studio, 128 per metadata root; include in-flight and captured/unsettled records |
| Separate emergency rollback pool | 32 per Studio, 128 per root; ordinary work cannot use it. Recording failure/full capacity still permits recovery rollback in a preallocated volatile session slot with an explicit warning |
| Root reserved payload | 8 MiB, 256 total slots × 32 KiB; reserve worst-case bytes before ordinary dispatch |
| Retained payload memory | 64 slots / 2 MiB per Studio plus at most 256 KiB replay scratch; account for temporary copies and replace full result objects in retry maps |
| Replay transaction | At most eight records / 256 KiB; keyset batches, yield between them, stop the pass on write failure |
| Owner files | 4,096/root admission ceiling, counted safely before creation; refuse new ownership at the ceiling. Safe dead/unreferenced collection is still required for practical lifetime behavior |

DML counts must remain canonical unsigned decimals of at most 38 digits. DDL
target identity has three components of at most 255 characters/1,020 UTF-8 bytes
each; enforce a conservative 4 KiB raw target limit and compare with the frozen
reviewed identity. It is object identity metadata, not statement text. Audit
tuples need derived operation UUID IDs of at most 48 bytes, connection IDs at most
256 bytes, a 64-character lowercase hexadecimal digest, closed action/outcome
enums, JavaScript-safe timestamps and at most two tuples. Validate execution
accounting against admitted statement count rather than applying one route's
100-statement limit to every producer.

Existing limits are 32 connections and generic jobs per Studio, four connections
per profile, 32 prepared plans / 8 MiB and 60-second preview/preparation expiry.
They do not bound all typed queued tasks or root-wide operations. Reserve the
original slot until atomic settlement commits, and preserve the one-pending-
rollback-per-session rule. These logical budgets do not bound SQLite pages/WAL,
overall RSS or backup size; existing metadata backup limits are 20 / 64 MiB each.

Limits are transitional safety gates, not proof of DataGrip workload parity.
Never evict unsettled
originals to admit another action. Full-budget refusal must be atomic before
ordinary dispatch and leave rollback/reconciliation available.

Owner-file reclamation requires proven owner death, no remaining references and
exclusive coordination. Never unlink/recreate a referenced ownership inode.
Ticket/privacy ancestry compaction needs a separate proof covering old backups,
late completions and SQL resurrection. An outbox byte limit does not bound those
existing lifetime stores, audit storage or all restore proof handles; keep their
acceptance gates open until demonstrated. Complete applicable DataGrip history,
SQL error logs and daily workload acceptance remain separate required work.

## Reproducible acceptance to add

Inject history UPDATE, audit INSERT and ownership DELETE faults separately in a
real local SQLite store. Prove original capture committed, kill/reap only the
owned child, remove the fault and restart. Assert exact original evidence, one
stable audit obligation and no additional engine request. Use an actual isolated
PostgreSQL query for the generic process case; qualify typed helper behavior with
a synthetic SDK without claiming external Snowflake effects.

Add kill points before capture, after capture, during settlement and after its
commit. Repeat privacy clear/opt-out/deletion, live-owner exclusion, imported
backup provenance, plain/encrypted restore, exact/full capacity and malformed or
conflicting records. Include typed DML, DDL and boundary acknowledgements.
Initial-capture failure is a separate test: require a truthful volatile warning
and admission/close holds, not a durability claim. Run changed UI flows through
restart and preserve the original remote-effect uncertainty.

The read-only design review used `gpt-6.1-sol` at high effort, with Tier 2 graph
evidence and direct source reads. Relevant paths had no recorded coverage gaps;
that is a best-effort signal, not completeness proof. No implementation or
external qualification was performed by that review.
