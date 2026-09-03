# Kesharon Master Build Track

Status values are `not-started`, `in-progress`, `blocked`, or `verified`.
`verified` requires recorded commands and outcomes.

| Milestone | Deliverable | Status |
| --- | --- | --- |
| M0 | Constitution, monorepo, CI, architecture and resource gates | in-progress |
| M1 | React/Tauri host, daemon sidecar, IPC, project open and health | in-progress |
| M2 | Domain kernel, SQLite state, credentials, recovery | in-progress |
| M3 | Provider-neutral planning loop and initial adapters | not-started |
| M4 | Permissions, Git worktrees, tools, optional containers | not-started |
| M5 | Complete plan-execute-review Code workflow UI | not-started |
| M6 | Incremental TokenGraph v1 | not-started |
| M7 | Security, reliability, and resource hardening | not-started |
| M8 | Tray schedules and extensibility foundation | not-started |
| M9 | Native packages, signing, provenance, and public beta | not-started |

## Current increment

Implemented:

- Cargo/pnpm workspace, Apache-2.0 license, ADRs, provenance rules, CI matrix,
  CodeQL workflow, architecture checker, and pinned toolchains.
- Framework-free project/task domain types and guarded task transitions.
- Versioned 8 MiB length-prefixed JSON protocol with mutating-request
  idempotency enforcement and redacted 256-bit launch tokens.
- Named-pipe/Unix-socket transport, Unix `0600` socket mode, daemon process,
  authenticated health request, bounded reads, and process-level tests.
- React workbench shell, narrow Tauri command surface, sidecar supervision,
  one-restart budget, tray lifecycle, and reproducible sidecar preparation.
- Concurrent daemon session (8 connection permits, 64-message buffer, 256-entry
  idempotency ledger, cancellation flag propagation).
- Tauri command bridge (`choose_project_directory`, `daemon_open_project`,
  `daemon_cancel_request`, `subscribe_daemon_events`) with background execution.
- Resilient event streaming worker with exponential backoff (50ms–1s), sequence
  gap detection, and authoritative reconnect snapshot synchronization.
- React workspace lifecycle state, repository trust badges, in-flight opening
  progress card with cancellation, and mutation-guarded task composer.
- Windows named pipe same-user ACL isolation verification script and CI job.

Not yet implemented:

- SQLite/FTS5, credential vaults, providers, permissions, Git worktrees, tools,
  TokenGraph, scheduling, updater/signing, SBOM publication, and release
  packaging.
- Resource-budget harnesses and hosted macOS/Linux evidence.

M0 and M1 remain `in-progress` until their full gates, including hosted
cross-platform and resource evidence, pass.

## Gate policy

Each milestone must produce independently testable software. A milestone is not
verified by source review alone: its focused tests, full affected suites,
architecture checks, builds, and relevant packaged-runtime checks must all run
successfully.

## Post-MVP tracks

- Browser-centered Work profile and artifact pipelines
- Google, xAI, OpenRouter, and additional provider adapters
- Signed plugin and skill distribution
- Additional semantic language packs
- Always-on per-user services
- Optional synchronization or team administration as a separate architecture
