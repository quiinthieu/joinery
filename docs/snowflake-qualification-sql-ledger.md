# Snowflake qualification SQL contract and offline ledger

This local work shares production SQL definitions and expands the proposed
[existing-schema lifecycle](snowflake-existing-schema-qualification.md) into an
exact ordered ledger, now enforced by a harness-only owning policy. It does not
connect to Snowflake, enable an account runner or authorize account activity.
The complete replacement Goal and all unqualified DataGrip workflows remain
required.

## Consumed runner guard (12 November 2026)

`Runner::consumed` takes the consumed envelope by value and compares its exact
canonical Plan/digest and seven-field profile with the owning policy. The same
guard is attached to transport and metadata. Full revalidation runs before
authentication and each physical admission, outside process/policy locks. Final
publication callbacks perform only retained-failure/time checks; they cannot
scan files or reenter filesystem callbacks while publication locks are held.

The authentication wait is at most 120 seconds; a complete guard scan/admission
wait is at most 30 seconds. Existing stricter process limits, including the
three-second writer admission wait, still apply. Timeout, cancellation and even
an unpolled dropped check revoke the attempt. One blocking scanner owns the
permit until the actual OS work returns; its late completion cannot admit SQL.
Settlement is reported separately and may be false.

Cleanup records exactly one `executionEnvelopeFinal` event containing
`revalidation: {ok, error?}` and `scannerSettled`, before final evidence flush.
Storage success is separate from admission failure. Failed or interrupted writes
reserve the original event, prevent replacement and preserve the first error.
Original helper/local reports and operation completions, including known ACKs,
are forwarded and recorded independently after guard failure. This prevents a
dropped caller from losing the final integrity observation while retaining only
successful phase records. No phase advances or SQL retries occur during cleanup.

Source review is clean; the final optimized module passes 200 tests with one
default ignore. The corrected terminal/control race reserves only exact local
inactive credit/discard completion, consumes its candidate once and blocks phase
advancement until settlement; it cannot authorize a helper write. See the
[verification ledger](replacement-verification.md). A fixed account executable,
exclusive private-file handling and construction exclusively from bound roots
remain required. These components supply no account approval or verified live
effects; full optimized scan latency must fit the stricter process deadline.

## Offline execution envelope (12 November 2026)

The private inventory/envelope/consumption modules add byte identity and local
one-use primitives for `existing-schema-table-lifecycle-v1`. The envelope includes
the original canonical Plan, at-most-30-minute half-open window, fixed source
scopes, complete runtime tree, executable/compiler and private seven-field
profile. Restore extracts only the closed Plan/window, recaptures the supplied
physical roots and compares exact canonical bytes/digest. Snapshot-supplied hashes,
paths, limits and SQL cannot replace that recomputation. Output serialization
stops at 16 MiB; this is not a total-memory guarantee.

Inventories include root, every directory/empty directory and each regular file's
full-content SHA256, length, mode and lossless sorted relative path. Fixed bounds
are 32,768 entries, 64 descendant levels, 512 MiB of file contents and 256 MiB per
file. Absolute physical inputs are bounded to 64 KiB/64 components. Root/included
nodes require effective-user ownership and no group/other write or special bits;
regular files require a single link. Anchored no-follow opens and complete-name/
metadata rechecks reject observed replacements. Ancestors allow root/effective-
user ownership, with a root-owned sticky-directory exception for system temporary
paths. This does not provide exclusive ancestor names or prevent same-user races
between checks. The private profile additionally requires exact mode 0600 and
a bounded 16 KiB read through the same descriptor walk.

Consumption checks a distinct closed approval schema and the actually running
executable, then verifies window/content before exclusively creating and syncing
the RUN marker first and execution-hash marker second. RUN keys bind workflow,
account, host, database, schema and original RUN, excluding source/window hashes.
Partial failure, changed approval window and losing the non-cloneable capability
cannot refund or resume the RUN. Retained directory/marker descriptors support
later identity/content checks; envelope failure stays sticky. The envelope binds
the current transport/default read limits and the proposed 120-second auth/
30-second admission waits, plus its checked 15-minute monotonic deadline.

