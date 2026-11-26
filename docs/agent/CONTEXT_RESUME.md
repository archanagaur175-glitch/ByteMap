# CONTEXT_RESUME.md — what a fresh agent must know in two minutes

Project: ByteMap — native Windows 11 disk-space analyzer. Tauri 2 + Rust workspace
(crates/bytemap-core, bytemap-scan, bytemap-actions, src-tauri) + React 19/TS/Vite frontend.
Licence: MIT OR Apache-2.0 (dual). Repo: C:\Users\archa\Documents\ByteMap.
Remote: origin https://github.com/archanagaur175-glitch/ByteMap (branch main, push authorized after each task).

Current state: // P5 (2026-11-17): T1-T7 COMPLETE, gate check green, AWAITING OWNER CODE REVIEW - basket + dry-run/stale guard + Restart Manager probe + recycle + permanent-delete(typed PERMANENT) + JSONL undo journal + undo + archive .bma (zstd+sha2, verify-then-remove) + Action Hub UI; tests 26+5+2 Rust + 10 vitest; deps added: trash 5.3, zstd 0.14.4, sha2 0.11.1 (all owner-approved). PHASES 0+1+2 COMPLETE, gates green (2026-11-15). Arena tree + roll-ups in bytemap-core;
walker scanner (jwalk 0.9) with all scanner rules verified by fixture tests; bench-walk binary with
reusable %TEMP%\bytemap-bench-100k fixture (101,112 entries: cold 71.9k/s, warm 73.3k/s, best 82.0k/s,
with hardlink dedup 38.0k/s; target 33.3k/s met; Defender real-time was ON). Phase 2 done: start_scan/
cancel_scan (progress Channel <=20/s, proven by flow.rs integration test on the real 100k volume),
binary node_view API (packed LE format in bytemap-core::view; top-N + folded "N small items"), zustand
store + parser (vitest 3/3), minimal UI (virtualized 28px-row TreeList, breadcrumb, dark tokens).
src-tauri is now a lib crate (bytemap_lib: run(), commands, state, view modules) + thin main.rs, so
src-tauri/tests/flow.rs drives the full scan pipeline without a webview. Phase 3 (Orbit view) approved
and T1..T4 done: orbit.rs sunburst layout in bytemap-core (fixed-point angles 65536/turn, MIN_SWEEP=16
skip, 4000-arc cap, packed LE via node_orbit command), Canvas2DRenderer behind a Renderer interface
with hit-test, OrbitView component wired into store/App (openNode fetches view+orbit in parallel),
320ms ease-out drill motion (instant under prefers-reduced-motion), Inspector panel with text flag
badges. src/orbitView.ts was renamed src/orbit.ts (Windows case-insensitive collision with
OrbitView.tsx broke tsc). P3.T5 perf gate MEASURED on real hardware: the agent ran `npx tauri dev`
with VITE_BENCH=orbit (app auto-opens into bench mode) and captured the result through the new
bench_report command printing to dev-process stdout: ORBIT-BENCH arcs=4000 frames=776 avgMs=7.78
p95Ms=13.90 avgFps=128.6 -> the brief's "60 fps at <=4000 primitives" gate PASSED. Phase 3 COMPLETE.
Phase 4 COMPLETE (2026-11-16): embedded TOML rule engine (toml 1.2.1, owner-approved dep) in
bytemap-core::categorizer — path rules, then ext rules, then size heuristics; protected names
(pagefile.sys, hiberfil.sys, swapfile.sys, Windows, System Volume Information, $Recycle.Bin) force
tier PROTECTED. Wire formats v2 (u8 cat + u8 tier per item/arc); view/orbit version = 2 everywhere.
Frontend: colors.ts (brief tokens), Legend with category+tier filter chips, orbit arcs colored by
category, filtered items dimmed, inspector shows category + tier pill. bytemap-actions now has the
fail-closed can_offer_delete(tier) gate (only Safe/Review; unknown ids refused) — every Phase 5
action path must call it first. 35 Rust tests + 10 vitest green.
Phase 5 COMPLETE + owner-reviewed (2026-11-17, APPROVED: Phase 6). Phase 6 (MFT fast path)
IN PROGRESS, T0-T3 done (2026-11-18): bytemap-helper crate = elevated bin (requireAdministrator
manifest via embed-resource .rc trick, bin test=false), pipe server \\.\pipe\bytemap-scan
(one instance, FILE_FLAG_FIRST_PIPE_INSTANCE), BYM1 v2 protocol (Record gained mtime i64,
PROTOCOL_VERSION=2). Helper mft.rs streams REAL MFT records (ntfs-reader 0.6.0 as real dep):
two passes - dirs grouped by parent, then root resolve (case-insensitive from record 5;
junctioned root refused) + BFS subtree (reparse stops descent) + one Record per non-DOS-alias
name; root Record FIRST (file_ref==parent_ref, name "" for drive root); logical = sum of
unnamed $DATA; allocated=0 (app computes); mtime = FILETIME ticks inline (pre-1970 -> 0);
every failure = terminal Error frame ("cannot open ..."). App side: bytemap-scan::mft_client
(MftClient, retry ~2s) + mft_tree::build_tree_from_records (out-of-order children via pending
queue, orphans -> denied, compressed/sparse re-measured post-stream via Tree::set_on_disk,
root node mirrors walker naming) + tests/mft_parity.rs (walker tree == synthetic-record tree).
src-tauri: start_scan(root, useMft) -> run_scan_job(use_mft) -> mft_scan (MftClient +
RecordStream iterator w/ cancel + progress -> build_tree_from_records) with AUTOMATIC walker
fallback; complete event carries "tier": "walker"/"mft" + "fallbackReason". P1.T7 spike
(owner, elevated, real C:): 527,391 entries/s = 1.58x the 333k entries/s T4 gate target.
T4 OWNER RUN (2026-11-18, elevated): PARITY PASSED exactly (walker == MFT: 101,112 entries,
files=100,001 dirs=1,111 denied=0, root logical equal). SPEED GATE originally FAILED (10,264 vs 333k)
by design, not by a bug: the helper did TWO full 1.73M-record $MFT passes per request (dir index +
stream) and MFT has no subtree shortcut, so a 100k subtree costs ~2 whole-volume passes (~9.9s).
Owner approved the gate adjustment (option A) and P6.T4d is done: helper single-pass fast path for
DRIVE ROOTS only (root record 5 first, stream every live record, no index, no second pass); folder
scans are routed to the walker BEFORE ensure_helper (no UAC prompt, fallbackReason names the rule);
bench-mft has two modes - parity (default, 100k fixture, exact counts + root logical) and
--whole-volume [drive] (single-pass stream, speed gate >= 333k entries/s, 1% count-drift sanity,
no huge tree built). T4a/T4b/T4c committed (96857b0, 6a805a8, d7ef858; memory 38735c4, ba572e4).
PHASE 6 CLOSED 2026-11-18 (owner-approved Phase 7 in the same turn). T4 speed number DEFERRED, not
passed: it is OPEN QUESTION Q4 (docs/agent/OPEN_QUESTIONS.md) with the exact elevated command; parity
passed exactly. Never claim the MFT speed gate as met until Q4 is answered.
PHASE 7 COMPLETE (owner-approved 2026-11-19; T2 Nebula skipped by owner). Built: Strata layered
treemap end to end (bytemap-core::strata squarified layout in a normalized 10000-unit canvas, depth
1..=4 default 3, 4000-tile budget, MAX_CHILDREN 32, packed LE v1 + node_strata command; src/strata.ts
decoder with deepest-tile hit-test; Canvas2DStrataRenderer; StrataView + Orbit/Strata tablist;
openNode fetches view+orbit+strata in ONE Promise.all) and the Windows 11 look (window-vibrancy 0.8.1
Mica -> acrylic -> none, reported to the UI as data-backdrop and runtime-proven: `[backdrop] mica`;
styles.css is the single color source with dark default + light per brief line 28, 12px radius, 24px
glass blur, 180ms motion; theme.ts persists the theme in localStorage; colors.ts caches the tokens
for DOM and canvas, renderers re-read them each frame; strata label ink follows tile lightness).
T4 gate: theme.test.ts proves both token sets, the motion scale and the reduced-motion contract
(CSS media block + shared theme.ts::reducedMotion()/DRILL_MS used by both views, no hardcoded
transition durations, no ungated keyframes); docs/agent/TOKENS.md is the audit table.
NEXT: Phase 8 (snapshots + change tracking) is APPROVED (2026-11-19). All decisions are closed:
usn-journal-rs 0.5.0 only in the ELEVATED helper (needs Administrator), snapshots in
%LOCALAPPDATA%\ByteMap\snapshots with no dialog plugin, synthetic 1M-node reopen gate, snapshot code in
bytemap-core, codec = hand-rolled versioned packed LE + zstd (NO bincode: 3.0.0 is a tombstone).
P8.T1 DONE: bytemap-core::snapshot (magic BYTEMAPS, u16 format v1 + u16 codec v1, zstd frame with content
checksum, header = created_at/tier/counts/totals/scan_ms/root_name/app_version/body lengths, body = node
count + string arena + cat/tier columns + 38-byte node records; child links rebuilt in arena order;
read_header works on a 33 KB prefix so the list UI never decompresses). Loader validates root/parent/depth/
name bounds+UTF-8/cat+tier ids/exact lengths and refuses truncation, trailing bytes and checksum damage.
39 core tests pass (10 new), clippy -D warnings clean, fmt clean, cargo check --workspace clean.
P8.T1b DONE (2026-11-20): src-tauri::snapshot_store = saves under %LOCALAPPDATA%\ByteMap\snapshots as
bytemap-<unix>[-n].bms (atomic .part + rename, borrowed save_snapshot_parts so the session lock never clones
the tree), header-only listing (corrupt files listed with an error so they can be deleted), strict bare-name
validation, idempotent delete; commands snapshot_save/list/load/delete registered in lib.rs. apply_snapshot
(refused while a scan runs) swaps tree/cats/root_path/scan_tier/scan_ms, clears the basket and sets
loaded_snapshot; new scans clear it. UI: Snapshots toolbar button + panel (save/load/delete) and a snapshot
banner; store gained the 6 snapshot actions. 13 bytemap lib tests (2 new swap/refusal tests), 39 core,
clippy clean, cargo check workspace clean, build + vitest 33, scripts/check.ps1 ALL CHECKS PASSED.
P8.T2 DONE (2026-11-21): bytemap-core::diff walks both trees from the root, matching children by lowercase name
(NTFS case-insensitive) in an explicit stack. Common directories are only descended into (never entries, so
ancestors cannot double-count); added/removed directories collapse to one entry carrying their subtree total;
files report grew/shrank logical deltas. Summary counts/bytes are complete, entries are capped (200 default,
5000 max) and ranked by |delta|; different roots are refused. Wire v1 packed LE (header + kind/is_dir/cat/tier/
before/after/path). src-tauri: view::diff_bytes + snapshot_diff command (read-only, refuses old == new).
UI: compare selects in the Snapshots panel + DiffView (added/removed/grew/shrank cards, net, top-mover rows).
50 core tests (11 new), 14 bytemap lib (1 new wiring test), vitest 40 (7 new), clippy workspace clean,
cargo check workspace clean, scripts/check.ps1 ALL CHECKS PASSED. Also fixed a pre-existing flake: the
mft_tree unit tests shared one PID-named temp folder while running in parallel (now per-test counter).
NEXT: OWNER REVIEW of P8.GATE (checklist green 2026-11-26; on approval close Phase 8 and ask for Phase 9). The owner elevated
USN proof (Q6) and the MFT whole-volume speed gate (Q4) are STILL OUTSTANDING and must not be claimed met. P8.GATE DONE
(2026-11-26): scripts/check.ps1 ALL CHECKS PASSED (core 65, helper 22, scan 4+11+8+1+9, actions 26, bytemap 21+4, reopen_1m
1+1 ignored, vitest 46; clippy/fmt/build clean); release reopen re-run reopen_ms=250 (PASS); usn-journal-rs only in the helper
manifest, Cargo.lock gained exactly lru + usn-journal-rs, no dialog/HTTP/telemetry deps or calls; tree clean at 6afc827 and
1c07508..6afc827 pushed. Code review of refresh.rs + snapshot_store.rs found no defect (unique_path never overwrites: every
candidate is exists()-checked, then it errors). No webview click-through of the Phase 8 UI yet. P8.T5 DONE (2026-11-26):
crates/bytemap-core/tests/reopen_1m.rs builds 1,001,001 nodes (root + 1,000 folders x 1,000 files), saves through save_snapshot,
writes the real .bms, then times fs::read + load_snapshot = the user-visible "reopen": 232/233 ms in release (target < 1000 ms;
save 518-635 ms reported, not gated; snapshot 7.78 MB). The timed test is #[ignore]d with the exact command in its reason
(cargo test --release -p bytemap-core --test reopen_1m -- --ignored --nocapture) so the normal gate stays fast; a small always-on
round-trip test in the same file covers the codec in every build. Fixture gotcha: TREE flags are not Win32 attributes —
flags::DIR = 1, not 0x10; using 0x10 made roll-ups treat folders as files (root total 0). Workspace tests, clippy, fmt, build,
vitest 46 and scripts/check.ps1 ALL CHECKS PASSED (2026-11-26).
P8.T4 DONE (2026-11-26): CSV export writes RFC 4180 (CRLF, minimal quoting, raw byte sizes) to
%LOCALAPPDATA%\ByteMap\exports\bytemap-<scope>-<unix>[-n].csv (BYTEMAP_EXPORT_DIR overrides; NO dialog plugin) and returns
the absolute path; scope "view" = every direct child of the open node sorted by size (not the folded top-200 — a folded row
has no path), scope "basket" = staging order with the kind read from the current tree (empty for a stale id); cap 100k rows
with an explicit truncated/totalRows; refused while a scan runs, with no tree, or for an unknown scope. UI: Export CSV in the
toolbar, CSV in the Action Hub, notice with counts + full path. categorizer::{category_name,tier_name} are public now (unknown
tier reads "unknown"). Tests: bytemap lib 21 (7 new), vitest 46 (6 new), workspace tests + clippy + fmt + build green,
scripts/check.ps1 ALL CHECKS PASSED (2026-11-26).
P8.T3 DONE (2026-11-25, code + tests + gate; owner elevated proof pending = Q6): USN refresh is
PATH-based (snapshots store no file ids) — helper resolves journal entries with an uncached path resolver, stats each path and
streams UsnChange frames (protocol v3: 5 UsnRequest, 6 UsnCheckpoint, 7 UsnChange, 8 UsnDone); bytemap-core::refresh applies an
order-independent fact list to the snapshot tree (case-insensitive paths, whole-subtree removal, MAX_REFRESH_CHANGES 500k);
snapshot format v2 stores the checkpoint (v1 files still load, usn = None) and a whole-volume MFT scan emits UsnCheckpoint first
(best effort). Fail-closed: journal_id mismatch, wrapped cursor, cursor ahead, a renamed FOLDER, >500k paths, unplaceable facts,
or a count mismatch refuse with "run a full scan"; a non-NotFound stat is unresolved, never a guessed delete. snapshot_refresh
loads the file, applies the delta, saves a NEW timestamped snapshot and loads it, so the old one survives. UI: Refresh button on
refreshable rows + counts notice. Tests: core 65, helper 22; workspace tests, clippy, fmt, build, vitest 40 and
scripts/check.ps1 ALL CHECKS PASSED (2026-11-25).

