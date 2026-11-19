// Renderer interface for the visual views (PROJECT_BRIEF: Canvas 2D behind
// a Renderer interface; WebGL2 only if profiling proves it necessary).
// Canvas2DRenderer draws the sunburst and exposes its geometry so the
// component can hit-test clicks with the same numbers.

import { formatBytes } from "./parseView";
import { CATEGORY_COLORS, canvasColors, labelInk, refreshColors } from "./colors";
import type { Orbit } from "./orbit";
import { CANVAS } from "./strata";
import type { Strata } from "./strata";

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

export class Canvas2DRenderer implements OrbitRenderer {
  private ctx: CanvasRenderingContext2D;
  private w = 0;
  private h = 0;
  private geo = { cx: 0, cy: 0, innerR: 0, ringWidth: 0 };
  /** Theme tokens for the frame being painted (read once per render). */
  private ink = canvasColors();

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
    // Follow the active theme (dark/light) without restarting the renderer.
    refreshColors();
    this.ink = canvasColors();
    const { bg, text, panel, muted, border } = this.ink;
    ctx.fillStyle = bg;
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
    ctx.fillStyle = panel;
    ctx.fill();
    ctx.strokeStyle = border;
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.fillStyle = text;
    ctx.font = '600 13px "Segoe UI Variable Text", "Segoe UI", sans-serif';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(formatBytes(orbit.centerLogical), cx, cy - 7);
    ctx.fillStyle = muted;
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
    const { bg, text } = this.ink;
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
    let fill = CATEGORY_COLORS[arc.cat] ?? "#8892a6";
    if (filtered) fill = mix(fill, bg, 0.75);
    if (hovered && !filtered) fill = mix(fill, "#ffffff", 0.35);
    ctx.fillStyle = fill;
    ctx.fill();
    // 1px separation between wedges.
    ctx.strokeStyle = bg;
    ctx.lineWidth = 1;
    ctx.stroke();
    if (selected) {
      ctx.strokeStyle = text;
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

// ---- Strata (layered treemap) renderer ----

export interface StrataScene {
  strata: Strata;
  hoveredId: number | null;
  selectedId: number | null;
  /** Category ids filtered out (drawn dimmed). */
  hiddenCats?: number[];
  /** Tier ids filtered out (drawn dimmed). */
  hiddenTiers?: number[];
  /** Drill-in fade: 1 = opaque, 0 = invisible. */
  alpha?: number;
}

/**
 * Canvas 2D treemap. Tiles arrive sorted parent-first and nested; painting
 * depth by depth with a 1-unit gap leaves the parent color visible in the
 * gaps left by tiles too small to draw, so no area ever looks unassigned.
 */
export class Canvas2DStrataRenderer {
  private ctx: CanvasRenderingContext2D;
  private w = 0;
  private h = 0;

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

  render(scene: StrataScene): void {
    const { ctx, w, h } = this;
    // Follow the active theme (dark/light) without restarting the renderer.
    refreshColors();
    const { bg, text } = canvasColors();
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);
    if (w <= 0 || h <= 0) return;

    const sx = w / CANVAS;
    const sy = h / CANVAS;
    const alpha = scene.alpha ?? 1;
    ctx.save();
    ctx.globalAlpha = alpha;

    // Depth by depth: parents first, so nested tiles sit on top.
    for (let depth = 1; depth <= scene.strata.maxDepth; depth++) {
      for (const rect of scene.strata.rects) {
        if (rect.depth !== depth || rect.w === 0 || rect.h === 0) continue;
        const x = rect.x * sx;
        const y = rect.y * sy;
        const tw = rect.w * sx;
        const th = rect.h * sy;

        const filtered =
          (scene.hiddenCats?.includes(rect.cat) ?? false) ||
          (scene.hiddenTiers?.includes(rect.tier) ?? false);
        let fill = CATEGORY_COLORS[rect.cat] ?? "#8892a6";
        // Each nested layer sits a little darker: the strata read as depth.
        if (depth > 1) fill = mix(fill, bg, 0.12 * (depth - 1));
        if (filtered) fill = mix(fill, bg, 0.75);
        const hovered = scene.hoveredId === rect.id;
        if (hovered && !filtered) fill = mix(fill, "#ffffff", 0.32);

        ctx.fillStyle = fill;
        ctx.fillRect(x, y, tw, th);

        // 1px separation from the neighbour (depth 1 keeps the outer edge).
        if (depth > 1) {
          ctx.strokeStyle = mix(bg, "#ffffff", 0.06);
          ctx.lineWidth = 1;
          ctx.strokeRect(x + 0.5, y + 0.5, Math.max(0, tw - 1), Math.max(0, th - 1));
        }
        if (scene.selectedId === rect.id) {
          ctx.strokeStyle = text;
          ctx.lineWidth = 2;
          ctx.strokeRect(x + 1, y + 1, Math.max(0, tw - 2), Math.max(0, th - 2));
        }

        // Labels only where they fit: a 60x20px tile reads, a sliver does not.
        if (tw >= 64 && th >= 20) {
          ctx.save();
          ctx.beginPath();
          ctx.rect(x, y, tw, th);
          ctx.clip();
          // Ink follows the tile's lightness, so it stays readable in both themes.
          ctx.fillStyle = labelInk(fill, 0.86);
          ctx.font = '600 12px "Segoe UI Variable Text", "Segoe UI", sans-serif';
          ctx.textAlign = "left";
          ctx.textBaseline = "top";
          ctx.fillText(truncate(ctx, rect.name, tw - 10), x + 5, y + 4);
          if (th >= 38) {
            ctx.fillStyle = labelInk(fill, 0.6);
            ctx.font = '11px "Cascadia Mono", Consolas, monospace';
            ctx.fillText(formatBytes(rect.logical), x + 5, y + 20);
          }
          ctx.restore();
        }
      }
    }
    ctx.restore();
  }
}

/** Trim `text` with an ellipsis until it fits `maxWidth` pixels. */
function truncate(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (maxWidth <= 0) return "";
  if (ctx.measureText(text).width <= maxWidth) return text;
  let end = text.length;
  while (end > 1) {
    end -= 1;
    const candidate = text.slice(0, end) + "…";
    if (ctx.measureText(candidate).width <= maxWidth) return candidate;
  }
  return "…";
}
