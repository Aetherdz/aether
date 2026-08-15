# Changelog

All notable changes to aether are tracked here. Format follows [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/); versions adhere to
[SemVer](https://semver.org/spec/v2.0.0.html) once 1.0 is reached (pre-1.0
minors may still break surface APIs).

## [Unreleased]

### Added

- TUI: error styling — failures render in red (`theme::danger`) in both the
  chat status line and the sessions footer, instead of muted gray.
- TUI: Agent screen now renders three real horizontal panels
  (PLAN / BUILD / ROUTE) with distinct accent colors and an iteration
  progress bar in the BUILD panel.
- TUI: fenced code blocks (` ```lang `) in assistant replies get a distinct
  background and a language label; indentation is preserved when lines wrap.
- TUI: scrollbar on the transcript when history exceeds the viewport.
- TUI: live token accounting — `input`/`output` counters update as `Usage`
  events stream in, not only when the reply finishes.
- Agent/TUI: `write_file` diffs — `ToolResult` and `AgentPhase::ToolCalled`
  carry a rendered `+`/`-` preview, and the Agent BUILD panel shows the last
  write diff inline (up to 12 lines) under the tool name.
- Tests: usage-counter live update, code-block rendering, write-diff carry +
  BUILD-panel rendering, and 10 earlier editing/palette/stream cases (now 56
  in `aether-tui`, 167 workspace-wide).
- README: explicit privacy note for the built-in free `zen` provider
  (hosted by opencode.ai at `https://opencode.ai/zen/v1`), clarified
  stability wording, and a rewritten feature checklist.
- Docs: `docs/zen-privacy.md` — every privacy claim mapped to a source
  line (no telemetry code paths, `key_env: None` for zen, only
  `chat/completions` endpoint, `OPENCODE_ZEN_API_KEY` used for paid
  models only).
- CI: cargo-audit (advisories) + cargo-deny (licenses/bans/sources) gates;
  `--workspace --all-targets` on build/test/clippy; README test badge now
  192 passing.
- Docs: `docs/feature-matrix.md` — honest coverage matrix vs OpenCode and
  Aider ("covers what they have", not a superiority claim), with a
  "Verified against source" table and marked gaps (MCP client, git
  auto-commit, slash commands, watch mode, plugins, subagents).
- Benchmark: `crates/aether-agent/tests/maturity_bench.rs` — deterministic
  (no-LLM) maturity proof: read/search/list, write+undo round-trip,
  sandbox escape block, and benign command run against 5 real repos
  (express, requests, ripgrep, jq, bootstrap), skippable in hermetic CI;
  `benchmark/maturity.sh` clones the repos.
- Tests: workspace-wide serialization of env-mutating tests via
  `aether_core::testutil` (single lock shared by session/sync/mcp/tui)
  — kills a cross-crate race on `AETHER_CONFIG_DIR` that flaked CI.

### Changed

- TUI: usage bar shows session-cumulative `in`/`out`/`total` tokens with the
  active model — no emoji glyphs in the comparison tables or the bar.
- TUI: user messages render as full-width blocks with a distinct background
  (opencode-style) instead of plain prefixed lines.
- TUI: `finish_reply` no longer re-adds token counters (they are already
  updated by the streaming `Usage` handler — previously double-counted).
- README: comparison tables drop the "default provider" and "install
  method" rows; emoji checkmarks replaced with `Yes`/`No`/`Partial`;
  added an "Interface" row; badge now 165.

### Fixed

- Agent: `write_file` undo for brand-new files — a fresh file now gets a
  `was_new` snapshot so `undo` deletes it instead of restoring a phantom
  previous version (caught by the real-repo maturity benchmark).
- Agent: undo file-missing detection is platform-independent
  (`sandbox.exists()` instead of Unix-only error-string matching) — the
  Windows CI failure is gone.
- Tests: env-mutating tests across crates (session/sync/mcp/tui) now
  serialize on one shared `aether_core::testutil` lock instead of four
  per-crate locks, eliminating a `cargo test --workspace` race on
  `AETHER_CONFIG_DIR`.

## [0.2.0] — 2026-08-10

### Added

- TUI v2: chrome header + tabs (chat / sessions / agent / commands),
  live Agent screen with plan cards, status line, persistent keybinding bar.
- CLI: slash-command palette (`/clear`, `/help`, `/exit`) inside the TUI.
- Session persistence for the agent loop: `AgentState` checkpoint + resume.
- Benchmark harness (`benchmark/`): reproducible **4.99 MB · ~112 ms ·
  ~5 MB RSS** measurement of the real TUI.
- Install script: official `curl | bash` with OS/arch detection and
  SHA-256 verification.

### Changed

- CLI surface reduced to 6 root commands (`ask`, `chat`, `agent`, `tui`,
  `provider`, `session`); legacy names still parse with a deprecation notice.
- Modern dark theme (slate + green accent), RAM reductions in the TUI.
- Version bumped to 0.2.0 (TUI v2).

### Fixed

- TUI panics when run inside a tokio runtime (nested-runtime panic).
- Windows CI test failures; native ARM release runner added.

## [0.1.0] — 2026-08-04

### Added

- 3-model agent loop (plan → build → route) with native tool-calling and the
  `aether-agent` crate.
- Sandboxed file tools, diff preview + confirm gate, undo/checkpoint,
  stagnation detection.
- JSONL session store with auto-title, recall search, usage ledger.
- Gist/folder sync backends with line-level merge.
- MCP server over stdio + Streamable HTTP.
- Matrix build workflow (linux/macOS/windows, x86_64 + aarch64).

### Docs

- SECURITY.md (threat model), CONTRIBUTING.md, CODE_OF_CONDUCT.md,
  docs index in README, competitive comparison table.
