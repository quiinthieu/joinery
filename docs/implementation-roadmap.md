# Joinery implementation roadmap

## Durable console admission decisions (12 November 2026)

This increment implements SQL-free durable admission refusals and request lookup,
schema-3 migration with old-writer exclusion, exact restore merging and frontend
response-loss/restart recovery. Local verification passes 205 focused frontend,
nine IPC and 71 focused native cases (one internal child ignored by default).
Full validation passes 893 frontend / 559 standard Rust cases (137 default
ignores), and the actual browser-to-Rust/helper/SQLite/Arrow workflow passes with
a synthetic SDK. At 1024 × 720, bounded receipt history supports ordinary clicks
and keyboard scrolling while pending notices remain visible. The
[contract](snowflake-console-negative-receipts-design.md) and
[ledger](replacement-verification.md) track this source separately. Earlier
sections describe their recorded predecessor, not verification of new source.

Normal SDK-bearing ARM64 packaging from clean `9b20aa2` now passes resources,
strict ad-hoc signature and guarded helper startup with zero authentication/SQL.
The app remains local and uninstalled; packaged native UI acceptance is open.
Continue practical receipt retention without losing recovery identities,
configurable qualified long-running queries, then the fresh approved disposable
Snowflake/native/owner acceptance plan. Every applicable
DataGrip capability remains required. No account, installation or publication
authority is granted by local tests.

## Retained console application integration (12 November 2026)

The retained owner and UI are locally implemented and verified with synthetic SDKs:
immutable review, one-use admission, same-client SQL-free recovery, revocation,
intent/ACK outbox holds, history/audit and receipt-owned retained results. Follow
the [workflow](snowflake-sdk-console-workflow.md) and source-scoped
[ledger](replacement-verification.md); earlier private-foundation evidence below
does not qualify this new integration.

Full validation passes 872 frontend and 546 standard Rust tests (137 default
fixture ignores); 18 owner cases and one actual browser-to-Studio/helper workflow
also pass. Independent bounded review has no remaining material finding.
The normal SDK-bearing ARM64 app builds from clean `12fef9a`; strict ad-hoc
signature/resources and network-guarded helper startup pass with no sign-in or
SQL. It remains local and uninstalled; packaged UI startup is not established.

At this predecessor, the next work included
[durable negative admission receipts](snowflake-console-negative-receipts-design.md)
for a lost refusal response; the increment above now verifies that local recovery.
Remaining work includes practical journal retention that
preserves deduplication/recovery, and qualified configurable long-running query
behavior. Then execute a fresh approved disposable account plan for real
Snowflake writes/DDL/transactions/interruption and million-row interaction/export.
Normal macOS external-browser SSO/restart and hands-on acceptance remain open,
alongside every other required DataGrip capability in the matrix.

## Retained console adapter and local evidence (12 November 2026)

The [retained adapter/journal contract](snowflake-sdk-console-journal.md) is
implemented with focused local coverage (35 passing cases; one internal child
fixture invoked by its parent). Separate profile consent, permanent physical
session ownership, hidden-SQL suppression, ACK-before-publication ordering and
SQL-free durable intent/original outcomes are available as private foundations.
The SQLite journal distinguishes proven non-dispatch from crash uncertainty,
deduplicates client correlation and preserves live evidence through old/v1
metadata restore. Independent source review has no remaining material finding;
full validation passes 823 frontend / 545 standard Rust cases (137 default
ignores), build/compiler checks and strict Clippy on the recorded unchanged
sources in the [ledger](replacement-verification.md).

The broader adapter scope passes 117 cases and the Studio rerun passes 36.
One initial Studio helper-start failure remains unexplained despite isolated
and full rerun success. Add closed test-only startup diagnostics before making
reliability claims; preserve the current deadline and avoid raw private errors.

Advance to the complete Studio owner/UI workflow next: immutable one-use review
and policy checks, durable intent/ACK sequencing with in-process outbox holds,
SQL-free lost-start-response recovery/cancellation, honest results/history and
suppression of implicit SQL across separate runs. Connect BEGIN, later DML and
later explicit COMMIT/ROLLBACK on one physical console, plus SHOW/RESULT_SCAN,
USE/SET/DDL and selected server-validated units. Test interruption and restart
without automatic replay or settlement. Provide safe SQL completion and journal
retention without erasing recovery/deduplication evidence. Replace fixed foundation
delivery deadlines with qualified configurable long-running query behavior.
Full native/account
qualification and every other required DataGrip area remain open. The private
foundations expose no new user command and do not qualify a daily console.

## Reviewed Snowflake console foundations (12 November 2026)

The [pure compiler and private transport](snowflake-sdk-console-transport.md)
are locally implemented. They preserve ordered mixed SQL, canonical scalar
bindings, original acknowledgement and bounded result streaming without granting
application permission. The [ledger](replacement-verification.md) records their
source-scoped checks and review; no account effects or usable console workflow
are established by transport tests.

Next implement the retained native console owner, default-off profile consent,
one-use immutable review/admission, SQL-free run correlation and local receipts.
Then integrate editor controls and exercise complete mixed reads/writes/explicit
boundaries across runs, hidden-observer suppression, interruption and recovery.
The [full design](snowflake-sdk-console-scripts-design.md) remains the acceptance
contract. Generated-action recovery holds, file-transfer permission, actual
Snowflake effects, million-row results and the other required capability areas
remain open; no permanent feature reduction is accepted.

## Script early-stop accounting (12 November 2026)

