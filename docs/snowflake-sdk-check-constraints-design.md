# Snowflake CHECK-constraint inspection

## Scope and status

A separate retained-target **View CHECK constraints** action is implemented
locally. Focused/full validation, changed browser checks and independent review
pass, as do hidden native startup/IPC and cleanup; new normal package evidence
is pending. See the
[ledger](replacement-verification.md) for actual completed commands. This read-only inventory neither enables editing nor closes
the full metadata or DataGrip replacement gate. No account activity is included.

## Closed read and retained authority

`inspect_relation_check_constraints` accepts only `sessionId` and a request with
`revision` and `targetId`; unknown request fields are rejected. Rust resolves the
opaque target in the retained catalog, preserves current target admission and
uses the existing serialized relation lane. The driver checks retained authority,
full session context and exact relation before and after the read, then checks
retained authority again before publication. Disconnection, catalog refresh,
context drift or an active metadata/transaction hold invalidates stale results.
These observations do not promise atomic object identity against concurrent DDL.

One closed query reads the retained database's fully quoted
`INFORMATION_SCHEMA.CHECK_CONSTRAINTS`; exact catalog, schema and table values
are bound parameters. It selects `CONSTRAINT_CATALOG`, `CONSTRAINT_SCHEMA`,
`CONSTRAINT_TABLE`, `CONSTRAINT_NAME` and `CHECK_CLAUSE`, ordered by full
constraint identity, with `LIMIT 1001`. There is no caller SQL, name-only join,
context switch, schema-wide fallback or qualification-ledger expansion.

At most 1,000 rows and 1 MiB are accepted. The sentinel row rejects overflow.
Require five complete text columns, matching row counts, exact target identity,
valid identifiers and unique catalog/schema/table/name tuples. Reject NULL,
truncation, malformed rows and empty/whitespace-only or NUL-containing clauses.
Bound the final serialized DTO too, including JSON escaping overhead. Preserve
the clause exactly and render it as inert read-only text.

The response contains revision, targetId, database, table and constraints with
catalog, schema, tableName, name and checkClause. It reports no invented
enforced/validated flag or engine-eligibility claim. Empty role-visible results
say **No visible CHECK constraints returned**; a denied request remains an error.
Failure preserves independent columns, keys, mappings, definition and preview.
Selection, ownership/context changes, refresh, reference navigation and transient
holds clear prior CHECK output and prevent late publication.

## Official contract and remaining acceptance

Refreshed 12 November 2026 against the official
[CHECK_CONSTRAINTS view](https://docs.snowflake.com/en/sql-reference/info-schema/check_constraints).
Its five fields are VARCHAR and visibility follows the current role. The view
provides neither enforcement nor existing-row validation status. The current
[constraint overview](https://docs.snowflake.com/en/sql-reference/constraints-overview)
documents CHECK constraints and distinguishes validation of existing rows from
enforcement of future writes. Inventory text supplies no editing authority.

Actual account output compatibility, privileges, propagation, interruptions and
normal packaged keyboard/accessibility usage remain unqualified. CHECK creation,
modification and validation are separate required schema workflows. Existing
frozen table-lifecycle qualification plans do not authorize this new read. Any
live evaluation needs its own reviewed scope and possible-billing approval.
The [prepared walkthrough](migration-from-datagrip.md) and
[acceptance gates](replacement-acceptance.md) retain these requirements.
