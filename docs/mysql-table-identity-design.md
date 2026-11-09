# MySQL/MariaDB retained-edit identity: bounded guard implementation

**Status:** implemented and locally committed as `997680e`. Final source checks
pass 581 frontend / 193 standard Rust tests, 124 actual fixture tests (including
11 guard tests looping across both engines), 27/27 strict application identity
scenarios and 137/137 browser workflows. The strict diagnostic includes all 18
historical MySQL/MariaDB replacement cases under the hidden four-second fixture
policy; production uses 60 seconds. Actual local TLS-over-SSH guard/writer checks
pass on both engines. Clean-source native/bundle qualification is still pending.
These checks do not qualify external engines or the owner's required Snowflake
workflow. `ff59ca6` remains the historical unsafe baseline. Guard capacity and
other disclosed lifecycle cases remain unqualified below.

## What the disposable probe established

[`scripts/mysql-mdl-probe.py`](../scripts/mysql-mdl-probe.py)
ran against only the fixed local MySQL (53306) and MariaDB (53307) fixtures as
the ordinary `joinery` role. The local report is
`/tmp/joinery-mysql-mdl-report.json`. Both engines passed; eight UUID-owned
tables per engine were removed and catalog absence verified, with no cleanup
errors.

Observed on both engines:

- Reading InnoDB dictionary identity failed with error 1227 (`PROCESS` required).
- An explicit transaction that read an empty table retained metadata protection:
  `DROP TABLE` and `RENAME TABLE` waited and returned error 1205 at the configured
  two-second lock timeout. Rollback released the lock and the DDL then succeeded.
- A writer queued behind observed waiting DDL also timed out while the original
  guard remained live. Long-lived guards can therefore stall migrations and
  writers.
- After intentional socket loss, the client retained a cached open handle and
  `IN_TRANS` status, but a fresh `ping(reconnect=False)` failed with 2006. A new
  writer could otherwise read the replacement. Cached client state is not proof
  that the original guard survives.
- A writer transaction that acquired its own target lock continued to block DDL
  after guard transport loss, until writer rollback.
- The writer's empty-result query shape
  `SELECT id,note FROM joinery.<table> WHERE 1=0 FOR UPDATE` held protection.
  A separate `START TRANSACTION READ ONLY` guard at READ COMMITTED, followed by
  a table read and fresh non-reconnecting ping, also retained protection.

The standalone report explicitly sets `applicationSafetyQualified: false`.
These results establish a protocol premise for these fixture versions and SQL
forms; they do not qualify application integration, establish behavior on every
server/version, or cover all application workflows. The separate, preliminary
application evidence is summarized below.

## Implemented guard contract

The implementation assigns each guard a stable generation, session identity
and canonical table identity. Result-column names corroborate the source lease;
they do not prove physical table identity. Source AST eligibility checks and
temporary-table rejection remain required.

| Bound | Value |
|---|---:|
| Absolute, nonrenewing lease TTL | 60 seconds |
| Guards per session | At most 32; acquisition rejects while full (no eviction) |
| Guard acquisition | 10 seconds |
| Fresh original-guard PING/verification | 3 seconds |
| Guard close | 2 seconds |
| Writer metadata-lock wait | 5 seconds |
| Editor non-read metadata-lock wait | 5 seconds |
| Existing transport operation ceiling | 65 seconds |

1. **Acquire before source reads.** Browse, eligible query-result reads and CSV
   preview establish the original guard before reading rows. Each CSV preview
   carries a distinct opaque generation so late async responses or an unmounted
   dialog cannot attach a stale lease to a newer preview. Preparing or committing
   a CSV plan requires its exact nonempty preview token; no tokenless legacy Rust
   path or latest-preview fallback is accepted. The native named IPC boundary
   requires a string. Pure preview analysis may omit the token, but cannot prepare
   a plan. Wire names corroborate the lease; they do not replace it.
2. **Own and expire the lease.** Releasing the last result, preview or approved
   plan owner revokes that generation. TTL is absolute and nonrenewing, and
   actively closes the connection even if references remain; do not rely on
   lazy cache pruning. Cancellation and shutdown also revoke guards. Failed or
   ambiguous PING, expiry, generation mismatch or session close invalidates the
   retained edit and requires a fresh read and review.
3. **Order writer checks.** The writer first acquires its own metadata lock
   under the five-second bound. It then unconditionally sends a fresh `COM_PING`
   on the original guard with reconnect disabled, verifies `IN_TRANS` and the
   expected generation/session/canonical table within three seconds, then checks
   schema/conflicts and runs DML in that same writer transaction. Successful
   writer MDL plus fresh original-guard verification is the linearization point.
   Failure before it is fail-closed. Never reconnect, re-`BEGIN`, or replay
   writes to make a lost guard appear valid.
4. **Handle expiry and CSV batches.** An already-authorized write may finish
   after later guard expiry once writer MDL and fresh verification passed;
   cancellation and shutdown still apply. Every subsequent CSV batch must
   acquire writer MDL and verify the same original generation again. A batch
   commit does not replace the original lease.
