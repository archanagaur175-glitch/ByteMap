# Design tokens — brief ↔ code match (Phase 7, T4)

The contract is PROJECT_BRIEF.md line 28. This page is the audit trail: every
value there, where it lives, and what enforces it.

**Single source of truth:** `src/styles.css` (`:root` = dark, the default;
`:root[data-theme="light"]` = light). Nothing else holds a palette — the canvas
renderers read the same custom properties through `src/theme.ts::token()`, so a
color can only be changed in one place.

**Enforcer:** `src/theme.test.ts` reads the stylesheet and asserts the exact
values below, for both themes. `scripts/check.ps1` runs it, so a drifted token
fails the gate.

## Colors

| Brief token | Dark (default) | Light | CSS variable | Consumers |
|---|---|---|---|---|
| background | `#0B0F1A` | `#F3F5FA` | `--bg` | body, canvas background |
| text | `#E8ECF6` | `#121826` | `--text` | body, canvas ink |
| accent | `#6EA8FF` | `#2F6FE0` | `--accent` | buttons, crumbs, focus ring |
| Code | `#4FD1C5` | `#1FA89C` | `--cat-0` | orbit arcs, strata tiles, dots, legend |
| System | `#A78BFA` | `#7C5CE0` | `--cat-1` | same |
| Archives | `#FFB347` | `#D98A12` | `--cat-2` | same |
| Media | `#FF5C8A` | `#E03A6B` | `--cat-3` | same |
| Documents | `#5B9DFF` | `#2F6FE0` | `--cat-4` | same |
| Apps | `#8BDD6B` | `#4FA83A` | `--cat-5` | same |
| Other | `#8892A6` | `#6B7488` | `--cat-6` | same |
| Safe | `#3DDC97` | `#1E9E68` | `--tier-0` | tier pill, legend |
| Review | `#FFC857` | `#C48A00` | `--tier-1` | same |
| Protected | `#FF5D5D` | `#D13B3B` | `--tier-2` | same, plus error text |

Derived (not in the brief, chosen to sit next to the tokens above): `--panel`,
`--glass`, `--muted`, `--border`, `--line`, `--hover`, `--selected`, `--ok`,
`--warn`, `--danger*`, `--scrim`, `--on-accent`. They are theme-local values of
the same one source of truth.

Category ids are the wire ids from `bytemap-core::categorizer`: 0 Code,
1 System, 2 Archives, 3 Media, 4 Documents, 5 Apps, 6 Other. Tier ids: 0 Safe,
1 Review, 2 Protected.

## Geometry and motion

| Brief | Value | Where |
|---|---|---|
| panel radius | 12px | `--radius` in styles.css |
| glass blur | 24px | `--glass-blur`, used by every glass surface (`backdrop-filter`) |
| standard motion | 180ms | `--motion` in CSS, `STANDARD_MS` in theme.ts (test ties them together) |
| drill motion | 320ms | `DRILL_MS` in theme.ts, used by OrbitView and StrataView |
| fonts | Segoe UI Variable / Cascadia Mono | body font stack, `.mono` |

Mica/acrylic is the window backdrop, not a CSS value: `src-tauri/src/backdrop.rs`
applies it and reports `mica` / `acrylic` / `none`; the UI sets
`data-backdrop` and only lets the window be transparent for a real material.

## Reduced motion

Three layers, all enforced:

1. `styles.css` `@media (prefers-reduced-motion: reduce)` zeroes every CSS
   transition and animation duration (tested).
2. Both canvas views call `reducedMotion()` from `theme.ts` before starting a
   drill animation; when it is true they draw one final frame instead (tested —
   the test also fails if either view starts calling `matchMedia` directly or
   drops `DRILL_MS`).
3. No `@keyframes` exist; a test fails if one is added without being gated.

`npx tauri dev` follows the Windows animation setting, so the OS toggle is the
switch to test it by hand.
