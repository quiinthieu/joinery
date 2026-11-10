# Snowflake SDK submission guard proposal

Status: read-only feasibility design, 2026-11-10. No guard is implemented or
qualified by this document. No new tests, network requests, authentication or
account SQL were performed during the investigation. The SDK route's existing
ReadOnly gate remains unchanged: writes, DDL, transaction controls and unknown
statements remain disabled. The full requirements in
[snowflake-datagrip-replacement.md](snowflake-datagrip-replacement.md) remain open.

The earlier six actual-SDK loopback scenarios recorded retransmission after a
lost acknowledgement, including two query POSTs and two simulated effects when
the fake server did not deduplicate. They establish the problem, not this
proposal's effectiveness or Snowflake's remote deduplication behavior. See
[driver design](snowflake-driver-design.md) and
`snowflake-helper/test/transport.test.mjs`.

## Scope and rationale

Investigate an original wrapper around Node's native `http.request` and
`https.request`, installed before the SDK import in
`snowflake-helper/src/main.mjs`. Each helper owns one SDK connection and a
serialized statement lane. This makes a child-process-local guard practical:
the application can identify the one intended submission without changing
global Node configuration, dependency source, the owner's environment or other
application processes. Do not patch installed SDK/Axios files or introduce an
IPC/profile-selected transport module.

The proposed invariant is **at most one admitted native query submission for
each registered noReplay statement**, even if the SDK attempts another request.
It does not promise one remote effect, delivery, commit, rollback or a known
outcome. A server can execute the first submission before its response is lost.
The wrapper is not an OS network sandbox or a general guarantee for arbitrary
SDK/dependency versions.

## Inspected pins and request chain

Inspection used the repository lock and staged resources at
`.cache/snowflake-runtime/macos-arm64/helper/node_modules/`:

| Component        | Exact inspected version       |
| ---------------- | ----------------------------- |
| Node runtime     | 24.22.2, existing runtime pin |
| snowflake-sdk    | 3.4.1                         |
| axios            | 1.21.0                        |
| follow-redirects | 1.16.3                        |

The last three versions match `snowflake-helper/package-lock.json` and the
staged package metadata. The following evidence paths are relative to that
staged `node_modules` directory; line numbers describe the inspected files.
Recheck them whenever the lock or runtime changes.

| Source                                                         | Observed behavior                                                                                                                     |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `snowflake-sdk/dist/lib/connection/connection.js:344`          | `execute` constructs a pre-execution statement.                                                                                       |
| `snowflake-sdk/dist/lib/connection/statement.js:1072`, `:1236` | Builds POST `/queries/v1/query-request?requestId=…` and enables retry middleware.                                                     |
| `snowflake-sdk/dist/lib/services/sf.js:544`                    | Adds session headers/parameters and calls the HTTP client; token renewal can subsequently resubmit queued work.                       |
| `snowflake-sdk/dist/lib/http/base.js:36`, `:79`, `:265`        | Calls its private Axios instance on the next tick for normal statement requests; constructs a closed request configuration.           |
| `snowflake-sdk/dist/lib/http/axios_instance.js:65`             | Retries network errors and selected HTTP failures after an asynchronous sleep. A thrown wrapper error alone can therefore be retried. |
| `axios/dist/node/axios.cjs:2523`, `:4511`, `:4830`, `:4902`    | Node selects the supported HTTP adapter; HTTP version defaults to 1; dispatch uses native HTTP(S) or follow-redirects.                |
| `follow-redirects/index.js:92`, `:361`                         | Retains its options object and passes that exact object to the native protocol's `request`.                                           |

The SDK HTTP builder does not forward a caller-selected Axios adapter,
`httpVersion` or custom transport. Axios contains HTTP2 and fetch implementations,
but this inspected statement path does not select them. SDK platform detection
has a separate fetch path, disabled before import by the existing helper.
These are bounded source findings, not a claim that native HTTP wrappers cover
all possible future transports. A changed adapter, direct `ClientRequest`,
native `get`, HTTP2 or fetch path requires renewed analysis and tests; do not
silently treat it as guarded.

The code graph covers the helper source with no recorded gap. Staged dependency
directories are deliberately excluded from the graph, so their transport claims
come from targeted direct source reads. Clean coverage metadata is only a
best-effort signal. This design does not authenticate every staged dependency
file; implementation must tie its reviewed transport behavior to exact source
digests/pins and reject unsupported versions rather than relying on SDK version
alone.

