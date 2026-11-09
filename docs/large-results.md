# Large query results

The owner requires Snowflake results up to one million rows. The shared result
store now has an opt-in temporary-storage implementation. Actual isolated
PostgreSQL and browser workflows retain and export one million rows; final
source checks pass 582 frontend / 205 standard Rust tests, 124 actual fixtures,
three dedicated large-result fixtures and all 138 browser workflows. Native
preference/restart qualification follows separately. This does not qualify Snowflake
authentication, SDK memory use, types, million-row transport or writes.

## Use and privacy

Open **Preferences → Data privacy** and enable **Temporary disk storage for large
results** before running a large query. The setting is off by default and saved
in local metadata. Each newly created result captures the current choice. Turning
it off does not invalidate existing results; close them to release their data.
Restoring a metadata backup applies its saved preference immediately.

The shared result store admits up to 64 MiB of retained Arrow data, indexes and
metadata. With temporary storage enabled, subsequent batches can spill to private
anonymous files, up to 8 GiB globally. Each result retains at most 1,000,000 rows;
at most 100 result descriptors can exist. Files are opened privately and unlinked
before writing on macOS. Closing results, application exit or process termination
closes their owned handles. Joinery does not encrypt the temporary data or promise
secure erasure. Disk protection is provided by the operating system.

Rows never enter the SQLite workspace, backups or history. Restart restores the
chosen preference and permitted drafts, with no query execution or result replay.
Explicit exports use their own native save-dialog grants and destination files.

## Bounds and failure behavior

The 64 MiB accounting limit applies to retained result-store structures; it is
not a process-RSS limit. Drivers, incoming rows, bounded codec buffers, the WebView
and filesystem cache use additional memory. The frontend keeps its separate
32 MiB / 512-page cache. Disk reads load requested batches rather than the entire
snapshot; result export iterates batches without rerunning SQL.

Normal stored batches contain at most 512 rows and approximately 8 MiB of values.
A single larger row can occupy its own bounded batch, up to the 64 MiB input-row
limit. The existing 4 MiB IPC frame limit still means some individual large values
cannot be displayed; complete large-object support remains a replacement gap.
Rows beyond the row/memory/disk quota are not retained and the snapshot is marked
**Retention limit reached**. Query completion and retention completeness are
separate. An append I/O error preserves the atomically retained prefix and reports
a notice; driver-specific cancellation and SQL outcome handling remain responsible
for the database statement. Read errors surface through the grid/export error path.
No automatic query retry fills a truncated or unavailable snapshot.

Result exports have a separate 1,000,000-row / 512 MiB limit. Wider million-row
results can therefore require further export work. Copying, comparison and imports
also retain their existing independent limits; temporary storage does not remove
them. Snowflake connector buffering and large-value protocol limits must be
qualified separately before replacement acceptance.

## Reproducible acceptance

Use only the disposable PostgreSQL fixture from `compose.yaml`:

1. Enable temporary storage and run:

   ```sql
   SELECT n AS id, repeat('x', 96) AS payload
   FROM generate_series(1, 1000000) n;
   ```

2. Wait for **1,000,000 rows** and completed status, without a retention warning.
   Inspect the status bar's separate memory and temporary-storage figures.
3. Select the first `id` cell and press **⌘↓** to reach `1000000`; **↑** reaches
   `999999`. Paging should fetch those retained rows without another SQL run.
4. Disable temporary storage. Export the existing snapshot as CSV. Verify one
   header plus one million records, exact ordering/content and no partial file.
5. Close the result. Its temporary bytes should be released. Quit/reopen and
   verify the preference persists while results and live sessions do not restore.

Automated checks (pinned repository toolchains):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test large_results -- --include-ignored --nocapture
pnpm exec playwright test tests/e2e/large-results.spec.ts
```

The Rust fixture checks one million rows with exact bigint/decimal/Unicode/
NULL/empty values, boundary and last-row Arrow pages, full streamed CSV hash and
cleanup. Browser coverage exercises opt-in persistence, actual query, keyboard
navigation, complete retained-snapshot export and release. These commands must
pass on the final revision; outcomes belong in the
[verification ledger](replacement-verification.md).
