# Feature coverage matrix — Aether vs Aider / Claude Code / opencode

This is a **coverage** comparison, not a superiority claim. The question it
answers is: *"Does Aether cover the essential features that already exist in
the other tools?"* — marked honestly, including the gaps.

Legend: ✅ full · 🟡 partial · ❌ absent.

| Feature | Aether | Aider | Claude Code | opencode |
|---|---|---|---|---|
| Interactive chat (REPL/TUI) | ✅ `aether chat` / `aether tui` | ✅ | ✅ | ✅ |
| One-shot ask | ✅ `aether ask` | ✅ | ✅ | ✅ |
| Agent loop with tools | ✅ plan→build→route (3 models, capped iterations, resume) | ✅ | ✅ | ✅ |
| File read / write / list | ✅ `read_file` `write_file` `list_dir` | ✅ | ✅ | ✅ |
| Code search | ✅ `search` (grep over sandbox) | ✅ | ✅ | ✅ |
| Shell command execution | ✅ `run_command` (30 s timeout) | ✅ | ✅ | ✅ |
| Diff preview before write | ✅ built-in y/N gate | ✅ | ✅ | ✅ |
| Undo / checkpoint | ✅ `.aether-undo` + `aether undo` | 🟡 git-based | ✅ | ✅ |
| Sessions (list/show/resume/delete/search) | ✅ JSONL files | 🟡 sqlite | ✅ | 🟡 sqlite |
| Session sync across devices | ✅ gist/folder line-merge | ❌ | ❌ | ❌ |
| Multi-provider (cloud + local) | ✅ 19 providers incl. Ollama/LM Studio | ✅ | 🟡 no local | ✅ |
| MCP **server** (expose tools) | ✅ stdio + Streamable HTTP | ❌ | 🟡 | 🟡 |
| MCP **client** (consume servers) | ❌ | ❌ | ✅ | ✅ |
| Tool sandboxing (path confinement) | ✅ cap-std capability handle | 🟡 per-cmd confirm | 🟡 | 🟡 |
| Dangerous-command approval | ✅ risk classifier + allowlist | 🟡 | ✅ | ✅ |
| Plain-file session format (scriptable) | ✅ JSONL | 🟡 | ❌ | 🟡 |
| TUI | ✅ ratatui | ❌ | ❌ | ✅ |
| No telemetry (verifiable) | ✅ zero, greppable | 🟡 opt-in | 🟡 | 🟡 |
| Git auto-commit after edits | ❌ | ✅ | ✅ | ✅ |
| Slash commands in chat | ❌ | ✅ | ✅ | ✅ |
| Watch mode (react to file changes) | ❌ | ✅ | ❌ | ✅ |
| Plugins / extensions | ❌ | ✅ | ✅ | ✅ |
| User-defined subagents | ❌ | ❌ | ✅ | ✅ |

## Reading the matrix honestly

- Aether **covers the core agent essentials**: chat, ask, tool loop, file
  tools, diff gate, undo, sessions, providers, sandboxing, approval gate.
- Aether **exceeds the others** in exactly one place: cross-device sync
  (gist/folder). That is a fact, not a superiority claim — it is a feature
  the others do not ship.
- Aether **lacks** six features the others have: MCP client, git auto-commit,
  slash commands, watch mode, plugins, user-defined subagents.
- The gaps are **feature-completeness gaps, not design compromises**: the
  architecture (provider registry, tool registry, session store, sandbox)
  is built to host all of them; none requires re-architecting.

## Verified against source (2026-08-14)

| Claim | Evidence |
|---|---|
| 6 agent tools | `crates/aether-agent/src/tools.rs` — `read_file`, `write_file`, `list_dir`, `run_command`, `search`, `undo` |
| 19 providers | `crates/aether-provider/src/registry.rs` — zen, openai, anthropic, google, deepseek, openrouter, ollama, groq, mistral, xai, cerebras, togetherai, fireworks, perplexity, moonshot, minimax, huggingface, lmstudio, github |
| 3-model loop + resume | `crates/aether-cli/src/cli.rs` — `--plan-model`, `--build-model`, `--route-model`, `--resume` |
| Diff gate | `crates/aether-agent/src/tools.rs` — `ConfirmPolicy` (Disabled/AutoApprove/Prompt) |
| Undo | `crates/aether-agent/src/undo.rs` — `UNDO_DIR_NAME = ".aether-undo"` |
| MCP server only | `crates/aether-mcp/src/lib.rs` — `serve_stdio`, `serve_http`; no client module |
| No git auto-commit | no `git commit` invocation outside `run_command` test fixtures |
| No slash commands | `crates/aether-cli/src/main.rs` — help text lists commands, no in-chat `/` dispatch |
| No watch/plugins/subagents | absent from CLI surface and crate layout |