# Snowflake table qualification command

The dedicated command prepares, verifies and, after separate exact approval,
executes the proposed [existing-schema lifecycle](snowflake-existing-schema-qualification.md).
`emit` and `verify` remain offline: no Node/compiler launch, Studio/keychain,
SSO, Snowflake connection or permanent marker creation. `execute` is a separate
route through the fixed consumption store and retained private factory. Merely
building the binary, emitting evidence or supplying an approval file grants no
permission for account activity or billing. Runtime inventory binds bytes; it
does not establish SDK validity or build provenance.

The owner permits a fresh table under `SANDBOX.BILLIE_TESTING`. The earlier
CREATE DATABASE approval is spent. Current source/runtime/executable pins, exact
SQL and metadata scope, a current window and fresh possible-billing approval
remain required before any account attempt. Local synthetic checks do not qualify
the account or normal packaged app. See the [verification ledger](replacement-verification.md).

## Build and prepare inputs

Build the dedicated command from the repository root with the pinned toolchain:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --release --locked --no-default-features --features test-harness --bin snowflake-table-qualification
src-tauri/target/release/snowflake-table-qualification --help
```

The feature shares existing qualification code; this binary does not start the
HTTP test harness. Use the built executable directly. It always binds its own
actual executable; there is no `--executable` override. The exact build recipe is
recorded in the envelope, but the command does not attest that recipe was used.

Supply separate absolute, physical, UTF-8 paths for:

- The repository root, covering the fixed Rust/helper source and build inputs.
- The complete runtime root. Included trees must have no symbolic links, belong
  to the effective user and have no group/other writes or special mode bits.
  The recorded staged runtime has dependency links; a reviewed resource tree with
  zero links is needed. Refer to the package checks in the verification ledger.
- The actual compiler file, rather than a rustup shim or symbolic link.
- A private `0600` JSON profile containing exactly the seven non-secret fields
  below. No password, token, key or additional field is accepted.
- A separate existing `0700` output directory outside every reviewed artifact.
  Source/build/runtime artifacts must not be inside the output directory either.

```json
{
  "account": "synthetic-account",
  "host": "synthetic-account.snowflakecomputing.com",
  "username": "synthetic-person@example.test",
  "role": "QUALIFICATION",
  "warehouse": "SYNTHETIC_WH",
  "database": "SANDBOX",
  "schema": "BILLIE_TESTING"
}
```

The account must match the driver's normalized hostname derivation. This example
is synthetic and is not an acceptance connection. Do not extract private
application state or read credentials to prepare inputs without the owner's
applicable authorization. The command does not discover or adopt a desktop profile;
it reads only the explicitly supplied private input file.
Strict ownership/path checks can reject machine-specific symlinks or writable
ancestors; they do not repair modes or fall back to other paths.

## Emit, review and verify the original evidence

Use one intended approval window of at most 30 minutes, expressed as positive
Unix milliseconds. Offline emission/verification can inspect an expired window;
neither operation supplies a current execution approval.

```sh
src-tauri/target/release/snowflake-table-qualification emit \
  --repository /ABS/repository \
  --runtime /ABS/reviewed-runtime \
  --compiler /ABS/toolchain/bin/rustc \
  --profile /ABS/private/profile.json \
  --output-parent /ABS/private/plan-output \
  --not-before-unix-ms 1000 \
  --expires-unix-ms 2000
```

The example timestamps are deliberately expired. A successful JSON receipt names
one exclusive `plan-<RUN>` directory containing:

- `execution-envelope.json`: exact canonical envelope, at most 16 MiB, with Plan,
  profile/policy, window and complete source/runtime/executable/compiler hashes.
- `plan.json`: exact canonical closed Plan, at most 64 KiB, for review of generated
  SQL, binds, metadata templates, operation order, effects and limits. This must
  match the envelope's `plan_json` string byte for byte.

Both files use `0600` permissions. Successful emission includes file and
directory synchronization plus final content/identity readback. An error retains
any partial output; the command never adopts or overwrites a directory. Do not
delete or edit evidence to make a failed attempt reusable. Only `emit` generates
a new RUN and `JOINERY_QUAL_<RUN>` table name. The receipt prints the envelope and
Plan digests, sole target, 32 phases and the upper bound of 610 SQL statements
(`386 + 7S`, for at most 32 census schemas). No SQL is sent by `emit`.

Review the Plan together with the [physical SQL ledger](snowflake-qualification-sql-ledger.md)
and [lifecycle proposal](snowflake-existing-schema-qualification.md). The Plan's
historical integration-gap fields describe the offline definition alone; current
implementation and unresolved account gates are tracked in the
[capability matrix](datagrip-feature-matrix.md). Emission is not approval.

Use the **original** envelope filename and `executionSha256` from the receipt:

```sh
src-tauri/target/release/snowflake-table-qualification verify \
  --repository /ABS/repository \
  --runtime /ABS/reviewed-runtime \
  --compiler /ABS/toolchain/bin/rustc \
  --profile /ABS/private/profile.json \
  --envelope /ABS/private/plan-output/plan-RUN/execution-envelope.json \
  --sha256 LOWERCASE_SHA256_FROM_EMISSION
