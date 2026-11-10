# Offline Snowflake qualification manifest

The offline compiler is implemented; local verification is recorded below. It prepares a reviewable account test; it does
not authenticate, execute SQL, inherit the completed canary's approval or qualify
the normal application. The broader
[permission plan](snowflake-account-qualification-plan.md) remains proposed.

The first supported manifest workflow is `session-options-scalar-state-v1`,
through the fixed SDK helper directly. It covers the current observer's SQL and
schema-name disclosure explicitly rather than adding them to an older stage's
allowance. The application route remains blocked until its automatic observer,
completion and browsing requests pass the same reviewed admission boundary.

| Ordered group                                                    | Maximum SQL executions |
| ---------------------------------------------------------------- | ---------------------: |
| Connect                                                          |                      0 |
| Startup `context {}` identity read                               |                      1 |
| Version read                                                     |                      1 |
| Context options: five-read sample, schema list, five-read sample |                     11 |
| Fixed typed scalar read                                          |                      1 |
| State observation: two five-read samples                         |                     10 |
| Close                                                            |                      0 |
| **Total, on one connection**                                     |                 **24** |

The manifest must expand every command into exact SQL and positional binds.
`context {}` contributes one fixed SELECT even though its wire command contains
no SQL text. No health read, schema switch, table inventory, lazy column query,
application-table read, export, polling, retry, reconnect or mutation is included.
Other workflows, including the one-million-row and write/DDL/transaction stages,
must be reported as unsupported until their own implementation and review.

The schema list is limited to the pinned database: at most 1,000 successful
names/2 MiB, with a 1,001-row sentinel that fails qualification. Other results
are limited to one row/64 KiB, with a 4 MiB aggregate result-content budget.
Authentication has a 120-second local wait, SQL a 30-second local wait, options
and state groups a 65-second overall deadline, and admission a ten-minute window.
These are intended local limits, not a currency cap or proof remote work stopped.
The later runner must enforce them; a compiler alone does not.

The offline artifact binds the workflow, supplied run identity/window, validated
connection context, exact templates, source snapshot and runtime manifest.
Canonical hashing and verification must reject tampering or drift. Connection
identifiers appear in the owner-private request input and the emitted manifest;
rendered SQL appears in the manifest. Both files must be `0600`. Emission creates
an exclusive owner-private file under the repository's ignored
`.cache/snowflake-qualification/` root.
Shared output contains hashes, counts, bounds and blockers. Credentials, tokens,
SSO URLs and raw dependency output do not belong in either artifact.

Dry-run must create no helper, network connection, authentication root or
evidence files. Explicit manifest emission is an offline local file operation.
There is no execution command in this compiler increment. The separate pure
[result/protocol modules](snowflake-qualification-protocol.md) now implement
typed validation, fixed-ledger reservation and bounded streaming-credit/control
ownership. A future runner still needs strict physical framing, dispatch and
timing, approval/source verification and process/cleanup ownership before account
approval can be requested. Approval must name the exact manifest hash, run window, route
and possible billing; elapsed time or an older approval is not authorization.

Retirement, child reaping and identity-checked local cleanup remain available
after a failure without extra SQL. Neither local process cleanup nor a cancel
acknowledgement establishes remote cancellation, logout or rollback. The full
[replacement acceptance](replacement-acceptance.md) remains unresolved.

## Offline API and commands

`scripts/snowflake-qualification-manifest.mjs` exports `compileManifest(input)`,
`verifyManifest(manifest, currentInput)`, `canonicalJson(value)` and
`summarizeManifest(manifest)`. The pure API takes explicit source and runtime
fingerprints; supplied hashes alone do not prove the current files. The CLI
independently hashes the fixed repository sources, cross-checks the fixed SQL
templates and validates the prepared runtime resource tree. It does not spawn
that runtime.

The private request has exactly `workflow`, `route`, `runId`, `window` and
`profile`. Set the workflow to `session-options-scalar-state-v1`, route to
`helper-direct`, a nonzero 32-character uppercase hexadecimal run ID and an exact
UTC ISO window (`notBefore`, `expiresAt`) of at most ten minutes. The profile has
exactly `account`, `host`, `username`, `database`, `schema`, `warehouse` and `role`.
The host must match the account's Snowflake hostname. This first workflow accepts
only the configured uppercase unquoted database/schema/warehouse/role identifiers.
It does not accept credentials, arbitrary SQL, approval or executable paths.

Store that request as an owner-owned regular `0600` file under the repository's
ignored `.cache/`. With a prepared runtime matching the current helper source:

```sh
pnpm snowflake:qualification:manifest --input .cache/request.json
pnpm snowflake:qualification:manifest --input .cache/request.json \
  --emit .cache/snowflake-qualification/review.json
pnpm snowflake:qualification:manifest --input .cache/request.json \
  --verify .cache/snowflake-qualification/review.json
pnpm test:snowflake:qualification:manifest
```

Emission requires the output root and parent to exist as owner-owned `0700`
directories. It creates a new `0600` file exclusively, refuses an existing output
and rejects symlinked parents. Dry-run and verification write no artifacts.
`--run`, unknown/duplicate options and simultaneous emit/verify are rejected.
The compiler validates the window's shape and duration; a future runner must
check current time, approval freshness and one-time consumption. No command here
grants authorization or establishes account behavior.

## Local verification

The 12 synthetic compiler tests pass, including exact SQL/command counts,
immutable canonical output, typed scalar bindings, resource limits, tampering
with and without a recomputed hash, current-evidence drift, template drift,
redacted output, dry-run side-effect guards, private file validation and exclusive
emission/verification. The dry-run side-effect test injects synthetic evidence;
actual prepared-resource validation is a separate local check. Evidence lives in
`.cache/verification/connection-attempts/sdk-manifest-tests-final.log` and
`sdk-manifest-format-check.log`. These tests establish offline compiler behavior,
not enforcement by an executable runner or the packaged application.

Actual CLI checks with synthetic connection identifiers first reject the stale
prepared helper source, then pass after offline runtime preparation. Dry-run,
exclusive emission and independent verification pass against the current local
resources; a repeated emission is refused. Logs are `sdk-manifest-current-stage-first.log`,
`sdk-manifest-current-stage-final.log`, `sdk-manifest-emit-final.log`,
`sdk-manifest-verify-final.log` and `sdk-manifest-overwrite-refused.log` in the same
evidence directory. This uses no account connection. Independent review found
two documentation inaccuracies about run freshness and identifier storage;
both are corrected above. No material implementation finding remains from that
bounded review.
