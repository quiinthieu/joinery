# MySQL/MariaDB retained-edit identity: bounded design draft

**Status:** design proposal only. MySQL 8.4.13 and MariaDB 11.4.13 pass a
standalone metadata-lock protocol probe; Joinery's 18 historical drop/recreate
application cases remain unsafe. This is not an application repair or a
production-readiness claim. The owner's required engines and tolerance for
concurrent migration blocking remain unanswered.

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

The report explicitly sets `applicationSafetyQualified: false`. These results
establish a protocol premise for these fixture versions and SQL forms, not that
Joinery already tracks a guard, that every server/version behaves identically,
or that all 18 application workflows are safe.

## Candidate ordinary-role protocol to falsify before implementation

1. **Acquire before reading.** Create a dedicated, backend-owned guard
   connection and explicit transaction before the original table browse, query
   result, or CSV preview reads the target. Access the target within that
   transaction so its metadata lock spans the retained snapshot and approval
   period. Store an opaque application generation/lease ID with the result or
   preview; never infer lease identity from a connection pool slot or cached
   transaction flag.
2. **Bound and own the lease.** Set bounded server lock wait and application
   lease deadlines. Expiry must actively close/rollback the guard connection,
   even when no later request arrives; do not rely on lazy cache pruning. Release
   when the last result/preview/plan owner releases it, including rejection,
   cancellation, eviction and disconnect. Explicit invalidation and shutdown
   revoke every reference. CSV must retain its original lease across batch
   commits. If expiry or transport loss occurs, mark retained edits stale
   and require a new read and review.
3. **Validate from the writer transaction.** Start the writer transaction and
   acquire its own lock on the exact qualified target first. Before schema/row
   checks or DML, always issue a fresh ping on the original guard with reconnect
   disabled; require success, an active transaction status, matching live
   application generation, and an unexpired lease. Then validate schema and
   conflicts and execute DML in that same writer transaction. Any failed or
   ambiguous check fails closed and rolls back. Never auto-reconnect, begin a
   replacement transaction, or replay reads to make a lost guard look valid.
4. **Keep every workflow inside the contract.** Browse, eligible query results
   (including empty results), and CSV preview need a lease established before
   their source read. CSV batches reacquire/retain writer locks per batch and
   revalidate the original guard each time; a batch commit does not replace the
   original lease. Define ownership when multiple results share one page/lease.
   Existing manual console transactions stay independent and must never be
   committed or rolled back by the edit guard. Raw approved SQL effects that do
   not have proven target provenance need a separate policy; do not claim this
   protocol covers them.
5. **Make waiting visible and bounded.** A queued DDL operation can sit ahead of
   the writer and cause it to time out behind the still-held original lease.
   Use a bounded metadata-lock wait, show rejection/expiry clearly, release the
   guard explicitly before user-approved DDL, then require a fresh read and
   review. Measure whether release unblocks queued DDL and how cancel/expiry
   behaves while a writer is waiting.

The probe covers basic drop/rename, queued DDL, guard loss and writer-held-lock
premises. It does not qualify Joinery lifecycle races, active TTL cleanup,
multiple readers sharing a lease, cancellation while waiting, CSV batch commits,
all 18 original application cases, engine variants, privilege combinations, TLS,
or external deployments. Implement and qualify those interactions in isolated
increments. Replacement acceptance must resolve the owner's concurrent-migration
workflow and tolerable waiting behavior.

## Reproduction

Start the README's fixed local fixtures using its opt-in fixture setup. The
probe accepts only loopback ports and fixed public fixture credentials; its TLS
certificate verification is intentionally disabled for the self-signed local
fixture only. This is not TLS or credential qualification.

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

## Next implementation increments

1. Build lifecycle-only tests for guard registration, active TTL closure,
   disconnect/release, eviction, cancellation, late connection handshakes and
   shutdown; prove one dead guard cannot retire a replacement generation.
2. Integrate the guard through source reads and a single-row writer, testing
   stale replacement before prepare, after approval and while queued DDL blocks
   the writer. Keep the target lock, final verification, conflict check and DML
   in one writer transaction.
3. Extend to CSV preview and each committed batch, shared page/result leases,
   empty-result inserts and UI expiry/review recovery. Keep raw SQL and console
   transaction ownership separately specified.

Relevant Joinery paths to review include `browse_with_approval`,
`resolve_statement_provenance` / `Output::describe`, `prepare_table_changes`,
`preview_csv_import`, `prepare_csv_import_batch`, and the MySQL/MariaDB driver
apply path. These names locate current read/plan/write responsibilities; they
do not imply lease support exists.

## Source boundary

The official references document metadata-lock behavior and the privilege gate;
they do not specify Joinery's proposed lease protocol or DataGrip's behavior
across drop/recreate:

- [MySQL 8.4 metadata locking](https://dev.mysql.com/doc/refman/8.4/en/metadata-locking.html)
- [MariaDB metadata locking](https://mariadb.com/docs/server/reference/sql-statements/transactions/metadata-locking)
- [MySQL 8.4 InnoDB table metadata](https://dev.mysql.com/doc/refman/8.4/en/information-schema-innodb-tables-table.html)

The bounded report and script are local experimental evidence. The historical
Joinery application probe at `ff59ca6` remains the evidence that MySQL/MariaDB
can currently write to a replacement in 18 cases.
