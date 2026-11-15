// Orbit (sunburst) canvas. The renderer draws; hitTest maps clicks and
// hovers back to node IDs. Opening a directory re-centers the view.

import { useEffect, useRef, useState } from "react";
import { Canvas2DRenderer } from "./renderer";
import { hitTest } from "./orbit";
import type { OrbitGeometry } from "./orbit";
import { useStore } from "./store";
import { FLAG_DIR } from "./orbit";

export function OrbitView() {
  const orbit = useStore((s) => s.orbit);
  const selectedId = useStore((s) => s.selectedId);
  const openNode = useStore((s) => s.openNode);
  const select = useStore((s) => s.select);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);
  const [hoveredId, setHoveredId] = useState<number | null>(null);

  const draw = () => {
    const renderer = rendererRef.current;
    const cur = useStore.getState();
    if (!renderer || !cur.orbit) return;
    renderer.render({
      orbit: cur.orbit,
      hoveredId,
      selectedId: cur.selectedId,
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

  // Redraw when the orbit, selection, or hover changes.
  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orbit, selectedId, hoveredId]);

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

  return (
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
  );
}
