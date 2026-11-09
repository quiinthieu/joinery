# Retained edits and replacement tables

Investigation date: 9 November 2026. **The strict PostgreSQL 18.6 fixture
diagnostic passes all nine cases; full application qualification remains open.
MySQL and MariaDB remain unsafe in the reproduced cases.** This is a safety qualification within the active
[replacement Goal](replacement-acceptance.md), not a claim about undocumented
DataGrip behavior. The passing [qualification baseline](replacement-verification.md)
does not cover this scenario.

## Reproduced behavior

The historical durable diagnostic at
[`src-tauri/examples/table_identity_probe.rs`](../src-tauri/examples/table_identity_probe.rs)
reproduced **27 failures: nine per engine** on PostgreSQL 18.6, MySQL 8.4.13
and MariaDB 11.4.13. Each operation reported `Completed` and changed the replacement.
All 27 approvals were consumed, fresh-read insert controls succeeded, and owned
table removal was independently verified. There were no operational errors.
This report belongs to unchanged application revision `ff59ca6`; it is historical
evidence, not the current PostgreSQL result. The current partial PostgreSQL repair
and its qualification limits are recorded below.

| Case (each engine) | Replacement timing | Replacement rows afterward (`id`, `note`) |
|---|---|---|
| Browse update | Before prepare | `(1, approved)` |
| Query-result update | Before prepare | `(1, approved)` |
| Query-result update | After approval | `(1, approved)` |
| Browse insert | After approval | `(1, before)`, `(2, before)` |
| Browse delete | After approval | Empty |
| Empty browse insert | After approval | `(2, before)` |
| CSV import | After preview | `(1, before)`, `(2, approved)` |
| CSV import | After batch approval | `(1, before)`, `(2, approved)` |
| CSV import second batch | After first 500-row batch committed | `(1, before)`, `(502, approved)` |

The final case independently confirms the first batch's 500 rows before replacing
the table. The original preview then supplies row 502 to the replacement. This
diagnostic adds no passing regression count: `--require-safe` exits **2**, with
`safetyPassed: false`. The application at that report's revision was `ff59ca6`.

### Initial PostgreSQL investigation

A development-only Rust fixture probe used actual `Studio` browse/query,
prepare, authorize and execute paths against disposable PostgreSQL 18 on
loopback port 55432. An independent `Studio` connection replaced each uniquely
named table. Both instances used temporary app roots and `MemorySecrets`.
All four owned tables were dropped before the final safety assertion failed.
Application source was local revision `95694e8`; only the temporary probe differed.

Each original and replacement table had `(id integer PRIMARY KEY, note text NOT
NULL DEFAULT 'before')` and row `(1, 'before')`. Update probes requested
`note = 'approved'`; the insert probe requested `id = 2` with the note omitted.

| Original result | Replacement timing | Mutation | Observed outcome in replacement |
|---|---|---|---|
| Table browse | Before preparing the edit | Update | `Completed`; one row, note `approved` |
| Eligible `SELECT id, note` | Before preparing the edit | Update | `Completed`; one row, note `approved` |
| Eligible `SELECT id, note` | After authorization | Update | `Completed`; one row, note `approved` |
| Table browse | After authorization | Insert | `Completed`; two rows, minimum note `before` |

Every approval was consumed: resubmission returned `ApprovalExpired`. One-use
approval and full-row comparison therefore did not protect the original table
identity. Desired strict behavior is rejection with the replacement unchanged.
The probe ended with exit 101 and was removed from the normal fixture source;
it is **failed investigation evidence**, not another passing regression. Local
probe source and observed values are retained under `.cache/table-identity-probe`.
The steps below remain reproducible without those ignored local artifacts.

## Reproduce on disposable data

Start the README's PostgreSQL, MySQL and MariaDB disposable fixtures. Use the
pinned Rust toolchain documented in the [verification ledger](replacement-verification.md).
Run from the repository root:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example table_identity_probe -- \
  --run-disposable-fixtures --engine all --require-safe
