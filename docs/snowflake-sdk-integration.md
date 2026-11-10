# Snowflake external-browser application integration

Status on 10 November 2026: application integration is implemented and locally
qualified. This increment does not satisfy the owner's
required writes, DDL, transactions, metadata or complete DataGrip replacement.
The [required inventory](snowflake-datagrip-replacement.md) remains unchanged.

## Profile and runtime ownership

Snowflake profiles store an explicit authentication mode:

| Mode                    | Selection                                     | Current behavior                                                                             |
| ----------------------- | --------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `externalBrowser`       | Default for a newly created Snowflake profile | Pinned Node/official SDK helper, one connection per logical session, read-only SQL           |
| `localApplicationOauth` | Default when an existing profile has no mode  | Existing local-application OAuth/SQL API route, read-only SQL and its bounded metadata pilot |

Changing a role, warehouse, database or schema preserves the selected mode.
Switching modes is explicit. Unknown serialized values fail validation. A missing
SDK runtime produces an unsupported error; there is no automatic OAuth fallback.
Saved modes survive a new `Studio` instance and metadata backup/restore in actual
local persistence tests. New-profile and legacy-profile UI flows also exercise
real persistence through the offline browser harness. That harness refuses both
Snowflake authentication routes before creating an attempt.

Native macOS ARM64 startup selects the verified development stage in debug builds
or Tauri's resource directory in release builds. Release builds cannot fall back
to checkout resources, PATH, an environment override or a profile-supplied path.
Node bytes, helper source bytes, SDK version, the compiled lock digest, resource
tree types/modes and contained dependency symlinks are checked. This does not
authenticate every dependency file or replace package signing/provenance review.

Each connection attempt reserves cleanup capacity before allocating a private
state root. The launch attaches its cleanup observer before fallible setup or
authentication. Cancellation, an abandoned connect future, disconnect and native
shutdown retain that authority until no child started or the actual child was
reaped and the original directory was removed. A retained driver reference does
not delay logical retirement. Snapshots include pending launch permits, so reset
cannot miss a later attachment and cannot remove a newer attempt's slot.

Failed directory removal remains observable and retryable. Identity checks reject
a replacement directory or symlink. Restoring the original directory can allow
retry after the child has been reaped. On APFS, a removed directory's open handle
can retain a nonzero link count; successful removal after verifying the original
identity is the receipt. A missing pathname alone does not establish removal.
The trusted parent boundary assumes no hostile same-user pathname mutation.
Initial filesystem removal is synchronous; async wait bounds are not hard
wall-clock limits against blocked local filesystem I/O.

## Runtime and package commands

Prepare the pinned runtime using the [runtime guide](snowflake-runtime.md), then:

```sh
pnpm desktop:snowflake:bundle:mac
pnpm test:snowflake:startup --bundle src-tauri/target/release/bundle/macos/Joinery.app
pnpm desktop:snowflake:smoke:build
pnpm test:native --runs 1 --attributed-memory
```

The opt-in configuration maps the prepared tree to
`Contents/Resources/snowflake-runtime/`. The hidden smoke preparation validates
and copies that same hierarchy; build metadata records inclusion and manifest
hash. Its native checkpoint reports whether the application loaded a runtime.
The startup checker imports the actual SDK from the selected bundle, emits one
hello frame and closes on empty stdin, with zero authentication/SQL requests.
Its Node socket/DNS guard is not an OS sandbox. Runtime loading and hello/EOF
startup do not prove account sign-in, remote TLS, cancellation or session behavior.

Standard bundle/smoke commands remain available without a prepared runtime.
Such builds cannot connect profiles that explicitly request the SDK route.
The [notice inventory](snowflake-sdk-licenses.md) retains one unresolved
`proxy-agent-negotiate` notice-applicability gap. Local packaging is not
redistribution clearance, notarization, installation or publication.

## Qualification record

Local evidence is under `.cache/verification/connection-attempts/`. Cache paths
identify the current local run; committed tests and commands permit reproduction.

| Check                               | Current result                                   | Scope and evidence                                                                                                                                                                                             |
| ----------------------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime factory                     | 18 tests pass                                    | `sdk-runtime-factory.log`; immutable pins, unsafe paths/modes, fixed route, private roots and failed launch preparation                                                                                        |
| Cleanup registry/authority          | 15 focused tests pass                            | `sdk-cleanup-owned-final.log`; pending permits, retained retries, identity replacement, APFS regression                                                                                                        |
| Actual child supervisor             | 31 tests pass                                    | `sdk-child-adapter.log`; includes forced kill/reap and replacement-root recovery                                                                                                                               |
| Read adapter                        | 11 tests pass                                    | Same log; actual framed helper with a synthetic SDK, no account                                                                                                                                                |
| Application connection lifecycle    | Four tests pass                                  | `sdk-studio-after-apfs.log`; actual Node/helper with synthetic SDK, publication, cancel, abandoned future and shutdown                                                                                         |
| Profile persistence                 | Six tests pass                                   | `sdk-profile-interim.log`; includes mode defaults, invalid values, restart, backup/restore and no-runtime rejection                                                                                            |
| Browser authentication form         | Six focused workflows pass                       | `sdk-browser-focused-final.log`; two actual offline-persistence flows plus four explicitly synthetic legacy metadata/plan flows                                                                                |
| Runtime preparation/startup scripts | 48 tests pass and actual staged startup passes   | `sdk-runtime-scripts.log`, `sdk-stage-startup.log`; no login or SQL                                                                                                                                            |
| Combined validation                 | Passed: 604 frontend and 297 standard Rust tests | `sdk-validate.log`; includes frontend production build, Rust formatting, desktop check and strict all-target/all-feature Clippy                                                                                |
| Native resource verification        | Seven tests and actual stage pass                | `sdk-native-resources-final.log`, `sdk-stage-resources.json`; 15,104 entries, five contained dependency links, notice/source pins                                                                              |
| Real isolated database fixtures     | 124 pass                                         | `sdk-real-fixtures.log`; PostgreSQL, MySQL, MariaDB, Redis, SSH, permissions and interrupted transaction outcomes                                                                                              |
| Full browser qualification          | All 143 workflows pass (9.5 minutes)             | sdk-browser-all-final.log; source snapshot comparison is unchanged                                                                                                                                             |
| Normal SDK bundle/resources/startup | Passed                                           | `sdk-bundle.log`, `sdk-bundle-artifact.json`; ad-hoc signature, packaged-resource gate and actual SDK hello/EOF; no authentication or SQL                                                                      |
| SDK hidden native application       | Passed: 19 checks and ten cleanup cycles         | sdk-native-smoke.log and sdk-native-runs.json; every checkpoint reports the packaged runtime available, zero connection/SQL invocations                                                                        |
| Actual Snowflake account            | Approved two-query helper canary passes          | After a reproduced validator repair, both result contracts, context/session continuity, logout acknowledgement and local cleanup pass. Normal packaged application and complete account workflows remain open. |
| Owner hands-on acceptance           | Open                                             | All applicable latest DataGrip workflows remain required                                                                                                                                                       |

