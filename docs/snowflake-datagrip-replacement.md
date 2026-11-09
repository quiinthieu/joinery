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
when checked on 9 November. Version-specific features, including bundled Air,
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

The existing Snowflake adapter uses hardcoded `LOCAL_APPLICATION` OAuth
code/PKCE, a loopback callback, token refresh and SQL API requests. README requires
`SNOWFLAKE$LOCAL_APPLICATION` to be enabled on the account. This differs from the
owner's confirmed externalbrowser connector path; a similar IdP browser screen
does not establish protocol, account setup or credential-lifecycle compatibility.

Profiles enforce ReadOnly. Execution rejects non-read SQL, row editing is disabled,
transactions are unsupported, and request-scoped context is stateless. The role,
warehouse and database are configured per profile; schema switching is bounded.
Most transport/browser qualification is synthetic. A historical live browser-SSO
canary does not establish the required externalbrowser, write or transaction path.

Source evidence: `src-tauri/src/drivers/snowflake_auth.rs`,
`src-tauri/src/drivers/snowflake.rs`, `src-tauri/tests/snowflake_profile.rs`,
Snowflake transport/lifecycle fixture tests and `tests/e2e/snowflake.spec.ts`.
The bounded Tier 2 audit used project generation `2026-11-09T12:48:17Z`, checked
coverage and read exact source. This is scoped evidence, not an exhaustive audit.

## Required workflow inventory

| Area | Observable replacement acceptance | Current state |
|---|---|---|
| Connections and authentication | Existing account externalbrowser SSO; correct original account/host identity, TLS validation, browser cancellation/timeouts, expired login, explicit reconnect and logout; no credential leakage or automatic SQL replay | Required externalbrowser path missing; local-application OAuth pilot only |
| Session context | Observe current user/role/warehouse/database/schema; switch each deliberately, update metadata/completion, retain temporary/session objects and variables; isolate consoles when requested | Stateless configured context; stateful session missing |
| Metadata and search | Browse/find and inspect every required Snowflake object type with DDL/keys/dependencies, bounded paging, denied-role behavior and refresh after external changes | Partial catalog pilot; exact object inventory and live qualification open |
| SQL editing | Snowflake-aware completion, quoted identifiers and semi-structured expressions, parameters, diagnostics, formatting, navigation/refactoring and all applicable DataGrip authoring tools | Bounded common editor; full Snowflake feature inventory/qualification open |
| Execution | Statement/selection/batch execution, typed binding and session commands; cancellation, timeouts, interrupted network, no hidden reconnect/retry of writes | Read-only SQL API exists; required stateful write path missing |
| Results and data movement | Complete results up to 1,000,000 rows; exact Snowflake scalar/semi-structured values, inspection, filtering/sorting/paging, copying, supported import/export and format/destination behavior on representative data | Shared store passes actual PostgreSQL million-row paging/export and browser acceptance with opt-in bounded temporary storage. Native preference/restart checks follow. Actual Snowflake retention/types/performance remain unqualified |
| Data editing | Review and commit/rollback row changes with conflict/identity safeguards and truthful affected-row counts; support required keyless workflows safely or record a blocking gap | Disabled; no accepted narrowing |
| Transactions | Auto/manual modes, shared vs isolated consoles, explicit BEGIN/COMMIT/ROLLBACK, DDL implicit commits, autocommit changes and acknowledgement loss handled truthfully | Unsupported; primary blocker |
| History and scripts | Reuse scripts, query history and session-independent drafts with chosen privacy/persistence, no automatic reconnect/execute after restart | Common library exists; Snowflake lifecycle/native acceptance open |
| Performance | Appropriate Snowflake estimated/actual plans, query history/profile/timing and required monitoring/cancellation workflow | Synthetic estimated logical plans; live features and full inventory open |
| Schema and engine behavior | Required Snowflake DDL/object operations, schema compare/migration and session/role behavior, with explicit implicit-commit warnings and stale-target checks | Write/DDL path disabled; metadata and engine semantics incomplete |
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

Separate local prototypes implement a versioned bounded child protocol and a
Rust supervisor. They are not yet integrated or packaged and do not enable an
application capability. One child owns one console generation; cancellation
bypasses row backpressure, and process loss never causes automatic SQL replay.
The SDK itself retries recoverable HTTP requests with a stable request ID; its
public options do not provide a documented statement-retry disable switch.
Preserve SDK query/request IDs, distinguish statement acknowledgement from commit,
and prove fail-after-commit behavior before enabling mutations. Browser-auth abort,
logout completion, context continuity and exact result types also require proof.
[Pinned statement implementation](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/statement.js),
[pinned retry configuration](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/connection_config.js).

### Required million-row acceptance

The shared [opt-in temporary result store](large-results.md) now implements
bounded memory/disk retention and passes an actual PostgreSQL million-row
paging/export fixture, all 124 combined fixtures and all 138 browser workflows.
Native preference/restart qualification follows in the ledger. This does not
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
settings, OS credential state, Snowflake account or production resources were
accessed. The confirmed workflow and profile reuse are not blanket
authorization for paid queries or external writes. A canary must name its account,
role, warehouse policy and disposable objects, bound its work and specify cleanup.
Authentication occurs through the app/browser; secrets must never be pasted into
source, logs or the repository. Do all local implementation and review first so
any authorization request concerns a concrete, reviewable test plan.

Account/version/environment, role/warehouse policy, representative sanitized
workloads/data sizes and performance/accessibility expectations remain to be
provided. They refine the all-applicable-DataGrip target rather than reduce it.
