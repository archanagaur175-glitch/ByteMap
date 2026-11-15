// Full-window orbit perf bench: open the app with #orbit-bench in the URL.
// Animates a rotating synthetic 4000-arc sunburst for ~6 s, shows live fps,
// then prints one result line to the console and on screen.

import { useEffect, useRef, useState } from "react";
import { Canvas2DRenderer } from "./renderer";
import { runOrbitBench } from "./perf";
import type { BenchResult } from "./perf";
import { ARC_COUNT, BENCH_SECONDS } from "./benchConfig";

export function OrbitBench() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [fps, setFps] = useState(0);
  const [result, setResult] = useState<BenchResult | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const renderer = new Canvas2DRenderer(canvas);
    const resize = () =>
      renderer.resize(window.innerWidth, window.innerHeight, window.devicePixelRatio || 1);
    resize();
    window.addEventListener("resize", resize);
    const cancel = runOrbitBench(renderer, ARC_COUNT, BENCH_SECONDS, (r) => {
      setResult(r);
      // One grep-able line for the session log.
      console.log(
        `ORBIT-BENCH arcs=${ARC_COUNT} frames=${r.frames} avgMs=${r.avgMs.toFixed(2)} p95Ms=${r.p95Ms.toFixed(2)} avgFps=${r.avgFps.toFixed(1)}`,
      );
    }, setFps);
    return () => {
      cancel();
      window.removeEventListener("resize", resize);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div id="app-shell">
      <div className="bench-hud">
        {result === null ? (
          <>
            ORBIT-BENCH · {ARC_COUNT} arcs · measuring {BENCH_SECONDS}s · live fps:{" "}
            <span className="mono">{fps.toFixed(0)}</span>
          </>
        ) : (
          <>
            ORBIT-BENCH DONE · {ARC_COUNT} arcs · {result.frames} frames · avg{" "}
            <span className="mono">{result.avgMs.toFixed(2)} ms</span> · p95{" "}
            <span className="mono">{result.p95Ms.toFixed(2)} ms</span> · avg fps{" "}
            <span className="mono">{result.avgFps.toFixed(1)}</span> (same line in the devtools
            console)
          </>
        )}
      </div>
      <canvas ref={canvasRef} className="orbit-canvas" />
    </div>
  );
}
