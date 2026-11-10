# Snowflake SDK explicit transactions

**Status: explicit adapter/application controls implemented and locally qualified (`929ae06`); actual-account effects and acceptance open.**
Reads, writes, schema changes and explicit transactions remain required for the
owner's DataGrip replacement. This document records the transaction contract and
remaining qualification;
it does not qualify an account or authorize billable work, DML or DDL. The earlier
two-SELECT canary is complete and grants no further authority.

## Database contract

Explicit transactions and AUTOCOMMIT mode changes are different actions. Keep the
SDK mode dropdown disabled until its own reviewed workflow exists: assigning
AUTOCOMMIT can implicitly commit an active transaction, including when the value
does not change. Explicit BEGIN can operate while AUTOCOMMIT is enabled. DDL also
has implicit-commit behavior and needs a separate reviewed path; initially reject
DDL while a transaction is open. A failed DML statement alone does not prove that
the transaction is unusable. See Snowflake's
[transaction rules](https://docs.snowflake.com/en/sql-reference/transactions).

With TRANSACTION_ABORT_ON_ERROR enabled, an aborted transaction can reject
observer statements until COMMIT or ROLLBACK closes it. Requiring a successful
observer before every control would prevent recovery. A dedicated same-session
rollback must therefore accept retained backend ownership without mandatory
pre-read SQL. See the
[parameter contract](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error).

## Transport slice

Add a strict typed helper operation with exactly an action (`begin`, `commit` or
`rollback`) and expected principal/session identity. The helper accepts no caller
SQL or identifiers and compiles exactly one of `BEGIN TRANSACTION`, `COMMIT`, or
`ROLLBACK`. It checks the established identity pin and matches supervisor impact
to action before admission. Preserve its one-submission guard through result
drain and terminal reporting.

Install a distinct boundary acknowledgement before callbacks or result draining.
The supervisor retains action and statement association separately from generic
request progress. An acknowledged command does not by itself establish committed
or rolled-back database effects. Known acknowledgement must survive a later
readback/drain error, cancellation or malformed trailing frame. Lost responses
remain unknown and must never be replayed automatically.

The transport slice alone did not enable application controls. The dependent
application increment now adds reviewed buttons while arbitrary public SQL
transaction statements, DML, DDL and AUTOCOMMIT changes remain blocked.

The helper now accepts only the typed action and expected identity, rechecks the
pin at dispatch, and compiles one fixed statement. `boundaryAcknowledged` is
retained before result draining; the supervisor validates its exact action and
registered statement identity. Post-ACK failures preserve `StatementSucceeded`
without claiming `Committed` or `RolledBack`. A definitive database rejection
retains its failure classification and allows a same-session recovery command.
Local supervisor/process and actual SDK loopback evidence is recorded in the
[verification ledger](replacement-verification.md#sdk-transaction-transport-10-november-2026).

## Dependent adapter and application slice

The explicit-boundary increment includes an SDK-specific policy prerequisite
with typed admission. Use the existing SafeMode
field: permit owner-selected policies only for explicit external-browser SDK
profiles; keep local-application OAuth and missing-authentication legacy profiles
ReadOnly. Preserve HTTPS/443, verified TLS, no SSH and no stored-password rules.
No persistence migration or automatic elevation of existing profiles is needed.

New SDK profiles must still default to ReadOnly. Switching SDK to OAuth resets
ReadOnly; switching back must not restore an elevated policy automatically.
Group changes preserve ReadOnly and normalize explicitly chosen non-ReadOnly
production policies to PasswordGated. Form copy must describe authorization for
explicit boundary actions without implying that DML or DDL has been enabled.
Keep the public adapter's one-classified-read gate and all editing/DDL/mode
capabilities unchanged, even for a non-ReadOnly SDK profile. Land the profile
gate with its usable typed workflow, rather than exposing elevated policy alone.

Separate explicit-control capability from the existing transaction capability
that also enables the AUTOCOMMIT dropdown. BEGIN requires fresh observed idle
authority; normal COMMIT requires matching observed transaction authority. Hold
one console lane and invalidate plans, completion and browsing authority before
dispatch. Publish the adapter's verified state directly through Studio instead
of adding its ordinary ten-read observer afterward.

A normal control can use five baseline reads, one boundary and five final reads:
eleven statement executions. Recovery ROLLBACK uses no mandatory baseline and can
use one boundary plus five final reads. These fixed adapter counts are verified
through synthetic SDK/helper fixtures,
not an account approval or a complete application SQL budget.

After a known boundary acknowledgement, an ordinary observer SQL/permission
failure must retain a restricted same-session recovery state. Do not reuse schema
switching's blanket post-admission retirement rule here. Transport/protocol
uncertainty, abandoned ownership and unacknowledged boundaries still require
conservative retirement. Distinguish command acknowledgement, observed activity,
transaction usability and verified disposition in DTOs, history and UI.

The integration uses a dedicated explicit-boundary capability and
`transaction_control(session_id, {action, revision})` driver route. It returns
structured acknowledged/rejected/unknown command evidence, observed state,
observed/rollback-only/unavailable authority and retirement status. Keep the
complete observation sample and principal/session pin server-side. Authority is
generation/revision-bound, consumed before dispatch, and distinct from context
options so an ordinary options invalidation cannot erase recovery ownership.

When restricted to rollback-only recovery, reject public queries, browsing,
schema switching, BEGIN and COMMIT before helper dispatch. Allow owner-requested
rollback and disconnect. An unsuccessful permitted observer refresh must not
erase that recovery state. Use a transaction-specific RAII lease: known boundary
ACK plus ordinary observer denial preserves restricted recovery; lost responses,
transport/protocol uncertainty and abandoned ownership retire conservatively.

Studio needs a dedicated stored-plan action bound to boundary/revision/session
and the existing one-use authorization policy. Recheck effective SafeMode on
consumption: BEGIN/COMMIT follow ReadOnly, confirmation and production-password
requirements; rollback retains the recovery exemption. Do not route through
legacy commit/rollback methods that discard command evidence, observe again and
can label a command ACK as committed. Publish the returned observation directly
and audit boundary acknowledgement/rejection/uncertainty.

Profile saves currently close matching sessions and discard their plans;
passphrase changes clear plans. Preserve those protections. The dedicated route
must additionally rederive effective policy when consuming its stored action,
match action/revision/session, and recheck session closure after lane acquisition
and immediately before dispatch. Ordinary SQL execution must reject stored
boundary actions so it cannot bypass typed evidence and recovery handling.

## Application scheduling and recovery

`prepare_transaction_control` and `execute_transaction_control` use a distinct
one-use stored plan. The console lane is held through the adapter result. A
failed start audit blocks BEGIN/COMMIT but cannot trap ROLLBACK recovery; final
audit failure cannot erase known command evidence. A concurrent disconnect
returns retained command evidence with retired state and no authority.

The frontend rejects controls while any owning tab has unsettled queries. On
dispatch it advances owning tab revisions to reject delayed earlier query
events, revokes schema authority, invalidates metadata caches and pauses
automatic metadata/health work. After publication, completion remains paused
until the owner chooses **Refresh completion context**. An open relation browser
is closed rather than silently remounted after a boundary. Pending controls
block competing work. Rollback-only state blocks ordinary queries and metadata;
its typed ROLLBACK remains available without normal read/approval prerequisites.

The UI reports **BEGIN/COMMIT/ROLLBACK acknowledged**, rejected or outcome unknown.
It does not infer committed or rolled-back data effects. Unknown/retired sessions
are not reconnected or replayed automatically.

## Required verification

- Exact typed input, identity/action/impact matching and one fixed submission;
  foreign, duplicate, missing and mismatched statement acknowledgements.
- Loss of BEGIN, COMMIT and ROLLBACK responses; acknowledged boundary followed by
  drain failure, observer denial, cancellation or malformed trailing bytes.
- Cancellation before admission versus after dispatch; exact-operation drop,
  cleanup, child exit and no retry/reconnect or outcome rewriting.
- Aborted-transaction recovery without preflight; nullable role/warehouse,
  changed transaction identity, stale authority and cross-tab ownership.
- Backend safety-policy rejection with zero helper dispatch; pending controls,
  late retired results and truthful close/recovery messaging in browser/native UI.
- SDK/OAuth/legacy profile-policy matrix, unchanged transport restrictions,
  ReadOnly restart/backup round trips, production/group normalization and no
  policy elevation when authentication changes. Every permitted SDK policy must
  still reject direct public transaction/DML/DDL/session-setting SQL without
  dispatch. Prepared actions invalidated by profile save, passphrase change,
  disconnect or queued retirement must never reach the helper.
- Separately approved isolated Snowflake BEGIN/DML/COMMIT/ROLLBACK effects, failed
  statements, aborted COMMIT behavior, descriptors and recovery on the actual
  account. Synthetic helpers and loopback SDK fixtures cannot establish these.

An abrupt disconnection can leave a detached transaction. Child reaping and local
authentication-directory cleanup do not prove remote rollback; see
[detached transactions](https://docs.snowflake.com/en/sql-reference/transactions#aborting-transactions).
Meaningful [reviewed DML](snowflake-sdk-dml-design.md) and DDL workflows remain
subsequent required slices, followed by owner hands-on acceptance.
