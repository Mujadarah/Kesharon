export function ReviewPanel() {
  return (
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
          Approved edits, test results, and inline review comments will appear
          here.
        </p>
      </div>
    </section>
  );
}
