# Query history browsing and search

Open **Query history** to browse local executions. Search words may occur in
different parts of the saved SQL, connection name or execution metadata. Search
covers all retained entries, including text beyond the visible preview. The view
shows 50 summaries at a time; use **Next history page** and **Previous history
page**. **Refresh** returns to the newest page. Pages use descending execution
time and identity to keep timestamp ties deterministic.

Searchable metadata includes lifecycle provenance, unavailable duration, run
status, script counts and stop reason,
Snowflake DML/DDL kind and outcome, validated affected/multi-joined row counts,
unavailable-summary status, DDL target, retirement and reconciliation state.
Numeric elapsed time, dates and general explanatory warnings are not search fields.
Complete DataGrip query-history, SQL-log and Local History workflows remain
separately tracked requirements.

The command palette's **History** scope searches the same retained history. It
shows up to 80 matches; **Open query history** opens the complete paged browser,
where you can enter a more specific search. Other palette scopes continue to
search their own actions, objects, tabs and saved queries.

Summaries include at most 256 Unicode scalar values of SQL preview, with an
ellipsis when truncated. Selecting an entry fetches its current exact saved SQL
and opens a new editor tab. This never connects or executes a statement. Empty
stored SQL opens an empty editor. A metadata-only entry reports that SQL was not
saved. If the entry was cleared or pruned, opening reports that it is missing;
the preview is never substituted for the original SQL.

SQL text is saved only when the connection's history setting permits it. Parameter
values and rewritten wire SQL are excluded. **Clear history** requires local
confirmation and removes history rows; audit entries remain. Clearing invalidates
pending list and detail responses in the history view and mounted palette. An
already opened editor is independent work and is not erased by clearing history.

## Completion and recovery evidence

History distinguishes a recorded outcome from admission and local recovery. A
missing measured duration displays **Duration unavailable**, never an invented
zero. A known original status remains visible when its duration is unavailable.

| Provenance | Presentation and meaning |
| --- | --- |
| Recorded | The original recorded status; this includes failed, cancelled, stopped and unknown-outcome results, as well as completed results. Older rows without provenance retain their original status and duration |
| Admitted | **Admitted; completion pending**. Admission does not prove dispatch or server submission |
| Interrupted | **Recovered after interruption**, preserving any original recorded completion; otherwise **Completion unknown** |
| Restore recovered | **Recovered from metadata restore**, preserving any original recorded completion; otherwise **Completion unknown** |

The same labels appear in Query history and the keyboard palette. Reopening an
entry still requires explicit activation and only opens saved SQL in a new tab.
Recovery provenance does not authorize resuming a run or infer failure,
cancellation, rollback or committed effects.

This is the shared history representation and presentation contract. Normal
execution producers currently record terminal rows. The console publication-panic
fallback now preserves its recorded status with unavailable duration because the
owner's monotonic timer is no longer available. Production admission, interruption
recovery and metadata restore do **not yet create the new lifecycle states**;
the fixed local fixture exercises those states. Atomic creation/completion and
privacy suppression remain the next required implementation.

History writes, full-detail reads, summary pages, compatibility reads and backup
validation share lifecycle invariants. Missing legacy provenance defaults to
Recorded; explicit null or unknown provenance is invalid. Admitted rows cannot
carry a terminal status, measured duration or completed execution/DML/DDL
evidence. An unknown completion cannot carry a measured duration. These JSON
contract changes do not migrate the SQLite physical schema, which remains 4.

## Current limits and remaining requirements

Search accepts at most 512 UTF-8 bytes. Matching lowercases Unicode text and
requires every whitespace-separated word; it is not full Unicode case folding
(`Straße` and `STRASSE` differ). Preview length counts Unicode scalar values,
not grapheme clusters. The backend accepts 1–100 entries per page and validates
cursor identities and JavaScript-safe timestamps.

Paging is a live view, not a snapshot across inserts, pruning or restoration.
Refresh to restart browsing after changes. The existing policy retains up to
10,000 executions and 30 days, pruned when recording history; configurable
retention and full DataGrip history parity remain required.

Empty searches project short previews in SQLite. Nonempty searches stream one
SQL row at a time and can scan the retained store under the shared persistence
mutex. The sorter holds keys rather than all SQL payloads. This bounds returned
row count, not arbitrary legacy row size or full-capacity latency. Large-payload
memory and writer-contention acceptance remain open. The compatibility `history`
API still returns up to 1,000 full matching entries; the history view and palette
use the new summary/detail APIs.

This browsing and lifecycle-model increment does not implement the dependent
[atomic admitted/completed history and privacy contract](snowflake-console-retention-design.md#atomic-history-lifecycle-and-privacy-contract),
publisher settlement or durable reservations. Those remain required, together
with account qualification and owner acceptance. See the
[capability matrix](datagrip-feature-matrix.md) and
[verification ledger](replacement-verification.md).

## Reproducible local acceptance

Current-source full validation passes 1,038 frontend and 653 standard Rust cases,
including 15 inline history cases, a backup-invariant case and all eight new
integration cases. All 54 focused UI, 68 feature-specific Snowflake producer and
five actual browser cases pass. The local SDK-bearing ARM64 app at
`src-tauri/target/release/bundle/macos/Joinery.app` packages from clean `adabd44`;
resources, strict ad-hoc signature, original license bytes and guarded SDK startup
pass with zero authentication/SQL. The preceding `99a3ca3` app is preserved.
Normal packaged owner interaction and restart acceptance remain open.

Use the pinned Node/Rust toolchains and isolated fixture setup in the README.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib persistence::history -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test history_lifecycle -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features test-harness --lib execution::snowflake_ -- --test-threads=1
pnpm exec vitest run src/features/studio/HistoryMetadata.test.tsx src/features/studio/UtilityViews.test.tsx src/features/studio/SearchEverywhere.test.tsx src/features/studio/search-everywhere.test.ts
pnpm test:e2e tests/e2e/history.spec.ts
```

The browser suite seeds 1,011 fixed synthetic history rows through a test-only
loopback command excluded from desktop IPC. It exercises real local SQLite,
timestamp ties, paging, a match older than the newest 1,000 entries, deep Unicode
SQL text, exact reopening, renderer reload, metadata-only and empty SQL, and
clearing while an actual detail response is delayed. Five older fixture rows
exercise pending/recovered provenance, known original completion and unavailable
duration through the history view and keyboard palette. These history cases
submit no database SQL. The separately scoped PostgreSQL browser case executes
reviewed synthetic bindings and verifies template-only history.

For a manual walkthrough, execute a disposable query with history text enabled,
open Query history, search for text near the end of the statement, and activate
its summary. Confirm the complete statement opens without running. Repeat with
history text disabled, then clear history and verify that refresh and the palette
show no retained entries. Actual Snowflake and normal packaged macOS acceptance
remain separate gates.
