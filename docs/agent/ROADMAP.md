# Roadmap (checkbox = not done; every task: ID, goal, files, verification)

## Phase 0 — Foundations (APPROVED 2026-11-14, order amended: T5 before T4)
- [ ] P0.T1a | Memory files pt.1 | AGENTS.md, docs/agent/PROJECT_BRIEF.md, docs/agent/DECISIONS.md, docs/agent/ROADMAP.md | files exist, AGENTS.md <=60 lines
- [ ] P0.T1b | Memory files pt.2 | docs/agent/STATUS.md, CONTEXT_RESUME.md, SESSION_LOG.md, OPEN_QUESTIONS.md | files exist, CONTEXT_RESUME.md <=40 lines
- [ ] P0.T2 | Git init (main) + remote origin + workspace scaffold: .gitignore, LICENSE-MIT, LICENSE-APACHE, rust-toolchain.toml (1.97.1, clippy+rustfmt), root Cargo.toml (license "MIT OR Apache-2.0"), crates/bytemap-core stub | 5 files + licenses | cargo check -p bytemap-core
- [ ] P0.T3 | bytemap-scan + bytemap-actions stubs (no deps) | 4 files | cargo check --workspace; commit Cargo.lock
- [ ] P0.T5 | Frontend scaffold (React 19, TS, Vite) | package.json, vite.config.ts, tsconfig.json, index.html, src/main.tsx | npm install && npm run build -> dist/ ; commit package-lock.json
- [ ] P0.T4 | src-tauri shell (tauri, tauri-build, serde, serde_json; frontendDist ../dist; io.github.archanagaur175-glitch.bytemap) + placeholder icon via npx tauri icon | src-tauri/Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, src/main.rs | cargo check -p bytemap
- [ ] P0.T6 | scripts/check.ps1: fmt --check, cargo check, clippy -D warnings, cargo test, npm run build, vitest --passWithNoTests; stop on first failure | scripts/check.ps1 | powershell -File scripts/check.ps1 exits 0
- [ ] P0.GATE | Full check pass; update memory files; commit; push; <=10-line summary; ask APPROVED: Phase 1 | docs/agent/* | scripts/check.ps1 exit 0

## Phase 1 — Core model and walker scanner (amendments recorded in DECISIONS.md; DO NOT START until approved)
- [ ] P1.T1 | Arena tree: Vec<Node>, u32 indices, shared string arena, <=80 B/entry target | bytemap-core/src/tree.rs, lib.rs + 1 test | cargo test -p bytemap-core
- [ ] P1.T2 | Roll-ups: aggregate logical + on-disk sizes, sort children by size | bytemap-core/src/rollup.rs + 1 test | cargo test -p bytemap-core
- [ ] P1.T3 | Walker scanner (jwalk + rayon): reparse via file_attributes (0x400 / 0x400000 / 0x40000), windows-sys only for file IDs, long paths, denied-count, no OneDrive hydration; size on disk = logical rounded to cluster size, GetCompressedFileSizeW only for compressed/sparse | bytemap-scan/src/walker.rs, entry.rs, error.rs, lib.rs | cargo test -p bytemap-scan
- [ ] P1.T3b | Hardlink dedup behind an option; benchmark with and without; report both to owner; record chosen default | bytemap-scan/src/hardlink.rs + bench wiring | cargo run --release --bin bench-walk (both modes)
- [ ] P1.T4 | Fixture tests: junction, symlink, hardlink, ACL-denied (#[ignore] with reason if icacls fails), deep paths | bytemap-scan/tests/fixtures.rs, walker_rules.rs | cargo test -p bytemap-scan --test walker_rules
- [ ] P1.T5 | Benchmark binary; 100k fixture built once into %TEMP%\bytemap-bench-100k, reused; report cold + warm + Defender status | bytemap-scan/src/bin/bench-walk.rs | cargo run --release -p bytemap-scan --bin bench-walk
- [ ] P1.T6 | Clippy-clean; record 100k benchmark in STATUS/SESSION_LOG | docs/agent/* | scripts/check.ps1
- [ ] P1.T7 | (SPIKE, PENDING OWNER APPROVAL — DO NOT RUN) mft_probe.rs example with ntfs-reader, read-only, entry count + elapsed; owner runs it (elevation) | bytemap-scan/examples/mft_probe.rs | owner-run manually
- [ ] P1.GATE | Tests pass; junction/hardlink/denied covered; 100k benchmark recorded | — | scripts/check.ps1

## Phase 2 — IPC and minimal UI (detail pending approval)
- [ ] P2.T1 scan start/cancel commands | [ ] P2.T2 batched progress via Channel <=20/s | [ ] P2.T3 binary view API (tauri::ipc::Response) | [ ] P2.T4 virtualized tree list | [ ] P2.T5 real-folder no-freeze gate (zustand approved here)

## Phase 3 — Orbit view (detail pending approval)
- [ ] P3.T1 Rust sunburst layout | [ ] P3.T2 Renderer interface + Canvas 2D | [ ] P3.T3 drill + breadcrumb | [ ] P3.T4 hover + inspector | [ ] P3.T5 60 fps @ 4000 primitives gate

## Phase 4 — Categorization and safety tiers (detail pending approval)
- [ ] P4.T1 embedded TOML rule engine | [ ] P4.T2 tiers + badges | [ ] P4.T3 legend + filters | [ ] P4.T4 Protected-never-deletable gate

## Phase 5 — Action Hub (detail pending approval; owner code review before close)
- [ ] P5.T1 basket | [ ] P5.T2 dry run + Restart Manager | [ ] P5.T3 recycle + stale-scan guard | [ ] P5.T4 undo journal | [ ] P5.T5 archive mode

## Phase 6 — MFT fast path (detail pending approval)
- [ ] P6.T0 | DECIDE CODE SIGNING BEFORE BUILDING THE ELEVATED HELPER (owner undecided)
- [ ] P6.T1 elevated helper process | [ ] P6.T2 named-pipe protocol | [ ] P6.T3 MFT scan + fallback | [ ] P6.T4 parity benchmark gate

## Phase 7 — Strata and visual polish (detail pending approval)
- [ ] P7.T1 treemap view | [ ] P7.T2 optional Nebula | [ ] P7.T3 Mica/acrylic + motion + themes | [ ] P7.T4 token match + reduced-motion gate

## Phase 8 — Snapshots and change tracking (detail pending approval)
- [ ] P8.T1 save/load (bincode + zstd) | [ ] P8.T2 diff view | [ ] P8.T3 USN refresh | [ ] P8.T4 CSV export | [ ] P8.T5 <1 s reopen @ 1M gate

## Phase 9 — Packaging and hardening (detail pending approval)
- [ ] P9.T1 NSIS installer | [ ] P9.T2 portable zip | [ ] P9.T3 signing per P6.T0 decision | [ ] P9.T4 a11y/perf audit | [ ] P9.T5 clean install/uninstall on fresh Win11 VM
