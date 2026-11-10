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

## Strict physical framing

`scripts/snowflake-qualification-framing.mjs` exports
`createQualificationDecoder({ onFrame })`. `push(Buffer | Uint8Array)` delivers
each validated `{ frame, bodyBytes }` synchronously and returns a count;
`end()` validates EOF. The required callback returns `undefined`. A valid success
acknowledgement is delivered before examining a malformed suffix in the same pipe
read, preserving the known outcome. Callback failure, reentry, malformed input or
resource exhaustion permanently stops decoding. A future runner must retain both
its protocol failure and decoder failure and prevent further writes on either.

The decoder rejects malformed UTF-8, BOMs, duplicate decoded JSON keys, unpaired
surrogates and numbers outside the exact safe-integer subset. Decimal database
values remain strings. It enforces the protocol's 1 MiB frame / 8 MiB physical
input / 4,096 frame ceilings, plus depth 16, 10,000 value nodes and 4,096 entries
per container. Raw whitespace and partial headers/bodies count toward byte limits.
It allocates one checked body buffer per frame and copies split input linearly;
input constructor/species hooks cannot substitute copying behavior.

## Supplied-child lifecycle

`scripts/snowflake-qualification-child.mjs` takes one valid supplied child's
lifecycle through serialized physical writes, callback/drain backpressure,
per-stage/group deadlines and bounded close/EOF/TERM/KILL cleanup. It reserves
each command before writing and checks both decoder/protocol latches before
queued writes. Only an explicit owner interruption may request an identified
cancel; timeouts do not invent one. Known SQL acknowledgement evidence survives
later malformed input or interrupted cleanup.

Both absolute admission-window edges are checked at every new admission. A
separate fixed monotonic 360-second workflow deadline applies independently of
the offline ledger's maximum ten-minute admission window. A reduction-only test
seam cannot increase either deadline. Receipts distinguish write reservation,
attempt/completion, stdin-end request/local finish, logout acknowledgement,
process exit and stdio closure. They disclose unknown local cleanup and make no
remote rollback/cancellation claim.

This module selects no process/runtime, reads or writes no files, consumes no
approval and has no CLI. Invalid configuration rejects before lifecycle
ownership; its caller must retain and reap that child. The executable boundary
must separately own its fixed runtime/environment, approval and state roots.

## Executable boundary requirements

The pure result/protocol/framing modules grant no execution authority. The
supplied-child lifecycle still does not implement approval consumption,
source-file/runtime verification, process/environment selection or
identity-checked authentication-root cleanup. The executable
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
  scripts/snowflake-qualification-protocol.test.mjs \
  scripts/snowflake-qualification-framing.test.mjs
pnpm exec prettier --check scripts/snowflake-qualification-results*.mjs \
  scripts/snowflake-qualification-protocol*.mjs \
  scripts/snowflake-qualification-framing*.mjs
```

The tests use synthetic results and an in-process production helper with a
synthetic SDK and real split framing. They exercise the complete fixed ledger,
schema ceiling, shared budgets, identity/order/control association, cancellation,
late credit and permanent semantic stopping. No account, real SDK authentication
or subprocess behavior is established. Current review and exact outcomes belong
in the [verification ledger](replacement-verification.md).
