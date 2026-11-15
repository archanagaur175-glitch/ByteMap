// Inspector: details of the hovered (or last clicked) orbit arc.
// Safety tiers/categories arrive in Phase 4; flags are shown as text
// badges, never color-only (PROJECT_BRIEF accessibility rule).

import type { OrbitArc } from "./orbit";
import {
  formatBytes,
  FLAG_DIR,
  FLAG_REPARSE,
  FLAG_PLACEHOLDER,
  FLAG_DUPLICATE_HARDLINK,
} from "./parseView";

export function Inspector(props: { arc: OrbitArc | null }) {
  const arc = props.arc;
  return (
    <aside className="inspector">
      {arc === null ? (
        <div className="inspector-hint">Hover an arc for details</div>
      ) : (
        <>
          <div className="inspector-name" title={arc.name}>
            {arc.name}
          </div>
          <div className="inspector-row">
            <span>logical</span>
            <span className="mono">{formatBytes(arc.logical)}</span>
          </div>
          <div className="inspector-row">
            <span>on disk</span>
            <span className="mono">{formatBytes(arc.onDisk)}</span>
          </div>
          <div className="inspector-row">
            <span>ring</span>
            <span className="mono">{arc.depth}</span>
          </div>
          <div className="inspector-badges">
            {arc.flags & FLAG_DIR ? <span className="badge">directory</span> : null}
            {arc.flags & FLAG_REPARSE ? <span className="badge warn">junction/link</span> : null}
            {arc.flags & FLAG_PLACEHOLDER ? <span className="badge">cloud placeholder</span> : null}
            {arc.flags & FLAG_DUPLICATE_HARDLINK ? (
              <span className="badge warn">duplicate hardlink</span>
            ) : null}
            {!(arc.flags & (FLAG_DIR | FLAG_REPARSE | FLAG_PLACEHOLDER | FLAG_DUPLICATE_HARDLINK)) ? (
              <span className="badge">file</span>
            ) : null}
          </div>
        </>
      )}
    </aside>
  );
}
