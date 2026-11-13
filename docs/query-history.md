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

Current execution admits a history row before dispatch. Only the connection's
current persisted setting can permit SQL text; a stale open session cannot
override an opt-out. The original reviewed template is retained exactly,
including comments and Unicode, before parameter binding or wire rewriting.
Completion carries a SQL-free ticket and cannot recreate a cleared or pruned
row. Turning text storage off, deleting the profile or clearing history revokes
authority to retain the corresponding SQL. Turning storage back on does not restore
previously removed text.

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

Generic, typed Snowflake and retained-console paths now create admitted history
before dispatch and finish through the original ticket. Console Prepare and
Complete atomically bind history, receipt and audit evidence; publisher sealing
must settle these obligations before releasing a recording hold. Startup can
recover terminal console evidence whose publisher did not finish. Compatible
metadata restore preserves disjoint pending SQL and marks restored evidence;
live clear/opt-out authority prevents an older backup from reviving removed text.
Incomparable privacy branches are refused before creating undo or replacing data.

Generic process-crash owner recovery remains incomplete: an unowned admitted row
stays admitted rather than guessing its outcome. Generic and typed completion
retry payloads remain process-local until SQLite accepts them; a durable history
ticket alone does not preserve a failed completion payload across process death.
The console publication-panic fallback preserves its known status with unavailable
duration when the owner's monotonic timer is no longer available. No recovery
automatically reconnects or replays SQL.

History writes, full-detail reads, summary pages, compatibility reads and backup
validation share lifecycle invariants. Missing legacy provenance defaults to
Recorded; explicit null or unknown provenance is invalid. Admitted rows cannot
carry a terminal status, measured duration or completed execution/DML/DDL
evidence. An unknown completion cannot carry a measured duration. Schema 5 adds
SQL-free admission/privacy authority and physically fences cached schema-4
writers. Preserve an original backup and use a separate evaluation metadata root;
the previous `adabd44` app cannot open schema-5 metadata. Do not run older and
newer builds against the same directory.

## Current limits and remaining requirements

Search accepts at most 512 UTF-8 bytes. Matching lowercases Unicode text and
requires every whitespace-separated word; it is not full Unicode case folding
(`Straße` and `STRASSE` differ). Preview length counts Unicode scalar values,
not grapheme clusters. The backend accepts 1–100 entries per page and validates
cursor identities and JavaScript-safe timestamps.

Exact SQL retained for a new admission is limited to 1 MiB. With SQL-text history
enabled, an oversized template is refused before dispatch. This local safety
bound does not establish complete DataGrip script-size parity. New ticket seed
and completion metadata are SQL-free and individually bounded; that does not
bound their lifetime count.

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

The [atomic history/privacy contract](snowflake-console-retention-design.md#atomic-history-lifecycle-and-privacy-contract)
now has local execution/storage coverage. Durable reservations and a complete
resource/reclamation policy for lifetime tickets, clear eras and privacy
generations remain required; console count gates do not bound these tables.
Generic crash-owner proof, full-capacity measurements, account qualification and
owner acceptance also remain open. Keep both count gates and
`retentionEnabled=false`. See the
[capability matrix](datagrip-feature-matrix.md) and
[verification ledger](replacement-verification.md).

## Reproducible local acceptance

Current-source full validation passes 1,038 frontend and 676 standard Rust cases,
including all eight history integration and nine encrypted history privacy cases.
All ten actual isolated PostgreSQL cases, 73 synthetic Snowflake execution cases,
27 offline qualification-runner cases and six actual browser cases pass. Exact
source scopes, commands, initial failures and corrections are
recorded in the ledger. The new normal SDK-bearing package remains pending;
`src-tauri/target/release/bundle/macos/Joinery.app` is still the preceding clean
`adabd44` schema-4 app, now separately preserved. Keep its metadata root separate.
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
duration through the history view and keyboard palette. Those seeded cases
submit no database SQL. The additional live-history case runs an actual isolated
PostgreSQL sleep, inspects admission before completion, clears history and checks
late completion plus renderer reload. The separately scoped parameter case
executes reviewed PostgreSQL bindings and verifies template-only history.

For a manual walkthrough, execute a disposable query with history text enabled,
open Query history, search for text near the end of the statement, and activate
its summary. Confirm the complete statement opens without running. Repeat with
history text disabled, then clear history and verify that refresh and the palette
show no retained entries. Actual Snowflake and normal packaged macOS acceptance
remain separate gates.
