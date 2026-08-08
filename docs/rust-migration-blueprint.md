# AETHER CLI → Rust Migration Blueprint
**Target: beat jcode at its weak points — not parity.**
*Evidence-first: every jcode claim cites a real path/line read from `/tmp/opencode/jcode` (clone of github.com/1jehuang/jcode, Aug 2026). Every aether-cli claim cites `/home/abdozaik720/aether-cli/`.*

---

## 1. EXECUTIVE SUMMARY

**Why Rust.** The current aether-cli is 35 TypeScript files / ~7,405 LOC on Node + ai-sdk v7 + Ink v7 (33 `.ts` = 6,258 LOC + 2 `.tsx` = 1,147 LOC). It works, but ships a ~100 MB `node_modules`, needs a runtime, and its TUI is a React reconciler over ANSI. Rust gives us: a single static binary (~15–25 MB), sub-100 ms startup, zero runtime deps, and a TUI that renders at 60 fps with native mouse support. jcode already proved the Rust CLI model at scale (83 crates, 661,340 LOC) — we do not need to prove it, we need to *beat it*.

**Market position — attack jcode's weak sides, not its strengths.** jcode is a heavyweight generalist (661,340 LOC, 982 crates in `Cargo.lock`, default features pull pdf + embeddings + bedrock). We verified four concrete weaknesses in its actual source:

| # | jcode weakness | Evidence (path:line) | Our differentiator |
|---|---|---|---|
| 1 | **MCP stdio-only** — HTTP/SSE entries are *recognized and skipped* | `README.md:526`; `crates/jcode-base/src/mcp/protocol.rs:191-192, 204-207, 227-238, 667-676` | MCP client with **stdio + Streamable HTTP + legacy SSE** transports |
| 2 | **Zero cross-device session sync** — storage crate has no export/import/gist/dropbox backend | `crates/jcode-storage/src/` contains only `lib.rs` (770 LOC) + `active_pids.rs` (393 LOC); grep for `gist`/`dropbox` across `crates/` → zero hits | **`aether sync`** (gist/folder + line-level merge) as flagship Rust feature |
| 3 | **Telemetry worker ships in the prod workspace** while docs claim "anonymous, minimal" | `telemetry-worker/` (Cloudflare Worker: D1 + Analytics Engine firehose, 4.5 GB soft limit, emergency prune); `jcode-telemetry-core` in workspace `Cargo.toml:90`; `TELEMETRY.md:3` vs `:5` | **Zero telemetry code.** No worker, no events, no opt-out needed — there is nothing to opt out of |
| 4 | **Massive footprint** | 661,340 LOC total; `Cargo.toml:213` `default = ["pdf", "embeddings", "bedrock"]`; 982 crates in lockfile | **Lean workspace**: ~24 crates, feature-gated heavy deps, `cargo build` in seconds not minutes |

**Strategy.** Port the *features* of aether-cli, borrow the *patterns* of jcode's small focused crates (MIT — legally reusable), and reject jcode's heavy crates outright. Ship a binary that does `aether chat` with 8+ providers, sessions, recall, sync, and MCP — in a fraction of the footprint.

---

## 2. AETHER-CLI FEATURE INVENTORY (current TS → Rust porting notes)

*Source: `/home/abdozaik720/aether-cli/src/**` — 33 `.ts` files (6,258 LOC) + 2 `.tsx` files (`tui/app.tsx` 1,098, `tui/index.tsx` 49) = 35 files, ~7,405 LOC. `cli.ts` imports `./tui/index.js` so the `.tsx` files are load-bearing.*

### 2.1 File inventory (all 35 files)

**Root `src/` (14 files):**