## Admission and lifetime

1. Install the wrappers before importing the SDK. Preserve original function
   references privately and do not expose an uninstall or reset path to IPC.
2. Register an operation generation and one individual intended `execute`
   invocation before any query dispatch. Obtain and validate the exact SDK
   `statement.getRequestId()` synchronously after `execute` returns. Normal
   scalar requests reach native transport on a later tick in the inspected
   source. If registration cannot precede dispatch, reject it; do not guess that
   the next observed UUID belongs to the operation. Supplying a caller
   `requestId` is not an equivalent workaround: the SDK treats it as resubmission.
3. Normalize supported Node request argument forms without changing their
   effective authority, path, method or callback. The inspected Axios path uses
   an options object. Reject ambiguous or unsupported query-endpoint shapes,
   missing/duplicate/malformed request IDs, unexpected authority and unexpected
   methods before calling native transport. In particular, a query-endpoint GET
   must be denied, not exempted because only POSTs are counted. Do not forward
   suspicious alternate encodings as an unclassified non-query request.
4. Permit one canonical POST for that exact registered statement ID. Consume
   its allowance before invoking the original native function, including when
   that invocation throws. A conservative unknown outcome is preferable to
   admitting another submission after an ambiguous local failure.
5. Keep ownership and the spent allowance through acknowledgement, result
   draining and the terminal decision. A cancel acknowledgement does not settle
   the original statement. Register each individual execution separately: the
   existing helper `context` operation can execute USE followed by SELECT, so a
   whole helper envelope is not one statement.
6. Deny unowned or stale query submissions. A delayed retry from an earlier
   operation must not consume a new operation's allowance. Read operations also
   need explicit ownership if sharing this wrapper; their replay policy must be
   deliberate rather than an unarmed pass-through that lets old work escape.
7. A violation irreversibly closes submission admission for the helper. Notify
   the helper directly, settle a still-pending uncertain operation with a safe
   `OutcomeUnknown`, stop queued work and retire the process. Do not depend on
   the SDK recognizing an error code or exhausting retries. Preserve an already
   established acknowledgement/outcome and never replace an already-published
   terminal with a second terminal. The existing supervisor still owns bounded
   termination, child reaping and cleanup receipts.

Use explicit process-local ownership rather than relying solely on async-local
context: existing sockets, callback scopes and delayed SDK tasks need not follow
the intuitive lifetime of a JavaScript call. Do not clear a failure latch in
`finally` and reopen admission while a retry timer remains alive.

## Redirect enforcement

A path-only POST counter does not suffice. A 307/308 can resend the body to a
different path or authority; a 301/302/303 can change POST into GET. Reject all
redirects originating from the admitted query request before another native
request is created, regardless of the next method or path.

In pinned follow-redirects 1.16.3, `request()` merges options once; the constructor
sets `this._options = options`; `_performRequest()` passes `this._options`
directly to `nativeProtocol.request`. Thus a wrapper assigning
`options.maxRedirects = 0` before the original native call modifies the actual
object retained by `RedirectableRequest`, not a clone. `_processResponse()`
checks `++this._redirectCount > this._options.maxRedirects` at line 447 before
method/URL rewriting and before another `_performRequest()`.

This is pinned dependency behavior, not a public Node redirect-control API.
Changing a cloned options object would not enforce the limit. The implementation
must also observe a redirect response and latch helper failure promptly, for
example before delegating the native response callback; otherwise Axios can
sleep and retry its redirect-limit error. The spent allowance independently
blocks that retry. Do not expose Location, request URLs or raw transport errors
in protocol diagnostics.

## Implicit SQL and supported modes

Do not let an unreviewed SDK setup statement consume the sole allowed POST.
Before calling `connection.execute`, preserve the helper's strict scalar binds
and closed option set:

- Only a flat bind array of null, Boolean or bounded string values. Integers,
  decimals, temporal, binary and JSON payloads remain canonical strings under
  the existing typed-binding validation; JSON text is not a nested bind array.
- Reject nested arrays, bind-stage/file-stream inputs, PUT/GET transfer modes,
  caller-supplied SDK execution options, async/detached execution, resubmission
  and multi-statement modes in this first foundation. Later batch/file workflows
  require their own complete design rather than relaxing this contract.
- Pin a positive array-binding threshold if absence of staging is an invariant,
  rather than depending on a server-provided threshold. Do not accept a profile
  override for it.

