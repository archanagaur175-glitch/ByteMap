# Open Questions (for the owner — answered questions move to DECISIONS.md)

- Q2 CLOSED (2026-11-17): P1.T7 spike ran by the owner — 527,391 entries/s on real C: (1.73M entries, 3.28 s scan). MFT feasible; see DECISIONS.md.
- Q4 OPEN (2026-11-18): the P6.T4 **whole-volume speed number** was never run. Parity passed exactly
  (101,112 = 101,112) and the single-pass fast path is implemented and covered by tests, but
  "MFT >= 333k entries/s on C:" has NOT been measured since the gate was adjusted. The owner approved
  Phase 7 before the run, so this is deferred, not passed. One command closes it, elevated:
  `cargo run --release -p bytemap-scan --bin bench-mft -- --whole-volume`
  Paste the `[mft]` / `[walker]` / `BENCH-MFT` lines; `speed_gate=passed` (plus `count_drift <= 1%`)
  closes Q4. Until then, no summary may claim the MFT speed gate as met.
- Q5 OPEN (2026-11-19): the brief approves **bincode** for snapshots, but the crate is unmaintained: 3.0.0 (2025-12-16) is a deliberate TOMBSTONE release whose lib.rs is only a compiler error pointing users elsewhere; the last usable version is 2.0.1. Options: pin bincode 2.0.1 (already on the approved list, works, no maintenance), hand-roll the packed LE format we already use for view/orbit/strata (no dep, full control, more code), or add a maintained crate (postcard/wincode - new dependency). Owner decision needed before P8.T1 writes the file format.