No account/runner constructor yet accepts the capability. Fixed account entry,
exclusive private manifest/output handling, physical check placement, wait
enforcement and composed final lifecycle evidence remain required before any
executable approval plan. Local fsync tests do not simulate power loss; content
hashes are not authenticated build provenance. See the
[verification ledger](replacement-verification.md) for actual tests and results.

## Retained owner and payload bound (12 November 2026)

The private shared runner now owns all 32 Studio phases, original completion
recording, phase barriers and local cleanup; existing synthetic route/fault
fixtures delegate to it. Construction remains test-only and checks exact Plan/
Policy identity before creating an attachment from the same owner. A future
account constructor must require a sealed consumed-attempt capability.

Bounded capture failures use typed `incompleteRead` journal completions retaining
the capture error, original operation error, actual event prefix and cleanup error
separately. Failed first drains remain incomplete even after late actual
`Finished`; late suffixes do not erase the original failure. The same completion
helper serves execution and durable regression tests. Normal complete failures
retain their preceding `failed` record shape. A dropped waiter detaches the owner
rather than cancelling recording or cleanup. This does not prove crash durability.

The qualification transport enforces 32 MiB over all inbound frame bodies,
including metadata/control/malformed frames, excluding four-byte headers. Declared
length reserves budget before allocation/read; successful reads report actual
body bytes, including partial frames. Interrupted framing cannot resume or refund
that reservation. This conservative upper bound on result content is distinct
from semantic-only accounting and the existing independent 64 MiB wire, 1 MiB
frame, 32,768 frame-count and 64 KiB stderr limits. Ordinary million-row queries
are unchanged. The future executable manifest must bind these actual semantics.

Final validation and failed initial runs are in the
[verification ledger](replacement-verification.md). Source/binary/full-runtime/
private-profile binding, one-use consumption and the fixed account entry remain
incomplete; no Snowflake account effects or packaged daily workflows are qualified
by this local increment. Earlier sections retain historical scope.

## Owning policy and durable barriers (11 November 2026)

`Policy` composes the physical sequencer, lifecycle observations, phase evidence
validator and private journal. The frozen seven-field connect payload is supplied
only by its internal owner; no profile lookup or account/desktop entry exists.
Connect and SQL must use fresh wire IDs/tickets and exact final wire bytes. Credit
and discard target the current query with the production fixed batch grammar;
cancellation targets the production-supported operation set. Close has its own
explicit permission and empty payload. These controls cannot advance SQL phases.

The first validated CREATE-preflight census freezes the expansion. Its original
observation and expanded ledger must sync before the next physical admission.
Callback debt binds each metadata/runtime publication to its exact completed SQL,
site, schema and phase. Context must match the bootstrap user/server session and
frozen role/warehouse/database/schema before lifecycle pinning. Console identity
is separately captured from the actual startup result. Published runtime samples
must use distinct authority revisions and preserve captured transaction A/B.
There are 28 runtime publications on a complete route; raw samples without a
publication do not create an invented observation.

Phase completion accepts original typed operation evidence rather than a caller
success flag. It validates successful DDL/DML/boundary outcomes, catalog and
inspection shapes, reconciliation options plus its actual same-call catalog, and
the complete fixed-read event/result/row sequence. Read counts come from original
events and must agree with complete retained rows. Ownership readbacks agree with
raw creation/transient markers. Exhausting requests or helper terminals alone
cannot advance a phase. The harness-only Studio bridge retains the catalog
already produced by reconciliation; another catalog query would violate this
ledger. The complete synthetic Studio runner consumes that original catalog.

The journal requires an existing owner-only `0700` directory and exclusively
creates a `0600` file through an anchored descriptor. It checks file/directory
identity, permissions, link count and length; recursively canonicalizes lossless
JSON; and returns hash-chain receipts only after fsync. Bounds are 4 MiB per
record, 128 MiB total and 65,536 records including the header, with bounded JSON
depth/node count. Partial writes, sync failures and invalid records poison it;
there is no reopen, resume, truncate or retry. Hashes provide local consistency,
not authenticated or immutable evidence, and tests do not simulate power loss.

