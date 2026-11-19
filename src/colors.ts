// Category + tier presentation. Every color comes from the active theme's
// CSS custom properties (styles.css is the single source of truth, see
// PROJECT_BRIEF.md line 28). The maps below are a cache: refreshColors()
// re-reads the tokens after a theme switch, so call sites keep the plain
// [id] lookup (Legend, TreeList, Inspector, ActionHub, renderer).

import { token } from "./theme";

/** Dark-token fallbacks, used before the first refresh or outside a document. */
const DEFAULT_CATEGORY: Record<number, string> = {
  0: "#4fd1c5", // Code
  1: "#a78bfa", // System
  2: "#ffb347", // Archives
  3: "#ff5c8a", // Media
  4: "#5b9dff", // Documents
  5: "#8bdd6b", // Apps
  6: "#8892a6", // Other
};

const DEFAULT_TIER: Record<number, string> = {
  0: "#3ddc97", // Safe
  1: "#ffc857", // Review
  2: "#ff5d5d", // Protected
};

export const CATEGORY_COLORS: Record<number, string> = { ...DEFAULT_CATEGORY };
export const TIER_COLORS: Record<number, string> = { ...DEFAULT_TIER };

export const CATEGORY_NAMES: Record<number, string> = {
  0: "Code",
  1: "System",
  2: "Archives",
  3: "Media",
  4: "Documents",
  5: "Apps",
  6: "Other",
};

export const TIER_NAMES: Record<number, string> = {
  0: "Safe",
  1: "Review",
  2: "Protected",
};

/** Re-read every color token from the active theme. */
export function refreshColors(): void {
  for (let cat = 0; cat <= 6; cat++) {
    const value = token(`--cat-${cat}`);
    if (value) CATEGORY_COLORS[cat] = value;
  }
  for (let tier = 0; tier <= 2; tier++) {
    const value = token(`--tier-${tier}`);
    if (value) TIER_COLORS[tier] = value;
  }
}

/** Surface tokens the canvas renderers paint with. */
export function canvasColors(): {
  bg: string;
  text: string;
  panel: string;
  muted: string;
  border: string;
} {
  return {
    bg: token("--bg") || "#0b0f1a",
    text: token("--text") || "#e8ecf6",
    panel: token("--panel") || "#131a2b",
    muted: token("--muted") || "#8892a6",
    border: token("--border") || "#2a3350",
  };
}

function channels(hex: string): { r: number; g: number; b: number } {
  const clean = hex.trim().replace("#", "");
  const full =
    clean.length === 3
      ? clean
          .split("")
          .map((c) => c + c)
          .join("")
      : clean.padEnd(6, "0");
  const value = Number.parseInt(full.slice(0, 6), 16);
  if (!Number.isFinite(value)) return { r: 11, g: 15, b: 26 };
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255 };
}

/**
 * Readable label ink for a tile filled with `fill`: dark ink on light tiles,
 * light ink on dark ones. Without this a light theme would print near-black
 * labels on the darker light-theme category colors.
 */
export function labelInk(fill: string, alpha = 0.85): string {
  const { r, g, b } = channels(fill);
  const luma = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luma > 0.55
    ? `rgba(11, 15, 26, ${alpha})`
    : `rgba(255, 255, 255, ${alpha})`;
}
