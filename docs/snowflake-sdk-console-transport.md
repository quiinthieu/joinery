# Reviewed Snowflake console transport foundations

The pure compiler and private helper transport implement part of the
[reviewed-console design](snowflake-sdk-console-scripts-design.md). They do not
expose a new application command or grant a profile permission. Retained console
ownership, review UI, native admission, durable run receipts and end-to-end
acceptance remain required before this is a usable daily workflow.

## Compilation boundary

`safety::snowflake_script::compile` accepts original SQL and typed scalar inputs.
Parsed mode validates the complete batch before returning ordered units, checks
each split independently and retains original UTF-16 spans. Selected mode keeps
the exact selection as one server-validated submission; it does not claim that
the local parser understands a scripting body or proves its statement count.
Anonymous parameter ordinals span the complete script. Named values are shared
inputs; each occurrence becomes an ordered wire bind without interpolation.

The compiler bounds source text to 1 MiB, units to 100, each unit's original and
rewritten SQL to 256 KiB, parser tokens to 65,536 and each unit to 1,000 binds.
Existing parameter limits also apply. Separate per-unit and aggregate canonical
JSON budgets cover escaping and repeated values; the eventual application owner
must additionally bound its complete review/IPC envelope. Canonical verification
recompiles original text and inputs and compares every unit's SQL, order and typed
bindings. Rewrite maps describe diagnostics, not execution authority.

## Private helper operation

The closed `reviewedSql` payload contains only `sql`, `binds`, `expected`,
`credits`, `maxRows` and `maxBytes`. `expected` pins authenticated user/session,
not mutable role/database/schema/warehouse. SQL remains a primitive string;
bindings remain scalar null/boolean/string values under the existing bounds.
The Rust supervisor admits this operation only as potentially mutating.

The helper supplies `MULTI_STATEMENT_COUNT=1` on every physical execute request.
Callers cannot override that parameter object. Immediately before execution, it
vetoes the exact pinned SDK `Util.isPutCommand`/`isGetCommand` dispatch on the
unchanged wire string. This includes the SDK's prefix behavior; it is a client
file-transfer veto, not a general SQL side-effect classifier. PUT/GET and import
remain required separate workflows with local-file permission.

Successful original execution publishes a sticky `statementAcknowledged` event
associated with the sole registered SDK statement. Before result draining, the
helper captures the statement identity and passive successful-response context:
database, schema, role and warehouse. Missing, malformed or throwing getters
produce individual unknown fields. Startup configuration is not a substitute.
These fields do not establish transaction state, AUTOCOMMIT, search path,
affected-row counts or independently verified commit.

The supervisor validates event order, closed fields and statement identity and
retains that acknowledgement in the operation/report. A later query-ID upgrade
updates the same identity. Cancellation, malformed later events, EOF or result
drain failure cannot turn the retained original acknowledgement into an
unacknowledged outcome. Uncertain or post-acknowledgement failures retire the
session conservatively. There is no automatic retry or reconnect.

Results use the existing bounded credit-controlled columns/rows protocol.
Successful terminal counters and identity have a closed shape and require the
original acknowledgement. Discarding retained rows does not cancel execution or
prove commit. Cancellation acknowledgement remains separate from original SQL
outcome.

## Authority and verification boundaries

Adding this private operation does not change application profile policy, native
commands or the closed account-qualification ledger. Qualification wire tests
must reject substitution of `reviewedSql` for every existing ledger entry. The
new route also invalidates the disposable setup bootstrap, as any unrelated
operation must.

Compiler, helper, actual pinned-SDK loopback and actual-child supervisor checks
are recorded in the [verification ledger](replacement-verification.md). Loopback
tests exercise the SDK against a synthetic local service; they do not establish
Snowflake account execution, transaction effects, privileges or normal packaged
SSO. No account requests, marker-store access or owner-profile reads are needed
for this increment.
