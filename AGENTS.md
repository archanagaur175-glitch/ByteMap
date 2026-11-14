# ByteMap — Agent Rules

Native Windows 11 disk-space analyzer: Tauri 2 + Rust backend, React 19 + TypeScript frontend.
Fast scanning, immersive visual model, meaningful categorization, safe cleanup, robust saved state.

## Read order (every session, and after any context reset)
1. AGENTS.md  2. docs/agent/CONTEXT_RESUME.md  3. docs/agent/STATUS.md
Then continue from STATUS.md. Never rely on memory of prior sessions.

## Rules
1. Plan first, approval second, code third. No code changes without "APPROVED: <phase name>".
2. One phase at a time. After each: run checks, update memory files, summarize in <=10 lines, stop.
3. Memory lives on disk: the read order above. Keep STATUS.md and CONTEXT_RESUME.md current.
4. Small tasks: <=5 files each, with a verification command.
5. Never claim something works without running the check. Log command + last output lines in SESSION_LOG.md.
6. Ask before: new dependencies, architecture changes, deletions, admin-rights actions.
   Remote `origin https://github.com/archanagaur175-glitch/ByteMap` is authorized; push after each task.
7. Delete/move/archive code is highest risk: needs tests and explicit owner review before phase close.
8. Unsure -> write docs/agent/OPEN_QUESTIONS.md and ask. Do not guess.
9. Tests use temp fixture folders only. Never scan or modify real user folders unless the owner names them.
10. Local-only: no telemetry, no network calls, no accounts.

## Commands
- Check:      powershell -File scripts/check.ps1
- Rust:       cargo check --workspace
              cargo clippy --workspace --all-targets -- -D warnings
              cargo test --workspace
- Frontend:   npm run build
              npx vitest run --passWithNoTests
- Dev app:    npm run tauri dev
- Benchmark:  cargo run --release -p bytemap-scan --bin bench-walk
