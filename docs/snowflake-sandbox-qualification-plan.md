# Bounded `SANDBOX.BILLIE_TESTING` live qualification and acceptance plan

**Status: Prepared for explicit owner review and approval. Zero live Snowflake
authentication requests, queries, writes, DDL statements, or billable warehouse
operations have been executed to prepare or validate this plan.**

This document defines the concrete, bounded live-account qualification and
hands-on macOS acceptance walkthrough required before retiring JetBrains
DataGrip (**2026.3**, build `263.7307.216`) for day-to-day Snowflake production
work. Read alongside [Snowflake DataGrip replacement](snowflake-datagrip-replacement.md),
[daily replacement acceptance](replacement-acceptance.md), and the
[verification ledger](replacement-verification.md).

## 1. Pre-execution authorization gate

No phase below may be started until the owner explicitly approves:

1. **Exact scope:** newly created disposable objects prefixed with
   `JOINERY_QUAL_<RUN>_` inside `SANDBOX.BILLIE_TESTING` only (`<RUN>` is a
   fresh 32-character uppercase hexadecimal run token generated for the single
   approved window). Existing tables, views, stages, functions, and procedures
   in `SANDBOX.BILLIE_TESTING` and every other database/schema are off-limits
   for mutation and row-level reads.
2. **Identity and billing:** `authenticator=externalbrowser` SSO on the owner's
   Snowflake account, using the approved role and existing warehouse (for
   example `X-SMALL`), including explicit approval for warehouse auto-resume and
   compute/cloud-services billing during the bounded qualification window.
3. **Exclusive disposable object window:** no concurrent user or automation
   mutates `JOINERY_QUAL_<RUN>_*` objects while the run is active.
4. **Hard stop on uncertainty:** if any write, DDL statement, or transaction
   boundary returns an `unknown` delivery outcome, unexpected principal/context
   drift, privilege denial, or name collision, stop immediately, do **not**
   automatically retry or replay SQL, and inspect Snowflake `QUERY_HISTORY`
   before taking manual recovery or teardown steps.

## 2. Application binary and isolated local state

- **Application bundle:** uninstalled schema-8 ARM64 macOS bundle at
  `src-tauri/target/release/bundle/macos/Joinery.app`, built from clean
  `feat/datagrip-replacement` with bundled Apache-2.0 `snowflake-sdk` `3.4.1`
  and pinned Node `24.22.2`
  (`b6fb419f0f43bb2fbca273700294dd2f3b685a64b60e8417c295f47646a70e4a`).
- **Preserved prior bundle:** the prior schema-7 `d6cfb2a` bundle remains
  separately preserved at `.cache/verification/preserved-release-d6cfb2a/Joinery.app`
  (`0600dc7978fa9bcddbe67a3cae910abf560b23e163d04b0b6e35be3ee26b99ac`).
- **Disposable local data root:** launch the qualification run against an
  isolated macOS application-data directory (`JOINERY_DATA_DIR` under a `0700`
  local evaluation directory) so schema-8 SQLite migrations and encrypted
  recovery tests never touch existing local profiles or history without an
  explicit backup.
- **Profile configuration:** create one isolated **Staging** (`Confirm` Safe
  Mode) Snowflake profile with:
  - Authentication: `externalBrowser`
  - Initial database / schema: `SANDBOX` / `BILLIE_TESTING`
  - Console query limit: `300` seconds
  - Explicit opt-ins enabled for qualification: `reviewedDml`, `reviewedDdl`,
    `transactionControls`, and `consoleScripts`

## 3. Disposable objects created and dropped

For run token `<RUN>`, only the following three objects are created, exercised,
and dropped in `SANDBOX.BILLIE_TESTING`:

| Object                                                      | Type                                                                          | Purpose                                                                                                                                                                | Teardown                  |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| `"SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_PARENT"`    | Base table (with declared `PRIMARY KEY`)                                      | Parent target for `ObjectInspectorView` foreign-key column mapping, referenced-table jump, and `EXPLAIN USING JSON` join plan diff                                     | `DROP TABLE ... RESTRICT` |
| `"SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_ALL_TYPES"` | Base table (14 columns across all lossless codecs + declared FK to `_PARENT`) | Reviewed DDL column operations, lossless inline grid editing, server sort/filter/offset paging, `<ReviewDialog>` cancel recovery, and 1,200-row multi-batch CSV import | `DROP TABLE ... RESTRICT` |
| `"SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_SUMMARY_V"` | View                                                                          | Reviewed `CREATE VIEW` / `DROP VIEW`, `ObjectInspectorView` view DDL and dependency inspection                                                                         | `DROP VIEW ...`           |

