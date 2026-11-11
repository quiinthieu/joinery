# Internal Snowflake qualification attachments

This increment provides transport and metadata plumbing for the proposed
[existing-schema lifecycle](snowflake-existing-schema-qualification.md). It does
not enable an account runner, grant permission to connect, or establish any live
Snowflake capability. The normal desktop connection path supplies no attachment.
The implementation is crate-private, Unix-only, and requires `test-harness`;
there is no IPC command, stored profile option, or environment switch for it.

## Connection and failure ownership

`QualificationAttachment` binds one transport attachment and one metadata
attachment to a shared first-failure latch. Studio's internal connection entry
accepts only Snowflake external-browser authentication without a tunnel, supplied
secrets or password storage. It installs both attachments before helper startup.
Transport resource failures revoke metadata; metadata rejection revokes further
transport admission. A failed attempt cannot be reused.

The connection path retains its check handle through setup, then rechecks after
the final configuration, attempt-registry and session-map lock waits and the
profile/cancellation checks before publishing a session. An exited helper is also
rejected. Attempts discarded before attachment settle their unused counters; a
claimed Launch owns settlement thereafter.

Every synchronous `check` must use in-memory state only, perform no I/O and
never reenter the adapter, Studio or connection-attempt registry. Publication
can invoke it while holding the registry/session-map or cache/authority locks.
Durable intent recording belongs in asynchronous `admit`, outside those locks.

## Physical transport boundary

The single writer presents the exact final request envelope, wire bytes, resolved
target, operation ticket, impact and generation to asynchronous admission. The
policy may durably record intent without holding the process-state mutex. An
admitted intent remains consumed if cancellation, replacement or a completed
target later prevents sending.

The writer revalidates ownership after admission and checks policy on every
physical write poll, including polls after mutex contention and pipe backpressure.
Policy callbacks run outside the process mutex. A synchronous write poll owns
that mutex so retirement cannot incorrectly classify accepted bytes as never
submitted. Zero accepted bytes retains `NotSubmitted`; partial writes remain
`OutcomeUnknown`. Acknowledged original mutations retain their typed outcomes
even if later observation or transport accounting fails. There is no replay.

The attachment counts actual stdout bytes from the handshake onward, complete
frames, stderr bytes, and outbound write progress. Frame limits are checked before
body allocation. Stderr is concurrently drained and discarded; reports contain
numeric counts and dispositions rather than raw output. A one-byte sentinel
detects byte-cap overflow. Default ceilings are 64 MiB stdout, 32,768 complete
frames, 1 MiB per frame and 64 KiB stderr; the future reviewed runner must select
and bind its own permitted limits.

`TransportAttachment::wait` proves attached counter/writer tasks have exited,
not merely that a terminal disposition was recorded. It does **not** prove child
reaping or removal of the original private authentication directory. Those remain
separate cleanup-observer requirements. The existing pinned SDK submission guard
remains mandatory.

## Metadata boundary

Observers receive validated schema names in original wire order as well as the
production parser's sorted names, and parsed relation records retaining creation
and last-DDL markers. Sites distinguish schema census, session options, relation
batches/snapshots, exact relation checks, schema selection and completion.
Callbacks run synchronously, perform no I/O and must not reenter the adapter,
Studio or connection-attempt registry. In particular, their `check` callbacks
are in-memory only because cache/authority or publication locks can be held.

Rejection stops dependent SQL and successful result publication. Final checks
follow the relevant cache/authority lock waits before installing snapshots or
grants. The read-only operation wrapper also checks before returning success.
Mutation-specific wrappers retain their original acknowledged outcomes; the
metadata gate must not turn a known SQL acknowledgement into a fabricated unknown
delivery.

## Verification and remaining work

Reproducible commands, red/green regressions, independent review and current
source/artifact scope are recorded in the [verification ledger](replacement-verification.md).
Actual framed-helper tests use a synthetic SDK and disposable local state. They
are not account qualification.

The following remain required before an executable approval request:

1. Export the production bootstrap SQL definitions and implement the exact closed
   physical SQL order/count ledger, including bounded post-rejection observations.
2. Freeze and enforce the raw census and created-object markers using these
   observers. Callback availability alone does not enforce a lifecycle policy.
3. Bind source, runtime, binary, private connection and generated target identities
   to a one-use reviewed execution envelope and durable intent/outcome journal.
4. Implement and independently qualify the fixed production-route runner,
   including complete 393/610-execution synthetic routes, rejected suffixes,
   interruption, partial delivery and failed local recording.
5. Prepare the concrete plan for a fresh table under `SANDBOX.BILLIE_TESTING`
   and obtain its account activity and possible-billing approval before running.

Normal packaged SSO, real Snowflake mutation effects, cancellation/recovery,
million-row account results, the remaining DataGrip workflows and owner hands-on
acceptance remain open. Nothing in this internal plumbing establishes replacement
readiness.

## Next contract increment

The next bounded implementation should share production-used SQL definitions
before building the closed ledger. Bootstrap currently executes the helper's
six-function identity SELECT, then the adapter's `SELECT CURRENT_VERSION()`.
The helper's typed transaction SQL also needs an authoritative contract consumed
by both JavaScript and Rust; duplicating literals in a test ledger would not
prove the production request's meaning.

| Responsibility | Expected result |
| --- | --- |
| Helper contract | One shared, source-pinned definition for bootstrap identity and typed transaction SQL, consumed by the helper and embedded Rust |
| Adapter metadata contract | Named production-used read specifications and ordered sequences, including version, context, session samples, DDL checks and inspections; fixed SQL, empty binds and retention bounds |
| Offline compiler | Expand the existing 32 phases from those definitions and a validated census into exact ordered physical entries, preserving `386 + 7S` and no caller-selected SQL/target |
| Lead integration | Bind entries to physical admission, metadata observations and phase/receipt ownership; validate complete routes before designing an executable approval envelope |

The first CREATE schema callback should freeze its candidate census in memory.
Callbacks cannot perform disk I/O or reenter the adapter, Studio or connection
registry; synchronous checks must remain in-memory. The next asynchronous
admission must durably record the census-bound expansion before any dependent
request is physically written. Likewise, the SQL ledger needs a separate closed
grammar for credits,
cancellation and teardown; statement counts alone do not bound those frames.
After a statement rejection, only the active call's frozen read-only suffix may
finish. No phase advancement, replay or cleanup SQL is permitted.

This is a planned increment. Shared definitions, expanded enforcement and their
production-route regression evidence are not implemented by the attachment APIs.
