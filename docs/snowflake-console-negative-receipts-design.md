# Durable console admission refusals

Status: implemented and locally verified, including schema-3 migration, refusal
recording failures and browser response-loss/restart recovery with a synthetic
SDK. Both admitted runs and committed refusals have SQL-free recovery.
Absence remains uncertain. See the
[workflow](snowflake-sdk-console-workflow.md) and
[roadmap](implementation-roadmap.md).

## Local contract and evidence

The bounded native review on 12 November 2026 inspected admission, journal
validation, SQLite writes, restore merging and closed-owner recovery. It found
that recording only admitted runs cannot prove a missing request was refused.
Lookup absence must continue to mean unknown. A committed refusal must survive
response loss, restart, policy changes and metadata restoration without becoming
executable authority.

Persist an immutable SQL-free refusal alongside admitted runs. Store only the
canonical client UUID, session UUID, lowercase plan digest, closed error code and
recording timestamp. Never persist SQL, parameter values, approval secrets or
arbitrary error text in this record. A caller-supplied digest is correlation data,
not execution authority.

Publish `NotAdmitted` only after successful commit or confirmation of the exact
committed record. An existing identity with the same session and digest returns
its original decision. A changed identity payload returns `Conflict`. Malformed
identities fail without persistence. A fresh manual review uses a fresh client
identity; a refused identity never becomes an admitted run.

## Atomicity and failure handling

Schema version 3 uses separate physical run/step/refusal tables with `_v3`
suffixes and exact validated cross-decision triggers. Migration holds the existing
OS lease without recovering intents. An already-open version-2 writer fails
against its removed table names; new writers also check the exact version.
The separate rejection table contains no SQL. Every positive preparation
and refusal writer must check both decision tables within an `IMMEDIATE` SQLite
transaction while holding the existing OS journal lease. Reject any journal
containing both decisions for one client. Older writers must fail the journal
version check before mutation.

Preserve coordinator ordering: configuration gate, known retained or durable
decision, closed-owner check, OS lease, durable decision recheck, then admission
validation. Check in-memory positive runs and pending decisions before recording
a refusal. Once an in-memory positive run exists, subsequent recording failures
remain `Started` with pending recording even if no SQL was sent.

Freeze a refusal payload before attempting persistence. If an uncertain commit
cannot be confirmed, retain that exact payload in a SQL-free outbox, block its
identity from admission and return an ordinary error. Lookup may reconcile local
recording; it cannot execute SQL. Shutdown must flush refusals before releasing
the lease. A closed coordinator may read known decisions but may not reacquire
the lease. Process loss before successful persistence still leaves absence
unknown; this design does not promise proof when storage was unavailable.

Use a separately bounded refusal budget, initially 10,000 records, with dedup
checked before capacity. Do not silently prune records with history or profile
deletion. Capacity, lease, SQLite or readback failure cannot itself establish a
durable refusal. Practical retention across both decision tables remains a
separate required design.

## API and restoration

Keep the current start result and run-only lookup compatible. Add a SQL-free
request lookup returning null, an admitted run view, or the exact refusal. Use it
for frontend lost-response reconciliation. Null and arbitrary errors remain
uncertain; only the matching authoritative decision settles a request.

Extend schema validation, backup allowlists and metadata restore merging.
Restoring legacy or older backups must preserve current refusal tombstones.
Exact duplicates merge; changed payloads and positive/negative conflicts fail
before replacement. Version-2 missing refusals cannot be reconstructed.

## Local verification and remaining acceptance

Focused checks pass 205 frontend, nine IPC and 71 native cases (one internal child
fixture ignored by default). Full validation passes 893 frontend / 559 standard
Rust cases with 137 default ignores. The actual browser-to-Rust/helper/SQLite/
Arrow case passes at 1024 × 720, including ordinary clicks, keyboard receipt
scrolling, a dropped committed-refusal response and graceful restart with no
replay or lookup SQL. The SDK is synthetic. Current normal packaging, real
Snowflake effects, native UI/SSO/restart, million-row workflows and owner
acceptance remain open; see the [ledger](replacement-verification.md).

The local regression scope covers:

- Lose a committed refusal response, restart and recover it with zero helper SQL.
- Repeat it after authorization or policy changes; preserve the original refusal.
- Race positive and negative admission across coordinators; retain one decision.
- Reject malformed, foreign and changed identities without overwriting evidence.
- Inject insert, commit and readback failures; retain a pending immutable decision
  without publishing an unproven refusal or sending SQL.
- Exhaust refusal capacity while preserving existing decision lookup and dedup.
- Restore old backups without losing refusals; reject conflicting decisions.
- Preserve closed-owner dedup without claiming a replacement owner's lease.
- Clear history, delete profiles and restart without erasing admission evidence.
- Exercise frontend lost-response recovery end to end using the real Rust
  journal and isolated synthetic helper. Account semantics remain separately
  qualified; synthetic evidence grants no account authorization.