| File | LOC | Role | Rust port |
|---|---|---|---|
| `index.ts` | 9 | entry: dotenv + `cli(argv)` | `aetherdz-cli` main.rs |
| `cli.ts` | 695 | **Commander CLI — all commands** (ask/use/models/providers/swarm/agent/agents/jobs/sessions/doctor/upgrade/completions/alias/config/connect/login/logout/keys/login-device/logout-device/sync/cache/status/cost/stats/mcp/recall); TTY→TUI vs REPL | clap derive |
| `features.ts` | 666 | Admin set: doctor, upgrade, completions, aliases, config get/set, login/logout (.env 0600), keys, status, cost, device-login orchestration, cache stats/clear, interactive `connect` | `aetherdz-cli` admin module |
| `chat.ts` | 1129 | **Core REPL + streaming pipeline**: `streamTurn` (ai-sdk `streamText`, `stopWhen: isStepCount(8)`), token-cache, rate-limit fallback, zen reasoning-replay, `chatRepl` with 20+ slash commands, Esc/Ctrl+C abort, session ledger | `aetherdz-cli` repl + `aetherdz-core` stream |
| `session.ts` | 220 | JSONL session store: append user/assistant/meta, auto-title, `*.title` sidecars, list/read/delete, **`recallSessions`** cross-session search, `totalsAcrossSessions` | `aetherdz-session` |
| `sync.ts` | 294 | **Sync engine (v0.5 flagship)**: gist (GitHub API) or folder backend, `sync.json` state, device-id, bundle v1 `{v, deviceId, sessions}`, timestamp-sorted line de-dup merge (`mergeLines`), `mergeBundles`, write-new + merge-existing | `aetherdz-sync` (flagship) |
| `mcp.ts` | 273 | **Lean stdio JSON-RPC MCP client (proto 2024-11-05), zero deps**: `~/.config/aether/mcp.json`, spawn/handshake/tools/list/call, 8s timeout, allow-list, `buildMcpTools` → ai-sdk `tool()` | `aetherdz-mcp` (stdio+http+sse) |
| `config.ts` | 125 | Config `~/.config/aether/config.json`, defaults merge, legacy-shape normalization, `updateDefault`, self-healing normalize | `aetherdz-config` (toml) |
| `render.ts` | 138 | Box-drawing `box()`, wrap, divider, REPL header, promptTag, statusIndicator, usageSummary | ratatui widgets |
| `prompt.ts` | 6 | Shared `SYSTEM_PROMPT` | `aetherdz-core` prompt |
| `defaults.ts` | 2 | `DEFAULT_PROVIDER="zen"`, `DEFAULT_MODEL="deepseek-v4-flash-free"` | `aetherdz-provider` consts |
| `version.ts` | 1 | `VERSION="0.5.0"` | `env!("CARGO_PKG_VERSION")` |
| `package-meta.ts` | 27 | Reads package.json from disk | `env!` |
| `fs-util.ts` | 5 | `ensureDir` | `aetherdz-core` fs |

**`agents/` (4):**

| File | LOC | Role | Rust port |
|---|---|---|---|
| `index.ts` | 103 | `runOne`, `runTask` (route → planner split → parallel/single → summary), `printAgents` | `aetherdz-agent` |
| `registry.ts` | 167 | **6 agent profiles**: explore, secure-coder, writer, critic (read-only), planner, default | `aetherdz-agent` |
| `router.ts` | 29 | Keyword-scoring router (hashtag count, forced id wins) | `aetherdz-agent` |
| `executor.ts` | 391 | Agent runner + **jobs subsystem**: `resolveSkills` (repo + `~/.config/aether/skills/`), `runSubAgent`, `runSubAgentsParallel` (bounded pool, max 4), JSON job files in `~/.aether/jobs/`, `runAgentBackground`, `waitForJob` | `aetherdz-agent` + `aetherdz-core` jobs |

**`auth/` (3):** `cache.ts` 117 (SHA-256 content-addressed token cache, TTL 7d, 500 entries/64MB LRU, reasoning replay), `fallback.ts` 91 (15 rate-limit patterns, chain ollama→gemini-2.5-flash→keyed providers, `withFallback`), `device.ts` 200 (OAuth 2.0 device grant RFC 8628 for GitHub/Google, token store 0600) → `aetherdz-auth`.

**`providers/` (2):** `models.ts` 152 (static model lists for 19 providers, `ZEN_FREE_MODELS` 9 no-key, `fetchZenModels` live fetch + 10-min cache), `registry.ts` 485 (19 built-in providers via ai-sdk factories: zen/openai/anthropic/google/deepseek/openrouter/ollama/groq/mistral/xai/cerebras/togetherai/fireworks/perplexity/moonshot/minimax/huggingface/lmstudio/github + customs; `resolveDefault` graceful fallbacks) → `aetherdz-provider`.

