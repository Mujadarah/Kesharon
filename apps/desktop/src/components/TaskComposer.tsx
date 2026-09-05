import type { TaskSnapshot } from "@kesharon/protocol";

interface TaskComposerProps {
  taskGoal: string;
  hasProject: boolean;
  onGoalChange: (goal: string) => void;
  activeTask?: TaskSnapshot | null;
}

export function TaskComposer({
  taskGoal,
  hasProject,
  onGoalChange,
  activeTask
}: TaskComposerProps) {
  const modeLabel = `[Mode: ${activeTask?.executionMode?.toUpperCase() || "PLAN"}]`;

  return (
    <div className="plan-composer-wrapper">
      <div className="plan-intro">
        <div className="plan-intro-meta">
          <span className="eyebrow">New task</span>
          <span className="mode-badge" data-testid="execution-mode-badge">
            {modeLabel}
          </span>
        </div>
        <h1>Plan before execution</h1>
        <p>
          Describe the outcome. Kesharon will inspect the repository and
          prepare a bounded plan before requesting permission.
        </p>
      </div>

      {activeTask && (
        <div className="active-task-info" data-testid="active-task-info">
          <div className="active-task-header">
            <span className="active-task-label">Active Task</span>
            <span
              className="active-task-status"
              data-testid="active-task-status"
            >
              {activeTask.state}
            </span>
          </div>
          <p className="active-task-goal">{activeTask.goal}</p>
        </div>
      )}

      <label className="task-composer">
        <span className="sr-only">Task goal</span>
        <textarea
          rows={4}
          placeholder="What should change in this repository?"
          value={taskGoal}
          onChange={(e) => {
            if (hasProject) {
              onGoalChange(e.target.value);
            } else {
              e.target.value = taskGoal;
            }
          }}
          disabled={!hasProject}
        />
        <span className="composer-footer">
          <span>
            {hasProject
              ? "Planning available · execution disabled in Phase 2"
              : "Open a repository to enable planning"}
          </span>
          <button type="button" disabled>
            Prepare plan
          </button>
        </span>
      </label>
    </div>
  );
}
