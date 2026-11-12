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

**Open retained results** opens existing process-local results without executing
SQL or signing in. **Reconcile receipt (no SQL)** looks up the same client identity;
it never resends a start request or resumes a suffix. A lost response is not proof
of non-execution. A native, identity-matched **No SQL was admitted** response allows
a fresh manual review; a generic error or missing receipt does not.

There is a remaining recovery gap: if a definitive pre-admission refusal response
itself is lost, the backend has no durable negative receipt. Its lookup can remain
missing and the UI conservatively holds new admissions. Durable negative
correlation/recovery is required before daily replacement acceptance; the
[reviewed design](snowflake-console-negative-receipts-design.md) is not yet
implemented. Never infer
that such a request is safe to resubmit from its error code or a null lookup.

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
lost-start-response recovery and backend restart without replay. This does not
qualify actual Snowflake effects, cancellation, SSO or performance.

Account acceptance still needs a fresh exact reviewed plan and billing scope for
the permitted disposable `SANDBOX.BILLIE_TESTING` target, then independently read
back write/rollback/commit and DDL effects. Complete interruption, long-running
query behavior, million-row paging/export, normal packaged macOS SSO/restart and
hands-on acceptance. The current fixed foundation delivery deadline and bounded
journal capacity need practical qualified replacements. All other applicable
DataGrip capabilities remain required by the
[matrix](datagrip-feature-matrix.md).
