# Windows M1 Reliability & Project Session Evidence — 2026-09-03

This record covers the M1 milestone completion on Windows `x86_64-pc-windows-msvc`. It documents verified local gates, integration test outcomes, binary provenance, and security isolation evidence.

## Toolchain

- Rust `1.97.1` (MSVC)
- Cargo `1.97.1`
- Node.js `24.13.3` / `25.6.1`
- pnpm `11.9.0`
- Tauri `2.11.5`

## Passing Local Gates

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --all-features --release -- -D warnings
cargo test --workspace --all-targets
pnpm check
pnpm prepare:sidecar:release
pnpm --filter @kesharon/desktop tauri build --no-bundle
pwsh -NoProfile -File scripts/test-windows-pipe-acl.ps1
```

### Observed Results

- **Rust Workspace**: 97 tests passed across all 5 workspace crates and the desktop host (0 failed, 0 ignored).
- **Frontend & Protocol Workspace**: 59 tests passed (36 desktop vitest tests, 13 protocol vitest tests, 10 node:test architecture and tooling tests).
- **TypeScript & Lint**: `pnpm lint` and `tsc` passed with 0 errors.
- **Strict Linting**: `cargo clippy` passed with `-D warnings` in both `dev` and `release` configurations with 0 warnings.
- **Architecture Integrity**: `scripts/check-architecture.mjs` confirmed strict inward-only dependencies across all 5 crate manifests.
- **Sidecar Provenance**:
  - `target\release\kesharon-daemon.exe`: SHA-256 `45C702AC9BE50E55DDD33989431EDA1D1F4A52EBD6C68134344F39085B35294B`
  - `apps\desktop\src-tauri\binaries\kesharon-daemon-x86_64-pc-windows-msvc.exe`: SHA-256 `45C702AC9BE50E55DDD33989431EDA1D1F4A52EBD6C68134344F39085B35294B`
  - Hashes are verified identical.
- **Tauri Application Build**: `tauri build --no-bundle` compiled `target\release\kesharon.exe` cleanly.
- **Named Pipe Security & Isolation**:
  - Windows named pipe SDDL `D:P(A;;GA;;;OW)(A;;GA;;;SY)` verified.
  - Same-user authenticated connect and health response round-trip verified via `scripts/test-windows-pipe-acl.ps1`.

## Implemented Deliverables

- **Host IPC & Commands**: Dedicated asynchronous Tauri commands (`choose_project_directory`, `daemon_open_project`, `daemon_cancel_request`, `subscribe_daemon_events`) off the UI thread.
- **Stream Worker & Recovery**: Single authoritative subscriber with 64-message buffer, exponential backoff (50ms–1s), sequence gap detection (`lastSequence + 1`), and automatic reconnect snapshot synchronization.
- **React Workbench Shell**: Unprivileged `DesktopBridge`, workspace lifecycle reducer, repo identity and trust status presentation, real-time opening progress card with functional cancellation, and mutation-guarded task composer input.

## Evidence Limits and Open Gates

- M2 SQLite/FTS5 durable storage, OS credential vaults, and crash recovery are part of the next milestone.
- M3 providers/planning, M4 Git worktrees/sandbox tools, and M6 TokenGraph indexing remain open.
- Hosted cross-platform Linux/macOS and Windows CI evidence runs upon GitHub PR submission.
