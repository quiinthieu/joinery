# Snowflake qualification SQL contract and offline ledger

This local increment shares production SQL definitions and expands the proposed
[existing-schema lifecycle](snowflake-existing-schema-qualification.md) into an
exact ordered ledger. It does not connect to Snowflake, enable an account runner,
enforce admission, or authorize account activity. The complete replacement Goal
and all unqualified DataGrip workflows remain required.

## Shared production definitions

[sql-contract.json](../snowflake-helper/src/sql-contract.json) is the source of
truth for bootstrap identity, version and typed BEGIN/COMMIT/ROLLBACK SQL. The
helper loads a validated frozen contract; Rust's build script validates the same
file and generates constants. The domain transaction SQL, adapter version read,
and prior helper-only qualification definitions consume those values. The two
new contract resources participate in preparation, bundle checks, runtime source
hashes and actual-helper fixture copies. Invalid or stale resources fail closed;
there is no caller-selected contract path.

The adapter's immutable `ReadSpec` factories own SQL, empty binds, semantic kind
and retention ceilings. Production calls and the offline compiler share context
and session-sample order, schema/relation queries, exact lookups and three-read
column inspection. Parsers and the existing identity checks still run between
dependent reads. Named specs do not make observations atomic against other actors.

Retention ceilings are distinct from expected result rows. Exact lookups retain
up to three rows; schema reads retain the 1,001-row sentinel; relation batches
retain up to 10,001 rows/2 MiB; inspection retains up to 1,001 rows/1 MiB and
zero-row resolution retains zero rows. Version and normal readback requests keep
the application's existing 1M-row/8 GiB request ceilings. The fixed synthetic
readback SQL separately limits its result to two rows. Resource admission limits
must still be bound and enforced by the future runner.

## Closed expansion

`Plan::expand_physical_ledger` accepts only a validated ordered `SchemaCensus`.
The existing sole plan factory generates the new target under
`SANDBOX.BILLIE_TESTING`; callers cannot supply SQL, another target or a serialized
plan. The expansion includes all 32 phases and **386 + 7S** SQL invocations:
393 for one schema through 610 for the maximum 32. Every phase's expanded count
must equal its declared budget before expansion succeeds.

Each entry records one-based global/phase ordinals, operation kind, exact SQL,
empty binds, query-frame retention bounds where applicable and its rejection
suffix allowance. Canonical bytes/hash bind the parent plan hash, exact census
and every entry. This is local tamper detection, not execution authentication.
The ledger covers application SQL; SDK-internal authentication activity is a
separate disclosure and approval concern.

Definitive database rejection has route-specific continuation:

| Rejected SQL | Maximum permitted read-only suffix within that active call |
| --- | --- |
| Normal Studio readback SELECT | Two five-read session samples, ten reads |
| Reviewed DDL statement | One five-read session sample |
| Reviewed DML or typed transaction boundary | Zero |
| Any metadata/observer read | Zero after that read |

Each sample orders extended context, quoted-identifier policy, AUTOCOMMIT,
TRANSACTION_ABORT_ON_ERROR and current transaction. Continuations can stop at
any prefix; a failed sample read never permits the rest of that sample or the
second sample. Cancellation, connection closure or another rejection can stop
earlier. No allowance crosses a phase, authorizes replay, adds rollback, or
admits cleanup SQL. A definitive rejected statement is distinct from unknown
delivery, which must retire without replay.

## Remaining integration gates

The next increment must bind these entries to the existing
[transport and metadata attachments](snowflake-qualification-attachments.md).
Freeze the first CREATE schema census in memory; before the next dependent write,
asynchronous admission must durably record the census-bound expansion. Enforce
phase ownership, result validation and original receipt persistence before
advancement, plus frozen lifecycle context/traits/creation markers. Metadata
callbacks remain synchronous and cannot perform I/O or reenter the adapter.

A separate closed grammar must bound connect, credits, discard, cancellation and
teardown; a SQL ledger alone does not constrain those frames. Source/runtime/
binary/private-profile binding, one-use consumption, durable intent/outcome
journaling and a fixed production-route runner remain unimplemented. Complete
393/610-entry synthetic execution routes and interruption/rejection/recording
failures must pass before preparing a fresh executable account plan. The old
database-creation approval remains consumed.

Commands and results are recorded in the
[verification ledger](replacement-verification.md). Offline expansion and
synthetic helper dispatch do not establish account write/transaction effects,
normal packaged SSO, million-row account performance or replacement readiness.