A reproduced defect marked a script Completed after its first classified read
hit a result limit and the remaining suffix never ran. The executor now reports
**Stopped** when that limit leaves a suffix. Final events, history, editor and
history search retain planned, attempted, successful and unattempted counts plus
the stop reason. Driver attempts do not prove server submission; successful
returns do not prove commit or complete result retention. Existing history without
accounting stays explicitly unknown. No automatic replay or transaction settlement
is added. See the [contract](script-execution-accounting.md) and
[usage walkthrough](migration-from-datagrip.md#script-stops-and-statement-counts).

Focused checks pass 160 frontend cases and 13 executor cases. Seven real isolated
PostgreSQL cases pass, including row/byte limits with a sequence proving the suffix
untouched, cancellation, explicit server termination, error prefixes, restart
persistence and a skipped COMMIT left open until explicit rollback. The real
browser workflow passes 1/1 using 1,000,001 synthetic PostgreSQL rows and a fresh
observer proving a skipped UPDATE did not execute. Independent bounded review
has no material finding. Full validation passes 823 frontend and 500 standard
Rust tests (136 fixture/helper ignores), formatting, TypeScript, production UI
build, compiler checks and strict all-target/all-feature Clippy. Hidden native
startup/IPC and ten cleanup cycles pass on the same frozen source. The normal
SDK-bearing ARM64 package builds from clean feature commit `278754b`; strict
ad-hoc signature/resources and guarded SDK startup pass with zero authentication
or SQL requests. Version/build 0.1.0 totals 229.35 MiB in final regular files.
The app remains local and uninstalled; Developer ID/notarization and normal
packaged UI/SSO/restart acceptance stay open.

Generalized mixed Snowflake scripts, account write/transaction/DDL effects,
million-row Snowflake interaction/export, safe editing/import, broader metadata
and performance tools remain required. The account plan expired unexecuted;
SANDBOX.BILLIE_TESTING permission is recorded, but a fresh reviewed exact plan and
possible-billing approval remain needed. The Goal is active and incomplete.

## Snowflake CHECK inspection (12 November 2026)

A separate **View CHECK constraints** action is implemented through retained
opaque targets, the serialized metadata lane, typed IPC and main-window native
permissions. The [contract](snowflake-sdk-check-constraints-design.md) fixes exact
identity, complete bounded results and inert clause display. Focused frontend/IPC
140/140, focused Rust 13/13 and synthetic SDK sessions 104/104 pass. Full validation
passes 803 frontend / 486 standard Rust tests (129 fixture/helper ignores) and
strict checks; the changed synthetic browser workflow passes 1/1. Independent
bounded UI/integration and backend reviews have no material finding. Hidden
native startup/IPC and 10 cleanup cycles pass. The normal SDK-bearing ARM64
package builds from clean 313bc11; strict ad-hoc signature/resources and guarded
helper startup pass with zero authentication/SQL requests. Version/build 0.1.0
totals 229.34 MiB in final regular files. It remains local and uninstalled;
Developer ID/notarization and normal app UI/SSO/restart acceptance remain open.
No account requests or production qualification-ledger expansion are included.

The [prepared walkthrough](migration-from-datagrip.md#snowflake-check-constraints-prepared-acceptance-walkthrough)
leaves actual role visibility, output compatibility, interruption and normal
packaged usability open. CHECK creation/modification/validation, cross-database
catalogs, inbound relationships, safe grid editing/import, broader SQL/DDL,
performance investigation, actual million-row Snowflake workflows and owner
acceptance remain required. The Goal is active and incomplete.

### Next product priorities

1. Implement the required [reviewed SQL console/script design](snowflake-sdk-console-scripts-design.md):
   one retained owner across runs, private execution without hidden SQL observers,
   broad reviewed owner-authored SQL, scalar binds and explicit transaction
   boundaries. Reuse transport primitives beneath public probe-bearing actions;
   looping those actions would alter LAST_QUERY_ID and session semantics. Retain
   passive per-field SDK response context separately from coherent generated-action
   authority. Preserve rollback-only recovery, lost-start-response correlation,
   whole-plan local admission, original outcomes and an untouched suffix. No
   automatic replay, settlement, reconnect or suffix resume. Read-only Run all and
   typed batch parameters already work; the broader workflow remains incomplete.
2. Continue complete metadata/search, safe grid editing/import, broader DDL/session
   commands, plans/performance, actual million-row Snowflake interaction/export
   and normal packaged persistence/accessibility. All remain in required scope.

The read-only planning review is recorded in the [ledger](replacement-verification.md)
and ignored next-script-review.json; it ran no tests or account activity. Account
qualification under owner-permitted SANDBOX.BILLIE_TESTING still requires a fresh
reviewed exact Plan/window and possible-billing approval. The expired plan grants
no authority; keep independent product work moving.

## Product increment: select a referenced table (12 November 2026)

**Select referenced table** is implemented locally; source and browser checks
pass. It resolves the exact foreign-key database/schema/table against the
entire retained catalog, requires one match before checking availability, clears
the filter and old details, selects the retained opaque target and focuses the
Relation selector. It sends no RPC. Visible explanations cover missing,
ambiguous, unavailable and external-database destinations. Fresh ownership,
context and render-generation checks protect activation across refresh and
transient holds.

Three focused backend regressions and all 97 synthetic SDK session regressions
pass, proving exact quoted cross-schema inspection against the existing backend
admission. Focused UI tests pass 97/97; full validation passes 762 frontend /
481 standard Rust tests (129 fixture/helper ignores) and strict checks. The
changed synthetic browser workflow passes 1/1. Independent bounded review has
no material finding. Production backend source is unchanged. The
[contract](snowflake-sdk-key-mappings-design.md#referenced-table-selection),
[prepared walkthrough](migration-from-datagrip.md#referenced-snowflake-tables-prepared-acceptance-walkthrough)
and [ledger](replacement-verification.md) record results. Hidden native
startup/IPC and 10 cleanup cycles pass. The normal SDK-bearing ARM64 package
builds from clean feature commit 5219db5; strict ad-hoc signature/resources and
guarded helper startup pass with zero authentication/SQL requests. It is local
and uninstalled; normal UI/SSO/restart, Developer ID/notarization and account
acceptance remain open.

Cross-database navigation remains required and needs a separately admitted
database-scoped retained catalog; do not relax profile-database checks or borrow
schema-switch authority. Inbound/related-row navigation, virtual foreign keys,
JOIN completion and diagrams remain in the required applicability inventory.
Current official Snowflake documentation confirms CHECK constraints are
applicable; their inventory remains required. Dynamic-table mappings,
dependencies, complete object coverage, safe grid editing/import, general SQL,
broader schema changes, performance tools and actual million-row Snowflake
interaction/export remain open. Normal packaged SSO/recovery, live account
effects and owner hands-on acceptance remain gates. The Goal is active and
incomplete; no new account activity is authorized.

### CHECK-constraint design before implementation

The initial design below is implemented in the new increment above; local and
account qualification remain separately tracked. It specifies an independent retained-target
`inspect_relation_check_constraints` action. The official
[CHECK_CONSTRAINTS view](https://docs.snowflake.com/en/sql-reference/info-schema/check_constraints)
provides constraint catalog/schema/table/name and clause directly. Use one closed
read against the fully quoted retained database, binding its exact catalog,
schema and table; no constraint-name join or schema-wide fallback is needed.
Reuse the serialized relation lane, retained revision/opaque target and pre/post
context/relation checks. Preserve current unavailable-target admission.

The proposed response retains the target identity and five text fields per
constraint. Bound it to 1,000 rows with an overflow sentinel and 1 MiB, reject
incomplete/malformed/duplicate/mismatched rows, and preserve clause text as inert
read-only text. Empty role-visible results and denied requests remain distinct;
neither grants editing authority or proves engine eligibility. The view supplies
no enforced/validated flag. Current traits do not distinguish Snowflake-managed
and externally managed Iceberg tables, so do not infer support from that flag.

The read-only design review used Tier 2 generation 2026-11-12T07:13:02Z,
coverage for nine source paths, snippets/trace and direct reads. Required
regressions include quoted cross-schema identities, same names on other tables,
multiline/Unicode clauses, malformed/NULL/duplicate/overflow results,
denial/empty states, forged/stale IDs before SQL, held reads and context drift,
independent UI failure and stale-result invalidation. No files, tests, builds,
account requests or qualification-ledger authority changed for that review.

## Product increment: ordered key columns (12 November 2026)

The separate Snowflake **View key column mappings** action is implemented locally.
It pairs composite foreign-key columns by ordinal,
reports qualified referenced tables, preserves missing/unsupported states and
leaves declaration/column/definition views available on a mapping failure.
Dynamic-table mappings and unknown-trait targets remain gated. The
[contract](snowflake-sdk-key-mappings-design.md),
[prepared walkthrough](migration-from-datagrip.md#snowflake-key-columns-prepared-acceptance-walkthrough)
and [ledger](replacement-verification.md) distinguish local checks from account
acceptance. Focused tests and full validation pass (743 frontend / 481 standard
Rust tests, 129 fixture/helper ignores and strict checks); the changed synthetic
browser workflow passes 1/1. Independent bounded review has no unresolved
material finding. Hidden native command registration/rejection and 10 cleanup
cycles pass. The normal SDK-bearing ARM64 package builds from clean feature
commit 54f59b6; strict ad-hoc signature/resources and guarded helper startup pass
with zero account requests. It is local and uninstalled; normal app UI/SSO,
Developer ID/notarization and account inspection remain unqualified.
No new account execution or approval is included.

Next required product work includes foreign-key table navigation/inbound
references, dynamic-table mappings, CHECK constraints, dependencies and complete
object coverage. Safe grid editing/import, general SQL/multiple statements,
broader schema changes, performance tools and actual million-row Snowflake
interaction/export remain required. Normal packaged SSO/recovery and owner
hands-on acceptance remain gates; the Goal is active and incomplete.

Referenced-table selection now has its own increment above. Its initial
read-only design used Tier 2 generation 2026-11-12T07:01:27Z, coverage for ten
evidence paths and direct reads on clean 54f59b6. It made no edits, builds or
account requests.

## Product increment: declared keys (12 November 2026)

Add a separate read-only key inventory to the Snowflake relation browser,
preserving reported flags, hidden foreign-key references and explicit empty/error
states. The implementation and focused tests are complete; independent review
has no unresolved material finding. Full validation passes 718 frontend / 476
standard Rust tests (129 fixture/helper ignores) and strict checks. The changed
headless browser flow passes 1/1. Hidden native command registration/rejection
and 10 cleanup cycles pass. The normal SDK-bearing ARM64 package builds from
clean fabeb62, with strict ad-hoc signature/resources and guarded SDK startup
passing without account requests. It is local and uninstalled; normal SSO/usage,
Developer ID/notarization and account key visibility remain unqualified. See the
[contract](snowflake-sdk-constraints-design.md) and source-specific
[ledger](replacement-verification.md).

Next metadata work includes key column mappings, foreign-key table navigation,
CHECK constraints, dependencies and complete object coverage. These declarations
do not establish safe row identity. Grid editing/import, broader SQL and schema
changes, performance investigation and million-row Snowflake workflows remain
required alongside normal packaged SSO/recovery and owner acceptance. The
separate frozen table-lifecycle account plan does not qualify this command.
No account activity, installation or publication is part of local verification.

## Product increment: generated relation definitions (12 November 2026)

The Snowflake relation browser now displays and copies generated table/view DDL
through a closed typed GET_DDL command. It resolves retained opaque catalog
targets, serializes metadata operations, rejects incomplete/oversized responses
and discards stale UI results. It does not execute the displayed SQL. Full
validation passes 697 frontend / 471 standard Rust tests (129 fixture ignores)
plus strict checks; see the [contract](snowflake-sdk-browsing-design.md)
and [ledger](replacement-verification.md). Independent bounded review is clean.

The changed browser workflow passes after the recorded initialization fix.
The corrected hidden native IPC/cleanup check passes; strict Rust checks pass
after the two ACL enrollment entries. The normal local SDK-bearing ARM64 package
passes on clean `e218aef`, including resources, strict ad-hoc signature and guarded
helper startup with zero account requests. It remains uninstalled.
The [reviewed exact account plan](snowflake-table-attempt-9a8d9d3d.md) for
05:52–06:22 UTC on 12 November expired without approval or execution. Any account
attempt needs a new reviewed Plan/window and explicit account, permanent
marker-store and billing approval. Account definition
privileges, secure views and supported Snowflake object
families still need explicitly scoped acceptance. Continue broader metadata,
object search, grid editing/import, multi-statement SQL, performance and million-row
workflows; displaying a definition closes none of those required gates.

A separate offline review binds copied source `103cb45`, the actual binary,
compiler and zero-link runtime. Both the expired preview and fresh Plan were
emitted and verified without SSO or account activity. The later attempt expired
at 06:22 UTC without an approval reply, approval file or owner marker-store access;
the earlier CREATE DATABASE approval is spent. This qualification does not qualify
the new definition command.
Normal packaged SSO/recovery, actual database effects, all applicable DataGrip
capabilities and owner hands-on acceptance remain required. No publication or
installation is authorized.

## Execute entry and next qualification gates (12 November 2026)

The dedicated command now connects original-envelope restoration to the fixed
macOS consumption store and private factory. SIGINT/SIGTERM registration precedes
blocking preparation; observed interruption revokes future admissions while the
entry retains preparation, original calls, shutdown and terminal persistence.
Closed receipts distinguish unsuccessful execution from successful receipt
persistence. Offline emission and verification remain free of account activity.
The [command guide](snowflake-table-offline-command.md) describes the boundary;
the [ledger](replacement-verification.md) records completed source-specific checks.

Five optimized process cases pass, covering both signals before preparation and
during held CREATE, foreground process-group Ctrl-C with lost acknowledgement,
the full 32-phase lifecycle and spent-marker rejection in a fresh process with
another output/environment. Tests use synthetic homes and a synthetic SDK; the
production binary has no fixture override. A prior case stopped in phase 1 and
two CREATE barriers were unavailable; their cause and shared origin remain
unconfirmed. Passing repeats do not close the account-entry stability gate.

Next, bind the actual reviewed executable,
current source/compiler and a complete zero-link runtime. Prepare original exact
Plan/envelope evidence with the table-only SQL, metadata disclosure, window,
ceilings and possible billing. Obtain fresh approval before any account attempt
under `SANDBOX.BILLIE_TESTING`; the rejected CREATE DATABASE approval is spent.
No account approval question is pending during local verification.

The twelve product areas and additional applicability inventory in the
[matrix](datagrip-feature-matrix.md) remain required. Normal packaged SSO/recovery,
actual writes/transactions/DDL, million-row Snowflake results/export, grid
editing/import, broader metadata/SQL/schema/performance tools and owner hands-on
acceptance remain open. This command closes no product replacement gate. No
publication or installation is included.

## Fixed store and revocation boundary (12 November 2026, preceding integration)

The internal macOS permanent store now uses a fixed namespace selected from the
effective user's OS account record. Synthetic-home and subprocess tests exercise
bootstrap, unsafe path rejection, concurrent consumption and reuse independent
of output/environment. A prepared attempt also exposes a memory-only revocation
handle; retained-owner tests cover cancellation before startup and around CREATE
acknowledgement/completion. See the [ledger](replacement-verification.md) for
completed verification and its limits. These are internal qualification changes;
the command remains offline-only and no account activity occurs.

The latest full repeat passes 263 tests with six default ignores, but a preceding
repeat fails three factory cases before CREATE. Diagnostic parallel/serial
follow-up passes without reproducing it. Keep the unconfirmed early failure
open and investigate with the retained terminal diagnostics before claiming a
reliable account entry; do not widen production deadlines to mask it.

Next, connect one separately approved execute route to this fixed store and the
private factory. Register interruption handling before starting, revoke future
SQL without aborting retained completion/terminal evidence, and prove the complete
entry with the synthetic SDK. Then present the exact table-only SQL, metadata
scope, window, ceiling and possible billing for fresh approval under
`SANDBOX.BILLIE_TESTING`. No approval request is pending while that entry is absent.

The twelve required product areas in the [matrix](datagrip-feature-matrix.md),
the additional applicability inventory and owner hands-on acceptance remain open.
In particular, normal packaged SSO/recovery, actual writes/transactions/DDL,
million-row Snowflake results/export, grid editing/import and broader daily
workflows still require implementation and evidence. No publication or install
is included in this increment.

## Dedicated offline entry and remaining execution boundary (12 November 2026, preceding integration)

The dedicated binary and narrow facade now provide offline `emit`/`verify` with
actual-executable binding, the release build recipe, exclusive persistent Plan/
envelope output and exact original-byte restoration. Only emission generates a
new target. `execute` is rejected. The [command guide](snowflake-table-offline-command.md)
documents reproducible local preparation; the
[verification ledger](replacement-verification.md) records current checks.

The final actual-command suite passes seven tests. Full validation passes
679 frontend / 467 standard Rust tests (129 fixture ignores) plus strict checks;
all 44 final source hashes match. Independent review has no material finding.
The earlier 252-test qualification result precedes the parser-only lint cleanup;
the ledger preserves that scope and the final affected-test rerun.

Next, fix permanent consumption-store placement independently of caller paths,
then connect one separately approved execute route to the private factory. Add
cross-process permanent RUN reuse rejection and test interrupted callers through
retained completion/terminal evidence. Prove the complete fixed entry with the
synthetic SDK before presenting a fresh exact account plan, metadata scope,
window, SQL ceiling and possible-billing approval for `SANDBOX.BILLIE_TESTING`.
No account activity or approval question is pending during offline implementation.

Normal packaged SSO/reconnect/cancellation/restart, actual Snowflake lifecycle
effects, million-row results/export, grid editing/import, broader metadata/SQL/
schema/performance tools and every remaining applicable DataGrip workflow stay
required. These product workflows must advance alongside the qualification
boundary; offline evidence does not establish replacement readiness. Owner
hands-on acceptance remains required. No install, push, merge or publication
occurs in this increment.

## Fresh consumed factory and next account boundary (12 November 2026)

The private factory now implements persistent exclusive attempt/workspace/state
allocation before consumption, exact consumed runtime/profile construction,
keychain-free ordinary Studio saving/preparation, and detached ownership through
terminal fsync. Output is separate from both reviewed artifacts and permanent
markers. Original acknowledgements are retained independently of final errors;
failed or unsettled output is never automatically deleted or resumed. Independent
bounded review is clean. See the [verification ledger](replacement-verification.md)
for checked source and results; preceding construction notes below are historical.
The optimized suite passes 244 qualification tests (5 default ignores) and 22
runtime tests; the new detached-owner ignore passes separately through all 32
phases / 393 synthetic requests. Full validation passes 679 frontend / 467
standard Rust tests, with 129 fixture ignores, plus strict checks. All 38 frozen
source hashes match. These local checks do not qualify account or normal UI use.

Next, add the dedicated binary and narrow facade for offline emit/verify and
separately approved execute. Fix permanent consumption-store placement independently
of caller output, bind the actual binary/build recipe, and prove offline zero-launch
and cross-process RUN reuse rejection. Restore the original reviewed envelope;
generate a new Plan only during emission. Then present the exact table-only SQL,
metadata scope, limits and billing implications for fresh approval before using
`SANDBOX.BILLIE_TESTING`. No approval request is pending while that command is absent.

Actual Snowflake lifecycle effects, normal packaged SSO/reconnect/cancellation/
restart, million-row results/export, safe grid editing/import, broader metadata/
SQL/schema tools, performance investigation and all remaining applicable DataGrip
capabilities stay required. The final owner walkthrough remains an acceptance
gate. No account activity, install, push, merge or publication occurs here.

## Complete-content performance gate (12 November 2026, passed locally)

The complete optimized envelope capture/restore/consume test passes against the
unchanged full packaged runtime. The consumed S=1 Studio workflow also passes:
32 phases / 393 synthetic SQL requests, 527 successful admissions, maximum
1,983 ms, and 667,423 ms from consumption through final persistence. Reproduced
test-timer ownership and whole-read timeout defects are corrected with focused
regressions and independent review. See the
[verification ledger](replacement-verification.md) for exact evidence and failures.

S=32 fails after nine completed phases when one admission reaches three seconds;
the frozen source still matches. Profiling identifies software SHA-256 as a
substantial cost; the macOS ARM64 acceleration feature now passes local verification.
Its independent digest vector and inventory suite pass; full revalidation now
measures 0.85–0.87 seconds against the exact unchanged packaged runtime. The
changed-source S=32 run passes all 32 phases / 610 SQL requests: 751 successful
admissions, maximum 1,419 ms and 703,735 ms consumption-to-persistence. S=1 also
passes 32 phases / 393 requests: 525 successful admissions, maximum 1,206 ms and
495,200 ms consumption-to-persistence. Both settle and persist; all 13 frozen
source hashes match. The combined optimized module passes 204 tests with 4
default ignores. Full application validation passes 679 frontend / 467 standard
Rust tests, with 129 fixtures ignored, plus formatting/build/check/strict Clippy.
Source commits `db18e03` and `a05cfae` preserve every inventory check and existing
admission/lifecycle limit. The normal ARM64 package build, zero-link resources,
ad-hoc signature and network-guarded helper startup pass on clean `f5d02e4`;
normal UI, installation and account acceptance remain open. Proceed to the fixed
account entry and bound-root construction below. `SANDBOX.BILLIE_TESTING` is the owner's
permitted new-table location; prepare the exact table-only plan before fresh
execution/billing approval. No new account activity has occurred. All applicable
DataGrip workflows and hands-on acceptance remain required; the Goal is active
and incomplete.

Read-only integration review identifies the next concrete construction work:

The fixed profile/policy and private envelope input increment passes 223 optimized
tests, with 4 default ignores. Account/host mismatch and case-only RUN reuse are
reproduced and corrected. See the [verification ledger](replacement-verification.md).
Full validation passes 679 frontend / 467 standard Rust tests, with 129 fixture
ignores, plus formatting/build/check/strict Clippy; all seven source hashes match.
The next owner must reserve persistent exclusive attempt/workspace/state paths
and terminal output before consumption, retain them through detached final
persistence, and preserve them after incomplete shutdown. The current generic
Studio opener must never receive an existing application workspace.

The exact qualification runtime constructor now selects the supplied root with
compiled pins and no packaged/development fallback, requiring separate private
state. A reserved-journal constructor moves an already exclusively created,
untouched header into Policy only for the same Plan/profile; used, failed or
changed journals are rejected. These are closed construction seams, not the
factory or account entry. Optimized checks pass 227 qualification tests with
4 default ignores and 22 runtime tests, including all nine new cases.
Full validation passes 679 frontend / 467 standard Rust tests, with 129 fixture
ignores, plus formatting/build/check/strict Clippy; all four source hashes match.
Their commands and scope are recorded in the
[verification ledger](replacement-verification.md).

- Add one dedicated binary and a narrow private entry facade for offline
  emit/verify and separately approved execute. Bind that binary's actual build
  recipe; only emission generates a Plan. Verify/execute restore original
  reviewed bytes and their explicit digest.
- The closed 16 MiB envelope reader and file restoration route are implemented;
  smaller profile/approval limits and descriptor-relative ownership, permission
  and identity checks remain intact. Wire this route into the fixed entry and
  fix the permanent consumption root independently of output paths.
- Construct Studio, runtime and the new connection attempt only from the consumed
  roots. The current assembled-component runner constructor is suitable for
  fixtures, not proof of runtime provenance. Wire the exact runtime constructor
  and reserved journal into one private owner. Use fresh exclusive private storage;
  the generic persistence opener can adopt existing databases and change modes.
- Retain resource/guard/attempt/output ownership through journal and terminal
  receipt persistence, including dropped waiters and failed launches. Prove
  zero-launch offline commands, cross-process RUN reuse rejection, no profile or
  runtime fallback and truthful original outcomes after local storage failure.
- The isolated Staging/Confirm profile and recomputed fixed policy declaration
  are implemented, including opt-ins, verified TLS/443, no SSH/secrets and exact
  target. The seven-field input now validates the account against the driver's
  hostname derivation. The private factory must allocate exclusive fresh storage,
  inject a keychain-free secret store, save through the normal Staging group and
  use the normal prepare/authorize route before launch.
  Never adopt, modify or downgrade an existing profile; Production still enforces
  PasswordGated and is a separate qualification scope. Offline mismatch/tamper/
  missing-token/stale-token tests precede account execution. The first closed
  lifecycle uses fixed literals; parameterized and million-row account checks
  require separately reviewed scopes.

## Consumed runner guard (12 November 2026, local verification)

The retained runner now accepts the consumed envelope by value and checks exact
Plan/profile identity against its owning policy. Full artifact checks precede
authentication and every physical admission; synchronous publication callbacks
check only retained failure, wall time and the monotonic deadline. Authentication
is bounded to 120 seconds and the guard to 30 seconds, preserving the process
writer's stricter three-second admission limit. A timed-out or dropped scanner
retains its blocking-work permit until the OS read actually returns, while the
attempt is immediately revoked.

Final integrity and scanner settlement are journaled once, independently of
original operation outcomes. Review found and resolved a receipt-only final
failure that could disappear with a dropped waiter. New regressions cover this
case, pre-authentication drift, drift after CREATE acknowledgement, stalled
authentication and interrupted/failed final evidence storage. Source review is
clean; the final optimized module passes **200 tests / 1 default ignore**. The
reproduced terminal/control race is corrected with both actual-helper orderings
covered. Full validation passes **679 frontend / 467 standard Rust tests**, with
**129 fixture cases ignored**, plus formatting/build/check/strict Clippy. See the
[verification ledger](replacement-verification.md) for final results.
Code is committed locally as `f80c8c5`; nothing is pushed or published.

Next, in dependency order:

1. Measure complete optimized Envelope capture/restore/consume/revalidation with
   actual fixed repository scopes, executable/compiler and unchanged full packaged
   runtime, using a synthetic private profile. Then run S=1/32 consumed routes on
   full runtime copies actually used by Studio, with synthetic SDK changes frozen
   before capture and all mutable logs/controls outside the inventory. Record
   complete admission count/total/maximum and consumption-to-persistence time;
   preserve the three-second admission and 15-minute workflow limits. Runtime-only
   timing and the current minimal synthetic runtime do not prove this gate.
2. Add a dedicated fixed binary with offline emit/verify and approved execute
   modes; bind its actual build recipe rather than the HTTP fixture binary.
3. Use one permanent consumption-store root and fresh exclusive per-attempt
   private output/state directories. A different output directory must not allow
   RUN reuse. Add a closed 16 MiB envelope reader; keep smaller profile/approval
   bounds and do not generate approval in the executable.
4. Construct Studio/runtime/profile exclusively from the consumed bound roots
   through one private factory. Retain execution, private resources and terminal
   output ownership together; journal originals remain independent of reporting.
5. Verify offline zero-launch behavior, reuse/unsafe-file rejection, failed launch,
   dropped waiters and terminal-output failures through that same entry/factory.
6. Emit/review the concrete fresh-table plan for `SANDBOX.BILLIE_TESTING` and
   obtain fresh execution/billing approval.

Live account effects, packaged SSO, million-row behavior, all applicable DataGrip
workflows and owner acceptance remain required. The Goal remains active and
incomplete; this increment changes no normal application or package workflow.

## Offline execution envelope (12 November 2026)

The local envelope now binds the exact closed Plan, fixed Rust/helper/build
source scopes, complete zero-link runtime tree, executable, compiler, private
seven-field profile and at-most-30-minute admission window. Restoration recaptures
the supplied physical roots and requires exact canonical bytes/digest. A distinct
approval format is checked against the actually running executable before
run-first/plan-second create-once markers are consumed. Marker handles remain
retained for identity checks; failed or dropped attempts never refund markers.
This is an offline primitive, not an account entry or implemented placement of
the checks around authentication/each SQL. See the verification ledger for checks.

The complete qualification module passes 179 tests with one default ignore; the
ignored packaged-runtime inventory also passes explicitly. It covers all 15,110
runtime entries and 202,538,012 file bytes without launching the helper or SSO.
Focused counts overlap the module total. Independent bounded review is clean.
Full validation passes 679 frontend / 467 standard Rust tests and strict checks,
with 129 fixture cases ignored. The code is committed locally as `6b95a4d`.

Next, in dependency order:

1. Supply this consumed envelope only through a fixed Rust account entry to the
   retained runner. Check it outside process/policy locks before authentication,
   each physical SQL admission and after execution. Enforce the bound 120-second
   authentication and 30-second admission waits plus the retained 15-minute
   monotonic deadline; preserve stricter production limits and original outcomes.
2. Implement private exclusive manifest/approval/output handling and owned local
   lifecycle around the exact frozen profile and verified packaged runtime.
   Prove drift, expiry, interrupted/failed launch, no offline side effects and
   final durable evidence through the composed entry. These are not established
   by the separate primitives.
3. Emit and independently review the concrete fresh-table plan for
   `SANDBOX.BILLIE_TESTING`, then obtain fresh execution/billing approval.
4. Qualify real writes/DDL/transactions and interruptions, packaged SSO and 1M-row
   account behavior; complete all applicable DataGrip workflows and owner
   acceptance. The Goal remains active/incomplete.

No Snowflake activity or new installed/package workflow occurs here. Earlier
dated sections retain their original scope. Nothing is pushed or published.

## Retained owner integration (12 November 2026)

The shared 32-phase owner and conservative 32 MiB aggregate inbound body cap are
implemented locally. Route/fault fixtures use that same execution core. Typed
incomplete-read records preserve original operation/capture/cleanup errors and
actual event prefixes through late completion; dropping the waiter retains the
owner through recording and local settlement. Construction remains test-only.
The corrected-source qualification module passes 142/142 tests, including all
135 preceding cases; transport passes 27/27. Focused counts overlap these totals.
Full validation passes 679 frontend / 467 standard Rust tests and strict checks,
with 129 fixture cases ignored. Local commits are `963a455` and `68fff80`.
See the [verification ledger](replacement-verification.md) for final checks and
the source-review corrections; earlier roadmap sections retain historical scope.

Remaining work, in dependency order:

1. Bind a non-resumable executable attempt to exact reviewed source, actual
   binary, complete zero-link runtime contents and private profile. Restore the
   reviewed Plan and consume both its execution hash and account/host/database/
   schema/RUN marker before authentication. Supply a sealed consumed capability
   to the same runner; reject drift before authentication, each SQL and after
   execution. Verify cross-process reuse, partial consumption and launch failure.
2. Prepare the concrete fresh-table plan under `SANDBOX.BILLIE_TESTING`, with
   implemented limits and disclosures, then obtain fresh execution/billing
   approval. The location permission does not revive earlier consumed approvals.
3. Qualify actual write/DDL/transaction effects and interruptions, packaged SSO,
   and million-row account performance. Complete all applicable DataGrip
   workflows and the owner's hands-on acceptance before replacement readiness.

The normal local macOS bundle from the preceding increment remains the runnable
artifact; this harness-only increment changes no normal package workflow. The
Goal remains active/incomplete. Nothing is pushed, merged, published or installed.
The private project hub is not updated: available Sites updates require pushing
and deployment, both prohibited by the owner's instruction.

## Full Studio qualification composition (11 November 2026)

The synthetic-only owner now uses actual Studio APIs for all 32 phases. Initial
success runs at S=1/32 match the entire 393/610-request SQL ledger. Closed DDL
actions use the Plan's definitions and actual validated catalog/inspection tokens;
reads decode local retained Arrow data and transaction boundaries use prior actual
runtime authority. Original operation completions are recorded before validation.
The stateful SDK independently validates all nine SQL strings and generated target;
its ten Node regressions pass. Pre-repair full application validation passes 679
frontend and 466 standard Rust tests. The first expanded qualification run passes
119/122: all seven mutations pass each rejection, SDK callback loss and observation
failure scenario, and the dropped waiter passes too. Three cases fail at helper
startup in that run. Focused reruns pass the full success route and cancellation,
but the post-ACK journal-failure
case initially closes its session before reaching the intended fault. A reproduced
finalizing-operation race is now repaired: inactive credit/discard waits for that
exact operation's original public completion. Four deterministic regressions and
the original journal-failure case pass; independent review has no material finding.
The complete repaired-source qualification module passes all 122 tests; 131
process/adapter cases and strict Rust checks also pass. The two later full runs
fail the previously recorded PostgreSQL startup timeouts. A timing reproduction
now attributes those failures to synchronous native trust enumeration with TLS
disabled; the bounded correction passes the normal parallel protocol target and
45 real PostgreSQL/SSH/TLS fixture tests. Two new full-route tests also pass all
14 actual helper-child-exit scenarios, retaining unknown versus acknowledged
mutation outcomes without replay. Current full validation passes 679 frontend/
467 standard Rust tests and strict checks. The earlier generic Snowflake helper-
start cause remains unconfirmed; exact evidence is in the ledger.
The complete combined-source qualification module passes all 124 tests in normal
three-thread execution, including both new child-exit loops and existing faults.

Remaining work, in dependency order:

1. Extract the same retained 32-phase owner from its synthetic fixture, preserving
   production Studio routes, original completions, bounded event capture, local
   settlement and durable receipt barriers. Convert test assertions into retained
   typed errors; no account/profile/SQL/target selector or extra cleanup SQL.
2. Bind one non-resumable executable attempt to reviewed source, actual binary,
   complete runtime contents and private profile; consume a one-use envelope.
   Restore the exact reviewed Plan rather than regenerate its target. Enforce
   the proposed 32 MiB aggregate semantic-result cap independently of transport
   and per-read bounds. Bind a zero-link runtime with a complete inventory; the
   newly verified normal bundle supplies such a resource tree, while the default
   stage has five symlinks. Verify rejection before authentication, cross-process
   reuse, drift, failed launch and final receipt.
3. Prepare the exact fresh-table plan under `SANDBOX.BILLIE_TESTING`, then request
   fresh account execution/billing approval. Old approvals remain consumed.
4. Qualify actual writes, DDL, transaction effects, normal packaged SSO and
   million-row performance. Complete all applicable DataGrip workflows and the
   owner's hands-on walkthrough before replacement acceptance.

The closed Plan restoration preserves the original generated table, SQL,
expanded ledger and journal header across processes. Seven identity tests and
four drift tests/eight scenarios are included in the complete passing 135-test
module, with clean independent reviews. Drift covers original census order/
overflow, same-count schema replacement before CREATE and in catalog refresh,
role/warehouse, secondary roles and search path; the latter is not order-only
coverage. Full validation passes 679 frontend / 467 standard Rust tests and
strict checks, with 129 fixtures ignored. Plan restoration is an offline identity
primitive, not the executable envelope or approval; local child exits and
synthetic responses do not establish account effects. Exact source/evidence is in
the [ledger](replacement-verification.md#closed-plan-restoration-and-context-drift-11-november-2026).
The normal arm64 macOS bundle is rebuilt and locally verified with ad-hoc signing,
zero runtime symlinks and guarded zero-authentication/SQL helper startup. It is not
installed, notarized or account qualified.

See the [verification ledger](replacement-verification.md) for final results and
the distinction between synthetic routes and externally qualified product behavior.
The Goal remains active and incomplete. Nothing is pushed, merged, published or
installed. Earlier roadmap entries below retain their historical scope.

The private project hub is not updated in this local-only increment: available
Sites content updates require pushing source and deploying, both prohibited by
the owner's current instruction. Repository documentation remains the durable
current evidence until publication is authorized.

## Owning qualification policy (11 November 2026)

Implemented a harness-only owner of the exact physical ledger/control grammar,
callback debt, frozen lifecycle, actual runtime publications and typed phase
evidence. A private create-once journal records original intents, helper reports,
observations and validated phase evidence. Admission and advancement wait for
their own fsync receipts. Interrupted owners retain failure and consume completed
receipts once; cleanup cannot advance or retry SQL. See the
[contract](snowflake-qualification-sql-ledger.md#owning-policy-and-durable-barriers-11-november-2026)
and [verification evidence](replacement-verification.md#owning-qualification-policy-11-november-2026).

Next, in dependency order:

1. Extend actual adapter/Studio composition beyond startup/preflight through the
   complete 32-phase route and rejected/interrupted calls. The same-call Studio
   reconciliation bridge now retains the original options and catalog without
   another SQL request. Full-route composition remains open.
2. Extend local-completion coverage into the full actual production route.
   Verify original early ACKs and final durable evidence across interruption,
   rejected calls and local storage failure; never repeat SQL during cleanup.
3. Bind one executable, non-resumable attempt to reviewed source, binary, runtime
   and private profile identities; implement one-use approval consumption and the
   fixed runner. No caller-supplied SQL or arbitrary target.
4. Independently review full-route success, rejection, cancellation, connection
   interruption and local journal failures. Preserve exact statement budgets and
   prove cleanup does not replay SQL or evidence.
5. Prepare the executable fresh-table plan under `SANDBOX.BILLIE_TESTING`, then
   request fresh exact execution/billing approval before account activity.

The next runner increment uses existing typed Studio APIs. Share the Plan's
closed CREATE/ADD/DROP definitions; bind ADD/DROP only to validated, fresh real
catalog/inspection revisions. Retain each original typed completion or error
before validation, including acknowledged mutations whose later observations
fail. Own read events through `Finished`, preserving their arrival order and
both result metadata events. Decode at most two locally retained Arrow rows
through `Studio::range`; result retrieval must issue no SQL. Transaction
authority comes from the preceding actual runtime evidence, with no extra state
query. Keep results until their evidence is durably recorded.

Start with an opaque synthetic-only harness that owns a fresh Plan, disposable
Studio/helper and journal. Accept no SQL, target, saved-profile identifier,
arbitrary session or account selector. Exercise all 32 actual Studio routes with
a stateful synthetic SDK at census sizes 1 and 32, asserting exactly 393/610 SQL
and rollback/commit visibility within the fixture. On failure retain partial
events/original outcomes, settle and persist locally; do not advance, replay,
issue recovery SQL or run cleanup DROP. This is required runner work, not an
implemented or account-qualified workflow.

The separate local-completion seam now carries exact ticket/generation identity,
optional actually assigned wire ID, reserved candidate and a closed origin.
Pending removal and ticket-keyed retained evidence precede observation outside
the process mutex and unchanged caller publication. In-flight publication guards
participate in settlement. A deterministic adapter/helper test forces a query to
finish during durable credit admission: its inactive local completion consumes
the reservation without inventing a wire assignment or advancing SQL. Other local
outcomes retain the original report and stop the attempt. Full-runner evidence
and real account interruption qualification remain open; see the verification
ledger for passing and failed development runs.

All applicable DataGrip authoring, navigation, grid/import, schema/performance,
native persistence/packaging, normal packaged SSO and million-row account gates
remain in scope. The local policy is not replacement acceptance. Earlier roadmap
entries below retain their historical context; the Goal remains incomplete.

## Offline validation primitives (11 November 2026)

Implemented pure final-wire matching and in-memory lifecycle validation, plus a
harness-only callback for validated helper terminals before caller publication.
The original mutation report/ACK survives callback rejection and prior policy
failure. Wire/lifecycle validators remain unattached to an owning execution
policy. See the [contract](snowflake-qualification-sql-ledger.md#offline-wire-and-lifecycle-validators-11-november-2026)
and [verification evidence](replacement-verification.md).

Next bind these primitives to phase ownership, exact SQL/control order and
callback correlation. Phase completion must include drained/validated results
and durable original receipts; helper terminal alone is insufficient. Capture
retirement outcomes as well as helper terminals before claiming complete receipt
capture. Then finish one-use source/runtime/profile binding, durable journaling
and the fixed runner. Complete synthetic routes and failure paths precede a
fresh exact account plan for `SANDBOX.BILLIE_TESTING`. No account access or new
execution approval occurs in this increment. Full replacement remains incomplete.

## Shared SQL and offline ledger (11 November 2026)

Implemented [shared production definitions and census-bound expansion](snowflake-qualification-sql-ledger.md):
one pinned helper/Rust bootstrap and transaction contract, production-used named
read specs, and exact ordered entries for all 32 lifecycle phases. Fourteen
compiler tests and three read-spec tests pass; the 48-case qualification run
also passes. Definitive rejection permits only the production route's tail:
DDL five observer reads, normal Studio readback ten, DML/typed boundaries/metadata
zero. These are local/synthetic checks, not complete account execution routes.

Next implement the in-memory phase/census/lifecycle policy and bind the ledger
to physical admission; then durable intent/outcome journaling, a source/runtime/
profile-bound one-use envelope and the fixed production-route runner. Exercise
complete 393/610-entry synthetic routes and interrupted/rejected paths before
preparing any executable account plan. The permitted new-table target remains
`SANDBOX.BILLIE_TESTING`; no account approval is requested during this local work.

Full DataGrip workflow scope, actual account behavior, million-row performance,
installation/signing and owner acceptance remain open. See the
[verification ledger](replacement-verification.md) for the final checks and scope.

## Qualification attachments (11 November 2026)

Implemented the [internal attachment contract](snowflake-qualification-attachments.md):
asynchronous physical-request admission, bounded raw pipe/frame accounting,
validated wire-order census and creation-marker observations, cross-domain
failure latching, and post-lock publication checks through Studio. Focused
transport/metadata/Studio checks pass 13/13/8 respectively; seven new timing
regressions reproduced before repair. Independent final source review has no
outstanding material finding. Full validation and current source/artifact scope
are recorded in the [ledger](replacement-verification.md).

These hooks are not the qualification policy. Continue with the exact physical
SQL/phase ledger and exported bootstrap definitions, lifecycle census/marker
enforcement, private source/runtime/profile envelope and durable one-use journal,
then the fixed production-route runner. Qualify complete 393/610-execution routes
and failure suffixes locally before preparing any new account approval request.
The permitted target remains one fresh table in `SANDBOX.BILLIE_TESTING`.

All remaining DataGrip workflow gaps, normal packaged account behavior,
million-row account performance, installation/signing and hands-on acceptance
remain required. The Goal is active and incomplete; nothing is pushed, merged,
published or installed.

One bounded reliability follow-up remains from broad validation: PostgreSQL
currently loads native trust roots synchronously even for `TlsMode::Disable`.
Avoid that unnecessary work and prove disabled TLS never invokes the loader,
while preserving VerifyFull native/custom roots and hostname rejection. One
existing protocol readiness wait timed out and passed unchanged in isolation;
certificate loading is a plausible, unmeasured contributor, not an established
cause. Keep readiness diagnosis separate from cancellation/EOF deadlines.
The reconciliation increment reproduced both startup-cleanup dispatch timeouts
in full validation; unchanged serial and parallel protocol reruns pass. A bounded
follow-up can own `drivers/postgres.rs` only: gate native trust enumeration by
VerifyFull and inject its loader in deterministic tests proving Disable never
calls it. Preserve custom-CA validation, TLS setup, protocol assertions and
timeouts; then rerun affected protocol and full validation checks. This fix is
not yet implemented and must not be reported as resolving the unknown cause.

## Receipt recovery and qualification definition (11 November 2026)

Implemented and independently reviewed in `b06f030` / `3039975`: shared original
SDK DDL/DML/transaction outcome retention, local-only recording recovery, owned
transaction execution after a dropped caller, and a pure offline table-lifecycle
definition. Final validation passes 679 frontend / 463 standard Rust tests
(129 fixture tests ignored). Separate receipt, Studio, compiler, actual isolated
PostgreSQL and browser checks pass; see the [ledger](replacement-verification.md)
for scope and reproducible evidence. Account behavior remains unqualified.

Three hidden native runs and the final 224.90 MiB normal SDK bundle pass local
checks; all 430 frozen inputs remain unchanged through packaging. Strict ad-hoc
signing/resources and guarded SDK startup pass with zero authentication/SQL.
Installation, Developer ID/notarization and normal packaged account behavior
remain open.

Next useful local work is the existing-schema qualification infrastructure:

1. Export and enforce the exact physical SQL order/counts around production calls,
   including the bounded automatic read-only suffix after a rejected operation.
2. Intercept and freeze the raw schema census and created-object identity markers;
   enforce inbound frame/byte/stderr resource limits as well as outbound admission.
3. Bind the fixed runner to reviewed source, binary, runtime and private profile
   identities, with one-use approval consumption and a durable execution journal.
4. Regress interruption, rejected statements, local-recording failure, lost ACKs
   and partial runs without account access; independently review the result.
5. Prepare the exact executable plan for the owner's permitted new table under
   `SANDBOX.BILLIE_TESTING`, then request fresh possible-billing approval.

The [offline definition](snowflake-existing-schema-qualification.md) is not an
account runner or approval. In parallel, the complete metadata, SQL authoring,
grid/import, schema/performance, native lifecycle and million-row account gaps
remain required. Do not reinterpret this lifecycle check as replacement acceptance.

Bounded read-only integration review (Tier 2 generation
`2026-11-11T11:11:15Z`, best-effort coverage plus exact source) identifies these
next implementation seams; none is implemented by the offline compiler:

- Attach an opaque harness-only attempt capability through
  `Studio::connect_prepared_inner` / `SnowflakeExternal::open` before startup.
  Keep it out of desktop IPC and stored profiles.
- Admit and consume physical operations in `snowflake_process::write_loop`
  immediately before writing, after durable intent and ownership rechecks.
  Queue insertion is insufficient. Bootstrap includes the helper's hidden fixed
  context SELECT and version query; export their definitions instead of copying.
- Observe validated wire-order census rows before `parse_schemas` sorting hides
  order, and parsed `BrowseRelation` creation/DDL markers before public summaries
  discard them. Enforce these observations before dependent dispatch/publication.
- Count actual stdout bytes from handshake onward, complete frames and partial
  failures; concurrently drain/count stderr without retaining contents. Reap and
  counter-task completion are separate evidence. Preserve known ACKs on later
  read failure. The existing native query-submit guard remains mandatory and
  does not replace exact SQL/phase admission.

Split transport/observation plumbing, closed ledger enforcement, and the private
execution envelope/runner into reviewable increments. Required synthetic coverage
includes complete 393/610-execution production routes, census/marker drift,
rejected-operation suffixes, intent failure, cancellation/partial writes, resource
overflow after ACK and dropped callers with failed local recording. These tests
must precede any executable approval request.

## Recorded table DDL (11 November 2026, local integration)

Final evidence: 679 frontend / 456 standard Rust tests, 356 helper cases,
14 browser workflows, 43 isolated PostgreSQL tests and three hidden native runs
pass. The final 224.88 MiB normal bundle passes signature/resources/guarded SDK
startup; all 426 frozen source/config/test inputs remain unchanged. No install or
account mutation qualification is claimed. Details and artifact hashes are in
the [ledger](replacement-verification.md).

That review identified DML/transaction terminal persistence failures and the need
to specify automatic post-failure observations. The subsequent receipt increment
above reproduces and repairs the recording gap without account activity. A fresh
execution approval is not pending yet; the executable runner remains unfinished.

The SDK external-browser route now includes a separate opt-in for generated
CREATE TABLE, nullable ADD COLUMN and DROP TABLE. Every statement uses its own
short-lived review and one-use authority; DROP also requires the exact fully
qualified target name. ReadOnly and OAuth deny this route. Admission requires a
coherent idle AUTOCOMMIT session, supported catalog traits and fresh context.
Permanent and transient ordinary base tables are supported; transient objects
have no Fail-safe. DDL cannot be rolled back and can race with external actors.
This is a bounded implemented subset, not complete Snowflake DDL parity.

The UI binds drafts and approval to the owning console, retains acknowledgement
separately from metadata reconciliation, pauses automatic observers, and requires
an explicit successful catalog/context refresh before normal metadata resumes.
Unknown delivery retires without replay. Original typed outcome receipts survive
a dropped caller; failed local history/audit recording blocks further DDL and
normal shutdown until a local-only retry succeeds. Abrupt loss while SQLite cannot
write is not claimed crash-durable. See the [DDL contract](snowflake-sdk-ddl-design.md),
[walkthrough](migration-from-datagrip.md#reviewed-sdk-table-ddl-prepared-acceptance-walkthrough)
and [current verification](replacement-verification.md).

The earlier approved database-creation setup was consumed: SSO and initial
observations passed, CREATE DATABASE was rejected, and no later schema/table SQL
was dispatched. The owner now permits newly created tables in
`SANDBOX.BILLIE_TESTING`. The [new lifecycle proposal](snowflake-existing-schema-qualification.md)
records the complete budget and remaining runner work. A fresh exact plan and
possible-billing approval are required before account activity; the prior setup must not be replayed. Actual
account writes/DDL/transaction effects, normal packaged SSO, million-row account
performance, broader DDL/grid/import/authoring workflows and hands-on acceptance
remain required. The Goal stays active and incomplete; nothing is merged,
published or installed.

## Recorded SQL-write increment (11 November 2026, sources `eae1e81` / `fb091db`)

The SDK external-browser route now implements a separately opted-in, individually
reviewed single INSERT/UPDATE/DELETE workflow with scalar parameters, exact status
counts, one-use submission authority and durable outcome history. It targets the
fully qualified SQL name shown in review; concurrent DDL, temporary-name shadows,
defaults, policies and dependencies can change resolution/effects. There is no
immutable object-identity or safe grid-edit claim. Acknowledged success is separate
from committed data effects. Unknown outcomes retire without replay; ordinary
observer denial retains rollback recovery. Metadata remains paused until an
explicit coherent refresh. ReadOnly and OAuth still deny writes.

Local commits `eae1e81` and `fb091db` pass full `pnpm validate`: 671 frontend
and 426 standard Rust tests, with 129 fixture tests explicitly ignored. Focused
checks pass 51 Rust DML cases, then 15 final Studio cases including the reproduced
and repaired graceful-shutdown history gap. All 21 Studio/actual-helper synthetic
SDK fixtures, seven browser DML/transaction flows and 43 actual disposable
PostgreSQL regressions pass. Helper coverage is 255 passing cases across the 107
actual-SDK loopback and 148 remaining-helper runs. The recorded DML-source normal
macOS bundle (224.33 MiB), strict ad-hoc signature/resources/startup checks and
three hidden native runs pass with zero startup authentication/SQL. See the
[verification ledger](replacement-verification.md) for source pins and limits.
These tests do not qualify actual Snowflake writes, transaction effects or a
million-row account result. Broader DML, grid editing, DDL, AUTOCOMMIT changes and
all remaining applicable DataGrip capabilities remain required gaps.

The owner requested a [disposable setup plan](snowflake-disposable-setup.md).
The separate helper-only runner implements a fixed 28-SQL ledger with three
CREATEs, permanent one-use consumption and private acknowledgement receipts.
Its recorded local qualification preceded the consumed account attempt described
above. The original pure DDL generator checkpoint had 11 tests; the current
application workflow is recorded in the section above. A separate million-row
helper/driver/store/CSV bridge passes with a synthetic SDK; actual Snowflake
performance and complete account workflows remain unqualified.
The Goal remains active and incomplete; nothing is merged, published or installed.

## Recorded transaction increment (10 November 2026, source `929ae06`)

Local source now includes separately reviewed SDK BEGIN/COMMIT/ROLLBACK controls,
one-use authority, policy rechecks and rollback-only recovery. SDK profiles still
default ReadOnly; only explicitly selected external-browser policies can permit
BEGIN/COMMIT. OAuth remains ReadOnly. On that recorded source, DML, DDL and AUTOCOMMIT
changes remained blocked. The UI retains command acknowledgement separately from transaction
activity and verified data effects, and pauses automatic metadata after a control.
See the [transaction contract](snowflake-sdk-transactions-design.md).
The next required SQL-write increment follows the
[reviewed DML proposal](snowflake-sdk-dml-design.md): explicit opt-in, strict AST
admission, exact affected-count evidence and same-session recovery. It is a design,
not enabled write support; broader DML and DDL remain required.

Full `pnpm validate` passes: 659 frontend and 384 Rust tests, formatting,
production frontend build, Rust checks and strict all-target/all-feature Clippy.
The standard run reports 129 fixture tests ignored. Separate checks pass 31
actual-helper/synthetic-SDK internal adapter tests, 31 adapter integration tests
and 16 Studio/helper fixtures; all six affected cases also pass against the final
source. Seven Studio policy/race tests and actual local SQLite profile restart/
backup round trips pass. Three transaction browser flows pass, including keyboard
approval and 31-second quiet-window checks; seven related UI cases passed in the
prior run. These UI tests intercept RPCs and do not qualify account behavior.

The rebuilt isolated SDK native app passes three runs, 20 check categories and
ten cleanup cycles each, including typed IPC admission guards. The normal SDK
macOS bundle passes packaging, strict ad-hoc signature/resource checks and guarded
SDK startup: 224.14 MiB, 15,106 resource entries, no symlinks, zero authentication
or SQL requests. All 324 frozen inputs stay unchanged. This source is committed
as `929ae06`; it is unmerged, unpublished and not installed. No account transaction
statements or writes are authorized or executed for this increment. Actual
account effects, Developer ID/notarization and owner acceptance remain open.

## Confirmed owner target (9 November 2026)

The owner requires **Snowflake on macOS with SSO `authenticator=externalbrowser`**,
and **everything DataGrip currently provides for that workflow**, explicitly
including reads, writes, schema changes and multi-statement transactions.
The [Snowflake implementation queue](snowflake-datagrip-replacement.md) records
the required workflow inventory and qualification sequence.
This is not a read-only-pilot or MVP target. All applicable capabilities in the
official DataGrip baseline remain required until the owner explicitly excludes
one; existing exclusions are blockers, not optional enhancements.

The owner also requires the **latest stable DataGrip release** and Snowflake
query results up to **1,000,000 rows**. Current comparison: DataGrip 2026.3,
released 5 November 2026. Complete million-row paging/export with bounded memory
is required. The [opt-in temporary result store](large-results.md) now targets
one million rows with bounded memory/disk. Earlier actual PostgreSQL paging/export
and browser/native preference checks pass; their source and artifact identities
are recorded in the verification ledger.
Actual Snowflake million-row behavior remains unqualified.

Current Joinery implements explicit local-application OAuth/SQL API and stateful
SDK external-browser profile routes. OAuth restricts SQL to reads. SDK typed controls, separately opted-in bounded
SQL DML and separately opted-in generated table DDL use their dedicated review
paths; broader SQL/session commands remain restricted. SDK same-session
completion is implemented and locally qualified separately. A distinct
revision-bound relation catalog/inspection/read-only preview is now implemented
locally; see the [browsing contract](snowflake-sdk-browsing-design.md) and
[current qualification](replacement-verification.md#sdk-relation-browsing-10-november-2026).
Generic editable browsing, complete object/constraint inspection, broader writes/DDL
and complete transaction behavior remain incomplete. Full account/session compatibility remains
unqualified. These are primary replacement blockers. Local
PostgreSQL/MySQL safety repairs remain useful application work but do not close
Snowflake acceptance. External account qualification needs explicit non-production
scope and permission for any billable activity/writes; no private DataGrip state
or credentials will be inspected.

The current [session observer](snowflake-sdk-session-observation.md) implements
bounded, same-connection AUTOCOMMIT/policy/opaque-transaction observations,
nullable roles and explicit Unknown states. Stable observations preserve cached
plans; changed or unavailable observations revoke them. Local qualification
passes; the [ledger](replacement-verification.md#sdk-session-observation-10-november-2026)
records current tests, packaging and native evidence. Guarded SDK schema
selection and explicit transaction controls are locally qualified. Account
transaction effects remain unqualified.
The separately approved helper observation ledger passes on the actual account. Its ten/eleven additional SQL
executions are outside the earlier two-query canary approval.

An [offline qualification manifest](snowflake-qualification-manifest.md) is now
implemented and locally verified: 12 synthetic tests and actual CLI checks with
synthetic identifiers, current source/runtime validation and private exclusive
output. The distinct observer/options/scalar workflow has a complete 24-SQL
ledger. A separately reviewed [execution boundary](snowflake-qualification-runner.md)
now implements one-use approval consumption, source/runtime validation, bounded
child lifecycle and private evidence. Its 18 runner tests and the complete
113-test qualification suite pass. One freshly approved account attempt passed
SSO and the two startup queries, then stopped on a definitive rejection of its
session-context query. Logout, child reaping and temporary-state removal passed.
The approval is consumed; no retry was attempted. Diagnose and locally repair
the context query before preparing any new approved attempt. The SDK query now
uses documented TO_VARCHAR text normalization and passes focused regression,
independent review and full validation. A separately approved corrected attempt
then passed all 24 statements on the actual account at 12:59:21 UTC, including
context/settings, bounded schema discovery and scalar validation. See the
[account pass and its limits](replacement-verification.md#corrected-account-readmetadata-qualification-10-november-2026).
Normal application and complete account-workflow qualification remain open; see the
[attempt evidence](replacement-verification.md#approved-account-qualification-attempt-10-november-2026).

The [schema-switch transport foundation](snowflake-sdk-session-design.md) is
implemented: strict read-only context, separate typed USE/readback operation,
retained USE acknowledgement and conservative retirement on uncertainty. Full
validation passes **627 frontend / 363 standard Rust** tests; 169 helper/guard/
actual-SDK loopback tests, 33 process fixtures, 30 synthetic-SDK adapter fixtures
and three focused browser workflows pass. Fresh normal SDK macOS packaging,
ad-hoc signature and packaged hello/EOF startup pass with zero authentication/SQL;
hidden-native checks will accompany the dependent IPC/UI slice. The revision-bound
adapter, typed IPC and guarded UI selector are now implemented under the
[current qualification](replacement-verification.md#sdk-guarded-schema-selection-10-november-2026).
Account switch acceptance remains required and incomplete. No public writes,
DDL or transaction actions are enabled by this foundation.

The [explicit-transaction transport foundation](snowflake-sdk-transactions-design.md)
is implemented and locally tested. It separates a boundary command's
acknowledgement from verified database disposition. The dependent adapter now retains same-session rollback recovery when an aborted
transaction rejects observer reads. The complete controls pass local application
qualification on `929ae06`; actual transaction effects remain unqualified.

At `c543ef5`, the transaction transport and synthetic-function policy repairs
also pass fresh normal SDK macOS packaging, ad-hoc signing/resources and guarded
hello/EOF startup with zero authentication/SQL. Three hidden native runs pass
19 check categories and ten cleanup cycles each, with all 322 frozen inputs
unchanged. This does not qualify actual transactions or enable their UI; current
[artifact evidence](replacement-verification.md#transaction-increment-package-and-native-checks-10-november-2026)
records the runnable local bundle and its limits.

The preceding relation-browsing increment is locally qualified: **619 frontend / 338
standard Rust** tests, **29** synthetic-SDK actual-helper adapter tests, the
application/helper browsing flow, **11** focused browser workflows and an actual
PostgreSQL context/transaction regression pass. Fresh normal SDK macOS packaging,
signature/startup and **19** hidden native check categories with ten cleanup
cycles pass. The ledger records source hashes, artifact identities and explicit
account/installed-application limits. This does not qualify Snowflake account
metadata or enable writes, DDL or transactions.

The current [SDK application integration](snowflake-sdk-integration.md) adds explicit
authentication modes, fixed native runtime selection, retained cleanup authority,
pending launch permits and a reviewed opt-in resource-packaging route. Focused
checks and combined validation pass (604 frontend / 297 standard Rust tests),
as do 124 actual fixture tests and normal SDK resource-bearing macOS packaging,
ad-hoc signature and packaged SDK hello/EOF startup. All 143 browser workflows and 19 hidden SDK native checks with ten cleanup
cycles pass, as do two standalone PostgreSQL native recovery pairs.
The subsequent [SDK same-session completion](snowflake-sdk-metadata-design.md)
passes local qualification: 607 frontend / 312 standard Rust tests, 124 real-engine
regressions, all 145 browser workflows, fresh SDK-bearing packaging/startup and
19 hidden native check categories. Complete browsing/session context and the
required write/transaction workflows remain in the queue. No target capability
was made optional.
A separately approved two-query SDK helper canary now passes actual SSO,
configured context, session continuity, synthetic scalar values and local
cleanup. The ledger preserves its first validator failure and corrected rerun;
broader account SQL and writes still require concrete authorization.

The default Snowflake SQL starter repair reproduces the exact script's
`PolicyBlocked` failure in both policy and actual-helper tests, then adds strict
Snowflake-only zero-argument `CURRENT_USER()` / `CURRENT_VERSION()` observers.
Qualified/quoted functions, unknown functions and relative-history forms remain
blocked. All 19 safety and 19 synthetic-SDK actual-helper tests pass, as do
607 frontend / 314 standard Rust tests and full validation; see the
[repair evidence](replacement-verification.md#snowflake-starter-repair-10-november-2026-local-checks).
Complete browsing and authoritative session state remain required.

The reviewed [browsing/session proposal](snowflake-sdk-browsing-design.md)
defines the catalog → inspect → bounded preview → refresh workflow, qualified
targets and truthful unknown transaction/context states. The separate
[submission guard](snowflake-sdk-submit-guard-design.md) now implements a
process-local boundary against SDK retransmission and passes local qualification:
159 helper tests, 607 frontend / 314 standard Rust tests, focused browser flows,
fresh SDK-bearing macOS packaging/startup and 19 hidden native check categories.
The browsing contract now has a local catalog/column-inspection/preview
implementation. Truthful session observation is now locally qualified;
typed schema switching is now locally qualified under the
reviewed [session observation/switching contract](snowflake-sdk-session-design.md);
the complete browsing/session workflow is not finished. Neither is
account qualified. The guard's actual pinned-SDK fault, redirect, proxy and cleanup
evidence establishes a bounded submission foundation; that foundation alone
must not enable writes or transactions.

The preceding stateful transport foundation is in the repository: a pinned SDK helper
passes 31 local tests (six use the actual SDK against loopback HTTP), and its
Rust supervisor passes eight unit and 30 integration tests. The latter includes
explicit cleanup receipts for unused/failed launches and actual child reaping.
The read adapter adds 11 integration tests, including exact typed binding and
terminal/cancellation ordering. Nine runtime-preparation tests, actual offline
macOS arm64 staging and staged SDK startup pass; the stage is not an app bundle.
The [driver design](snowflake-driver-design.md) records the observed SDK retry
behavior and remaining gates. Those foundation counts predate the current profile/cleanup integration. Current
resource packaging now passes locally. Normal packaged SSO/session behavior,
writes and transactions retain separate open qualification gates.

The preceding lifecycle increment added reserved connection-attempt identifiers, cancellation
before/during authentication, exact retirement of late responses, and a native
shutdown publication barrier. Focused UI tests pass; independent review exposed
startup socket/task ownership gaps in PostgreSQL, SSH and MySQL. Their focused
repairs and 124 actual database fixtures now pass. Combined qualification passes
598 frontend / 262 standard Rust tests, all 141 browser workflows, three native
smoke runs, two actual PostgreSQL recovery pairs and the normal ad-hoc macOS
bundle. This is prerequisite lifecycle work for
SDK integration; it does not implement externalbrowser SSO. See the
[active verification record](replacement-verification.md#connection-startup-increment-10-november-2026-locally-qualified).
The [connection ownership contract](connection-lifecycle.md) records cancellation,
preflight, resource bounds and shutdown limits. The current SDK integration implements profile routing and retained cleanup;
hidden native qualification now passes. SDK completion now has an implementation;
complete session metadata, browsing and the required write/transaction workflows
remain open.

The active 9 November 2026 Goal and next qualification queue are in
[daily replacement acceptance](replacement-acceptance.md). The required Snowflake externalbrowser/macOS target is confirmed above;
historical slice completion does not establish daily replacement acceptance.

The earlier P0 repairs cover query-result provenance, truthful bounded snapshots,
abrupt backend restart, manual transaction outcomes and delayed result inspection.
Their historical qualification remains below and in the ledger. The current
identity-safety increment has separate, narrower evidence and open gates.
The first pinned run passed validation, 91 fixtures, bundle and native smoke;
116/118 browser workflows passed and the two failures were reproduced. The
repaired runtime passed normal macOS packaging/signature, three isolated
native processes and 120/120 browser workflows. The repairs have independent
reviews; no material finding remains. The native-test-only follow-up `a1ab58b`
passes full validation (542 frontend / 180 standard Rust), two isolated actual
native close/reopen pairs against PostgreSQL, six runner acceptance tests and
ordinary smoke compatibility. Pending drafts/preferences survive close, manual
writes roll back and native invocation counters show no replay before observation.
Normal packaged restart and credentials remain unqualified. Next: confirm
required workflows and engine/authentication pairs, qualify authorized external combinations, and
obtain the owner walkthrough acceptance. Follow the
[verification ledger](replacement-verification.md); source presence and old
slice test counts do not close these gates.

The reviewed-writer safety follow-up repairs synchronous PostgreSQL retirement,
blocks queued writes after session closure, and fixes CSV unknown-outcome text
and editor retirement. Real regressions reproduced the defects before repair;
independent reviews found no material issues. Validation (545 frontend / 180
standard Rust), 103 real fixtures and four new fault browser workflows pass.
Combined packaging/signature, two native recovery pairs, ordinary smoke and all
124 browser workflows pass for that earlier source. A separate diagnostic on
`ff59ca6` reproduced 27 drop/recreate identity failures (nine each on PostgreSQL,
MySQL and MariaDB), including deletes, empty-result inserts and CSV between batch
commits. The PostgreSQL identity repair is committed locally and the current
MySQL/MariaDB increment implements bounded retained metadata-lock guards,
fail-closed original-generation checks and exact CSV preview ownership.
Preliminary integrated validation passed 580 frontend / 193 standard Rust,
123 real fixtures, 27/27 strict three-engine identity scenarios and all 137 browser
workflows. A later actual PostgreSQL regression exposed tokenless CSV association
with a newer preview; the legacy path is removed and named IPC requires the exact
token. Verified TLS-over-SSH now exercises actual guarded browse and approved
writes on both MySQL and MariaDB. Final source checks for local code `997680e`
pass 581 frontend / 193 standard Rust, 124 actual fixtures, all 27 strict scenarios
and all 137 browser workflows. Clean-source `6218e60` hidden native recovery,
ordinary smoke and normal ad-hoc macOS bundle qualification also pass; see the
ledger for artifact identities and limits. Historical `5762ee5` native/bundle
qualification belongs to the earlier PostgreSQL increment. See the
[identity investigation](table-identity-safety.md) and
[retained-guard contract](mysql-table-identity-design.md).

Next after local integration: implement the confirmed Snowflake externalbrowser
stateful transport and million-row result path; qualify all applicable DataGrip
2026.3 capabilities, performance and accessibility;
complete normal packaged restart/credentials and the hands-on walkthrough.
The Goal remains active until acceptance. No push, installation or publication
is authorized. The owner authorized reuse of the existing Snowflake connection;
only its non-secret configuration was inspected. Other private application state
and all external query/write qualification remain outside that authorization.

This delivery sequence supplements [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).
The [feature matrix](datagrip-feature-matrix.md) records engineering evidence;
the owner-private [project hub](https://joinery-project-hub.otohime-nokimi.chatgpt.site)
records shipping status. Priority P0–P3 is not the blueprint's P0–P7 milestone
number. Existing released capabilities and external gates retain their history.

## Agreed first batch

The user selected SQL correctness, then a bounded repair across existing SQL
engines before live role/search-path tracking or JOIN generation. Baseline source
is `bbba421`; baseline validation and two synthetic Snowflake browser workflows
passed. Initial shell-toolchain and Chromium sandbox blockers were environmental
and resolved. No production access is needed or authorized.

| Slice | Value, priority, complexity | Dependencies and observable acceptance |
|---|---|---|
| Completion scope/quoting/invalidation (E01, E03–E05) | Trust the identifiers inserted while writing SQL; P1, Medium | Existing CodeMirror and worker. Ignore comments/strings, isolate caret query block, respect CTE/derived shadows, preserve quoted identity and insertion, reject obsolete responses. Regression corpus and browser acceptance pass |
| Live completion context (E02, I04, M05) | Match the actual SQL session; P1, High | Rust-owned context DTO/revision and engine adapters. Database, authenticated identity, role and search-path changes invalidate caches; no metadata reuse across contexts or automatic replay |
| Console/session binding (M06, E14, X04) | Make transaction ownership visible; P1, High | Existing four-session backend limit. Explicit console binding; cancel/disconnect affects only its owned session; grid commits remain independent |
| FK metadata and JOIN completion (N03, E06) | Write correct joins quickly; P1, High | Constraint identity plus ordered source/target columns, then scoped identifier insertion. Composite/multiple/self-referencing relationships produce correct reviewed text without executing it |
| Diagnostics and parameters (E07, E15, X08) | Find errors and rerun safely; P1, High | Driver error positions, Unicode offset mapping, typed binding contracts. Exact statement navigation and bindings included in immutable authorization plans |

### First-slice implementation contract

- Keep Rust connections, metadata, credentials, execution and approval ownership.
  No Tauri command or persistence migration is required for this repair.
- Use CodeMirror dialect token trees and a bounded identifier/scope reader.
  Current query blocks expose direct base-table bindings only. CTE and derived
  aliases shadow physical tables; derived outputs and outer correlated bindings
  are deliberately not inferred. Incomplete/unsupported analysis suppresses
  contextual columns rather than guessing.
- Preserve original identifier segments. PostgreSQL folds unquoted identifiers
  to lowercase; Snowflake to uppercase. MySQL matches exact catalog spellings
  first and declines ambiguous case-folded matches. Known configured default
  schemas retain precedence; ambiguous unresolved table names require qualification.
- Keep keyword/catalog lookup in the existing worker. Distinguish document,
  context and installed-index revisions; do not rebuild the catalog on ordinary
  keystrokes. Worker responses must identify the installed index, and the editor
  must reject stale document/caret/session/metadata results.
- Retain at most eight active referenced-table requests and 100 returned
  suggestions. Clear obsolete cache entries; an older failed request must not
  delete its replacement. A denied table must not erase successful metadata for
  other current bindings.
- Insert valid dialect quoting, preserving surrounding SQL and UTF-16 ranges.
  PostgreSQL/MySQL/Snowflake share the existing architecture; Redis keeps its
  native grammar and command completion.

### First-slice result (2026-10-28)

The bounded repair is implemented locally. See the matrix's final evidence
section for scope and outcomes: 59 frontend tests, 83 Rust tests, 20 browser
workflows and the six-workflow final completion/Snowflake rerun pass. Three
selected read-only Rust integration workflows additionally verify cleanup,
metadata paging and TLS hostname checks. Native packaging results are recorded
in `IMPLEMENTATION_STATUS.md`. That repair subsequently shipped in preview.12
through PR #20 (`06146bc`); its original local evidence remains historical.

The 100,000-name index meets the warm 5 ms lookup target in the measured browser
sample (p95 0.10 ms). Warm explicit visible completion misses the 50 ms target
(p95 55–61 ms); the installed CodeMirror scheduler adds a fixed 50 ms before
requesting explicit suggestions. Treat visible scheduling/typing latency as a
measured follow-up, without patching dependency internals or claiming native
performance qualification. Keep cold metadata waits and large-catalog visible
measurements separate.

The PostgreSQL live-context slice shipped in preview.13 (PR #21, `e8f1cec`); see
[live-completion-context.md](live-completion-context.md) for the actual contract,
resource bounds and verification scope. E02/I04 remain PARTIAL across all engines.
The [MySQL/MariaDB live adapter](mysql-live-completion-context.md) shipped in
preview.14 (PR #22, `dc220b0`), with mode-0 fixtures and clean-source release
evidence. PR #23 adds mode-1 qualification on both disposable engines.
Mode 2 needs a case-insensitive filesystem. The bounded
[Snowflake request-context slice](snowflake-completion-context.md) shipped in
preview.15; broader search-path/quoted-policy support and live qualification remain
open. External metadata changes still require refresh.
The [isolated query-session slice](isolated-query-sessions.md) merged on `main`
via PR #26 (`877904b`, all six PR checks passing with 79 frontend tests and 31
browser workflows) for PostgreSQL/MySQL/MariaDB using existing Rust sessions. It
remains unreleased; clean-source release provenance and grouped-FK JOIN
generation follow.
External release gates remain open and need their own authorization/scope.

### Acceptance and verification contract

1. Reproduce and then fix references in comments, strings, dollar quotes and
   separate statements; cover semicolons in literals, CTE/derived shadows, UNION
   arms, nested blocks, aliases and malformed/incomplete documents.
2. Exercise case twins, embedded dots/delimiters, Unicode, qualified names,
   same-name schemas and quoted-prefix insertion. Accept a suggestion and run a
   harmless SELECT against disposable PostgreSQL/MySQL/MariaDB fixtures; use
   synthetic Snowflake API responses and disclose that boundary.
3. Delay metadata/worker responses while editing, moving the caret, refreshing
   metadata, switching connections and closing tabs. Verify obsolete responses
   disappear, promises/timers settle, and partial metadata errors degrade safely.
4. Measure warm prefix lookup on 100,000 names and visible suggestions separately.
   Compare with the blueprint's p95 5 ms lookup/50 ms visible targets; report
   environment and any failures without generalizing browser numbers to WebView.
5. Run `pnpm validate`, relevant integration/browser tests, `pnpm desktop:build`,
   `pnpm desktop:bundle:mac`, and hidden native smoke. Record Passed, Failed,
   Skipped, Blocked by environment, and Not executed separately.

## DataGrip replacement program (approved 2026-10-30)

The owner asked for Joinery to replace DataGrip on the existing engines
(PostgreSQL, MySQL/MariaDB, Snowflake, Redis), and approved this
milestone order. It supersedes the order of the "Subsequent phases" table
below without changing its exit criteria. Console/session binding moves to R4
so table-data parity, which also supplies grouped FK metadata for JOIN
completion and diagrams, ships first. Each batch needs its own approved plan,
fixture evidence and PR.

| Milestone | Scope (feature IDs) | Exit criteria |
|---|---|---|
| R1 Data view parity | Batch 1: N03, G03 typed, G08 outbound. Batch 2: G05, G09, G10, G07, raw WHERE/ORDER BY text classified by `safety::prepare`, inbound referencing rows | Sorted/filtered pages equal reference queries; composite FKs navigate; paste all-or-nothing; inserts/deletes via one-use plans |
| R2 Data movement | G11 export, copy-as formats, G12 CSV import, G13 data compare | Exact values round-trip; cancelled exports marked partial; no result rows in app data |
| R3 Explorer and search | N01, N02, N04–N07, P01 | Keyboard tree; quoted/overloaded names; per-object permission degradation |
| R4 Consoles, sessions, files | M05, M06, X04, E14, P02, P03 | Isolated console transactions/cancellation; unsaved-file guard |
| R5 Editor intelligence | E06, E07/X08, E15, E11, E13, E09, E12, E16 | Correct composite JOIN text; multibyte error carets; bindings in approval digest |
| R6 Query plans | X07, A01, A02, X05 | ANALYZE explicitly authorized; estimates and actuals distinct |
| R7 Database design | D01–D05, N08 | DDL round-trips on disposable schemas; no unattended apply |
| R8 Productivity and monitoring | P05, P06, A03–A05, E08, E10 | Shortcut conflicts detected; monitoring permission-aware |
| RX Restriction relaxation | G14 single-table result editing, PK-less row editing, Snowflake DML (A07) | Rust-proven provenance; one-use plans and production passphrase retained; Snowflake writes qualified only on an approved non-production account |
| v1.0 gates | I05–I07 | Unchanged external signing, platform and performance gates |

### R1 batch 1 result (merged in PR #25 `6445b0e`; unreleased)

- FK constraints carry name and ordinal-paired columns; Structure lists them.
- `browse_table` accepts an optional typed `BrowseQuery`. Rust rebuilds the SQL
  from the current schema with bound values, PK tie-breakers, explicit NULL
  order (ascending last, descending first) and a NULL-safe keyset seek.
  Engines without `tableFiltering` (Snowflake, Redis) reject non-empty queries.
- Header clicks sort (Shift adds a key); a filter bar builds typed predicates;
  controls lock while edits are pending. ⇧⌘B, the palette or the inspector
  opens the referenced row in a new tab, with a chooser for multi-key columns.
- Two pre-existing defects found by the new tests are fixed separately:
  MySQL read-only profiles could not browse editable tables, and the frozen
  primary-key column stayed blank for pages loaded after scrolling.

See the matrix rows N03/G03/G08 and `IMPLEMENTATION_STATUS.md` for evidence.
Merged on `main` (`6445b0e`), unreleased.

### R1 batch 2 result (2026-10-30, local branch `feat/table-data-editing`)

- Optional raw `WHERE` and `ORDER BY` clauses on PostgreSQL and MySQL/MariaDB
  table tabs are synthesized and classified via `safety::prepare` (blocked in
  `ReadOnly` when non-read/unparseable; gated by one-use `ReviewDialog` approval
  in writable modes; raw `ORDER BY` disables keyset continuation cleanly).
- Permission-filtered inbound `referencing_foreign_keys` metadata powers `⇧⌘G`
  navigation to referencing rows in a new filtered tab across single, composite
  and multi-constraint relationships.
- Column hide (non-PK), reorder, resize and reset persist per
  `connectionId + schema + table` in bounded `SavedWorkspace.table_layouts`
  (purged on profile delete and reconciled on schema refresh).
- Multi-cell TSV paste validates every target cell and budget atomically before
  mutating `DirtyStore`, respecting reordered visible columns.
- Draft row inserts (`<default>` / `<required>`) and row deletes (`FOR UPDATE`
  full-row conflict checks) execute alongside updates in one atomic reviewed
  `BEGIN … COMMIT` transaction across PostgreSQL, MySQL 8.4 and MariaDB 11.4.
- `ValueDialog` provides 16-byte-per-row binary Hex/ASCII dumps, 1-based indexed
  PostgreSQL array element viewers (multi-dimensional/custom bounds, SQL `NULL`
  vs `"NULL"` vs `""`) and lossless token-level formatted JSON preserving
  64-bit integers and numeric object key order.

See matrix rows G03/G05/G07–G10 and `IMPLEMENTATION_STATUS.md` for evidence
(`99` frontend tests, `102` standard Rust tests, `41` disposable integration
tests, `37` Playwright workflows and clean-commit macOS native gates). Merged on
`main` via PR #27 (`94f63ba`); unreleased.

### R2 Data movement result (merged in PR #28 `c8d8aa1`; unreleased)

- Grid selection Copy-as (`CSV`, `TSV`, `JSON`, `NDJSON`, `SQL INSERT`,
  `Markdown`; max 5,000 rows / 2 MiB), capability-gated Rust `.partial` streaming
  export (`CSV`, `TSV`, `JSON`, `NDJSON`; max 1,000,000 rows / 512 MiB), upfront-validated
  capability-gated CSV/TSV import into reviewed 500-row `ChangePlan` batches, and
  key-aware bounded `LiveTable` / `RetainedResult` data comparison (including
  cross-engine PostgreSQL ↔ MySQL/MariaDB).
- Merged on `main` via PR #28 (`c8d8aa1`, all six PR checks green); unreleased.

### R3 Explorer and object search result (merged in PR #29 `2625933`; unreleased)

- Categorized WAI-ARIA keyboard object explorer (`Tables`, `Views`, `Materialized
  views`, `Functions`, `Procedures`, `Sequences`, `Triggers`, `Indexes`) with
  kind filter chips, bounded 256 KiB read-only DDL (`Copy DDL`, `Open in SQL
  editor`), bounded 200-edge 1-hop `Uses` / `Used by` dependency chips, and
  cross-connection `SearchEverywhere` (`⌘K` / `⇧⌘O`).
- Merged on `main` via PR #29 (`2625933`, all six PR checks green); unreleased.

### R4 Consoles, sessions and files result (merged in PR #30 `a0ba1a6`; unreleased)

- Per-session active schema/database switcher (`M05`: PostgreSQL `search_path`,
  MySQL/MariaDB `USE`, Snowflake stateless request-scoped schema verification)
  invalidating and refreshing live CodeMirror completion per session.
- Named & attachable multi-console session pool (`M06`: up to four total sessions
  per connection, multi-console attachment to named secondary sessions or `Shared
  session`, rename/disconnect controls, last-owner close lifecycle).
- Per-session `Auto-commit` vs `Manual commit` transaction mode and explicit
  `Commit` / `Rollback` controls (`X04`) driven by wire-level transaction state
  (`Idle` | `InTransaction` | `Failed`) on PostgreSQL and MySQL/MariaDB while
  keeping table-grid edits isolated on separate lanes.
- Capability-gated `.sql` file Open (`⌘O`), atomic in-place Save (`⌘S` on query
  tabs; `reviewEdits` preserved on table tabs) and Save As (`⇧⌘S`) via reusable
  `GrantKind::SqlFile` tokens (`E14`: ≤ 4 MiB UTF-8, dirty indicator, unsaved-close
  guard, zero raw paths over IPC, excluded from `SavedWorkspace.drafts`).
- Local SQLite saved-queries library (`P03`: max 256, ≤ 256 KiB each / 4 MiB
  total, privacy-safe detached default when `storeQueryText === false`,
  `SearchEverywhere` `Saved` scope + `LibraryModal`), persistent object &
  saved-query bookmarks (`P02`: `⌘D`, max 256, profile-delete cascade), and a
  50-item in-memory Recent locations ring buffer (`⇧⌘E`).
- Merged on `main` via PR #30 (`a0ba1a6`) on 31 October; unreleased.

## Assessment and next slices (2 November 2026)

R1–R4 are merged but absent from the last verified preview. Recorded R5 commit
`bb77b0f` remains readable; its former worktree/branch are now absent and former
dirty documentation is not recovered. Historical counts (146 frontend, 139 Rust,
56 fixture and 49 browser passes plus macOS gates) are not current qualification.
Integration is proceeding by reviewed feature group on the R6 branch.

Current candidate `51ad451` integrates six R5 slices plus formatting, named
outputs, console-bound inspectors, bounded alias/CTE rename, qualified-column
inspection/correction across ordered direct-table JOINs, and context-aware live
templates, alongside seven R6 slices. R5/R6 remain partial, local, unmerged,
unreleased and not installed. Templates expand observed database/schema
identifiers and literal selected SQL from the active console, with tab stops
and one undo, and never prepare or execute SQL. The focused template checks are
31 frontend tests, one Rust snippet-backup test, and six PostgreSQL/MySQL/MariaDB
browser flows. The prior JOIN candidate passed 379 frontend / 156 standard Rust
tests and its focused browser flows; those full checks were not rerun here.
Next: broader CTE/derived/unqualified inspection and semantic template placement,
then remaining R5/R6/R7/R8 and delivery gates. See
[current evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-context-aware-live-templates-2-november-2026).

### R5 integration slice 1: live FK JOIN completion (local `e0dad5f`)

Recorded `40c59a7` plus the SQL alias correction from `41957ea` now coexist with
the seven R6 slices. Permission-filtered outbound/inbound metadata preserves
composite pairs and temporary shadows. Scoped JOIN/initial ON suggestions handle
multiple/self relationships, aliases and quoting without executing SQL.
Snowflake FK metadata and advanced output inference remain outside this slice.

Validation passes 152 frontend / 136 standard Rust tests, two live metadata
fixture tests spanning PostgreSQL/MySQL/MariaDB, and fourteen completion/
Snowflake browser flows, including nine inserted-and-executed composite joins.
The browser check caught and fixed the PostgreSQL built-in-token alias defect.
Native/package gates and full integration suites were not rerun on this source.
R5 remains partial, local, unmerged and unreleased; see
[exact evidence](IMPLEMENTATION_STATUS.md#r5-integration-live-fk-join-completion-1-november-2026).

### R5 integration slice 2: Unicode diagnostics (local `ce7a7fa`)

Recorded `581c527` now coexists with the current R6 safety and cancellation
paths. Error DTOs and engine adapters retain UTF-16 ranges, statement identity
and available server diagnostics. PostgreSQL positions are character-based;
MySQL/MariaDB use best-effort syntax token/line ranges or whole-statement
fallback. Leading whitespace and unclassified-batch origins are covered by
live regression tests. Direct Snowflake wrapper validation shifts locations
once; Snowflake server diagnostics remain synthetic qualification.

All 54 SQL fixture tests and 22 diagnostic/FK/plan browser flows pass, including
mandatory analysis review and cancellation. Standard validation and three hidden
native runs pass for the exact code/tree. Inline markers/navigation and typed
parameters remain pending, including editor-selection and generated-wrapper
origin mapping. See [exact evidence](IMPLEMENTATION_STATUS.md#r5-integration-structured-unicode-diagnostics-1-november-2026).

### R5 integration slice 3: typed parameters (local `39359e7`)

Recorded `9cfefa2` now uses the common Rust plan store for query execution,
Explain and Analyze. Named/positional scanning excludes SQL strings/comments;
all values/types/NULL are validated before review. Bindings are immutable and
part of approval identity. Parameter drafts stay transient per tab, and history
keeps templates only. Snapshot comparison rejects different canonical bindings.
PostgreSQL uses typed SQL PREPARE/EXECUTE with encoded constants; MySQL/MariaDB
use protocol parameters; Snowflake SQL API bindings have synthetic coverage only.

Rewritten error locations map back to original placeholders; generated Explain
prefixes map to their input query. Inline markers and selection-to-editor origin
mapping remain pending. All 58 SQL fixture tests and 23 parameter/plan/timing/
diagnostic browser flows pass, plus 156 frontend and 154 standard Rust tests.
Clean-source native evidence and precise scope are in
[implementation evidence](IMPLEMENTATION_STATUS.md#r5-integration-typed-query-parameters-1-november-2026).

### R5 integration slice 4: inline diagnostics/navigation (local `65530c0`)

Recorded `655d8b7` now coexists with parameters and R6 preparation/approval paths.
Wavy marks, error gutters, SQLSTATE/location details and Jump to error use exact
submitted source identity. Current-statement and selection offsets pass through
parameter/review dialogs. Original placeholder and generated Explain input
coordinates are shifted once; CRLF/bare CR map to CodeMirror newline units.
Changed documents and malformed/out-of-slice locations cannot receive marks.
Statement navigation uses the active dialect parser and has no lexical guess
while parsing remains incomplete. Redis command behavior is preserved.

Twelve focused unit cases and six diagnostic browser flows pass, including
parameterized Run/Explain/Analyze and a late error after an edit. Twenty existing
parameter/plan/timing browser flows and standard validation also pass. See
[exact evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-inline-diagnostics-and-navigation-1-november-2026).

### R5 integration slice 5: definitions/usages/documentation (local `8ef9dfb`)

Recorded `b5a744d` now coexists with R6 and exact execution origins. F12 and
Cmd/Ctrl-click navigate CTE/table-alias declarations and primary-session object
structure; Shift+F12 lists scoped editor references; F1 shows current-session
metadata. Nested shadows, correlated bindings, unknown/ambiguous columns and
routine overloads are handled conservatively. Projection inference and complete
semantic SQL analysis remain open. Database dependents are object-level results.

Edits, refreshes, session/context changes and close reject obsolete replies.
Secondary consoles retain quick docs/local declaration jumps; object Structure
navigation is withheld because Structure currently uses the primary session.
Reference results have a visible 500-entry / 10,000-name-token bound. Standard
validation and 46 browser flows pass, including five new symbol flows; clean-source
macOS baseline passes three fresh processes / 30 cleanup cycles. See
[exact evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-sql-definitions-usages-and-documentation-1-november-2026).

### R5 integration slice 6: formatting/settings/snippets (local `db5cd1b`)

Recorded `3ca2ad2` now coexists with R6, with serialized format saves addressing
`41957ea` and relevant `bb77b0f` native guards. Selection formatting preserves
unselected text/newlines, undo and bind syntax; obsolete worker replies are
rejected after any document edit or session change. Global style preferences
persist with visible errors and rollback to the last successful save. Per-dialect
style overrides remain open.

Built-in and custom SQL templates use completion and keyboard tab stops without
automatic execution. Custom engine scope overrides global/built-in triggers.
SQLite bounds/validates templates and includes them in metadata recovery.
Context-aware variables and complete semantic placement remain open. Full
standard validation, all 86 local browser flows and clean-source macOS hidden
baseline (three processes / 30 cleanup cycles) pass. See
[evidence and limits](IMPLEMENTATION_STATUS.md#r5-integration-formatting-and-snippets-2-november-2026).

### R5 style follow-up (local `d39b89b`)

Preferences supports global defaults and independent PostgreSQL, MySQL/MariaDB
and Snowflake styles. First customization snapshots the global case/indent/
spacing; reset resumes inheritance. The console dialect automatically selects
the style. Rust migrates legacy settings and validates all styles on save and
metadata recovery. Three real database browser flows verify reload, automatic
selection and reset; standard checks and the clean-source native persistence/
worker baseline pass. Broader style controls and context-aware template variables
remain open. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-per-engine-formatting-styles-2-november-2026).

### R5 named-output follow-up (local `c32f0c2`)

Directly named SELECT columns now resolve through CTEs and derived tables using
current-session metadata. Full explicit CTE column lists are authoritative.
Definitions jump to local declarations, usages retain scoped identities and docs
label source metadata. Nested/quoted names, ambiguity, unavailable metadata and
unsupported shapes have regressions; wildcards, expressions and set-operation
outputs remain withheld. Metadata follows bodies within the existing fetch cap.
Inference has explicit recursion/body/output/token limits and action-local memoization.
Standard checks, five real navigation browser flows and the native synthetic
baseline pass. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-named-query-outputs-2-november-2026).

### R5 console-bound Structure follow-up (local `fe518fb`)

Primary and isolated consoles now open separate read-only Structure/DDL tabs
retaining the originating session and observed catalog revision. Metadata never
falls back to the primary session; pre/post revision checks and source lifecycle
guards reject stale replies. Column highlighting, explicit detail-denial fallback
and supported non-tabular inspectors are retained without browsing table data or
executing SQL. Existing inspector links and DDL-to-editor actions are withheld in
this path until they preserve the origin. Standard checks, six real navigation
flows and the native synthetic baseline pass. See [evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-console-bound-structure-2-november-2026).

### R5 context-preserving inspector actions (local `34c001a`)

Foreign-key, dependency, owner-table, index and trigger links now retain the
frozen originating console/session/context. Explicit DDL append preserves source
SQL, file dirty tracking and session ownership; it never executes. Fresh catalog
checks reject stale/denied actions, source lifecycle changes and obsolete replies;
queued/running consoles cannot receive DDL. Standard checks, six navigation flows,
one real inspector fixture and the native synthetic baseline pass. See
[evidence and limits](IMPLEMENTATION_STATUS.md#r5-follow-up-context-preserving-inspector-actions-2-november-2026).

### R5 bounded alias rename (local `e052626`)

Explicit preview / Apply changes only the selected SELECT alias declaration and
proven qualified references, with dialect quoting, nested shadowing, correlated
references, CTE/derived boundaries, capture checks and one-step undo. Stale
text/session/context and unsupported shapes are withheld; no SQL executes.
Standard checks, nine focused browser flows and the native synthetic baseline
pass. See [bounds and evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-bounded-alias-rename-2-november-2026).

### R5 ordered JOIN alias visibility (local `1edc2d7`)

Each ON clause and its correlated subqueries use the JOIN's actual operands;
later aliases and earlier comma groups cannot hide valid outer references.
Pending JOIN conditions preserve nested association; completed CROSS/NATURAL/
USING joins retain their operands. MySQL CROSS conditions stay supported.
Capture simulation uses the same environments. The full preview/Apply, quoted
identifier, stale-document/context and one-undo guarantees remain in place.
Standard checks and twelve focused browser flows pass; three new flows execute
only explicit test reads before/after rename across all three disposable engines.
No new native, full-suite, distribution or external qualification is claimed.
See [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-ordered-join-alias-visibility-2-november-2026).

### R5 bounded CTE rename (local `072e4ff`)

Shift-F6 / Rename alias / CTE previews the declaration, proven relation uses and
unaliased qualifiers; explicit relation aliases retain their independent identity.
Ordered WITH visibility, PostgreSQL nested shadowing, physical-table separation,
quoted delimiters, output lists and proposed-name capture have regressions.
Recursive and unsupported shapes stay withheld. The shared MySQL/MariaDB dialect
also withholds nested WITH beneath an outer CTE scope: direct probes on the
11.4 fixture fail to resolve outer CTEs in nested definitions where MySQL 8.4
and PostgreSQL 18 succeed. No version-specific semantic rule is guessed.
Standard checks, fifteen focused browser flows and the clean native synthetic
baseline pass. Browser checks compare decoded result values before/after explicit
rename; preview/Apply never prepares or executes SQL. See
[exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-bounded-cte-rename-2-november-2026).

### R5 bounded qualified-column inspections (local `cfb6139`)

Inspect columns checks qualified references in one direct-table SELECT on
PostgreSQL/MySQL/MariaDB. Fresh inspection metadata is stricter than completion:
partial column grants, missing visibility, unsupported DDL and stale contexts
withhold claims. The user selects a real column, previews its exact quoted edit,
then explicitly applies one occurrence with one-step undo. Editor SQL is never
prepared/executed. Joins, CTEs, derived relations and broader inference remain open.
See [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-qualified-column-inspections-2-november-2026).

### R5 ordered direct-table JOIN inspections (local `46912f5`)

Inspect columns now checks qualified references over flat direct-table JOINs
and comma groups on PostgreSQL/MySQL/MariaDB. ON clauses see only their actual
operands, including pending JOIN association; later relations and earlier comma
groups cannot leak into their scope. Self-joins share one fresh metadata read
while retaining separate aliases. The correction chooser follows each issue's
owning table. A failed or stale table read prevents the whole inspection result.
Limits are 32 relations, 2,048 columns per table, 8,192 columns / 4 MiB aggregate,
alongside existing statement/token/expression limits. Nested/derived/CTE inference,
unqualified-column diagnostics and broader semantic fixes remain open. See
[exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-ordered-join-column-inspections-2-november-2026).

### R5 context-aware live templates (local `51ad451`)

1. Extend existing built-in/custom templates with a small documented variable
   vocabulary for current database/schema and selected text. Resolve values from
   the active console's observed context; quote SQL identifiers by engine and
   distinguish literal selected text from identifier values. Do not guess missing
   context or read another console's catalog.
2. Preserve numbered tab stops, final cursor stop, engine/global precedence,
   custom-template CRUD/reload and bounded backup/restore. Unsupported variables
   need explicit feedback; expansion must never prepare or execute SQL. Resolve
   asynchronous context before insertion and reject changed documents/sessions.
3. Reproduce missing/stale context and quoted-name cases, exercise primary and
   isolated console expansion on PostgreSQL/MySQL/MariaDB, compare an explicit
   harmless read after expansion, and prove zero expansion execution requests.
   Implemented locally on `51ad451`. See
   [exact evidence](IMPLEMENTATION_STATUS.md#r5-follow-up-context-aware-live-templates-2-november-2026).
   Broader CTE/derived inspection and semantic placement stay partial. Then
   continue remaining R5/R6 integration and R7/R8; merge/release/install and
   external/native/platform qualification remain separate gates.

### R6 slice 1: estimated query plans (local `feat/estimated-query-plans`)

Commit `fad0341`, based on fetched `origin/main` `a0ba1a6`, adds a single-query
estimated-plan action for PostgreSQL and MySQL/MariaDB, routed through existing
Rust authorization and cancellation. Bounded tree/raw views share the retained
JSON result; optimizer costs, estimated rows and actual per-loop metrics retain
distinct labels. See [query-plans.md](query-plans.md) for the contract and bounds.

Validation passes 126 frontend tests and 130 standard Rust tests, plus seven
relevant browser workflows (three live-engine plan flows and four session/file
regressions), macOS bundling and three hidden native runs / 30 cleanup cycles.
This branch does not contain R5. It is unmerged and unreleased; R6 is partial.

### R6 slice 2: reviewed PostgreSQL actual plans (local `fa1ff17`)

PostgreSQL single-query `EXPLAIN (ANALYZE, FORMAT JSON)` is implemented with
mandatory Rust review even on Silent profiles. Review shows exact SQL and an
execution warning; ReadOnly denial and production passphrases remain enforced.
Recognized direct SQL and parenthesized ANALYZE options hit the same gate.
Owned-session execution, cancellation and one-use approval are unchanged.

Standard validation passes 126 frontend and 132 Rust tests. Two PostgreSQL
fixtures prove denied analysis never executes, temporary state remains attached,
approvals expire and are single use, disconnect invalidates plans, and errors
and cancellation recover safely. Five plan browser flows and four console/file
regressions pass. Clean-source macOS bundle and three hidden native runs /
30 cleanup cycles pass. Full ignored integration/full browser suites were not
rerun. R5 integration, merge, application publication and installation remain open.

### R6 slice 3: bounded snapshots and comparison (local `ba984f2`)

Explicit captures retain raw plans and immutable executed query/context identity;
comparison never reruns SQL. A shared ten-capture / 2 MiB UTF-8 payload budget
includes SQL and context, with FIFO eviction and tab/session/connection cleanup.
Only matching engine/version, session, observed context and exact underlying
query bytes compare. PostgreSQL estimated and actual plans may be viewed side
by side; incompatible snapshots show a disabled reason. Independent trees and
raw JSON preserve labels and exact text without guessed node pairing or deltas.
Snapshots remain memory-only and are excluded from workspace persistence.

Standard validation passes 134 frontend and 132 Rust tests. Six plan browser
flows and four console/file regressions pass against disposable PostgreSQL 18,
MySQL 8.4 and MariaDB 11.4. The snapshot unit/lifecycle suite covers eviction,
invalid input, mismatch, exact numeric text, cleanup and persistence exclusion.
See [verification evidence](IMPLEMENTATION_STATUS.md#r6-bounded-plan-snapshots-and-comparison-1-november-2026).
This branch remains separate from R5, unmerged and unreleased.

### R6 slice 4: execution timing and observed plan context (local `cb1ca1a`)

Query runs separately measure queue wait, backend active/total, first nonempty
row batch and client request-to-finish time. Missing values remain unavailable;
zero remains visible. Driver batch delivery is not first-wire-row latency or
render time. Server planning/execution values stay separate in PostgreSQL plans.
Plan capture now uses context observed by Rust after acquiring the editor lane;
unavailable observations withhold capture rather than reusing frontend state.
Database state and unobserved settings remain outside this identity guarantee.

Standard validation passes 136 frontend tests and 132 standard Rust tests.
All 26 PostgreSQL fixture tests and twelve relevant browser flows pass; these
include queued context changes, empty/error/multi-result timing, active/queued
cancellation and unavailable-context capture denial. Clean-source macOS gates
are recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-execution-timing-and-observed-plan-context-1-november-2026).
This branch remains separate from R5, unmerged and unreleased.

### R6 slice 5: reviewed MariaDB actual plans (local `c660bc4`)

MariaDB's Analyze action uses `ANALYZE FORMAT=JSON` only when a prepare-only
server capability check succeeds. It never executes analysis on connection
or changes optimizer/output settings. Explicit review applies even on Silent,
including directly entered SQL that falls outside the AST grammar. ReadOnly
and production policy remain authoritative; one-use approval, owned temporary
state, error recovery, cancellation retirement and observed-context comparison
are verified. MariaDB inclusive node time, average rows per loop, filtering and
optimization remain distinct from PostgreSQL's metrics.

Standard validation passes 138 frontend and 133 standard Rust tests. All 26
PostgreSQL and 24 MySQL/MariaDB base fixtures pass, plus fifteen relevant browser
workflows. The separate case-folded fixture was excluded. Exact macOS gates are
recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-reviewed-mariadb-actual-plans-1-november-2026).
MariaDB qualification is bounded to 11.4.13. R6 stays partial, local and separate
from the preserved R5 candidate.

### R6 slice 6: reviewed MySQL TREE actual plans (local `c54ecab`)

MySQL Analyze now explicitly uses `EXPLAIN ANALYZE FORMAT=TREE` after a
non-executing prepare-only capability probe. Session JSON/output settings remain
unchanged. A separate bounded TREE adapter preserves engineering tokens and
unexecuted iterators; costs, rows, loops and inclusive per-loop times retain
their scopes. JSON estimates can compare with TREE actuals using the existing
exact executed query and observed-context identity, without replay.

Reproduced review bypass for `DESC ANALYZE ... FOR SCHEMA` is fixed before AST
fallback, including DESC/DESCRIBE aliases. Reproduced partial-result success
after MySQL KILL QUERY now honors cancellation and retires the session. This
does not roll back side effects or change ordinary write acknowledgements.

Validation passes 142 frontend and 133 standard Rust tests, all 26 PostgreSQL
and 25 MySQL/MariaDB base fixtures, and eighteen relevant browser workflows.
The separate case-folded fixture remains excluded. Capability/settings, one-use
approval, temporary state, ReadOnly, errors, cancellation, comparison and cleanup
are qualified on MySQL 8.4.13 / MariaDB 11.4.13 only. Clean-source macOS gates are
recorded in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-reviewed-mysql-tree-actual-plans-1-november-2026).
R6 stays partial, local and separate from the preserved R5 candidate.

### R6 slice 7: Snowflake estimated logical plans (local `48dfb21`)

Explicit Explain wraps one complete Snowflake SELECT/WITH query with
`EXPLAIN USING JSON`; read-only classification still rejects unsafe/unknown
input. A step-local DAG adapter expands shared operators once and retains
all other parent edges as explicit references. Pruning estimates stay separate
from actual timing/rows; unsafe numeric estimates defer to exact raw JSON.
Malformed, cyclic, orphaned and excessive graphs retain raw/grid access.

Standard validation passes 148 frontend / 136 Rust tests, and four synthetic
Snowflake browser flows pass. Loopback Rust transport verifies the exact SQL
wrapper and configured request context without a live account. Snowflake
snapshots are unavailable until warehouse and session context are independently
observed and qualified. No Analyze or warehouse setting change is introduced.
Clean-source macOS evidence is in [implementation evidence](IMPLEMENTATION_STATUS.md#r6-snowflake-estimated-logical-plans-1-november-2026).
R6 remains partial, local, unmerged and unreleased.

### R6 next slices

1. Six R5 slices and seven R6 slices now include per-engine styles and named
   CTE/derived-table outputs, console Structure, inspector continuation and
   bounded alias/CTE rename and ordered JOIN column inspection/correction (`46912f5`).
   Standard checks and 21 focused browser flows pass. The clean-source
   native synthetic baseline passes three processes / 30 cycles on this revision.
   Prior combined-source qualification passed all 66 ignored local SQL/Redis/SSH
   fixtures and a normal ad-hoc macOS bundle on `b80b6c7`; the full 86-flow browser
   suite passed on `db5cd1b`.
   Next: the remaining R5/R6/R7/R8 scope.
   A non-recursive CTE UNION is inspected from the anchor projection. A recursive arm can read its CTE.
   A derived table UNION is inspected from the anchor projection.
   INTERSECT and EXCEPT use that same anchor projection inside a CTE or derived table.
   A select list that is only * or alias.* for one relation expands to that relation's columns.
   An unaliased schema.table.* expands to that same relation.
   A star beside simple columns keeps those names as well. A repeated output name still withholds.
   A lone star on every arm of a set operation expands when each star is the same relation.
   An unaliased schema.table.* may be that star on every arm.
   A later arm of a recursive CTE may be that same lone star, including a star of the CTE.
   Every arm of a set operation may add the same number of simple columns beside that star.
   A star of another relation is accepted when metadata gives every arm the same width.
   A full explicit column list renames a star when the list matches that star's width.
   A top-level UNION, INTERSECT, or EXCEPT of flat SELECT arms uses that same width check.
   One pair of parentheses may wrap a flat arm of a CTE or derived-table set operation.
   `{{lower(selection)}}` and `{{upper(selection)}}` change the selected text's case.
   A recursive CTE of more than two flat SELECT arms is inspected from the anchor projection.
   A full explicit CTE column list renames that CTE's projected outputs. A partial list still withholds.
   A parenthesized query in a function argument uses the same template placement.
   A parenthesized statement after EXPLAIN uses the same template placement.
   A custom snippet can declare its completion site. A built-in trigger keeps its place.
   Completion classifies the statement inside an EXPLAIN wrapper.
   A recursive CTE of two flat SELECT arms is inspected from the anchor projection.
   Completion places statement, JOIN and LIMIT templates inside a parenthesized query after FROM, JOIN, AS, IN, EXISTS, SELECT, a comparison, or a comma.
   A derived table can read an earlier CTE in the same WITH list.
   Several non-recursive CTEs use the same projected-name rule. A CTE can see earlier CTEs in that WITH list.
   Parenthesized select-list expressions use the same one-owner column rule.
   Function arguments use the same one-owner column rule.
   Bare names in WHERE, GROUP BY, HAVING, ORDER BY and ON use the same
   one-owner rule as the select list. Completion places statement templates at a statement
   start and JOIN or LIMIT templates after that clause. A top-level select-list name is
   inspected when one relation owns it. Several owners withhold the claim.
   One non-recursive CTE or derived query can be inspected from its projected
   names (`15089a1`). Multiple CTEs, recursive queries and stars still withhold.
   Context-aware templates are local on `51ad451`. Preserve conservative ambiguity
   handling, every
   preparation/approval path and exact editor origins. Keep the earlier startup
   flake open until its cause is known.
   Application merge, release and installation remain separate authorized gates.
2. Design observed Snowflake warehouse/session identity before snapshot support.
   Keep capture explicitly unavailable meanwhile. Live read-only account
   qualification requires explicit external scope; mock evidence is local only.
3. Broaden supported MySQL/MariaDB versions and TREE shapes only with captured
   engine evidence. Unsupported capability/format combinations retain clear
   unavailable/raw states; do not silently change user settings.

The MariaDB adapter follows its
[official analysis contract](https://mariadb.com/docs/server/reference/sql-statements/administrative-sql-statements/analyze-and-explain-statements/analyze-format-json).
The separate MySQL TREE adapter follows the
[8.4 EXPLAIN manual](https://dev.mysql.com/doc/refman/8.4/en/explain.html).
The earlier JSON-v2 probe remains research evidence; no JSON-v2 adapter is enabled.

R7 schema/design and R8 remaining productivity follow these R6 slices. Release,
platform, security, accessibility, recovery and performance qualification run
throughout; application publication and installation remain separate actions.

### R7 slice 1: read-only ER diagrams (branch `feat/er-diagram`)

The owner asked on 3 November to continue until Joinery can replace DataGrip.
Read-only slices go first: D01 diagrams, then A05/A04 monitoring, A03 statistics,
D02 snapshots/diff, D03 DDL generation, D05/N08 reviewed editors, D04 migration
review, P05/P06, RX and the v1.0 gates. `schema_diagram` reads one schema's
relations, readable columns and outbound foreign keys in one bounded metadata call
on PostgreSQL and MySQL/MariaDB. It never prepares or executes editor SQL. See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-1-read-only-er-diagrams-3-november-2026).

### R8 slice 1: lock waits and session monitoring (branch `feat/lock-monitoring`)

`lock_waits` reads current waits and blockers on PostgreSQL, MySQL and MariaDB.
The Processes view derives root blockers and deadlock cycles locally and adds
filtering and optional refresh. Merged in PR #62 (`80de7aa`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-1-lock-waits-and-session-monitoring-3-november-2026).

### R8 slice 2: relation storage and index usage statistics (branch `feat/storage-statistics`)

`object_details` populates `TableStorageStats` (`estimated_rows`, `table_bytes`,
`index_bytes`, `free_bytes`, `total_bytes`, `live_tuples`, `dead_tuples`,
`sequential_scans`, `index_scans`, `last_analyze`, `last_vacuum`, `stats_stale`,
`restricted_reason`) and per-index `size_bytes`, `scans`, `tuples_read`,
`tuples_fetched` and `cardinality` on PostgreSQL and MySQL/MariaDB. It never
issues automatic `COUNT(*)` or `ANALYZE`, withholds PostgreSQL relation storage
and index counters when the session lacks `SELECT` privilege on the relation,
and degrades cleanly on MySQL/MariaDB when `mysql.innodb_index_stats` or
`performance_schema` is unreadable or disabled. Merged in PR #63 (`130e192`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-2-relation-storage-and-index-usage-statistics-3-november-2026).

### R7 slice 2: schema snapshots and deterministic schema diff (branch `feat/schema-diff`)

`schema_snapshot(sessionId, schema)` captures a bounded, canonical structural
snapshot (`SchemaSnapshot`) of a PostgreSQL schema or MySQL/MariaDB database
(tables, views, materialized views, partitioned tables, columns, constraints,
indexes, triggers, overloaded routines, sequences, and disclosed unsupported
objects such as custom PostgreSQL enum/domain/range/composite types, foreign
tables, non-view rewrite rules, and MySQL scheduled events). `compare_schemas`
computes a deterministic `SchemaDiffReport` independent of catalog row order,
with engine-quoted object and member names. `SchemaDiffView` compares any two
live schemas or up to 16 in-memory captured snapshots and never prepares or
executes DDL. Merged in PR #64 (`046d05c`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-2-schema-snapshots-and-deterministic-schema-diff-3-november-2026).

### R7 slice 3: Rust DDL generator and migration preview (branch `feat/ddl-generator`)

`generate_schema_ddl(snapshot, targetSchema)` and
`generate_migration_ddl(left, right, targetSchema)` generate dependency-ordered
PostgreSQL and MySQL/MariaDB full-schema and migration DDL scripts in Rust
(`src-tauri/src/ddl.rs`), retargeting self-schema references and flagging
destructive `DROP` statements and warnings without auto-executing SQL. Round-trip
disposable fixture tests on PostgreSQL 18, MySQL 8.4 and MariaDB 11.4 verify
zero structural diffs (`compare_schemas == 0`) after executing both full-schema
and migration DDL plans. Merged in PR #65 (`b67e65b`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-3-rust-ddl-generator-and-migration-preview-3-november-2026).

### R7 slice 4: index and constraint editor (branch `feat/index-constraint-editor`)

`preview_index_constraint_ddl(engine, action)` and
`prepare_index_constraint_ddl(sessionId, action)` build and authorize
engine-aware `CREATE INDEX`, `DROP INDEX`, `ADD CONSTRAINT` (`PRIMARY KEY`,
`UNIQUE`, `CHECK`, `FOREIGN KEY` with `ON DELETE`/`ON UPDATE` and PostgreSQL
`NOT VALID`) and `DROP CONSTRAINT` statements on PostgreSQL and MySQL/MariaDB.
`StructureView` and `ObjectInspectorView` wire `IndexConstraintDialog` with live
Rust DDL previews, lock/implicit-commit warnings, `Constraint-backed` index
guardrails, and mandatory `ReviewDialog` approval. Merged in PR #66 (`792aa99`).
See [evidence](IMPLEMENTATION_STATUS.md#r7-slice-4-index-and-constraint-editor-3-november-2026).

### R7 slice 5: table and column DDL editor (branch `feat/table-object-editor`)

`preview_table_object_ddl(engine, action)` and
`prepare_table_object_ddl(sessionId, action)` build and authorize engine-aware
`CREATE TABLE` (ordered columns, `NOT NULL`, defaults, PostgreSQL `GENERATED BY
DEFAULT AS IDENTITY` vs MySQL/MariaDB `AUTO_INCREMENT`, composite `PRIMARY KEY`
and table comments), `ADD COLUMN`, `ALTER COLUMN` (PostgreSQL `ALTER COLUMN …
DROP DEFAULT` before `TYPE … USING …`, `SET`/`DROP NOT NULL`, `SET`/`DROP
DEFAULT` and `RENAME COLUMN`; MySQL/MariaDB `MODIFY COLUMN` / `CHANGE COLUMN`),
`DROP COLUMN` (with optional PostgreSQL `CASCADE`), and `DROP TABLE` / `DROP
VIEW` / `DROP MATERIALIZED VIEW` (with optional PostgreSQL `CASCADE`).
`Sidebar.tsx` (`Create table in <schema>`) and `StructureView` (`Add column…`,
`Drop table…` / `Drop view…`, per-column `Edit…` and `Drop…`) open
`TableObjectDialog` with live Rust DDL previews, lock/implicit-commit/destructive
warnings, and mandatory `ReviewDialog` approval. Merged in PR #67 (`c855c04`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-5-table-and-column-ddl-editor-3-november-2026).

### R7 slice 6: migration review and schema synchronization workflow (branch `feat/migration-sync`)

`prepare_migration_ddl(sessionId, expectedSnapshot, statements)` verifies that
the live target schema has not drifted from `expectedSnapshot` (returning
`ErrorCode::Conflict` on any structural difference or truncation/privilege
change), classifies selected `DdlStatement`s as `StatementKind::Ddl` via
`safety::plan_for_statements` (`SafeMode` + one-use approval + production
passphrase), and attaches destructive, implicit-commit and lock warnings.
`SchemaDiffView` adds per-statement checkboxes (`Select all`, `Select
non-destructive`), **Review & apply migration…**, and `ReviewDialog` integration,
refreshing the catalog and schema comparison upon completion. Merged in PR #68
(`83b1260`). See
[evidence](IMPLEMENTATION_STATUS.md#r7-slice-6-migration-review-and-schema-synchronization-workflow-3-november-2026).

### R8 slice 3: contextual command palette, custom shortcuts and tab management (branch `feat/shortcuts-and-layouts`)

`shortcuts.ts` defines 16 canonical keyboard shortcut actions (`openPalette`,
`searchObjects`, `recentLocations`, `newQuery`, `openSqlFile`, `saveSqlFile`,
`saveSqlFileAs`, `toggleBookmark`, `preferences`, `closeActiveTab`,
`closeOtherTabs`, `closeCleanTabs`, `reopenClosedTab`, `togglePinTab`,
`toggleSidebar`, `toggleInspector`) with conflict and reserved-chord
(`Mod+C`/`V`/`X`/`Z`/`A`) validation, persisted in `SavedWorkspace.shortcuts`
(validated in `persistence/workspace.rs`). `SearchEverywhere` renders contextual
`contextBadge` pills and `disabledReason` subtitles for active-tab, tab-strip
and layout actions. `studio.ts` adds tab pinning (`QueryDraft.pinned`),
`closeOtherTabs`, `closeCleanTabs` (preserving pinned, dirty, running and
last-secondary-session-owner tabs), a bounded 10-entry `closedTabs` reopen stack
(`reopenClosedTab`, purged on connection removal), and persisted
`sidebarCollapsed` / `inspectorVisible` toggles. Merged in PR #69 (`8be971a`). See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-3-contextual-command-palette-custom-shortcuts-and-tab-management-3-november-2026).

### R8 slice 4: closed typed IPC command map and granular engine capabilities (branch `feat/typed-commands-capabilities`)

`src/ipc/command-map.ts` defines the closed `JoineryCommandMap` pairing every
backend command with its exact argument and result DTOs from
`src/ipc/generated.ts`, enforced by `StudioClient.call` in `src/ipc/client.ts`.
`EngineCapabilities` (`src-tauri/src/domain.rs`) adds `schemaDiagram`,
`schemaDiff`, `ddlEditing`, `lockInspection`, `explainAnalyze` and
`oauthBrowser`, populated by each engine driver and enforced in `Studio`
(`ErrorCode::Unsupported`) as well as in `Sidebar`, `ProcessesView`,
`StructureView`, `ObjectInspectorView` and `Workbench`. See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-4-closed-typed-ipc-command-map-and-granular-engine-capabilities-3-november-2026).

### R8 slice 5: engine administration and table maintenance tools (branch `feat/typed-commands-capabilities`)

`build_table_maintenance_sql` (`src-tauri/src/ddl.rs`),
`preview_table_maintenance`, and `prepare_table_maintenance` generate reviewed
PostgreSQL (`ANALYZE`, `VACUUM`, `VACUUM FULL`, `REINDEX TABLE`) and
MySQL/MariaDB (`ANALYZE TABLE`, `OPTIMIZE TABLE`, `CHECK TABLE`) maintenance
statements with lock, implicit-commit, and non-transactional warnings.
`Postgres::execute` / `execute_bound` reject non-transactional maintenance
statements (`VACUUM`, `CLUSTER`, `CONCURRENTLY`) with `ErrorCode::Conflict` when
a manual transaction is open. `StorageStatsSection` (`ObjectInspectorView.tsx`,
`UtilityViews.tsx`) adds a **Maintenance…** action opening
`TableMaintenanceDialog.tsx` and routing execution through `ReviewDialog`. See
[evidence](IMPLEMENTATION_STATUS.md#r8-slice-5-engine-administration-and-table-maintenance-tools-3-november-2026).

### RX slice 1: single-table query result editing with Rust-proven provenance (branch `feat/typed-commands-capabilities`)

`safety::analyze_single_table_select` and `safety::verify_result_provenance`
(`src-tauri/src/safety.rs`) inspect `SELECT` statements on PostgreSQL and
MySQL/MariaDB against the live `TableSchema` and driver-reported result columns,
proving single-base-table and primary-key provenance or recording an explicit
`ResultEditability::ReadOnly { reason }`. Proven results store a projected
`TableSchema` in `ResultStore` and expose `ResultEditability::Editable { table,
primary_key }` on `ResultInfo`, enabling inline cell edits, row inserts, row
deletes, TSV paste, and reviewed `prepare_table_changes` commits (`SELECT … FOR
UPDATE` full-row conflict checks + in-place `ResultStore::apply_committed_changes`
refresh) directly inside SQL query result grids. See
[evidence](IMPLEMENTATION_STATUS.md#rx-slice-1-single-table-query-result-editing-with-rust-proven-provenance-3-november-2026).

## Subsequent phases and blueprint cross-reference

| Phase | Feature IDs / user value | Priority / complexity | Dependencies and exit criteria | Blueprint relationship |
|---|---|---|---|---|
| 0 Reliability | M01–M03, X01–X03, G04/G06, I01/I02/I07; preserve working workflows | P0 / High | Fixture engine matrix, cancellation/cleanup, Safe Mode, budgets and recovery pass on current revision; external gates remain explicit | Continue existing qualification batch B/E, do not reopen shipped engines as greenfield work |
| 1 SQL editor | First-batch slices above, then E09–E13/E16 | P1–P2 / High | Session metadata and error contracts before context/navigation; snippets/formatting stay local and never execute automatically | Extends SQL Studio/schema indexing; scoped defects first |
| 2 Explorer | N01–N08, A03; inspect objects without hand-written catalog SQL | P1–P2 / High | Typed engine object identities, bounded catalog/DDL, permission states; object changes use reviewed Rust plans | Extends existing catalog; grouped FKs pulled forward as JOIN prerequisite |
| 3 Grid | G03/G05/G07–G13; filter, edit and move data safely | P1–P2 / High | Typed sort/filter requests with PK tie-breakers and NULL ordering; atomic paste; row mutation types before insert/import; explicit bounded Rust export | Retains existing provenance, overlays, transactions and memory-only results |
| 4 Analysis | X05/X07/A01/A02/A04/A05; explain execution behavior | P2 / High | Engine capabilities and raw/normalized plans agree; estimated and actual metrics distinct; ANALYZE explicitly authorized, never auto-run | Advances query-plan stretch goal after safety/metadata prerequisites |
| 5 Design | D01–D05/N07; understand and review schema changes | P2 / High | Metadata → snapshots → deterministic diff → immutable reviewed migration; no automatic schema synchronization | Advances diagrams/migration stretch goals |
| 6 Productivity | P01–P07/E12/E14; navigate and reuse work | P1–P2 / Medium | Extend existing palette/Zustand/persistence; keyboard workflows, opt-in SQL storage and dirty-state protection | Retains single-window Joinery identity and default shortcuts |
| 7 Hardening/expansion | I03/I05/I06, new engines; prove performance and distribution | P0 release gates, P3 new engines / High | Measure throughout, not only at the end. Actual platform installers/UI, signing/update integrity and attributable memory/startup/FPS. Every new engine gets independent dialect, metadata, capability, security and fixture contracts | Existing P7 gates remain; SQLite/SQL Server/Oracle/BigQuery are separate projects |

## Release and architecture constraints

All application changes go on a task branch from fetched `origin/main`; use the
repository owner-identity scripts, inspect diffs, and make small passing
Conventional Commits with regression tests. Do not push, merge, publish or install
an application release without authorization. Update matrix evidence and the
private hub after implementation/verification; record local-only implementation
as unmerged and unreleased. Keep hub access owner-private.

No new state manager, execution system, editor, grid or adapter architecture is
introduced. The scoped repair fits the blueprint; the subsequent live context
contract is recorded in [architecture-decisions.md](architecture-decisions.md).
Future interface changes must preserve Rust validation,
one-use approvals and explicit memory budgets. No live credentials, production
tables, signing setup or external accounts are prerequisites for the first slice.

Snowflake remains read-only; Redis Cluster/Sentinel/streams/PubSub and external
MySQL process killing remain unsupported. Arbitrary SQL results remain read-only.
Full derived/CTE output inference and wider engine/context qualification remain
open. Direct-base-table PostgreSQL/MySQL/MariaDB FK JOIN assistance is locally
verified in the first R5 integration slice above.
