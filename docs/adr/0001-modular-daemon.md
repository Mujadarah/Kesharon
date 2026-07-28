# ADR 0001: Supervised Modular Daemon

- Status: Accepted
- Date: 2026-07-29

## Decision

Run the agent core as a separately supervised Rust daemon. Keep domain,
application, and adapter modules in one deployable daemon until profiling or
security evidence justifies an additional process boundary.

The Tauri host supervises the daemon while the application is open or minimized
to the tray. An explicit application exit shuts the daemon down. V1 does not
install an always-on operating-system service.

## Consequences

- Renderer reloads and agent failures are isolated.
- A later CLI can reuse the daemon protocol.
- IPC compatibility and recovery become first-class requirements.
- The extra process has a measurable resource cost that must remain inside the
  published budgets.
