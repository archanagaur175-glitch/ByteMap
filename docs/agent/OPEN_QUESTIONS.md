# Open Questions (for the owner — answered questions move to DECISIONS.md)

- Q1 (since 2026-11-15, after P1.T3b): hardlink-dedup DEFAULT — without dedup 82.0k entries/s, with dedup 38.0k entries/s (still above the 33.3k/s walker target). Which should be the default for real scans?
- Q2 (since 2026-11-15): approve P1.T7 MFT spike? It is a throwaway `bytemap-scan/examples/mft_probe.rs` (ntfs-reader, read-only) that YOU would run personally — it needs elevation. Purpose: confirm MFT feasibility before Phase 6.
- Q3 (since 2026-11-17, P5.T6): archive mode currently refuses DIRECTORIES (files only). Reason: a folder's tier does not describe its contents — archiving+removing a folder could silently take Protected-named content inside it. Options: (a) keep files-only for Phase 5 (current, fail-closed); (b) expand folders at archive time into their contained files (recursion + empty-dir removal after verify). Which do you want?
