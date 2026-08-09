# Phase 4 — aether-agent + aether-tools + aether-auth + aether-tui — Ready-to-fire spec
*Blueprint §4: agent registry + tools + auth + ratatui TUI. The 4 crates that make aether feel like a product. Phase 4 is the longest (2-3 wks) — split into 4 sub-phases, each independently shippable.*

# 4.a aether-tools (bash/files/web) — port of TS tools/
Reference: /home/abdozaik720/aether-cli/src/tools/ (files.ts 173, bash.ts 59, web.ts 48).
- FileTool: read_file/write_file/list_dir/grep_files — cwd path sandboxing isInside/assertInside (port exactly).
- BashTool: /bin/bash -lc, timeout 30s/300s, 4MB cap, ALWAYS y/n confirm, DENY non-TTY (port exactly). These guards are the same jcode-like gate.
- WebTool: web_fetch GET, 15s timeout, 12k truncation.
- Security: sandbox regex→ strict canonicalization (no .., no symlink escapes), no secrets in output (redact common env keys).

# 4.b aether-agent (registry/router/executor) — port TS agents/ (4 files)
- registry.ts 167 (6 profiles: explore, secure-coder, writer, critic, planner, default), router.ts 29 (keyword scoring), executor.ts 391 (skills resolution repo + ~/.config/aether/skills/, subagents, bounded parallel pool max 4, JSON job files ~/.aether/jobs/, run background, waitForJob).
- Jobs subsystem moves here: tokio task registry + status JSON (port sync of jobs state).

# 4.c aether-auth (device flow/cache/fallback) — port TS auth/ (3 files)
- device.ts 200: OAuth 2.0 device grant RFC 8628 GitHub/Google — reqwest, token store 0600 (jcode-storage write_text_secret pattern).
- cache.ts 117: SHA-256 content-addressed token cache, TTL 7d, 500 entries/64MB LRU, reasoning-replay (nice-to-have).
- fallback.ts 91: 15 rate-limit patterns, chain request→gemini-2.5-flash→keyed providers, withFallback.

# 4.d aether-tui (ratatui full) — the visible upgrade over Ink TUI
- ratatui 0.30 + crossterm 0.29 (jcode-verified versions).
- Port tui/app.tsx 1098: full-screen chat, gradient wordmark, scrollable transcript, right sidebar live token ledger, input panel, slash-command completion, **Ctrl+P history palette**, **Alt+M model picker**, **mouse-wheel scroller** (ratatui+crossterm native; port hand-rolled X10/SGR-1006 wheel decode from tui/mouse.ts if needed).
- bridge.ts 171 pattern: emit-only, never writes stdout.
- state in core; TUI renders only (blueprint §7-risks-3 mitigation).

## Phase 4 exit gate (same for all sub-phases)
1. cargo build --release (0 errors)
2. cargo test on the sub-phase's crate (≥2 meaningful)
3. clippy -D warnings clean
4. Manual smoke: bash tool runs with confirm (TTY); agent routes a keyword; TUI renders transcript, wheel scrolls, Ctrl+P toggles palette.
5. Print: tree + `aether` (TUI) transcript + verification.

## READ FIRST (before any code)
- All 6 TS files above at /home/abdozaik720/aether-cli/src/{tools,agents,auth}/.
- jcode-provider-core trait + jcode patterns where useful (auth.rs, tools.rs).
- blueprint §4 table row per crate + §2.2 items 6,7,9,10.