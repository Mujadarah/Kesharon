# Windows Foundation Evidence — 2026-07-29

This record covers the `feature/m0-m1-foundation` worktree on Windows
`x86_64-pc-windows-msvc`. It is not evidence for macOS, Linux, signing,
installation, or the unimplemented milestones.

## Toolchain

- Rust `1.97.1`
- Cargo `1.97.1`
- Node.js `24` baseline in manifests
- pnpm `11.9.0`
- Tauri `2.11.5`

## Passing gates

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
pnpm check
pnpm --filter @kesharon/desktop tauri build --no-bundle
```

Observed results:

- 30 Rust tests passed.
- 11 architecture, tooling, protocol, and React tests passed.
- TypeScript checks and both production frontend builds passed.
- Architecture boundaries passed across five checked crate manifests.
- Tauri produced `target/release/kesharon.exe` at 11,332,608 bytes.
- Tauri staged `target/release/kesharon-daemon.exe` at 790,016 bytes.
- A packaged-runtime process probe observed one live host and exactly one
  daemon child after startup.
- Playwright inspected the real React shell at 1440×900 and 760×760.

## Evidence limits and open gates

- The hosted Windows/macOS/Linux CI jobs have not run because the branch has
  not been published.
- The browser-only Playwright run intentionally showed `Daemon unavailable`;
  authenticated health was verified through Rust integration tests and the
  packaged host/child process probe, not through browser-only Vite.
- No installer, signature, updater, SBOM, or downloaded-artifact verification
  exists.
- CPU, RAM, disk-write, WAL, and idle-soak baselines are not measured yet.
- Project opening, event streaming, reconnect snapshots, and cancellation are
  not implemented.
- Windows named-pipe same-user ACL hardening is not yet independently proven.
