# Roadmap (checkbox = not done; every task: ID, goal, files, verification)

## Phase 0 — Foundations (APPROVED 2026-11-14, order amended: T5 before T4)
- [x] P0.T1a | Memory files pt.1 | AGENTS.md, docs/agent/PROJECT_BRIEF.md, docs/agent/DECISIONS.md, docs/agent/ROADMAP.md | files exist, AGENTS.md <=60 lines
- [x] P0.T1b | Memory files pt.2 | docs/agent/STATUS.md, CONTEXT_RESUME.md, SESSION_LOG.md, OPEN_QUESTIONS.md | files exist, CONTEXT_RESUME.md <=40 lines
- [x] P0.T2 | Git init (main) + remote origin + workspace scaffold: .gitignore, LICENSE-MIT, LICENSE-APACHE, rust-toolchain.toml (1.97.1, clippy+rustfmt), root Cargo.toml (license "MIT OR Apache-2.0"), crates/bytemap-core stub | 5 files + licenses | cargo check -p bytemap-core
- [x] P0.T3 | bytemap-scan + bytemap-actions stubs (no deps) | 4 files | cargo check --workspace; commit Cargo.lock
- [x] P0.T5 | Frontend scaffold (React 19, TS, Vite) | package.json, vite.config.ts, tsconfig.json, index.html, src/main.tsx | npm install && npm run build -> dist/ ; commit package-lock.json
- [x] P0.T4 | src-tauri shell (tauri, tauri-build, serde, serde_json; frontendDist ../dist; io.github.archanagaur175-glitch.bytemap) + placeholder icon via npx tauri icon | src-tauri/Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, src/main.rs | cargo check -p bytemap
- [x] P0.T6 | scripts/check.ps1: fmt --check, cargo check, clippy -D warnings, cargo test, npm run build, vitest --passWithNoTests; stop on first failure | scripts/check.ps1 | powershell -File scripts/check.ps1 exits 0
- [x] P0.GATE | Full check pass; update memory files; commit; push; <=10-line summary; ask APPROVED: Phase 1 | docs/agent/* | scripts/check.ps1 exit 0

## Phase 1 — Core model and walker scanner (amendments recorded in DECISIONS.md; DO NOT START until approved)
- [x] P1.T1 | Arena tree: Vec<Node>, u32 indices, shared string arena, <=80 B/entry target | bytemap-core/src/tree.rs, lib.rs + 1 test | cargo test -p bytemap-core
- [x] P1.T2 | Roll-ups: aggregate logical + on-disk sizes, sort children by size | bytemap-core/src/rollup.rs + 1 test | cargo test -p bytemap-core
- [x] P1.T3 | Walker scanner (jwalk + rayon): reparse via file_attributes (0x400 / 0x400000 / 0x40000), windows-sys only for file IDs, long paths, denied-count, no OneDrive hydration; size on disk = logical rounded to cluster size, GetCompressedFileSizeW only for compressed/sparse | bytemap-scan/src/walker.rs, entry.rs, error.rs, lib.rs | cargo test -p bytemap-scan
- [x] P1.T3b | Hardlink dedup behind an option; benchmark with and without; report both to owner; record chosen default | bytemap-scan/src/hardlink.rs + bench wiring | cargo run --release --bin bench-walk (both modes)
- [x] P1.T4 | Fixture tests: junction, symlink, hardlink, ACL-denied (#[ignore] with reason if icacls fails), deep paths | bytemap-scan/tests/fixtures.rs, walker_rules.rs | cargo test -p bytemap-scan --test walker_rules
- [x] P1.T5 | Benchmark binary; 100k fixture built once into %TEMP%\bytemap-bench-100k, reused; report cold + warm + Defender status | bytemap-scan/src/bin/bench-walk.rs | cargo run --release -p bytemap-scan --bin bench-walk
- [x] P1.T6 | Clippy-clean; record 100k benchmark in STATUS/SESSION_LOG | docs/agent/* | scripts/check.ps1
- [ ] P1.T7 | (SPIKE, PENDING OWNER APPROVAL — DO NOT RUN) mft_probe.rs example with ntfs-reader, read-only, entry count + elapsed; owner runs it (elevation) | bytemap-scan/examples/mft_probe.rs | owner-run manually
- [x] P1.GATE | Tests pass; junction/hardlink/denied covered; 100k benchmark recorded | — | scripts/check.ps1

Result: 100k fixture (101,112 entries incl. marker): cold 1.407 s / 71.9k entries/s, warm 1.379 s / 73.3k entries/s (process 2 cold 1.233 s / 82.0k); with hardlink dedup 2.659 s / 38.0k entries/s. Defender real-time protection ACTIVE during runs. Walker target (< 30 s per 1M entries = 33.3k/s) met in both modes. Dedup default: AWAITING OWNER CHOICE.

## Phase 2 — IPC and minimal UI (APPROVED 2026-11-15)
- [x] P2.T1a | Scanner hooks: cancel flag + per-entry progress callback (caller throttles) | bytemap-scan/src/walker.rs, error.rs, lib.rs | cargo test -p bytemap-scan (9 passed incl. cancel + progress)
- [x] P2.T1b | start_scan/cancel_scan commands; session state (Rust owns tree); Throttle 50ms unit test | src-tauri/src/commands.rs, state.rs, main.rs | cargo test -p bytemap
- [x] P2.T2 | Binary view API: build_view/serialize_view in bytemap-core (top-N + folded "N small items", packed LE); node_view command via ipc::Response Raw | crates/bytemap-core/src/view.rs, src-tauri/src/view.rs, commands.rs | cargo test -p bytemap-core (10 passed)
- [x] P2.T4a | Frontend store (zustand) + binary view parser + vitest | src/store.ts, src/parseView.ts, src/parseView.test.ts | npx vitest run (3 passed)
- [x] P2.T4b | Minimal UI: virtualized tree list (28px rows, window rendering), breadcrumb, progress line, dark tokens | src/App.tsx, src/TreeList.tsx, src/styles.css, src/main.tsx | npm run build
- [x] P2.T5 | Testability refactor: lib crate bytemap_lib + run_scan_job extraction; full-flow integration test on %TEMP%\bytemap-bench-100k (throttling proven, view bytes decoded) | src-tauri/src/lib.rs, commands.rs, src-tauri/tests/flow.rs | scripts/check.ps1
- [x] P2.GATE | Full check green; memory files updated; commit + push | docs/agent/* | scripts/check.ps1 exit 0

## Phase 3 — Orbit view (APPROVED 2026-11-15)
- [x] P3.T1 | Rust sunburst layout: orbit.rs in bytemap-core (angle = share of parent's aggregate size, rings = depth, fixed-point 65536/turn, min-sweep skip, arc cap), packed-LE serialize; node_orbit command via ipc::Response Raw | crates/bytemap-core/src/orbit.rs, lib.rs; src-tauri/src/view.rs, commands.rs, lib.rs | cargo test -p bytemap-core (6 new orbit tests) && cargo test -p bytemap
- [x] P3.T2a | orbit binary parser + hit-test geometry (ring + angle, wrap-safe) + Renderer interface/Canvas2DRenderer | src/orbit.ts, src/orbit.test.ts, src/renderer.ts | npx vitest run (6 new tests)
- [x] P3.T2b | OrbitView canvas component (hover/click hit-test) wired to store (openNode now loads orbit too); orbitView.ts renamed orbit.ts (Windows case-insensitive collision with OrbitView.tsx) | src/OrbitView.tsx, src/store.ts, src/App.tsx, src/styles.css | npm run build && npx vitest run
- [x] P3.T3 | Drill: click arc re-centers via openNode; 320ms ease-out zoom drill motion, instant under prefers-reduced-motion; breadcrumb navigation (from P2) | src/renderer.ts (zoom/alpha), src/OrbitView.tsx (rAF) | npm run build
- [x] P3.T4 | Inspector panel: name, logical/on-disk, ring, flag badges (text, never color-only) | src/Inspector.tsx, src/OrbitView.tsx, src/styles.css | npm run build
- [x] P3.T5 | Perf gate: MEASURED on real hardware via agent-run `npx tauri dev` with VITE_BENCH=orbit + bench_report stdout capture: ORBIT-BENCH arcs=4000 frames=776 avgMs=7.78 p95Ms=13.90 avgFps=128.6 (gate 60fps @ <=4000 primitives: passed) | src/perf.ts, src/OrbitBench.tsx, src/benchConfig.ts, src/main.tsx; src-tauri bench_report command | bench output recorded in SESSION_LOG.md
- [x] P3.GATE | Full check green; memory files updated; commit + push; summary; ask APPROVED: Phase 4 | docs/agent/* | scripts/check.ps1 exit 0

## Phase 4 — Categorization and safety tiers (APPROVED 2026-11-15)
- [x] P4.T1 | TOML rule engine in bytemap-core (categorizer.rs): path rules, then ext rules, then size heuristics; embedded rules.toml via include_str!; categories Code/System/Archives/Media/Documents/Apps/Other; toml 1.2.1 added (owner-approved dep) | categorizer.rs + rules.toml + tests | cargo test -p bytemap-core (6 new tests)
- [x] P4.T2a | Wire formats v2 (u8 cat + u8 tier per item/arc); session stores CategoryMap, computed once per completed scan; flow test decodes v2 | bytemap-core view.rs/orbit.rs; src-tauri state.rs, commands.rs, view.rs, tests/flow.rs | cargo test --workspace (35 tests) && clippy -D warnings
- [x] P4.T2b | Frontend parsers v2 + CAT_*/TIER_* ids; vitest 10/10 | src/parseView.ts, src/orbit.ts, both test files, src/perf.ts | npx vitest run && npm run build
- [x] P4.T3a | Legend: category chips (brief colors) + tier chips, click toggles filter | src/colors.ts, src/Legend.tsx, src/store.ts, src/styles.css, src/App.tsx | npm run build
- [x] P4.T3b | Orbit arcs colored by category; filtered items dimmed in orbit + list rows | src/renderer.ts, src/OrbitView.tsx, src/TreeList.tsx, src/styles.css | npm run build && npx vitest run
- [x] P4.T4 | Protected gate: fail-closed can_offer_delete(tier) in bytemap-actions (only Safe/Review; unknown ids refused) + 4 tests; inspector shows category + tier pill (text + color) | crates/bytemap-actions/src/lib.rs, src/Inspector.tsx, src/styles.css | cargo test --workspace && npm run build
- [x] P4.GATE | Full check green (after cargo fmt); memory files updated; commit + push; summary; ask APPROVED: Phase 5 | docs/agent/* | scripts/check.ps1 GATE_EXIT=0

## Phase 5 — Action Hub (APPROVED 2026-11-16, detail approved same day; HIGHEST RISK, owner code review before close)
Brief (line 26): basket; dry run listing every path, size, tier, in-use status (Restart Manager); Recycle Bin by default; permanent delete only with typed confirmation, never for Protected; stale-scan guard (re-check size + mtime vs scan, drop changed items before executing); append-only undo journal; archive mode = compress, verify, then remove original. Gate (P4.T4): every action path calls can_offer_delete first. Documents excluded from bulk actions by default (brief line 24). UI sends node IDs only; Rust resolves paths.
- [x] P5.T1 | Basket model in bytemap-actions: BasketItem (node_id, path, name, logical, on_disk, tier, cat) + Basket add/remove/clear; add() refuses Protected/unknown tier (fail-closed) | crates/bytemap-actions/src/basket.rs, lib.rs | cargo test -p bytemap-actions (5+ new tests)
- [x] P5.T2 | Session + commands: Session gains root_path + basket; node->absolute-path resolution (parent chain + root); basket_add/remove/clear/list commands; bulk-add excludes Documents by default (include_documents override) | src-tauri/src/state.rs, commands.rs, pathres.rs | cargo test -p bytemap (basket flow test on %TEMP% fixture)
- [x] P5.T3 | Dry run + stale-scan guard: re-stat each path (exists, size, mtime) vs scan snapshot -> DryRunReport {path, size_now, size_scan, tier, cat, in_use, status ok|changed|missing}; in-use via Restart Manager FFI (windows-sys RmStartSession/RmRegisterResources/RmGetList) | crates/bytemap-actions/src/dryrun.rs, inuse.rs, Cargo.toml | cargo test -p bytemap-actions (fixture: modify -> changed, delete -> missing)
- [x] P5.T4 | Recycle + permanent delete: trash 5.3.0 (verified latest, on approved list); can_offer_delete gate + stale guard enforced before executing; permanent delete requires exact confirmation string from UI; append-only JSONL journal starts here | bytemap-actions/src/recycle.rs, journal.rs, Cargo.toml, src-tauri commands | cargo test -p bytemap-actions (%TEMP% fixture recycled -> path gone)
- [ ] P5.T5 | Undo: journal in %LOCALAPPDATA%\ByteMap\undo (dir overridable by env for tests); recycle undo restores from Recycle Bin (trash Windows os API); undo_last command | bytemap-actions/src/restore.rs, journal.rs, commands | cargo test -p bytemap-actions (temp-dir journal round-trip)
- [ ] P5.T6 | Archive mode: custom container (JSONL manifest + raw file bytes, whole stream zstd-compressed), verify = decompress + compare sizes + SHA-256, then remove original; undo entry records archive location | bytemap-actions/src/archive.rs, commands | cargo test (fixture round-trip: archive -> verify -> original removed -> restore)
- [ ] P5.T7 | Action Hub UI: basket panel (list/remove/clear), dry-run report view, Recycle / Permanent (typed confirm modal) / Archive buttons, Undo; Protected rows disabled (UI mirrors Rust gate); Documents-excluded notice on bulk add | src/ActionHub.tsx, src/store.ts, src/App.tsx, src/styles.css | npm run build && npx vitest run
- [ ] P5.GATE | scripts/check.ps1 green; OWNER CODE REVIEW of the full phase diff (rule 7) before close; memory files; commit + push | docs/agent/* | GATE_EXIT=0 + owner review recorded in DECISIONS.md


## Phase 6 — MFT fast path (detail pending approval)
- [ ] P6.T0 | DECIDE CODE SIGNING BEFORE BUILDING THE ELEVATED HELPER (owner undecided)
- [ ] P6.T1 elevated helper process | [ ] P6.T2 named-pipe protocol | [ ] P6.T3 MFT scan + fallback | [ ] P6.T4 parity benchmark gate

## Phase 7 — Strata and visual polish (detail pending approval)
- [ ] P7.T1 treemap view | [ ] P7.T2 optional Nebula | [ ] P7.T3 Mica/acrylic + motion + themes | [ ] P7.T4 token match + reduced-motion gate

## Phase 8 — Snapshots and change tracking (detail pending approval)
- [ ] P8.T1 save/load (bincode + zstd) | [ ] P8.T2 diff view | [ ] P8.T3 USN refresh | [ ] P8.T4 CSV export | [ ] P8.T5 <1 s reopen @ 1M gate

## Phase 9 — Packaging and hardening (detail pending approval)
- [ ] P9.T1 NSIS installer | [ ] P9.T2 portable zip | [ ] P9.T3 signing per P6.T0 decision | [ ] P9.T4 a11y/perf audit | [ ] P9.T5 clean install/uninstall on fresh Win11 VM
