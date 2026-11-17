# Status
Phase: 6 IN PROGRESS (T0, T1 done 2026-11-17) | Task: next = P1.T7 spike file (owner runs it) + P6.T2 pipe protocol | Last completed: P6.T1
Next: (a) write the read-only MFT spike example + run instructions for the owner; (b) P6.T2 length-prefixed frames (BYM1) in bytemap-helper/src/proto.rs + client bytemap-scan/src/mft_client.rs + loopback test; helper lib tests stay UAC-free. Then T3 MFT end-to-end, T4 parity/bench, gate.
Blockers: none technical. Spike needs the OWNER to run elevated. OPEN_QUESTIONS: Q2 spike in progress; everything else closed (dedup OFF, Q3 files-only, signing deferred to P9).
P6.T1 facts: helper crate crates/bytemap-helper — lib serve() over \\.\pipe\bytemap-scan (one master at a time, PIPE_ACCESS_DUPLEX, FILE_FLAG_FIRST_PIPE_INSTANCE, nMaxInstances=1), handshake BYM1+u16 version+status byte; bin carries requireAdministrator manifest (embed-resource 3.0.12, bins-only link-arg, bin target test=false). Tests: 3 passed, clippy+fmt clean. New dep approved by owner: embed-resource 3.0.12 (build-time only).
Resume: read AGENTS.md -> docs/agent/CONTEXT_RESUME.md -> docs/agent/STATUS.md; do the "Next" actions.
