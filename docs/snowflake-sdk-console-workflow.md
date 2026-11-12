# Reviewed Snowflake console workflow

This workflow is implemented and locally verified with an isolated synthetic SDK. It is not yet
qualified against the owner's Snowflake account or accepted as a DataGrip
replacement. The [verification ledger](replacement-verification.md) records the
source and checks; the [design](snowflake-sdk-console-scripts-design.md) remains
the complete acceptance contract.

## Connect and review

In an SDK external-browser Snowflake profile, explicitly enable **Enable reviewed
Snowflake SQL console**. This permits exact owner-written SQL, including writes,
schema changes and server-validated selected units. Existing profiles default to
off. Use a separately named profile for ordinary metadata browsing if needed.
Those profiles have separate physical sessions and do not share temporary
objects, variables or transactions.

A successfully connected console owns one physical session for its lifetime.
Automatic health SQL, metadata queries, completion lookups and generated-action
routes are suppressed on that session. Static editing remains available. The
application does not reconnect or reconstruct session state automatically.

Open a query tab and choose **Run all statements** to review parsed top-level
units, or select a complete unit and choose **Review selected server unit** for
server validation. Check the exact ordered SQL and scalar parameter values in
**Review this operation** before authorizing. Approval is one-use and bound to that native plan,
session, profile and policy generation. A policy or passphrase change revokes
future admissions even if a later settings write fails.

## Continue on the same session

Separate reviewed runs retain the same physical session. For example, a reviewed
`SET` followed by a later variable read, or `SHOW` followed immediately by a later
`RESULT_SCAN(LAST_QUERY_ID())`, must have no intervening application SQL. Explicit
`BEGIN`, later writes, and a later explicit `COMMIT` or `ROLLBACK` follow the same
rule. The application inserts no transaction boundary and does not settle a
transaction when a tab closes or a script stops.

These examples are acceptance scenarios, not proof of Snowflake behavior. Do not
run writes in an account without the required scoped authorization. Snowflake
DDL may implicitly commit; statement acknowledgement alone does not establish
committed effects or transaction state.

## Inspect and recover receipts

**Console receipts** is application-wide. It survives editing, closing or rebinding
the originating tab. Each run identifies its client request and native run, ordered
durable intents, original acknowledgements/rejections/unknown outcomes, proven
local non-dispatch and untouched suffix. Result delivery, truncation, cancellation
and recording failures are separate observations.

The receipt card stays above the status bar and uses a bounded scrollable region
so history cannot displace the editor or run toolbar. Pending notices stay visible
when settled history is collapsed. Focus **Console receipt details** to scroll
with the keyboard; use **Console receipts** to show or hide settled history. An
expanded card can cover a bounded part of the lower result area.

**Open retained results** opens existing process-local results without executing
SQL or signing in. **Reconcile receipt (no SQL)** looks up the same client identity;
it never resends a start request or resumes a suffix. A lost response is not proof
of non-execution. A native, identity-matched **No SQL was admitted** response allows
a fresh manual review; a generic error or missing receipt does not.

The local implementation now records a durable refusal before reporting
**No SQL was admitted**. A lost refusal response can be recovered using the same
SQL-free request lookup, including after restart. The original refused identity
stays refused after later authorization or policy changes; use a fresh manual
review to create a new request. A conflicting positive/negative response holds
new execution and cannot be dismissed by another matching refusal in that process.
See the [durability contract](snowflake-console-negative-receipts-design.md).
Local migration/failure checks, full validation and the actual browser recovery
case pass; see the ledger. This is synthetic-SDK evidence, not account qualification.

Native history is now authoritative for browsing. Open **Console receipts** even
when no connection profiles or browser correlations remain. The history shows 25
SQL-free summaries per page; filter visible/archived decisions or connection, use
**Next receipts** / **Previous receipts**, or enter an exact client request UUID.
Select a summary to inspect its original decision. Only one detail expands at a
time; recent positive and refusal details share a bounded cache.

**Archive receipt** changes a local browsing preference. It keeps every original
ACK, unknown outcome, refusal and deduplication identity, and does not reclaim
journal disk space. A lost archive response is reconciled by exact native lookup;
it is not retried automatically. Unknown outcomes remain unknown when archived.
**Release retained result rows** has a separate confirmation. It releases only
that settled run's process-local buffers and removes them from all editor tabs;
original row counts/outcomes remain evidence. After release, unavailable rows do
not mean the query returned zero rows. An interrupted release response leaves
local state until an exact SQL-free lookup can establish the remaining buffers.

Only opaque request UUIDs are written to browser persistence. A fresh exact native
inventory confirmation is required before removing each browser correlation.
Malformed or quota-failed storage is preserved and blocks new submissions;
recent full-detail payload compaction does not depend on a successful browser
write. Native inventory can rediscover previously committed positive and negative
IDs even if older browser state discarded them. It cannot reconstruct a refusal
that version 2 never committed. A missing lookup remains unknown and never permits
replay. Contradictory decisions keep execution held.

The lifetime decision-count gates remain unchanged. Storage accounting reports
logical charged bytes, not reclaimable disk or an implemented admission byte
budget. Metadata backups are still plaintext with the existing 64 MiB / 20-copy
limits. Encrypted streaming backup/restore and full-capacity validation remain
required; see the [retention design](snowflake-console-retention-design.md).

A recording hold retains the owner and pending evidence in memory and prevents
normal shutdown/new admission until local recording succeeds. Forced process
termination during a storage failure can lose that in-memory evidence. After a
restart, receipts contain no result rows or SQL; unavailable rows do not mean
zero rows were returned. An abandoned durable intent is recovered as unknown,
without a login, replay or automatic commit/rollback.

## Reproducible acceptance

The synthetic browser case `tests/e2e/snowflake-retained-console.spec.ts` uses the
real UI, Rust Studio, helper framing, journal and Arrow result store with an
isolated synthetic SDK. It checks session adjacency, transaction-shaped sequences,
lost-start-response recovery, committed-refusal response loss, and backend
restart without replay. It also edits the real profile form from the legacy
65-second default to 120, verifies the native review value, and preserves that
setting across full UI/backend restart. See the
[query-limit contract](snowflake-console-query-limits.md). At 1024 × 720 it checks ordinary run/review clicks
after eleven receipts and keyboard scrolling/reconciliation in the bounded panel.
Full local validation passes 921 frontend / 561 standard Rust tests (137 default
ignores); later harness-only corrections pass three regressions and strict Rust
checks, followed by the actual browser pass. This does not qualify actual Snowflake effects, cancellation, SSO,
performance or packaged native UI behavior. Normal SDK-bearing ARM64 packaging
from clean `72ee3e4` includes the configured limit and passes resources, strict
ad-hoc signature and guarded helper startup with zero authentication/SQL. The
local app remains uninstalled; normal native UI/SSO/restart acceptance remains
open. The ledger records its binary hash and preserved predecessor.

Account acceptance still needs a fresh exact reviewed plan and billing scope for
the permitted disposable `SANDBOX.BILLIE_TESTING` target, then independently read
back write/rollback/commit and DDL effects. Complete interruption, long-running
query behavior, million-row paging/export, normal packaged macOS SSO/restart and
hands-on acceptance. The configurable local limit still has a finite 24-hour
ceiling and needs account qualification; bounded journal capacity needs a practical
qualified replacement. All other applicable
DataGrip capabilities remain required by the
[matrix](datagrip-feature-matrix.md).
