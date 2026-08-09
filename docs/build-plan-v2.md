# BUILD PLAN v2 — merged with studied reference projects (Aug 2026)
*Supersedes §6 phased plan internals: same phases, but every phase now names the EXACT reference files to read/copy (cloned at `/tmp/opencode/inspiration/`). Keeps our trait API (jcode-provider-core shape); steals battle-tested internals from MIT projects.*

## Pattern book (from studying aichat / agentive / agent-code)

| Pattern | Source file | What we adopt | Phase |
|---|---|---|---|
| **One generic OpenAI-compatible client** (config: base_url+models+api_key), NOT one client per provider | `aichat/src/client/openai_compatible.rs` (162 LOC) | `aether-provider::providers::compatible` — 8+ providers become config rows, not code | 0 |
| **reasoning_content in compatible client** (aichat gap: only openai.rs:138 has it) | `aichat/src/client/openai.rs:138,358` + our verified zen/deepseek streams | `StreamEvent::Reasoning` handled in compatible path — our edge over aichat | 0 |
| **SSE event framing** — chunk assembly, finish_reason, per-provider stream events | `aichat/src/client/stream.rs` (296 ln) | `aether-provider::stream::SseParser` — unit-tested framing | 0 |
| **Agent loop events** — RunnerEvent enum (ToolResult/Status/MessagesUpdated/Done/Error) | `agentive/lib/src/runner.rs` | Maps 1:1 to TuiEvent bridge (TS tui/bridge.ts) | 4 |
| **Context trimming/summarization** — TS never did this; beats old TS aether | `agentive/lib/src/context.rs` | `aether-core::context` behind `memory` feature (gated) | 4/5 |
| **CancellationToken** — matches TS Esc/Ctrl+C abort | `agentive/lib/src/cancel.rs` (tokio_util) | `aether-core::cancel` | 0 (chat abort) |
| **Tool-call accumulation across SSE chunks** — partial JSON args | `agentive/lib/src/providers/sse.rs` + `agent-code/crates/lib/src/llm/` | `aether-provider::tool_acc` | 4 |
| **Path sandbox + permission allow/deny** — for our tools/isInside+assertInside | `agent-code/crates/lib/src/sandbox/` + `permissions/` | `aether-tools::sandbox` (canonicalize, deny .., symlink-safe) | 4 |
| **Skills resolution** — repo + ~/.config/aether/skills/ | `agent-code/crates/lib/src/skills/` | `aether-agent::skills` | 4 |

## Phases v2 (same 6, refined internals)

### Phase 0 (1–2 wks) — scaffold + zen chat working
- DELIVERABLE: `aether chat` streams from `https://opencode.ai/zen/v1`, model `deepseek-v4-flash-free`, NO key. (verified contract in phase1-spec addendum)
- Copy pattern: aichat openai_compatible.rs + stream.rs. Implement reasoning_content in compatible client (our edge).
- Gate: build release 0 err + ≥1 test + clippy -D warnings + golden `aether --help` == TS.

### Phase 1 — sessions (JSONL, auto-title, recall, ledger) — per existing spec (no external ref; TS session.ts is the oracle)

### Phase 2 — sync (gist/folder line-merge) — FLAGSHIP — per existing spec (no external ref; jcode has zero sync — we build it)

### Phase 3 — MCP stdio+HTTP+SSE via rmcp — per existing spec + blueprint §5 rmcp decision

### Phase 4 (2–3 wks) — 4 sub-phases, each with a reference:
- **4a tools** ← agent-code sandbox/permissions (safety) — port TS files/bash/web guards
- **4b agent** ← agentive runner.rs loop + RunnerEvent → TuiBridge; skills resolution
- **4c auth** — device flow RFC 8628 + token cache 0600 (jcode-storage pattern) — no ext ref
- **4d TUI** — ratatui 0.30 (mouse wheel, ctrl+p palette) — port TS tui/app.tsx panels; state in core

