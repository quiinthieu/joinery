# Official DataGrip capability baseline

Research date: 2026-11-09. Product documentation observed in the current
DataGrip 2026.3 and older 2026.2 help versions (called out by area below), with the
2026.3 release notes and official product pages where version-specific context
matters. This is a user-facing capability
benchmark for the adjacent Joinery comparison; actual availability varies by
database, driver, permissions, and IDE configuration. It is not a claim that
Joinery already implements every DataGrip feature. The owner's confirmed target
requires every applicable Snowflake/macOS capability and results up to one million
rows. The source-linked [Snowflake benchmark](snowflake-official-baseline.md)
records that inventory and connector constraints. JetBrains' official
[release API](https://data.services.jetbrains.com/products/releases?code=DG&latest=true&type=release)
confirms **2026.3**, build `263.7307.216`, released 5 November 2026, as the latest
stable release checked on 9 November and rechecked on 10 November. The latter
check refreshes release identity only; the source research below retains its
original observation date.

## Licensing and AI are separate questions

JetBrains says DataGrip's non-commercial license includes the same IDE feature
set as its commercial version; there is no Community edition split by core
functionality. The FAQ names AI-powered completion, query console, data editor,
Git integration, and multiple databases among included features. The license
is limited to qualifying non-commercial activity, is activated through a
JetBrains Account, lasts one year, and renews automatically if used in the last
six months. [Non-commercial announcement and FAQ](https://blog.jetbrains.com/datagrip/2025/10/01/datagrip-is-now-free-for-non-commercial-use/),
[license registration](https://www.jetbrains.com/help/datagrip/register.html),
[DataGrip FAQ](https://www.jetbrains.com/help/datagrip/faq-about-datagrip.html).

That license statement does not mean every AI model or service is unlimited or
included. AI Assistant and agent integrations are a distinct layer: the current
docs describe JetBrains AI subscription, third-party provider accounts or API
keys, local models, ACP-compatible agents, and MCP tools as setup choices. The
2026.3 release page says Air is bundled and includes free AI with its Junie Lite
configuration. Treat model/provider access, quotas, and paid subscriptions as
separate from DataGrip's non-commercial IDE license. [AI Assistant docs](https://www.jetbrains.com/help/datagrip/ai-assistant-in-jetbrains-ides.html),
[2026.3 What's New](https://www.jetbrains.com/datagrip/whatsnew/).

## Capability baseline by user area

| User area | Officially documented capability |
|---|---|
| Connections and engines | Manage data sources, drivers, authentication/connection settings, and which databases or schemas are introspected. Secure connections support SSL and SSH tunnels. The FAQ says generic JDBC drivers can connect and run queries with generic SQL support, while dialect-specific highlighting, completion, and richer introspection depend on dedicated support. [Data sources](https://www.jetbrains.com/help/datagrip/managing-data-sources.html), [SSH/SSL](https://www.jetbrains.com/help/datagrip/configuring-ssh-and-ssl.html), [FAQ](https://www.jetbrains.com/help/datagrip/faq-about-datagrip.html) |
| Object navigation and metadata | Database Explorer organizes introspected objects. Search covers database objects, data, code usages, files, settings, and IDE actions. [Database Explorer](https://www.jetbrains.com/help/datagrip/database-explorer.html), [Search](https://www.jetbrains.com/help/datagrip/search-in-ide.html) |
| SQL authoring | SQL assistance includes schema-aware completion, inspections/error highlighting, quick-fixes, configurable formatting, refactoring, and live templates. The active data source/schema or search path informs assistance; support depth depends on dialect/driver. [Completion](https://www.jetbrains.com/help/datagrip/auto-completing-code.html), [inspections](https://www.jetbrains.com/help/datagrip/code-inspection.html), [formatting](https://www.jetbrains.com/help/datagrip/configure-the-sql-code-style.html), [refactoring](https://www.jetbrains.com/help/datagrip/refactoring-source-code.html), [live templates](https://www.jetbrains.com/help/datagrip/using-live-templates.html) |
| Query execution and sessions | Query consoles run scripts and display results; execution settings cover statement/script behavior, output placement, and parameters. Sessions can be managed independently and associated with consoles. [Run queries](https://www.jetbrains.com/help/datagrip/run-a-query.html), [sessions](https://www.jetbrains.com/help/datagrip/managing-connection-sessions.html), [parameters](https://www.jetbrains.com/help/datagrip/settings-tools-database-user-parameters.html) |
| Transactions and cancellation | Auto and Manual transaction modes are configurable per data source/console; Manual mode exposes commit and rollback controls, and the editor can preview pending DML changes. Cancel Running Statements requests cancellation; deactivating a data source stops work in the IDE while the database query may continue, and repeated cancellation offers remote-process termination. 2026.3 release notes describe improved transaction-state tracking for PostgreSQL, Greenplum, MySQL, MariaDB, and Oracle. [Transaction controls](https://www.jetbrains.com/help/datagrip/submitting-and-reverting-changes.html), [connection defaults](https://www.jetbrains.com/help/datagrip/configuring-database-connections.html), [query cancellation](https://www.jetbrains.com/help/datagrip/run-a-query.html), [2026.3 release notes](https://www.jetbrains.com/help/datagrip/release-notes-datagrip.html) |
| Results and table data | The data editor works with database object data, result sets, and user files. It supports table/tree/text/transpose views, paging, sort and filters, cell/row add/edit/delete where writable, value editing, related-record navigation, and data comparison. Server-side sorting/filtering behavior differs by source/result type. [Data editor](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html), [query results](https://www.jetbrains.com/help/datagrip/viewing-query-results.html) |
| Data movement | Import/export supports database objects and data; export can include query results and multiple formats. [Import](https://www.jetbrains.com/help/datagrip/import-data.html), [export](https://www.jetbrains.com/help/datagrip/export-data.html) |
| Schema and object changes | Users can edit database object source/DDL, create and modify objects, compare schemas, and review/migrate detected differences. Available operations remain database-specific. [Database objects](https://www.jetbrains.com/help/datagrip/database-objects.html), [schema comparison and migration](https://www.jetbrains.com/help/datagrip/schema-comparison-and-migration.html) |
| Diagrams and query plans | DataGrip generates diagrams for data sources, schemas, and tables. Query plans have tree/raw/graph presentation options; actual-plan support and explain modes depend on the database. [Diagrams](https://www.jetbrains.com/help/datagrip/creating-diagrams.html), [query execution plans](https://www.jetbrains.com/help/datagrip/query-execution-plan.html) |
| Query/data search and comparison | Search includes object/code/data search, including full-text database search. The product can compare table data and query result sets. [Search](https://www.jetbrains.com/help/datagrip/search-in-ide.html), [data editor and comparison](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html) |
| Saved work and recovery | SQL files/project work, saved queries, bookmarks, recent locations, Local History, and Git support workflows for finding and recovering edits. [User files](https://www.jetbrains.com/help/datagrip/working-with-user-files.html), [saved queries](https://www.jetbrains.com/help/datagrip/store-your-queries.html), [bookmarks](https://www.jetbrains.com/help/datagrip/bookmarks.html), [recent locations](https://www.jetbrains.com/help/datagrip/recent-files-and-changes.html), [Local History](https://www.jetbrains.com/help/datagrip/local-history.html) |
| Keyboard and accessibility | Predefined and customizable keymaps support shortcut lookup/editing. Accessibility settings include screen-reader support, UI zoom and colors, plus keyboard navigation; the 2026.2 accessibility guide documents Windows, macOS, and Linux screen-reader support. [Keymap (2026.2)](https://www.jetbrains.com/help/datagrip/settings-keymap.html), [Accessibility (2026.2)](https://www.jetbrains.com/help/datagrip/accessibility.html) |
| Installation, updates, and local persistence | Standalone installs and Toolbox-managed installs have update channels/policies, with automatic update checks enabled by default for standalone. Projects store data-source settings in `.idea/dataSources.xml`; personal IDE settings, caches/Local History, and projects occupy separate configuration, system, and projects directories. Backup and Sync can copy/sync selected settings through a JetBrains Account; projects/files remain local unless otherwise shared. [Update DataGrip (2026.3)](https://www.jetbrains.com/help/datagrip/update.html), [project data (2026.3)](https://www.jetbrains.com/help/datagrip/working-with-projects.html), [IDE directories (2026.3)](https://www.jetbrains.com/help/datagrip/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html), [settings sync (2026.3)](https://www.jetbrains.com/help/datagrip/sharing-your-ide-settings.html), [data sources](https://www.jetbrains.com/help/datagrip/managing-data-sources.html) |
| AI and agents | AI Assistant provides chat, in-editor assistance, and coding agents. It can use database objects as context; external agents can connect via ACP, and MCP can expose tools or the IDE. In 2026.3, Air is included by default with a free Junie Lite configuration. This is a provider-dependent capability layer in the required applicability inventory and does not change non-commercial licensing parity for core IDE functionality. [AI Assistant](https://www.jetbrains.com/help/datagrip/ai-assistant-in-jetbrains-ides.html), [2026.3 What's New](https://www.jetbrains.com/datagrip/whatsnew/) |

## Scope notes for comparisons

- Official feature pages describe the product generally; they do not guarantee
  identical operations across every DBMS or JDBC driver. State engine-specific
  limits where they affect a comparison.
- “All commercial features” describes the non-commercial IDE license entitlement.
  It should not be expanded into a claim that third-party AI services, model
  usage, provider quotas, or commercial-use rights are bundled.
- The feature matrix remains the detailed implementation/evidence inventory;
  this file records the external product baseline and licensing distinction.
