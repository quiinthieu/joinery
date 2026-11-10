# Offline qualification results and protocol

These pure modules support the fixed
[24-SQL manifest](snowflake-qualification-manifest.md). They perform no I/O,
authenticate no connection and grant no execution authority. The manifest remains
`offlineOnly` with `runner: null`. A complete synthetic transcript does not qualify
the Snowflake account or application.

## Result validation

`scripts/snowflake-qualification-results.mjs` provides a shared result budget,
incremental query collectors, exact fixed-template parsers and coherent observation
comparisons. One complete workflow admits at most 1,000 schema names plus 23
single-row results. A 1,001st schema is a failing sentinel; it cannot qualify a
truncated inventory. The aggregate attempt budget is 1,024 rows / 4 MiB of result
content, schema content is at most 2 MiB, and each other result is at most 64 KiB.
Column descriptors and uninterpreted SHOW fields count toward content limits.

Parsing requires complete, untruncated results, exact expected columns and types,
bounded well-formed text and the appropriate NULL semantics. Session/transaction
identifiers stay opaque strings. Context, quoted-identifier policy, AUTOCOMMIT and
abort-on-error observations are compared across their fixed brackets; a nonnull
transaction ID does not establish transaction usability. Returned observations
contain private identifiers and values and must never be logged.

## Protocol API and ownership

`createQualificationProtocol({ manifest, currentInput, generation })` in
`scripts/snowflake-qualification-protocol.mjs` independently verifies the pure
manifest contract. It exposes:

- `reserveNext()`: reserve the next immutable ledger frame, or return `null` while
  waiting for hello, a command or its outstanding controls.
- `reserveCredit()`, `reserveCancel("ownerCancellation")`, `reserveClose()`:
  reserve explicit, separately owned control frames. No method writes a frame.
- `receive(decodedFrame, { bodyBytes })`: validate one decoded frame and its
  caller-supplied actual physical body-byte count. It never dispatches follow-up
  SQL or sends credit automatically.
- `snapshot()`: fixed-code status, counters and demand without connection values.
- `outcomes()`: SQL-only evidence installed at reservation, separating request
  progress, remote outcome and result validation. A reservation starts unknown;
  a valid success acknowledgement remains succeeded if later validation fails.
- `observations()`: private typed values for subsequent in-memory validation.

Request IDs and ownership are installed before reserved frames escape to a
writer. Exactly the regenerated ledger is admitted, serially; no caller SQL can
be substituted. A semantic or protocol failure permanently prevents further
ledger admission. Owner cancellation likewise prevents more SQL. Explicit local
cleanup remains a caller responsibility.

Only schema streaming receives extra credit: one outstanding request, at most
1,000 replenishments, each after a consumed nonempty batch. Control completion
cannot replace the original SQL terminal outcome. A late, unaccepted credit
rejection is benign only after the matching original command validly finished.
Inbound framing is bounded to 4,096 frames / 8 MiB including four-byte headers;
individual bodies are at most 1 MiB. Outbound fixed-ledger, credit and cancellation
reservations have separate bounds.

## Required future runner boundary

These modules deliberately do not implement physical framing, duplicate-JSON-key
or UTF-8 rejection, trustworthy raw byte counts, serialized pipe writes and
backpressure, time/window checks, approval consumption, source-file/runtime
verification, process/environment selection, authentication, EOF/TERM/KILL,
reaping or identity-checked authentication-root cleanup. The future executable
manifest must bind that runner and these modules' hashes and limits. It must stop
queued writes after protocol failure and reserve permission before physical
dispatch; a reservation is not proof of submission or success.

Approval must be requested only after this executable boundary is implemented,
reviewed and locally tested. Historical canary approval does not apply. Lost
responses, process exit, local cancellation or cleanup alone do not establish
remote cancellation, logout, rollback or a rejected statement.

## Local checks

Run with the repository's Node 24 toolchain:

```sh
node --test scripts/snowflake-qualification-results.test.mjs \
  scripts/snowflake-qualification-protocol.test.mjs
pnpm exec prettier --check scripts/snowflake-qualification-results*.mjs \
  scripts/snowflake-qualification-protocol*.mjs
```

The tests use synthetic results and an in-process production helper with a
synthetic SDK and real split framing. They exercise the complete fixed ledger,
schema ceiling, shared budgets, identity/order/control association, cancellation,
late credit and permanent semantic stopping. No account, real SDK authentication
or subprocess behavior is established. Current review and exact outcomes belong
in the [verification ledger](replacement-verification.md).
