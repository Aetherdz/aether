# Changelog

All notable changes to aether are tracked here. Format follows [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/); versions adhere to
[SemVer](https://semver.org/spec/v2.0.0.html) once 1.0 is reached (pre-1.0
minors may still break surface APIs).

## [Unreleased]

## [0.3.0] — 2026-08-15

### Security

- Sandbox replaced with cap-std capability isolation (no more lexical
  path matching) — closes the Phase-1 audit item.
- Approval gate for dangerous `run_command` patterns (`rm -rf`,
  `git push --force`, `curl | sh`, ...) + per-project allowlist; `--yes`
  bypass only after explicit user intent.
- Session `delete()` routed through `safe_join` — blocks traversal IDs.
- Sync rejects bundles with unknown format version.
- CI: cargo-audit (advisories) + cargo-deny (licenses/bans/sources) gates;
  `--workspace --all-targets` on build/test/clippy; windows/macos/linux
  matrix now fully green.

### Added

- `docs/zen-privacy.md` — every privacy claim mapped to a source line.
- `docs/feature-matrix.md` — honest coverage matrix vs OpenCode/Aider
  (coverage, not a superiority claim).
- Maturity benchmark: `crates/aether-agent/tests/maturity_bench.rs` —
  deterministic (no-LLM) proof on 5 real repos (express, requests,
  ripgrep, jq, bootstrap): read/search/list, write+undo round-trip,
  sandbox escape block, benign command run; `benchmark/maturity.sh`
  clones the repos; skips cleanly in hermetic CI.
- TUI: agent 3-panel screen (PLAN/BUILD/ROUTE), write-diff preview in the
  BUILD panel, code-block rendering, scrollbar, error styling, live token
  counters, slash-command palette, usage bar.
- CLI: `aether tui` guarded behind an IsTerminal check.

### Changed

- TUI: usage bar shows session-cumulative `in`/`out`/`total` tokens with the
  active model — no emoji glyphs in the comparison tables or the bar.
- TUI: user messages render as full-width blocks with a distinct background
  (opencode-style) instead of plain prefixed lines.
- TUI: `finish_reply` no longer re-adds token counters (they are already
  updated by the streaming `Usage` handler — previously double-counted).
- README: comparison tables drop the "default provider" and "install
  method" rows; emoji checkmarks replaced with `Yes`/`No`/`Partial`;
  added an "Interface" row.

### Fixed

- `write_file` undo for brand-new files (`was_new` snapshots) — caught by
  the real-repo maturity benchmark.
- Undo file-missing detection platform-independent (`sandbox.exists()`).
- Cross-crate race on `AETHER_CONFIG_DIR` in `cargo test --workspace`
  (shared lock in `aether_core::testutil`).
- Install script idempotent + reliable update path.

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
