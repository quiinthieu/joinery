# Joinery

A desktop database studio built with Tauri v2, Rust, React, CodeMirror, and a Canvas data grid.

The approved [development blueprint](https://chatgpt.com/space/page_c4b9e62285c481918eec36b706cca3a7) is also recorded in [docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md).

## Development

Install Node 24, pnpm 11, the pinned Rust toolchain, and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm desktop
```

Run the frontend separately with `pnpm dev`. Desktop-only functionality uses the Rust core.

```sh
pnpm check
pnpm test:rust
pnpm desktop:build
```

## Implementation status

The approved roadmap starts with performance feasibility and the PostgreSQL foundation. Later engine adapters and distribution milestones are tracked in the development blueprint.
