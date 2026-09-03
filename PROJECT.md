# Project: Kesharon M1 Phase 2

## Architecture
Kesharon is a reliable, local-first developer agent application built with a Rust backend workspace and a TypeScript/React desktop webview.
- **Backend Crates**:
  - `kesharon-domain`: Pure domain entities (`Project`, `Task`, `Budget`), strictly zero external dependencies (std only).
  - `kesharon-application`: Application use-cases (`OpenProject`, `RepositoryService`, `CancellationSignal`), depends only on domain and std.
  - `kesharon-protocol`: Versioned cross-boundary protocol frames (`ClientRequest`, `ServerResponse`, `StreamMessage`, `WorkspaceSnapshot`, `DaemonEvent`).
  - `kesharon-ipc`: Local socket IPC transport with SDDL on Windows (`D:P(A;;GA;;;OW)(A;;GA;;;SY)`) and `0o600` permissions on Unix.
  - `kesharon-daemon`: Authoritative background session manager, Git worktree inspector, concurrency controller (8 connections), event sequence generator.
  - `kesharon-desktop-host`: Native Tauri host supervisor, sidecar launcher, IPC client (`DaemonClient`), stream worker, and Tauri command bridge.
- **Frontend Packages**:
  - `@kesharon/protocol`: TypeScript protocol decoders, types, and schemas matching Rust protocol.
  - `@kesharon/desktop`: React 19 webview application, unprivileged `DesktopBridge`, workspace lifecycle management, UI components.

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---|---|---|---|
| F1 | `choose_project_directory` Command | Native folder picker via `tauri-plugin-dialog` executed asynchronously off the main UI loop | M1 | ORIGINAL_REQUEST §R1 |
| F2 | `daemon_open_project` Command | Tauri command dispatching `OpenProject` request over IPC off UI thread | M1 | ORIGINAL_REQUEST §R1 |
| F3 | `daemon_cancel_request` Command | Tauri command dispatching `CancelRequest` over IPC off UI thread | M1 | ORIGINAL_REQUEST §R1 |
| F4 | `subscribe_daemon_events` Command | Tauri command creating event streaming channel to React webview | M1 | ORIGINAL_REQUEST §R1 |
| F5 | `DaemonClient` API Expansion | Generalize `DaemonClient` with `send_request`, `open_project`, `cancel_request`, and `subscribe` | M1 | ORIGINAL_REQUEST §R1, R2 |
| F6 | Authoritative Stream Subscription | Connect to daemon session stream, decode initial `WorkspaceSnapshot` and live `DaemonEvent`s | M2 | ORIGINAL_REQUEST §R2 |
| F7 | Sequence Gap & Disconnect Recovery | Detect stream id changes or sequence gaps, reconnect and request authoritative snapshot | M2 | ORIGINAL_REQUEST §R2 |
| F8 | Exponential Backoff Reconnection | 50ms–1s backoff (50ms base doubling up to 1000ms) on disconnect, EOF, or daemon restart | M2 | ORIGINAL_REQUEST §R2 |
| F9 | Subscription Supersedence | Clean replacement of active stream channel when new subscription is requested | M2 | ORIGINAL_REQUEST §R2 |
| F10 | Protocol Package Synchronization | Explicit type exports and decoders in `@kesharon/protocol` (`CancellationOutcome`, `ProjectOpenResult`) | M3 | ORIGINAL_REQUEST §R3 |
| F11 | `DesktopBridge` Interface & Tauri Bridge | Full unprivileged TS bridge interface and Tauri invoke/channel implementation | M3 | ORIGINAL_REQUEST §R3 |
| F12 | Workspace State Management | `useWorkspace` hook reacting to stream snapshots, events, and user actions | M3 | ORIGINAL_REQUEST §R3 |
| F13 | Project Opening & Cancellation UI | "Add project" / "Open repository" buttons, in-flight opening progress card with Cancel action | M3 | ORIGINAL_REQUEST §R3 |
| F14 | Opened Repository State Presentation | Render canonical root path, display name, trust badge (`Trusted` / `Untrusted (Sandbox)`) | M3 | ORIGINAL_REQUEST §R3 |
| F15 | Task Composer Input Activation | Enable composer `<textarea>` when project is open; keep mutation execution safely disabled | M3 | ORIGINAL_REQUEST §R3 |
| F16 | Automated Host & Bridge Test Suites | Host integration tests for commands, reconnect, restart recovery; TypeScript component tests | M4 | ORIGINAL_REQUEST §Acceptance Criteria |
| F17 | Quality Gates & Adversarial Hardening | Full workspace validation: `cargo test`, `cargo fmt`, `cargo clippy`, `pnpm check`, Tier 5 adversarial tests | M4 | ORIGINAL_REQUEST §Acceptance Criteria |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|---|---|---|---|
| M1 | Desktop Host Transport & Commands | Implement `choose_project_directory`, `daemon_open_project`, `daemon_cancel_request`, `subscribe_daemon_events`, `DaemonClient` methods in `apps/desktop/src-tauri` | none | DONE |
| M2 | Stream Recovery, Snapshot & Backoff | Implement stream worker with exponential backoff (50ms–1s), sequence gap detection, authoritative snapshot reload, and restart recovery | M1 | DONE |
| M3 | React Workbench Workspace Integration | Implement `@kesharon/protocol` exports, `DesktopBridge`, `useWorkspace`, project opening/cancel UI, repo state display, composer activation in `apps/desktop` | M1 | IN_PROGRESS |
| M4 | E2E Integration, Quality Gates & Hardening | Run all host integration tests, webview component tests, full quality gates (`cargo test`, `cargo fmt`, `cargo clippy`, `pnpm check`), Tier 5 adversarial tests | M1, M2, M3 | PLANNED |

