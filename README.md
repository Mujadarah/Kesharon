# Kesharon

Kesharon is an Apache-2.0, local-first desktop agent for planning, executing,
and reviewing repository work under explicit user control.

The project is currently implementing M0 and M1. The repository contains a
tested domain kernel, versioned authenticated IPC, a supervised daemon
sidecar, and a React/Tauri workbench shell. It does not yet contain a
production-ready agent, sandbox, durable SQLite state, provider integration,
TokenGraph indexing, or public release artifact.

## Product principles

- Plans and permission decisions precede mutations.
- The React webview has no direct filesystem, shell, database, provider, or
  credential access.
- A supervised Rust daemon owns agent execution and durable state.
- Domain and application code are independent of UI and infrastructure
  frameworks.
- Local content and secrets are private by default.
- CPU, memory, disk writes, logs, and caches have enforceable budgets.
- Native Windows, macOS, and Linux behavior is tested directly.

## Planned repository layout

```text
apps/desktop/          React presentation and narrow Tauri host
crates/                Rust domain, application, protocol, daemon, and adapters
packages/              Shared TypeScript packages
scripts/               Architecture and verification tooling
docs/                  Architecture, decisions, security, and roadmap
```

See [the product architecture](docs/architecture/product-architecture.md) and
[master build track](docs/roadmap/master-track.md).

## Current verification

On Windows, the release-mode desktop host builds and launches with exactly one
supervised daemon child. Rust formatting, strict Clippy, the Rust workspace
tests, architecture tests, TypeScript checks, component tests, and frontend
production build pass locally.

macOS and Linux native builds are configured in CI but remain unverified until
the first hosted run completes. See
[the Windows reliability evidence](docs/evidence/2026-09-03-m1-reliability.md)
for the exact boundary.

## Development prerequisites

- Rust stable, installed with `rustup`
- Node.js 24 or later
- pnpm 11
- Platform prerequisites from the Tauri 2 documentation

The exact toolchain and package versions are pinned by repository manifests and
lockfiles.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
