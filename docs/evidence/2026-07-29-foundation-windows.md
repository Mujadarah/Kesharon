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
pnpm prepare:sidecar:release
pnpm --filter @kesharon/desktop tauri build --no-bundle
```

Observed results:

- 34 Rust tests passed.
- 15 architecture, tooling, protocol, and React tests passed.
- TypeScript checks and both production frontend builds passed.
- Architecture boundaries passed across five checked crate manifests.
- Tauri produced `target/release/kesharon.exe` at 11,327,488 bytes.
- The isolated release sidecar build produced a 345,600-byte daemon. Its
  SHA-256 matched the binary staged for Tauri:
  `3E545A14FD675EA05E0AF1DED0C075CE998829C389C43EB29F97AF8F1A09EDE4`.
- The Tauri runtime copy matched the same release-sidecar SHA-256.
- A locally built release-executable probe observed one live host and exactly
  one
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
