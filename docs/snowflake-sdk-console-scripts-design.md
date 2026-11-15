# Snowflake reviewed SQL consoles and scripts

Status: required workflow, incomplete and not externally qualified. Its
[compiler and private transport foundations](snowflake-sdk-console-transport.md)
are implemented locally; retained application ownership, admission, receipts and
UI remain open. This is the next product workflow after script early-stop
accounting in `278754b`. All applicable DataGrip capabilities remain required.
The design is an execution path to broader owner-authored SQL, not a permanent
grammar-only allowlist.

## Daily workflow and acceptance target

On an explicitly opted-in writable external-browser SDK profile, the owner must
be able to run a reviewed sequence of reads, scalar-bound writes and explicit
transaction boundaries on one retained console connection. BEGIN in one run,
DML/SELECT in another and an explicit COMMIT or ROLLBACK in a later run must keep
the same physical session. SHOW followed by RESULT_SCAN(LAST_QUERY_ID()) must
preserve adjacency within and across runs. USE, SET, temporary objects, broader
DML/DDL, procedures and scripting remain required, with honest server validation
where local grammar coverage is incomplete. Client PUT/GET and import remain
required separate local-file workflows; generic database SQL consent cannot
silently grant filesystem authority.

The owner sees the exact immutable SQL/ordered units and parameter bindings in
one policy-appropriate review. The UI reports original statement acknowledgement,
retained results, interruption and an untouched suffix separately. It never
labels a command acknowledgement as independently verified committed data.

## Why looping current actions is insufficient

Current read/DML/transaction public owners independently lock the Studio lane,
consume plans and revoke grants. Typed adapter operations issue SQL observations
before/after their command. Generic run finalization, health, editor completion,
context templates and error recovery can also issue SQL on the connection.
Those operations change LAST_QUERY_ID and potentially other session semantics.
They cannot transparently wrap arbitrary user SQL.

Use one retained owner plus a new private console lease. Reuse guarded transport,
scalar binding, original-outcome retention and bounded streaming beneath the
public probe-bearing owners. Preserve existing generated-operation authority;
do not weaken it or claim a script lease is a coherent transaction observation.

## Admission, policy and SQL units

- Add separate default-off profile consent for reviewed console SQL. Legacy,
  OAuth and ReadOnly settings must not acquire write permission. Authentication,
  group, production-passphrase and queued-policy invalidation rules remain.
  Merely unobserved generated-operation context is distinct from rollback-only,
  retired, unrecorded-outcome or other recovery holds. Console consent cannot
  authorize DML/COMMIT through a rollback-only state or revive a retired session;
  preserve each hold's narrower existing recovery permissions natively and in UI.
- Native admission binds immutable text, ordered units, scalar binds, session
  user/ID pin, policy generation and stop settings. Consume one approval once,
  under the configuration gate and console lane; check revocation before each
  next dispatch. No automatic retry, reconnect, settlement or suffix resume.
- Provide a parsed sequence for sound locally segmented statements and an
  explicit selected single server-validated unit for unsupported dialect or
  scripting. Do not split procedural bodies at arbitrary semicolons. Local
  syntax/binding/policy checks cover the whole retained plan; no universal
  side-effect-free server dry-run is claimed.
- A first-class reviewedSql dispatch family ultimately supports exact
  owner-authored database SQL whose effects cannot be proven statically, under
  writable profile permission, explicit applicable review and database-role
  privileges. Typed known operations remain useful optimizations and warnings,
  not the sole route forever. Unknown effects receive conservative treatment.
- Force helper-owned per-execute MULTI_STATEMENT_COUNT=1 for every unit, even if
  user SQL changed the session default. One guarded physical submission is not
  proof of one SQL statement in its body. No caller-controlled parameter object.
- Preserve scalar-only bindings and canonical wire recompilation. Named inputs
  remain global; anonymous slots need deterministic statement-local maps without
  interpolating values. Bounds and each stricter per-step limit remain explicit.
- Veto the exact pinned SDK PUT/GET pre-execute branch on the unchanged primitive
  SQL string before execute. Keep file transfer blocked visibly until its own
  implementation/permission exists; this is not a general SQL classifier.

## Console ownership and knowledge

