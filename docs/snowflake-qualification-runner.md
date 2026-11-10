# Snowflake qualification execution boundary

Status: implemented and independently reviewed with local synthetic tests.
**No execution approval or account qualification is granted by this document.**
The historical two-SELECT canary is complete. Its approval does not authorize this
distinct workflow, additional metadata, browser authentication or possible charges.

This helper-direct tool implements exactly the
[`session-options-scalar-state-v1` ledger](snowflake-qualification-manifest.md):
one connection, 24 potential fixed statement executions, current-database schema
names capped at 1,000, coherent session settings/state and synthetic scalar
values. It performs no existing application-table reads, schema switches, writes,
DDL or transaction commands. It does not exercise the normal application or its
automatic browsing/completion SQL. Those require separate complete approvals.

## Execution plan and approval

`scripts/snowflake-qualification-runner.mjs` wraps the unchanged offline-v1
ledger in a distinct `joinery-snowflake-qualification-execution-plan-v1` envelope.
Its SHA-256 binds the exact private ledger, profile, run token, admission window,
source/runtime evidence, runner/child/framing/protocol/results source hashes and
physical limits. The embedded offline ledger stays `offlineOnly` with `runner:null`;
it is not an execution grant. The CLI accepts no arbitrary SQL, executable,
environment, runtime, authentication root or timeout override.

All input/plan/approval files must be regular, owner-private 0600 files under this
repository's ignored `.cache` directory. Plans and receipts are emitted
exclusively under the existing owner-private 0700
`.cache/snowflake-qualification` root. Do not print or commit their profile,
rendered SQL, identifiers or binds. The runner prints only safe hashes, limits,
fixed codes, counts and redacted outcomes.

With the repository's pinned Node 24 toolchain, the offline commands are:

```sh
node scripts/snowflake-qualification-runner.mjs --input .cache/REQUEST.json
node scripts/snowflake-qualification-runner.mjs --input .cache/REQUEST.json \
  --emit .cache/snowflake-qualification/PLAN.json
node scripts/snowflake-qualification-runner.mjs --input .cache/REQUEST.json \
  --plan .cache/snowflake-qualification/PLAN.json
```

These modes validate the current prepared runtime and source templates but never
spawn the helper, authenticate, allocate authentication state or execute SQL.
Prepare the pinned runtime offline using the documented repository cache when
necessary; runtime preparation itself is not permission to connect.

Before execution, obtain fresh explicit owner approval naming the execution-plan
hash and admission window, external-browser SSO, current-database schema-name
disclosure and possible warehouse/cloud-service charges. Only then may the
operator record that consent in a private approval file with exactly:

```json
{
  "format": "joinery-snowflake-qualification-approval-v1",
  "executionPlanSha256": "EXACT_REVIEWED_EXECUTION_PLAN_SHA256",
  "scope": "one-helper-session-externalbrowser-24-read-statements-schema-names-possible-billing",
  "maximumAttempts": 1
}
```

The file records operator-supplied consent; it cannot authenticate a conversation
or replace the owner's approval. The CLI deliberately has no approval-creation
command. After consent, the one permitted execution is:

```sh
node scripts/snowflake-qualification-runner.mjs --input .cache/REQUEST.json \
  --plan .cache/snowflake-qualification/PLAN.json \
  --approval .cache/snowflake-qualification/APPROVAL.json --run
```

The deterministic `consumed-<execution-plan-hash>.json` marker is created
exclusively, its file bytes and containing directory are synced, and sources and
runtime are revalidated before authentication can start. The marker is never
removed, including on local launch/authentication failure. Copying the approval
file or removing the result receipt does not restore authority. A changed plan,
expired window or failed attempt needs a newly reviewed approval; no old canary
or earlier attempt is inherited.

## Dispatch, limits and cleanup

The supplied-child engine serializes physical writes through callback and drain,
reserves permission before writing, checks both decoder/protocol failure latches,
and stops further statements after the first failure. Strict decoding counts all
physical bytes, including whitespace and incomplete frames. Limits include
1 MiB/frame, 8 MiB inbound framing, 4,096 frames and 64 KiB discarded/count-only
stderr. Result/row/credit and operation deadlines remain bound by the ledger.
An independent monotonic 360-second workflow deadline applies within the at-most
ten-minute admission window; both absolute window edges are checked at admission.
Local deadlines do not promise that remote work stopped.

Only an explicit owner interruption may request the one identified-query cancel.
Timeouts and malformed responses do not invent cancellation authority. Cleanup
is bounded through close, stdin EOF, TERM and KILL; receipts distinguish reserved,
attempted and completed writes, acknowledged logout, process exit and closed
stdio. Known statement outcomes survive later malformed input or source drift.

The outer runner uses the fixed prepared Node/helper and only its fresh private
authentication-state environment. It holds directory identity authority, removes
only its original authentication root after actual reaping or a settled failed
OS launch, and preserves replacement/unconfirmed roots. Spawn return, actual
process launch, no-child confirmation and actual reaping are distinct evidence.
Emergency cleanup closes owned pipes even if the process already exited. Source
drift after execution or failed receipt persistence fails qualification while
preserving known outcomes. No cleanup result establishes remote rollback,
independent server cancellation or an account-wide session inventory.

These are local admission and evidence boundaries, not an OS sandbox, a currency
ceiling or a physical HTTP limit. Source checks are pathname-based before/after
validation; they do not defend against hostile mutation by the same operating
system user. SDK authentication, renewal, logout and result retrieval can perform
additional network work. Actual account/runtime acceptance and owner replacement
acceptance remain open.

## Reproducible local verification

Run `node --test scripts/snowflake-qualification-runner.test.mjs` and the combined
qualification suite. Synthetic tests cover the full 24-statement production-helper
workflow in an actual owned subprocess, drift, one-use consumption, directory-sync
failure, permissions/symlinks, evidence collisions, actual failed OS launch,
replacement-root preservation and pre-ownership emergency reaping. These fixtures
use synthetic SDK responses, not actual Snowflake behavior. See the
[verification ledger](replacement-verification.md) for exact commands, counts,
review repairs and actual CLI dry-run evidence.