## 4. Ordered execution phases and statement budgets

### Phase 1 — External-browser SSO, session observation, and guarded context switching

- **Actions:**
  1. Connect the Staging profile via `externalbrowser` SSO; verify browser
     handoff, TLS certificate validation, and zero credential storage in SQLite
     or logs.
  2. Confirm the toolbar runtime badges report exact `CURRENT_USER()`,
     `CURRENT_ROLE()`, `CURRENT_WAREHOUSE()`, `SANDBOX`, `BILLIE_TESTING`,
     `AUTOCOMMIT=true`, and `QUOTED_IDENTIFIERS_IGNORE_CASE=false`.
  3. Open the guarded Role, Warehouse, Database, and Schema selectors
     (`snowflake_session_contexts`): switch schema to `"INFORMATION_SCHEMA"` and
     back to `"BILLIE_TESTING"` via `switch_snowflake_context` (and exercise
     approved role/warehouse/database round-trip back to
     `SANDBOX.BILLIE_TESTING`), confirming `CURRENT_SESSION()` continuity and
     immediate completion/catalog invalidation.
- **Statement budget:** `1` SSO handshake + `<= 45` bounded read/context
  observations + `2..6` `USE ROLE/WAREHOUSE/DATABASE/SCHEMA` commands.

### Phase 2 — Reviewed Snowflake DDL lifecycle (8 actions)

- **Actions:**
  1. Via `SnowflakeDdlDialog` (`prepare_snowflake_ddl` -> `<ReviewDialog>` ->
     `execute_snowflake_ddl` -> `"Reconcile catalog"`):
     - `createTable`: create `"SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_ALL_TYPES"`
       with columns:
       - `"ID" NUMBER(38, 0) NOT NULL`
       - `"PARENT_ID" NUMBER(38, 0)`
       - `"UNIT_FRAC" NUMBER(38, 38)`
       - `"RATIO" FLOAT`
       - `"ACTIVE" BOOLEAN`
       - `"LABEL" VARCHAR(256)`
       - `"RAW_BYTES" BINARY(64)`
       - `"D_VAL" DATE`
       - `"T_VAL" TIME(9)`
       - `"TS_NTZ" TIMESTAMP_NTZ(9)`
       - `"TS_LTZ" TIMESTAMP_LTZ(9)`
       - `"TS_TZ" TIMESTAMP_TZ(9)`
       - `"PAYLOAD" VARIANT`
       - `"ITEMS" ARRAY`
       - `"META" OBJECT`
     - `addColumn`: add `"TEMP_NOTE" VARCHAR(128)`
     - `alterColumnNullability`: set `"TEMP_NOTE"` `NOT NULL` (empty table), then
       `DROP NOT NULL`
     - `renameColumn`: rename `"TEMP_NOTE"` to `"RENAMED_NOTE"`
     - `dropColumn`: drop `"RENAMED_NOTE"`
     - `createView`: create `"SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_SUMMARY_V"`
       as `SELECT "ID", "LABEL", "ACTIVE" FROM "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_ALL_TYPES"`
  2. Via the reviewed retained SQL console, create `"JOINERY_QUAL_<RUN>_PARENT"`
     (`"PARENT_ID" NUMBER(38, 0) PRIMARY KEY`, `"NAME" VARCHAR(64)`) and add an
     informational `FOREIGN KEY ("PARENT_ID") REFERENCES "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_PARENT" ("PARENT_ID")`
     on `"JOINERY_QUAL_<RUN>_ALL_TYPES"` so constraint mappings are populated on
     disposable objects.
- **Statement budget:** `7` reviewed generated DDL operations + `2` reviewed
  console DDL statements + explicit catalog reconciliations (`<= 280` bounded
  metadata/observation reads at `S <= 10` schemas).

### Phase 3 — Object Explorer, Object Inspector, and Snowflake SQL authoring

- **Actions:**
  1. Expand `SANDBOX` -> `BILLIE_TESTING` in the sidebar Object Explorer
     (`database_objects`); verify tables and views render with accurate kinds.
  2. Open `"JOINERY_QUAL_<RUN>_ALL_TYPES"` and `"JOINERY_QUAL_<RUN>_SUMMARY_V"`
     in `ObjectInspectorView` (`object_details`): verify column types/nullability,
     PK/FK constraint mappings, view dependencies, `GET_DDL` output, and the
     `"Browse relations"` jump button.
  3. In `SqlEditor`, verify Snowflake SQL keyword/function completion
     (`QUALIFY`, `ILIKE`, `PARSE_JSON`, `OBJECT_CONSTRUCT`), quoted column
     completions, semi-structured path syntax (`PAYLOAD:tier::STRING`), and
     `"Inspect SQL columns"` (`completion_schema`) live column verification.