Console ownership lasts across runs, not merely while a job is pending. Suppress
implicit SQL baseline/between/final/next-run observers on that physical connection.
Enforce this natively and in UI scheduling, including stale mounted handlers.
Health must be passive/unprobed rather than a hidden SELECT 1 or a fabricated
server-health success. Static and cached local authoring remain available.

Separate three kinds of knowledge:

1. Stable authenticated user/session identity for the retained helper lease.
   Role/database/schema/warehouse are separate mutable context, not this pin.
2. Per-field context observed from the successful statement response. SDK 3.4.1
   exposes public getSessionState() getters for current database, schema, role
   and warehouse, backed by final*Name fields. Capture them with original ACK
   and statement identity before draining. Missing fields stay individually
   unknown; never substitute startup configuration or claim a complete snapshot.
3. Coherent observation required for generated actions. Response context does
   not establish transaction ID/disposition, AUTOCOMMIT, abort-on-error or search
   path. Such authority is invalidated by console SQL and restored only through
   an explicit bounded user-requested observation, with its SQL/session-history
   effect disclosed. Refresh must not reenable automatic probes on this console.

Future separately authorized metadata connections and console-local authoring
can provide completion without altering main-session history. They must not
pretend to share temporary objects or transactions. Full DataGrip authoring,
metadata, accessibility and normal native usability remain acceptance work;
permanent blanket Unknown or permanently disabled completion is not the target.

## Original outcomes, limits and recovery

Add a closed helper reviewedSql operation with a generic sticky success ACK,
registered statement association and full bounded row streaming. Do not overload
DML's insert/update/delete ACK or its one-row/two-column summary limits. Pin and
verify newly trusted SDK result.js and util.js sources in addition to existing
transport pins. Cancellation confirmation and row-drain errors cannot overwrite
an already retained original acknowledgement.

One detached tracked owner holds the lane and processes units sequentially.
Correlate admission with a client request identity, or an equivalent immutable
session/epoch active-run lookup, before execute can start. If the start response
is lost, the UI must recover that owner's run ID for cancellation and SQL-free
snapshot reconciliation; it must never resubmit execute to discover the outcome.
Deduplicate admission atomically and retain the correlation through reconciliation.
Retain step-start intent before dispatch and original ACK/rejection/unknown
before advancing or publication. Stop admissions on cancellation, error,
retention limit, revocation, retirement or recording failure. Preserve an
acknowledged prefix; never automatically execute a skipped COMMIT/ROLLBACK.
Define event-delivery loss separately from SQL cancellation and retain outcomes
through local receipt reconciliation. Polling loss must not fabricate a final
remote outcome. Do not restart a server action to recover a local UI receipt.

Counts represent client top-level execution units, not statements inside CALL
or scripting bodies, and must be labeled accordingly. A typed transport Ok
carrying Rejected is not a completed SQL acknowledgement. Keep the ordinary
ActionSql driver-return accounting contract unchanged. Exact affected counts,
where validated, remain decimal strings; generic status rows are ordinary
results rather than invented affected counts.

A durable local journal records run and per-step intent/outcome without bind
values or secrets, respecting SQL-text history preference. After process loss,
an unresolved intent is unresolved/unknown, never proven unsubmitted and never
automatically resumed. An ACK lost before durable recording cannot be recovered
by assertion. Preserve the in-process outbox and shutdown barrier on local write
failure; do not promise durability across forced exit while storage is failing.

A failed Snowflake DML statement usually rolls back that statement, while
TRANSACTION_ABORT_ON_ERROR changes recovery behavior. DDL and setting AUTOCOMMIT
can commit preceding work, even when AUTOCOMMIT is assigned its current value.
Disconnect can leave detached transactions/locks. Report known command evidence
and state knowledge separately; preserve explicit recovery and do not silently
wrap scripts, reset settings, roll back, commit or replace the session.

## Implementation groups and sequencing

Lead owns the DTO/API contract, profile policy, retained Studio owner, journal,
commands/native permissions, integration and final verification. Agree the
contract first. Then parallelize bounded independent groups:

- Helper/process: closed reviewedSql payload, generic sticky ACK/context frames,
  exact single-statement override, source pins, file-transfer veto, streaming,
  supervisor validation and actual-SDK loopback/process tests.
