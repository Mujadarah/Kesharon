# E2E Test Infra: Kesharon M1 Phase 2

## Test Philosophy
- Opaque-box, requirement-driven, and boundary-enforced.
- Methodology: Category-Partition + Boundary Value Analysis (BVA) + Pairwise Combinatorial + Real-World Workload Testing.

## Feature Inventory
| # | Feature | Source | Tier 1 | Tier 2 | Tier 3 |
|---|---|---|:---:|:---:|:---:|
| 1 | `choose_project_directory` | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 2 | `daemon_open_project` | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 3 | `daemon_cancel_request` | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 4 | `subscribe_daemon_events` | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 5 | `DaemonClient` Expansion | ORIGINAL_REQUEST §R1, R2 | 5 | 5 | ✓ |
| 6 | Stream Subscription & Snapshot | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ |
| 7 | Sequence Gap & Disconnect Recovery | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ |
| 8 | Exponential Backoff (50ms–1s) | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ |
| 9 | Protocol Synchronization | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 10 | `DesktopBridge` Implementation | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 11 | Workspace State Management | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 12 | Project Open & Cancel UI | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 13 | Repository State Presentation | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 14 | Task Composer Input Activation | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |

## Test Architecture
- **Host Integration Tests**: Headless Rust integration tests in `apps/desktop/src-tauri/tests/` testing IPC command dispatch, stream subscription, sequence gap detection, and daemon restart recovery.
- **Frontend Unit & Component Tests**: Vitest + React Testing Library in `apps/desktop/src/` testing `DesktopBridge`, `useWorkspace`, and `App` components with deterministic mock bridge and stream message emitters.
- **Quality Gates**:
  - `cargo test --workspace --all-targets` (100% pass)
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `pnpm check` (`check:architecture`, `pnpm lint`, `pnpm test`, `pnpm build`)

## Real-World Application Scenarios (Tier 4)
| # | Scenario | Features Exercised | Complexity |
|---|---|---|---|
| 1 | Full Happy Path Project Lifecycle | F1, F2, F4, F6, F11, F12, F13, F14, F15 | High |
| 2 | In-Flight Cancellation on High-Latency Repo | F2, F3, F4, F11, F12, F13 | High |
| 3 | Daemon Crash & Auto-Restart with Snapshot Resync | F5, F6, F7, F8, F11, F12, F14 | High |
| 4 | Network Socket Drop & Sequence Gap Recovery | F6, F7, F8, F12 | Medium |
| 5 | Non-Git Directory Rejection & UI Recovery | F1, F2, F10, F11, F12, F13 | Medium |
| 6 | Webview Remount & Subscription Supersedence | F4, F6, F9, F11, F12 | Medium |

## Coverage Thresholds
- Tier 1: ≥5 test cases per feature (Happy-path isolated verification)
- Tier 2: ≥5 boundary & corner test cases per feature (Limits, error payloads, timeouts, saturated slots)
- Tier 3: Pairwise coverage of major feature interactions
- Tier 4: ≥6 realistic application scenarios
- Tier 5: Adversarial white-box coverage hardening