Physical admission syncs its intent and earlier original reports/observations
before returning. Phase advancement waits for its own evidence receipts. All
blocking I/O occurs outside process/policy-state locks. If an async owner is
dropped while a blocking batch finishes, that batch consumes its own successful
receipts; final persistence waits for settlement and cannot duplicate records.
Failure stays latched, including external transport failure delivered to the
concrete owner. Final persistence may drain original records after failure but
cannot clear failure, advance a phase or replay SQL.

Original local completions use a separate callback for queued cancellation,
queued close, inactive controls and retirement. Removal and ticket-bound retained
evidence precede observation outside process locks, unchanged publication and
cache removal. Settlement counts in-flight publications. The record distinguishes
reserved candidate IDs from actually assigned wire IDs; successful admission
consumes its candidate even if later revalidation prevents sending. The sequencer
accepts only exact benign inactive credit/discard completions for a successfully
completed query in the same phase. They clear their reservation without advancing
SQL. Every other local completion retains its original report/ACKs and fails the
attempt. A late original is still recorded after an earlier policy failure.

Full offline owner composition and actual adapter/helper checks are distinguished
in the [verification ledger](replacement-verification.md). The former supplies
synthetic shaped callbacks/DTOs. The newer complete 32-phase Studio runner uses a
stateful synthetic SDK, actual requests and parsed original operation results at
S=1/32, with exact 393/610 SQL vectors. Pre-repair full application validation
passes; the first expanded qualification run passes 119/122. Focused reruns pass success and
cancellation. The post-ACK storage-failure case passes after a reproduced
finalization race is repaired, and all 122 qualification and 131 process/adapter
tests pass. Both earlier full application runs fail the recorded PostgreSQL
startup timeouts. A subsequent timing reproduction and repair pass the normal
parallel protocol target and 45 real PostgreSQL/SSH/TLS fixtures. Two additional
tests pass all 14 actual helper-child-exit scenarios across the seven mutations,
before and after ACK, preserving original outcomes without replay. The complete
combined-source qualification module passes all 124 tests. Current full validation
is recorded in the verification ledger.
Earlier startup failures remain recorded in the ledger. No
account SDK or account qualification is involved.

## Shared production definitions

[sql-contract.json](../snowflake-helper/src/sql-contract.json) is the source of
truth for bootstrap identity, version and typed BEGIN/COMMIT/ROLLBACK SQL. The
helper loads a validated frozen contract; Rust's build script validates the same
file and generates constants. The domain transaction SQL, adapter version read,
and prior helper-only qualification definitions consume those values. The two
new contract resources participate in preparation, bundle checks, runtime source
hashes and actual-helper fixture copies. Invalid or stale resources fail closed;
there is no caller-selected contract path.

The adapter's immutable `ReadSpec` factories own SQL, empty binds, semantic kind
and retention ceilings. Production calls and the offline compiler share context
and session-sample order, schema/relation queries, exact lookups and three-read
column inspection. Parsers and the existing identity checks still run between
dependent reads. Named specs do not make observations atomic against other actors.

Retention ceilings are distinct from expected result rows. Exact lookups retain
up to three rows; schema reads retain the 1,001-row sentinel; relation batches
retain up to 10,001 rows/2 MiB; inspection retains up to 1,001 rows/1 MiB and
zero-row resolution retains zero rows. Version and normal readback requests keep
the application's existing 1M-row/8 GiB request ceilings. The fixed synthetic
readback SQL separately limits its result to two rows. Resource admission limits
must still be bound and enforced by the future runner.

## Closed expansion

`Plan::expand_physical_ledger` accepts only a validated ordered `SchemaCensus`.
The existing sole plan factory generates the new target under
`SANDBOX.BILLIE_TESTING`; callers cannot supply SQL, another target or a serialized
plan. The expansion includes all 32 phases and **386 + 7S** SQL invocations:
393 for one schema through 610 for the maximum 32. Every phase's expanded count
must equal its declared budget before expansion succeeds.

Each entry records one-based global/phase ordinals, operation kind, exact SQL,
empty binds, query-frame retention bounds where applicable and its rejection
suffix allowance. Canonical bytes/hash bind the parent plan hash, exact census
and every entry. This is local tamper detection, not execution authentication.
The ledger covers application SQL; SDK-internal authentication activity is a
separate disclosure and approval concern.

Definitive database rejection has route-specific continuation:

