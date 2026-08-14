# zen Provider — Privacy Notes

> What actually happens with your data when you use the default `zen` provider.
> Every claim below maps to a line of source — nothing is aspirational.

## 1. What is zen?

`zen` ("OpenCode Zen") is a free, OpenAI-compatible chat-completions endpoint
at `https://opencode.ai/zen/v1` (`ZEN_BASE_URL` in
`crates/aether-provider/src/registry.rs`). It is the default provider when no
other provider or API key is configured.

## 2. What is sent to the endpoint

For every `ask` / `chat` / `agent` / `tui` exchange, Aether sends a single
HTTP request:

```
POST https://opencode.ai/zen/v1/chat/completions
Content-Type: application/json
```

The JSON body contains (see `ChatRequest` in
`crates/aether-provider/src/client.rs`):

- `model` — the model id (e.g. `deepseek-v4-flash-free`)
- `messages` — the **conversation content**: your prompts, the model's
  replies, and — while an agent runs — tool results (file excerpts that were
  read, command output, and diffs of files the agent wrote)
- `tools` — the static tool registry descriptors (names + parameter
  schemas); tool *arguments* appear inline in `messages` as part of the
  conversation

**No API key is sent for free zen models.** `zen_provider()` sets
`key_env: None` and `needs_key: false`, so the client never attaches an
`Authorization` header for the default configuration
(`try_complete` / `try_open_stream` only call `bearer_auth` when `api_key`
is `Some`).

**`OPENCODE_ZEN_API_KEY` is only used when you opt into a paid zen model.**
If you configure a non-free zen model and set that env var, the key is sent
as a standard `Authorization: Bearer` header — same as any other provider's
key. Without the var, paid zen models fall back to a free model with a
warning (`resolve_default` in `registry.rs`).

## 3. What is NOT sent

- **No telemetry.** There is no analytics, no usage reporting, no beacon, no
  crash reporter anywhere in the workspace (grep `telemetry`, `analytics`,
  `posthog`, `sentry` — nothing).
- **No account or identity.** No user id, no email, no device fingerprint is
  generated or transmitted.
- **No config, keys, or session files.** Only the conversation payload above
  leaves your machine. Your config (`~/.config/aether/config.json`), session
  ledger, undo snapshots, and sync files stay local unless you run `session
  sync` to your own Gist/folder.

## 4. What stays on your machine

| Path | Contents |
|---|---|
| `~/.config/aether/config.json` | provider/model defaults (`AETHER_CONFIG_DIR` overrides the base) |
| `~/.config/aether/sessions/` | JSONL transcripts of every session |
| `<project>/.aether-undo/` | write snapshots for `agent undo` |
| `<project>/.aether-allowlist` | your per-project command allowlist |

See `config_dir()` / `sessions_dir()` in `crates/aether-core/src/config.rs`.
These files are plain text on your disk; nothing in Aether uploads them.

## 5. Who operates the endpoint

The zen endpoint is operated by the OpenCode project
(`https://opencode.ai/zen`). Data you send to it is subject to OpenCode's own
privacy policy and terms — Aether's privacy guarantees cover only what *this
program* transmits, which is strictly the request described in §2. Free zen
tiers may rate-limit or log requests server-side; that is outside Aether's
control.

## 6. How to avoid the zen endpoint entirely

- Set another provider: `OPENAI_API_KEY` for `openai`, or point `ollama` at a
  local server — no network egress beyond your own machine.
- Or use a custom provider in `config.json` (`api_key_env` + `base_url`).
- For fully offline operation, `ollama` with a local model sends nothing
  outside your host.

## 7. Verification

Run from the repo root:

```sh
# 1. No telemetry anywhere in the tree
grep -rniE "telemetry|analytics|posthog|sentry" crates/ || echo "clean"
# 2. zen carries no key by default
grep -n "key_env: None" crates/aether-provider/src/registry.rs
# 3. The only request Aether makes is chat/completions
grep -n "chat/completions" crates/aether-provider/src/client.rs
```
