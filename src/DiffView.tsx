// Diff view (Phase 8, P8.T2): the top movers between two saved snapshots,
// plus complete counts/totals computed by Rust. The list is ranked by
// absolute size change; directories appear only when the whole subtree was
// added or removed (see bytemap-core::diff).

import { formatBytes } from "./parseView";
import { CATEGORY_COLORS } from "./colors";
import { formatDelta, KIND_ADDED, KIND_GREW, KIND_REMOVED, KIND_SHRANK } from "./diff";
import { useStore } from "./store";

const KIND_LABEL: Record<number, string> = {
  [KIND_ADDED]: "added",
  [KIND_REMOVED]: "removed",
  [KIND_GREW]: "grew",
  [KIND_SHRANK]: "shrank",
};

function SummaryCard(props: { label: string; count: number; bytes: number; className: string }) {
  const { label, count, bytes, className } = props;
  return (
    <div className={`diff-card ${className}`}>
      <span className="diff-card-label">{label}</span>
      <span className="diff-card-count">{count.toLocaleString()}</span>
      <span className="diff-card-bytes">{formatBytes(bytes)}</span>
    </div>
  );
}

export function DiffView() {
  const { diff, diffPair, diffLoading, closeDiff } = useStore();

  const header = (
    <div className="hub-head">
      <span className="hub-title">Changes</span>
      <span className="hub-count">
        {diffPair ? `${diffPair.new} vs ${diffPair.old}` : "comparing snapshots"}
      </span>
      <button className="btn tiny" onClick={closeDiff}>
        Back to snapshots
      </button>
    </div>
  );

  if (diffLoading) {
    return (
      <div className="diff-view">
        {header}
        <div className="snapshot-empty">Comparing snapshots…</div>
      </div>
    );
  }
  if (!diff) return null;

  const s = diff.summary;
  const net = s.net;
  return (
    <div className="diff-view">
      {header}

      <div className="diff-summary">
        <SummaryCard label="Added" count={s.addedCount} bytes={s.addedBytes} className="added" />
        <SummaryCard label="Removed" count={s.removedCount} bytes={s.removedBytes} className="removed" />
        <SummaryCard label="Grew" count={s.grewCount} bytes={s.grewBytes} className="grew" />
        <SummaryCard label="Shrank" count={s.shrankCount} bytes={s.shrankBytes} className="shrank" />
        <div className={`diff-card net ${net > 0 ? "grew" : net < 0 ? "shrank" : ""}`}>
          <span className="diff-card-label">Net</span>
          <span className="diff-card-count">
            {formatDelta(net, formatBytes)}
          </span>
          <span className="diff-card-bytes">
            {formatBytes(s.beforeTotal)} → {formatBytes(s.afterTotal)}
          </span>
        </div>
      </div>

      {diff.entries.length === 0 ? (
        <div className="snapshot-empty">No size changes between these snapshots.</div>
      ) : (
        <>
          <div className="diff-rows">
            {diff.entries.map((entry) => (
              <div className={`diff-row ${entry.delta > 0 ? "grew" : "shrank"}`} key={`${entry.path}-${entry.kind}`}>
                <span
                  className="cat-dot"
                  style={{ background: CATEGORY_COLORS[entry.cat] ?? "#8892A6" }}
                  aria-hidden="true"
                />
                <span className="diff-kind">{KIND_LABEL[entry.kind] ?? "changed"}</span>
                <span className="diff-path" title={entry.path}>
                  {entry.path}
                  {entry.isDir ? <span className="diff-dir">folder</span> : null}
                </span>
                <span className="diff-sizes">
                  {formatBytes(entry.before)} → {formatBytes(entry.after)}
                </span>
                <span className="diff-delta">{formatDelta(entry.delta, formatBytes)}</span>
              </div>
            ))}
          </div>
          {diff.truncated && (
            <div className="snapshot-empty">
              Showing the top {diff.entries.length.toLocaleString()} of{" "}
              {s.totalChanged.toLocaleString()} changes (largest first).
            </div>
          )}
        </>
      )}
    </div>
  );
}
