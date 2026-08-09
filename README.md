<p align="center">
  <img src="site/logo.svg" width="96" height="96" alt="aether logo" />
</p>

<h1 align="center">Aether</h1>

<p align="center">
  <b>A terminal AI coding agent in one Rust binary.</b><br/>
  Talk to your codebase from the shell — no Node runtime, no browser tab, no noise.
</p>

<p align="center">
  <a href="https://github.com/Aetherdz/aetherdz"><img src="https://img.shields.io/badge/rust-1.97+-black?logo=rust&logoColor=white" alt="Rust 1.97+" /></a>
  <a href="https://github.com/Aetherdz/aetherdz/blob/main/LICENSE"><img src="https://img.shields.io/github/license/Aetherdz/aetherdz" alt="MIT license" /></a>
  <a href="https://github.com/Aetherdz/aetherdz"><img src="https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20windows-lightgrey" alt="Cross-platform" /></a>
</p>

---

Aether is a **Rust port of [AETHER](https://github.com/Aetherdz/aethercode)** — the
same UX, the same providers, the same session format, but compiled to a single
static binary that talks to any OpenAI-compatible API. It started as a migration
exercise and became a full agent: chat, sessions, sync, MCP, and a ratatui TUI.

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

## What it does

| Command | Description |
|---|---|
| `aether ask "…"` | One-shot question, streams the answer |
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
| `aether-provider` | 19-provider registry, graceful fallbacks, SSE client |
| `aether-session` | JSONL sessions, auto-title, recall, usage ledger |
| `aether-sync` | gist/folder backends, line-level merge |
| `aether-mcp` | MCP server (stdio + Streamable HTTP) |
| `aether-tui` | ratatui terminal UI |
| `aether-cli` | the `aether` binary |

```
aether/
  crates/
    aether-core/      # config, error, fs, prompt
    aether-provider/  # client, registry, model, provider
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

## Contributing

Ideas, issues, and PRs welcome. Open an issue first for anything non-trivial.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

[MIT](./LICENSE) — build on it, fork it, vendor it.
