# Snowflake runtime preparation

The externalbrowser driver foundation uses its own pinned Node runtime and
locked SDK dependencies. Application profile integration, Tauri resource
packaging, signing/notarization and actual Snowflake qualification are still
open. A prepared stage is not an installed or usable application capability.
The existing application continues to use the OAuth/SQL API route.

## Prepare and check on macOS arm64

Use Node 24 or newer and the repository's pnpm version for development commands.
The prepared runtime itself is exactly Node 24.22.2 and SDK 3.4.1. The preparation
script supports macOS arm64 only; it never changes global Node/npm settings.

Obtain the archive and checksum list from the official
[Node 24.22.2 release directory](https://nodejs.org/dist/v24.22.2/), storing them
at `.cache/toolchains/node-v24.22.2-darwin-arm64.tar.gz` and
`.cache/toolchains/node24-shasums.txt`. The script also checks an independently
pinned archive SHA-256, binary hash, architecture, version and original signature.
It extracts npm from that same verified archive and uses isolated npm
configuration with lifecycle scripts disabled.

```sh
pnpm snowflake:prepare
pnpm test:snowflake:runtime
pnpm test:snowflake:startup
```

The first command installs the exact lockfile dependencies from the public npm
registry into a private staging root, copies source and package notices, then
atomically promotes `.cache/snowflake-runtime/macos-arm64/`. It includes a
manifest of hashes, dependency versions and copied notices. No authentication
or database query runs. Repeated preparation keeps the previous stage intact on
failure before promotion. A backup-cleanup warning after promotion means the new
stage is usable and the old backup remains for inspection.

For a populated repository-owned npm cache, offline preparation is:

```sh
pnpm snowflake:prepare --cache-dir .cache/snowflake-helper-dev/npm-cache --offline
```

Use the cache directory actually populated by your earlier installation;
offline mode cannot fill missing registry artifacts. It fails without replacing
the previous stage. Do not point preparation at private application state or
credential caches.

The startup check verifies current source/manifest hashes and the installed SDK
version, starts the absolute staged Node/helper with an isolated parent-owned
temporary state directory, reads one bounded hello frame and checks clean EOF
exit. A Node socket/DNS guard detects attempted traffic. This guard is not an OS
sandbox or evidence about SSO, TLS, native addons, subprocesses or account behavior.

## Protocol and driver checks

```sh
pnpm test:snowflake:helper
pnpm test:snowflake:process
```

Helper tests require the helper package's locked dependencies installed first
with lifecycle scripts disabled. Six tests use the actual SDK against local
loopback HTTP; remaining helper tests use synthetic SDKs. Rust supervisor and
adapter tests use real child processes and a synthetic SDK, with Node selected
explicitly by the runner. They do not authenticate to Snowflake.

These tests demonstrate local framing, bounded streaming, exact binding
transport, cancellation/terminal ordering and process cleanup. They do not
prove remote request deduplication, write outcomes, rollback or a million-row
Snowflake workflow. Writes and transactions remain disabled. See the
[driver design](snowflake-driver-design.md), [license audit](snowflake-sdk-licenses.md)
and [required acceptance inventory](snowflake-datagrip-replacement.md).
