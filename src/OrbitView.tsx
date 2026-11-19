// Orbit (sunburst) canvas. The renderer draws; hitTest maps clicks and
// hovers back to node IDs. Opening a directory re-centers the view.

import { useEffect, useRef, useState } from "react";
import { Canvas2DRenderer } from "./renderer";
import { hitTest } from "./orbit";
import type { OrbitGeometry, OrbitArc } from "./orbit";
import { useStore } from "./store";
import { FLAG_DIR } from "./orbit";
import { Inspector } from "./Inspector";

export function OrbitView() {
  const orbit = useStore((s) => s.orbit);
  const selectedId = useStore((s) => s.selectedId);
  const theme = useStore((s) => s.theme);
  const openNode = useStore((s) => s.openNode);
  const select = useStore((s) => s.select);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);
  const animRef = useRef<number | null>(null);
  const [hoveredId, setHoveredId] = useState<number | null>(null);

  const draw = () => {
    const renderer = rendererRef.current;
    const cur = useStore.getState();
    if (!renderer || !cur.orbit) return;
    renderer.render({
      orbit: cur.orbit,
      hoveredId,
      selectedId: cur.selectedId,
      hiddenCats: cur.hiddenCats,
      hiddenTiers: cur.hiddenTiers,
    });
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const renderer = new Canvas2DRenderer(canvas);
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

  // Plain redraw on hover/selection/theme changes (declared before the drill
  // effect so a same-commit orbit change ends up rendered by the drill's
  // synchronous first frame, not by this one).
  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hoveredId, selectedId, theme]);

  // Drill motion: 320ms zoom-in whenever the orbit is re-centered
  // (PROJECT_BRIEF), skipped entirely under prefers-reduced-motion.
  useEffect(() => {
    if (!orbit) return;
    const reduce =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce) {
      draw();
      return;
    }
    const renderer = rendererRef.current;
    if (!renderer) return;
    const start = performance.now();
    const dur = 320;
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / dur);
      const e = 1 - Math.pow(1 - t, 3); // ease-out cubic
      const cur = useStore.getState();
      if (cur.orbit) {
        renderer.render({
          orbit: cur.orbit,
          hoveredId: null,
          selectedId: cur.selectedId,
          hiddenCats: cur.hiddenCats,
          hiddenTiers: cur.hiddenTiers,
          zoom: 0.85 + 0.15 * e,
          alpha: e,
        });
      }
      if (t < 1) {
        animRef.current = requestAnimationFrame(tick);
      } else {
        animRef.current = null;
      }
    };
    renderer.render({
      orbit,
      hoveredId: null,
      selectedId: useStore.getState().selectedId,
      zoom: 0.85,
      alpha: 0,
    });
    animRef.current = requestAnimationFrame(tick);
    return () => {
      if (animRef.current !== null) {
        cancelAnimationFrame(animRef.current);
        animRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orbit]);

  if (!orbit) return null;

  const pointArc = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    const renderer = rendererRef.current;
    if (!canvas || !renderer) return null;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const g = renderer.geometry();
    const geom: OrbitGeometry = { ...g, rings: orbit.rings };
    return hitTest(orbit, geom, x, y);
  };

  const hoveredArc: OrbitArc | null = hoveredId === null ? null : (orbit.arcs.find((a) => a.id === hoveredId) ?? null);

  return (
    <div className="orbit-row">
      <div className="orbit-wrap" ref={wrapRef}>
        <canvas
          ref={canvasRef}
          className="orbit-canvas"
          onMouseMove={(e) => {
            const arc = pointArc(e);
            setHoveredId(arc ? arc.id : null);
            e.currentTarget.style.cursor = arc ? "pointer" : "default";
          }}
          onMouseLeave={() => {
            setHoveredId(null);
          }}
          onClick={(e) => {
            const arc = pointArc(e);
            if (!arc) return;
            if (arc.flags & FLAG_DIR) {
              void openNode(arc.id);
            } else {
              select(arc.id);
            }
          }}
        />
      </div>
      <Inspector arc={hoveredArc} />
    </div>
  );
}
