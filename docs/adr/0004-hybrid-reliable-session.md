# ADR 0004: Hybrid Reliable Session Architecture

- Status: Accepted
- Date: 2026-09-03

## Context

Kesharon requires reliable synchronization between an unprivileged React webview, a narrow Tauri desktop host, and a supervised Rust daemon. Command dispatch and long-lived event monitoring have conflicting requirements: commands are short-lived, request-response, and require strict idempotency and cancellation, while event streaming requires continuous authoritative state delivery without head-of-line blocking or unbounded resource accumulation.

Multiplexing all commands over a single stateful socket introduces connection-level serialization and failure coupling. Polling introduces latency, CPU churn, and battery drain.

## Decision

Adopt a hybrid architecture combining short-lived authenticated command connections with exactly one persistent authoritative event stream.

1. **Short-Lived Command Connections**:
   - Each mutation (`openProject`, `cancelRequest`, `health`) uses an independent authenticated IPC connection.
   - Concurrency is bounded by an eight-connection permit cap (`MAX_ACTIVE_CONNECTIONS = 8`). An excess authenticated request receives `serverBusy`.
   - Mutations carry client-generated idempotency UUIDs. In-flight duplicates return `requestInProgress`; identical completed mutations replay stored results from a 256-entry eviction ledger without re-executing.

2. **Single Authoritative Event Stream**:
   - Exactly one event subscriber is active at any time. A new subscription captures a consistent `WorkspaceSnapshot` under the session lock and replaces any previous subscriber.
   - Events carry monotonic sequence numbers starting at one.
   - Stream buffer is capped at 64 messages. Overflow forcibly disconnects the subscriber.
   - Clients never infer across sequence gaps or stream ID changes; they disconnect, back off exponentially (50 ms to 1 s), and request a fresh snapshot.

3. **Desktop Host & Webview Bridge**:
   - The Tauri host supervises the stream worker, handles backoff reconnection across daemon restarts, and forwards typed `DaemonStreamUpdate` objects through Tauri `Channel`.
   - The React webview remains unprivileged and operates through a typed `DesktopBridge` interface.

## Consequences

- Command operations never block the accept loop or event distribution.
- Bounded thread and memory allocations prevent runaway resource consumption.
- Client state is guaranteed to reconcile with daemon authority after network drops, restarts, or buffer overflows.
- Re-executing mutations after ledger eviction permits idempotent retries without unbounded memory retention.
