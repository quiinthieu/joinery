# Snowflake DataGrip replacement on macOS

## Confirmed acceptance target

On 9 November 2026 the owner confirmed Snowflake on macOS with
`authenticator=externalbrowser` SSO, **everything DataGrip currently provides for
that workflow**, and explicitly reads, writes, schema changes and transactions.
The [official benchmark](datagrip-official-baseline.md) and
[capability matrix](datagrip-feature-matrix.md) remain the comparison sources.
This document is the implementation and qualification queue for that target.
It is not a declaration of full parity or an agreed feature reduction.

The owner subsequently selected the **latest stable DataGrip release** and
reported query results of **up to 1,000,000 rows**. JetBrains' official
[release API](https://data.services.jetbrains.com/products/releases?code=DG&latest=true&type=release)
reported DataGrip **2026.3**, build `263.7307.216`, released 5 November 2026,
when checked on 9 November and rechecked on 10 and 11 November. Version-specific features, including bundled Air,
cross-database queries and the new grid column controls, belong in applicability
review. Their provider entitlements and database support remain separate facts.

All applicable DataGrip capabilities are required until the owner explicitly
excludes one. The twelve broad areas below organize work but do not prove an
exhaustive inventory. Database-specific facilities, authoring tools and normal
macOS usability/installation must be checked against current official docs and
actual usage. Features cannot be made optional merely because the current adapter
cannot support them. Other platforms are outside the confirmed macOS host target;
other existing engine adapters do not substitute for Snowflake.

The source-linked [Snowflake official benchmark](snowflake-official-baseline.md)
records connector alternatives and the expanded latest-release inventory.

## Current application boundary

The latest local increment (`b06f030` / `3039975`) retains original typed SDK
DDL/DML/transaction outcomes when local recording fails, retries missing local
stages without SQL replay and keeps dropped transaction callers owned through
recording. Pending receipts block new reviewed mutations, BEGIN/COMMIT and normal
shutdown; explicit rollback retains bounded recovery. This is in-process recovery,
not forced-exit durability while local storage is unwritable. Current validation
and source-scoped evidence are in the [ledger](replacement-verification.md).

Application profiles now select explicitly between the stateful SDK external-browser
route and the existing local-application OAuth/SQL API route. New profiles choose
external browser; missing mode on an existing profile defaults to its prior OAuth
route. No automatic fallback occurs. Ordinary SDK execution stays read-only;
strict single INSERT/UPDATE/DELETE uses a separate reviewed opt-in and typed path.
Typed transaction controls use their own boundary policy. The SDK route currently
also supports separately opted-in generated CREATE TABLE, nullable ADD COLUMN
and DROP TABLE through the [dedicated DDL workflow](snowflake-sdk-ddl-design.md).
This subset requires review in every writable Safe Mode, exact-target DROP confirmation,
idle-session admission and explicit coherent metadata reconciliation afterward.
It does not establish complete Snowflake DDL or account mutation effects.
The SDK route currently
advertises SQL and cancellation. Subsequent same-session completion is implemented
with [separate qualification](snowflake-sdk-metadata-design.md). The later
[browsing slice](snowflake-sdk-browsing-design.md) implements a revision-bound
relation catalog, column inspection and bounded read-only preview locally.
The [session observer](snowflake-sdk-session-observation.md) now implements
bounded settings/activity observations and explicit Unknown states locally;
the corrected helper-only 24-read account observation passes on its recorded
source. Normal app and changed-source qualification remain open.
The [typed switch transport](snowflake-sdk-session-design.md) now has separate
USE/readback acknowledgement and retirement handling with local regression
coverage. Its guarded application adapter, typed IPC and schema selector are now
locally qualified, including browser and hidden native checks. The selector requires a
fresh observed idle session and an exact revision-bound target; refresh is
explicit when intervening work revokes that authority. No account switching or
additional account authorization is established.
Complete object/constraint inspection, generic editable browsing, schema/session
switching, broader writes, DDL and complete transaction workflows remain incomplete.
The [reviewed DML workflow](snowflake-sdk-dml-design.md) and explicit boundaries
are locally implemented; none qualifies actual account mutation effects. The owner
requested the [disposable setup proposal](snowflake-disposable-setup.md). Its
approved single account attempt passed SSO and initial observations, then stopped
on definitive CREATE DATABASE rejection without creating a schema or table.
That approval is consumed. The owner now permits new tables under
`SANDBOX.BILLIE_TESTING`; the [existing-schema lifecycle proposal](snowflake-existing-schema-qualification.md)
now has a tested harness-only offline definition. It still needs physical SQL and
inbound resource gates, raw census/creation-marker interception, a source/runtime-
bound executable plan, a fixed runner, review and fresh possible-billing approval.
The preceding
SDK integration qualification passes, including all 143 browser workflows,
124 actual local database fixtures and the packaged SDK runtime checks. These
checks do not qualify the actual account workflow. See
[integration evidence](snowflake-sdk-integration.md).

Local tests cover persisted modes, backup/restore, immutable runtime/source pins,
retained child cleanup and pending-auth cancellation/shutdown. The native opt-in
resource route passes package/startup verification and 19 hidden native checks
with ten cleanup cycles. Historical SQL API
browser-SSO canaries do not establish the required SDK account behavior.

## Required workflow inventory

| Area | Observable replacement acceptance | Current state |
|---|---|---|
| Connections and authentication | Existing account externalbrowser SSO; correct original account/host identity, TLS validation, browser cancellation/timeouts, expired login, explicit reconnect and logout; no credential leakage or automatic SQL replay | SDK profile route implemented; local lifecycle checks pass; actual account and native package acceptance open |
| Session context | Observe current user/role/warehouse/database/schema; switch each deliberately, update metadata/completion, retain temporary/session objects and variables; isolate consoles when requested | SDK same-connection observations locally qualified with explicit Unknown and nullable role. Guarded schema selection locally qualified; account switching, database/role/warehouse switching, temporary/session objects and account continuity remain unqualified or incomplete |
| Metadata and search | Browse/find and inspect every required Snowflake object type with DDL/keys/dependencies, bounded paging, denied-role behavior and refresh after external changes | Partial catalog pilot; exact object inventory and live qualification open |
| SQL editing | Snowflake-aware completion, quoted identifiers and semi-structured expressions, parameters, diagnostics, formatting, navigation/refactoring and all applicable DataGrip authoring tools | Bounded common editor; full Snowflake feature inventory/qualification open |
| Execution | Statement/selection/batch execution, typed binding and session commands; cancellation, timeouts, interrupted network, no hidden reconnect/retry of writes | Read execution plus strict reviewed SQL INSERT/UPDATE/DELETE locally implemented; broader write/batch/session semantics and remote outcomes remain open |
| Results and data movement | Complete results up to 1,000,000 rows; exact Snowflake scalar/semi-structured values, inspection, filtering/sorting/paging, copying, supported import/export and format/destination behavior on representative data | Shared store passes actual PostgreSQL million-row paging/export and browser acceptance with opt-in bounded temporary storage. The synthetic-SDK Snowflake helper/driver/store/full-CSV bridge passes one million rows and an independent export hash. Actual SDK 3.4.1 loopback qualification separately passes one million rows across 100 compressed chunks plus cancellation. Native preference/restart checks pass. Actual account retention/types/performance and complete packaged-app memory remain unqualified |
| Data editing | Review and commit/rollback row changes with conflict/identity safeguards and truthful affected-row counts; support required keyless workflows safely or record a blocking gap | SQL-name reviewed DML with exact status strings is locally implemented. Safe grid editing, conflict/immutable identity safeguards and account effects remain incomplete; no accepted narrowing |
| Transactions | Auto/manual modes, shared vs isolated consoles, explicit BEGIN/COMMIT/ROLLBACK, DDL implicit commits, autocommit changes and acknowledgement loss handled truthfully | Explicit typed controls and recovery locally qualified on `929ae06`. Reviewed DML and the separate three-action generated DDL route are now locally implemented; DDL requires idle AUTOCOMMIT=true state and is not rollbackable. SDK AUTOCOMMIT changes and broader DDL remain unavailable. Actual account effects and complete transaction workflow remain primary blockers |
| History and scripts | Reuse scripts, query history and session-independent drafts with chosen privacy/persistence, no automatic reconnect/execute after restart | Common library exists; Snowflake lifecycle/native acceptance open |
| Performance | Appropriate Snowflake estimated/actual plans, query history/profile/timing and required monitoring/cancellation workflow | Synthetic estimated logical plans; live features and full inventory open |
| Schema and engine behavior | Required Snowflake DDL/object operations, schema compare/migration and session/role behavior, with explicit implicit-commit warnings and stale-target checks | Reviewed SQL DML and generated CREATE TABLE/nullable ADD COLUMN/exact-target DROP are locally integrated with typed native routes, revision-bound admission and explicit metadata reconciliation. The separate fixed disposable setup runner confers no application DDL authority. Actual account effects, complete metadata, broader object operations and engine semantics remain incomplete |
| Everyday macOS use | Full applicable keyboard/accessibility workflows, native installation/update/startup/quit/relaunch, secure credential/session lifecycle and local persistence | macOS bundle/build smoke exists; normal packaged and owner acceptance open |
| Other DataGrip facilities | Identify remaining applicable features in current official documentation, including integrations/AI where provided by the non-commercial context; explicitly track provider/entitlement limits | Required applicability inventory open; not silently optional |

## Implementation sequence and safety gates

1. Finish and locally commit the already reviewed cross-engine safety increment;
   its database/UI/native evidence does not establish Snowflake readiness.
2. Compare current official supported connector routes for externalbrowser and
   persistent sessions. Prefer an appropriately licensed official SDK/connector
   over implementing undocumented authentication endpoints. Record runtime,
   redistribution, dependency, session and packaging implications before choosing.
3. Establish one serialized transport connection per logical console, explicit
   ownership/cancellation/close and typed bounded protocol if a helper is used.
   Metadata and health operations must not commit/reset user transactions.
4. Implement verified session context, read/bind/cancel and lifecycle first, keeping
   unsupported capabilities disabled until their full workflow is implemented.
   Synthetic protocol tests are useful but never count as real Snowflake proof.
5. Add reviewed DML/DDL and explicit transactions with truthful known/unknown
   outcomes, no automatic retry after uncertain submission, DDL/autocommit
   semantics and safely drained shutdown. Review substantial increments.
6. Complete required metadata, editor, grid/data movement, schema/performance and
   usability workflows against the official inventory. Keep every gap visible.
7. Obtain a concrete non-production account/role/warehouse/object scope and approval
   for billable work and writes. Execute the actual acceptance suite, inspect
   committed state independently, and retain evidence without credentials.
8. Build and exercise the normal macOS package, restart/credential behavior and the
   owner's actual daily walkthrough. Keep the Goal incomplete until explicit
   hands-on acceptance resolves all required capabilities.

### Selected transport candidate and current work

Use the official Apache-2.0 `snowflake-sdk` **3.4.1** with a pinned bundled Node
runtime, conditional on qualification. Its documented `EXTERNALBROWSER` route
matches the requested authenticator, and one connection can own an interactive
session. The current SQL API requires transaction statements inside a single
multi-statement request, so it cannot preserve the required open transaction
between separate console actions. See [Node authentication](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-authenticate),
[pinned package and license](https://github.com/snowflakedb/snowflake-connector-nodejs/tree/v3.4.1),
and [SQL API limits](https://docs.snowflake.com/en/developer-guide/sql-api/intro).

The initial SDK integration checkpoint contained a versioned bounded child protocol and a
Rust supervisor (31 helper tests, 8 supervisor unit and 30 integration tests),
plus a read adapter with 11 integration tests. Offline macOS arm64 runtime
staging and actual SDK startup pass. Explicit profile routing and retained cleanup
are now integrated, with focused actual-child and offline UI/persistence tests.
Combined validation passes 604 frontend / 297 standard Rust tests plus 124 actual
fixtures. The normal SDK macOS bundle passes resource/signature and actual SDK
hello/EOF checks; all 143 browser workflows and 19 hidden SDK native checks pass.
See the [integration evidence](snowflake-sdk-integration.md) and
[preparation guide](snowflake-runtime.md).
One child owns one console generation; cancellation
bypasses row backpressure, and process loss never causes automatic SQL replay.
Actual SDK loopback fault injection observes retransmission after a lost response
with the same request ID and body; a simulated server without deduplication applies
two effects. This does not prove actual Snowflake deduplication. The SDK's
public options do not provide a documented statement-retry disable switch.
A subsequent original [submission guard](snowflake-sdk-submit-guard-design.md)
now enforces one admitted native query submission per exact returned statement
ID for the reviewed transport sources. Its local qualification is separate from
the unguarded baseline and does not establish server effects or transaction
semantics. Writes, DDL and transaction controls were disabled at that checkpoint;
their current bounded local implementations are recorded above.
Preserve SDK query/request IDs, distinguish statement acknowledgement from commit,
and prove fail-after-commit behavior before enabling mutations. Browser-auth abort,
logout completion, context continuity and exact result types also require proof.
[Pinned statement implementation](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/statement.js),
[pinned retry configuration](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/connection_config.js).

### Required million-row acceptance

The shared [opt-in temporary result store](large-results.md) now implements
bounded memory/disk retention and passes an actual PostgreSQL million-row
paging/export fixture. The earlier qualification passed 124 combined fixtures and
138 browser workflows on its recorded source.
Native preference/restart qualification passes on `c860a78`; see the ledger. This does not
qualify Snowflake SDK buffering, transport or types.
The UI must distinguish retained, complete, truncated, failed and cancelled
results, and must not rerun SQL merely to page or export a snapshot.

Reproducible acceptance must cover one million synthetic rows on an actual local
database, then the authorized Snowflake account: exact count; first, random and
last-page access; duplicate column names; NULL versus empty strings; integers and
decimals outside JavaScript's exact range; timestamps and semi-structured values;
complete export with an independently verified row count/content; bounded memory
and backpressure; cancellation and connection loss during fetch; storage exhaustion;
and private-cache cleanup on release, quit and restart. Include a dataset larger
than the current 64 MiB memory budget and representative column widths. Synthetic
helper tests qualify protocol throughput only, never Snowflake correctness.
Record elapsed time, peak process memory and temporary disk usage; practical
latency thresholds and CSV import sizes remain to be established.

## External qualification boundary

The owner authorized reuse of the existing Snowflake connection. Only allowlisted
non-secret settings from the locally configured default profile were inspected;
they are retained in an ignored, owner-only local cache. No private DataGrip
settings or OS credential state were inspected. The owner subsequently approved
the [two-query read canary](snowflake-read-canary.md), including possible billing.
The initial run authenticated and submitted both fixed reads, then failed local
result validation; child exit and local authentication-state removal succeeded.
The validator incorrectly required a numeric session ID even though Snowflake
documents `CURRENT_SESSION()` as VARCHAR. That reproduced local defect was
corrected in `9e39e51`; it does not establish the sole cause of the first failure.
The same-scope rerun passed both result contracts, context/session continuity,
logout acknowledgement and local cleanup. See the verification ledger.
The confirmed workflow and profile reuse are not blanket
authorization for additional paid queries or external writes. A canary must name its account,
role, warehouse policy and disposable objects, bound its work and specify cleanup.
Authentication occurs through the app/browser; secrets must never be pasted into
source, logs or the repository. Do all local implementation and review first so
any authorization request concerns a concrete, reviewable test plan.

Account/role/warehouse/database/schema settings have been located locally. Server
version/environment, warehouse policy, representative sanitized
workloads/data sizes and performance/accessibility expectations remain to be
provided. They refine the all-applicable-DataGrip target rather than reduce it.
