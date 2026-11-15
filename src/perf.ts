// Synthetic orbit generator + frame-time measurement for the P3.T5 perf
// gate (brief: 60 fps at <= 4000 drawn primitives; measure, never claim).

import type { Orbit, OrbitArc } from "./orbit";
import type { OrbitRenderer } from "./renderer";

/** Deterministic xorshift32 in [0,1). */
function makeRng(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 4294967296;
  };
}

/**
 * Recursive split like bytemap-core::orbit (children in random-weight
 * order, each taking sweep * weight / total), until `rings` deep or
 * `arcCount` arcs exist.
 */
export function syntheticOrbit(arcCount: number, rings: number, seed = 0x9e3779b9): Orbit {
  const rnd = makeRng(seed);
  const arcs: OrbitArc[] = [];
  let nextId = 2;

  const split = (start: number, sweep: number, depth: number) => {
    if (depth > rings || arcs.length >= arcCount) return;
    const n = 3 + Math.floor(rnd() * 4); // 3..6 children
    const weights: number[] = [];
    for (let i = 0; i < n; i++) weights.push(0.2 + rnd());
    const total = weights.reduce((a, b) => a + b, 0);
    let cursor = start;
    for (const w of weights) {
      if (arcs.length >= arcCount) break;
      const s = (sweep * w) / total;
      const id = nextId++;
      arcs.push({
        id,
        depth,
        start: cursor,
        sweep: s,
        logical: Math.round(1e9 * w),
        onDisk: Math.round(1e9 * w),
        flags: depth < rings ? 1 : 0,
        name: `bench-${id}`,
      });
      if (depth < rings && s > 0.004) split(cursor, s, depth + 1);
      cursor += s;
    }
  };

  split(0, 1, 1);
  return {
    center: 1,
    centerLogical: arcs.reduce((a, b) => a + b.logical, 0),
    centerOnDisk: arcs.reduce((a, b) => a + b.logical, 0),
    rings,
    arcs,
  };
}

export interface BenchResult {
  frames: number;
  durationMs: number;
  avgMs: number;
  p95Ms: number;
  avgFps: number;
}

/**
 * Animate a rotating 4000-arc scene with rAF, measuring per-frame wall
 * time, for `seconds`. Calls `done(result)` once when finished.
 * Returns a cancel function.
 */
export function runOrbitBench(
  renderer: OrbitRenderer,
  arcCount: number,
  seconds: number,
  done: (r: BenchResult) => void,
  onFrame?: (fps: number) => void,
): () => void {
  const orbit = syntheticOrbit(arcCount, 4);
  const frameMs: number[] = [];
  let raf = 0;
  let prev = performance.now();
  const start = prev;
  let stopped = false;

  const tick = (now: number) => {
    if (stopped) return;
    frameMs.push(now - prev);
    prev = now;
    const elapsed = (now - start) / 1000;
    const rot = (elapsed / 12) % 1; // one full rotation per 12 s
    const rotated: Orbit = {
      ...orbit,
      arcs: orbit.arcs.map((a) => ({ ...a, start: (a.start + rot) % 1 })),
    };
    renderer.render({ orbit: rotated, hoveredId: null, selectedId: null });
    if (onFrame) {
      const recent = frameMs.slice(-30);
      const avg = recent.reduce((a, b) => a + b, 0) / recent.length;
      onFrame(avg > 0 ? 1000 / avg : 0);
    }
    if (elapsed >= seconds) {
      const durations = frameMs.slice(1); // drop the first warm-up frame
      const sorted = [...durations].sort((a, b) => a - b);
      const p95 = sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))] ?? 0;
      const avg = durations.reduce((a, b) => a + b, 0) / Math.max(1, durations.length);
      done({
        frames: durations.length,
        durationMs: now - start,
        avgMs: avg,
        p95Ms: p95,
        avgFps: avg > 0 ? 1000 / avg : 0,
      });
      return;
    }
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);

  return () => {
    stopped = true;
    cancelAnimationFrame(raf);
  };
}
