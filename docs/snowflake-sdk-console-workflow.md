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

Only opaque request UUIDs are retained in browser persistence, including settled
refusals; native lookup supplies the proof after restart. This change cannot
rediscover negative IDs discarded by older versions or reconstruct refusals that
were never recorded by version 2. If recording fails, the exact refusal remains
pending in memory, without an authoritative negative response. Process loss
before a successful commit still leaves absence unknown. Never infer safe replay
from a transport error, arbitrary error code or null lookup.

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
restart without replay. At 1024 × 720 it also checks ordinary run/review clicks
after eleven receipts and keyboard scrolling/reconciliation in the bounded panel.
Full local validation passes 893 frontend / 559 standard Rust tests (137 default
ignores). This does not qualify actual Snowflake effects, cancellation, SSO,
performance or packaged native UI behavior. Normal SDK-bearing ARM64 packaging
from clean `9b20aa2` passes resources, strict ad-hoc signature and guarded helper
startup with zero authentication/SQL. The local app remains uninstalled; normal
native UI/SSO/restart acceptance remains open.

Account acceptance still needs a fresh exact reviewed plan and billing scope for
the permitted disposable `SANDBOX.BILLIE_TESTING` target, then independently read
back write/rollback/commit and DDL effects. Complete interruption, long-running
query behavior, million-row paging/export, normal packaged macOS SSO/restart and
hands-on acceptance. The current fixed foundation delivery deadline and bounded
journal capacity need practical qualified replacements. All other applicable
DataGrip capabilities remain required by the
[matrix](datagrip-feature-matrix.md).
