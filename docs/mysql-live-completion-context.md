# MySQL and MariaDB live completion context

Feature E02-MM extends E02/I04. The initial slice merged as `dc220b0` through
PR #22 and shipped in preview.14. The private project hub remains canonical
shipping status. Broader multi-engine completion remains PARTIAL.

## Workflow and acceptance

The SQL editor shows its actual current database and effective account/active
roles, refreshes after app execution or explicit refresh, and requests columns
with the same revision-bound IPC and existing worker as PostgreSQL. `USE` changes
unqualified lookup; qualified names remain `database.table`, without a duplicated
three-part path. No automatic reconnect, SQL replay or role synchronization is
introduced. Safe Mode still authorizes every submitted session change.

Completion reads DATABASE(), CURRENT_USER(), CURRENT_ROLE() and the server's
lower_case_table_names on the actual editor connection, under the existing Rust
execution lane. The identity's optional caseSensitiveTableNames is set only by
the MySQL adapter; PostgreSQL retains its existing quoting/folding rules. A null
default database is an empty namespace with a visible No database label; qualified
accessible relations can still be suggested.

The bounded catalog comes from INFORMATION_SCHEMA on that same connection.
Server comparisons determine the current unqualified database set. Columns use
quoted SHOW FULL COLUMNS, filtering its effective privileges to SELECT and
returning read-only completion metadata. This follows session-local temporary
shadows of known permanent tables; querying INFORMATION_SCHEMA.COLUMNS instead
would describe the permanent table. No user table/view is SELECTed just to infer
columns. Legacy browser/editing metadata retains its separate connection.

Acceptance: switch to an alternate role-granted database, accept and execute a
column suggestion, withhold an UPDATE-only column, reject old revisions, remove
role-only names after SET ROLE NONE, resolve a temporary shadow, preserve the
actual database after USE inside a rolled-back transaction, and keep active-query
cancellation usable. Live case-sensitive lookup must decline wrong-case table
references while valid quoted names and database.table suggestions still work.

## Decisions, bounds and limitations

The initial fixture reproduced Unsupported in the shared adapter. A second regression
showed the editor SELECT DATABASE() returning the changed database while cached
prepared metadata still returned its preparation database. Live metadata now uses
text protocol; existing bound-parameter grid reads retain binary protocol. Both
share the existing row/byte collection checks. Text SQL is fixed except for names
validated and escaped with the existing identifier helpers.

Existing snapshot limits remain 10,000 relations / 2 MiB, 4,096 columns / 1 MiB,
eight frontend reads/cache entries and 100 suggestions. The identity read is
bounded at 64 KiB, metadata uses the adapter's 65-second operation deadline, and
active execution returns busy before reading metadata. Session errors are real
errors, never a login-configured fallback or automatic reconnect.

The original fixture qualifies MySQL 8.4 and MariaDB 11.4 with case mode 0.
The follow-up `mysql-folded` profile and Rust regression in PR #23 qualify mode 1
on both engines using separately initialized disposable servers. Mode 2 remains
unqualified on a suitable case-insensitive filesystem. The app currently
requires a nonempty database in saved SQL profiles, so a no-default-database
connection is not an exposed workflow; the empty-namespace code path remains
unverified.
New temporary-only names are not discoverable from the catalog; existing known
names resolve their actual temporary columns. Catalog visibility can include
objects with non-SELECT privileges; their columns are withheld unless SELECT is
available. External DDL/grant changes require refresh or subsequent app execution.
Snowflake live context, independent consoles and grouped-FK JOIN suggestions
remain separate slices. Arbitrary SQL results remain read-only.

## Sources and verification

- `drivers/mysql.rs`: completion_identity, fetch_text, collect_metadata.
- `drivers/mysql/catalog.rs`: live_completion_catalog/live_completion_schema.
- `domain.rs`: optional caseSensitiveTableNames; postgres.rs sets null.
- SqlEditor, CompletionSession and tableLookup carry the existing context revision
  and server case policy; no new commands, global store or execution system.
- `tests/fixtures/mysql.sql`: role-only database, same-name sample, case-preserved
  table and SELECT-versus-UPDATE column grants.
- mysql.rs fixture test covers the live transitions on both engines; existing
  catalog/paging and cancellation tests cover legacy metadata and busy guards.
- The mode-1 fixture test verifies folded database/table names, the server case
  policy, role-visible catalog, unqualified names, accepted uppercase SQL and
  old-revision rejection on MySQL 8.4 and MariaDB 11.4.
- sql-context.test.ts checks strict/folded lookup and two-part names.
- completion.spec.ts accepts and executes suggestions after USE on both engines,
  then verifies column grants and removal of old role metadata.

Source verification used graph generation `2026-10-29T02:42:01Z` with targeted
call paths/snippets and direct reads of the final changes. Coverage reported
partial fixture SQL parsing; those ranges were read directly and the role/grant
SQL executed on both disposable engines. Graph coverage is best-effort.

Final command outcomes and release eligibility are recorded in the feature matrix
and IMPLEMENTATION_STATUS.md. Production accounts and destructive runtime SQL are
outside this local fixture scope.

### Reproduce the case-mode-1 qualification

Use a fresh disposable project so the mode is set before database initialization:

```sh
docker compose -p joinery-mysql-folded --profile mysql-folded up -d --wait mysql-folded mariadb-folded
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test mysql mysql_live_completion_uses_server_case_policy -- --ignored --test-threads=1
docker compose -p joinery-mysql-folded --profile mysql-folded down -v --remove-orphans
```

The selected test passed on both engines. Follow-up `pnpm validate` passed
63 frontend and 83 standard Rust tests, formatting, types, build, Cargo check
and strict Clippy. The additional ignored test runs separately; this fixture-only
follow-up changes no application runtime code. Browser/native release evidence
belongs to preview.14 above, rather than a new run of this follow-up.

## Official references

- [MySQL information functions](https://dev.mysql.com/doc/refman/8.4/en/information-functions.html)
- [MySQL identifier case rules](https://dev.mysql.com/doc/refman/8.4/en/identifier-case-sensitivity.html)
- [MariaDB identifier case rules](https://mariadb.com/docs/server/reference/sql-structure/sql-language-structure/identifier-case-sensitivity)
- [MySQL SHOW COLUMNS](https://dev.mysql.com/doc/refman/8.4/en/show-columns.html)
- [MariaDB CURRENT_ROLE](https://mariadb.com/docs/server/reference/sql-functions/secondary-functions/information-functions/current_role)
- [MariaDB SHOW COLUMNS](https://mariadb.com/docs/server/reference/sql-statements/administrative-sql-statements/show/show-columns)
