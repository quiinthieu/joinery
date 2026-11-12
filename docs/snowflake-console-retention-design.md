# Reviewed console receipt retention design

## Decision and status

Partially implemented locally, 12 November 2026. This document specifies the
complete target; the inventory/compaction slice below does not qualify unlimited
lifetime retention, full-capacity recovery, account effects or daily replacement. It follows
the [roadmap](implementation-roadmap.md), the
[console workflow](snowflake-sdk-console-workflow.md) and the
[durable admission decision contract](snowflake-console-negative-receipts-design.md).
The local source inventory is
`.cache/verification/reviewed-console/retention-inventory.json`, SHA-256
`49e710ef615d818f1eb93793c1e562dfab38aa06eae5f422217646f32282b5d5`.

Keep complete original evidence in one authoritative SQLite database. Archive is
a browsing preference; settled-memory compaction is independent. Neither deletes
evidence nor frees journal disk space. Add bounded, paged SQL-free receipt
inventory and detail, independent result-row release, and bounded-memory
encrypted full-metadata backup/restore. Ship these together before removing the
current lifetime decision-count gates. A browser-only archive or larger constants
would leave native memory, startup and recovery limits unresolved.

Full immutable evidence grows with use. Finite local storage cannot promise
unlimited lifetime retention. At an explicit storage boundary, preserve all old
decisions and stop new admissions. Never delete an identity or replace full
original outcomes with hashes to make another query fit.

## Implemented inventory and compaction slice

Schema 4 now physically fences older journal writers and provides a native,
SQL-free paged inventory, exact identity lookup and compare-and-set archive
preferences. Original positive decisions, refusals and their revisions remain
intact. Migration and archive use a raw exclusive lease; archiving one receipt
cannot recover unrelated abandoned runs. Restore retains the exact union of
original evidence and gives the merged inventory a fresh ordering epoch.

Settled native full-detail caching is bounded to 16 entries / 16 MiB; working
owners have separate active/publisher/dirty reservations and result ownership.
Frontend detail payloads have the same cache bounds independently of browser
storage success. Pending, unrecorded or conflicting evidence stays held. Opaque
browser IDs are removed only after a fresh exact native inventory confirmation;
malformed/quota-failed browser storage stays preserved and blocks new admission.
Background reconciliation has four shared slots and retry backoff. Manual native
inventory uses 25-row pages and one expanded detail. Explicit result release is
separate from archive and preserves original outcomes.

**The 10,000-positive and 10,000-refusal lifetime gates remain.** Native accounting
reports logical charged bytes, while reservations, admission byte budget and
available journal budget remain unavailable. `retentionEnabled` is false. The
following [encrypted recovery increment](encrypted-metadata-recovery.md) now has
full validation, actual browser/hidden-native acceptance and normal SDK-bearing
packaging from clean `f9c1ce2`. Its 20-copy / 64 GiB aggregate / 16 GiB plaintext
limits do not establish full-capacity performance. The earlier inventory-only
slice used plaintext 64 MiB snapshots. No memory plateau, full-capacity recovery
or actual-account qualification is claimed.
See the [verification ledger](replacement-verification.md) and
[usage walkthrough](snowflake-sdk-console-workflow.md) for source-scoped checks.
The remaining sections describe the full design, including unimplemented gates.

## Pre-increment behavior and constraints

The source baseline is `b86ac44a2124d2690cf5dfeaf87dea462b62c0a7`, inspected with
the concurrent query-limit increment present as uncommitted changes. Earlier
verification results do not verify this design.

