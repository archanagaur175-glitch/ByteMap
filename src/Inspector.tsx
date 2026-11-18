// Inspector: details of the hovered (or last clicked) arc or tile.
// Safety tiers/categories arrive in Phase 4; flags are shown as text
// badges, never color-only (PROJECT_BRIEF accessibility rule).

import {
  formatBytes,
  FLAG_DIR,
  FLAG_REPARSE,
  FLAG_PLACEHOLDER,
  FLAG_DUPLICATE_HARDLINK,
} from "./parseView";
import { CATEGORY_NAMES, TIER_COLORS, TIER_NAMES } from "./colors";

/** What the inspector needs; both OrbitArc and StrataRect satisfy it. */
export interface Inspectable {
  name: string;
  logical: number;
  onDisk: number;
  flags: number;
  cat: number;
  tier: number;
  /** Ring (orbit) or layer (strata). */
  depth: number;
}

export function Inspector(props: { arc: Inspectable | null }) {
  const arc = props.arc;
  return (
    <aside className="inspector">
      {arc === null ? (
        <div className="inspector-hint">Hover an arc or tile for details</div>
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
            <span>category</span>
            <span className="mono">{CATEGORY_NAMES[arc.cat] ?? "Other"}</span>
          </div>
          <div className="inspector-row">
            <span>safety tier</span>
            <span className="tier-pill" style={{ borderColor: TIER_COLORS[arc.tier] }}>
              <span
                className="swatch"
                style={{ background: TIER_COLORS[arc.tier] }}
                aria-hidden="true"
              />
              {TIER_NAMES[arc.tier] ?? "Review"}
            </span>
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