An initial browser run failed because the harness correctly rejected remote
profile persistence. The added offline fixture permits only one synthetic profile
shape and continues to refuse authentication; it does not relax desktop policy.
An initial Rust test rerun caught a missing test-local variable, corrected before
the four lifecycle tests passed. Earlier APFS receipt failures remain in
`sdk-cleanup-owned-interim.log` and `sdk-studio-interim.log` as reproduced defects.

The first full browser run passed 142 workflows and failed one old sign-in-copy
assertion. That assertion now checks the explicit SDK mode and its current
description. The application source did not change during that run; the recorded
source comparison identifies only package-script changes. The fresh full browser
run passed all 143 workflows in 9.5 minutes; its recorded 264-file snapshot
comparison found no changed paths during execution.

The owner separately approved the fixed two-query account read canary, including
possible warehouse billing. The first live run authenticated and submitted both
reads, then failed local result validation. An offline regression reproduced a
validator defect: `CURRENT_SESSION()` returns VARCHAR, while the runner required
a numeric/fixed column. The corrected runner in `9e39e51` passed the same approved
SQL scope at `2026-11-10T05:49:42.148Z`, including both result contracts, context,
session continuity, logout acknowledgement and local cleanup. The failed first
run remains in the ledger; its safe evidence cannot establish the defect as its
sole cause. This helper-level pass does not qualify packaged application sign-in
or any additional account workflow. See [the verification ledger](replacement-verification.md).

The normal opt-in bundle is 223.41 MiB. Its executable SHA-256 is
`a6b09c0b0a73d9626cbf7bab9e9a5bdb1c69ffeda0172f0f04dd96b5f50ad1f2`;
its runtime manifest SHA-256 is
`e99ae96378b685b7a49027c77fe36039d6a32a6f2f470b9353ae36ad27307d76`.
Tauri copied dependency links as regular files in this bundle; resource validation
passed with 15,104 entries and zero links. The actual packaged SDK hello/EOF check
reported zero authentication and SQL requests. The release build succeeded despite
the pre-existing toolchain warning about `rust-objcopy` locating `libLLVM.dylib`.
The normal application has not been launched against the owner's private state.

The source changes are committed locally as `521b086` (application integration)
and `cdfba88` (resource packaging). Build provenance records `daa4203` with a
dirty tree because qualification preceded these commits. The recorded source
snapshots identify those bytes; the commits do not change the qualified files.

The hidden bundle executable SHA-256 is
`2fad04be58bc700ab3807cb9240c52b843030c3668efcc463030a4a26879235b`.
Its 19 checks and ten cleanup cycles passed in 4374 ms; all six checkpoints
reported `snowflakeRuntimeAvailable: true` and zero connection/execution calls.
This is hidden smoke evidence, not representative performance or actual SSO.
Two additional standalone hidden native PostgreSQL close/reopen pairs passed
with pending-close exits of 74.24 ms and 50.61 ms (within the 400 ms debounce),
actual rollback, restored drafts/preferences and zero reconnect/SQL replay before
observation. See `sdk-native-recovery-runs.json`; the standalone recovery route
does not qualify packaged SDK resource discovery or Snowflake transactions.

## Remaining required workflows

The SDK route advertises SQL and cancellation. A subsequent
[same-session completion increment](snowflake-sdk-metadata-design.md) implements
bounded relation suggestions and lazy resolved columns; the preceding integration
counts and packaged artifacts do not qualify that later source. Metadata browsing,
authoritative object inspection, schema tools, plans and transactions remain disabled
until their workflows are implemented. The UI skips unsupported catalog discovery
and removes stale metadata rather than reporting a successful connection as an
authentication failure. The existing SQL API metadata pilot does not qualify SDK
metadata behavior.

Writes, DDL and explicit transactions remain required and blocked by unqualified
SDK retransmission/outcome behavior; they are not optional. Actual Snowflake
deduplication after a lost successful response needs independent server-state
observation. Grid/CSV target identity also remains an open safety gate: documented
name-based references do not pin an object generation. Raw reviewed SQL and safe
metadata work can advance independently of that grid gate. See the
[driver design](snowflake-driver-design.md).

Account acceptance must additionally demonstrate external-browser cancellation,
expired login, context continuity, interruption and reconnect behavior, exact
types, million-row paging/export, session cleanup and persistence across normal
packaged application restarts. No synthetic or local-engine result closes those
required account workflows.