| Layer           | Current boundary or cost                                                                                | Consequence                                                                                                   |
| --------------- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Journal         | 100 units/run; separately 10,000 positives and 10,000 durable refusals; 16 KiB per JSON field           | Count exhaustion does not erase old evidence. Field limits are not a total database budget.                   |
| Startup         | Enumerates and validates all decisions and bounded step records before admission                        | Work grows with lifetime evidence; current enumerators also build ID vectors.                                 |
| Native owner    | Retains every positive full snapshot/result-ID list; flush and some lookups scan retained runs          | Settled runs continue consuming memory and scan time.                                                         |
| Browser         | Persists up to 10,000 combined opaque request IDs; retains full views; periodically reconciles held IDs | Native capacity and browser capacity differ. Lost/corrupt browser storage has no paged native discovery path. |
| Results         | Separate 100-descriptor, 64 MiB RAM, 1,000,000-row/result and optional 8 GiB temporary-disk limits      | Receipt archive alone cannot release result capacity or preserve rows across restart.                         |
| Metadata backup | At most 20 snapshots, each at most 64 MiB; whole-file digest buffer and in-memory restore copy          | A valid lifetime journal can outgrow recovery before reaching a decision-count limit.                         |

At this baseline, metadata snapshots are **plaintext SQLite**, protected by filesystem
permissions. The current OS-stored write-passphrase secret is an Argon2 verifier,
not a backup encryption key. Existing backup creation has no encryption-password
workflow. The new encryption design below is a separate capability.

## Permanent evidence and schema 4

Preserve existing run metadata, client/session/connection identity, plan digest,
unit count, step intents, registered original ACK/rejection/non-dispatch/unknown,
passive nullable context, monotonic observations, revisions, terminal reason,
accounting and timestamps. Preserve a durable admission refusal's exact
client/session/digest/code/recorded-at tuple. No SQL, bind values, result rows,
credentials or raw error messages enter new receipt DTOs or browsing indexes.

For each canonical client request UUID there is at most one decision: an original
positive run or a durable admission refusal. Neither archive, restart, restore,
profile deletion nor row release can change that decision or make the UUID fresh.
Changed session/digest authority remains a conflict. A missing lookup remains
unknown; absence is not proof of non-execution. Archive of an uncertain terminal
does not make it safe to replay, acknowledge, commit or roll back.

Migrate under the existing **raw exclusive journal lease**, without invoking
abandoned-run recovery merely to migrate. An active other owner rejects migration
with no schema or evidence changes. In one SQLite transaction:

1. Validate the exact v3 schema, guards and every existing decision in bounded
   memory. Rename physical tables to `snowflake_console_runs_v4`,
   `snowflake_console_steps_v4` and `snowflake_console_rejections_v4`. Retarget and
   validate foreign keys and recreate exact cross-decision guards.
2. Add browsing-only fields on the two decision tables: `receipt_sequence`
   (positive signed 64-bit integer), `archived_at` (nullable nonnegative integer),
   and a distinct browsing revision. These are excluded from the existing
   original-evidence DTO/revision. Initial archive state is visible/recent.
3. Add one validated retention-control row for the next sequence, inventory epoch,
   aggregate charged bytes and outstanding recording reservations. This is
   derived/control data, not an alternative decision ledger. Migration creates a
   fresh canonical UUID inventory epoch. Assign existing sequences deterministically
   by decision time and canonical client UUID, preserving all original timestamps.
   A sequence is immutable and never reused **within that epoch**, not globally
   across independent database branches or later restore epochs. Gaps are allowed.
4. Index both decision tables for `(archived_at IS NULL, receipt_sequence)` and
   exact client UUID; retain exact positive run-ID uniqueness. Guard sequence
   uniqueness across both decision tables within the current inventory epoch, as
   well as permanent client-decision uniqueness. Validate one well-formed current
   epoch, positive unique sequences and a next sequence greater than every
   allocated sequence. Browsing responses bind row order to that epoch.
   Update backup schema/index/trigger allowlists and validation before setting
   `user_version=4` and committing.

Do not leave writable old aliases or compatibility triggers. An already-open v3
connection must fail its old table-name INSERT, outcome UPDATE and recovery SQL
after upgrade, including a statement prepared before migration. Existing v2
names remain absent. Version checks alone cannot fence already-open older code.
All v4 journal mutations recheck exact schema version under their transaction;
all reads reject incompatible or dual decisions. Older apps must fail safely on
opening v4 and cannot restore a v4 backup through their old allowlist.

