import type { TaskSnapshot } from "@kesharon/protocol";

interface ResourcePanelProps {
  activeTask?: TaskSnapshot | null;
}

function ResourceMetric({
  label,
  value
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="resource-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

export function ResourcePanel({ activeTask }: ResourcePanelProps = {}) {
  const memoryValue = activeTask
    ? `${Math.round(activeTask.budget.maxMemoryBytes / (1024 * 1024))} MB`
    : "—";

  const diskValue = activeTask
    ? `${Math.round(activeTask.budget.maxDiskWriteBytes / (1024 * 1024))} MB`
    : "—";

  const workersValue = activeTask
    ? `0 / ${activeTask.budget.maxConcurrentTools}`
    : "0 / 2";

  let tokensValue: string | null = null;
  if (
    activeTask?.budget.maxPromptTokens !== undefined ||
    activeTask?.budget.maxCompletionTokens !== undefined
  ) {
    const prompt = activeTask.budget.maxPromptTokens;
    const completion = activeTask.budget.maxCompletionTokens;
    if (prompt !== undefined && completion !== undefined) {
      tokensValue = `${prompt.toLocaleString()} prompt / ${completion.toLocaleString()} completion`;
    } else if (prompt !== undefined) {
      tokensValue = `${prompt.toLocaleString()} prompt`;
    } else if (completion !== undefined) {
      tokensValue = `${completion.toLocaleString()} completion`;
    }
  }

  return (
    <section className="resource-panel" aria-label="Resource monitor">
      <ResourceMetric label="CPU" value="—" />
      <ResourceMetric label="Memory" value={memoryValue} />
      <ResourceMetric label="Disk writes" value={diskValue} />
      {tokensValue && <ResourceMetric label="Tokens" value={tokensValue} />}
      <ResourceMetric label="Cache" value="—" />
      <ResourceMetric label="Workers" value={workersValue} />
      <div className="isolation-state">
        <span className="status-dot status-dot--neutral" />
        Native execution not started
      </div>
    </section>
  );
}