- **Statement budget:** `<= 25` bounded `SHOW` / `DESCRIBE` / `GET_DDL` /
  `INFORMATION_SCHEMA` reads.

### Phase 4 — Lossless editable grid, slice paging/sort/filter, review cancel, and multi-batch CSV import

- **Actions:**
  1. Open `"JOINERY_QUAL_<RUN>_ALL_TYPES"` in `RelationBrowser`.
  2. Stage `3` new rows exercising:
     - `NUMBER(38, 0)` values beyond JS safe integers
       (`9007199254740993`, `-99999999999999999999999999999999999999`)
     - `NUMBER(38, 38)` full-scale decimals
       (`0.12345678901234567890123456789012345678`, `-0.00000000000000000000000000000000000001`)
     - `FLOAT` (`3.141592653589793`, `NaN`, `Infinity`), `BOOLEAN`, `VARCHAR`
       (`NULL` vs `""` vs Unicode/case-sensitive text), hex `BINARY`, `DATE`,
       nanosecond `TIME(9)` and `TIMESTAMP_NTZ/LTZ/TZ(9)`, and structured JSON
       `VARIANT` / `ARRAY` / `OBJECT` (`INSERT INTO ... SELECT ... PARSE_JSON(...)`).
  3. Click `"Review 3 changes…"`, inspect the compiled SQL/bindings in
     `<ReviewDialog>`, click **Cancel** (`cancel_snowflake_grid_plan`), and
     verify the snapshot is restored so `"Review 3 changes…"` re-opens and
     applies cleanly in **Automatic** transaction mode without reloading.
  4. Reload preview and verify lossless readback across all 15 columns.
  5. Apply a server **Filter** (`ACTIVE = TRUE` / `LABEL ILIKE '%alpha%'`),
     **Sort** (`ID DESC`), and custom page slice (`offset > 0`), then stage and
     commit a single-row `UPDATE` (including `VARIANT` JSON and UTF-8
     collation-sensitive text) and a single-row `DELETE`, confirming exact
     affected-row count `1` and whole-original predicate matching.
  6. Switch transaction mode to **Manual** and import a synthetic **1,200-row**
     CSV file (`3` sequential batches: `500` + `500` + `200` rows) into
     `"JOINERY_QUAL_<RUN>_ALL_TYPES"`:
     - Verify multi-batch progress and the **Pending Snowflake grid transaction**
       banner.
     - Click `"Reload pending preview"` (`preview_snowflake_grid_transaction_relation`)
       with slice sort/offset, stage a continuation cell edit on an imported row,
       and click `"Rollback transaction"`, confirming all `1,200` pending rows
       and the continuation edit are rolled back cleanly.
     - Re-run the `1,200`-row multi-batch CSV import in **Manual** mode and click
       `"Commit transaction"`, then refresh completion context and confirm exact
       row count `1,202`.
- **Statement budget:** `<= 14` owned mutation batches (`BEGIN`, bound DML
  units, `DESCRIBE TRANSACTION`, `COMMIT`/`ROLLBACK`) + `12`-read freshness
  brackets per batch/preview.

### Phase 5 — Explicit multi-statement transactions, reviewed SQL DML, and retained console recovery

- **Actions:**
  1. Run typed `BEGIN` -> reviewed parameterized `INSERT` / `UPDATE` / `DELETE`
     (`prepare_snowflake_dml` / `execute_snowflake_dml`) -> `ROLLBACK`, then
     `BEGIN` -> reviewed `UPDATE` -> `COMMIT`, verifying affected-row counts and
     data state.
  2. In the opted-in retained SQL console, run a multi-statement session script
     (`SET joinery_qual_var = 42; SELECT $joinery_qual_var AS v; SELECT * FROM TABLE(RESULT_SCAN(LAST_QUERY_ID()));`),
     verify `Console receipts` pagination/lookup, quit and relaunch `Joinery.app`
     against the same evaluation `JOINERY_DATA_DIR`, and confirm history and
     receipts reload with **zero** automatic reconnect or SQL replay.
- **Statement budget:** `2` explicit transaction cycles + `1` three-statement
  retained console run (`<= 40` total statements including observations).

