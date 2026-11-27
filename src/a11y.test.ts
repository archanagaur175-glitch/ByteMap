// P9.T4 accessibility gate. Two halves:
//  1. WCAG contrast computed from the real token values in styles.css.
//  2. A source check that the keyboard/screen-reader wiring the audit added
//     is still present, so a later refactor cannot silently remove it.
//
// Thresholds: 7:1 for body text (AAA, matches the brief's high-contrast
// intent), 4.5:1 for secondary/status text (AA normal text), 4.5:1 for link
// and accent text, and 3:1 for tier graphics (WCAG 1.4.11 non-text). Tier
// colors always sit beside their text label (Legend, tier pills), so they are
// never the only signal; the 3:1 floor is still enforced against the panel.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const lightAt = css.indexOf(':root[data-theme="light"]');
const darkCss = lightAt === -1 ? "" : css.slice(0, lightAt);
const lightCss = lightAt === -1 ? "" : css.slice(lightAt);

function valueOf(block: string, name: string): string | null {
  const all = [...block.matchAll(new RegExp(`${name}\\s*:\\s*([^;]+);`, "g"))];
  return all.length === 0 ? null : all[all.length - 1][1].trim().toLowerCase();
}

function rgb(hex: string): [number, number, number] {
  const m = /^#([0-9a-f]{6})$/.exec(hex.trim().toLowerCase());
  if (!m) throw new Error(`not a 6-digit hex color: ${hex}`);
  const n = Number.parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/** WCAG relative luminance. */
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio (1..21). */
function contrast(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

const themes = [
  { name: "dark", block: darkCss },
  { name: "light", block: lightCss },
];

describe("WCAG contrast from the theme tokens", () => {
  for (const { name, block } of themes) {
    const token = (n: string): string => {
      const value = valueOf(block, n);
      expect(value, `${name} ${n} token`).not.toBeNull();
      return value as string;
    };

    it(`${name}: body text reaches AAA on both surfaces`, () => {
      expect(contrast(token("--text"), token("--bg"))).toBeGreaterThanOrEqual(7);
      expect(contrast(token("--text"), token("--panel"))).toBeGreaterThanOrEqual(7);
      expect(contrast(token("--text"), token("--selected"))).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token("--text"), token("--hover"))).toBeGreaterThanOrEqual(4.5);
    });

    it(`${name}: secondary text reaches AA on both surfaces`, () => {
      expect(contrast(token("--muted"), token("--bg"))).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token("--muted"), token("--panel"))).toBeGreaterThanOrEqual(4.5);
    });

    it(`${name}: links, accents and their ink reach AA`, () => {
      expect(contrast(token("--link"), token("--bg"))).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token("--link"), token("--panel"))).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token("--accent"), token("--panel"))).toBeGreaterThanOrEqual(4.5);
      expect(contrast(token("--on-accent"), token("--accent"))).toBeGreaterThanOrEqual(4.5);
    });

    it(`${name}: status text reaches AA`, () => {
      for (const status of ["--ok", "--warn", "--danger-strong"]) {
        expect(contrast(token(status), token("--bg")), `${status} on bg`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(token(status), token("--panel")), `${status} on panel`).toBeGreaterThanOrEqual(4.5);
      }
    });

    it(`${name}: tier graphics clear the 3:1 non-text floor`, () => {
      for (const tier of ["--tier-0", "--tier-1", "--tier-2"]) {
        expect(contrast(token(tier), token("--panel")), `${tier} on panel`).toBeGreaterThanOrEqual(3);
      }
    });
  }
});

function source(file: string): string {
  return readFileSync(new URL(file, import.meta.url), "utf8");
}

describe("keyboard and screen-reader wiring", () => {
  it("labels the folder input and exposes live status regions", () => {
    const app = source("./App.tsx");
    expect(app).toContain('aria-label="Folder to scan"');
    expect(app).toContain('role="alert"');
    expect(app).toContain('role="status"');
    expect(app).toContain('aria-live="polite"');
    expect(app).toContain('aria-current="page"');
  });

  it("makes breadcrumbs real buttons and the view switch a keyboard tablist", () => {
    const app = source("./App.tsx");
    expect(app).toContain('className="crumb"');
    expect(app).toContain('onClick={() => void openNode(a.id)}');
    expect(app).toContain('role="tablist"');
    expect(app).toContain('aria-controls="visual-view"');
    expect(app).toContain('"ArrowRight"');
  });

  it("gives the tree a keyboard grid with roving focus", () => {
    const tree = source("./TreeList.tsx");
    expect(tree).toContain('role="grid"');
    expect(tree).toContain("aria-rowcount={items.length}");
    expect(tree).toContain("aria-rowindex={index + 1}");
    expect(tree).toContain('role="gridcell"');
    expect(tree).toContain('"ArrowDown"');
    expect(tree).toContain('"ArrowUp"');
    expect(tree).toContain('"Home"');
    expect(tree).toContain('"End"');
    expect(tree).toContain("tabIndex={");
  });

  it("names both canvas views and points at the keyboard list", () => {
    for (const view of ["./OrbitView.tsx", "./StrataView.tsx"]) {
      const text = source(view);
      expect(text, view).toContain('role="img"');
      expect(text, view).toContain("aria-label={`");
      expect(text, view).toContain("keyboard access");
      expect(text, view).toContain('role="tabpanel"');
    }
  });

  it("gives the permanent-delete dialog a label, Esc and a focus trap", () => {
    const hub = source("./ActionHub.tsx");
    expect(hub).toContain('aria-labelledby="permanent-title"');
    expect(hub).toContain('aria-describedby="permanent-desc"');
    expect(hub).toContain('"Escape"');
    expect(hub).toContain('"Tab"');
    expect(hub).toContain("permanentBtnRef.current?.focus()");
  });

  it("announces snapshot and basket results", () => {
    expect(source("./Snapshots.tsx")).toContain('role="status"');
    expect(source("./ActionHub.tsx")).toContain('role="status"');
    expect(source("./Legend.tsx")).toContain("aria-pressed");
  });

  it("gives every interactive class a visible focus ring", () => {
    for (const selector of [
      ".btn:focus-visible",
      ".legend-chip:focus-visible",
      ".row-add:focus-visible",
      ".crumb:focus-visible",
      ".compare-select:focus-visible",
      ".folder-input:focus-visible",
      ".tree-row:focus-visible",
    ]) {
      expect(css, selector).toContain(selector);
    }
  });
});
