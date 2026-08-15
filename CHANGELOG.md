# Changelog

All notable changes to aether are tracked here. Format follows [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/); versions adhere to
[SemVer](https://semver.org/spec/v2.0.0.html) once 1.0 is reached (pre-1.0
minors may still break surface APIs).

## [Unreleased]

### Added

- TUI: right usage/session sidebar on the chat screen (opencode-style) — a
  fixed 32-column panel docked on the right edge of the transcript showing
  live token counters (`in 1.2K · out 3.4K · total 4.6K`, compact units via
  `chrome::format_tokens`), the current session (title, turns, messages,
  session in/out tokens), a divider, `model:` / `provider:`, and a muted
  `mcp: server crate (external)` note (aether-mcp is a standalone MCP
  *server* crate — the TUI has no in-app MCP client data). The sidebar is
  skipped on terminals narrower than 72 columns so the transcript keeps the
  full width, and the chat scrollbar stays on the transcript column.
- TUI: live agent-pattern badge in the chat status line — while an agent
  run is active the status area shows `plan -> build -> route` with the
  current stage highlighted (accent) and the other two muted, updating in
  real time as `AgentPhase` events arrive (the chat now drains the same
  observer channel as the Agent screen via a shared `drain_agent_events`).
- TUI: interactive model picker — the ctrl+P palette (also reachable via
  `/model`) now lists the provider's static models plus every custom model
  from `config.json`, highlights the active model, wraps around with j/k,
  and offers an `+ add custom model` entry that walks through base URL,
  API key env var name, and model name (Esc cancels at any step) and
  persists the provider via `save_config`. Selecting a model re-sends the
  last question against it; custom models are served by their own provider
  endpoint (`client_for_model`).
- TUI: dynamic terminal tab/window title (OSC 0 via crossterm `SetTitle`) —
  shows the current page (`aether — sessions` / `aether — chat` /
  `aether — agent loop`) and switches to `aether — <session> | <model>`
  (session title truncated to 40 chars) once a session is opened, like
  opencode/claude-code.

### Changed

- TUI: original terminal title is restored on exit — the app captures the
  pre-existing title via the XTerm title query (`\x1B[21t`, parsed from
  `\x1B]l…\x1B\\` / `\x1B]0;…\x07` / `\x1B]2;…\x07` replies) at startup and
  re-applies it on every exit path, so the window/tab title returns to
  whatever it was before the app started (best-effort: a terminal that
  doesn't answer the query leaves the title untouched).
- TUI: thinking state is visible immediately — the status line shows an
  animated braille spinner (`⠋ thinking…` / `⠋ streaming · n chunks`) that
  advances on every ~100 ms redraw tick, so the UI never looks frozen while
  waiting for the first streamed chunk.
- TUI: chat input stays editable while a reply is streaming — typed
  characters append to the input box during thinking/streaming (Enter is
  still gated until the reply finishes).
- TUI: chat input editing matches opencode — ctrl+Backspace / ctrl+W
  delete exactly one previous word (whitespace-delimited, word by word on
  repeat), plain Backspace still deletes a single char.
- TUI: ctrl+L clears the chat transcript and resets the scroll on any
  screen (same path as `/clear`), staying on the current screen.
- TUI: error/status indicators render as plain colored text — `✗` replaced
  with an `error:` label in the danger style, `⏳`/`●` status glyphs dropped
  from the chat status line (list cursor `▸` and spinner kept).
- TUI: quit keys re-engineered to match opencode — ctrl+C toggles a
  centered quit-confirmation dialog (never quits directly), Esc cancels
  the dialog or backs out of screens, `q` is the only key that quits.

## [Unreleased]

### Added

- TUI: welcome screen on an empty chat — the aether logo (ascending triangle
  + `aether` wordmark) centered in the transcript with a
  `aether — plan · build · route` tagline and a muted hint line
  (`type a message to start · / for commands · ctrl+P for model`), so the
  first thing users see is the brand mark instead of an empty box.

### Changed

- TUI: palette semantics unified — one accent concept across the app: green
  `#22c55e` is the brand accent (header brand, selected tab, active stage),
  indigo `#818cf8` is the AI/agent color (ai labels, route panel, logo
  base), sky `#38bdf8` is info/titles, slate is borders/muted. `chrome.rs`
  no longer duplicates a conflicting indigo "accent" for the selected tab
  (now bold green + underlined).
- TUI: chat status/input/usage rows now align with the transcript column —
  the right usage/session sidebar is a full-height rail (opencode-style)
  spanning the transcript + status + input + usage rows instead of ending at
  the transcript, so the input box and usage bar share the transcript's
  left/right edges (footer stays full-width).
- TUI: header right group (model/provider/version/sessions) truncates
  gracefully with `…` on narrow terminals instead of being hard-clipped.
- TUI: agent panels show their live phase in the panel header
  (`PLAN · planning` / `BUILD · building` / `ROUTE · routing`) in the
  panel's role color, so the active stage is visible at a glance.
- TUI: session list token counts use compact units (`1.2K in / 3.4K out`)
  matching the sidebar, and long session titles truncate with `…`.
- TUI: palette models, slash-command labels and the add-model value truncate
  with `…` when they overflow; the chat input shows the tail of long typed
  text (most recent chars) with a leading `…` instead of clipping the end.

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
