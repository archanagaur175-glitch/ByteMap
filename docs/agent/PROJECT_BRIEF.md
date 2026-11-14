# PROJECT BRIEF (verbatim, owner-approved)

Product: ByteMap scans a Windows drive or folder extremely fast, shows storage as an immersive, navigable visual model, classifies files by meaning, and lets the user clean up safely.

Goals: (1) blazing-fast multi-threaded scanning; (2) immersive, creative UI, not a flat grid; (3) smart categorization; (4) safe cleanup; (5) robust saved state so no context is lost.

Stack decisions (change only with owner approval):
- Shell: Tauri 2. Backend: Rust workspace with crates bytemap-core (arena tree, roll-ups, categorizer, layout), bytemap-scan (scanner tiers), bytemap-actions (dry run, recycle, archive, undo journal), and src-tauri (commands and IPC).
- Frontend: React 19, TypeScript, Vite, Zustand, Vitest. Canvas 2D renderer behind a Renderer interface (WebGL2 only if profiling proves it necessary).
- Approved Rust dependencies: tauri 2, serde, jwalk, rayon, crossbeam-channel, parking_lot, thiserror, tracing, bytemuck, trash, windows (or windows-sys), bincode, zstd, ntfs-reader and usn-journal-rs (Phase 6 only), window-vibrancy (Phase 7 only). Verify current versions before adding.

Scanner design:
- Tier A: direct NTFS MFT read, local NTFS only, needs elevation. Run it in a separate elevated helper process talking over a named pipe; the main app never runs as administrator. ReFS has no MFT.
- Tier B: parallel walker (jwalk + rayon), works without admin.
- Tier C: network and removable paths, conservative concurrency.
- Rules for all tiers: do not follow junctions, symlinks or reparse points; count hardlinks once by file ID; store logical size and size on disk; never hydrate OneDrive placeholders; support long paths; count and report access-denied entries instead of aborting.

Memory and IPC design:
- Flat arena Vec<Node> with u32 indices and a shared string arena. Target at most 80 bytes per entry. This is a target to measure, not a fact.
- Rust owns the tree. The UI never receives the whole tree. It requests views (top N children of a node by size, rest folded into one "N small items" entry, layout computed in Rust).
- Views travel as packed binary via tauri::ipc::Response. Scan progress travels via Channel, batched to at most 20 updates per second. Never emit one event per file.
- The UI refers to items by node ID. Destructive commands take node IDs and resolve paths in Rust. The frontend can never send an arbitrary path to delete.

Categorization: data-driven rules (embedded TOML), evaluated as path rules, then extension rules, then size/age heuristics. Categories: Code and build artifacts (node_modules, target, .gradle, __pycache__, .venv, dist), System bloat, Archives, Media, Documents, Apps and games, Other. Safety tiers: Safe, Review, Protected. Never offer a delete action for pagefile.sys, hiberfil.sys, System Volume Information or the Windows folder. Documents are excluded from bulk actions by default.

Action Hub safety: basket, then a dry run listing every path, size, tier and in-use status (Restart Manager); Recycle Bin by default; permanent delete only with typed confirmation and never for Protected items; before executing, re-check size and modified time against the scan and drop changed items (stale-scan guard); append-only undo journal; archive mode compresses, verifies, then removes the original.

UI design: Orbit view (sunburst: center is the current folder, rings are depth, arc angle is size) is the default; Strata (layered treemap) and Nebula (bubble map of top 200 items) follow. Windows 11 look: Mica backdrop, glass panels (blur 24px), Segoe UI Variable and Cascadia Mono, 12px radius, dark and light themes, 180ms standard and 320ms drill motion, prefers-reduced-motion respected, safety tier shown by icon and label as well as color. Dark tokens: background #0B0F1A, text #E8ECF6, accent #6EA8FF; category colors Media #FF5C8A, Archives #FFB347, Code #4FD1C5, System #A78BFA, Documents #5B9DFF, Apps #8BDD6B, Other #8892A6; tiers Safe #3DDC97, Review #FFC857, Protected #FF5D5D. Light theme: background #F3F5FA, text #121826, accent #2F6FE0; Media #E03A6B, Archives #D98A12, Code #1FA89C, System #7C5CE0, Documents #2F6FE0, Apps #4FA83A, Other #6B7488; Safe #1E9E68, Review #C48A00, Protected #D13B3B.

Performance targets (measure on real hardware, do not claim): first pixels under 500 ms after scan start; MFT tier under 5 s per million entries; walker tier under 30 s per million entries on NVMe; 60 fps with at most 4,000 drawn primitives.
