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

export function ResourcePanel() {
  return (
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
  );
}
