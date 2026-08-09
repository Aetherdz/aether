# Contributing to Aether

Thanks for wanting to help. Aether is a small, focused project — the bar for
merging is "is this clearly better, and is it verified?" — but every kind of
contribution is welcome: bug reports, docs, tests, features.

## Ground rules

- **Open an issue first** for anything non-trivial (new command, new crate,
  behavior change). For bug fixes and typos, a PR straight away is fine.
- **No telemetry, no accounts, no lock-in** is a feature, not a limitation.
  Contributions that add mandatory accounts, phone-home calls, or non-plain
  session formats will be rejected.
- **Behavior is verified, never re-imagined.** Golden tests exist precisely
  so ports don't drift. If your change alters output shape, update the
  golden fixtures deliberately — with a reason.

## Development workflow

```sh
# Build (debug is fine for dev)
cargo build

# Test: unit + golden
cargo test --workspace
bash golden-tests/run.sh        # needs a built binary; see golden-tests/README.md

# Quality gates — CI enforces all of these, so run them locally first
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

All three gates must pass on your branch before CI will go green.

## Project structure

| Path | What lives here |
|---|---|
| `crates/aether-core/` | config, errors, fs helpers (`safe_join_rel` sandbox) |
| `crates/aether-provider/` | provider registry (20 providers), client, tool-calling types |
| `crates/aether-agent/` | 3-model loop (plan → build → route) + sandboxed tools |
| `crates/aether-session/` | JSONL sessions, recall, usage ledger |
| `crates/aether-sync/` | gist/folder backends, line-level merge |
| `crates/aether-mcp/` | MCP server (stdio + Streamable HTTP) |
| `crates/aether-tui/` | ratatui terminal UI |
| `crates/aether-cli/` | the `aether` binary (entry point) |
| `docs/` | migration blueprint + per-phase specs |
| `golden-tests/` | behavior verification vs golden outputs |

## Security-sensitive changes

Anything touching the sandbox (`safe_join_rel`), tool execution
(`run_command`), or secret handling is security-sensitive:

- The sandbox must reject: absolute paths, `..`, `.`, empty, NUL, `a//b`.
- `run_command` keeps its timeout and output cap — never remove them.
- API keys must never reach sessions, logs, or sync payloads.
- Add a regression test for every new escape path.

See [SECURITY.md](SECURITY.md) for the full threat model.

## Commit style

- Imperative subject line, ≤ 72 chars: `agent: add diff preview before write`
- Scope prefix by area: `agent:`, `sync:`, `tui:`, `docs:`, `ci:`, `core:`
- One logical change per commit. Small commits are easier to review.
- Reference the issue when one exists: `fixes #12`

## Reviewing

- Reviewers approve when the change is correct, tested, and the gates pass —
  not because it's fashionable.
- Be kind and concrete. Assume good intent on both sides.

## What NOT to do

- Don't add new dependencies without discussion — the dependency tree is
  deliberately tiny (that's the product).
- Don't reformat unrelated code.
- Don't submit changes that break the "no Node runtime" promise.
