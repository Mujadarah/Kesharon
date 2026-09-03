import type { DesktopBridge } from "./bridge";
import { ConnectionBadge } from "./components/ConnectionBadge";
import { ProjectRail } from "./components/ProjectRail";
import { ResourcePanel } from "./components/ResourcePanel";
import { ReviewPanel } from "./components/ReviewPanel";
import { TaskComposer } from "./components/TaskComposer";
import { WorkflowRail } from "./components/WorkflowRail";
import { useWorkspace } from "./useWorkspace";

import "./app.css";

export type { DesktopBridge } from "./bridge";

export function App({ bridge }: { bridge: DesktopBridge }) {
  const { state, chooseAndOpen, cancelOpening, setTaskGoal, clearError } =
    useWorkspace(bridge);

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
        <ConnectionBadge state={state.connection} />
      </header>

      <main className="workspace">
        <ProjectRail
          project={state.project}
          openingOperation={state.openingOperation}
          openingError={state.openingError}
          onChooseAndOpen={chooseAndOpen}
          onCancelOpening={cancelOpening}
          onClearError={clearError}
        />

        <section className="plan-panel panel" aria-label="Plan and assistant">
          <TaskComposer
            taskGoal={state.taskGoal}
            hasProject={state.project !== null}
            onGoalChange={setTaskGoal}
          />
          <WorkflowRail />
        </section>

        <ReviewPanel />
        <ResourcePanel />
      </main>
    </div>
  );
}
