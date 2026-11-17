# Open Questions (for the owner — answered questions move to DECISIONS.md)

- Q2 (since 2026-11-15, APPROVED 2026-11-17 — in progress): P1.T7 MFT spike. Throwaway read-only ntfs-reader probe at crates/bytemap-helper/examples/mft_probe.rs, compiled by the agent, RUN BY THE OWNER with elevation. Report entry count + elapsed.
- Q3 (since 2026-11-17, P5.T6): archive mode currently refuses DIRECTORIES (files only). Reason: a folder's tier does not describe its contents — archiving+removing a folder could silently take Protected-named content inside it. Options: (a) keep files-only for Phase 5 (current, fail-closed); (b) expand folders at archive time into their contained files (recursion + empty-dir removal after verify). Which do you want?
