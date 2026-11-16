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
when checked on 9 November and rechecked on 10, 11, 12 and 13 November. Version-specific features, including bundled Air,
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

Schema-8 source (`682b1ed..HEAD`) now implements the complete local Snowflake
DataGrip-replacement workflow surface across the bundled Apache-2.0
`snowflake-sdk` `3.4.1` and pinned Node `24.22.2` helper:

- **Lossless editable grid, DataGrip 2026.3 column/view controls, and multi-batch CSV import**
  (`682b1ed`, `a794426`, `32b2d15`, `44fcbac`): Inline cell editing, whole-original row
  deletion, paginated new-row insertion & row `Clone`, `<ReviewDialog>` cancel
  recovery (`cancel_snowflake_grid_plan`), server-side `PreviewSlice` pagination
  (`OFFSET`), single-column `ORDER BY ... ASC/DESC NULLS FIRST/LAST`, typed
  column filters, `Transpose` & `Record` grid views, persisted column pinning /
  freezing (`Show all` / `Hide optional` / `Fit to window`), detached `"result"`
  tabs, and multi-batch RFC-4180 CSV staging up to `100,000` rows in frame- and
  row-bounded batches (`<= 500` rows, `<= 2,000` scalar binds, `<= 768 KiB` unit
  SQL, and `< 1 MiB` helper frame per batch) across base/temporary tables up to
  `1,000` columns and all lossless Snowflake scalar/temporal/binary/semi-structured/spatial
  types (`TEXT`/`VARCHAR`, `BOOLEAN` with wire `"TRUE"`/`"FALSE"` normalization,
  `NUMBER(1..=38, 0..=38)`, IEEE-754 `FLOAT`/`DOUBLE`, `BINARY`, `DATE`,
  `TIME(0..9)`, `TIMESTAMP_NTZ/LTZ/TZ(0..9)`, `VARIANT`/`ARRAY`/`OBJECT`, and
  writable `GEOGRAPHY`/`GEOMETRY` WKT/EWKT/RFC-7946 GeoJSON codecs with
  `TO_GEOGRAPHY`/`TO_GEOMETRY` binds and `ST_ASEWKB` conflict predicates).
- **Guarded session context switching, observed `EXPLAIN USING JSON` plans, and post-execution query profiler**
  (`a05306a`, `8e1b5f1`): Same-connection inventory and guarded `USE ROLE`,
  `USE WAREHOUSE`, `USE DATABASE`, and `USE SCHEMA` switching with readback
  verification and automatic metadata/completion reconciliation, live
  `EXPLAIN USING JSON` operator-tree parsing (`QueryEvent::PlanContext`) and
  plan snapshot capture/comparison in `SqlEditor`, plus post-execution
  `INFORMATION_SCHEMA.QUERY_HISTORY_BY_SESSION` and `GET_QUERY_OPERATOR_STATS`
  operator DAG, partition-pruning, and disk-spill inspection in
  `SnowflakeQueryProfileModal` (honoring `store_query_text` SQL redaction).
- **Object Explorer, special-table metadata, virtual foreign keys, schema diagrams, schema diff/migration DDL, and AST SQL authoring**
  (`ff131ef`, `4eba148`, `8e1b5f1`, `a7a1e4e`): `ObjectExplorer` / `ObjectInspector`
  (`database_objects` and `object_details`) across standard and special tables
  (`DYNAMIC`, `EXTERNAL`, `HYBRID`, `EVENT`, `ICEBERG`, `TRANSIENT`, and
  `TEMPORARY`, including `SHOW DYNAMIC TABLES` / `SHOW EXTERNAL TABLES`
  attributes and Dynamic Table upstream lineage), views, materialized
  views, sequences, and overloaded user functions/procedures with FK constraint
  mappings, view dependencies, and `GET_DDL`; user-defined **Virtual Foreign
  Keys** (`persistence/library.rs`, max `256`) merged across ER diagrams (dashed
  `virtual` edges), live SQL `JOIN` completion, and outbound/inbound row
  navigation; bounded `SchemaDiagram` ER graphs (PK + composite physical/virtual
  FK layout with click-through to `ObjectInspectorView`); `SchemaDiffView`
  live/snapshot schema comparison and dialect-accurate Snowflake
  `generate_schema_ddl` / `generate_migration_ddl` (`CREATE`/`ALTER`/`DROP`
  across tables with `CLUSTER BY` and `IDENTITY`, views, sequences, and
  `$$`-delimited overloaded routines, plus explicit
  `DYNAMIC`/`EXTERNAL`/`HYBRID`/`EVENT` table disclosures and `Open in SQL editor`
  handoff); rich cell value inspection across `ValueDialog` and Snowflake
  `RelationBrowser` (`VARIANT`/`OBJECT`/`ARRAY` `:path` tree + lossless JSON
  formatter + staged cell apply, raw-hex `BINARY` Hex/ASCII dump, and
  `GEOGRAPHY`/`GEOMETRY` GeoJSON/EWKT/WKT 2D SVG plot + staged spatial cell
  apply); 8 reviewed Snowflake DDL
  actions (`CreateTable`, `AddColumn`, `AlterColumnNullability`, `RenameColumn`,
  `DropColumn`, `DropTable`, `CreateView`, `DropView`) gated on idle
  `AUTOCOMMIT=true` with post-execution metadata reconciliation; and Snowflake
  SQL dialect authoring with semi-structured `:path` completion, `$$` /
  single-quote literal context suppression, live `inspection_schema`
  unknown-column diagnostics, AST-aware workspace `Find Usages`
  (`sql_refactor.rs` across open tabs + saved queries), and `Extract CTE` /
  `Expand Wildcard (* / alias.*)` refactorings (`SqlRefactorDialog.tsx`).
