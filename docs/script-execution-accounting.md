# Script execution accounting

This increment makes the existing SQL executor's sequential stopping behavior
visible. It does not broaden Snowflake SQL admission or add mixed-script support.
Current verification and remaining acceptance are recorded in the
[ledger](replacement-verification.md).

## Contract

`QueryEvent::Finished` and `HistoryEntry` optionally carry `executionSummary`:

| Field | Meaning |
|---|---|
| `planned` | Number of statements in the retained SQL plan |
| `attempted` | Number of local driver execution calls invoked |
| `completed` | Number of those calls returning success |
| `notAttempted` | `planned - attempted`; these statements were never handed to the driver |
| `stopReason` | `resultLimit`, `cancelled`, `failed`, `outcomeUnknown`, or null |

`completed <= attempted <= planned`; an attempted statement without success is
neither completed nor part of the unattempted suffix. An invocation does not
prove that Snowflake or another server received SQL. A successful call is not
proof of transaction commit, complete result retention, or verified data effects.
A read whose retained result is truncated can still have a successful driver
acknowledgement. Existing result retention labels remain authoritative for the
retained snapshot.

Only the ordinary SQL action supplies these counts. Dedicated data-change,
process-control and typed Snowflake mutation outcomes retain their existing
contracts. Missing summaries in older history or other actions mean accounting
was not recorded; never infer successful execution from `statementCount`.

## Stopping and recovery

When a classified read reaches its result limit, the executor stops the script.
This stopping rule applies to classified reads; statements with possible writes
retain their existing outcome handling. If a suffix remains, terminal status is
`stopped`, with `resultLimit` and an explicit count of statements not attempted. A final or single truncated read can
finish with `completed` status and a `resultLimit` summary because no suffix
remains. A later cancellation request cannot erase an earlier acknowledged
statement. Failed and unknown outcomes remain distinct from a result-limit stop.

The change neither resumes a suffix nor retries a statement. It does not commit,
roll back, reconnect or change transaction mode automatically. A skipped COMMIT
can leave a transaction open; the observed transaction state and explicit user
decision govern recovery. Unknown outcomes still require investigation before
retrying. Counting is metadata only and does not enable SQL-text persistence.

## Reproducible acceptance

Use a fresh disposable local PostgreSQL service and isolated application data.
The Rust integration fixture injects small `ResultStore` budgets without changing
production limits or exposing a configuration setting.

1. A classified read exceeds a row limit; a following read advances a
   session-local sequence. Confirm the sequence remains untouched, the first
   result is truncated, status is `stopped`, and counts are 2 planned, 1 attempted,
   1 completed, 1 not attempted. Repeat with a byte limit.
2. A nontruncated two-read control executes both statements. Confirm the same
   sequence advances and the final summary reports 2 attempted/completed.
3. An error, cancellation or interrupted connection retains the successful prefix
   and reports the unattempted suffix honestly. Do not infer remote outcomes from
   local invocation counts.
4. A transaction whose COMMIT is skipped remains open until explicit rollback or
   commit. Confirm independently with the actual database driver.
5. Exercise the editor, terminal summary, result grid, history and history search
   in the browser using the real local database bridge. A fresh Studio process
   must read the same persisted summary; legacy records show unknown accounting.

Local PostgreSQL evidence qualifies shared executor behavior. It does not qualify
Snowflake account behavior, external-browser SSO, million-row Snowflake handling,
normal packaged daily usage or complete DataGrip replacement readiness.