SDK `statement.js:50–80` selects staged binding when `countBinding` exceeds its
threshold. `countBinding` at line 1319 counts only nested array elements, so the
current helper's validated scalar array counts as zero. The staged path at line
764 uploads binds before the intended statement; `bind_uploader.js:5`, `:25`
issues `CREATE OR REPLACE TEMPORARY STAGE SYSTEM$BIND`, followed by PUT. Its error
path can fall back to inline execution. An accidentally admitted staging mode
could therefore execute hidden DDL before the user's statement, including
transaction-boundary effects that a submission count alone cannot make safe.
Exact statement-ID admission provides a second check against its distinct hidden
request IDs, but does not replace rejecting the mode before SDK invocation.

PUT/GET selects the SDK file-transfer path before bind staging. Query-context
cache handling inspected here instead builds an in-memory DTO, included in the
query body; it does not issue setup SQL. Do not generalize that observation to
other execution modes or future SDK versions.

## Required local proof

Reuse the actual pinned SDK fixture in `snowflake-helper/test/transport-probe.mjs`
with disposable owned listeners, synthetic credentials/data, outbound socket/DNS
denial and bounded output. Do not replace SDK transport methods in the proof;
the new production wrapper is the component under test. Keep the existing
unguarded retransmission baseline and demonstrate that removing enforcement
reproduces the second submission.

| Scenario                                                                    | Required observation                                                                                                                            |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Successful scalar-bound statement                                           | Exactly the intended SQL is the first and only query POST; success and result streaming remain intact.                                          |
| Server accepts/applies then drops acknowledgement                           | One server-observed query POST/effect; retry denied before native dispatch; prompt unknown outcome and retirement. No deduplication assumption. |
| Retryable HTTP status, socket reset, delayed retry                          | No second query POST; irreversible latch remains closed through SDK timers and attempted queued work.                                           |
| 301/302/303 and 307/308                                                     | No second destination request, including changed path, authority and protocol; a method change cannot escape query enforcement.                 |
| Token expiry/renewal and queued resubmission                                | Renewal does not authorize another submission of a spent statement.                                                                             |
| Cancel before dispatch, during lost response and after acknowledgement      | Correct original terminal/acknowledgement preserved; cancel ACK does not reopen allowance or assert rollback.                                   |
| Abandoned operation, timeout and stale callback                             | No subsequent operation can inherit the old request's allowance; process is actually reaped before state removal.                               |
| Nested binds, file/PUT/GET, extra options and unsupported batch modes       | Rejected before SDK invocation; zero query submissions and no stage creation/upload.                                                            |
| Malformed ID, duplicate ID parameter, unexpected query method/options shape | Denied before native dispatch; bounded redacted diagnostics only.                                                                               |
| Direct HTTP, local TLS and CONNECT proxy                                    | Same query count boundary, including reused sockets; preserve real TLS verification and agent behavior.                                         |
| Dependency/source pin mismatch and guard-removal mutation                   | Unsupported source fails closed; removing enforcement makes the relevant retransmission/redirect regression fail.                               |

The wrapper should not count certificate OCSP/CRL requests, result downloads,
logout or cancellation as user SQL submissions, but those routes must not become
an escape for a query or redirected body. HTTPS proxy CONNECT is emitted by the
agent through raw net/TLS after the native target request has been admitted;
loopback proxy proof must verify that a rejected second target request cannot
start a second tunnel. TLS tests must use isolated test trust material without
disabling production certificate verification.

Persist only fixed diagnostic categories, counters and cleanup facts. Do not
log SQL, binds, tokens, request/query IDs, proxy credentials, full URLs, Location
or raw errors. Reap test children before deleting their owned private state;
leave failed cleanup observable rather than sweeping unrelated directories.

## Gates after a successful foundation

Passing these tests would establish a bounded local noReplay submission
foundation for these exact sources. It would not by itself enable writes or
establish remote transaction semantics. Keep the existing ReadOnly gate until
the application has truthful DML/DDL results, transaction and autocommit state,
batch behavior, cancellation/deadline outcomes, uncertain-session retirement and
recovery behavior, plus separately authorized actual-account qualification.
Neither disconnect nor helper termination proves rollback. Do not replay an
uncertain statement on a replacement connection.

No new documented public retry-disable alternative was established by this
local-only investigation. The existing request-ID resubmission API addresses a
different contract and does not implement this guard's noReplay invariant.
