# Snowflake SDK session observation and proposed schema switching

The observation and bounded schema-options portion is now implemented; see its
[current contract and qualification](snowflake-sdk-session-observation.md).
Typed schema switching below remains proposed. Neither portion establishes
Snowflake account qualification. Both use the existing SDK connection.
The helper/supervisor transport foundation is implemented and locally qualified;
see its [verification evidence](replacement-verification.md#sdk-schema-switch-transport-10-november-2026). It is not yet exposed through the application
adapter, typed IPC or schema selector. Ordinary helper `context {}` is now a
strict read-only operation. The separate `switchSchema` operation retains known
USE acknowledgement independently of final context verification, quotes each
identifier component and retires on uncertainty. Definitive USE rejection is
also conservatively retired in this foundation. It does not enable public SQL
session commands or bypass ReadOnly.
The current read-only policy remains: writes, DDL,
autocommit changes, BEGIN, COMMIT, and ROLLBACK stay disabled. It does not complete
the [DataGrip replacement scope](snowflake-datagrip-replacement.md).

The existing [relation browsing workflow](snowflake-sdk-browsing-design.md) and
completion metadata retain their separate contracts. Schema switching must revoke
both workflows' old targets. Cross-database switching, transaction controls, and
switching safely within an active transaction remain later requirements.

1. **Represent unavailable observations explicitly.**

   Add `Unknown` to the generic transaction mode and activity enums. Add one
   optional tagged `transactionObservation` payload to `SessionRuntimeState`,
   omitted for legacy engines:

   ```ts
   type TransactionObservation =
     | { status: "unavailable" }
     | {
         status: "observed";
         transactionId: string | null;
         abortOnError: boolean;
         usability: "unknown";
       };
   ```

   Successful observations require every observed field. Unavailable or
   inconsistent observations use mode/state Unknown; a successful NULL ID maps
   to Idle, and a nonempty opaque ID maps to InTransaction. Never
   substitute AutoCommit/Idle after an error or when the SDK has not observed them.

   `Unknown` is observational only. Adding it to the shared mode enum must not
   widen `set_transaction_mode` admission: reject it before state changes or SQL
   in Studio and concrete drivers, or use a separate two-value setter DTO. Never
   coerce it to AutoCommit. Regression tests must prove zero mode-change effects
   for malformed/unknown setter input on PostgreSQL and MySQL as well as SDK.

   Mode, activity, and usability are separate facts. AUTOCOMMIT can be true while
   an explicit transaction is open. A nonnull CURRENT_TRANSACTION result identifies
   an open transaction; it does not establish that further statements can succeed.
   The official COMMIT example shows NULL when no transaction remains.
   [CURRENT_TRANSACTION](https://docs.snowflake.com/en/sql-reference/functions/current_transaction),
   [COMMIT](https://docs.snowflake.com/en/sql-reference/sql/commit)

   Generic `InTransaction` means observed activity only; the tagged SDK payload
   explicitly leaves usability unknown. Do not map a nonnull ID to a healthy Active
   state, or map any SQL error to Failed/aborted. The abort-on-error parameter is
   policy, not evidence that the current transaction has aborted. When enabled,
   an error in a non-autocommit transaction can make subsequent statements fail
   until COMMIT or ROLLBACK; even the observer can therefore become unavailable.
   [TRANSACTION_ABORT_ON_ERROR](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error)

   Make observed role nullable throughout the shared runtime DTO and consumers.
   CURRENT_ROLE can return NULL; never replace it with the configured profile role.
   Existing engines supply their observed role as Some. This increment still
   requires the fixed configured database rather than adding database switching.
   [CURRENT_ROLE](https://docs.snowflake.com/en/sql-reference/functions/current_role)

2. **Observe with fixed, bounded reads.**

   Add pure query constants and parsers, preferably in
   `src-tauri/src/drivers/snowflake_session.rs`. The additional fixed statements
   are:

   ```sql
   SHOW PARAMETERS LIKE 'AUTOCOMMIT' IN SESSION
   SHOW PARAMETERS LIKE 'TRANSACTION_ABORT_ON_ERROR' IN SESSION
   SELECT CAST(CURRENT_TRANSACTION() AS VARCHAR) AS TRANSACTION_ID
   ```

   Each result must be complete and untruncated, at most one row and 64 KiB,
   including metadata. Require exactly one row, unique expected column names,
   authoritative Text kinds, exact parameter keys, and only documented boolean
   spellings. Read `value`, not `default`. A parameter inherited from another
   level need not report level SESSION. LIKE is case-insensitive and supports
   underscore wildcards, so the exact-key check is necessary even for fixed SQL.
   Parse the transaction cell as bounded nonempty opaque text or SQL NULL; never
   convert it through a JavaScript Number or require decimal digits.
   [SHOW PARAMETERS](https://docs.snowflake.com/en/sql-reference/sql/show-parameters)

   An observation sample combines these three results with the existing fixed
   `snowflake_metadata::CONTEXT_SQL` and quoted-identifier-policy query. Compare
   two bounded samples under one SDK lane and one overall metadata deadline;
   reject changed session, principal, context fingerprint, parameters, or
   transaction observation. There are at most ten explicit statement executions
   for this proposed observer, with no repeat-until-stable loop. This count does
   not describe SDK authentication/session setup or assert an account cost.
   Failure produces Unknown, not stale successful state. No alternate-session,
   history, LAST_QUERY_ID, or RESULT_SCAN fallback is permitted.

   Observation must never set AUTOCOMMIT. Snowflake can implicitly commit an
   active transaction when ALTER SESSION SET AUTOCOMMIT executes, even if its
   value is unchanged. Observing a setting is not permission to change it.
   [Transactions](https://docs.snowflake.com/en/sql-reference/transactions)

3. **Return coherent state and schema choices.**

   Recommend a `Driver::session_context_options(session_id)` method, with a
   compatibility default that preserves existing engines' state/list behavior
   and propagates list errors. Override it in `snowflake_external.rs` so SDK
   observation and schema enumeration share one inner lane and verified context
   bracket. Use private helpers that do not recursively acquire that lane.

   Studio's `build_session_contexts` should call this method and check session and
   driver closure before publishing. Reuse the bounded current-database SCHEMATA
   builder/parser: at most 1,000 schemas, with complete-result and byte checks.
   A failed list invalidates options; it is not an empty successful list. Showing
   the current schema separately must not authorize switching to a name omitted
   from the freshly validated choices. Visibility is privilege-filtered.
   The fixed state observation uses ten executions; inserting one bounded schema
   list between its five-statement samples uses eleven. The planned typed switch
   uses thirteen: before sample (five), schema list (one), USE (one), the helper's
   existing six-column identity readback (one), and a full after sample (five).
   The Rust context read has eight columns plus a separate identifier-policy
   read; the six-column helper result cannot replace it. Any extra verification
   reads must increase the budget explicitly. Samples detect sampled drift; they are not
   an atomic snapshot and cannot detect every change between statements.
   SqlEditor also currently prepends its current schema to choices; remove that
   behavior for revision-bound SDK options. An omitted current schema may be
   displayed separately as unavailable, never inserted as a selectable target.

4. **Give schema changes a typed admission and outcome contract.**

   Recommend `SwitchSchemaRequest { revision, database, schema, context_fingerprint }`, a
   default-Unsupported `Driver::switch_schema`, and an explicitly enrolled
   `switch_session_schema` command. Other engines retain their current API;
   generic SDK `switch_context(&str)` remains unsupported. Keep arbitrary USE and
   ALTER rejected by public SQL authorization.

   Issue an opaque server-held options revision alongside each SDK options
   result. The content fingerprint is not a generation: an A→B→A context change
   must not revive an obsolete switch request. Bind the revision to the verified
   context and schema choices, revoke/consume it before the first await, and
   recheck ownership after lane acquisition. Reject stale revisions before USE.
   Frontend generations alone prevent stale publication, not stale SQL dispatch.

   Studio acquires the exact session lane, rejects running work, and invalidates
   completion, relation targets, and prepared plans before dispatch. The SDK
   validates a fresh context against the supplied fingerprint, the fixed initial
   database, the supported identifier policy, and the selected visible schema.
   For this first slice, require an observed idle transaction; Unknown or open
   activity cannot silently enter a schema-switch workflow while recovery and
   transaction controls remain unavailable.

   Require actual strings before identifier validation. Quote each identifier
   component with the existing escaping helper and submit this one statement:

   ```sql
   USE SCHEMA "<verified database>"."<selected schema>"
   ```

   Dots, quotes, or punctuation inside a name remain part of that single quoted
   component. Do not accept SQL fragments or silently switch databases. Snowflake
   supports a database-qualified schema name and case-sensitive quoted names.
   [USE SCHEMA](https://docs.snowflake.com/en/sql-reference/sql/use-schema)

   Read back the same SDK session before publishing success: require the target
   schema, unchanged database/principal/session, and expected remaining context.
   Preserve the observed nullable role, warehouse, secondary roles, identifier
   policy, AUTOCOMMIT, abort-on-error policy and idle activity. The resolved search
   path may legitimately change with the selected schema; retain and fingerprint
   the complete newly observed path instead of requiring the old path verbatim.
   A definitive rejection leaves targets revoked and requires fresh observation.
   A lost, cancelled, timed-out, malformed, or mismatched post-dispatch result
   leaves context Unknown and retires the session. Never replay the switch or
   reconnect to infer its result. Known statement acknowledgement remains evidence
   without becoming a claim of verified final context.

   The dependent adapter method returns `SessionContextOptions` directly. One
   lane and one bounded operation contain exactly thirteen SQL executions:
   first five-read sample, one bounded schema inventory, USE, helper identity
   readback and final five-read sample. Before the first await, atomically consume
   the matching options snapshot and replace its public revision with a private
   workflow generation. Check generation again before USE admission and final
   publication. The first sample must equal the consumed baseline and be idle;
   the fresh schema inventory must contain the selected target.

   Arm an adapter retirement guard before admitting the helper switch, and
   disarm it only after final verification and generation-safe publication.
   Supervisor operation ownership covers USE/readback, but its successful
   terminal no longer owns cancellation or abandonment during the last five
   reads. Those later interruptions must also retire an unverified switched
   session. Studio publishes the returned options directly: another options
   refresh would add eleven SQL executions and revoke the returned revision.
   Frontend state must retain that opaque revision, revoke it before dispatch
   and reject late success/failure by the exact request and session generation.

   Options authority needs its own request ownership, separate from the UI
   context revision that triggers editor metadata effects. Revoke it before
   SQL preparation and again before approved execution, automatic health,
   completion catalog/lazy columns, relation catalog/inspection/preview, and
   template-context observation. Those operations either revoke the backend
   revision unconditionally or can discover drift/error without a runtime event.
   A pending older options response cannot restore it. Do not increment the
   editor context revision merely to clear authority: that can cause a metadata
   refresh loop. Initial editor metadata must settle before its existing options
   refresh; do not add hidden refresh/retry SQL to restore choices. Template
   observation must use the same guarded options path. Automatic health skips
   pending context work and queued/running queries, and clears authority before
   any admitted read. Session retirement clears all ownership.

   If USE is known acknowledged but later verification fails, preserve that
   distinction in the fixed application error message. Close the session without
   describing USE as rejected or its execution as unknown. Final context remains
   unverified, and no reconnect/replay may be used to infer it.

5. **Migrate the existing helper setup callers explicitly.**

   The preceding `helper.mjs` had a `context { schema }` path that could issue USE
   followed by readback, while `snowflake_process.rs` classified context as
   `Impact::Read`. The transport foundation replaces it with strict `context {}`
   and a dedicated typed `switchSchema` operation using
   `Impact::ContextChange`, with updated admission, statement-count/phase checks,
   abandonment and cancellation handling. Each USE and readback execution keeps
   its own submit-guard ticket through consumption; operation success requires
   final readback, not just an earlier USE acknowledgement.

   Keep operation-level USE dispatch/acknowledgement evidence separate from the
   per-execute readback state. A definitive USE rejection may report a database
   rejection. Once USE succeeds, even a definitive permission error on readback
   means final context is unverified: retire and publish Unknown, retaining the
   known USE acknowledgement without claiming the USE was rejected. The final
   operation acknowledgement must follow validated readback. Cancellation and
   abandonment after dispatch follow ContextChange uncertainty; only proven
   local unsent cancellation can be NotSubmitted.

   The reviewed helper payload is exactly:

   ```json
   {
     "database": "DB",
     "schema": "Selected.Schema",
     "expected": {
       "user": "SERVER_USER_NAME",
       "role": null,
       "warehouse": "WH",
       "session": "opaque-session-id"
     }
   }
   ```

   Every key is required; unknown keys are rejected. Role and warehouse may be
   SQL NULL, independently of the configured profile. Rust retains revision,
   fingerprint and generation authority. The helper receives the fresh baseline
   rather than making an additional pre-read. Identifiers are actual nonempty
   strings, bounded to the documented 255 characters and a local byte limit,
   with no NUL or malformed Unicode. Quote database and schema independently,
   doubling internal quotes; dots remain part of one component.
   [Identifier requirements](https://docs.snowflake.com/en/sql-reference/identifiers-syntax)

   The operation event sequence is accepted → dispatching → first statement
   identity → `useAcknowledged` → readback statement identity → final acknowledged
   → terminal. Keep `Report.use_acknowledged: Option<StatementIdentity>` and the
   pending equivalent through retirement. The USE event is legal only once for
   `switchSchema`, while dispatching, and must identify the first registered
   statement. It must precede result drain so a later drain failure cannot erase
   already received acknowledgement. Two distinct statements and at most four
   identity publications are allowed. Final acknowledgement requires exactly one
   bounded six-field readback row with matching target/expected identity. Final
   acknowledgement and successful terminal events must name the readback's
   registered SDK request ID; neither may reuse the USE ID or omit association.

   `Impact::ContextChange` distinguishes a proven locally unsent operation from
   every uncertain post-dispatch interruption. A cancel-control acknowledgement
   does not settle the switch. Abandonment retires instead of using read-result
   discard semantics. Definitive USE rejection can remain a database rejection;
   after USE acknowledgement, a readback rejection is final-context uncertainty
   and retirement, with the USE evidence retained.

   Audit all context callers and migrate schema-bearing helper tests before
   making ordinary context observation read-only. Preserve startup's profile
   role/database/schema/warehouse and same-session identity assertions, accounting
   explicitly for any required setup statement. Do not silently widen the prior
   approved canary: its two fixed SELECTs and network scope remain unchanged.
   Refresh packaged source pins and repeat package/native verification after
   helper changes; synthetic fixture substitution is not production verification.

6. **Invalidate the frontend before dispatch and retain Unknown on failure.**

   In `state/studio.ts`, advance the context revision and mark observation/options
   unavailable before sending a switch. Guard late completion with the exact
   session and request generation. Failed and superseded switches must not restore
   an old context, completion snapshot, inspection, or preview. Readback success
   installs fresh state and choices; a list/readback error cannot leave old
   choices presented as current.

   SqlEditor, RelationBrowser, and Workbench should render Unknown explicitly,
   without AutoCommit/Idle defaults, and keep writes/TX actions disabled. Unknown
   or open transaction state participates in close confirmation. Closing copy
   must not promise that local helper reap proves immediate remote rollback:
   abruptly disconnected transactions can remain detached.
   [Transactions](https://docs.snowflake.com/en/sql-reference/transactions)

The minimum acceptance sequence is pure parser tests, helper/protocol tests,
actual-Node synthetic SDK/Studio integration, frontend regressions, then fixed
runtime/package/native checks. Cover duplicate/missing headers and keys, malformed
booleans, NULL versus unavailable transaction observations, opaque IDs, truncation,
byte limits, inconsistent samples, and nonnull-ID usability uncertainty. Verify
one SDK connection throughout and zero ALTER/DML/DDL/COMMIT/ROLLBACK submissions.

Switch tests must reject stale, forged, cross-database, and malformed targets
before USE; preserve quoted names; and revoke targets before errors. Inject lost
USE acknowledgements, cancellation after acknowledgement, failed readback,
schema-list failure, and session-generation replacement: one USE dispatch, no
replay, conservative state, retained cleanup ownership, and no late publication.
Exercise deferred frontend failures/successes and Unknown close handling, while
preserving PostgreSQL/MySQL compatibility. Actual pinned-SDK loopback tests must
prove the new transport operation independently of synthetic SDK fixtures.

Account acceptance remains separately authorized and unresolved: actual observer
wire shapes, observation inside aborted transactions, schema privilege/drop races,
and any future switching within an active transaction. Rechecks do not create an
atomic server snapshot. Observer and metadata statements advance session history;
history-sensitive SQL stays disabled. The original proposal required no account
queries; later implementation and local builds retain their separate evidence.
