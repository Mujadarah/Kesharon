import type { ProjectSnapshot } from "@kesharon/protocol";
import type { OpeningOperationState } from "../useWorkspace";

interface ProjectRailProps {
  project: ProjectSnapshot | null;
  openingOperation: OpeningOperationState | null;
  openingError: string | null;
  onChooseAndOpen: () => void;
  onCancelOpening: () => void;
  onClearError: () => void;
}

export function ProjectRail({
  project,
  openingOperation,
  openingError,
  onChooseAndOpen,
  onCancelOpening,
  onClearError
}: ProjectRailProps) {
  const isOpening = openingOperation !== null;
  const isCancelling = openingOperation?.status === "cancelling";

  return (
    <nav className="project-rail panel" aria-label="Workspace navigation">
      <div className="panel-heading">
        <span className="eyebrow">Workspace</span>
        <button
          className="icon-button"
          type="button"
          aria-label="Add project"
          onClick={onChooseAndOpen}
          disabled={isOpening}
        >
          +
        </button>
      </div>

      {openingError && (
        <div className="error-banner" role="alert">
          <div className="error-content">
            <span className="status-dot status-dot--error" />
            <p>{openingError}</p>
          </div>
          <button
            type="button"
            className="dismiss-button"
            aria-label="Dismiss error"
            onClick={onClearError}
          >
            Dismiss
          </button>
        </div>
      )}

      {isOpening && (
        <div className="opening-card" role="status" aria-live="polite">
          <div className="opening-header">
            <span className="status-dot status-dot--checking" />
            <strong>Opening repository...</strong>
          </div>
          {openingOperation.path ? (
            <small className="canonical-root">{openingOperation.path}</small>
          ) : (
            <small className="canonical-root">Inspecting directory...</small>
          )}
          <div className="opening-actions">
            <button
              type="button"
              className="cancel-action"
              aria-label="Cancel opening"
              disabled={isCancelling}
              onClick={onCancelOpening}
            >
              {isCancelling ? "Cancelling..." : "Cancel"}
            </button>
          </div>
        </div>
      )}

      {!isOpening && project && (
        <div className="project-card">
          <div className="project-header">
            <span className="repository-glyph-small" aria-hidden="true" />
            <div className="project-info">
              <strong>{project.displayName}</strong>
              <small className="canonical-root" title={project.canonicalRoot}>
                {project.canonicalRoot}
              </small>
            </div>
          </div>
          <div className="project-meta">
            <span
              className={`trust-badge ${
                project.trusted ? "trust-badge--trusted" : "trust-badge--untrusted"
              }`}
            >
              {project.trusted ? "Trusted" : "Untrusted (Sandbox)"}
            </span>
          </div>
          <button
            className="secondary-action"
            type="button"
            onClick={onChooseAndOpen}
          >
            Open another repository
          </button>
        </div>
      )}

      {!isOpening && !project && (
        <div className="empty-project">
          <span className="repository-glyph" aria-hidden="true" />
          <p>No repository open</p>
          <span>Choose a Git repository to begin a reviewed task.</span>
          <button
            className="secondary-action"
            type="button"
            onClick={onChooseAndOpen}
          >
            Open repository
          </button>
        </div>
      )}

      <div className="rail-section">
        <span className="eyebrow">Recent sessions</span>
        <p className="quiet-copy">Sessions will stay on this device.</p>
      </div>
    </nav>
  );
}