| Rejected SQL | Maximum permitted read-only suffix within that active call |
| --- | --- |
| Normal Studio readback SELECT | Two five-read session samples, ten reads |
| Reviewed DDL statement | One five-read session sample |
| Reviewed DML or typed transaction boundary | Zero |
| Any metadata/observer read | Zero after that read |

Each sample orders extended context, quoted-identifier policy, AUTOCOMMIT,
TRANSACTION_ABORT_ON_ERROR and current transaction. Continuations can stop at
any prefix; a failed sample read never permits the rest of that sample or the
second sample. Cancellation, connection closure or another rejection can stop
earlier. No allowance crosses a phase, authorizes replay, adds rollback, or
admits cleanup SQL. A definitive rejected statement is distinct from unknown
delivery, which must retire without replay.

## Offline wire and lifecycle validators (11 November 2026)

The harness now includes pure validators for final writer envelopes and lifecycle
observations. They do not attach an admission policy or enable execution.

Wire matching requires exact envelope fields and canonical bytes, a resolved
nonzero decimal wire ID, exact SQL/binds/retention/classification, matching typed
operation/impact and expected mutation user/session. The logical handle and
zero-based ticket are distinct from the wire ID; ticket zero is valid. Connection
matching accepts only the owner's separately frozen seven-field payload. It does
not itself establish that a profile is trusted, one-use or approved.

Lifecycle validation freezes the first CREATE-preflight census and complete live
context. Later observations require the same ordered census, principal, role,
warehouse, session, search path, secondary roles and identifier policy. The new
target must be absent before CREATE, present with ordinary base-table traits and
creation/DDL markers after CREATE, and absent after DROP. Creation markers and
traits remain fixed; only the first post-ADD observation may repin LAST_DDL, even
when its timestamp is unchanged. These are best-effort drift checks, not immutable
object identities or protection from concurrent external DDL.

A harness-only transport callback observes each validated helper terminal after
pending-state removal and outside the process mutex, before caller publication.
Failure revokes future admission while preserving the original report and typed
ACK. It still runs after an earlier policy failure. Malformed terminals cannot
reach it. A ticket-bound in-memory terminal cache keeps original ACKs accessible
to concurrent callers during the callback, including retirement and logical-ID
reuse. Publication and cache removal occur together under the process mutex.
Local cancellation and retirement-synthesized reports use the separate local
completion hook described above. Neither hook is a row-drain barrier or phase
completion, and full production-route outcome qualification remains required.

## Remaining integration gates

The policy is attached in local fixtures, including complete synthetic 32-phase
Studio/adapter/helper composition. Original operation completions are durably
recorded before validation, separately from validated phase evidence. Actual
helper child exit passes before/after ACK across all seven mutations. The newer
four full-route drift tests pass eight scenarios: census overflow/order, same-count
schema replacement, role/warehouse, secondary roles and search path. Search-path
coverage adds an entry and is not an order-only permutation. Seven exact-plan
restoration tests also pass; canonical private recompilation preserves the
original target/SQL/expanded ledger/journal header but conveys no authority.
The complete combined-source module passes all 135 tests, including both
census-extreme success routes and preceding fault loops; full application
validation also passes. Checks are recorded in the verification ledger. Account
interruption remains unqualified. Source/runtime/binary/private-
profile binding, one-use approval consumption and an account entry are still
unimplemented. Preserve early ACKs, actual same-call results and collector evidence
through that integration; local filesystem tests do not prove power-loss durability.
Enforce the proposed 32 MiB aggregate semantic-result cap separately from transport
and per-read limits. Strict runtime inventory must reject links; the default staged
runtime has five dependency symlinks, while the newly verified normal macOS bundle
has zero. The execution envelope must bind and fully inventory its zero-link tree.

Complete production-route success/rejection/interruption/recording-failure tests
must pass before preparing a fresh executable account plan. The old database-
creation approval remains consumed. Only a fresh generated table under
`SANDBOX.BILLIE_TESTING` is the permitted target location; execution and possible
billing still require a new exact plan and approval.

Commands and results are recorded in the
[verification ledger](replacement-verification.md). Offline expansion and
synthetic helper dispatch do not establish account write/transaction effects,
normal packaged SSO, million-row account performance or replacement readiness.
