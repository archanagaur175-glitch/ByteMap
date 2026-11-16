// Category legend (brief colors) + tier chips; clicking toggles a filter.
// Hidden entries are dimmed. Colors are never the only signal — every
// legend entry carries its label text (PROJECT_BRIEF accessibility rule).

import { CATEGORY_COLORS, CATEGORY_NAMES, TIER_COLORS, TIER_NAMES } from "./colors";
import { useStore } from "./store";

export function Legend() {
  const hiddenCats = useStore((s) => s.hiddenCats);
  const hiddenTiers = useStore((s) => s.hiddenTiers);
  const toggleCat = useStore((s) => s.toggleCat);
  const toggleTier = useStore((s) => s.toggleTier);

  return (
    <div className="legend">
      <div className="legend-row">
        {Object.keys(CATEGORY_NAMES).map(Number).map((id) => {
          const hidden = hiddenCats.includes(id);
          return (
            <button
              key={id}
              className={"legend-chip" + (hidden ? " hidden" : "")}
              onClick={() => toggleCat(id)}
              title={hidden ? "click to show" : "click to filter out"}
            >
              <span
                className="swatch"
                style={{ background: CATEGORY_COLORS[id] }}
                aria-hidden="true"
              />
              {CATEGORY_NAMES[id]}
            </button>
          );
        })}
      </div>
      <div className="legend-row">
        {Object.keys(TIER_NAMES).map(Number).map((id) => {
          const hidden = hiddenTiers.includes(id);
          return (
            <button
              key={id}
              className={"legend-chip tier" + (hidden ? " hidden" : "")}
              onClick={() => toggleTier(id)}
              title={hidden ? "click to show" : "click to filter out"}
            >
              <span
                className="swatch"
                style={{ background: TIER_COLORS[id] }}
                aria-hidden="true"
              />
              {TIER_NAMES[id]}
            </button>
          );
        })}
      </div>
    </div>
  );
}
