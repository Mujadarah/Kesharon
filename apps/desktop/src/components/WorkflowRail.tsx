interface WorkflowStepProps {
  label: string;
  detail: string;
  state?: "current" | "waiting";
}

function WorkflowStep({
  label,
  detail,
  state = "waiting"
}: WorkflowStepProps) {
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

export function WorkflowRail() {
  return (
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
  );
}
