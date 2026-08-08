# BUILD PLAN v2 — merged with studied reference projects (Aug 2026)
*Supersedes §6 phased plan internals: same phases, but every phase now names the EXACT reference files to read/copy (cloned at `/tmp/opencode/inspiration/`). Keeps our trait API (jcode-provider-core shape); steals battle-tested internals from MIT projects.*

## Pattern book (from studying aichat / agentive / agent-code)

| Pattern | Source file | What we adopt | Phase |
|---|---|---|---|
| **One generic OpenAI-compatible client** (config: base_url+models+api_key), NOT one client per provider | `aichat/src/client/openai_compatible.rs` (162 LOC) | `aetherdz-provider::providers::compatible` — 8+ providers become config rows, not code | 0 |
| **reasoning_content in compatible client** (aichat gap: only openai.rs:138 has it) | `aichat/src/client/openai.rs:138,358` + our verified zen/deepseek streams | `StreamEvent::Reasoning` handled in compatible path — our edge over aichat | 0 |
| **SSE event framing** — chunk assembly, finish_reason, per-provider stream events | `aichat/src/client/stream.rs` (296 ln) | `aetherdz-provider::stream::SseParser` — unit-tested framing | 0 |
| **Agent loop events** — RunnerEvent enum (ToolResult/Status/MessagesUpdated/Done/Error) | `agentive/lib/src/runner.rs` | Maps 1:1 to TuiEvent bridge (TS tui/bridge.ts) | 4 |
| **Context trimming/summarization** — TS never did this; beats old TS aether | `agentive/lib/src/context.rs` | `aetherdz-core::context` behind `memory` feature (gated) | 4/5 |
| **CancellationToken** — matches TS Esc/Ctrl+C abort | `agentive/lib/src/cancel.rs` (tokio_util) | `aetherdz-core::cancel` | 0 (chat abort) |
| **Tool-call accumulation across SSE chunks** — partial JSON args | `agentive/lib/src/providers/sse.rs` + `agent-code/crates/lib/src/llm/` | `aetherdz-provider::tool_acc` | 4 |
| **Path sandbox + permission allow/deny** — for our tools/isInside+assertInside | `agent-code/crates/lib/src/sandbox/` + `permissions/` | `aetherdz-tools::sandbox` (canonicalize, deny .., symlink-safe) | 4 |
| **Skills resolution** — repo + ~/.config/aether/skills/ | `agent-code/crates/lib/src/skills/` | `aetherdz-agent::skills` | 4 |

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