## Interface Contracts

### Desktop Host Tauri Commands (Rust ↔ Webview)
- `choose_project_directory()` -> `Result<Option<String>, String>`
- `daemon_open_project(path: String, request_id: Option<String>, idempotency_key: Option<String>)` -> `Result<ServerResponse, String>`
- `daemon_cancel_request(target_request_id: String, request_id: Option<String>, idempotency_key: Option<String>)` -> `Result<ServerResponse, String>`
- `subscribe_daemon_events(on_event: Channel<DaemonStreamUpdate>)` -> `Result<(), String>`
- `daemon_health()` -> `Result<ServerResponse, String>`

### `DaemonStreamUpdate` (Host ↔ Webview IPC Channel)
```rust
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DaemonStreamUpdate {
    Message { message: StreamMessage },
    Reconnecting,
    Unavailable { message: String },
}
```

### `DesktopBridge` Interface (TypeScript Webview)
```typescript
export interface DesktopBridge {
  getHealth(): Promise<HealthSnapshot>;
  chooseProjectDirectory(): Promise<string | null>;
  openProject(path: string): Promise<{ requestId: string; project: ProjectSnapshot }>;
  cancelRequest(targetRequestId: string): Promise<{
    requestId: string;
    targetRequestId: string;
    outcome: CancellationOutcome;
  }>;
  subscribeDaemonEvents(onMessage: (message: StreamMessage) => void): Promise<() => void>;
}
```

## Code Layout
```
kesharon/
├── crates/
│   ├── kesharon-domain/           # Pure domain models (std only)
│   ├── kesharon-application/      # Application use-cases & port traits (domain + std)
│   ├── kesharon-protocol/         # Wire protocol & frame codecs
│   ├── kesharon-ipc/              # Local IPC stream & security descriptors
│   └── kesharon-daemon/           # Authoritative daemon server & session runtime
├── apps/
│   └── desktop/
│       ├── src-tauri/             # Rust desktop host (Tauri commands, supervisor, stream worker)
│       │   ├── Cargo.toml
│       │   ├── src/
│       │   │   ├── lib.rs         # Tauri commands, stream worker, DaemonClient
│       │   │   └── main.rs
│       │   └── tests/
│       │       ├── daemon_client.rs
│       │       ├── commands.rs
│       │       ├── stream_recovery.rs
│       │       └── supervision.rs
│       └── src/                   # React 19 webview
│           ├── App.tsx
│           ├── bridge.ts
│           ├── useWorkspace.ts
│           ├── components/
│           │   ├── ConnectionBadge.tsx
│           │   ├── ProjectRail.tsx
│           │   ├── TaskComposer.tsx
│           │   ├── WorkflowRail.tsx
│           │   ├── ReviewPanel.tsx
│           │   └── ResourcePanel.tsx
│           └── app.css
├── packages/
│   └── protocol/                  # Shared TypeScript protocol decoders
│       └── src/index.ts
└── scripts/                       # Architecture & sidecar build scripts
```
