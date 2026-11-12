# Reviewed console query limits

## Scope and status

The 12 November 2026 increment replaces the fixed console delivery deadline with
an explicitly configured, reviewed execution and result-delivery limit. Local
synthetic verification passes, including the actual browser form, native review,
results and full application restart. The [verification ledger](replacement-verification.md)
records source-scoped checks and corrected fixture failures. Normal SDK-bearing
ARM64 packaging from clean `72ee3e4` passes resources, strict ad-hoc signature and
network-guarded helper startup with zero authentication/SQL. The app remains local
and uninstalled. This does not establish packaged native UI acceptance, account
qualification, unlimited execution, or complete DataGrip parity.

DataGrip's current [Run queries documentation](https://www.jetbrains.com/help/datagrip/run-a-query.html)
describes completion notifications for long-running database queries. Its HTTP
client timeout documentation is a different feature and is not evidence for a
database query limit. Snowflake documents the connection-level SDK
[`timeout` option](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-options)
separately from statement execution. The pinned SDK implementation must also be
checked: documentation defaults are not assumed to describe our exact packaged
version. Research was refreshed on 12 November 2026.

## Configuration and immutable review

An opted-in external-browser console profile has **Console query limit
(seconds)**, an integer from 1 through 86,400. An absent legacy setting means
65 seconds. Invalid explicit settings must fail validation, rather than silently
choosing another budget. Saving a changed profile closes its existing sessions;
the new setting applies on reconnect.

Native preparation captures the effective limit in the trusted immutable action,
its fingerprint, and the returned review. The review displays the server-returned
value. Start and each unit's admission must verify the captured session and
current profile still agree. An existing approval cannot authorize a changed
limit, and a running script cannot silently acquire a different limit between
units. Request recovery keeps returning its original admission decision.

The selected limit runs from unit admission through execution and result
delivery/drain. It is not a strict upper bound on the time until Joinery returns:
durable acknowledgement recording and bounded control/terminal settlement may
continue after expiry. Expiry must be checked again after these awaits before
publishing more results or admitting a suffix. Recording original evidence must
not be cancelled to make a timer appear precise.

For console connections only, native code derives an SDK HTTP request budget
of `max(90,000, seconds * 1,000 + 5,000)` milliseconds. This is a closed derived
setting, not a free-form transport override. Ordinary connections retain their
existing payload. Authentication/bootstrap, framing/write, acknowledgement
recording, credit/cancellation and terminal-settlement bounds remain independent.
The increment does not rewrite server session settings or add retries, observer
SQL, async execution, automatic reconnect or transaction settlement.

## Expiry and cancellation

Expiry retires the physical console and stops the unattempted suffix. It does
not prove Snowflake cancelled a submitted query, rolled back a transaction, or
avoided charges. An original acknowledgement or uncertain outcome remains in its
SQL-free receipt. Never infer safe replay from a local timer or transport error.
Explicit cancellation must remain responsive during a long selected limit, with
the existing bounded control/settlement behavior and truthful original outcomes.

## Required verification

- Persist/reopen a legacy default and a configured value; reject invalid direct
  IPC and persisted values, including malformed restores.
- Bind the selected limit into the native fingerprint and rendered review.
  Reject profile changes after preparation, approval, and between units without
  issuing a suffix or additional observer SQL.
- Complete a deliberately quiet synthetic unit beyond the former 65-second
  limit with a longer configured budget. With a short budget, retire it and
  preserve original evidence and untouched suffix.
- Exercise explicit cancellation, interrupted transport, and an acknowledgement
  recorder that crosses the selected deadline. Separate execution expiry from
  safe settlement latency; never invent non-execution or rollback.
- Use the actual browser profile form, native review, results and restart path.
  Test the real helper protocol with a synthetic SDK, then separately qualify
  long queries and cancellation against an isolated approved Snowflake scope.

The finite 24-hour setting, server-enforced limits, actual long-query behavior,
million-row account results and packaged macOS acceptance remain explicit gates.
