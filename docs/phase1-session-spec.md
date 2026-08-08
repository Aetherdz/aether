# Phase 1 — aetherdz-session: Ready-to-fire spec
*Prepared by strategy manager for instant dispatch the moment Phase 0 lands. Follows blueprint §2.1 (session.ts) + §4 (workspace) + §5 (JSONL decision).*

## Goal
Sessions with auto-title, recall, and ledger — JSONL files, no SQLite.

## Reference TS (READ BEFORE CODING)
- /home/abdozaik720/aether-cli/src/session.ts (220 LOC) — the whole contract: append, auto-title, *.title sidecars, list/read/delete, recallSessions, totalsAcrossSessions.
- /home/abdozaik720/aether-cli/src/chat.ts (1129 LOC) — the session-ledger wiring (lines around session append + ledger counts).
- /home/abdozaik720/docs/rust-migration-blueprint.md §4 (workspace) + §2.2 items 2,3,8.

## Storage layout (MUST match TS so both versions coexist — blueprint §10)
- Sessions: `~/.config/aether/sessions/<session-id>.jsonl`
- Each line = one JSON object: `{"role":"user"|"assistant"|"meta","ts":<unix>,...}"` — copy the EXACT line shape from session.ts.
- Auto-title: `<id>.title` sidecar file (plaintext). TS heuristic: from first user message, trimmed ≤40 chars. NO LLM call (cheap heuristic per blueprint §5.2.2).
- Meta lines carry token usage/cost per turn (ledger).

## Crate: aetherdz-session (+ integration in aetherdz-core jobs NOT yet)
Public API (3-5 items):
- `Session::create() -> SessionId` (generate id: same scheme as TS — check session.ts exact format)
- `Session::append(role, content, usage) -> Result<()>` (atomic append via core's fs::atomic_write or append-only write)
- `Session::title() -> String` (reads/titles sidecar; auto-generates if missing)
- `Recall::search(phrase) -> Vec<Hit>` (case-insensitive keyword over all session files; port recallSessions)
- `Ledger::totals() -> Totals` (port totalsAcrossSessions: tokens in/out, cost)

## Tests (gate: `cargo test` all pass + clippy -D warnings)
1. create → append 2 lines → read back == equal
2. auto-title generated once, stable on re-read
3. recall finds a unique keyword in 1 session, not in another
4. ledger totals math correct across 2 sessions

## Integration with existing Phase 0 crates
- Uses `aetherdz-core` (Error enum, fs helpers, config for dir).
- New CLI surface (extend aetherdz-cli clap): `aether sessions list|show|delete|resume|rename` + `aether recall "<phrase>"`.
- Reuse provider from Phase 0 for an actual title if requested later — NOT now (heuristic only).

## Security gates (bugbounty-secure-coding checklist)
- No secrets logged; paths via dirs crate; atomic append (no corruption on crash); fsync on session write (jcode-storage pattern lib.rs:672-685 append_json_line_fast).
- Recall search: no shell; pure iter lines; bound result count (top 10).
- Symlink-safe: before writing/reading session file, verify it's a regular file (no symlink into unexpected location) — port jcode-storage lib.rs:405-432 pattern.

## MUST-run at end
1. `cargo build --release` (0 errors)
2. `cargo test` (≥4 tests pass, the ones above)
3. `cargo clippy -- -D warnings` (0 violations)
4. Run: `aether sessions list` on an empty dir + with a dummy session; `aether recall "x"` returns hit
5. Print: created file tree, `aether --help` under sessions, verification transcript.