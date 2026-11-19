import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CATEGORY_COLORS, TIER_COLORS, canvasColors, labelInk, refreshColors } from "./colors";
import {
  DEFAULT_THEME,
  THEME_STORAGE_KEY,
  currentTheme,
  isTheme,
  loadTheme,
  saveTheme,
  token,
} from "./theme";

// The token gate: styles.css is the single source of truth for colors, so
// the brief's exact values are asserted here instead of being duplicated in
// TypeScript. This is the T4 "token match" check, both themes.
const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

const lightAt = css.indexOf(':root[data-theme="light"]');
const darkCss = lightAt === -1 ? "" : css.slice(0, lightAt);
const lightCss = lightAt === -1 ? "" : css.slice(lightAt);

/** Value of the LAST `--name: value;` declaration in a CSS slice. */
function valueOf(block: string, name: string): string | null {
  const all = [...block.matchAll(new RegExp(`${name}\\s*:\\s*([^;]+);`, "g"))];
  return all.length === 0 ? null : all[all.length - 1][1].trim().toLowerCase();
}

function expectTokens(
  block: string,
  tokens: Record<string, string>,
  theme: string,
): void {
  for (const [name, expected] of Object.entries(tokens)) {
    expect(valueOf(block, name), `${theme} ${name}`).toBe(expected.toLowerCase());
  }
}

// PROJECT_BRIEF.md line 28, verbatim.
const DARK = {
  "--bg": "#0b0f1a",
  "--text": "#e8ecf6",
  "--accent": "#6ea8ff",
  "--cat-3": "#ff5c8a", // Media
  "--cat-2": "#ffb347", // Archives
  "--cat-0": "#4fd1c5", // Code
  "--cat-1": "#a78bfa", // System
  "--cat-4": "#5b9dff", // Documents
  "--cat-5": "#8bdd6b", // Apps
  "--cat-6": "#8892a6", // Other
  "--tier-0": "#3ddc97", // Safe
  "--tier-1": "#ffc857", // Review
  "--tier-2": "#ff5d5d", // Protected
};

const LIGHT = {
  "--bg": "#f3f5fa",
  "--text": "#121826",
  "--accent": "#2f6fe0",
  "--cat-3": "#e03a6b",
  "--cat-2": "#d98a12",
  "--cat-0": "#1fa89c",
  "--cat-1": "#7c5ce0",
  "--cat-4": "#2f6fe0",
  "--cat-5": "#4fa83a",
  "--cat-6": "#6b7488",
  "--tier-0": "#1e9e68",
  "--tier-1": "#c48a00",
  "--tier-2": "#d13b3b",
};

describe("theme tokens", () => {
  it("has both theme blocks", () => {
    expect(lightAt).toBeGreaterThan(0);
    expect(darkCss).toContain(':root[data-theme="dark"]');
    expect(lightCss).toContain(':root[data-theme="light"]');
  });

  it("matches the brief's dark tokens exactly", () => {
    expectTokens(darkCss, DARK, "dark");
  });

  it("matches the brief's light tokens exactly", () => {
    expectTokens(lightCss, LIGHT, "light");
  });

  it("uses the brief's geometry and motion scale", () => {
    expect(valueOf(css.slice(0, lightAt === -1 ? css.length : lightAt), "--radius")).toBe("12px");
    expect(valueOf(css.slice(0, lightAt === -1 ? css.length : lightAt), "--glass-blur")).toBe("24px");
    expect(valueOf(css.slice(0, lightAt === -1 ? css.length : lightAt), "--motion")).toBe("180ms");
    // Drill motion stays 320ms in JS; the standard motion is 180ms.
    expect(css).toContain("prefers-reduced-motion: reduce");
  });

  it("gives every category and tier a token in both themes", () => {
    for (const block of [darkCss, lightCss]) {
      for (let cat = 0; cat <= 6; cat++) expect(valueOf(block, `--cat-${cat}`)).not.toBeNull();
      for (let tier = 0; tier <= 2; tier++) expect(valueOf(block, `--tier-${tier}`)).not.toBeNull();
    }
  });
});

describe("theme selection", () => {
  it("accepts only dark and light", () => {
    expect(isTheme("dark")).toBe(true);
    expect(isTheme("light")).toBe(true);
    expect(isTheme("Dark")).toBe(false);
    expect(isTheme("high-contrast")).toBe(false);
    expect(isTheme(null)).toBe(false);
    expect(isTheme(undefined)).toBe(false);
  });

  it("defaults to dark when storage is empty, corrupt or unavailable", () => {
    expect(DEFAULT_THEME).toBe("dark");
    expect(loadTheme(null)).toBe("dark");
    expect(loadTheme({ getItem: () => null })).toBe("dark");
    expect(loadTheme({ getItem: () => "neon" })).toBe("dark");
    expect(
      loadTheme({
        getItem: () => {
          throw new Error("storage disabled");
        },
      }),
    ).toBe("dark");
  });

  it("reads a stored choice back", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
    };
    expect(loadTheme(storage)).toBe("dark");
    saveTheme("light", storage);
    expect(store.get(THEME_STORAGE_KEY)).toBe("light");
    expect(loadTheme(storage)).toBe("light");
  });

  it("survives a failing write and reports the default outside a document", () => {
    expect(() =>
      saveTheme("light", {
        setItem: () => {
          throw new Error("quota");
        },
      }),
    ).not.toThrow();
    // No document in the node test environment.
    expect(currentTheme()).toBe("dark");
    expect(token("--bg")).toBe("");
  });
});

describe("canvas palette", () => {
  it("keeps readable dark-token fallbacks outside a document", () => {
    expect(canvasColors()).toEqual({
      bg: "#0b0f1a",
      text: "#e8ecf6",
      panel: "#131a2b",
      muted: "#8892a6",
      border: "#2a3350",
    });
    // refreshColors() with no document must not wipe the cache.
    refreshColors();
    expect(CATEGORY_COLORS[3]).toBe("#ff5c8a");
    expect(TIER_COLORS[2]).toBe("#ff5d5d");
  });

  it("picks label ink by tile lightness", () => {
    // Light tiles get dark ink, dark tiles get light ink.
    expect(labelInk("#ffffff")).toMatch(/^rgba\(11, 15, 26/);
    expect(labelInk("#f3f5fa")).toMatch(/^rgba\(11, 15, 26/);
    expect(labelInk("#0b0f1a")).toMatch(/^rgba\(255, 255, 255/);
    expect(labelInk("#2f6fe0")).toMatch(/^rgba\(255, 255, 255/);
    // 3-digit hex and alpha round-trip.
    expect(labelInk("#fff", 0.5)).toBe("rgba(11, 15, 26, 0.5)");
  });
});