Environment facts: rustc/cargo 1.97.1 pinned via rust-toolchain.toml (clippy + rustfmt components);
node 22.22.3 / npm 10.9.8; Tauri CLI comes from npm @tauri-apps/cli (no global install);
npm stack: vite 8.x, @vitejs/plugin-react 6.x, vitest 5.x, TS ^5 (7.x exists, deliberately avoided);
Tauri crate 2.12.5; src-tauri bundle id io.github.archanagaur175-glitch.bytemap;
frontendDist "../dist" (dist/ is gitignored but must exist before tauri build); placeholder icon
generated from app-icon/icon-source.png (not a brand asset). Scan deps: jwalk 0.9.0, windows-sys 0.61.2,
thiserror 2.0.21. Gotchas learned: [target.'cfg(windows)'.dependencies] (not [target.windows]);
jwalk: skip_hidden(false) required, failed child read_dir attaches to read_children.error (pub), parents
before children guaranteed (strict DFS), metadata() = symlink_metadata (no follow); denied-dir probe:
icacls /deny user:(OI)(CI)R + self-verify read_dir fails. Tauri 2 gotchas: Channel::send takes
InvokeResponseBody (wrap with ::Json(string)); Channel::new(handler) invokes the closure directly on
send (works webview-free in tests); frontend Channel API = property assignment (channel.onmessage = fn),
NOT .onmessage(fn); serde_json cannot serialize u128 (cast elapsed to u64); JS camelCase args
(onEvent, nodeId) map to Rust snake_case; node_view returns ArrayBuffer for Raw responses.

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
  Rust: tauri, tauri-build, serde, serde_json (src-tauri only). zustand approved + installed in Phase 2.
- Check: scripts/check.ps1 (fmt, check, clippy -D warnings, test, npm build, vitest).
- Open item: Q4 = P6.T4 whole-volume speed gate (owner elevated run) — still unmeasured, see OPEN_QUESTIONS.md. Q6 = P8.T3 USN refresh runtime proof (owner elevated run) — code proven by tests, no journal read yet. P6.T0 signing decision CLOSED (deferred to P9.T3).
- Windows 11 is the target; Win10 = best-effort flat fallback, untested.
