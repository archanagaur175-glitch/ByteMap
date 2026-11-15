# CONTEXT_RESUME.md — what a fresh agent must know in two minutes

Project: ByteMap — native Windows 11 disk-space analyzer. Tauri 2 + Rust workspace
(crates/bytemap-core, bytemap-scan, bytemap-actions, src-tauri) + React 19/TS/Vite frontend.
Licence: MIT OR Apache-2.0 (dual). Repo: C:\Users\archa\Documents\ByteMap.
Remote: origin https://github.com/archanagaur175-glitch/ByteMap (branch main, push authorized after each task).

Current state: PHASES 0+1 COMPLETE, gates green (2026-11-15). Arena tree + roll-ups in bytemap-core;
walker scanner (jwalk 0.9) with all scanner rules verified by fixture tests; bench-walk binary with
reusable %TEMP%\bytemap-bench-100k fixture (101,112 entries: cold 71.9k/s, warm 73.3k/s, best 82.0k/s,
with hardlink dedup 38.0k/s; target 33.3k/s met; Defender real-time was ON). Phase 2 NOT started;
awaiting owner "APPROVED: Phase 2". Two owner items pending: dedup default choice, P1.T7 MFT spike approval.

Environment facts: rustc/cargo 1.97.1 pinned via rust-toolchain.toml (clippy + rustfmt components);
node 22.22.3 / npm 10.9.8; Tauri CLI comes from npm @tauri-apps/cli (no global install);
npm stack: vite 8.x, @vitejs/plugin-react 6.x, vitest 5.x, TS ^5 (7.x exists, deliberately avoided);
Tauri crate 2.12.5; src-tauri bundle id io.github.archanagaur175-glitch.bytemap;
frontendDist "../dist" (dist/ is gitignored but must exist before tauri build); placeholder icon
generated from app-icon/icon-source.png (not a brand asset). Scan deps: jwalk 0.9.0, windows-sys 0.61.2,
thiserror 2.0.21. Gotchas learned: [target.'cfg(windows)'.dependencies] (not [target.windows]);
jwalk: skip_hidden(false) required, failed child read_dir attaches to read_children.error (pub), parents
before children guaranteed (strict DFS), metadata() = symlink_metadata (no follow); denied-dir probe:
icacls /deny user:(OI)(CI)R + self-verify read_dir fails.

Non-negotiables:
- Read AGENTS.md -> docs/agent/CONTEXT_RESUME.md -> docs/agent/STATUS.md every session.
- One phase at a time; gate checks; owner approval between phases ("APPROVED: <phase name>").
- Tasks <=5 files, each with a verification command; commit "P<phase>.T<n>: <goal>" per task.
- Never claim success without running the check; log output in SESSION_LOG.md.
- Rust owns the tree; UI gets binary views only (tauri::ipc::Response), progress via Channel <=20/s.
- UI sends node IDs only; Rust resolves paths; Recycle Bin default; stale-scan guard; undo journal.
- Scanner rules: no reparse follow (0x400), no OneDrive hydration (0x400000/0x40000), hardlink
  dedup by file ID behind an option, long paths, denied-count instead of abort, logical + on-disk size.
- Tests use %TEMP% fixtures only; 100k bench fixture reused at %TEMP%\bytemap-bench-100k.
- Local-only product: no telemetry, no network calls, no accounts.
- Deps approved for Phase 0 only: react, react-dom, vite, typescript, @vitejs/plugin-react,
  @types/react, @types/react-dom, @tauri-apps/api, @tauri-apps/cli (dev), vitest (dev);
  Rust: tauri, tauri-build, serde, serde_json (src-tauri only). zustand waits for Phase 2.
- Check: scripts/check.ps1 (fmt, check, clippy -D warnings, test, npm build, vitest).
- Open item: P6.T0 signing decision (owner undecided). P1.T7 MFT spike awaits owner approval.
- Windows 11 is the target; Win10 = best-effort flat fallback, untested.
