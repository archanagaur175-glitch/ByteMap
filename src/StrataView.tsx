// Strata (layered treemap) canvas. Tiles are nested rectangles from
// bytemap-core; the renderer paints them and hit-testing returns the
// deepest tile under the cursor. Opening a directory re-centers the view.

import { useEffect, useRef, useState } from "react";
import { Canvas2DStrataRenderer } from "./renderer";
import { hitTestStrata } from "./strata";
import type { StrataRect } from "./strata";
import { useStore } from "./store";
import { FLAG_DIR } from "./orbit";
import { DRILL_MS, reducedMotion } from "./theme";
import { Inspector } from "./Inspector";

export function StrataView() {
  const strata = useStore((s) => s.strata);
  const selectedId = useStore((s) => s.selectedId);
  const theme = useStore((s) => s.theme);
  const openNode = useStore((s) => s.openNode);
  const select = useStore((s) => s.select);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Canvas2DStrataRenderer | null>(null);
  const animRef = useRef<number | null>(null);
  const [hoveredId, setHoveredId] = useState<number | null>(null);

  const draw = (alpha = 1) => {
    const renderer = rendererRef.current;
    const cur = useStore.getState();
    if (!renderer || !cur.strata) return;
    renderer.render({
      strata: cur.strata,
      hoveredId,
      selectedId: cur.selectedId,
      hiddenCats: cur.hiddenCats,
      hiddenTiers: cur.hiddenTiers,
      alpha,
    });
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const renderer = new Canvas2DStrataRenderer(canvas);
    rendererRef.current = renderer;
    const ro = new ResizeObserver(() => {
      renderer.resize(wrap.clientWidth, wrap.clientHeight, window.devicePixelRatio || 1);
      draw();
    });
    ro.observe(wrap);
    renderer.resize(wrap.clientWidth, wrap.clientHeight, window.devicePixelRatio || 1);
    draw();
    return () => {
      ro.disconnect();
      rendererRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hoveredId, selectedId, theme]);

  // Drill motion: the same DRILL_MS standard the Orbit view uses, here as a
  // cross-fade (a treemap zoom would slide every tile at once). Skipped
  // entirely under prefers-reduced-motion.
  useEffect(() => {
    if (!strata) return;
    if (reducedMotion()) {
      draw();
      return;
    }
    const start = performance.now();
    const dur = DRILL_MS;
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / dur);
      draw(1 - Math.pow(1 - t, 3)); // ease-out cubic fade-in
      if (t < 1) {
        animRef.current = requestAnimationFrame(tick);
      } else {
        animRef.current = null;
      }
    };
    draw(0);
    animRef.current = requestAnimationFrame(tick);
    return () => {
      if (animRef.current !== null) {
        cancelAnimationFrame(animRef.current);
        animRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [strata]);

  if (!strata) return null;

  const pointTile = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    return hitTestStrata(
      strata,
      e.clientX - rect.left,
      e.clientY - rect.top,
      rect.width,
      rect.height,
    );
  };

  const hoveredTile: StrataRect | null =
    hoveredId === null ? null : (strata.rects.find((r) => r.id === hoveredId) ?? null);

  return (
    <div className="orbit-row">
      <div className="orbit-wrap" ref={wrapRef}>
        <canvas
          ref={canvasRef}
          className="orbit-canvas"
          onMouseMove={(e) => {
            const tile = pointTile(e);
            setHoveredId(tile ? tile.id : null);
            e.currentTarget.style.cursor = tile ? "pointer" : "default";
          }}
          onMouseLeave={() => {
            setHoveredId(null);
          }}
          onClick={(e) => {
            const tile = pointTile(e);
            if (!tile) return;
            if (tile.flags & FLAG_DIR) {
              void openNode(tile.id);
            } else {
              select(tile.id);
            }
          }}
        />
      </div>
      <Inspector arc={hoveredTile} />
    </div>
  );
}
