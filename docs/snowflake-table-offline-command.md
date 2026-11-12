# Offline Snowflake table qualification command

This command prepares and checks local review evidence for the proposed
[existing-schema lifecycle](snowflake-existing-schema-qualification.md). It has
**no execution command** and grants no account or billing authority. It does not
start Node, invoke the compiler, initialize Studio or the keychain, open SSO,
connect to Snowflake, or create permanent consumption markers. Runtime inventory
is byte binding; it does not establish SDK validity or build provenance.

The separately reviewed account command and complete fixed-entry acceptance
remain required before requesting a new execution approval. Internal storage now
provides a fixed macOS marker namespace, and prepared attempts provide an explicit
retained revocation handle; neither is reachable from these offline operations.
Permission to
create a fresh table under `SANDBOX.BILLIE_TESTING` remains recorded. The earlier
CREATE DATABASE approval is spent and cannot be reused.

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
(`386 + 7S`, for at most 32 census schemas). No SQL is sent by this command.

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

Success produces one JSON stdout line with `executionAvailable: false` and
`accountActivity: false`. Failures produce one JSON stderr line containing only
a closed `failureCode`, and exit status 1. Unknown/duplicate/missing flags,
unencodable/relative paths, invalid time windows and `execute` are rejected.
Receipts exclude profile data; the envelope itself is private and includes the
non-secret connection fields and local artifact paths. Avoid sharing it publicly.

## Acceptance scope

The private native-store entry selects its permanent location from the
effective UID's OS account record, independent of `HOME`, output or configuration:
`Library/Application Support/Joinery/private-qualification/table-lifecycle-consumption-v1`
under that account's physical home. Shared Library/Application Support directories
may have safe `0755` permissions; the Joinery namespace and its descendants require
`0700`. Unsafe existing paths fail without repair, deletion or alternate-root
fallback. Native lookup fails closed outside macOS. Offline `emit`/`verify` never
invoke that resolver or create this namespace. Do not delete markers to retry an
attempt. Synthetic tests use a private temporary home, not the owner's home.

A private revocation handle can stop future admissions while retaining the
original call, shutdown and terminal evidence. Dropping its handle does not revoke;
dropping the attempt's waiter does not abort the owner. This is not yet wired to
an executable account route or process signal handling. Already started local
construction may finish before its next check. Revocation does not imply server
rollback, successful logout, remote cancellation or durable evidence after a
process kill/power failure. Inspect recorded original outcomes before deciding
what cleanup or follow-up to authorize; do not replay a spent attempt.

The process regression suite invokes the actual binary against disposable
synthetic files, including trap Node/compiler executables. It checks emission,
cross-process restoration, unchanged RUN/digests, read-only verification, tamper
rejection, safe failure output and zero trap launches. Storage tests cover
exclusive creation, permissions, bounds, links, concurrent same-RUN emission and
retained partial outputs. These checks establish only local offline behavior.

Same-user races between checks and storage that misreports synchronization remain
outside this filesystem proof. Real Snowflake writes/transactions/DDL, normal
packaged SSO/recovery, million-row results, remaining DataGrip workflows and
owner hands-on acceptance remain open. See the
[verification ledger](replacement-verification.md) for completed commands,
results and source-specific limits.
