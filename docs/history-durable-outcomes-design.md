# Durable original history outcomes: implementation and acceptance

Status: schema-7 storage/runtime implementation passes full validation, actual
PostgreSQL crash recovery, browser restart/reopen and preserved-v6 migration
checks. All 73 synthetic SDK and 27 offline runner cases pass; two optimized
performance cases remain ignored. The normal SDK-bearing ARM64 app from clean
`d6cfb2a` passes package/resource/signature/license and guarded helper-startup
checks with zero authentication/SQL. It is local and uninstalled; normal UI,
SSO and owner acceptance remain open. The prior schema-6 app is preserved.
This is a required correctness prerequisite, not DataGrip replacement acceptance.

The first source-stable core run (`outcomes-core-first`, 13 November 2026)
compiled but failed: 544 passed, 112 failed, four ignored. Storage validation and
schema-7 backup integration were corrected; the complete passing validation has
1,038 frontend and 720 standard Rust cases. Initial failures remain in the ledger.
The preceding actual PostgreSQL process regression reproduced loss of a known
completion after a history UPDATE fault and process death. Evidence and final
checks are recorded in the [verification ledger](replacement-verification.md).

Runtime review required three corrections: ordinary BEGIN/COMMIT must also gate
on generic recording failures; admission guards must precede driver callbacks;
and original results must be captured before retirement/close callbacks. New
regressions cover these boundaries and pass full validation. Independent review
found no remaining material issue within its recorded scope.

In the schema-6 baseline, generic and typed completion/audit retry payloads live in memory.
The history ticket and owner lease survive a crash, but an original result whose
history write failed does not. Proven dead-owner recovery must therefore keep its
outcome unknown unless completion was already durable. Typed transaction-boundary
receipts had no history ticket or owner binding in that baseline.

## Implemented ordering and authority

Use the existing metadata SQLite database for a bounded, SQL-free obligation
record, keyed by a stable operation identity. Reserve capacity and bind ordinary
operations before dispatch. Capture an immutable original outcome in its own
transaction before trying history, audit or ownership settlement. Then atomically
apply completion, exact idempotent audit tuples, binding release and outbox
settlement. Retries compare the original payload and reject conflicts.

Keep an explicit durable Reserved → Captured → Settled phase witness, immutable
capture digest and stable operation/audit identities. Reservation plus owner and
optional ticket binding commit together. Capture payload plus phase commit
separately; projection, exact audit tuples, binding release and the Settled marker
commit together. Release the payload reservation only after that settlement.
Exact late retries compare retained authority; conflicting retries or an expected
captured payload that is missing/corrupt fail safely. Removing the payload must
not turn a settled boundary into a new operation. Budget compact settled markers
and deduplication authority separately, or prove existing ticket/audit authority
suffices for every producer, including history-less boundaries.

The storage APIs are `reserve_owned_operation`, `capture_original_outcome` and
`settle_original_outcome`. The captured payload is a strict whitelist of descriptor identity, original
completion/evidence and a frozen completion timestamp. Separate immutable
reservation metadata binds the owner, pool and optional exact history ticket;
stable audit tuples are derived from that binding and frozen timestamps. It must exclude SQL, credentials, bound values, result rows, arbitrary
error strings, `HistoryEntry` and complete SDK result objects. Typed audits need
stable identities/timestamps and `record_audit_once`; the legacy `audit()` creates
a new identity on each retry. Preserve original acknowledgement, rejection and
unknown classifications without claiming verified database effects.

Keep initial capture separate from projection so an UPDATE, audit INSERT or
binding DELETE fault does not roll back the captured original. If the initial
capture transaction also fails, retain the original in bounded memory, retain
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
Volatile rollback cannot depend on creating a durable owner row when storage
reservation failed. Retain a shared OS admission-gate handle from before dispatch
through capture/settlement, or provide an equally discoverable lease checked by
both restore and migration. SQLite unavailability does not permit bypassing an
exclusive restore/migration gate.

The next schema migration must exclude older live publishers, including Studios
with a boundary operation but no history binding. Prove this before changing
storage authority; physical history/profile/audit fencing alone cannot capture
an acknowledgement still held by an older process. Take the stable gate EX before
the migration SQLite writer lock, prove all existing Studio nonce leases rather
than only mapped history owners, and retain the proof handles through migration.
Refuse held or invalid required proof. A newly created nonce cannot finish startup
under EX. Recheck the exact current schema version after acquiring SH and before
returning a Studio; preserve a genuine version bump and physical writer fencing.
The existing v6 startup already rejects a newer schema through its post-SH
`schema_version` validation, including an opener paused before owner claim.
Already initialized idle or boundary-only v6 Studios require live-lease exclusion.
The implementation holds these proofs; actual preserved-v6 process checks prove
idle-owner exclusion and old-binary rejection after upgrade. The exact delayed
pre-owner-claim interleaving remains unverified, as detailed below.

## Required resource decisions

The schema-6 read-only field inventory found: history completion readers cap encoding at
65,536 bytes, without the matching write-size guard; audit strings have no
general size validation; typed retry maps retain full results containing unbounded
errors/session state. New outbox validation must use its own strict whitelist
without silently tightening legacy history decoding.

Implemented admission/encoding limits; complete capacity and lifetime acceptance
remain open:

