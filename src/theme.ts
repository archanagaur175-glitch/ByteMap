// Theme selection. Every color lives in src/styles.css as a CSS custom
// property (PROJECT_BRIEF.md line 28), so the DOM and the canvas renderers
// read the same values: this module only decides which theme is active,
// persists the choice locally, and reads tokens back out for canvas use.

export type Theme = "dark" | "light";

/** Dark is the default (owner decision 2026-11-18). */
export const DEFAULT_THEME: Theme = "dark";

/** localStorage key. Local-only: nothing is ever sent off the machine. */
export const THEME_STORAGE_KEY = "bytemap.theme";

export function isTheme(value: unknown): value is Theme {
  return value === "dark" || value === "light";
}

/** The browser storage, or null when it is unavailable (tests, disabled). */
function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null; // storage access can throw in restricted contexts
  }
}

/** Stored theme, or the default when storage is missing or corrupt. */
export function loadTheme(
  storage: Pick<Storage, "getItem"> | null = safeStorage(),
): Theme {
  try {
    const stored = storage?.getItem(THEME_STORAGE_KEY);
    return isTheme(stored) ? stored : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

/** Persist the choice (best effort: a failure must never break the UI). */
export function saveTheme(
  theme: Theme,
  storage: Pick<Storage, "setItem"> | null = safeStorage(),
): void {
  try {
    storage?.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // ignore: the theme still applies for this session
  }
}

function rootElement(): HTMLElement | null {
  return typeof document === "undefined" ? null : document.documentElement;
}

/**
 * Switch the theme by setting `data-theme` on <html>; all colors follow via
 * CSS. `color-scheme` is set too so native scrollbars and form controls
 * match instead of staying dark on a light page.
 */
export function applyTheme(theme: Theme, root: HTMLElement | null = rootElement()): void {
  if (!root) return;
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
}

/** The theme currently on <html> (falls back to the default). */
export function currentTheme(root: HTMLElement | null = rootElement()): Theme {
  const value = root?.dataset.theme;
  return isTheme(value) ? value : DEFAULT_THEME;
}

/**
 * Read a resolved CSS custom property, e.g. token("--bg"). Canvas renderers
 * use this so a theme switch repaints them without duplicating the palette:
 * returns "" outside a document (tests, SSR).
 */
export function token(name: string): string {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") {
    return "";
  }
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Category id -> color, straight from the active theme's --cat-N tokens. */
export function categoryColors(): Record<number, string> {
  const out: Record<number, string> = {};
  for (let cat = 0; cat <= 6; cat++) out[cat] = token(`--cat-${cat}`);
  return out;
}

/** Safety tier id -> color, from the active theme's --tier-N tokens. */
export function tierColors(): Record<number, string> {
  const out: Record<number, string> = {};
  for (let tier = 0; tier <= 2; tier++) out[tier] = token(`--tier-${tier}`);
  return out;
}
