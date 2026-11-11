# Snowflake runtime preparation

The external-browser driver uses its own pinned Node runtime and locked SDK
dependencies. Explicit application profile integration and local package/native
qualification pass. New Snowflake profiles choose the SDK route;
existing profiles retain OAuth/SQL API. OAuth remains read-only. The SDK also
provides separately reviewed SQL DML and explicit transaction controls locally;
actual account mutation effects remain unqualified. A prepared stage
alone does not establish a usable packaged workflow. See the
[integration evidence](snowflake-sdk-integration.md) for resource packaging,
verification commands and the remaining required account/write/transaction gates.

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
exit. Production main independently verifies its reviewed transport-source pins
and installs query admission before SDK import. These pins cover selected SDK,
Axios and follow-redirects sources, not every dependency file.
A separate startup socket/DNS guard detects attempted traffic. It is not an OS
sandbox or evidence about SSO, TLS, native addons, subprocesses or account behavior.

For the resource-bearing local application and hidden native fixture:

```sh
pnpm desktop:snowflake:bundle:mac
pnpm test:snowflake:startup --bundle src-tauri/target/release/bundle/macos/Joinery.app
pnpm desktop:snowflake:smoke:build
pnpm test:native --runs 1 --attributed-memory
```

The first build uses the explicit `tauri.snowflake.conf.json` resource mapping.
Release startup selects only the bundle's `Resources/snowflake-runtime`; debug
startup selects only the checkout's fixed stage. Missing or invalid resources
produce an unsupported SDK connection error. Standard bundle/smoke commands
remain available without a stage and cannot connect SDK profiles in that case.
The bundled startup check submits neither authentication nor SQL. The native
checkpoint's runtime-availability field proves loading, not account suitability.
These commands do not install, publish, notarize or authorize external queries.

## Protocol and driver checks

```sh
pnpm test:snowflake:helper
pnpm test:snowflake:process
```

Helper tests require the helper package's locked dependencies installed first
with lifecycle scripts disabled. The suite separates pure/native guard and
source-pin tests, synthetic-SDK Helper/process tests, and actual-SDK loopback
transport. The unchanged baseline has six actual-SDK HTTP cases. The guarded
matrix adds 49 actual-SDK HTTP/verified-TLS/CONNECT scenarios and one fixture
network-boundary self-check; these use synthetic local login, not Snowflake SSO.
Explicitly identified cases drive the production Helper with a preconnected
actual SDK connection. Rust supervisor and
adapter tests use real child processes and a synthetic SDK, with Node selected
explicitly by the runner. They do not authenticate to Snowflake.

These tests demonstrate local framing, bounded streaming, exact binding
transport, cancellation/terminal ordering and process cleanup. They do not
prove remote request deduplication, write outcomes, rollback or a million-row
Snowflake workflow. Broader DML, grid editing, DDL and actual transaction effects
remain required gaps. See the
[driver design](snowflake-driver-design.md), [license audit](snowflake-sdk-licenses.md)
and [required acceptance inventory](snowflake-datagrip-replacement.md).
