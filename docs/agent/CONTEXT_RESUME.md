# CONTEXT_RESUME.md — what a fresh agent must know in two minutes

Project: ByteMap — native Windows 11 disk-space analyzer. Tauri 2 + Rust workspace
(crates/bytemap-core, bytemap-scan, bytemap-actions, src-tauri) + React 19/TS/Vite frontend.
Licence: MIT OR Apache-2.0 (dual). Repo: C:\Users\archa\Documents\ByteMap.
Remote: origin https://github.com/archanagaur175-glitch/ByteMap (branch main, push authorized after each task).

Current state: Phase 0 approved; memory files staged in C:\Users\archa\.opencode\plan\
(AGENTS.md, PROJECT_BRIEF.md, DECISIONS.md, ROADMAP.md, STATUS.md, CONTEXT_RESUME.md,
SESSION_LOG.md, OPEN_QUESTIONS.md, phase0-execution.md) because repo writes were blocked in
plan mode. First task after switch: copy them into the repo verbatim, then follow
phase0-execution.md task order: T1a, T1b, T2, T3, T5, T4, T6, GATE.

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
- Toolchain pinned 1.97.1 (rust-toolchain.toml, clippy + rustfmt). Check: scripts/check.ps1.
- Open item: P6.T0 signing decision (owner undecided). P1.T7 MFT spike awaits owner approval.
- Windows 11 is the target; Win10 = best-effort flat fallback, untested.
