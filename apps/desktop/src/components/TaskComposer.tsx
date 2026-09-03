interface TaskComposerProps {
  taskGoal: string;
  hasProject: boolean;
  onGoalChange: (goal: string) => void;
}

export function TaskComposer({
  taskGoal,
  hasProject,
  onGoalChange
}: TaskComposerProps) {
  return (
    <div className="plan-composer-wrapper">
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