Backing up or restoring settings must include the complete receipt ledger, both
visible and archived. A receipt browsing index is never sufficient recovery proof.

## Admission capacity and bounded hot state

Keep the 100-unit limit, closed enums, identifier bounds and 16 KiB field bounds.
Replace lifetime row-count caps only after the following budgets and recording
reservation path exist. Proposed initial policy, subject to the measurements
below, is an 8 GiB charged-journal admission budget, at least 1 GiB free-space
headroom, and room for the selected backup/restore operation. These are visible
local policy limits, not promises that the filesystem cannot become full.

Charge canonical persisted bytes plus conservative per-record/index allowance.
Derive a checked worst-case reservation from **all 100 units**, maximum outcome
and observation fields, run/terminal metadata, monotonic delivery observations
and index/control overhead. Reserve before the durable positive preparation and
before any SQL; do not reserve merely the next intent. A durable refusal needs
its own smaller reservation. Maintain aggregates/reservations transactionally;
startup and restore recompute and verify them rather than trusting cached totals.
Return unused reservation only after terminal recording and all late publisher,
audit/history and journal work have settled. Recording already-admitted work
uses its reserved allowance even when new admission is closed.

Logical reservation and a free-space check cannot guarantee physical writes:
other processes, WAL/copy amplification and I/O faults can still exhaust disk.
Do not call sparse file allocation a guarantee. A failed intent write prevents
dispatch; failure after dispatch retains the original report and frozen outbox,
stops the suffix and holds subsequent admission/shutdown until reconciliation.
Commit/readback failure remains unconfirmed until exact durable readback.

If capacity prevents a new durable refusal too, do not advertise `notAdmitted`
as durable. Preserve any bounded local pending receipt and report a recording
problem/indeterminate response under the existing contract. A full outbox cannot
be made safe by dropping an original ACK. Refuse further new client identities
before accepting additional unrecordable work. Capacity lookup itself is SQL-free
and never creates a refusal or attempts remote SQL.

Use separate bounded registries for active run routing/cancellation, result
ownership and pending writes. Reserve their worst-case working memory before a
new run; stop admission if the reservation cannot be made. Flush only dirty
entries. After task completion, exact durable journal/audit/history flush and all
event-publication observations, remove the full settled `Retained` object.
Terminal receipt alone is insufficient: `Finished` delivery can still append a
delivery-failure observation. Pending outboxes are never evictable cache entries.

Use a byte-bounded full-detail LRU, initially at most 16 entries and 16 MiB, and
bounded summary pages. Cache misses load one validated receipt from SQLite.
Compaction does not release the process journal lease, drop a retained physical
console, restore generated authority, or change shutdown's drain/flush barrier.
Cancel and result ownership must no longer depend on the full settled-view map.

## SQL-free native API and archive rules

Keep exact client lookup as the authoritative detail/recovery path. Add these
closed, read-only DTOs and local mutations; none may reconnect, authenticate,
probe metadata, submit helper SQL, or resume a suffix:

| API                                                                      | Contract                                                                                                                                                                            |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `snowflake_console_receipts({filter,cursor,limit})`                      | `limit` 1–100; keyset page of SQL-free summaries and a next cursor. Filters are closed: visible/archived/all, positive/refusal/all, and optional exact known connection ID.         |
| `snowflake_console_pending()`                                            | Bounded process-local/durable pending summary and global recording hold; does not load all terminal details. A failed read is unavailable, not an empty list.                       |
| Existing `snowflake_console_request({clientRequestId})`                  | Exact original positive/refusal or unresolved absence; full bounded positive detail on demand. Reconciliation may flush/seal locally only under the established lease rules.        |
| `set_snowflake_console_receipt_archive({inventoryEpoch,items,archived})` | At most 100 exact client/session/digest decisions with current inventory epoch and expected original and browsing revision/fingerprint. Atomic local archive/unarchive or conflict. |
| `snowflake_console_storage()`                                            | Charged bytes, outstanding reservations, configured admission budget, observed free bytes, and closed availability/hold reason. No expensive per-page recount.                      |

