# Retained edits and replacement tables

Investigation date: 9 November 2026. **Incomplete: PostgreSQL, MySQL and MariaDB failures reproduced;
no repair implemented.** This is a safety qualification within the active
[replacement Goal](replacement-acceptance.md), not a claim about undocumented
DataGrip behavior. The passing [qualification baseline](replacement-verification.md)
does not cover this scenario.

## Reproduced behavior

The durable diagnostic at
[`src-tauri/examples/table_identity_probe.rs`](../src-tauri/examples/table_identity_probe.rs)
now reproduces **27 failures: nine per engine** on PostgreSQL 18.6, MySQL 8.4.13
and MariaDB 11.4.13. Each operation reported `Completed` and changed the replacement.
All 27 approvals were consumed, fresh-read insert controls succeeded, and owned
table removal was independently verified. There were no operational errors.

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
`safetyPassed: false`. The application remains unchanged from `ff59ca6`.

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
Namespace changes, partition/inheritance behavior, permissions, guard expiry and
connection loss during a future identity guard remain unqualified. The diagnostic
tests rejection after replacement completes; a repair that continuously locks the
original must separately prove bounded DDL blocking and guard cleanup.

## Why a prepare-time check is insufficient

Retained provenance currently carries table name, schema shape and keys.
PostgreSQL locks and compares original row values; MySQL/MariaDB additionally
lock and compare table shape during apply. Identical replacement before those
checks can satisfy them. Capturing a new identity at approval would still miss
replacement of the table that produced an older result or CSV preview.

Independent Tier 2 reviews checked the driver, result store, provenance,
preparation and CSV paths, with matching coverage metadata and direct source
reads. The important touch points are `results.rs::StoredResult`,
`execution.rs::resolve_statement_provenance`, `Output::describe`,
`browse_with_approval`, `prepare_table_changes`, `preview_csv_import`,
`prepare_csv_import_batch` and each SQL driver's apply path.

## Repair requirements and design options

This is a design proposal, not implemented capability:

- Retain a backend-owned `EditTarget { schema, identity }` from the original
  read, including empty results, and carry it through every contributing result,
  prepared plan and approved mutation. Schema-only consumers can keep their
  existing metadata interface.
- Verify the actual mutation target under a transaction-held guard before every
  DML operation, including inserts. Preserve full-row conflicts, atomicity,
  cancellation, one-use approval and truthful uncertain-COMMIT handling.
- PostgreSQL's actual wire `RowDescription` includes relation/attribute identity,
  but installed `tokio-postgres 0.7.18` simple-query columns retain only names.
  Exposing actual execution origins is a candidate client-library change. The
  current separate `prepare()` metadata cannot prove which table the subsequent
  simple-query execution read. A dependency change needs licensing, maintenance
  and regression review before adoption.
- Browsing needs schema, identity and rows collected in one protected driver
  operation. A CSV preview needs an opaque backend capability binding session,
  source grant, configuration and original target across all reviewed batches.
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
- Batch commits release transaction locks. CSV must retain a separately proven
  guard or reacquire the target and obtain fresh review for the next batch.
  A lock obtained only at approval does not protect an older preview.

Required real-engine regressions include all nine cases above, queued writers, permission
denial, connection loss, guard expiry and namespace/partition cases. Existing
manual console transactions must never be committed or rolled back by a hidden
editing guard. UI workflows must show expiry/rejection and require a fresh read.

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
