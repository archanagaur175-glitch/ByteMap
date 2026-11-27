// Virtualized tree list. Renders only the visible window (fixed 28px rows),
// so a 100k-entry view scrolls without jank. Items come from the Rust view
// (top-N + folded summary), identified by node id.
//
// Keyboard: the grid is one tab stop (roving tabIndex). Up/Down/Home/End move
// between real rows, Enter/Space opens the focused folder. Folded summary rows
// are not focus targets. The canvas views stay mouse-only, so this list is the
// keyboard path into the tree.

import { useEffect, useRef, useState } from "react";
import { formatBytes, TIER_PROTECTED } from "./parseView";
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
  const [focusedIndex, setFocusedIndex] = useState(-1);

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

  const firstNavigable = items.findIndex((it) => !it.isFolded);
  const tabbableIndex =
    focusedIndex >= 0 && focusedIndex < items.length && !items[focusedIndex].isFolded
      ? focusedIndex
      : firstNavigable;

  // Bring a keyboard-moved row into view, then focus it. Virtualization means
  // the row may not exist until the scroll update has re-rendered.
  useEffect(() => {
    if (focusedIndex < 0 || focusedIndex >= items.length) return;
    const el = containerRef.current;
    if (!el) return;
    const top = focusedIndex * ROW_HEIGHT;
    const bottom = top + ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight;
    const raf = requestAnimationFrame(() => {
      el.querySelector<HTMLElement>(`[data-row-index="${focusedIndex}"]`)?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [focusedIndex, items.length]);

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(items.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
  const window = items.slice(first, last);
  const totalHeight = items.length * ROW_HEIGHT;

  const moveFocus = (from: number, dir: 1 | -1) => {
    for (let i = from + dir; i >= 0 && i < items.length; i += dir) {
      if (!items[i].isFolded) {
        setFocusedIndex(i);
        return;
      }
    }
  };

  const lastNavigable = (): number => {
    for (let i = items.length - 1; i >= 0; i--) if (!items[i].isFolded) return i;
    return -1;
  };

  const onRowKeyDown = (
    e: React.KeyboardEvent<HTMLDivElement>,
    index: number,
    folded: boolean,
  ) => {
    // A nested button (the add affordance) owns its own keys.
    if (e.target !== e.currentTarget) return;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        moveFocus(index, 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        moveFocus(index, -1);
        break;
      case "Home":
        e.preventDefault();
        if (firstNavigable >= 0) setFocusedIndex(firstNavigable);
        break;
      case "End": {
        e.preventDefault();
        const lastRow = lastNavigable();
        if (lastRow >= 0) setFocusedIndex(lastRow);
        break;
      }
      case "Enter":
      case " ":
        e.preventDefault();
        if (!folded) onOpen(items[index].id);
        break;
    }
  };

  return (
    <div
      className="tree-list"
      ref={containerRef}
      role="grid"
      aria-label="Folder contents"
      aria-rowcount={items.length}
      aria-colcount={3}
    >
      <div style={{ height: totalHeight, position: "relative" }} role="rowgroup">
        {window.map((item, i) => {
          const index = first + i;
          const folded = item.isFolded;
          const filtered = hiddenCats.includes(item.cat);
          return (
            <div
              key={folded ? `folded-${index}` : item.id}
              data-row-index={index}
              className={
                "tree-row" + (selectedId === item.id ? " selected" : "") + (folded ? " folded" : "") + (filtered ? " filtered" : "")
              }
              style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
              role="row"
              aria-rowindex={index + 1}
              aria-selected={!folded && selectedId === item.id}
              aria-disabled={folded || undefined}
              tabIndex={!folded && index === tabbableIndex ? 0 : -1}
              onFocus={(e) => {
                // A nested add button owns its own focus; only a focus landing
                // on the row itself moves the roving tab stop.
                if (!folded && e.target === e.currentTarget) setFocusedIndex(index);
              }}
              onKeyDown={(e) => onRowKeyDown(e, index, folded)}
              onClick={() => {
                if (!folded) onOpen(item.id);
              }}
            >
              <span
                className="tree-indent"
                style={{ width: childDepth * 16 + (folded ? 0 : 18) }}
                aria-hidden="true"
              />
              <span
                className="cat-dot"
                style={{ background: CATEGORY_COLORS[item.cat] ?? "#8892A6" }}
                aria-hidden="true"
              />
              <span className={folded ? "tier-dir" : "tier-file"} aria-hidden="true">
                {folded ? "…" : item.flags & 1 ? "" : "▪"}
              </span>
              <span className="tree-name" role="gridcell" title={item.name}>
                {item.name}
              </span>
              <span className="tree-size" role="gridcell">
                {formatBytes(item.logical)}
              </span>
              <span className="tree-action-cell" role="gridcell">
                {!folded && (item.tier === 0 || item.tier === 1) && !(item.flags & 1) && (
                  <button
                    className="row-add"
                    title={`Add ${item.name} to the Action Hub basket`}
                    aria-label={`add ${item.name} to basket`}
                    onClick={(e) => {
                      e.stopPropagation();
                      void useStore.getState().addToBasket([item.id], false);
                    }}
                  >
                    ＋
                  </button>
                )}
                {!folded && item.tier === TIER_PROTECTED && (
                  <span className="row-add disabled" title="Protected — never deletable" aria-label="protected item">
                    ⃠
                  </span>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
