# aetherdz

Rust port of [AETHER](https://github.com/Aetherdz/aethercode) — a cross-platform
terminal AI coding agent. Same UX, same providers, same session format, zero Node
runtime. Builds a single static binary that talks to any OpenAI-compatible API.

## Status

Phase 0–4 complete (working binary):

- **ask / chat** — one-shot streaming answer and interactive REPL
- **use / models / providers** — provider + model management with graceful zen fallback
- **sessions** — JSONL session store with auto-title, recall search, usage ledger
- **recall** — keyword search across past sessions
- **sync** — gist or folder backends with line-level merge
- **mcp** — MCP server over stdio and Streamable HTTP (session/recall/sync tools)
- **tui** — ratatui terminal UI: session list + chat, ctrl+P model palette, mouse wheel, live token ledger

## Build

```sh
cargo build --release
./target/release/aether --help
```

No API key required by default — the free **zen** provider is the default.

## Workspace

| Crate | Role |
|---|---|
| `aetherdz-core` | config, errors, fs helpers |
| `aetherdz-provider` | 19-provider registry, graceful fallbacks, SSE client |
| `aetherdz-session` | JSONL sessions, auto-title, recall, usage ledger |
| `aetherdz-sync` | gist/folder backends, line-level merge |
| `aetherdz-mcp` | MCP server (stdio + Streamable HTTP) |
| `aetherdz-tui` | ratatui terminal UI |
| `aetherdz-cli` | the `aether` binary |

## Porting notes

`docs/` holds the migration blueprint and per-phase specs. Every module cites the
original TS file it ports; behavior is verified against golden outputs rather than
re-implemented from memory.

## License

MIT