**`swarm/` (2):** `orchestrator.ts` 154 (`planTask` JSON subtask plan, ≤4 subtasks, parallel bounded workers), `worktree.ts` 94 (git worktree isolation, temp-dir `aether-worktrees`) → `aetherdz-swarm` (Phase 5).

**`tools/` (3):** `files.ts` 173 (`read_file`/`write_file`/`list_dir`/`grep_files`, **cwd path sandboxing** `isInside`/`assertInside`), `bash.ts` 59 (`run_bash` `/bin/bash -lc`, timeout 30s/300s, 4MB cap, **always y/n confirm, denied non-TTY**), `web.ts` 48 (`web_fetch` GET, 15s timeout, 12k truncation) → `aetherdz-tools`.

**`tui/` (5):** `index.tsx` 49 (Ink render entry), `app.tsx` 1098 (**full-screen Ink chat UI**: gradient wordmark, scrollable transcript, right sidebar ≥118 cols with live token ledger, input panel, slash commands w/ prefix completion, **Ctrl+P history palette**, **Alt+M model picker**, **hand-rolled mouse-wheel scroller**), `bridge.ts` 171 (shared streaming bridge, `emit: (TuiEvent)` callback, never writes stdout), `text.ts` 46 (newline-preserving wrap + ANSI-aware clip), `mouse.ts` 75 (hand-rolled X10/SGR-1006 wheel decoding) → `aetherdz-tui` (ratatui).

**`ui/` (2):** `theme.ts` 75 (brand palette: bgNavy `#0b1120`, cyan `#22d3ee`, amber/green/red/gray; ANSI helpers), `banner.ts` 38 (violet→cyan gradient wordmark) → `aetherdz-tui` style module.

### 2.2 Feature-by-feature port notes