Summary fields are decision kind, opaque client/run/session IDs, known connection
ID or null, plan digest, original decision time, inventory epoch and sequence
(immutable within that epoch), current
original/browsing revision, closed terminal/refusal classification and recording
availability. Do not include SQL or human profile names in persisted identity.
Legacy refusals lack connection identity: show “Connection not recorded” and keep
them discoverable in All; do not attach a present profile by name or session guess.

Allocate sequence on first durable decision in the same transaction under the
current inventory epoch. Cursor binds filter, inventory epoch, high-water sequence
and last sequence; use strict bounded
encoding, 64-bit-safe representations and no OFFSET scan. New decisions above the
high-water mark appear on Refresh. Archive changes/restore invalidate affected
cursors rather than silently skipping rows; restore always creates a fresh epoch
and invalidates every previous cursor. Cursor conveys browsing position,
never execution authority. Exact lookup and pending warnings do not depend on
having traversed inventory pages. Counts are transactionally maintained or
explicitly unavailable, not a full `COUNT(*)` on every keypress.

Archive eligibility requires a confirmed terminal positive or committed refusal,
exact current inventory epoch and identity/revision, no active owner/publisher, and no journal,
history, audit or local refusal outbox. Unknown terminals may be archived with
their uncertainty intact. Reject a batch containing an ineligible/conflicting
item without partial changes. Reject an old-epoch archive/unarchive request even
if client identity, original revision and browsing revision still match. Update only browsing fields, never original
terminal, outcomes or observations. Serialize using configuration gate → journal
state → SQLite, rechecking eligibility under the locks; no await under a
parking-lot guard. Unarchive has no execution effect.

## Complete daily UI workflow

Provide an app-wide **Console receipts** view with Visible, Archived and All
filters, bounded pages, exact request-ID lookup, known-connection filtering and
keyboard-accessible details. Show pending/recording/unknown warnings outside the
paged settled list. An unknown outcome is visibly uncertain in every location.
Load full step/ACK/accounting details only on expansion; keep page/view cache and
DOM size bounded. Preserve focus after archive, page changes and reconciliation;
announce errors and updated counts without moving focus unexpectedly.

An explicit **Archive receipt** action explains: “Keeps original evidence and
recovery identity. Does not free journal disk space or run SQL.” It is unavailable
with an associated visible reason while recording or publication is pending.
Archive/unarchive response loss is recovered by exact local lookup, never by
starting SQL. No automatic deletion or journal vacuum is part of this workflow.

Native inventory becomes the settled history source of truth. Browser storage
keeps only unresolved opaque start correlations persisted **before** the start
RPC. Remove a correlation only after exact identity validation and confirmation
that the durable decision is discoverable in native inventory. Null/error,
uncommitted refusal, unconfirmed readback or opposite decision never permits
removal. Crash between native commit and browser removal is harmless duplication.

Migrate the existing up-to-10,000-ID browser list in bounded batches with bounded
concurrency (initially four lookups), backoff and cancellation on view teardown.
Do not repeat all held lookups every second. Native pending summaries and the
durable inventory discover receipts after browser loss. Malformed/quota-exhausted
browser storage produces an explicit warning and blocks unsafe new correlation,
not a claim that no past requests exist. An ID that never reached durable native
storage remains uncertain; inventory cannot reconstruct or prove its absence.

Keep **Release retained result rows** separate. Confirm which process-local
results will be released, use the existing result-store release path, and retain
the exact journal/accounting. Receipt archive, tab closure and disconnect do not
implicitly release receipt-owned results. Released/restarted rows show
“Results unavailable”, not zero rows. Receipt originals do not establish affected
rows, transaction commit, complete results or a safe automatic replay. Result
release allows later million-row work to reuse existing bounded result capacity;
it does not make result rows permanent evidence.

## Streaming encrypted full-metadata backup and restore

### Format and key workflow

