<p align="center">
  <img src="site/logo.svg" width="96" height="96" alt="aether logo" />
</p>

<h1 align="center">Aether</h1>

<p align="center">
  <b>A terminal AI coding agent in one Rust binary.</b><br/>
  Talk to your codebase from the shell — no Node runtime, no browser tab, no noise.
</p>

<p align="center">
  <a href="https://github.com/Aetherdz/aether"><img src="https://img.shields.io/badge/rust-1.97+-black?logo=rust&logoColor=white" alt="Rust 1.97+" /></a>
  <a href="https://github.com/Aetherdz/aether/blob/main/LICENSE"><img src="https://img.shields.io/github/license/Aetherdz/aether" alt="MIT license" /></a>
  <a href="https://github.com/Aetherdz/aether"><img src="https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20windows-lightgrey" alt="Cross-platform" /></a>
  <a href="https://github.com/Aetherdz/aether/actions/workflows/ci.yml"><img src="https://github.com/Aetherdz/aether/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://github.com/Aetherdz/aether"><img src="https://img.shields.io/badge/tests-86%20passing-brightgreen" alt="86 tests passing" /></a>
</p>

---

Aether is a **Rust port of [AETHER](https://github.com/Aetherdz/aethercode)** — the
same UX, the same providers, the same session format, but compiled to a single
static binary that talks to any OpenAI-compatible API. It started as a migration
exercise and became a full agent: chat, sessions, sync, MCP, and a ratatui TUI.

<p align="center">
  <img src="site/demo.gif" width="800" alt="aether in action — help, providers, models, ask, sessions" />
</p>

## Why Aether?

|  | Aether | Node-based agents |
|---|---|---|
| **Runtime** | One native Rust binary | Node + hundreds of MB of deps |
| **Startup** | Instant — compiled, no interpreter | Cold start on every run |
| **Default provider** | Free `zen` — works with zero API keys | Usually paid keys required |
| **Session format** | Plain JSONL — grep it, script it, own it | Often bespoke databases |
| **Sync** | Gist or folder, line-level merge | Varies, rarely portable |
| **MCP** | stdio + Streamable HTTP server built in | Requires separate config |

No telemetry, no accounts, no lock-in. Your sessions are files on your disk.

### vs. specific tools

| | Aether | Aider | Continue | Claude Code | opencode | jcode |
|---|---|---|---|---|---|---|
| **Runtime** | Native Rust binary | Python | Node/VS Code | Node | TypeScript/Node | Rust binary |
| **Install** | `cargo install` (one binary) | `pip install` | VS Code ext | npm package | npm/curl | cargo build |
| **Free default provider** | ✅ built-in `zen` | ❌ needs key (Ollama optional) | ⚠️ local models only | ❌ Anthropic key | ⚠️ needs key/local | ⚠️ needs key/local |
| **Agent loop w/ tools** | ✅ plan→build→route (3 models) | ✅ | ✅ | ✅ | ✅ | ⚠️ partial |
| **Sandboxed tools** | ✅ path-sandboxed, 30 s timeout | ⚠️ per-command confirm | ⚠️ | ⚠️ | ⚠️ | ⚠️ |
| **Diff preview before write** | ✅ built in (y/N gate) | ✅ | ✅ | ✅ | ✅ | ❌ |
| **Undo / checkpoint** | ✅ `.aether-undo` + `aether undo` | ❌ | ❌ | ⚠️ partial | ❌ | ❌ |
| **Sessions as plain files** | ✅ JSONL | ⚠️ sqlite | ❌ | ❌ | ⚠️ sqlite | ✅ JSONL |
| **Cross-device sync** | ✅ gist/folder line-merge | ❌ | ❌ | ❌ | ❌ | ❌ |
| **MCP server** | ✅ built in (stdio + HTTP) | ❌ (client only) | ✅ client | ✅ client | ✅ | ❌ |
| **Telemetry** | ❌ none, verifiable | ⚠️ opt-in | ⚠️ | ⚠️ | ⚠️ | ❌ |
| **Local models (Ollama/LM Studio)** | ✅ first-class providers | ✅ | ✅ | ❌ | ✅ | ⚠️ |

> Measured on this repo (see [benchmark/](benchmark/) for the harness):
> **4.99 MB binary · ~112 ms cold start · ~5 MB idle TUI RSS** — reproducible
> via `./benchmark/run.sh`. Numbers above are architectural claims otherwise.

## Quick start

```sh
# 1. Build the binary
cargo build --release

# 2. Ask your first question (the free zen provider works out of the box)
./target/release/aether ask "explain this repo in one paragraph"

# 3. Or open the interactive chat
./target/release/aether chat
```

> No API key required by default — **zen** is the built-in free provider.
> Bring your own key anytime: `aether use anthropic/claude-sonnet-5`.

### Offline / local models (first-class)

Aether treats local models as first-class providers — no key, no account,
no network needed once the model is downloaded:

```sh
# Ollama (default http://localhost:11434)
ollama pull llama3.2
aether use ollama/llama3.2

# LM Studio (default http://localhost:1234/v1)
aether use lmstudio/local-model
```

Both connect to any OpenAI-compatible local server and are auto-discovered
with sensible defaults — the only setup is running the server itself.

## What it does

| Command | Description |
|---|---|
| `aether ask "…"` | One-shot question, streams the answer |
| `aether agent "task"` | 3-model agent loop: plan → build → route with tools |
| `aether chat` | Interactive REPL chat |
| `aether use provider/model` | Set default provider and model |
| `aether models [--live]` | List models (zen fetches live) |
| `aether providers` | List all providers + key status |
| `aether sessions` | List, show, resume, rename, delete sessions |
| `aether recall "…"` | Keyword search across past sessions |
| `aether sync` | Sync sessions across devices (gist/folder) |
| `aether tui` | Full ratatui terminal UI |

```sh
# A real session
$ aether use zen/llama-3.3-70b
✓ default set to zen/llama-3.3-70b

$ aether ask "what is the sync line-merge strategy?"
▸ loading index · 1,284 symbols · 96 files
▸ aether scanning aether-sync …

The sync crate merges lines, not files. Each block carries an origin
fingerprint; a conflict is resolved by keeping the newer origin and
flagging the older one for review.
```

### The agent loop

`aether agent "task"` runs the loop that makes Aether a *coding agent*:
three models cooperate on one task, each with its own role:

1. **plan** — a planner reads the task and writes a step-by-step plan.
2. **build** — a builder executes the plan using real tools: `read_file`,
   `write_file`, `list_dir`, `run_command`, `search`. Every path is sandboxed
   to the working directory — absolute paths, `..` escapes and NUL bytes are
   rejected before anything touches disk.
3. **route** — a router watches the result and decides: keep going, revise
   the plan, or declare it done.

The loop runs until the router says **done** or the iteration cap is hit.

```sh
# Every role defaults to your configured model; override any of them.
aether agent "add a --dry-run flag to the sync command" \
  --plan-model zen/llama-3.3-70b \
  --build-model anthropic/claude-sonnet-5 \
  --route-model openai/gpt-5 \
  --iterations 8
```

Tool calls work two ways: native `tool_calls` when the provider supports
them, and a fenced-JSON fallback (`{"tool": "...", "args": {...}}` in the
reply) otherwise — so the loop runs on any OpenAI-compatible endpoint.

### The TUI

`aether tui` launches a ratatui interface: session list + chat on the left,
Ctrl+P model palette, mouse-wheel scrolling, and a live token ledger so you
always know what a session costs before you commit to it.

### MCP server

`aether-mcp` speaks the Model Context Protocol over **stdio** and
**Streamable HTTP**. Your editor, your agents, and Aether share one
context layer — sessions, recall, and sync exposed as tools.

### Sync

Sessions sync to a **GitHub gist** or a **local folder**, merged line-by-line
with origin fingerprints — concurrent edits never silently overwrite.

## Workspace

| Crate | Role |
|---|---|
| `aether-core` | config, errors, fs helpers |
| `aether-provider` | 19-provider registry, graceful fallbacks, SSE client, tool-calling types |
| `aether-agent` | the 3-model loop: plan, build (sandboxed tools), route |
| `aether-session` | JSONL sessions, auto-title, recall, usage ledger |
| `aether-sync` | gist/folder backends, line-level merge |
| `aether-mcp` | MCP server (stdio + Streamable HTTP) |
| `aether-tui` | ratatui terminal UI |
| `aether-cli` | the `aether` binary |

```
aether/
  crates/
    aether-core/      # config, error, fs, prompt
    aether-provider/  # client, registry, model, provider, tool-calling
    aether-agent/     # plan/build/route loop + sandboxed tool registry
    aether-session/   # JSONL store, recall, ledger
    aether-sync/      # gist + folder backends, line merge
    aether-mcp/       # MCP stdio + Streamable HTTP server
    aether-tui/       # ratatui session list + chat
    aether-cli/       # the `aether` binary (entry point)
  docs/                 # migration blueprint + per-phase specs
  golden-tests/         # verified behavior against golden outputs
```

## Building from source

```sh
cargo build --release          # optimized, stripped, thin-LTO
cargo test --workspace         # run the golden + unit suites
./target/release/aether --help
```

Requirements: **Rust 1.97+** (edition 2024), a C toolchain for linking.

## Status

Phase 0–4 complete — the binary works end to end:

- [x] `ask` / `chat` — one-shot streaming + interactive REPL
- [x] `agent` — 3-model loop (plan → build → route) with sandboxed tools
- [x] `use` / `models` / `providers` — provider management, graceful zen fallback
- [x] `sessions` — JSONL store, auto-title, recall search, usage ledger
- [x] `recall` — keyword search across past sessions
- [x] `sync` — gist or folder backends with line-level merge
- [x] `mcp` — MCP server over stdio + Streamable HTTP
- [x] `tui` — ratatui terminal UI with Ctrl+P palette and token ledger

> Early development: APIs may shift until 1.0. Golden tests keep every port
> honest — behavior is verified against reference outputs, never re-imagined.

## How the port works

`docs/` holds the migration blueprint and per-phase specs. Every module cites
the original TypeScript file it ports; behavior is verified against golden
outputs rather than re-implemented from memory. That discipline is what made
this more than a rewrite — it's a faithful, faster, dependency-free twin.

### Docs index

| Doc | What it covers |
|---|---|
| [rust-migration-blueprint.md](docs/rust-migration-blueprint.md) | The full migration plan: architecture, crate layout, phases, risks |
| [build-plan-v2.md](docs/build-plan-v2.md) | Second-pass build plan, sequencing, and status |
| [phase1-session-spec.md](docs/phase1-session-spec.md) | JSONL session format, auto-title, recall, usage ledger |
| [phase2-sync-spec.md](docs/phase2-sync-spec.md) | Gist/folder sync backends, line-level merge semantics |
| [phase3-mcp-spec.md](docs/phase3-mcp-spec.md) | MCP server over stdio + Streamable HTTP |
| [phase4-tools-agent-auth-tui-spec.md](docs/phase4-tools-agent-auth-tui-spec.md) | Sandboxed tools, 3-model agent loop, auth, ratatui TUI |
| [inspiration-extraction.md](docs/inspiration-extraction.md) | Design notes extracted from the original AETHER |

## Security

Aether is a local-first tool that can execute commands on your behalf — read
the [threat model](SECURITY.md) before granting it elevated trust. In short:

- **File tools** (`read_file`, `write_file`, `list_dir`, `search`) are
  sandboxed to the working directory: absolute paths, `..` escapes and NUL
  bytes are rejected before touching disk.
- **`run_command`** executes via `/bin/sh -c` with a **30-second timeout**
  and **128 KB output cap**. It is *not* network-isolated and has no
  allow-list — treat it as "run this command as you". Only run `agent` in
  directories you trust with shell access.
- **API keys** are read from environment variables / config files; they are
  never written into sessions or logs.

## Contributing

Ideas, issues, and PRs welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for
the workflow and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for the ground
rules. Open an issue first for anything non-trivial.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

[MIT](./LICENSE) — build on it, fork it, vendor it.
