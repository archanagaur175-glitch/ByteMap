# Open Questions (for the owner — answered questions move to DECISIONS.md)

- Q2 CLOSED (2026-11-17): P1.T7 spike ran by the owner — 527,391 entries/s on real C: (1.73M entries, 3.28 s scan). MFT feasible; see DECISIONS.md.
- Q4 OPEN (2026-11-18): the P6.T4 **whole-volume speed number** was never run. Parity passed exactly
  (101,112 = 101,112) and the single-pass fast path is implemented and covered by tests, but
  "MFT >= 333k entries/s on C:" has NOT been measured since the gate was adjusted. The owner approved
  Phase 7 before the run, so this is deferred, not passed. One command closes it, elevated:
  `cargo run --release -p bytemap-scan --bin bench-mft -- --whole-volume`
  Paste the `[mft]` / `[walker]` / `BENCH-MFT` lines; `speed_gate=passed` (plus `count_drift <= 1%`)
  closes Q4. Until then, no summary may claim the MFT speed gate as met.
- Q5 CLOSED (2026-11-19): snapshot codec = **hand-rolled versioned packed LE** (u16 version header, string
  arena, node arena, category map, scan metadata) compressed with **zstd** (already a workspace dep, used by
  bytemap-actions), same style as the view/orbit/strata codecs. bincode is NOT used: 3.0.0 (2025-12-16) is a
  TOMBSTONE release (lib.rs is only a compiler error) and 2.0.1 is abandoned. No new dependency. See DECISIONS.md.
- Q6 OPEN (2026-11-25): P8.T3 **USN refresh has no runtime proof yet** — the helper's journal read needs
  Administrator, so no unit test can exercise it. Everything else is proven (core delta 14 tests, protocol
  round-trips, helper gates, workspace tests, gate green), so this is a verification gap, not a known bug.
  Owner steps (all on a real drive, elevation accepted at spawn):
  1. `npm run tauri dev`, scan **C:\\** (whole volume, MFT tier), then Snapshots -> Save current.
  2. In `%TEMP%`, create a file, write a little, delete another, and rename a FILE (not a folder).
  3. Snapshots -> **Refresh** on the row you just saved. Expect a notice like
     `Refreshed -> bytemap-<new>.bms · N added, M removed, K updated`, a NEW row in the list, the refreshed
     tree loaded, and Compare (old vs new) showing exactly those paths. Paste the notice line.
  4. Failure path (also wanted): rename a FOLDER on C:, then Refresh the newer snapshot -> it must refuse with
     "a folder was renamed or moved since this snapshot; run a full scan" and leave the file untouched.
  Paste the notice / error string; that closes Q6. Until then no summary may claim refresh "works on a real
  volume" — only "proven by tests, pending an elevated run".
- Q7 CLOSED (2026-11-28): P9.T5's written gate asked for a fresh Win11 VM, but the owner chose a current-machine test and
  then accepted that evidence ("accept this current-machine evidence and close P9.T5/Phase 9"). The cycle PASSED: silent
  install exit 0, no UAC; files + HKCU key + shortcuts correct; app launched as `ByteMap` and closed cleanly; silent uninstall
  removed key/files/shortcuts; a dummy snapshot survived uninstall; no leftover processes. The fresh-VM run is explicitly
  WAIVED, not met — do not describe a fresh-VM gate as passed. P9.T5 and Phase 9 are closed on this basis.
- Q8 CLOSED (2026-11-28, option B): the install dir is now separate from the app data dir. Owner chose B, then approved
  the implementation. `src-tauri/nsis/installer.nsi` is a copy of the Tauri CLI v2.12.4 template (pinned raw file, SHA256
  `8AE02C66F8ADB4A0D15D97F984FBACD2B44262D8A4D03FEFAF6177F67F7008E5`) with a header note and ONE functional change:
  `StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"` instead of `$LOCALAPPDATA\${PRODUCTNAME}`; wired via
  `bundle.windows.nsis.template` = `nsis/installer.nsi`. Verified: install -> `%LOCALAPPDATA%\Programs\ByteMap`
  (exes + uninstall.exe), HKCU `InstallLocation` and both shortcuts point there, app launches as `ByteMap`; reinstall
  reuses the same folder; uninstall removes the whole program folder, HKCU key and shortcuts while `%LOCALAPPDATA%\ByteMap`
  snapshots/exports survive. New installer 2,528,779 bytes SHA256 `00CB36997B4624452363F8A4E8752D0D385C51FFB55373D6B71FFF5CB3FF859E`.
  Known leftovers (documented, not fixed): the uninstaller's "delete app data" checkbox still targets the bundle-id folder,
  not `%LOCALAPPDATA%\ByteMap`; `HKCU\Software\ByteMap\ByteMap` (previous install location) is only removed when that
  checkbox is ticked. The template must be re-synced on any Tauri CLI upgrade. See DECISIONS.md.