| Resource | Implemented limit / behavior |
| --- | --- |
| Canonical captured payload | 32 KiB encoded UTF-8 for descriptor, completion or boundary evidence, and frozen timestamp; bounded serialization, no truncation. Reservation, binding and audit metadata are additional |
| Ordinary reservations | 32 per Studio, 128 per metadata root; include in-flight and captured/unsettled records |
| Separate emergency rollback pool | 32 per Studio, 128 per root; ordinary work cannot use it. Recording failure/full capacity still permits recovery rollback in a preallocated volatile session slot with an explicit warning |
| Root reserved captured-payload allowance | 8 MiB, 256 total slots × 32 KiB; reserve worst-case payload bytes before ordinary dispatch. This excludes reservation/binding/audit/history metadata and SQLite/WAL/index overhead |
| Retained payload memory | 64 preallocated 32 KiB buffers / 2 MiB per Studio; descriptor, lease and other metadata are additional. Bounded serialization borrows fields rather than cloning payloads. Replay scratch is at most eight 32 KiB payloads, plus decoded metadata; overall RSS is not established |
| Replay transaction | At most eight records / 256 KiB of encoded source payload; decoded metadata and canonicalization scratch are additional. Keyset batches, yield between them, stop the pass on write failure |
| Owner files | 4,096/root admission ceiling, counted safely before creation; refuse new ownership at the ceiling. Safe dead/unreferenced collection is still required for practical lifetime behavior |

The 64-slot memory pool includes every preallocated volatile rollback slot and
all retained originals, including those from disconnected sessions. Temporary
canonicalization copies are additional to this pool, as quantified below;
overall memory accounting and capacity measurements remain incomplete. Opening a
new session/task requires its safety slot to be available; refuse churn that
would allocate around a full pool. Never evict an unsettled original to provide
a new session's fallback slot.

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

## Acceptance coverage and remaining checks

The ledger records 21 real SQLite outcome cases, ten actual PostgreSQL script
cases, both owned-child crash cases, two genuine v6/v7 migration cases and seven
actual browser workflows. These cover the principal capture/settlement and
privacy paths below. The following inventory also includes checks not yet
demonstrated; it must not be read as a list of passing tests. Full-capacity/RSS,
all exact crash interleavings and normal packaged/account acceptance remain open.

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

Exercise exact retries after settlement but before memory cleanup, conflicting
same-ID retries, corrupt/deleted expected captured payloads and legitimate
Reserved/no-capture death. Prove audit-once without redispatch. Test concurrent
quota admission, disconnect/session churn and fallback copy accounting. Hold a
volatile rollback ACK after full/read-only reservation failure, prove a second
process cannot restore, then capture/settle and permit restore. Pause separate
v6 children both after Persistence open but before owner SH and after full Studio
startup with no mapped history; migration must respectively reject the resumed
old startup and refuse the still-live initialized owner before publication.

The read-only design review used `gpt-6.1-sol` at high effort, with Tier 2 graph
evidence and direct source reads. Relevant paths had no recorded coverage gaps;
that is a best-effort signal, not completeness proof. No implementation or
external qualification was performed by that review.
The follow-up review also checked transaction boundary admission, schema-version
validation, fallback ownership, memory accounting and settled deduplication.
Its three material design clarifications are implemented with passing
regressions. The subsequent read-only implementation review found no remaining
material issue in storage, runtime, restore and changed browser paths; recorded
coverage is best-effort, not proof of exhaustive correctness.

## Implementation limits still requiring acceptance

The implementation has a 65,536 settled-marker ceiling and a 4,096 owner-file
ceiling per root. Safe collection, ticket/privacy ancestry compaction and audit
retention remain required; these admission ceilings do not establish useful
lifetime behavior or DataGrip parity.

Imported original-outcome records do not confer local ownership authority. An
imported settled boundary retains its historical audit acknowledgement, but its
full action/session/retired witness is not imported into the current-only
boundary lookup. Complete historical boundary presentation remains incomplete.

If original encoding fails or panics, the cell retains its ownership/admission
hold and does not fabricate a completion. A result that never reached durable
capture cannot be recovered as an original after process death. This is a
separate failure from a captured original awaiting projection.

The preserved real schema-6 harness can test migration against an already idle
live owner and reject opening schema-7 data with the old binary. The delayed
pre-owner-claim migration race remains unverified: that frozen harness has no
pause hook for the exact interleaving. Do not infer it from a hand-built PRAGMA.

No account, normal packaged UI/SSO/restart, installation or hands-on acceptance
is established by these checks. Resource/signature and guarded helper-startup
checks establish only their named package scope. Existing schema-6 evidence must
not be reported as proof of the schema-7 implementation.

Resource review checked JSON escaping against legal identity bounds: targets
forbid control characters and have three components of at most 255 characters;
connection IDs have at most 256 UTF-8 bytes. With fixed UUIDs, digests, enums and
numeric fields, legal typed payloads remain conservatively below 10 KiB even
with escaping. This supports the 32 KiB encoder allowance; it is not a measured
RSS ceiling. Reservation metadata is additional, conservatively below 6 KiB per
record. Multiple descriptor copies, decoded replay fields and encoder scratch
must be included in future process-memory and full-capacity measurements.

Capture validation canonicalizes into a separate 32 KiB buffer before acquiring
the database connection lock. Up to 64 producer cells can validate concurrently,
so this adds up to 2 MiB of transient canonicalization buffers per Studio, plus
decoded metadata. The runtime encoding mutex alone does not serialize this stage.
The replay source-byte batch ceiling is not a global scratch or RSS ceiling.
SDK result/error/request lifetimes, allocator overhead and SQLite caches are
outside the retained-original buffer allowance; full-capacity memory acceptance
remains open.
