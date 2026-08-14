<p align="center">
  <img src="site/assets/img/logo.svg" width="96" height="96" alt="aether logo" />
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
  <a href="https://github.com/Aetherdz/aether"><img src="https://img.shields.io/badge/tests-167%20passing-brightgreen" alt="167 tests passing" /></a>
</p>

---

Aether is a **Rust port of [AETHER](https://github.com/Aetherdz/aethercode)** — the
same UX, the same providers, the same session format, but compiled to a single
static binary that talks to any OpenAI-compatible API. It started as a migration
exercise and became a full agent: chat, sessions, sync, MCP, and a ratatui TUI.

<p align="center">
  <img src="site/assets/img/demo.gif" width="800" alt="aether TUI v2 — session list, chat transcript, live plan → build → route agent screen" />
</p>

## Install

One command, verified checksum-first — downloads the prebuilt binary for your
OS/arch from GitHub Releases, verifies its SHA-256, and installs it:

```sh
curl -fsSL https://raw.githubusercontent.com/Aetherdz/aether/main/scripts/install.sh | bash
```

Installs to `~/.local/bin` (falls back to `/usr/local/bin`). Overrides:
`AETHER_VERSION` (tag, default `latest`) and `AETHER_INSTALL_DIR`.

Re-running the same command **updates** an existing install: it detects the
installed version, upgrades when older, warns and keeps the binary when newer,
and exits early when already up to date (no re-download).

### Download types

| OS      | x86_64 / amd64           | aarch64 / arm64         |
|---------|--------------------------|-------------------------|
| Linux   | `aether-linux-x86_64`    | `aether-linux-aarch64`  |
| macOS   | `aether-macos-x86_64`    | `aether-macos-aarch64`  |
| Windows | `aether-windows-x86_64.exe` | —                     |

Every release ships `SHA256SUMS.txt`; the installer refuses to install on a
mismatch. No Node, no Python, no package-manager hoops — one native binary.

## Why Aether?

|  | Aether | Node-based agents |
|---|---|---|
| **Runtime** | One native Rust binary | Node + hundreds of MB of deps |
| **Size** | **4.99 MB** — one stripped binary | hundreds of MB installed |
| **Startup** | **~112 ms** cold start | 1 s+ cold start per run |
| **Interface** | ratatui TUI + six CLI commands | CLI or IDE panel |
| **Session format** | Plain JSONL — grep it, script it, own it | Often bespoke databases |
| **Sync** | Gist or folder, line-level merge | Varies, rarely portable |
| **MCP** | stdio + Streamable HTTP server built in | Requires separate config |

No telemetry, no accounts, no lock-in. Your sessions are files on your disk.

### vs. specific tools (2026)

| | Aether | Aider | Continue | Claude Code | opencode | jcode |
|---|---|---|---|---|---|---|
| **Interface** | ratatui TUI + six CLI commands | terminal REPL (git-native) | IDE extension (VS Code / JetBrains) | terminal REPL (Node) | terminal TUI (Bun) | terminal TUI (Rust) |
| **Runtime** | Native Rust binary | Python | Node/VS Code | Node | TypeScript/Bun | Rust binary |
| **Binary size** | **4.99 MB** (measured) | — (Python) | — (Node) | — (Node) | 35–55 MB* | — (Rust) |
| **Startup** | **~112 ms** `--version` (measured) | — | — | ~3.4 s† | ~1 s† | ~14 ms† (to first frame) |
| **Agent loop w/ tools** | Yes — plan→build→route (3 models) | Yes | Yes | Yes | Yes | Partial |
| **Sandboxed tools** | Yes — path-sandboxed, 30 s timeout | Partial — per-command confirm | Partial | Partial | Partial | Partial |
| **Diff preview before write** | Yes — built in (y/N gate) | Yes | Yes | Yes | Yes | No |
| **Undo / checkpoint** | Yes — `.aether-undo` + `aether undo` | No | No | Partial | No | No |
| **Sessions as plain files** | Yes — JSONL | Partial — sqlite | No | No | Partial — sqlite | Yes — JSONL |
| **Cross-device sync** | Yes — gist/folder line-merge | No | No | No | No | No |
| **MCP server** | Yes — built in (stdio + HTTP) | No (client only) | Yes — client | Yes — client | Yes | No |
| **Telemetry** | No — none, verifiable | Partial — opt-in | Partial | Partial | Partial | No |
| **Local models (Ollama/LM Studio)** | Yes — first-class providers | Yes | Yes | No | Yes | Partial |

> Aether figures are **measured in this repo** by [benchmark/run.sh](benchmark/run.sh)
> (5-run median, 2026-08-09) — same binary, same machine, reproducible.
> Other tools' figures are **their own published claims**, not measured here:
> `*` platform-binary size per third-party report; `†` vendor-published time-to-first-frame / boot benchmarks. Startup metrics are **not directly comparable** across tools (we measure `--version` exit; jcode/Claude Code publish first-frame time).

## Quick start

No Rust toolchain needed — install the prebuilt binary, checksum-verified:

```sh
# 1. Install (downloads the prebuilt binary for your OS/arch, verifies SHA-256)
curl -fsSL https://raw.githubusercontent.com/Aetherdz/aether/main/scripts/install.sh | bash

# 2. Ask your first question (the free zen provider works out of the box)
aether ask "explain this repo in one paragraph"

# 3. Or open the full TUI
aether tui
```

Prefer to build from source? See [Building from source](#building-from-source)
(requires Rust 1.97+).

> No API key required by default — **zen** is the built-in free provider.
> Bring your own key anytime: `aether use anthropic/claude-sonnet-5`.

> **zen privacy note** — the free `zen` endpoint is hosted by
> **opencode.ai** (`https://opencode.ai/zen/v1`). Anything you send through
> it (including code from your working directory) goes to that third party.
> For sensitive work, prefer a local model (below) or your own key.

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

Six root commands; the legacy names (`use`, `models`, `providers`, `sessions`,
`recall`, `sync`, `undo`) still parse but print a one-line deprecation notice:

| Command | Description |
|---|---|
| `aether ask "…"` | One-shot question, streams the answer |
| `aether chat` | Interactive REPL chat |
| `aether agent "task"` | 3-model agent loop: plan → build → route with tools |
| `aether agent undo [f]` | List / restore write snapshots |
| `aether tui` | Full ratatui terminal UI |
| `aether provider …` | `list` · `models [--live]` · `use provider/model` |
| `aether session …` | `list` · `show` · `delete` · `rename` · `resume` · `search` · `sync` |

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

`aether tui` launches a ratatui interface with a chrome header (brand, screen,
model, provider, version) and a tab bar — **chat · agent · sessions**:

- **Sessions** — pick a past session or start fresh, keyboard-first
- **Chat** — transcript with role-colored messages, plan cards (```plan
  fences become bordered cards with a todo progress meter), and a status line
  (ready / thinking / streaming with token count)
- **Agent** — three live panels (PLAN · BUILD · ROUTE) fed by the agent loop's
  observer channel: iteration, tool-call, and verdict counters update in real
  time while the three-model loop runs

Mouse-wheel scrolling and a live token ledger round it out, so you always know
what a session costs before you commit to it.

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

Phase 0–4 complete — the binary works end to end. Six root commands
(`ask`, `chat`, `agent`, `tui`, `provider`, `session`); the legacy names
(`use`, `models`, `providers`, `sessions`, `recall`, `sync`, `undo`) still
parse but print a deprecation notice:

- [x] `ask` / `chat` — one-shot streaming + interactive REPL
- [x] `agent` — 3-model loop (plan → build → route) with sandboxed tools, write snapshots + `agent undo`
- [x] `provider` — `list` / `models [--live]` / `use provider/model`, graceful zen fallback
- [x] `session` — JSONL store, auto-title, recall search, usage ledger
- [x] `sync` (legacy) — gist or folder backends with line-level merge
- [x] `mcp` — MCP server over stdio + Streamable HTTP
- [x] `tui` — ratatui TUI v2: chrome header + tabs, live agent panels, plan cards, status line
- [x] `benchmark/` — reproducible harness: **4.99 MB · ~112 ms · ~5 MB RSS**

> **Stability note** — the checklist above means the features exist and pass
> their test suites (167 tests, all green), not that the public API is frozen.
> The **CLI surface** (`ask`/`agent`/`tui` flags, config file format, JSONL
> session schema) is still pre-1.0 and may shift; the **interactive TUI** and
> session files you create today are the least stable parts. `CHANGELOG.md`
> tracks every change so you can see exactly what moved between releases.
> Golden tests keep every port honest — behavior is verified against
> reference outputs, never re-imagined.

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
| [CHANGELOG.md](CHANGELOG.md) | Version history — every change between releases, SemVer-tracked |

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
