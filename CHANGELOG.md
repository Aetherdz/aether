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
- Tests: usage-counter live update, code-block rendering, and 10 earlier
  editing/palette/stream cases (now 55 in `aether-tui`, 165 workspace-wide).
- README: explicit privacy note for the built-in free `zen` provider
  (hosted by opencode.ai at `https://opencode.ai/zen/v1`), clarified
  stability wording, and a rewritten feature checklist.

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
