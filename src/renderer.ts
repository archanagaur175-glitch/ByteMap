// Renderer interface for the visual views (PROJECT_BRIEF: Canvas 2D behind
// a Renderer interface; WebGL2 only if profiling proves it necessary).
// Canvas2DRenderer draws the sunburst and exposes its geometry so the
// component can hit-test clicks with the same numbers.

import { formatBytes } from "./parseView";
import { CATEGORY_COLORS } from "./colors";
import type { Orbit } from "./orbit";

export interface OrbitScene {
  orbit: Orbit;
  hoveredId: number | null;
  selectedId: number | null;
  /** Drill-in animation: 1 = rest, <1 = scaled down around the center. */
  zoom?: number;
  /** Drill-in animation: 1 = opaque, 0 = invisible. */
  alpha?: number;
  /** Category ids filtered out (drawn dimmed). */
  hiddenCats?: number[];
  /** Tier ids filtered out (drawn dimmed). */
  hiddenTiers?: number[];
}

export interface OrbitRenderer {
  resize(width: number, height: number, dpr: number): void;
  render(scene: OrbitScene): void;
  /** Current canvas geometry for hit-testing (cx, cy, innerR, ringWidth). */
  geometry(): { cx: number; cy: number; innerR: number; ringWidth: number };
}

const BG = "#0b0f1a";
const TEXT = "#e8ecf6";

export class Canvas2DRenderer implements OrbitRenderer {
  private ctx: CanvasRenderingContext2D;
  private w = 0;
  private h = 0;
  private geo = { cx: 0, cy: 0, innerR: 0, ringWidth: 0 };

  constructor(canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no 2d context");
    this.ctx = ctx;
  }

  resize(width: number, height: number, dpr: number): void {
    this.w = width;
    this.h = height;
    const canvas = this.ctx.canvas;
    canvas.width = Math.max(1, Math.round(width * dpr));
    canvas.height = Math.max(1, Math.round(height * dpr));
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  geometry() {
    return { ...this.geo };
  }

  render(scene: OrbitScene): void {
    const { ctx, w, h } = this;
    ctx.fillStyle = BG;
    ctx.fillRect(0, 0, w, h);

    const orbit = scene.orbit;
    const cx = w / 2;
    const cy = h / 2;
    const outer = Math.min(w, h) / 2 - 8;
    const innerR = outer * 0.22;
    const ringWidth = (outer - innerR) / Math.max(1, orbit.rings);
    this.geo = { cx, cy, innerR, ringWidth };

    const zoom = scene.zoom ?? 1;
    const alpha = scene.alpha ?? 1;
    ctx.save();
    ctx.globalAlpha = alpha;
    if (zoom !== 1) {
      ctx.translate(cx, cy);
      ctx.scale(zoom, zoom);
      ctx.translate(-cx, -cy);
    }

    // Arcs, outermost painted first so inner rings overlap cleanly.
    for (let depth = orbit.rings; depth >= 1; depth--) {
      for (const arc of orbit.arcs) {
        if (arc.depth !== depth) continue;
        this.drawArc(scene, arc, cx, cy, innerR, ringWidth);
      }
    }

    // Center disc: the current folder.
    ctx.beginPath();
    ctx.arc(cx, cy, innerR - 2, 0, Math.PI * 2);
    ctx.fillStyle = "#131a2b";
    ctx.fill();
    ctx.strokeStyle = "#2a3350";
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.fillStyle = TEXT;
    ctx.font = '600 13px "Segoe UI Variable Text", "Segoe UI", sans-serif';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(formatBytes(orbit.centerLogical), cx, cy - 7);
    ctx.fillStyle = "#8892a6";
    ctx.font = '11px "Segoe UI Variable Text", "Segoe UI", sans-serif';
    ctx.fillText("on disk " + formatBytes(orbit.centerOnDisk), cx, cy + 9);
    ctx.restore();
  }

  private drawArc(
    scene: OrbitScene,
    arc: import("./orbit").OrbitArc,
    cx: number,
    cy: number,
    innerR: number,
    ringWidth: number,
  ): void {
    const { ctx } = this;
    const a0 = arc.start * Math.PI * 2;
    const a1 = (arc.start + arc.sweep) * Math.PI * 2;
    const rOut = innerR + arc.depth * ringWidth;
    const rIn = innerR + (arc.depth - 1) * ringWidth + 1;

    ctx.beginPath();
    ctx.arc(cx, cy, rOut, a0, a1);
    ctx.arc(cx, cy, rIn, a1, a0, true);
    ctx.closePath();

    const hovered = scene.hoveredId === arc.id;
    const selected = scene.selectedId === arc.id;
    const filtered =
      (scene.hiddenCats?.includes(arc.cat) ?? false) ||
      (scene.hiddenTiers?.includes(arc.tier) ?? false);
    // Arc fill = category color (ring depth no longer carries color).
    let fill = CATEGORY_COLORS[arc.cat] ?? "#8892A6";
    if (filtered) fill = mix(fill, BG, 0.75);
    if (hovered && !filtered) fill = mix(fill, "#ffffff", 0.35);
    ctx.fillStyle = fill;
    ctx.fill();
    // 1px separation between wedges.
    ctx.strokeStyle = BG;
    ctx.lineWidth = 1;
    ctx.stroke();
    if (selected) {
      ctx.strokeStyle = TEXT;
      ctx.lineWidth = 2;
      ctx.stroke();
    }
  }
}

/** Mix two #rrggbb colors (used for the hover highlight). */
export function mix(a: string, b: string, t: number): string {
  const pa = parseInt(a.slice(1), 16);
  const pb = parseInt(b.slice(1), 16);
  const r = Math.round(((pa >> 16) & 255) * (1 - t) + ((pb >> 16) & 255) * t);
  const g = Math.round(((pa >> 8) & 255) * (1 - t) + ((pb >> 8) & 255) * t);
  const bl = Math.round((pa & 255) * (1 - t) + (pb & 255) * t);
  return `#${((r << 16) | (g << 8) | bl).toString(16).padStart(6, "0")}`;
}
