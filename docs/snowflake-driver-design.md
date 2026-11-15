# Snowflake stateful driver design

Status: transport/read-driver foundation and explicit application profile
integration are implemented and locally qualified. The application offers SDK
external-browser and legacy local-application OAuth/SQL API routes; both are
read-only. The approved two-query SDK helper canary passes actual SSO and bounded
read contracts. Normal packaged account sign-in and the remaining daily workflows
are unqualified. This document records the route and its remaining safety gates.
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
`src-tauri/src/drivers/snowflake_process.rs`; its application integration is
recorded in [SDK integration evidence](snowflake-sdk-integration.md).

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

A bounded 10 November review of the official
[Node resubmission guidance](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute#resubmitting-requests)
finds a documented duplicate-prevention contract, illustrated with an uncertain
INSERT, requiring the original connection and request ID. Recognized IDs retain
the original SQL; unrecognized IDs can execute the supplied SQL. No deduplication
retention interval or detailed transaction-control/DDL recovery contract was
found in that bounded documentation/source review. The 24-hour RESULT_SCAN
window does not establish such an interval. The SQL API's separate
`/api/v2/statements` plus `retry=true` contract must not be transplanted to the
Node driver's `/queries/v1/query-request` route. Pinned source renews existing
session tokens with RENEW; that does not authorize fresh-login replay.

Authorized mutation qualification should lose a terminal successful server
response before SDK delivery, record its query ID without credentials, allow the
natural retry, and require the recovered query ID to match. Verify durable effects
from a separate observer. Cover autocommit DML, transaction DML, BEGIN, COMMIT,
ROLLBACK and DDL separately: repeated transaction controls can be ignored, so row
counts alone cannot prove deduplication. DDL's implicit commit remains independent
of retry safety. Test session loss and unresolved outcomes separately; short fault
tests cannot establish an undocumented long-outage recovery guarantee. This adds
a concrete acceptance design without claiming any external test has run.

The transport-foundation checks above did not exercise a real Snowflake account.
Subsequent local macOS resource/package/native qualification passes, and the
separately approved two-query SDK helper canary passes real external-browser SSO,
context/session continuity, scalar values and local cleanup. Neither closes
normal packaged account sign-in, remote cancellation, transactions or full daily
acceptance. Before replacement readiness, require at least:

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
design does not narrow them; the integration guide and ledger distinguish the
completed narrow checks from open gates.


## Application integration contract: explicit SDK sessions

The current [application integration](snowflake-sdk-integration.md) implements
profile routing, fixed runtime selection and retained cleanup from this contract.
Combined local native/package qualification passes for that integration source.
Subsequent [same-session completion](snowflake-sdk-metadata-design.md) is implemented
with separate qualification. Complete SDK browsing/inspection, normal packaged
account qualification and the write/transaction gates remain open. The reviewed
integration sequence was:

1. Add an immutable runtime factory using the packaged `snowflake-runtime`
   resource directory. Development may explicitly select the repository's
   prepared arm64 stage in debug builds. Never accept executable paths through
   IPC/profiles, search PATH, or silently fall back to a global runtime or the
   other authentication mode. Validate fixed paths, containment, permissions,
   manifest pins and helper hashes before accepting the factory; perform hashing
   outside Studio's configuration gate. Missing resources fail only explicit SDK
   connections and leave existing engines and legacy OAuth available.
2. Add a serialized Snowflake authentication mode. Missing fields in existing
   profiles/backups retain legacy local-application OAuth. External browser SSO
   is an explicit selection; unknown modes reject. Regenerate contracts and test
   persistence/backup round trips, legacy defaults and routing.
3. Reserve cleanup capacity under the connection UUID while holding the
   configuration gate, before creating a helper state root. The tracked factory
   consumes that non-cloneable permit outside the gate and attaches retained root
   cleanup authority immediately after allocation, before further fallible
   initialization or an authentication await. A pending slot remains visible to
   cancellation/shutdown snapshots; dropping its permit without allocating a root
   settles it as no launch. Attachment cannot fail because admission closed or
   capacity changed. Recheck admission, cancellation and profile consistency
   under the gate before polling authentication. Cancellation, profile replacement/deletion and shutdown must retain
   cleanup ownership independently of the caller and session references. Keep
   internal retirement synchronous; command-facing close/cancel waits for bounded
   cleanup receipts. Shutdown closes admission, drains setup owners, then awaits
   every retained helper receipt. A timeout/failure remains tracked and prevents
   successful native exit approval. Retain the original root's native identity
   and serialized removal authority after a failed deletion. A bounded retry may
   remove that same root only after no-child or confirmed-reaped disposition;
   replacement directories and symlinks must remain untouched. Cancellation of a
   blocking worker does not stop it: that worker must retain the permit/authority
   and publish its eventual result. A receipt proves local child reaping and root
   removal only; it does not prove logout, remote cancellation or rollback.
4. Package the exact prepared tree without flattening helper dependencies or
   contained symlinks. Extend hidden native preparation to include resources,
   verify startup from the actual application resource directory, and record
   artifact identities. Validate signing/hash policy explicitly; notarization,
   installation and external account acceptance remain separate gates.
5. Implement metadata and live context through the same SDK connection used for
   the console. Never substitute a separate SQL API/OAuth session. The integration
   now gates unsupported automatic discovery instead of treating it as an
   authentication failure; subsequent same-session completion is implemented.
   Keep browsing/inspection and other unsupported UI actions behind truthful
   capability flags, clearing stale state, until their workflows are complete.

Required local regressions include missing/tampered resources, hostile or empty
PATH and NODE_OPTIONS, legacy profile/backup compatibility, and explicit mode
routing. Use the real helper with an isolated synthetic SDK to prove cancellation
before spawn, during authentication, abandoned connect futures, late publication,
disconnect with retained driver references, and shutdown success/failure. Retain
failed cleanup evidence/capacity and test an explicit recovery path; repeatedly
retrying a permanent failed receipt is not remediation. Verify packaged startup
separately from these protocol tests.

This increment may initially expose stateful reads while writes/DDL/transactions
remain gated by SDK retry/outcome proof. That intermediate boundary does not
change the required replacement target. Complete metadata, actual externalbrowser
SSO, million-row workflows, mutations and explicit transactions still need their
own evidence and owner acceptance.

## Reviewed-table generation remains a required write gate

A bounded official-documentation review on 10 November rules out
[`SYSTEM$REFERENCE`](https://docs.snowflake.com/en/sql-reference/references#objects-identified-by-name)
as an original-table generation guard. References identify objects by name.
Drop/recreate can authorize the replacement if the encoded roles retain
privileges; rename invalidates the reference and SWAP follows the name.
SESSION or PERSISTENT scope does not change this behavior. References can be
passed to `IDENTIFIER(?)`, but successful name binding does not protect reviewed
grid or CSV writes against concurrent replacement. No account test was run.

A targeted recheck on 12 November confirms the same documented name binding,
including replacement and SWAP behavior; it supplies no new atomic generation
pin. This refresh is limited to the linked reference documentation, not the full
Snowflake baseline. The grid/CSV design gate remains open; no account access or
mutation test was performed.

[`ACCOUNT_USAGE.TABLES.TABLE_ID`](https://docs.snowflake.com/en/sql-reference/account-usage/tables)
has documented metadata latency up to 90 minutes.
[`SYSTEM$GET_REFERENCED_OBJECT_ID_HASH`](https://docs.snowflake.com/en/sql-reference/functions/system_get_referenced_object_id_hash)
detects replacement, but its documented input is an installed Native App's
manifest reference name; the examples explicitly are not standalone consumer
statements. Neither establishes atomic ordinary-session DML targeting here.
The review found no documented ordinary-session generation pin; this is a
bounded finding, not proof that no supported mechanism exists.

Snowflake's [transaction rules](https://docs.snowflake.com/en/sql-reference/transactions)
do not establish a metadata-check-then-write lock: statements use READ COMMITTED,
and DDL implicitly commits. Hybrid-table row locks and temporary-table name
precedence need separate qualification. Before enabling reviewed grid/CSV writes,
establish a supported atomic target-acquisition mechanism and test two-session
rename, DROP/recreate, SWAP, empty-table, zero-row and temporary-shadow races.
Writes, schema changes and transactions remain required replacement capabilities.