5. **Coordinate DDL.** Explicitly approved application DDL revokes all
   MySQL/MariaDB guards across application sessions before SQL is sent. DDL and
   `Unknown` statements take this path; for example, `CREATE TRIGGER` is
   classified `Unknown`. If a new read races revocation, bounded DDL acquisition
   rejects without automatic retry. A write that already crossed the writer
   MDL plus fresh-guard verification linearization point may finish after a
   later expiry; cancellation and shutdown still apply. External DDL can wait
   behind a lease, and a writer can wait behind queued DDL. Surface
   timeout/expiry and require fresh read and review; provide explicit guard
   release before planned DDL.
6. **Keep ownership separate.** Existing manual console transactions are
   untouched. Raw approved SQL effects without proven target provenance remain
   outside this guard policy. Shared result/page ownership must not let releasing
   one result revoke another live owner. Successful browse results whose guard
   expires retain their returned snapshot but are narrowed to read-only. Late
   raw-browse responses are drained to their terminal outcome: a server SQL error
   remains a database failure, while transport loss after raw submission remains
   outcome-unknown and retires the session without retry.
   CSV previews use their own opaque generation token; release of a pending
   plan, dialog unmount or late response cannot bind it to a newer preview.

The user-visible tradeoff is that retained edits can hold a migration lock for
up to 60 seconds; writer/editor metadata-lock waits are bounded at five seconds
and may reject. The owner has not accepted this tradeoff. Do not extend TTL,
request `PROCESS`, or silently retry around it.

The standalone protocol report is not application qualification. Separate final
application checks pass all 27 strict scenarios and all 137 browser workflows;
the full actual fixture suite includes 11 guard tests. The historical 18-case
MySQL/MariaDB replacement diagnostic uses a hidden four-second TTL and therefore
does not qualify the production 60-second duration or every expiry race. The
actual TLS-over-SSH test requires original SNI on every accepted channel and
observes guard locking plus writer commit. It is positive local-fixture evidence,
not external authentication or an independent wrong-CA/hostname rejection test.
Capacity rejection at exactly 32 live targets, late handshakes and detailed close
failure permutations remain unqualified; existing successful lifecycle checks
do not imply those cases passed. Engine/version/privilege variants and owner
acceptance of migration blocking remain separate gates.

## Reproduction

Start the README's fixed local fixtures using its opt-in fixture setup. The
probe accepts only loopback ports and fixed public fixture credentials; its TLS
certificate verification is intentionally disabled for the self-signed local
fixture only. The standalone probe does not qualify TLS or credentials.

```sh
python3 scripts/mysql-tls-fixture.py
docker compose --profile mysql --profile mysql-folded --profile redis up -d --wait
UV_CACHE_DIR=/tmp/joinery-mysql-mdl-uv-cache \
  uv run --no-project --python /usr/bin/python3 --with PyMySQL==1.1.2 \
  scripts/mysql-mdl-probe.py --run-disposable-fixtures \
  --output /tmp/joinery-mysql-mdl-report.json
```

Review the JSON's per-engine results and cleanup, not exit status alone. The
probe drops only its UUID-owned tables and verifies absence. It grants no
privileges and changes no server-wide settings. Use disposable local fixtures
only.

## Remaining implementation and proof gates

1. Qualify the disclosed capacity, late-handshake and close-failure cases rather
   than extrapolating from existing lifecycle tests. Measure production-duration
   expiry and waiting behavior separately from short fixture policy runs.
2. Complete clean-source native/bundle qualification and normal packaged
   restart/credential acceptance. Keep exact artifact/source identity in the ledger.
3. Qualify external engine/version/privilege combinations only when needed and
   authorized. Resolve any owner requirement for retained edits during concurrent
   migrations; no acceptance of bounded rejection/lock delay has been inferred.
4. Preserve actual replacement/conflict/fresh-control and cleanup regressions
   when extending the driver. This guard does not supply Snowflake row identity.

Relevant Joinery paths include `browse_with_approval`,
`resolve_statement_provenance` / `Output::describe`, `prepare_table_changes`,
`preview_csv_import`, `prepare_csv_import_batch`, and the MySQL/MariaDB driver
apply path. These paths now use the guard contract above; final qualification
is still required before claiming completion or broad engine support.

## Source boundary

The official references document metadata-lock behavior and the privilege gate;
they do not specify Joinery's proposed lease protocol or DataGrip's behavior
across drop/recreate:

- [MySQL 8.4 metadata locking](https://dev.mysql.com/doc/refman/8.4/en/metadata-locking.html)
- [MariaDB metadata locking](https://mariadb.com/docs/server/reference/sql-statements/transactions/metadata-locking)
- [MySQL 8.4 InnoDB table metadata](https://dev.mysql.com/doc/refman/8.4/en/information-schema-innodb-tables-table.html)

The bounded report and script are local experimental evidence. The historical
Joinery application probe at `ff59ca6` recorded writes to replacement tables
in 18 cases; it describes that historical implementation, not the current
measured guard implementation.