1. **8 providers via @ai-sdk** (openai, openai-compatible, anthropic, google, ollama + 14 more) → one `aetherdz-provider` crate, feature-gated per provider, all speaking OpenAI-compatible chat-completions wire format + SSE streaming. Port the ai-sdk *abstraction* (provider trait + stream adapter), not the SDK. Copy jcode's `jcode-provider-core` trait pattern (see §3.2).
2. **Sessions w/ auto-title** → `aetherdz-session`: JSONL per session (see §5 storage decision), auto-title from first message (cheap heuristic, no LLM call for v1).
3. **Recall cross-session memory** → `aetherdz-session` recall module: case-insensitive keyword search over session JSONL (port `recallSessions` semantics); optional embeddings behind `memory` feature (jcode-embedding pattern, gated).
4. **Sync push/pull via gist/folder with line-merge** → `aetherdz-sync` (flagship): gist backend (reqwest + GitHub API) + folder backend (plain dir), line-level 3-way merge (reuse `similar` crate like jcode does).
5. **MCP client** → `aetherdz-mcp`: stdio (tokio child process) + Streamable HTTP (rmcp) + legacy SSE (thin hand-rolled transport). See §5.
6. **Agents registry** → `aetherdz-agent`: registry + router + executor. Port the TS registry semantics (6 profiles, keyword routing, skills injection).
7. **Jobs** → `aetherdz-core` jobs module: tokio task registry with JSON status files (port `runAgentBackground`/`waitForJob`).
8. **Stats/ledger** → `aetherdz-session` stats module: token/usage ledger per session, JSONL meta lines (port `totalsAcrossSessions`).
9. **readline REPL + Ink TUI** → `aetherdz-cli` repl (rustyline) + `aetherdz-tui` (ratatui 0.30 + crossterm 0.29, matching jcode's stack). Mouse wheel + ctrl+p palette overlay per user's prior wishes.
10. **Auth (device flow, cache, fallback)** → `aetherdz-auth`: RFC 8628 device flow via reqwest, token cache with 0600 perms (port jcode-storage's `write_text_secret` pattern), rate-limit fallback chain.
11. **Zen live models** → `aetherdz-provider`: `fetchZenModels` live fetch + static fallback + free/paid split.
12. **Swarm + worktree** → `aetherdz-swarm` (Phase 5): plan→parallel subagents, git worktree isolation.

---

## 3. JCODE WEAK-SIDE CRATE MAP (what to study, what to reject)

*Full 83-crate classification below (verified: 82 workspace members + `jcode-math` which is NOT a member — standalone, pulled only by `jcode-desktop2`).*

### 3.1 Classified crate map (all 83)

**COPY-PATTERN (small, focused, MIT — study and adapt):**
`jcode-storage` (1,110 LOC — atomic writes, .bak recovery, secret hardening), `jcode-fuzzy` (745, pure-std DP matcher), `jcode-transport` (270, Unix socket/named pipe IPC), `jcode-message-types` (896), `jcode-session-types` (1,066), `jcode-tool-core` (286) + `jcode-tool-types` (147), `jcode-core` (2,183, leaf helpers), `jcode-logging` (1,182), `jcode-auth-types` (180), `jcode-ambient-types` (32), `jcode-background-types` (153), `jcode-batch-types` (32), `jcode-gateway-types` (19), `jcode-memory-types` (1,985), `jcode-selfdev-types` (191), `jcode-side-panel-types` (93), `jcode-usage-types` (949), `jcode-provider-env` (408), `jcode-provider-metadata` (1,868), `jcode-tui-anim` (930, pure math kernels), `jcode-agent-runtime` (283, soft-interrupt/shutdown channels).

**PORT (medium, useful logic worth porting):**
`jcode-provider-core` (7,319 — **the provider trait pattern**: `Provider` trait with defaulted introspection + `fork()` + structured `RouteSelection`/`RuntimeKey`; copy the wiring, not all 7k lines), `jcode-provider-openai` (2,027) + `jcode-provider-openai-runtime` (8,352 — the OpenAI-compatible runtime template), `jcode-provider-anthropic` (1,606), `jcode-provider-openrouter` (2,454) + `-runtime` (8,026), `jcode-command-risk` (1,812, bash tool gate), `jcode-compaction-core` (1,036), `jcode-plan` (5,475), `jcode-protocol` (5,141), `jcode-harness-api` (1,552) + `-server` (4,032), `jcode-schema-dialect` (2,492), `jcode-import-core` (2,421), `jcode-render-core` (3,768), `jcode-sdk` (3,226), `jcode-update-core` (622), `jcode-config-types` (2,496), `jcode-core` (2,183), `jcode-build-meta` (138), `jcode-build-support` (3,228), `jcode-notify-email` (529), `jcode-overnight-core` (1,471), `jcode-productivity-core` (1,716), `jcode-swarm-core` (837), `jcode-plan` (5,475), `jcode-telemetry-core` (4,496 — **study only to know what NOT to build**), `jcode-terminal-image` (717), `jcode-terminal-launch` (1,530), `jcode-provider-doctor` (6,743).

**REJECT (heavy — do NOT pull in):**
`jcode-app-core` (133,524 — the monolith), `jcode-base` (106,246 — the monolith), `jcode-tui` (131,202 — the TUI monolith), `jcode-desktop2` (39,716 — winit+wgpu+Vello+Parley desktop GUI), `jcode-tui-markdown` (9,296), `jcode-tui-mermaid` (9,876), `jcode-tui-render` (4,654), `jcode-tui-core` (3,208), `jcode-tui-style` (1,200), `jcode-tui-messages` (1,135), `jcode-tui-permissions` (866), `jcode-tui-usage-overlay` (926), `jcode-tui-visual-debug` (857), `jcode-tui-workspace` (1,228), `jcode-tui-account-picker` (1,403), `jcode-tui-session-picker` (289), `jcode-tui-tool-display` (236), `jcode-embedding` (635 — tract ONNX + HF tokenizer, MiniLM), `jcode-pdf` (51 — but pulls heavy upstream), `jcode-provider-bedrock` (1,979 — AWS SDK stack), `jcode-provider-claude-cli-runtime` (1,142 — deprecated), `jcode-math` (3,416 — TeX typesetting, non-member), `jcode-setup-hints` (889 — platform hints for a non-goal UI), `jcode-azure-auth` (8 — dead weight).

### 3.2 Files to read first (before writing any Rust)

1. `crates/jcode-storage/src/lib.rs` — atomic write + .bak recovery pattern (lines 442–661), `append_json_line_fast` (672–685), symlink-safe auth file validation (405–432).
2. `crates/jcode-storage/src/active_pids.rs` — session presence pattern (lines 1–268), `StreamingGuard` RAII (89–105).
3. `crates/jcode-base/src/mcp/protocol.rs` — MCP config struct + `is_stdio()` (lines 188–249) — **this is the exact code we extend**.
4. `crates/jcode-base/src/mcp/client.rs` — stdio client handle pattern (lines 1–50).
5. `crates/jcode-provider-core/src/lib.rs` — the `Provider` trait (lines 75–487), `RouteSelection`/`RuntimeKey` (676–763), `shared_http_client` (599–643).
6. `crates/jcode-provider-core/src/transport.rs` — `is_transient_transport_error` classifier (lines 33–60).
7. `crates/jcode-fuzzy/src/lib.rs` — pure-std fuzzy matcher (745 LOC, copy wholesale).
8. `crates/jcode-tui/Cargo.toml` — dependency versions (ratatui 0.30, crossterm 0.29).

---

## 4. TARGET RUST WORKSPACE DESIGN (~24 crates)

**Naming:** `aetherdz-*` prefix (all names verified AVAILABLE on crates.io Aug 2026; `aether`, `aether-cli`, `aether-core`, `aether-tui`, `aether-sdk` are taken). Binary name stays `aether`.

```
aetherdz/                      # workspace root
├── Cargo.toml                 # workspace, shared deps, release profile
├── crates/
│   ├── aetherdz-core/         # shared types, config, prompt, jobs, fs
│   ├── aetherdz-provider/      # ONE crate, feature-gated providers
│   ├── aetherdz-session/       # sessions, auto-title, recall, stats/ledger
│   ├── aetherdz-sync/          # FLAGSHIP: gist/folder sync + line-merge
│   ├── aetherdz-mcp/           # MCP client: stdio + streamable-http + sse
│   ├── aetherdz-tools/         # bash/files/web tools
│   ├── aetherdz-agent/         # agent registry + executor + router
│   ├── aetherdz-auth/          # device flow, token cache, fallback
│   ├── aetherdz-tui/           # ratatui TUI (mouse wheel, ctrl+p palette)
│   ├── aetherdz-cli/           # clap CLI + REPL
│   ├── aetherdz-sdk/           # library API (for embedding)
│   └── aetherdz-swarm/         # (later) swarm orchestration
└── aether/                     # root binary crate (thin)
```

### Per-crate spec

| Crate | Responsibility | Key deps (version) | Public API (3–5 items) | Port vs New |
|---|---|---|---|---|
| `aetherdz-core` | shared types, prompt building, fs utils, jobs queue | serde 1, serde_json 1, anyhow 1, thiserror 2, tokio 1 | `Prompt::build()`, `Job::spawn()`, `fs::atomic_write()`, `Error` enum | Port (from TS + jcode-storage patterns) |
| `aetherdz-provider` | 8+ providers, feature-gated; OpenAI-compatible wire format; SSE streaming | reqwest 0.12 (rustls), serde, futures 0.3, tokio | `Provider::chat()`, `Provider::stream()`, `ModelCatalog::resolve()`, `ProviderRegistry::get()` | Port (from TS + jcode-provider-core pattern) |
| `aetherdz-session` | session store, auto-title, recall, stats/ledger | serde_json, chrono 0.4, dirs 5 | `Session::create()`, `Session::append()`, `Session::title()`, `Recall::search()`, `Ledger::record()` | Port (TS session.ts) |
| `aetherdz-sync` | **gist/folder sync + line-merge** | reqwest, serde_json, similar 2, git2 (folder backend) | `Sync::push()`, `Sync::pull()`, `Merge::three_way()`, `Backend::Gist/Folder` | **New (flagship)** |
| `aetherdz-mcp` | MCP client stdio+http+sse | rmcp (official), tokio, reqwest | `McpClient::connect_stdio()`, `connect_http()`, `connect_sse()`, `list_tools()`, `call_tool()` | Port+extend (jcode mcp + rmcp) |
| `aetherdz-tools` | bash/files/web tools | tokio, reqwest, `similar` | `BashTool::run()`, `FileTool::read/write()`, `WebTool::fetch()` | Port (TS tools/) |
| `aetherdz-agent` | registry, executor, router | aetherdz-core, aetherdz-provider | `Agent::register()`, `Agent::execute()`, `Router::route()` | Port (TS agents/) |
| `aetherdz-auth` | device flow, cache, fallback | reqwest, serde | `Auth::device_flow()`, `Auth::cached_token()`, `Auth::refresh()` | Port (TS auth/) |
| `aetherdz-tui` | ratatui TUI, mouse wheel, ctrl+p palette | ratatui 0.30, crossterm 0.29 | `App::run()`, `Palette::toggle()`, `MouseWheel::scroll()`, `render()` | New (from Ink) |
| `aetherdz-cli` | clap CLI, REPL | clap 4, rustyline 15 | `Cli::parse()`, `repl::run()`, `cmd::chat()`, `cmd::sync()` | Port (TS cli.ts) |
| `aetherdz-sdk` | library API | aetherdz-core | `Aether::new()`, `Aether::chat()`, `Aether::sync()` | New |
| `aetherdz-swarm` | (later) swarm | tokio | `Swarm::spawn()`, `Worktree::create()` | Port (TS swarm/) |

---

## 5. STACK CHOICES

| Concern | Choice | Version | Rationale |
|---|---|---|---|
| Async runtime | **tokio** | 1.x | jcode uses it; ecosystem standard |
| HTTP client | **reqwest** | 0.12, **rustls** (no OpenSSL) | pure-Rust TLS, static binary friendly |
| TUI | **ratatui** + **crossterm** | 0.30 / 0.29 | matches jcode exactly (`jcode-tui/Cargo.toml:59-60`); mouse wheel + palette overlay supported |
| Serialization | serde + serde_json + toml | 1.x | config in toml, sessions in jsonl |
| CLI | **clap** | 4.x derive | commander parity |
| Errors | anyhow (apps) + thiserror (libs) | 1 / 2 | jcode pattern |
| Paths | dirs | 5 | jcode uses it |
| **Session storage** | **JSONL files** (not rusqlite) | — | **Decision: JSONL.** Rationale: (a) sessions are append-only logs — JSONL is the natural fit; (b) jcode itself uses JSON files + `append_json_line_fast` (`jcode-storage/src/lib.rs:672-685`); (c) line-level sync/merge (our flagship) is trivial on JSONL, painful on SQLite; (d) no C dependency, smaller binary, no migration. rusqlite only if we later need full-text search over recall — gate behind `sqlite` feature. |
| **MCP SDK** | **rmcp** (official) | 3.x | **Recommendation: rmcp.** Evidence: official `modelcontextprotocol/rust-sdk`, ~4.7M downloads (dominant, 1–2 orders of magnitude ahead of alternatives per crates.io ecosystem analysis Mar 2026); supports stdio + Streamable HTTP client (reqwest) + server (Tower); SSE parsing built-in behind `client-side-sse` feature. **Gap:** rmcp deliberately does NOT ship the legacy 2024-11-05 two-endpoint HTTP+SSE transport. For legacy SSE-only servers, add a thin hand-rolled SSE transport (or `rust-mcp-sdk`'s SSE feature, 85K downloads, 100% conformance). jcode hand-rolls its MCP client (no SDK crate in `jcode-base/Cargo.toml`) — we do NOT copy that; we use rmcp and keep a small adapter. |

---

## 6. PHASED PLAN

| Phase | Goal | Crates touched | Exit criteria | Effort |
|---|---|---|---|---|
| **0** | Scaffold workspace + core + openai-compatible provider + minimal TUI (`aether chat` parity) | root, core, provider, cli, tui | `cargo build` clean; `aether chat` streams a response; 1 test | 1–2 wks |
| **1** | Sessions, auto-title, recall, stats/ledger | session, core | `aether session list`; auto-title on first message; recall search returns hits; 2 tests | 1–2 wks |
| **2** | **Sync flagship** (gist/folder + line-merge) | sync, session, cli | `aether sync push/pull` round-trips a session across two dirs; merge test | 1–2 wks |
| **3** | MCP stdio → HTTP/SSE | mcp, core | stdio server works; streamable-http server works; legacy SSE server works; 3 tests | 1–2 wks |
| **4** | Agents, tools (bash/files/web), auth | agent, tools, auth | agent registry routes; bash tool runs; device flow caches token; 2 tests | 1–2 wks |
| **5** | jcode-competitive extras: memory embeddings (gated), swarm, sdk | embedding (gated), swarm, sdk | `memory` feature builds; swarm orchestrates 2 agents; sdk embeds in a test | 2–3 wks |

---

## 7. RISKS

1. **Rust learning curve** — team is TS-first. Mitigate: Phase 0 is deliberately small; borrow jcode patterns (they're readable, MIT).
2. **SSE streaming parse errors** — malformed `data:` frames, partial chunks, provider quirks. Mitigate: reuse rmcp's battle-tested SSE parser for MCP; write a small, well-tested SSE parser for provider streams (jcode's provider-runtime already solved this — copy).
3. **TUI parity vs Ink** — Ink's React model vs ratatui's immediate-mode. Mitigate: keep TUI thin (render-only), all state in core; mouse wheel + ctrl+p palette are crossterm-native.
4. **Windows support** — crossterm handles it; jcode's `jcode-storage` shows the Windows hardening pattern (ACL worker, `lib.rs:282-398`). Test on Windows CI early.
5. **The 6,258-LOC TS history** — porting is not a rewrite; every feature must be re-verified. Mitigate: keep the TS version as the reference oracle; write a golden-file test harness that compares Rust output vs TS output for chat/session/sync.

---

## 8. SIZING

**Realistic timeline: 8–12 weeks** for full parity + differentiators (1 dev, part-time on Phase 5).

| Phase | Weeks |
|---|---|
| 0 | 1–2 |
| 1 | 1–2 |
| 2 | 1–2 |
| 3 | 1–2 |
| 4 | 1–2 |
| 5 | 2–3 |

**Buy vs build:**
- **BUY (reuse, MIT):** jcode crates are MIT (`LICENSE`). Reuse patterns from `jcode-storage`, `jcode-fuzzy`, `jcode-provider-core`, `jcode-base/src/mcp`. Also `rmcp` (official MCP SDK), `ratatui`, `crossterm`, `reqwest`, `similar`.
- **BUILD (our IP):** `aetherdz-sync` (line-merge sync — jcode has nothing), the provider registry (8+ providers), the TUI palette overlay, the SDK.

---

## 9. PRIORITY RANKING — 5 highest-ROI first targets

1. **`aetherdz-sync` (gist/folder + line-merge)** — jcode has ZERO cross-device sync; this is our flagship differentiator and the #1 reason a user switches. Highest ROI.
2. **`aetherdz-mcp` with stdio + Streamable HTTP + SSE** — jcode explicitly skips HTTP/SSE (`protocol.rs:667-676`); the MCP industry is moving to Streamable HTTP. Second-highest ROI.
3. **`aetherdz-provider` (8+ providers, feature-gated)** — the core value; without it nothing else matters. Third.
4. **`aetherdz-session` (auto-title, recall, stats)** — the daily-driver UX; jcode has sessions but no cross-device story. Fourth.
5. **`aetherdz-tui` (ratatui, mouse wheel, ctrl+p palette)** — the visible differentiator; Ink parity is a known weak spot. Fifth.

---

## 10. CLI COMMAND SURFACE (from aether-cli README.md — the parity target)

Verified from `/home/abdozaik720/aether-cli/README.md` (lines 127–164). Every command below must exist in the Rust CLI:

| Command | Purpose | Rust crate |
|---|---|---|
| `aether ask "<q>"` | one-shot streamed answer | cli |
| `aether chat` | REPL | cli |
| `aether use <provider>[/<model>]` | set default provider/model | cli+config |
| `aether models [provider]` | list models (`zen` fetches live list) | provider |
| `aether providers` | list providers + key status | provider |
| `aether swarm "<task>"` | parallel subagents | swarm |
| `aether agent "<task>"` / `aether agents` | route to subagent profile / list | agent |
| `aether jobs [id]` | background job status | core |
| `aether sync setup gist\|folder` / `push` / `pull` / `status` | **flagship sync** | sync |
| `aether doctor` / `status` / `cost` / `stats` / `keys` | diagnostics + ledger | core+session |
| `aether connect` / `login` / `logout` / `login-device` / `logout-device` | auth | auth |
| `aether cache stats\|clear` | token-efficiency cache | core |
| `aether config [path\|get\|set\|providers]` | config | config |
| `aether alias` / `completions` | aliases + shell completions | cli |
| `aether sessions list\|show\|delete\|resume\|rename` | sessions | session |
| `aether mcp` | list MCP servers | mcp |
| `aether recall "<phrase>"` | cross-session memory | session |
| `aether upgrade` | self-update | cli (later) |

**Storage layout to preserve** (from README:39-42): sessions as JSONL under `~/.config/aether/sessions/`, keys in `~/.config/aether/.env` (0600), MCP config at `~/.config/aether/mcp.json`. Rust must read/write the SAME layout so the TS and Rust versions can coexist during migration.

---

## 11. DISTRIBUTION (jcode's npm-native pattern, adapted)

jcode ships native binaries via npm (`sdk/npm/{darwin,linux,win32}-{arm64,x64}`) + a TS SDK (`sdk/typescript/`). For aetherdz:
- **Primary:** `cargo install aetherdz-cli` + GitHub Releases (prebuilt binaries per platform).
- **Parity:** npm package `aetherdz` that downloads the right native binary (keeps `npm i -g aetherdz` working for existing users).
- **SDK:** `aetherdz-sdk` crate for embedding; TS SDK later if demand.

---

## 12. EVIDENCE APPENDIX (all jcode claims, with paths)

| Claim | Evidence |
|---|---|
| MCP stdio-only | `README.md:526`; `crates/jcode-base/src/mcp/protocol.rs:191-192` ("Empty for HTTP/SSE servers, which jcode does not yet support"), `:204-207` (transport "used only to recognize and skip non-stdio servers"), `:227-238` (`is_stdio()`), `:667-676` ("Skipping non-stdio server ... HTTP/SSE transports are not yet supported") |
| No cross-device sync | `crates/jcode-storage/src/` = only `lib.rs` (770 LOC) + `active_pids.rs` (393 LOC); grep for `gist`/`dropbox` across `crates/` → zero hits (only schema-dialect "sync" of schemas) |
| Telemetry in prod workspace | `telemetry-worker/` (Cloudflare Worker: `wrangler.toml`, `schema.sql`, `users.sql`, `dau.sql`, `geo.sql`, `health.sql`, `token-value.sql`); `jcode-telemetry-core` in workspace `Cargo.toml:90`; `TELEMETRY.md:3` ("anonymous, minimal usage statistics") vs `TELEMETRY.md:5` (collects onboarding steps, feedback, session/workflow/tool-category summaries, per-turn timing, todo progress) |
| Footprint | 661,340 LOC across `crates/`; 982 crates in `Cargo.lock`; `Cargo.toml:213` `default = ["pdf", "embeddings", "bedrock"]`; TODO count in `.rs` files: 202 |
| TUI stack | `crates/jcode-tui/Cargo.toml:59-60` ratatui 0.30, crossterm 0.29 (event-stream) |
| MIT license | `LICENSE` (MIT, Copyright 2025 Jeremy Huang) |
| MCP hand-rolled | `crates/jcode-base/Cargo.toml` has no rmcp/mcp-sdk dep; `client.rs` hand-rolls JSON-RPC over tokio child process |
| Storage patterns worth copying | `crates/jcode-storage/src/lib.rs:442-601` (atomic write + .bak + fsync), `:672-685` (`append_json_line_fast`), `:405-432` (symlink-safe external auth file validation) |
| Session presence pattern | `crates/jcode-storage/src/active_pids.rs:89-105` (`StreamingGuard` RAII), `:186-239` (`session_presence`) |

---

*Blueprint v1.0 — generated Aug 2026. All jcode citations verified against the local clone at `/tmp/opencode/jcode`. All aether-cli claims verified against `/home/abdozaik720/aether-cli/README.md` and `src/`.*

---

## 13. NAMING DECISION (user-confirmed, Aug 2026)

**"AETHER وحده"** — the product name is **aether** (single word). No rebrand.
- Binary name: `aether` (already in Phase 0).
- CLI identity / website / docs: `aether` — keep existing branding, do NOT invent a new name.
- Internal crates may use `aetherdz-*` prefix (crates.io availability) — invisible to users.
- Website (aether-site): keep name, optionally refine design later — NOT a rename task.