### Phase 5 — extras (memory embeddings gated, swarm, sdk) — per blueprint

## Hard rules (unchanged from v1, now with sources)
1. `clippy -D warnings` is a merge gate on every phase. 2. golden-run.sh differential vs TS on every command. 3. trait API is OURS (jcode-provider-core shape); internals may come from MIT refs. 4. Every reference read must cite file:line in the handoff — no "I used aichat" without path. 5. never pull in heavy crates (pdf/bedrock/embedding default) — feature-gate.

## Decision log (new)
- **D1 (Phase 0):** copy aichat's `openai_compatible.rs` shape (generic client + RequestData), NOT its config/state design (we use our config).
- **D2 (Phase 4b):** adopt agentive's RunnerEvent as our event vocabulary → TuiEvent. This kills the TS bridge.ts translation layer.
- **D3 (Phase 4a):** port agent-code's PermissionChecker enum → our single `Sandbox::check(path|cmd) -> Result<()>`.
- **D4 (All):** reasoning_content is a first-class stream event in the compatible client from day 1.
---
## Phase 3/4 REFINEMENT (Aug 2026 — second research pass, verified from official SDKs)

### Phase 3 (MCP) — upgraded decisions from rmcp official docs
- rmcp targets protocol revisions **2025-11-25 and 2026-07-28** (offi rust-sdk). New capabilities to USE: subscriptions (`listen()`), multi-round-trip requests (elicitation/sampling inside tools/call), response caching, client OAuth. → Our MCP client gets these for free via rmcp API.
- **Legacy 2024-11-05 HTTP+SSE is a deliberate non-goal of rmcp** (confirmed in official docs). Decision stands: thin hand-rolled SSE transport ONLY to talk to legacy-only servers — otherwise streamable HTTP only. Front any legacy server with a small proxy, don't ship legacy client code by default (feature-gate `legacy-sse`).
- Client API confirmed: `StreamableHttpClientTransport::from_uri(uri)` + `ClientInfo::default().serve(transport)`; reqwest backend behind `transport-streamable-http-client-reqwest` feature. Stdio: `TokioChildProcess`. Session-id handling automatic.
- Streamable HTTP SSE parsing is handled internally (`client-side-sse` feature) — no manual SSE parser needed in our MCP crate.

### Phase 4d (TUI — upgraded decisions from ratatui 0.30.2 official examples)
- **Chat transcript = `List` in `direction: bottom_to_top`** (chat-log idiom) + `tui-scrollbar` widget for mouse-wheel. (verified ratatui List docs + tui-widgets/tui-scrollbar)
- **Markdown rendering** = `tui-markdown` crate (NOT hand-rolled) — ratatui org official.
- **Syntax-highlighted code blocks** = `tui-code-block` (syntect themes, 8 curated) — far beyond Ink.
- **Ctrl+P palette + model picker = `Popup` + `Clear`** overlay pattern (centered widget over dimmed background, drawn last) — official ratatui pattern. Slash-command autocomplete = same Popup pattern with a filtered List.
- **Input = `ratatui-textarea`** (multi-line, cursor, key handling) — official crate, saves hundreds of LOC vs hand-rolling.
- Resize: ratatui re-renders automatically; no resize plumbing needed in app state.
- Crate versions pinned: ratatui 0.30.2, crossterm 0.29, tui-markdown, tui-code-block, ratatui-textarea, tui-scrollbar.

### D5 (new): rmcp is a full client SDK — do NOT hand-roll MCP JSON-RPC. TS did (jcode does); we don't. Our only hand-rolled piece stays the thin legacy-SSE adapter (feature-gated).
### D6 (new): TUI scroll/palette/markdown via official ratatui-org crates — never hand-roll widgets the ecosystem ships (ratatui-textarea, tui-markdown, tui-code-block, tui-scrollbar, Popup/Clear pattern).