### Phase 6 — `EXPLAIN USING JSON` plan capture and snapshot comparison

- **Actions:**
  1. Run **Explain** (`EXPLAIN USING JSON`) on:
     ```sql
     SELECT a."ID", a."LABEL", p."NAME"
     FROM "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_ALL_TYPES" a
     JOIN "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_PARENT" p
       ON a."PARENT_ID" = p."PARENT_ID"
     WHERE a."ACTIVE" = TRUE
     ```
  2. Inspect the rendered operator DAG (`GlobalStats`, step expressions, raw
     JSON tab, and observed `planContext` badges).
  3. Capture a second **Explain** snapshot of the same query after inserting
     parent rows / updating statistics and verify side-by-side `"Plan comparison"`
     step deltas.
- **Statement budget:** `2` `EXPLAIN USING JSON` reads + `2` context checks.

### Phase 7 — `1,000,000`-row streaming read, disk-backed paging, cancellation, and CSV export

- **Actions:**
  1. Enable **Allow temporary disk storage for large results** before running.
  2. Execute the zero-write synthetic 1,000,000-row query:
     ```sql
     SELECT
       SEQ8() AS ID,
       TO_DECIMAL(SEQ8(), 38, 0) + 9007199254740993 AS BIG_ID,
       SHA2(TO_VARCHAR(SEQ8()), 256) AS PAYLOAD_HASH,
       TIMESTAMP_NTZ_FROM_PARTS(2026, 11, 14, 0, 0, MOD(SEQ8(), 60)) AS CREATED_AT,
       OBJECT_CONSTRUCT('seq', SEQ8(), 'bucket', MOD(SEQ8(), 32)) AS META
     FROM TABLE(GENERATOR(ROWCOUNT => 1000000))
     ```
  3. Record wall-clock elapsed time, `Joinery.app` + Node helper peak RSS, and
     bounded temporary spill usage.
  4. Verify exact `1,000,000` retained rows; jump to first (`1`), middle
     (`500,000`), and final (`1,000,000`) grid pages without re-running SQL;
     verify `BIG_ID` decimal precision and `META` JSON fidelity.
  5. Export the full retained snapshot to CSV and verify `1,000,001` lines
     (header + `1,000,000` rows) and deterministic first/last row values
     independently via `wc -l` and `shasum -a 256`.
  6. Start a second `GENERATOR(ROWCOUNT => 1000000)` query and cancel mid-stream;
     verify prompt cancellation, honest `Cancelled` status, and clean helper
     reuse on a subsequent `SELECT 1`.
  7. Explicitly release the retained large-result buffer and confirm temporary
     spill files are deleted.
- **Statement budget:** `2` read-only `GENERATOR` queries + `1` `SELECT 1` probe.

### Phase 8 — Deterministic teardown and zero-residue verification

- **Actions:**
  1. Via `SnowflakeDdlDialog`, review and execute:
     - `dropView`: `DROP VIEW "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_SUMMARY_V"`
     - `dropTable`: `DROP TABLE "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_ALL_TYPES" RESTRICT`
     - `dropTable`: `DROP TABLE "SANDBOX"."BILLIE_TESTING"."JOINERY_QUAL_<RUN>_PARENT" RESTRICT`
  2. Click `"Reconcile catalog"`, then run bounded verification reads confirming
     `0` remaining `JOINERY_QUAL_<RUN>_*` tables or views in
     `SANDBOX.BILLIE_TESTING`.
  3. Disconnect the Snowflake session, quit `Joinery.app`, and verify private
     helper/spill directories are cleaned up with zero credential artifacts.
- **Statement budget:** `3` reviewed `DROP` statements + `1` catalog
  reconciliation + `2` zero-residue `SHOW` checks.

## 5. Acceptance evidence to record upon authorized execution

When the owner approves and runs this plan, record in
`docs/replacement-verification.md` (with zero secrets, tokens, or private URLs):

- Exact `Joinery.app` commit hash and executable SHA-256
- Approved `<RUN>` token, warehouse size, and UTC start/end timestamps
- Snowflake query IDs (or anonymized status counts if preferred) and outcomes for
  each phase (`1` through `8`)
- `1,000,000`-row elapsed fetch time, peak RSS, temporary disk spill size, and
  exported CSV line count + SHA-256
- Confirmation of zero remaining `JOINERY_QUAL_<RUN>_*` objects in
  `SANDBOX.BILLIE_TESTING`
- Any usability, keyboard/accessibility, or engine discrepancy discovered during
  the owner's hands-on evaluation before retiring DataGrip 2026.3
