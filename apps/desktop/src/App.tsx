import { useEffect, useState } from "react";

import type { HealthSnapshot } from "@kesharon/protocol";

import "./app.css";

export interface DesktopBridge {
  getHealth(): Promise<HealthSnapshot>;
}

type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; health: HealthSnapshot }
  | { kind: "unavailable" };

export function App({ bridge }: { bridge: DesktopBridge }) {
  const [connection, setConnection] = useState<ConnectionState>({
    kind: "checking"
  });

  useEffect(() => {
    let active = true;

    bridge.getHealth().then(
      (health) => {
        if (active) {
          setConnection({ kind: "ready", health });
        }
      },
      () => {
        if (active) {
          setConnection({ kind: "unavailable" });
        }
      }
    );

    return () => {
      active = false;
    };
  }, [bridge]);

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="wordmark" aria-label="Kesharon">
          <span className="wordmark-mark" aria-hidden="true">
            K
          </span>
          <span>
            <strong>Kesharon</strong>
            <small>Local agent workbench</small>
          </span>
        </div>
        <ConnectionBadge state={connection} />
      </header>

      <main className="workspace">
        <nav className="project-rail panel" aria-label="Workspace navigation">
          <div className="panel-heading">
            <span className="eyebrow">Workspace</span>
            <button className="icon-button" type="button" aria-label="Add project">
              +
            </button>
          </div>
          <div className="empty-project">
            <span className="repository-glyph" aria-hidden="true" />
            <p>No repository open</p>
            <span>Choose a Git repository to begin a reviewed task.</span>
            <button className="secondary-action" type="button">
              Open repository
            </button>
          </div>
          <div className="rail-section">
            <span className="eyebrow">Recent sessions</span>
            <p className="quiet-copy">Sessions will stay on this device.</p>
          </div>
        </nav>

        <section className="plan-panel panel" aria-label="Plan and assistant">
          <div className="plan-intro">
            <span className="eyebrow">New task</span>
            <h1>Plan before execution</h1>
            <p>
              Describe the outcome. Kesharon will inspect the repository and
              prepare a bounded plan before requesting permission.
            </p>
          </div>

          <label className="task-composer">
            <span className="sr-only">Task goal</span>
            <textarea
              rows={4}
              placeholder="What should change in this repository?"
              disabled
            />
            <span className="composer-footer">
              <span>Open a repository to enable planning</span>
              <button type="button" disabled>
                Prepare plan
              </button>
            </span>
          </label>

          <ol className="execution-rail" aria-label="Task workflow">
            <WorkflowStep
              label="Plan"
              detail="Repository evidence and proposed steps"
              state="current"
            />
            <WorkflowStep
              label="Approve"
              detail="Paths, commands, network, and isolation"
            />
            <WorkflowStep
              label="Execute"
              detail="Bounded tools in a managed worktree"
            />
            <WorkflowStep
              label="Review"
              detail="Diff, tests, resources, and final decision"
            />
          </ol>
        </section>

        <section className="review-panel panel" aria-label="Files, diff, and review">
          <div className="panel-heading">
            <span className="eyebrow">Review</span>
            <span className="count-pill">0 changes</span>
          </div>
          <div className="review-empty">
            <div className="diff-lines" aria-hidden="true">
              <span />
              <span />
              <span />
              <span />
            </div>
            <h2>Nothing proposed yet</h2>
            <p>
              Approved edits, test results, and inline review comments will
              appear here.
            </p>
          </div>
        </section>

        <section className="resource-panel" aria-label="Resource monitor">
          <ResourceMetric label="CPU" value="—" />
          <ResourceMetric label="Memory" value="—" />
          <ResourceMetric label="Disk writes" value="—" />
          <ResourceMetric label="Cache" value="—" />
          <ResourceMetric label="Workers" value="0 / 2" />
          <div className="isolation-state">
            <span className="status-dot status-dot--neutral" />
            Native execution not started
          </div>
        </section>
      </main>
    </div>
  );
}

function ConnectionBadge({ state }: { state: ConnectionState }) {
  if (state.kind === "ready") {
    return (
      <div className="connection-badge" role="status">
        <span className="status-dot status-dot--ready" />
        <span>
          <strong>Daemon ready</strong>
          <small>Protocol v{state.health.protocolVersion}</small>
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
    <div className="connection-badge" role="status">
      <span className="status-dot status-dot--checking" />
      <span>
        <strong>Checking daemon</strong>
        <small>Establishing a local connection</small>
      </span>
    </div>
  );
}

function WorkflowStep({
  label,
  detail,
  state = "waiting"
}: {
  label: string;
  detail: string;
  state?: "current" | "waiting";
}) {
  return (
    <li className={`workflow-step workflow-step--${state}`}>
      <span className="workflow-node" aria-hidden="true" />
      <span>
        <strong>{label}</strong>
        <small>{detail}</small>
      </span>
    </li>
  );
}

function ResourceMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="resource-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
