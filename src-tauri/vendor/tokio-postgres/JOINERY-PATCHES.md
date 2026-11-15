# Joinery protocol metadata patch

This directory vendors the crates.io release `tokio-postgres` 0.7.18 from
`rust-postgres/rust-postgres`, tag `tokio-postgres-v0.7.18`, upstream revision
`f1cb6ec0d5766b136cbd68f3010d64142a5daa66`. The original registry archive SHA-256
is recorded in `JOINERY-PROVENANCE.json`. Both upstream license texts and source
copyright notices are retained unchanged (`MIT OR Apache-2.0`).

The normalized published Cargo.toml and legitimate upstream sources, tests,
examples, benchmarks, README, and changelog are retained. Registry cache metadata,
registry checksum bookkeeping, the nested Cargo.lock, and build artifacts are
omitted. The application root uses `[patch.crates-io]` so direct and TLS adapter
consumers share one patched client type and feature set. This is a path override,
not a Cargo source-replacement vendor tree; no `.cargo-checksum.json` is needed.

## Local source changes

`patches/0001-expose-actual-column-origins.patch` contains all changes to upstream
Rust sources (three files). It applies with `patch -p1` from the root of a clean
0.7.18 crate. Reversal with `patch -R -p1` reconstructs those three upstream files.
The provenance/patch notes themselves are Joinery additions.

- `src/simple_query.rs`: expose the actual incoming simple-query RowDescription
  table OID, attribute number, and raw type OID. Zero origin values become None;
  negative system attribute numbers remain intact. Empty results still expose
  metadata. This path performs no additional catalog/type queries.
- `src/bind.rs`: send Bind(P), Describe(P), Sync, with no Execute. Populate metadata
  from this bound portal rather than the earlier prepared statement. The portal
  is owned before waiting for BindComplete, so cancellation/errors queue Close(P).
  Custom result types use the existing type-resolution/cache path.
- `src/portal.rs`: expose `Portal::columns()` and let the existing RowStream decoder
  use that actual descriptor. Preserve the original Statement's lifetime
  separately; an unnamed internal Statement carries only the portal's columns.

Existing bind APIs now request Describe(P), adding protocol bytes without another
round trip. Applications may inspect columns and then call query_portal on the
SAME portal. NoData returns empty metadata rather than stale statement columns.
An opt-in API instead of changing all binds is an upstream-maintenance alternative;
this patch chooses the smaller internal change. No upstream submission is implied.

## Regression coverage and scope

`tests/postgres_protocol.rs` in the application contains six synthetic in-memory
protocol scenarios and one opt-in PostgreSQL fixture test with nine real scenarios.
The synthetic peer proves wire ordering, actual metadata, zero rows, NoData,
same-portal execution, and cancellation cleanup. The ignored fixture uses only
127.0.0.1:55432 with synthetic local credentials and UUID-owned schemas/sequence;
case/setup/cleanup waits are bounded and cleanup runs after errors or panics.

Run from the repository root:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres_protocol
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --test postgres_protocol -- --ignored --test-threads=1
```

Both repository commands passed on 2026-11-09: six synthetic protocol tests
and the ignored fixture test containing nine actual PostgreSQL 18.6 scenarios.
The earlier isolated prototype is separate evidence, not an additional test count.

This dependency foundation does not implement application table identity safety.
Original-read/catalog identity capture, browse/CSV retention, pre-DML portal
validation, schema consistency, privileges/RLS/rules, supported server versions,
and application integration remain separate work. Returning a logical root OID
does not attest all trigger/rule/partition effects or make OIDs permanent UUIDs.
Planning/type input functions may run during Parse/Bind; pre-Execute validation
prevents starting the DML executor, not every conceivable SQL function side effect.