```

Verification reads the private envelope and restores the original closed Plan
and window. It recaptures the separately supplied artifacts and requires the
complete canonical envelope to match exactly. It does not regenerate a target
or accept source lists, executable paths, SQL, limits or policy from unchecked
JSON. Changed source, runtime, compiler, executable, profile, window or envelope
requires new review evidence; an old digest cannot qualify new bytes.

Offline success produces one JSON stdout line with `executionAvailable: false`
and `accountActivity: false`. Offline failures produce one JSON stderr line
containing only a closed `failureCode`, and exit status 1. Unknown, duplicate or
missing flags, unencodable/relative paths and invalid time windows are rejected.
Receipts exclude profile data; the envelope itself is private and includes the
non-secret connection fields and local artifact paths. Avoid sharing it publicly.

## Separately approved execution

After exact approval has been obtained, an operator prepares a private `0600`
JSON file of at most 4096 bytes with exactly these fields. The digest must be
from the original reviewed envelope; the placeholder is deliberately invalid:

```json
{
  "format": "joinery-snowflake-table-lifecycle-approval-v1",
  "execution_sha256": "LOWERCASE_SHA256_FROM_APPROVED_EMISSION",
  "scope": "one-externalbrowser-attempt-fixed-table-lifecycle-with-possible-billing",
  "maximum_attempts": 1
}
```

The command cannot verify that a human authorized the file. Recording approval
is an operator responsibility; do not create or use it before the owner's exact
approval. The closed invocation is:

```text
snowflake-table-qualification execute
  --repository ABS --runtime ABS --compiler ABS --profile ABS
  --envelope ABS --sha256 LOWERCASE_SHA256
  --approval ABS --output-parent ABS
```

Execution restores the original envelope against separately supplied roots and
its own executable, validates the approval and current window, reserves new
private attempt/workspace/state/journal/terminal output, and permanently consumes
both RUN and execution digest before construction. Output must be separate from
the marker store and reviewed artifacts. No caller-selected marker root exists.
A new process with another output, `HOME` or `XDG_DATA_HOME` cannot reuse the
spent attempt. Failed, cancelled and partially constructed attempts retain their
markers and evidence. Do not delete them to retry.

The native store location comes from the effective UID's macOS account record:
`Library/Application Support/Joinery/private-qualification/table-lifecycle-consumption-v1`
under that account's physical home. Shared Library/Application Support directories
may be safely `0755`; the Joinery namespace and descendants require `0700`.
Unsafe paths fail without repair, deletion or alternate-root fallback. Other
platforms fail closed. Offline operations never invoke the native resolver.
Tests use a disposable synthetic home and never the owner's Application Support.

## Interruptions, receipts and unresolved outcomes

The dedicated execute process registers SIGINT/SIGTERM before blocking
preparation. An observed signal revokes future admission; preparation, original
calls, shutdown and terminal persistence stay owned and are awaited. Repeated
signals do not refund consumption or cause an automatic retry. Ordinary runtime
shutdown retains blocking work; see the pinned [Tokio runtime contract](https://docs.rs/tokio/1.54.1/tokio/runtime/struct.Runtime.html).
The signal coordinator belongs to this dedicated process; installing Unix signal
handlers changes process behavior for its remaining lifetime, as documented in
[Tokio's signal contract](https://docs.rs/tokio/latest/tokio/signal/unix/struct.Signal.html).

A signal addressed only to the Rust command lets its helper retain a pending
callback. Foreground terminal Ctrl-C reaches the helper's process group too;
the helper exits and an admitted operation can lose its acknowledgement. The
command must preserve that unknown outcome rather than invent success or retry.
Revocation does not establish server rollback, remote cancellation, successful
logout or durable evidence after SIGKILL, power failure or storage failure.
Already started local construction may finish before its next check.

Execution emits one closed JSON summary: successful attempts go to stdout with
exit 0; unsuccessful attempts go to stderr with exit 1. It includes RUN, execution
digest, cancellation observation, any attempt output path and terminal receipt
hash/size, `terminalPersisted`, `successful` and conservative
`mayHaveAccountActivity`. A retained failure can return a receipt; receipt presence
alone does not imply success. Early failures expose only a closed `failureCode`.
Full underlying errors, profiles and SQL are not printed to either stream.
The private terminal/journal contain the original outcomes and separate later
completion, integrity, shutdown and persistence errors. Preserve and inspect
those files before authorizing any cleanup or follow-up. A missing terminal
receipt does not prove there was no account activity.

## Acceptance scope

Actual-binary tests cover offline emission/restoration, unchanged RUN/digests,
read-only verification, tamper and argument rejection, safe failure output and
zero trap helper/compiler launches. Process-level execute tests use the same
parser, signal coordinator, preparation and factory with `cfg(test)`-only
synthetic-home/runtime injection. That injection is absent from the production
binary, including builds with `test-harness`. The helper is real; its SDK is
synthetic. These checks do not establish Snowflake server behavior.

The [verification ledger](replacement-verification.md) records completed tests,
commands, source identities, failures and review. Same-user filesystem races and
storage that misreports synchronization remain outside this proof. Real Snowflake
writes/transactions/DDL, normal packaged SSO/recovery, million-row results,
remaining DataGrip workflows and owner hands-on acceptance remain open.