- **Cross-source Workspace SQL, bounded full-text data search, SQL Local History, durable outcomes, retained console, and million-row results**
  (`8ec0477`, `d8bbba8`): Read-only sandboxed in-memory SQLite `WorkspaceSqlView`
  joining retained `ResultStore` snapshots across engines; bounded parameterized
  `search_database_data` across Snowflake SDK, PostgreSQL, and MySQL/MariaDB
  tables (`<= 50` tables, `<= 64` non-binary columns/table, `<= 200` hits / `2 MiB`,
  zero SQL interpolation); privacy-gated SQL Local History (`local_history.rs`)
  with Myers line diff and restore; schema-8 durable original capture/settlement,
  SQL-free batch archive, protected manual transaction rollback, configurable
  query limits up to 24 hours, encrypted metadata backup/restore, and
  disk-backed 1,000,000-row paging/export.

Live Snowflake account execution remains strictly unexecuted pending explicit
owner review and approval of the bounded
[`SANDBOX.BILLIE_TESTING` qualification plan](snowflake-sandbox-qualification-plan.md).
The prior schema-7 `d6cfb2a` app bundle is preserved separately while the
uninstalled schema-8 `Joinery.app` bundle is verified locally; Developer ID
signing/notarization, `/Applications` installation, and hands-on owner
acceptance remain open gates.

## Required workflow inventory