- Compiler: ordered script IR, sound segmentation/explicit single-unit mode,
  scalar input maps, exact wire recompilation, bounds and whole-plan policy
  warnings with pure regression tests.
- UI: profile consent, run/parameter/immutable review, permanent session mode,
  background suppression, original per-unit outcomes, SQL-free reconciliation,
  context provenance, history and explicit refresh/recovery behavior.

Keep shared domain/driver/IPC files coordinated by the lead. Perform dependent
adapter/owner integration after transport/IR contracts exist. Serialize Cargo
jobs; use independent frontend/helper checks in parallel where safe. A separate
bounded review must cover meaningful increments before final validation.

## Required evidence

- Reproduce the extra-probe adjacency problem through actual product/helper SQL
  inventory. Assert no implicit SQL before/between/after/across reviewed runs,
  including idle health, lazy completion, templates and stale UI handlers.
- Compiler and policy cases: full mixed plan, quoted/comment semicolons, opaque
  single unit, invalid later local input => zero dispatch, named/anonymous binds,
  exact approved text, limits, stale/replayed/profile-revoked plans.
- Actual helper/pinned SDK loopback: exact per-unit SQL and MULTI_STATEMENT_COUNT,
  scalar binds, no file-transfer/stage behavior, source-pin rejection, original
  ACK before drain/cancel/error and malformed/duplicate/foreign frames.
- Retained-owner races: queued/in-flight cancellation, disconnect, dropped IPC,
  lost start response and duplicate admission correlation, SQL-free run-ID/snapshot
  recovery, rollback-only/retired holds across broad SQL admission, local journal/history/audit failure, shutdown and
  restart uncertainty. No suffix replay or invented successful commit.
- Real isolated PostgreSQL can prove shared sequential-owner/transaction/history
  invariants, with fresh observers proving skipped writes and explicit rollback.
  It cannot qualify Snowflake SDK or transaction behavior.
- Separately approved disposable Snowflake acceptance must prove across-run
  transactions/effects, both abort settings, exact descriptor/context shapes,
  LAST_QUERY_ID adjacency, USE/SET/temp objects, DDL/AUTOCOMMIT boundaries,
  scripting units, cancellation/connection loss and million-row result/export.
- Real browser/native flows must cover whole review/parameters, per-unit results,
  pending cancellation, unknown and skipped COMMIT, metadata-only history,
  explicit refresh effect and local receipt recovery. Screenshots alone do not
  establish workflow success. Normal packaging, SSO/restart and owner acceptance
  remain separate gates.

No account SQL, owner marker/profile-store access, billing, installation,
publication, push or merge is authorized by this design. The owner-permitted
SANDBOX.BILLIE_TESTING target still needs a fresh exact account plan and approval.

## Evidence and official references

Read-only architecture/UI reports record matching source hashes and Tier 2
coverage with direct checks for excluded/generated/dependency paths. Their
ignored files are next-mixed-script-architecture.json, next-mixed-script-ui.json,
next-mixed-script-official.json and next-mixed-script-response-context.json under
.cache/verification/script-accounting. These are source/research findings, not
implementation or account qualification.

- [Transactions and implicit boundaries](https://docs.snowflake.com/en/sql-reference/transactions)
- [Abort-on-error parameter](https://docs.snowflake.com/en/sql-reference/parameters#transaction-abort-on-error)
- [LAST_QUERY_ID](https://docs.snowflake.com/en/sql-reference/functions/last_query_id)
- [Shared connection state](https://docs.snowflake.com/en/developer-guide/driver-connections)
- [SDK execute, binds, batches and cancellation](https://docs.snowflake.com/en/developer-guide/node-js/nodejs-driver-execute)
- [Snowflake Scripting blocks](https://docs.snowflake.com/en/developer-guide/snowflake-scripting/blocks)
- [Pinned SDK Statement](https://github.com/snowflakedb/snowflake-connector-nodejs/blob/v3.4.1/lib/connection/statement.js)
- [Pinned SDK Result](https://github.com/snowflakedb/snowflake-connector-nodejs/blob/v3.4.1/lib/connection/result/result.js)
- [Pinned SDK Util](https://github.com/snowflakedb/snowflake-connector-nodejs/blob/v3.4.1/lib/util.js)
