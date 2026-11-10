# Snowflake stateful driver design

Status: implemented and locally tested transport/read-driver foundation;
application integration and external qualification remain open. This document
records the route and the proof required before enabling it. The
current application still uses Snowflake's SQL API with local-application
OAuth and read-only behavior; this design does not change that implementation.
See the [replacement inventory](snowflake-datagrip-replacement.md) and
[official capability baseline](snowflake-official-baseline.md) for the owner's
requested workflow and current product boundary.

## Route and scope

The owner requires Snowflake SAML SSO through `EXTERNALBROWSER` on macOS, plus
the Snowflake reads, writes, schema changes, explicit multi-statement
transactions, and DataGrip capabilities applicable to that engine. An OAuth
browser sign-in or a successful SQL API query does not establish this
authentication or stateful-session requirement. The selected prototype route
is the official Snowflake Node.js driver (`snowflake-sdk` 3.4.1) in a pinned
Node 24 helper, supervised by Rust. This is not a claim of DataGrip parity or
production readiness.

The official Node driver documents `authenticator: "EXTERNALBROWSER"` and an
external-browser callback in its [authentication guide](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-authenticate)
and [connection options](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-options).
Its [2026 Node release notes](https://docs.snowflake.com/en/release-notes/clients-drivers/nodejs-2026)
also describe browser redirect/response options and `serverSessionKeepAlive`.
These documents establish supported APIs, not that Joinery's packaged macOS
application, owner's IdP, callback cancellation, or account policy works.
Keep externalbrowser SAML distinct from the existing local-application OAuth
flow.

Pinned implementation inputs are `snowflake-sdk` 3.4.1 (Apache-2.0; package requires
Node >=20), Node 24.22.2, and the Rust/Tokio supervisor. Versions and
notices are recorded in [the SDK license audit](snowflake-sdk-licenses.md).
The final product must bundle and sign a supported runtime; it must not depend
on a developer's global Node installation.

## Session and process contract

Use one helper process, one SDK connection, and one fresh application
generation per console. A Rust supervisor owns its lifetime and communicates
over bounded, length-prefixed JSON frames. The prototype contract uses a
1 MiB frame limit, monotonic wire request IDs, generation checks, a bounded
queue, at most 32 row credits, and exactly one terminal result per accepted
operation. Protocol violations, uncertain session state, and uncertain
connection outcomes retire the child. The supervisor must reap it; a kill or
socket loss is not evidence that Snowflake canceled work or rolled back a
transaction.

The supervisor exposes controls through an opaque operation token
bound to the exact supervisor and operation ticket. A stale control cannot bind
to a later operation that reused the same local name. Its reproduced late-Drop
race is repaired: cleanup of a completed operation is a no-op, while genuine
control failures still retire the child. This reviewed transport is now in
`src-tauri/src/drivers/snowflake_process.rs`; application integration remains open.

Keep all commands for a console serialized on the same SDK connection. On
connect, verify the expected server identity and role/database/schema/warehouse
on that session before exposing it. Context changes and health checks must not
issue `ALTER SESSION SET AUTOCOMMIT`: Snowflake documents that changing
AUTOCOMMIT implicitly commits an active transaction, even when the value is
unchanged ([transaction semantics](https://docs.snowflake.com/en/sql-reference/transactions)).
The same documentation says DDL implicitly commits an active transaction and
that concurrent use of one connection shares transaction context. Therefore,
transaction ownership is per console/session, not per UI request or worker
thread. DDL must have an explicit product policy because it cannot be rolled
back with the surrounding DML transaction.

The SDK's `serverSessionKeepAlive` remains false so a normal destroy requests
session logout. Its exact server effect and timing still require live
qualification. Do not enable client credential/MFA caches by default. Use an
explicit private client configuration path, isolated temporary OCSP/CRL cache
locations, and a clean, narrowly allowlisted child environment. These are
defense-in-depth settings, not an OS sandbox or a proof that the SDK makes no
other filesystem, telemetry, or network access.

## Results, cancellation, and transaction outcomes

Request array rows to preserve duplicate column labels and request exact string
representations for numeric, temporal, binary, and JSON-like values. Keep
`NULL` distinct from empty strings; reject unexpected SDK value types rather
than coercing rounded JavaScript numbers. Stream bounded batches into the
application's result sink and return credits only after batches are accepted
and released. A row/byte retention limit must stop retaining rows while the
driver continues draining the real stream terminal, including late stream
errors. Helper limits allowing one million rows do not qualify application
memory use, paging, export, or one-million-row behavior.

Report `dispatching` as “may have been submitted,” not server receipt. An SDK
success callback acknowledges statement success; it is not a transaction
commit. A successful query terminal follows stream completion. A transport
failure before acknowledgement is `outcomeUnknown`; failure after a prior
acknowledgement preserves that acknowledgement and retires the session. Do not
infer rollback from connection close, child termination, or a missing event.
Do not replay SQL on a fresh connection.

The important unresolved constraint is in the pinned SDK itself. The exact
3.4.1 [`statement.js`](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/statement.js)
uses Snowflake retry middleware for statement execution and cancellation.
The official [`execute` documentation](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute)
describes resubmitting a request with the same `requestId` after an uncertain
response. The helper does not itself retry, and the inspected public options
do not provide a supported switch to disable statement retry. `retryTimeout: 0`
is a login retry budget, not a statement-retry disable. Until fault injection
and source-level review establish safe behavior for writes and transaction
boundaries, keep mutation, DDL, transaction controls, and caller-classified
unknown statements disabled through this route. Do not claim “no retry” merely
because the wrapper never resubmits.

Cancellation is a separate operation. It may call the active SDK statement's
cancel method, but its acknowledgement does not settle the original statement
or prove remote cancellation, write outcome, or transaction rollback. Keep the
original operation alive until its terminal result or deadline, then retire the
session when its state is uncertain. Cancel during externalbrowser connect or
forced close requires bounded child termination and process reaping; late
browser/network callbacks must not resurrect that generation.

Snowflake's [SQL API transaction rules](https://docs.snowflake.com/en/developer-guide/sql-api/using-transactions)
require the transaction's begin, statements, and end to be sent together in
one request. SQL API therefore cannot provide a transaction that remains open
across user actions. A stateful SDK connection is the candidate for explicit
multi-action transactions, subject to same-session and loss-injection proof.

## Evidence and gates

The helper in `snowflake-helper/` passes **31 tests** with pinned Node
24.22.2 and SDK 3.4.1, rerun on 2026-11-10. Twenty-five exercise the framed protocol,
isolated child configuration and synthetic streams, including two million-row
streams. Six use the actual SDK against an isolated loopback HTTP service. The
Rust supervisor passes **8 unit and 30 integration tests**, covering bounded
streaming, protocol faults, cancellation, stale controls, handshake abandonment,
graceful/forced shutdown, runtime abandonment and cleanup receipts. Scoped strict
Clippy and formatting pass; independent review has no material finding. The
read adapter adds **11 integration tests** using the real framed helper with a
synthetic SDK. These cover exact typed bindings, rejection of forged compiled
statements, identity checks, late errors after result limits, cancellation versus
original terminal outcomes, and cleanup outside the caller's runtime. A
deterministic barrier reproduced a rejected late credit/discard masking the
original successful terminal; the repaired driver preserves that terminal.
Integer/boolean placeholders have explicit SQL types, preserving typed NULLs
and exact integer strings. This does not qualify remote Snowflake coercion. Run
`pnpm test:snowflake:helper` after locked dependency installation and
`pnpm test:snowflake:process` with pinned Node/Rust. These qualify local mechanics.

The macOS arm64 runtime staging command passes offline preparation and nine
preparation tests. It verifies the pinned Node archive/binary, original signature,
locked SDK dependencies and bounded source hashes, and promotes the stage
atomically. `pnpm test:snowflake:startup` launches the staged Node and actual SDK,
checks a single hello frame and clean EOF exit, and detects no attempted Node
socket/DNS traffic with its guard. This is an import/startup check, not an OS
network sandbox, SSO test or packaged-app check. See the
[runtime preparation guide](snowflake-runtime.md) for commands and limits.

Parent ownership now precedes the first handshake await. The parent supplies a
private `0700` root, and removes it only after an actual child wait. An abandoned
runtime retains an uncertain root without claiming cleanup. The helper requires
that root before SDK import. Missing-root behavior was reproduced before repair.
Actual SDK connect also exposed that a missing client-config file fails before
HTTP, and that the SDK requires its uid and primary gid to match. The helper now
creates an exclusive `0600` valid configuration through an owned file descriptor,
sets its uid/gid, and selects logging `OFF`. SDK connect can reset its logger;
this is not a promise of immutable custom logging or an OS sandbox.

The actual SDK retransmits after a lost successful response: two POSTs use the
same request ID and body hash, one SDK callback follows, and no sequence ID was
observed. A simulator with request-ID deduplication applies one effect; without
deduplication it applies two. Documented login retry settings and unsupported
statement-retry settings do not disable the retransmission. Persistent response
loss has no terminal callback within the bounded 6.5-second observation; that is
not retry exhaustion. The fake server does not establish Snowflake's own
deduplication, commit, rollback or interruption behavior. Writes remain gated.

No real Snowflake account, browser SSO, external IdP, remote statement,
transaction or remote cancellation was exercised. There is no current macOS
package qualification for this driver. Before implementation can
replace the current mode, require at least:

1. Owner-authorized macOS externalbrowser sign-in, callback lifecycle, account
   identity, role and warehouse checks, and clean disconnect.
2. Actual statement-context continuity, session retirement, reconnect
   prevention, exact value fidelity, streaming limits, late-error handling,
   cancellation, and unknown-outcome fault injection against disposable
   Snowflake fixtures.
3. SDK retry behavior demonstrated at the wire/server boundary before enabling
   writes; explicit commit/rollback tests with connection loss around each
   boundary; DDL implicit-commit UX and recovery requirements.
4. Bundled Node and SDK packaging, dependency/runtime notices, filesystem and
   telemetry review, resource limits, macOS signing/notarization, and a clean
   installed-app test.
5. Separate acceptance for the owner's full DataGrip capability inventory,
   including query history/profile, editing, import/export, schema tools,
   keyboard accessibility, and AI entitlement/provider costs. Driver support
   does not imply those workflows are implemented or equivalent.

The owner's SSO and workflow requirements remain acceptance criteria. This
design does not narrow them or assert that any gate has passed.