Propose the standard binary age v1 encrypted file format through a reviewed, pinned
Rust `age` dependency, using its streaming writer/reader rather than implementing
cryptography. The proposed dependency candidate is `age` 0.12.1; compatibility,
license, dependency and resource review are implementation gates. These are pinned
candidate references, not claims of latest versions or an implementation/dependency
selection. The [versioned format specification](https://c2sp.org/age@v1.0.0)
defines streaming encryption and links to the
[newer v1.1.0 specification](https://c2sp.org/age@v1.1.0);
the [Rust writer API](https://docs.rs/age/0.12.1/age/struct.Encryptor.html) requires
explicit successful finalization before a complete encrypted file exists.

For this increment, manual encrypted snapshot creation asks for a **separate
backup passphrase**, confirmed twice, 12–1024 Unicode scalar values with a bounded
UTF-8 input. Never derive it from the existing Argon2 verifier, silently reuse the
write passphrase, put it in localStorage/settings/history/logs, or remember it
across operations. Zeroize native secret buffers and clear UI inputs after use.
The user must keep this passphrase separately; losing it makes the copy
unrecoverable. Restoring an encrypted backup uses its passphrase to encrypt the
mandatory undo copy. Restoring a legacy plaintext snapshot additionally requires
a new undo-backup passphrase. Current write-passphrase authorization remains a
distinct check; successful decryption does not authorize restore.

Payload is one bounded, closed Joinery manifest followed by exactly one
self-contained SQLite snapshot, without an archive of arbitrary paths. Include
format/schema version, byte length and SHA-256 of the SQLite payload inside the
encrypted manifest. Bound manifest/header lengths, KDF resource cost, ciphertext
length and decrypted length before untrusted allocation; require authenticated
end-of-stream and reject trailing content, truncation and unsupported recipients.
Do not load the file into a `Vec` or trust an unauthenticated manifest. No
compression is needed in the first format. Backup list exposes only opaque ID,
format, file size, ciphertext digest and filesystem time until decryption;
times and display metadata are not original receipt evidence.

### Bounded I/O and publication

Create one app-owned private staging directory with non-symlink, fd-bound files,
0700 directory/0600 files and validated ownership. No arbitrary path crosses IPC;
native import/export pickers copy to/from pinned handles. Stream SQLite backup in
bounded page steps into an on-disk staging connection, then validate with bounded
iterators. Limit SQLite page cache and temporary work to a documented memory
budget; direct sorts/merge spill to this private disk directory. Hash using a
fixed-size buffer, then stream manifest and SQLite bytes into the encrypted
partial file, finalize encryption, sync file, atomically publish and sync parent.
Do not publish a successful backup after a failed finish, sync or rename.

Private staging and the live SQLite database are still plaintext at rest. This
design encrypts published backup/export files; it is not whole-application disk
encryption and cannot promise secure erasure on APFS. Remove owned temporary
files on success/failure and safely recognize abandoned staging on next startup.
Do not delete unfamiliar files. Requiring encrypted SQLite staging or an encrypted
live database would need a separate encrypted-VFS/key-lifecycle design.

Restore is an explicit tracked local job with progress and cancellation before
replacement. Preserve existing gates: no sessions, connection attempts, jobs,
exports or retained result buffers; correct confirmation and current write
authorization; pending generated/console receipts flushed or restore blocked;
exclusive journal owner lease. Long decryption/validation runs on blocking worker
tasks, not the UI thread. A local restore state blocks conflicting mutations;
recheck configuration generation and ownership at final replacement. Do not hold
a parking-lot guard across asynchronous waits.

Stream decrypt into private on-disk pinned SQLite, authenticate the entire input,
validate its exact version-specific schema and evidence, and upgrade only this
copy. Stream live and backup decisions through indexed bounded iterators into the
pinned database. Preserve the complete monotonic union, including archived
evidence, original refusal timestamps and every live client identity. Same-client
opposite decisions, identity changes, contradictory/newer originals, missing
steps or malformed records reject before live replacement. Live evidence covers
older compatible backup evidence; a backup-only incomplete run is sealed locally
as interrupted/unknown under existing rules, never resumed. Browsing state has
an explicit merge rule: live preference wins for existing identities, otherwise
retain validated backup preference. Original evidence comparisons ignore it.

Restore ordering is local browsing state, not immutable original evidence. Two
valid branches can both allocate sequence 11 to distinct client UUIDs. Validate
and merge their complete original evidence by client/run identity, ignoring
branch-local sequence values for conflict comparison; never insert those values
directly into the constrained merged decision tables. In the private pinned
copy, create a **fresh inventory epoch for every restore** and deterministically
reassign the complete validated union in decision-time/canonical-client order.
Preserve original identities, timestamps, revisions, outcomes and observations
exactly. Rebuild epoch-scoped ordering indexes, sequence uniqueness and the next
sequence counter; validate cross-decision guards and recomputed byte/reservation
totals. Publish the new epoch atomically with replacement. All old cursors and
archive CAS requests are stale even where their numeric revisions still match.
Original-evidence conflicts remain fatal; coincident branch-local sequences are
not evidence conflicts. Create and durably publish a complete encrypted undo snapshot of live
metadata **before** replacement. Retain current OS credentials/write-verifier
configuration. Use SQLite's backup transaction for final replacement so failure
rolls back rather than swapping an untracked path beneath open connections.
Once replacement begins, do not report an early cancelled success/failure; finish
or roll back, then return the actual local result. Invalidate old cursors/plans
and reinitialize UI manually; never reconnect or submit SQL.

### Space, compatibility and failure

Keep an explicit backup-count budget (initially 20), add a visible aggregate byte
budget, and replace the hidden 64 MiB ceiling with a documented per-operation
limit sufficient for the supported journal budget. A proposed 16 GiB plaintext
snapshot ceiling accommodates the initial 8 GiB journal policy, but other
metadata also counts. Preflight actual live/source sizes plus conservative WAL,
pinned-copy, merge, encryption-output and undo headroom; reject before work if the
full operation cannot fit. Restoring a live+backup union may exceed the admission
budget: existing proof must still be preserved, with new execution held over
budget. If the validated union cannot fit the restore operation limit, fail
without replacing live data. Never prune evidence or restore settings alone.

Disk fullness can still occur after preflight. Fail partial backups without
publishing them; fail pre-replacement restore without changing live metadata;
retain the durable undo and report accurately if final local replacement fails.
Deletion of a backup is a separate explicit confirmation; no automatic oldest-copy
deletion to make room. Local backup is not an off-device recovery plan: provide
explicit encrypted-copy export/import with native file selection and no shell or
clipboard secrets.

Read v1/v2/v3 plaintext snapshots only through the existing version-specific
allowlists and a private bounded upgrade. Show “Legacy unencrypted backup”; never
silently rewrite/delete the selected copy. Newly created backups and undo copies
are encrypted. Old applications cannot read the new format; new code must fail
closed on unknown schema/format and explain the upgrade requirement. A copied old
database opened in a different directory can still be written by an old app;
old-writer fencing protects the upgraded database, not every historical copy.
Reimport/restore must detect conflicting branches, never guess an outcome.

## Startup and query cost

Stream full schema/evidence validation at startup, migration and restore with
bounded memory, including archived receipts. Remove all lifetime-sized ID vectors
and full-snapshot aggregation from these paths. Use ordered step streams/bounded
joins to avoid an application-level query per archived run where practical.
Validation remains linear disk work; report progress and allow cancellation
before admission. A cancelled/failed validation leaves console execution
unavailable. Do not silently skip archived records based on a mutable archive flag
or cached “validated” bit. Fast trusted segments would require another integrity
and corruption model and are outside this increment.

Normal exact lookup uses unique indexes. List uses keyset indexes and `limit+1`,
does not load steps, and refreshes only bounded summary/detail caches. Dirty flush
cost depends on pending work rather than lifetime settled count. Record cache,
temporary-sort and worker budgets in tests; fixed application buffers alone do
not bound SQLite or password-KDF memory.

## Implementation slices and acceptance

These slices may be reviewed separately, but capacity removal is enabled only
after all applicable recovery/UI slices pass:

1. Persistence: schema 4 physical fencing, exact validation, sequences/browsing
   fields, maintained budgets/reservations, streaming iterators and paged reads.
   Keep current count admission gates until the complete path is enabled.
2. Owner: explicit active/result registries, dirty outbox iteration, settled
   compaction after publisher completion, bounded detail cache, archive CAS and
   unchanged intent → original ACK → publication ordering.
3. API/UI: native inventory/pending/storage commands, browser ID handoff,
   bounded reconciliation, accessible archive/detail and separate row release.
4. Recovery: reviewed encryption dependency/secret UI, bounded disk-backed
   snapshot/decrypt/merge/undo, export/import and truthful legacy compatibility.
5. Verification: run the complete migration/recovery/response-loss workflow and
   benchmark datasets, then remove lifetime count gates with documented budgets.

Required regression and measurement fixtures use disposable local SQLite and
synthetic helper sessions, never real account/profile state:

- More than 10,000 positives **and** 10,000 refusals, then 100,000 mixed small
  decisions, plus 100-unit/max-field runs. Archive/restart/exact lookup returns
  every original decision; changed authority and positive/negative collisions
  fail. No inventory/archive/release/restore action emits helper SQL.
- Real v3 migration with an active held lease changes nothing. An already-open
  old connection, including cached statements, cannot INSERT, UPDATE outcomes or
  interrupt work through old names after migration. Preserve exact v1–v3 records.
- Race archive with late `Finished` failure, cancel, profile/passphrase revocation,
  pending ACK recording, disconnect and shutdown. No premature compaction,
  released lease, suffix dispatch, lost outbox or changed original outcome.
- Browser crash/response loss before and after native commit/archive/handoff,
  corrupt/quota storage, existing 10,000 IDs and restarted native inventory.
  Unknown correlations remain held, no resend occurs, reconciliation concurrency,
  page cache and DOM are bounded; keyboard navigation/focus remains usable.
- Row release frees real descriptors/RAM/spool space while full ACK/accounting
  stays exact. Archive/disconnect keep ownership; restarted detail reports row
  unavailability. Exercise another million-row result after explicit release.
- Encrypted backups well above 64 MiB: round trip full live+archived positive and
  negative evidence, legacy restore union and mandatory encrypted undo. Inject
  wrong passphrase, excessive KDF/header/manifest size, truncated/reordered/
  modified/trailing ciphertext, symlink/file replacement, low disk, failed commit,
  readback, finalization, sync and replacement. Live stays unchanged unless the
  actual validated replacement commits; no secret or SQL enters receipt output.
- Restore conflicting branches/dual decisions, newer contradictory backup,
  missing step, over-budget union and insufficient undo space. Preserve live
  evidence, selected source bytes and old client dedup after every rejected case.
- Restore two valid v4 branches that each allocated sequence 11 to distinct client
  UUIDs, including positive/refusal combinations. The complete compatible union
  succeeds with a fresh epoch and deterministic unique sequences; every original
  identity, timestamp and outcome remains exact. Old-epoch cursors and archive/
  unarchive CAS fail even when the client identity and numeric revisions match.
- Measure cold/warm startup and validation, exact old-client lookup, first/later
  page latency, archive batches, dirty flush, peak native/UI RSS, DOM count,
  backup/restore throughput/peak RSS and temporary/total disk. Record hardware,
  source hashes, fixture sizes and encryption-KDF cost. Acceptance requires bounded
  memory as lifetime data grows, indexed page plans and no per-run settled-memory
  growth; numeric latency targets must be selected from recorded local results.

Forced-power-loss durability, packaged macOS key/input behavior, actual SSO/account
effects and daily-owner acceptance remain separate gates. Local benchmarks do
not qualify Snowflake transactions or arbitrary million-row usability.

## Remaining implementation decisions and measurements

- Validate the initial 8 GiB admission/16 GiB snapshot defaults and measured memory
  budgets, adjusting them from fixture evidence with the same failure semantics.
- Implement separate manual backup passphrases and private plaintext staging. A
  remembered device key, scheduled backup or fully encrypted live/staging SQLite
  changes key recovery and threat scope and needs its own design.
- Pin/review the encryption dependency and KDF resource limits, define numeric
  startup/page/backup targets from the benchmark fixture, and confirm aggregate
  backup storage policy. Do not claim lifetime readiness before these gates.
- Decide whether automatic logical archive is wanted later. This increment uses
  explicit archive; neither option permits deleting original evidence.

These are lead-owned engineering choices under the existing local development
scope, not additional permission requests. Dependency review and measurement
must precede enabling the complete workflow; owner evaluation still determines
daily suitability.

## Source evidence and review limits

Tier 2 graph project
`Users-billiesilvaner-orca-workspaces-joinery-datagrip`, generation
`2026-11-12T13:34:20Z`. Exact recovery symbol search returned 2/2 without another
page; recovery trace returned 12 callees/7 callers without continuation. Graph
call relationships included heuristic edges, so direct source established actual
ownership and replacement ordering. Coverage for every path below returned
`metadata_match` / `no_recorded_issue`; bounded persistence, secrets and Studio
scopes had no recorded gap or further page. This is best-effort coverage, not
proof of completeness. Ignored local inventory was read directly. No test,
build, process launch, owner-state access or measured performance was performed
for this document.

| Evidence path                                                                                                                                              | Relevant source                                                                                            |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/persistence/snowflake_console.rs`                                                                                                           | Limits/schema/decision guards; prepare/refusal; indexed lookup; owner lease; validation and restore union. |
| `src-tauri/src/persistence.rs`                                                                                                                             | Open-time validation, schema-3 lease/physical rename, separate history policy.                             |
| `src-tauri/src/persistence/recovery.rs`                                                                                                                    | Current 20/64 MiB policy, whole-file digest, snapshot and in-memory restore/undo.                          |
| `src-tauri/src/execution/snowflake_console.rs`                                                                                                             | Retained map/outboxes, intent/ACK publication, local reconciliation and owner completion.                  |
| `src-tauri/src/execution.rs`                                                                                                                               | Restore configuration/session/result gates, passphrase verifier, disconnect and shutdown ownership.        |
| `src-tauri/src/results.rs`                                                                                                                                 | Independent row/memory/disk limits and result ownership.                                                   |
| `src/features/studio/SnowflakeConsole.ts`, `SnowflakeConsoleReceipts.tsx`; `src/state/studio.ts`                                                           | Browser opaque-ID storage, receipt state, full views and reconciliation.                                   |
| `src/features/studio/MetadataRecovery.tsx`; `src-tauri/src/commands/recovery.rs`                                                                           | Current backup/restore UI and command ownership, distinct write authorization.                             |
| `src-tauri/src/secrets.rs`, `src-tauri/Cargo.toml`                                                                                                         | Existing OS-secret abstraction and dependency baseline; no implemented backup key workflow.                |
| `docs/implementation-roadmap.md`, `snowflake-sdk-console-journal.md`, `snowflake-sdk-console-workflow.md`, `snowflake-console-negative-receipts-design.md` | Required retention scope and existing original-outcome/admission contracts.                                |

The frozen inventory above pins the source hashes used for its capacity and
call-chain findings. Current query-limit changes were also read for compatibility;
they do not implement any proposed schema, paging, archive or encryption behavior.
The three persistence hashes still match that inventory. Current compatibility
pins for changed owner/frontend sources are:

```text
b4cd03bc6833c1876a959a4004b9933771645e84f51023294cb17ef7494bd553  src-tauri/src/execution/snowflake_console.rs
a1c67432a15a566ceba20410b4f04170d3a83423cde307a467a0e823443ad127  src-tauri/src/execution.rs
75a85deee1b1aed459f3676d214a14cb3121b03f2b68410c66b122eba9803555  src/features/studio/SnowflakeConsole.ts
```
