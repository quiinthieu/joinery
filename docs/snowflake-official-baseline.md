# Snowflake and DataGrip official-source baseline

**Checked:** 2026-11-09. **Scope:** Official Snowflake and JetBrains public
documentation only. No Snowflake account, IdP, or private workspace was
accessed. This is a requirements and route assessment, not an implementation or
parity qualification.

## Release and evidence boundaries

Use DataGrip **2026.3** as the current stable baseline for this review. The
official [JetBrains release API](https://data.services.jetbrains.com/products/releases?code=DG&latest=true&type=release)
reports version 2026.3, release date November 5, 2026, build 263.7307.216,
including macOS Apple Silicon and Intel installers. The
[2026.3 release notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html)
are dated November 5, 2026. They document bundled JetBrains Air, cross-database
queries, the new Database Output tool window, result sets opened in editor
tabs, and data-editor pin/manage/resize column controls.

JetBrains says DataGrip has no Community edition because it does not split
functionality into free and paid product tiers. The non-commercial license is
an entitlement for qualifying use, not a reduced feature edition; it has its
own eligibility, online activation/renewal and telemetry terms. See the
[DataGrip FAQ](https://www.jetbrains.com/help/datagrip/faq-about-datagrip.html)
and [registration and licensing](https://www.jetbrains.com/help/datagrip/register.html).
AI service/subscription/provider access is separate from that IDE license.

## Connector route: recommended candidate, not yet qualified

The exact user requirement is Snowflake SAML federated SSO using
`externalbrowser` on macOS, plus a persistent session for reads, writes, DDL,
and explicit multi-statement transactions. The closest official route found is
Snowflake’s Node.js driver, not the stateless SQL API:

* Pin `snowflake-sdk` **3.4.1**. The tagged official
  [package manifest](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/package.json)
  declares Node `>=20`; its tagged
  [license](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/LICENSE.txt)
  is Apache-2.0. Snowflake’s [2026 driver release notes](https://docs.snowflake.com/en/release-notes/clients-drivers/nodejs-2026)
  date 3.4.1 to October 26, 2026, and report Node 24 support work in the v3
  line. This makes the pinned driver compatible on paper with the available
  Node 24 runtime; it does not replace packaging and runtime qualification.
* The [Node authentication guide](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-authenticate)
  documents `EXTERNALBROWSER` and a browser-launch callback. JetBrains’ stable
  [Snowflake connection guide](https://www.jetbrains.com/help/datagrip/snowflake.html)
  likewise directs users to enter `externalbrowser` for browser SSO. Snowflake
  describes this authenticator as browser-based SAML 2.0 IdP authentication
  ([federated SSO](https://docs.snowflake.com/en/user-guide/admin-security-fed-auth-use));
  do not conflate it with OAuth/OIDC login. OAuth local applications use a
  different callback/PKCE flow ([OAuth local applications](https://docs.snowflake.com/en/user-guide/oauth-local-applications)).
* The driver exposes connection objects and SQL execution on them, supporting
  a stateful connection candidate. Keep `clientSessionKeepAlive` and
  `serverSessionKeepAlive` at documented defaults unless an explicit need is
  established; the former is false by default, and ordinary client sessions
  typically time out after roughly 3–4 hours idle
  ([options](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-options)).
  The pinned source config also defaults `serverSessionKeepAlive` to false
  ([3.4.1 config](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/connection_config.js)).
* **Retry is a material safety gate.** The driver supports request IDs and
  explicitly documents resubmitting a request with the same ID after uncertain
  acknowledgement. That is an explicit recovery API; it is not permission to
  submit a new mutation after an unknown outcome
  ([execute guide](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute)).
  The 3.4.1 config/source has internal retry machinery and a high default
  statement retry limit; the public connection options document login retry
  controls but do not document a switch that disables statement retry. Before
  mutation is enabled, inspect and fault-test the exact pinned source path for
  a request accepted by Snowflake followed by a dropped response. Preserve
  query/request identifiers and report `OutcomeUnknown` until reconciled; do
  not automatically issue a fresh SQL request. Pinned source:
  [connection config](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/connection_config.js),
  [statement implementation](https://raw.githubusercontent.com/snowflakedb/snowflake-connector-nodejs/v3.4.1/lib/connection/statement.js).
* `statement.cancel()` requests cancellation, but its callback reports the
  cancellation request result; the guide directs the caller to query status
  separately. `connection.destroy()` is not a substitute for statement
  cancellation and does not establish the server’s final statement outcome.
  Test cancellation, disconnect, reconnect, active transaction continuity,
  and “commit succeeded but response was lost” against the pinned helper.
  See [execution/cancellation](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute)
  and [connection lifecycle](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-connect).
* Configure driver logging explicitly: Snowflake documents logger levels and
  output controls, and the default driver writes to a log file and console.
  Verify SQL, bound values, tokens, and IdP details do not leak into app logs
  ([driver logging](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-logs)).

**Why not SQL API for this requirement:** Snowflake’s SQL API requires
`AUTOCOMMIT=TRUE` per statement/request. `BEGIN`, `COMMIT`, `ROLLBACK`, context
changes (`USE`, `ALTER SESSION`) and temporary objects are only supported inside
a request that contains multiple statements. That cannot provide an interactive
open transaction across separate user actions/requests. The API also notes
cloud-services billing for some result fetches. It is therefore not a fit for
the required long-lived console/editor session and explicit multi-request
transactions without a different server-side design. Sources:
[SQL API overview and limits](https://docs.snowflake.com/en/developer-guide/sql-api/intro),
[multi-statement requests](https://docs.snowflake.com/en/developer-guide/sql-api/submitting-multiple-statements),
[transaction semantics](https://docs.snowflake.com/en/developer-guide/sql-api/using-transactions).

Python’s official connector also supports `externalbrowser` and stateful
connections, but adds a Python runtime/package to bundle
([Python connect](https://docs.snowflake.com/en/developer-guide/python-connector/python-connector-connect),
[connector repository/license](https://github.com/snowflakedb/snowflake-connector-python)).
Snowflake ODBC on macOS is another documented driver route, but requires an
ODBC manager/driver packaging and equivalent connection, retry, cancellation,
and transaction proof. Universal Core is documented as shared connector
technology; this review found no stable public Rust connector API to prefer
over Snowflake’s Node/Python/ODBC/JDBC drivers
([Universal Core](https://docs.snowflake.com/en/developer-guide/universal-core/universal-core)).
The implementation candidate is the official Node driver, conditional on
those gates.

## Snowflake-applicable DataGrip capability inventory

“Required” below means it belongs in the user’s full-applicable-Snowflake
acceptance inventory. It does not assert that Joinery already implements or
has qualified it. Snowflake support varies by data type, object kind, role,
warehouse, and server behavior; mark cases unsupported only after a specific
compatibility decision.

| Area | Official DataGrip baseline / acceptance implication |
|---|---|
| Connection and auth | Snowflake data source/JDBC, account/host, `externalbrowser`, database, role and warehouse. Test macOS browser SSO, IdP cancel/retry, role/database/warehouse selection, reconnect, and credential handling. [Snowflake setup](https://www.jetbrains.com/help/datagrip/snowflake.html) |
| Session model | Current session documentation describes dedicated sessions, one shared session, and two-session shared/introspection mode. Establish which session owns consoles, browse, editors and introspection. Avoid health/context calls that mutate session/transaction state. [Session management](https://www.jetbrains.com/help/datagrip/managing-connection-sessions.html) |
| SQL editor/dialect | Syntax highlighting, dialect-aware completion (tables, columns, functions, joins), inspections, formatting, navigation/refactoring, code generation and object DDL. Snowflake-specific syntax needs current coverage. 2026.2 added support for Flow operations, database roles, HASH, UNION/INTERSECT BY NAME and ALTER USER parameters. [Completion](https://www.jetbrains.com/help/datagrip/auto-completing-code.html), [dialects](https://www.jetbrains.com/help/datagrip/settings-languages-sql-dialects.html), [2026.2 notes](https://www.jetbrains.com/datagrip/whatsnew/2026-2/) |
| Cross-database queries | Stable 2026.3 adds the in-memory Workspace data source, based on DuckDB, to query across different data sources/DBMSs without copying tables first. This remains applicable when Snowflake is one participant: assess Snowflake-to-other-source joins, permissions, types, cost, cancellation and result behavior. Do not silently scope this out because Snowflake is the primary connection. [2026.3 notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html) |
| Execute and cancel | Run selection/current statement, multi-statement SQL, parameters, result tabs, statement errors and cancel. DataGrip documents cancel-running-statements (Ctrl+F2); Snowflake cancellation is asynchronous/status-sensitive, so test terminal result and uncertain disconnect. [Data editor/query controls](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html), [Node cancel](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute) |
| Transactions | Auto/manual mode, submit versus submit-and-commit, explicit commit/rollback, pending changes preview and revert. User requires multi-statement explicit transactions. Test Snowflake implicit-commit DDL and same-session state. Stable 2026.3 says toolbar tracking covers PostgreSQL, Greenplum, MySQL, MariaDB and Oracle; its limitations say other DBMSs (example: SQL Server) do not update buttons when transaction statements run. Therefore transaction-state toolbar tracking is not documented for Snowflake, even though SQL transactions themselves are required. [Submit/revert](https://www.jetbrains.com/help/datagrip/submitting-and-reverting-changes.html), [2026.3 notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html) |
| Browse, grid and result editing | Table/view/query-result editing, add/delete/clone rows, cell editor, record view, paging, sort/filter, NULL/default/LOB editing, related rows, preview DML, submit/revert, auto/manual transaction. Stable 2026.3 adds pinned columns, Manage Columns (search/show/hide/reorder/pin), fit-to-window sizing, opening results in editor tabs and opening the originating SQL. Acceptance must include **1,000,000-row** datasets: validate paging/render/resource behavior at that scale without claiming the docs guarantee a million-row client-side load. Qualify Snowflake standard-table identity separately: its PK/UNIQUE/FK constraints are informational and not enforced; hybrid constraints are enforced, but that does not prove safe editable identity for all objects. [Grid](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html), [2026.3 notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html), [Snowflake constraints](https://docs.snowflake.com/en/sql-reference/constraints), [hybrid tables](https://docs.snowflake.com/en/user-guide/tables-hybrid) |
| Semi-structured data | Include VARIANT/OBJECT/ARRAY values, nested inspection/editing, large values, casts and round trips in hands-on acceptance. Public DataGrip grid docs establish generic value editing/LOB support but do not, by themselves, prove equivalent Snowflake nested-value editing. Do not claim type parity without direct qualification. [Grid](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html), [Snowflake SQL types](https://docs.snowflake.com/en/sql-reference-data-types) |
| History, logs, results | Query history/search/reopen, statement logs, retained result browsing, export and privacy/retention controls. Stable 2026.2 notes say AI-agent queries are included in history and prior AI results can be retrieved without rerunning. Stable 2026.3 moves results to Database Output and adds opening the result and originating SQL in editor context. [History](https://www.jetbrains.com/help/datagrip/find-recent-queries-and-files.html), [2026.2 notes](https://www.jetbrains.com/datagrip/whatsnew/2026-2/), [2026.3 notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html) |
| Plans/profile | Explain plan and raw/visual plan inspection are documented generally. Do not claim full Snowflake Query Profile parity from this: qualify supported explain/profile paths and metrics. [Execution plan](https://www.jetbrains.com/help/datagrip/query-execution-plan.html) |
| Schema and DDL | Object explorer/introspection, create/alter/view DDL, refresh, dependency/navigation, schema compare/diff, editable migration script and execution. Include role-dependent visibility and Snowflake object kinds. [Explorer](https://www.jetbrains.com/help/datagrip/database-explorer.html), [schema compare/migrate](https://www.jetbrains.com/help/datagrip/schema-comparison-and-migration.html) |
| Import/export and compare | File import preview/type mapping and target table creation/append; query/table export, CSV/TSV and other extractors; data comparison. Exercise Snowflake types, large files, failures, duplicate keys and transaction scope. [Import](https://www.jetbrains.com/help/datagrip/import-data.html), [export](https://www.jetbrains.com/help/datagrip/export-data.html), [compare](https://www.jetbrains.com/help/datagrip/compare-data.html) |
| Diagrams/navigation | Schema/table diagrams, object definition, references and related-row navigation. Verify Snowflake metadata and permissions; documentation of a generic feature is not proof every Snowflake object is represented. [FAQ/features](https://www.jetbrains.com/help/datagrip/faq-about-datagrip.html) |
| Debugging | Stored-code debugger is documented for Oracle, not Snowflake. Mark database debugging inapplicable unless a Snowflake-specific supported capability is independently documented. [Debugging](https://www.jetbrains.com/help/datagrip/debugging-code.html) |
| Keyboard/accessibility | Keymap/custom shortcuts, keyboard navigation, screen reader support (including macOS VoiceOver), zoom/contrast/color support. Require macOS keyboard-only and VoiceOver checks on connection, SQL editor, results, grid edits and dialogs; generic accessibility claims are not interaction qualification. [Accessibility](https://www.jetbrains.com/help/datagrip/accessibility.html), [shortcuts](https://www.jetbrains.com/help/datagrip/mastering-keyboard-shortcuts.html) |
| Install/update/local persistence | Native macOS install, bundled JetBrains Runtime, patch/full update behavior, local settings/keymaps/data-source persistence, logs and recovery. Verify durable app storage and protect secrets; DataGrip notes URL-only credentials may be plain text in URL/settings/logs. [Install](https://www.jetbrains.com/help/datagrip/installation-guide.html), [update](https://www.jetbrains.com/help/datagrip/update.html), [Snowflake settings](https://www.jetbrains.com/help/datagrip/snowflake.html) |
| AI | Required in the feature inventory: database-aware agent skills/MCP, schema and data context, SQL generation/run/feedback, query/result history, permission/consent controls and model/provider choice. Stable 2026.2 has AI database skills/MCP; stable 2026.3 bundles Air with database-aware chat/agent SQL execution, data browsing, object references, result display and DataGrip MCP retrieval. Qualify Snowflake tool safety and data sent to each provider. [AI Assistant](https://www.jetbrains.com/help/datagrip/ai-assistant-in-jetbrains-ides.html), [2026.2 skills](https://www.jetbrains.com/datagrip/whatsnew/2026-2/), [2026.3 notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html) |

This inventory is intentionally broad: a requirement cannot be made optional
merely because it is difficult to qualify. A feature can be declared
unsupported for Snowflake only with a specific product decision and evidence.

## Product entitlement and cost are separate decisions

DataGrip’s non-commercial license is not a reduced-functionality tier, but
eligibility and the license’s account/telemetry conditions still matter. It
does not imply included AI usage. JetBrains AI docs say most JetBrains AI
service access requires a separate AI subscription; other documented paths
include own provider keys, local models, or provider-account authorization.
Provider availability, subscription tier, quota, territory, or direct model
charges can change and require a user/product decision; do not present an
unverified price as a capability gap. See [JetBrains AI plans](https://www.jetbrains.com/help/ai-assistant/licensing-and-subscriptions.html)
and [DataGrip AI](https://www.jetbrains.com/help/datagrip/ai-assistant-in-jetbrains-ides.html).

Snowflake compute/provider costs are also account-specific. Warehouse size and
runtime drive compute use; resume has a minimum billing interval followed by
per-second billing. SQL API has additional cloud-services considerations for
some result fetches. Obtain the account’s warehouse policy and budget before
live acceptance; no account pricing was inspected. [Compute cost guide](https://docs.snowflake.com/en/user-guide/cost-understanding-compute),
[SQL API](https://docs.snowflake.com/en/developer-guide/sql-api/intro).

## Still required before calling this qualified

1. Package and fault-test the pinned Node helper: externalbrowser launch and
   cancellation, connection/session generation, transactions spanning
   separate calls, DDL implicit commits, row streaming/types, query IDs,
   cancellation status, SDK retry behavior after accept/drop, logging hygiene,
   shutdown and reconnect. Documentation support alone is not runtime proof.
2. Run hands-on macOS acceptance against the user’s actual Snowflake SAML
   externalbrowser IdP and authorized disposable account; then execute the
   full inventory above, including role/warehouse changes, DDL, multi-request
   explicit transactions, imports/exports, nested types, plans/history, AI and
   accessibility. No real connection or SSO was performed in this research.
3. Keep row-level grid/CSV editing gated until table identity semantics are
   separately proven for each Snowflake table kind. Standard Snowflake
   constraints cannot be assumed to enforce a unique row locator.
4. Obtain product-owner decisions for AI providers/data disclosure, license
   and usage cost, warehouse budget, supported DataGrip version baseline, and
   the behavior for any feature that cannot be safely matched. These are
   workflow/acceptance decisions, not settled by the connector docs.