| Area                           | Observable replacement acceptance                                                                                                                                                                                        | Current state                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Connections and authentication | Existing account externalbrowser SSO; correct original account/host identity, TLS validation, browser cancellation/timeouts, expired login, explicit reconnect and logout; no credential leakage or automatic SQL replay | SDK profile route, retained helper lifecycle, and schema-8 macOS bundle locally verified; live account SSO and hands-on native acceptance gated on [sandbox plan](snowflake-sandbox-qualification-plan.md)                                                                                                                                                                                                                                                               |
| Session context                | Observe current user/role/warehouse/database/schema; switch each deliberately, update metadata/completion, retain temporary/session objects and variables; isolate consoles when requested                               | Same-connection observations plus guarded `ROLE`, `WAREHOUSE`, `DATABASE`, and `SCHEMA` switching with readback and metadata refresh locally verified (`a05306a`, `7434818`); live account continuity open                                                                                                                                                                                                                                                               |
| Metadata and search            | Browse/find and inspect every required Snowflake object type with DDL/keys/dependencies, bounded paging, denied-role behavior and refresh after external changes                                                         | `ObjectExplorer` / `ObjectInspector` plus bounded `SchemaDiagram` ER graphs (PK + composite outbound FKs -> `ObjectInspectorView`) locally verified across standard + `DYNAMIC`/`EXTERNAL`/`HYBRID`/`EVENT`/`ICEBERG`/`TRANSIENT`/`TEMPORARY` tables (with Dynamic Table upstream lineage), views, materialized views, sequences, and overloaded functions/procedures with PK/UQ/FK/CHECK inspection, view dependencies, and `GET_DDL`; live account catalog shapes open |
| SQL editing                    | Snowflake-aware completion, quoted identifiers and semi-structured expressions, parameters, diagnostics, formatting, navigation/refactoring and all applicable DataGrip authoring tools                                  | Snowflake dialect keywords/functions, table/alias/CTE + `VARIANT` `:path` completion, string/comment suppression, live `inspection_schema` column diagnostics, AST workspace `Find Usages` (`sql_refactor.rs`), `Extract CTE` & `Expand Wildcard (* / alias.*)` refactorings, alias/CTE rename, parameters, formatting, and snippets locally verified (`ff131ef`, `7434818`, `a7a1e4e`); daily owner acceptance open                                                     |
| Execution                      | Statement/selection/batch execution, typed binding and session commands; cancellation, timeouts, interrupted network, no hidden reconnect/retry of writes                                                                | Parameterized reads, reviewed DML/DDL, configurable 1–86,400s query limits, and retained mixed-SQL console locally verified; live account execution and interruption checks open                                                                                                                                                                                                                                                                                         |
| Results and data movement      | Complete results up to 1,000,000 rows; exact Snowflake scalar/semi-structured values, inspection, filtering/sorting/paging, copying, supported import/export and format/destination behavior on representative data      | Disk-backed 1,000,000-row store/export, SDK 3.4.1 1,000,000-row compressed chunk loopback, server-paged/sorted/filtered relation browsing, multi-batch RFC-4180 CSV import up to `100,000` rows, and rich `VARIANT` `:path` tree / raw-hex `BINARY` / `GEOGRAPHY` 2D SVG cell inspectors locally verified; live account million-row & RSS acceptance open                                                                                                                |
| Data editing                   | Review and commit/rollback row changes with conflict/identity safeguards and truthful affected-row counts; support required keyless workflows safely or record a blocking gap                                            | Reviewed `RelationBrowser` inline cell editing (including `VARIANT`/`OBJECT`/`ARRAY` JSON and `GEOGRAPHY`/`GEOMETRY` WKT/EWKT/GeoJSON spatial inspector edits), row delete/insert/clone, `<ReviewDialog>` cancel recovery, pending preview continuation, and Commit/Rollback locally verified across lossless scalar/temporal/binary/JSON/spatial codecs up to `1,000` columns; live account qualification open                                                          |
| Transactions                   | Auto/manual modes, shared vs isolated consoles, explicit BEGIN/COMMIT/ROLLBACK, DDL implicit commits, autocommit changes and acknowledgement loss handled truthfully                                                     | Explicit console transaction controls, owned manual/auto grid batch transactions, protected rollback on recording error, and idle `AUTOCOMMIT=true` DDL fencing locally verified; live account commit/rollback verification open                                                                                                                                                                                                                                         |
| History and scripts            | Reuse scripts, query history and session-independent drafts with chosen privacy/persistence, no automatic reconnect/execute after restart                                                                                | Schema-8 durable original outcomes, SQL-free batch archive, paged search, privacy controls, and encrypted metadata recovery locally verified; hands-on owner restart acceptance open                                                                                                                                                                                                                                                                                     |
| Performance                    | Appropriate Snowflake estimated/actual plans, query history/profile/timing and required monitoring/cancellation workflow                                                                                                 | Observed Snowflake `EXPLAIN USING JSON` `PlanContext` operator-tree parsing, partition/byte estimates, `SqlEditor` plan snapshot capture/diff, and post-execution `QUERY_HISTORY_BY_SESSION` + `GET_QUERY_OPERATOR_STATS` operator profiler (`SnowflakeQueryProfileModal`) locally verified (`a05306a`, `7434818`, `8e1b5f1`); live account plan/operator-stats shapes open                                                                                              |
| Schema and engine behavior     | Required Snowflake DDL/object operations, schema compare/migration and session/role behavior, with explicit implicit-commit warnings and stale-target checks                                                             | 8 reviewed Snowflake DDL actions plus `SchemaDiffView` live/snapshot schema comparison and dialect-accurate Snowflake `CREATE`/`ALTER`/`DROP` migration DDL generation (`CLUSTER BY`, `IDENTITY`, sequences, views, `$$`-delimited routines, unsupported-table disclosures, and `Open in SQL editor`) locally verified; live account DDL qualification open                                                                                                              |
| Everyday macOS use             | Full applicable keyboard/accessibility workflows, native installation/update/startup/quit/relaunch, secure credential/session lifecycle and local persistence                                                            | Playwright E2E workflows (`27/27` Snowflake specs), native WebView smoke/recovery, and schema-8 macOS `Joinery.app` bundle locally verified (`v0.1.0-preview.17` shipped/installed; `v0.1.0-preview.18` candidate); Developer ID/notarization and hands-on owner walkthrough open                                                                                                                                                                                        |
| Other DataGrip facilities      | Identify remaining applicable features in current official documentation, including integrations/AI where provided by the non-commercial context; explicitly track provider/entitlement limits                           | Tracked in [feature matrix](datagrip-feature-matrix.md); external AI/VCS entitlements remain separate from core Snowflake database workflows                                                                                                                                                                                                                                                                                                                             |

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
