import type { ConnectionState } from "../useWorkspace";

export function ConnectionBadge({ state }: { state: ConnectionState }) {
  if (state.kind === "ready") {
    const version = state.health?.protocolVersion ?? 1;
    return (
      <div className="connection-badge" data-testid="connection-badge">
        <span className="status-dot status-dot--ready" />
        <span>
          <strong>Daemon ready</strong>
          <small>Protocol v{version}</small>
        </span>
      </div>
    );
  }

  if (state.kind === "reconnecting") {
    return (
      <div className="connection-badge connection-badge--warning" data-testid="connection-badge">
        <span className="status-dot status-dot--checking" />
        <span>
          <strong>Reconnecting to daemon</strong>
          <small>Attempting to restore session...</small>
        </span>
      </div>
    );
  }

  if (state.kind === "unavailable") {
    return (
      <div className="connection-badge connection-badge--error" role="alert">
        <span className="status-dot status-dot--error" />
        <span>
          <strong>Daemon unavailable</strong>
          <small>Restart Kesharon, then check the connection again.</small>
        </span>
      </div>
    );
  }

  return (
    <div className="connection-badge" data-testid="connection-badge">
      <span className="status-dot status-dot--checking" />
      <span>
        <strong>Checking daemon</strong>
        <small>Establishing a local connection</small>
      </span>
    </div>
  );
}
