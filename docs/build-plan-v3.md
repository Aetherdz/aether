# Aether v0.2 — Build Plan: OpenCode-grade infrastructure, jcode-grade speed

Reference targets, measured 2026-08-09:

| Metric | Aether now | OpenCode | jcode |
|---|---|---|---|
| Startup (--version) | 5-49 ms | ~1 s (Node) | ~14 ms |
| Binary size | 5.2 MB | n/a (Node) | 28 MB |
| Crate count | 8 | n/a (TS monorepo) | 1 |
| LOC | 8030 | n/a | n/a |

Aether already beats OpenCode on startup/size (Rust vs Node) and is at jcode
parity on startup. The gap is architectural: OpenCode-grade agent hierarchy,
subagents, skills, permissions, cost transparency, and jcode-grade memory,
swarm, and local-model ergonomics.

## Phase order (dependency-driven, each ends green)

### P1 — OpenCode infrastructure (foundation)

1. **Agent-loop session persistence** — `crates/aether-agent/src/agent.rs`
   currently runs plan->build->route in memory; `cmd_agent` prints and exits.
   Nothing survives a crash or Ctrl-C. Add:
   - `AgentState` serializable snapshot (plan, steps, iteration count, tool-call
     log, partial final_answer) written after every iteration to
     `<cwd>/.aether-agent-state.json` via `aether-session::Ledger`.
   - `Agent::resume(state)` — reloads plan + tool log, continues from
     `iteration+1`.
   - `aether agent --resume` CLI flag in `crates/aether-cli/src/main.rs`
     `cmd_agent`.
   - Test: run 2 iterations with a mock client, kill, resume, assert iteration
     counter continues and plan is identical.
   - Failure-injection tie-in (Phase 7): malformed provider response mid-loop
     leaves state consistent — resume test covers it.

2. **Pre-call cost/latency transparency** — `crates/aether-provider/src/registry.rs`
   has static model lists but no pricing table. Add:
   - `ModelPricing { input_per_mtok, output_per_mtok }` in
     `aether-core/src/config.rs` (or new `crates/aether-core/src/pricing.rs`),
     populated for builtin providers (zen free, openai, anthropic, google,
     deepseek, openrouter via their published per-Mtok rates).
   - `estimate_cost(text_chars, model) -> (usd, est_seconds)` shown BEFORE
     `aether agent` runs (in `cmd_agent`), and required-confirm if
     `estimate > config.agent.cost_threshold` (default $0.50).
   - `aether use` prints the per-model rate so the tradeoff is visible without
     running a task.
   - Tests: pricing table has no zero/negative entries; estimator matches a
     hand-computed example; threshold gate blocks and unblocks.

3. **Spending caps (hard stop)** — extend `AetherConfig`:
   - `agent.session_cap_usd`, `agent.daily_cap_usd`; daily tracked in
     `Ledger` (already aggregates tokens) — convert tokens->USD via pricing.
   - `Agent` checks cap before each LLM call; on breach returns
     `AgentStopReason::SpendCap` and refuses further calls.
   - Test: mock client with known usage; set tiny cap; assert loop stops and
     reason is SpendCap, ledger unchanged afterwards.

4. **Permission model** — `crates/aether-agent/src/tools.rs` has confirm gate
   for writes and `run_command` always executes. Port the OpenCode/Claude
   model:
   - `Permission { read_only, write_confirm, command_confirm }` with
     `aether agent --permission read-only|ask|auto` (persistent default in
     config, `aether use`/`aether config` to set).
   - `run_command` gets its own confirm gate (currently only writes ask);
     destructive-pattern list (`rm -rf`, `git push --force`, `curl | sh`,
     `mkfs`, `dd`) requires confirmation even in `auto` unless `--yes`.
   - SECURITY.md already documents intent; code must now match it. This closes
     the Phase-1 audit item 5.

### P2 — Subagents + skills (OpenCode agent hierarchy)

5. **Subagent delegation** — new `crates/aether-subagent/`:
   - `SubagentSpec { name, system_prompt, tools: &[&str], max_context }`.
   - `spawn_subagent(spec, task) -> JoinHandle<SubagentReport>`; report =
     `{ summary, files_touched, token_usage }` back to the main loop without
     polluting its context.
   - Agent tool `delegate(task, role)` registered in `tools.rs` — plan model
     can fan out a step to an explorer/reviewer subagent and get a summary.
   - Context isolation: each subagent runs its own `Agent` with its own
     history; only the report enters the parent.
   - Tests: mock client — assert parent history contains only the report,
     not the subagent's internal messages; files_touched propagate.

6. **Skills system** — `crates/aether-core/src/skills.rs`:
   - `Skill { name, description, trigger, body }` loaded from
     `~/.config/aether/skills/*/SKILL.md` (jcode/OpenCode layout).
   - `aether skills list`, `aether skills add <path>`; `agent` auto-loads
     skills whose trigger matches the task string into the plan-model system
     prompt.
   - Tests: parse a fixture SKILL.md; trigger matching picks the right skill;
     unknown dir is a clean error.

### P3 — jcode-grade memory + swarm

7. **Semantic memory** — new `crates/aether-memory/`:
   - Per-session `memories.jsonl`: `{ text, embedding, ts }`; embed via a
     cheap local default (e.g. all-MiniLM via ONNX runtime, optional) or a
     configured provider endpoint.
   - On each agent turn, cosine-similarity top-k memories are injected into
     the system prompt (`relevant context:` block). No token-burning recap —
     the graph just recalls.
   - `aether recall --semantic "phrase"` returns top matches (upgrade the
     existing substring `cmd_recall`).
   - Tests: pure cosine ranking over a fixed embedding set; injection only
     when similarity > threshold; empty store is a no-op.

8. **Swarm mode** — `crates/aether-agent/src/swarm.rs`:
   - `aether swarm "task" --agents N` — N parallel `Agent` instances, each a
     separate process/session with its own working copy of the plan.
   - File-conflict notification: a shared `files.lock` journal
     (append-only JSONL of {agent, file, mtime}); on write, agent checks
     whether another agent touched the file since its read and surfaces a
     warning instead of silently clobbering (jcode's notified-conflict
     behavior).
   - Tests: two agents write the same file in sequence; second sees the
     conflict warning; journal is append-safe under concurrent writes.

### P4 — Local models + polish

9. **Local model first-class** — `registry.rs` already has `ollama` and
   `llama.cpp` URL mapping. Finish parity:
   - `aether models ollama` lists running models via
     `GET http://localhost:11434/api/tags`; `aether use ollama/<model>` works
     without a key (already keyless).
   - `aether doctor` — health check: ollama reachable?, llama.cpp port open?,
     provider keys set?, config valid? single command, actionable output.
   - Tests: model-list parsing from a canned JSON body; keyless resolution.

10. **Perf pass** — binary is 5.2 MB and boots in <50 ms; lock it in:
    - `#[cfg(debug_assertions)]` on test-only paths; ensure `--version` and
      `--help` do zero provider/network init (verify with strace).
    - Add startup-time test (assert <150 ms) in CI `golden` job.

## Definition of done per item

Every item: code compiles, `cargo test` green (incl. its new test), golden
suite green, and a one-line entry added to `docs/CHANGELOG.md`. No item ships
without its proving test (Phase 5 rule).

## Verification order

After each phase, run: `cargo build && cargo test && AETHER_BIN=target/release/aether bash golden-tests/run.sh`.