```

The explicit opt-in is mandatory. Only fixed loopback fixtures are accepted:
PostgreSQL 55432, MySQL 53306 and MariaDB 53307, using public fixture credentials.
`--engine postgres`, `mysql` or `mariadb` narrows the run. UUID tables, temporary
application roots and `MemorySecrets` isolate the run. It drops only its owned
tables and verifies removal; no production or OS credential store is accessed.
Exit 1 means operational failure, exit 2 means an observed safety failure with
`--require-safe`. Omitting that flag collects evidence with exit 0 even when
unsafe; it is not a qualification command. Review the JSON, not just exit status.

For a manual before-preparation reproduction:

Use the README fixture setup and two independent Joinery connections to the
PostgreSQL fixture. Choose a unique suffix; use only the table created here.

1. On connection B, create `public.identity_probe_<suffix>` using the DDL above
   and insert `(1, 'before')`.
2. On connection A, browse that table, or run `SELECT id, note FROM
   public.identity_probe_<suffix>`. Retain its original result.
3. For the before-preparation case, use B to drop and recreate that table with
   identical DDL and seed row. Then stage A's original note as `approved` and
   review/submit it.
4. For the after-authorization case, use the backend fixture's
   `prepare_table_changes` and `authorize` before B replaces the table; execute
   the retained plan/token afterward. The ordinary UI submits authorization and
   execution together, so this exact timing needs the Rust fixture probe.
5. For the insert case, retain a browse result, prepare/authorize an insert with
   `c0 = 2`, replace the table on B, then execute the old plan/token.
6. On B, run `SELECT count(*)::text, min(note) FROM
   public.identity_probe_<suffix>`. Observe the values in the table above.
7. Drop only this probe table. Reuse of the consumed plan/token must fail.

The replacement DDL is deliberately destructive **only to the uniquely named
synthetic probe**. This is not an instruction to test against production data.
Namespace changes, partition/inheritance behavior, full permission combinations,
guard expiry and connection loss during an identity guard remain unqualified. The
diagnostic tests rejection after replacement completes; a repair that continuously
locks the original must separately prove bounded DDL blocking and guard cleanup.

## Why a prepare-time check is insufficient

Before the partial PostgreSQL repair, retained provenance carried table name,
schema shape and keys. PostgreSQL locked and compared original row values;
MySQL/MariaDB also lock and compare table shape during apply. Identical
replacement before those checks could satisfy them, as the historical diagnostic
shows. Capturing a new identity at approval would still miss replacement of the
table that produced an older result or CSV preview.

## Partial PostgreSQL repair and current qualification

The identity foundation is committed as `ea3ea76`: vendored `tokio-postgres`
0.7.18 under its MIT/Apache-2.0 licenses, with provenance for three upstream
source changes. Those changes preserve actual simple-query column origins and
support describing a bound portal without executing it. Six synthetic protocol
tests and one ignored set of nine actual client-protocol cases pass. This is
client-library evidence, not a DataGrip comparison or an application repair by
itself.

The PostgreSQL identity repair at `4b7e4c3` carries relation OID, attribute number
and type identity from the actual SELECT, table browse or CSV preview through
stored results, preparation, approval and every batch. Before writing, it
describes and executes the exact bound DML portal under the guarded transaction.
The initial repaired report at
`.cache/table-identity-probe/postgres-repaired-report.json` predates the final
review fixes. The final strict `--engine postgres --require-safe` report at
`.cache/table-identity-probe/postgres-csv-final-report.json` passes all nine
PostgreSQL 18.6 scenarios: every case rejected with `Conflict`, preserved the
replacement, consumed approval, passed fresh controls and cleanup, with zero
operational errors. A separate formal example regression run over the same nine
cases also passed. This is scoped fixture evidence, not general engine
qualification.

Review found an auxiliary-connection race where an edit could commit after the
originating session disconnected. A regression observed value `1` and
`Completed`; checking session registration under the retirement mutex fixed the path,
and its targeted rerun observed value `0` and `ConnectionClosed`. Raw browse
commit/drain handling and a read-only fallback on permission denial are also
implemented. Four actual permission/RLS/raw tests passed, including column-only
read-only fallback and late-error cutoff. Sixteen frontend state-behavior cases
exposed 11 failures while five passed; after repair, the 80-case state test file
passed. Latest validation passes 561 frontend / 186 Rust tests and strict
Clippy. One clean seven-suite fixture command passes 112 tests: 41 MySQL/MariaDB,
43 PostgreSQL, four permission/RLS/raw, one client-protocol suite covering nine
scenarios, six Redis, two SSH and 15 transaction transport. The final strict
PostgreSQL diagnostic and formal example regression are additional, separate
runs. The focused nine-case browser run passes 9/9, including CSV automatic-to-
explicit mapping compatibility, changed-configuration denial and stale-
replacement conflict. After the test-coordinate fix and UUID-based fixture
isolation, the full browser suite passed 129/129; the focused table-data/raw-
browse run passed 11/11. Both actual raw-browse unknown-outcome UI cases passed.
Earlier 126/127 and 10/11 attempts were test-coordinate and fixture-isolation
failures; they are historical, not current product failures. Fresh normal macOS
bundle/signature, two hidden native PostgreSQL recovery pairs and ten-cycle smoke
pass on clean `5762ee5`; the ledger records hashes and qualification limits. Earlier 545 frontend /
180 Rust / 103 fixtures / 124 browser results and normal-bundle hash belong to
the earlier writer repair only.

Scope remains PostgreSQL 18.6. The implementation requires `SELECT` access to
system columns for editable provenance; column-only readers fall back to
read-only, covered by the actual permission tests. OID identity does not
by itself qualify every rule, trigger, partition effect or OID reuse case.
Grid writes remain separate from console transactions. MySQL and MariaDB's 18
historical failures remain open and unsafe; no cross-engine repair is claimed.

Independent Tier 2 reviews checked the driver, result store, provenance,
preparation and CSV paths, with matching coverage metadata and direct source
reads. The important touch points are `results.rs::StoredResult`,
`execution.rs::resolve_statement_provenance`, `Output::describe`,
`browse_with_approval`, `prepare_table_changes`, `preview_csv_import`,
`prepare_csv_import_batch` and each SQL driver's apply path.

## MySQL and MariaDB design boundary

The MySQL and MariaDB wire-level result metadata observed in this investigation
provides names, not a documented table-incarnation identifier that can bind an
old result to a replacement-safe object. See the bounded [MySQL/MariaDB guard
design](mysql-table-identity-design.md) for probe evidence and a falsifiable
next-step proposal. InnoDB dictionary `TABLE_ID` metadata
is documented, but reading those catalogs requires the `PROCESS` privilege in
the documented MySQL 8.4 and MariaDB versions. Joinery must not request that
privilege automatically or treat it as an acceptable default.

One ordinary-role design to falsify on isolated fixtures is a dedicated,
explicitly bounded guard connection: begin a transaction before the original
read or CSV preview, access the target so its metadata lock remains held, and
retain that transaction through review and CSV batches. The writer must acquire
its own metadata lock and then verify the original retained guard using a fresh
non-reconnecting round trip, fresh active-transaction status, generation and
deadline checks before issuing DML. A ping on a replacement connection, automatic
reconnect or a new transaction cannot prove continuity of the original guard.
This protocol premise has passed the bounded standalone probe; application
integration and lifecycle qualification remain open.

The guard can delay migrations. Queued DDL may block a later writer behind the
original guard, so bounded deadlines/TTL, explicit release, cancel, eviction,
disconnect and shutdown cleanup all need measured behavior. Run actual isolated
drop/recreate and queued-DDL experiments, including CSV batch boundaries, before
choosing this design. Production readiness and the owner's concurrent-migration
requirement remain unresolved. Official references: [MySQL metadata locking](https://dev.mysql.com/doc/refman/8.4/en/metadata-locking.html),
[MariaDB metadata locking](https://mariadb.com/docs/server/reference/sql-statements/transactions/metadata-locking),
and [MySQL InnoDB table metadata](https://dev.mysql.com/doc/refman/8.4/en/information-schema-innodb-tables-table.html).

## Remaining proof obligations and design risks

The PostgreSQL foundation and scoped backend path are partially implemented as
described above; this does not qualify every related workflow or the other
engines. Remaining proof obligations and design risks are:

- The upstream PostgreSQL wire `RowDescription` includes relation/attribute
  identity, while upstream `tokio-postgres 0.7.18` simple-query columns retain
  only names. The vendored foundation carries actual execution origins. Keep
  provenance and licensing records, upstream maintenance review, and regression
  coverage with that dependency; separate pre-query `prepare()` metadata alone
  still cannot prove which table a query actually read.
- Permission denial must remain safely read-only; the current four actual
  permission/RLS/raw tests cover column-only grants, late read errors and raw
  browse behavior on the tested fixture versions.
- Locking a qualified PostgreSQL table name and checking its OID once does not
  establish unrestricted namespace safety when later DML parses the name again.
  Qualify schema rename/recreation and prepared-plan invalidation explicitly;
  do not label a simple OID comparison complete.
- The documented InnoDB identity catalogs require `PROCESS` on both
  [MySQL 8.4](https://dev.mysql.com/doc/refman/8.4/en/information-schema-innodb-tables-table.html)
  and [MariaDB](https://mariadb.com/docs/server/reference/system-tables/information-schema/information-schema-tables/information-schema-innodb-tables/information-schema-innodb_sys_tables-table).
  Do not grant privileges or substitute schema hashes/timestamps automatically.
  A privilege-free alternative is an explicit bounded edit connection retaining
  its metadata lock from snapshot through write. That changes resource use and
  can delay migrations; expiry, cancel, disconnect and shutdown must release it.
- Batch commits release transaction locks. CSV must retain the original target
  through each batch and qualify replacement after preview, after approval and
  between commits. Current PostgreSQL cases pass the final strict report; MySQL/MariaDB
  still demonstrate the unsafe outcome.

Complete qualification still needs all nine cases on each engine, queued writers,
permission denial, connection loss, guard expiry and namespace/partition cases.
Existing manual console transactions must never be committed or rolled back by a
hidden editing guard. UI workflows must show expiry/rejection and require a fresh read.

## Replacement decision and current usage

JetBrains documents staged table edits, DML preview, submit/revert and transaction
choices. Its bounded [submit-changes documentation](https://www.jetbrains.com/help/datagrip/submitting-and-reverting-changes.html)
also discusses code conflicts and an outdated-DDL warning; those passages do
not specify grid behavior across table drop/recreation. No DataGrip failure or
protection has been established here.

The owner's concurrent-migration workflow is now an explicit open acceptance
question. This capability remains provisional and incomplete; it has not been
silently classified as optional or assumed to block every possible daily
workflow. Until qualification is resolved, discard pending edits/import previews
and read again after external DDL; coordinate migrations so they cannot replace
the target during review and submission. A fresh read alone does not protect
against replacement occurring afterward.
