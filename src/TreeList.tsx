// Virtualized tree list. Renders only the visible window (fixed 28px rows),
// so a 100k-entry view scrolls without jank. Items come from the Rust view
// (top-N + folded summary), identified by node id.

import { useEffect, useRef, useState } from "react";
import { formatBytes } from "./parseView";
import { CATEGORY_COLORS } from "./colors";
import { useStore } from "./store";
import type { ViewItem } from "./parseView";

const ROW_HEIGHT = 28;
const OVERSCAN = 6;

export function TreeList(props: {
  items: ViewItem[];
  childDepth: number;
  selectedId: number | null;
  onOpen: (id: number) => void;
}) {
  const { items, childDepth, selectedId, onOpen } = props;
  const hiddenCats = useStore((s) => s.hiddenCats);
  const containerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(600);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onScroll = () => setScrollTop(el.scrollTop);
    const onResize = () => setHeight(el.clientHeight);
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("resize", onResize);
    setHeight(el.clientHeight);
    return () => {
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("resize", onResize);
    };
  }, []);

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(items.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
  const window = items.slice(first, last);
  const totalHeight = items.length * ROW_HEIGHT;

  return (
    <div className="tree-list" ref={containerRef}>
      <div style={{ height: totalHeight, position: "relative" }}>
        {window.map((item, i) => {
          const index = first + i;
          const folded = item.isFolded;
          const filtered = hiddenCats.includes(item.cat);
          return (
            <div
              key={folded ? `folded-${index}` : item.id}
              className={
                "tree-row" + (selectedId === item.id ? " selected" : "") + (folded ? " folded" : "") + (filtered ? " filtered" : "")
              }
              style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
              onClick={() => {
                if (!folded) onOpen(item.id);
              }}
              role="row"
            >
              <span
                className="tree-indent"
                style={{ width: childDepth * 16 + (folded ? 0 : 18) }}
              />
              <span
                className="cat-dot"
                style={{ background: CATEGORY_COLORS[item.cat] ?? "#8892A6" }}
                aria-hidden="true"
              />
              <span className={folded ? "tier-dir" : "tier-file"}>
                {folded ? "…" : item.flags & 1 ? "" : "▪"}
              </span>
              <span className="tree-name" title={item.name}>
                {item.name}
              </span>
              <span className="tree-size">{formatBytes(item.logical)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
