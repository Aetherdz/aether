# Inspiration Extraction — best patterns to absorb (Aug 2026)

Sources cloned locally at `/tmp/opencode/inspiration/` (all MIT/Apache, legally reusable):

| Project | Stars | Why it matters | Exact files to read |
|---|---|---|---|
| **sigoden/aichat** | ~9.7k | The OG Rust AI CLI (2023). Serveral years of battle-testing on the EXACT problem aetherdz solves | `src/client/openai_compatible.rs` (our provider pattern), `src/client/stream.rs` (SSE parse), `src/repl/` (REPL loop), `src/client/claude.rs`+`gemini.rs` (why SSE differs per provider) |
| **sethjuarez/agentive** | MIT | The agentic plumbing we need for Phase 4: tool-call accumulation across SSE chunks, context trimming, guardrails, cooperative cancel | `lib/src/providers/sse.rs`, `lib/src/runner.rs` (the agent loop), `lib/src/context.rs` (trimming), `lib/src/guardrails.rs`, `lib/src/cancel.rs` |
| **avala-ai/agent-code** | MIT | Pure-Rust coding agent with toolbox (like our tools/): sandbox, permissions, skills, hooks, schedule | `crates/lib/src/sandbox/`, `permissions/`, `skills/`, `tools/`, `query/` — the safety architecture we want for `aetherdz-tools` |

## What to absorb (mapped to our build)

### 1. `aetherdz-provider` (Phase 0) ← aichat
- aichat's `openai_compatible.rs` is a single generic SSE client for every OpenAI-compatible provider — matches our "8+ providers speak OpenAI wire format + SSE" decision from blueprint §5. STEAL the structure: one generic client + per-provider config (base_url, model map), NOT one client per provider.
- `stream.rs` handles: event framing, chunk assembly, `finish_reason`, thinking/reasoning_content (deepseek!). Read before writing our `Provider::stream()`.
- HOW aichat does per-provider quirks (claude/gemini differ) — warning: 2026 SSE streams carry `reasoning_content`; our default zen/deepseek uses it. aichat already solved the `reasoning_content` event handling — copy the approach.

### 2. `aetherdz-agent` (Phase 4) ← agentive
- `runner.rs` = the stream → tool_calls → execute → feed back → repeat transfer loop. This is EXACTLY executor.ts's loop. Steal: `RunnerEvent` enum (ToolResult/Status/MessagesUpdated/Done/Error) → maps 1:1 to our `TuiEvent` bridge (TS tui/bridge.ts).
- `context.rs` = context-window trimming/summarization — TS never did this well; this makes us BETTER than TS aether (blueprint §2.2 recall/embeddings gated). Adopt as `aetherdz-core::context` (phase 4).
- `cancel.rs` = CancellationToken — matches TS Esc/Ctrl+C abort (chat.ts). Use tokio_util::sync::CancellationToken (they use it).

### 3. `aetherdz-tools` (Phase 4) ← agent-code
- `sandbox.rs` + `permissions/` = the allow/deny model: agent-code enforces command allowlists + path sandboxes — exactly our tools/files.ts isInside/assertInside + bash.ts confirm gate. Steal the enum PermissionChecker pattern; port to our sandbox.
- `skills/` = how they resolve skill dirs — same as our executor resolveSkills (repo + ~/.config/aether/skills). Reference if TS implementation is thin.

### 4. Poison warnings (what NOT to copy)
- aichat's `repl/` is single-threaded blocking — we have tokio + TUI in a separate thread; keep OUR architecture (bridge emit-only). Don't copy aichat's render model.
- agent-code's Cli.rs is huge and product-bound to itself; we only need its safety modules.
- Never adopt agentive's provider abstraction wholesale: our Provider trait follows jcode-provider-core (runtime key / fork / introspection) — just network on the SSE/runner parts.

## Verdict
aichat = closest soulmate for our provider (same wire, same streaming pain). agentive = closest for agent loop. agent-code = closest for tool safety. All MIT — read-modified-reuse freely, but keep our own trait API (the jcode-provider-core shape) — network internals from theirs, surface is ours.