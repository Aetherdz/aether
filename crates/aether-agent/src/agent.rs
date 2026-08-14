//! The 3-model agent loop: plan -> build -> route.
//!
//! - plan model: decomposes the task into a JSON plan
//! - build model: executes plan steps by calling tools
//! - route model: after each build round decides done / continue / revise
//!
//! The loop is generic over [`Completions`] so tests drive it with a mock
//! client (no network). Tool calls are consumed natively when the provider
//! returns `tool_calls`; otherwise the build reply is scanned for fenced
//! JSON tool invocations (provider-agnostic fallback).
//!
//! Safety: the loop stops early when it detects stagnation — two consecutive
//! iterations where the plan did not change AND no tool call modified the
//! filesystem — and reports why it stopped via [`AgentStopReason`].

use aether_core::error::{Error, Result};
use aether_provider::{ChatCompletion, ChatMessage, ChatRequest, ToolCall, ToolDef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::observer::{AgentPhase, DynObserver, NoopObserver, VerdictPhase};
use crate::plan::{Plan, extract_json_object, parse_plan};
use crate::prompts::{PLAN_SYSTEM, ROUTE_SYSTEM, build_system_with_tools, render_plan};
use crate::route::{Verdict, parse_verdict};
use crate::tools::Tools;

/// Checkpoint file name, written to the agent working directory after every
/// iteration so an interrupted run can be resumed with `Agent::resume`.
pub const STATE_FILE: &str = ".aether-agent-state.json";

/// Anything that can run a non-streaming completion.
#[async_trait::async_trait]
pub trait Completions: Send + Sync {
    async fn complete(&self, request: &ChatRequest) -> Result<ChatCompletion>;
}

#[async_trait::async_trait]
impl Completions for aether_provider::OpenAICompatibleClient {
    async fn complete(&self, request: &ChatRequest) -> Result<ChatCompletion> {
        self.complete(request).await
    }
}

/// Why an agent run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStopReason {
    /// The router said done.
    Done,
    /// `max_iterations` was reached.
    IterationCap,
    /// Two consecutive iterations with no observable progress.
    Stagnation,
    /// The build model exhausted its per-iteration tool budget.
    BuildTurnCap,
}

/// Outcome of one full agent run.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub plan: Plan,
    pub iterations: u32,
    pub tool_calls: u32,
    pub final_answer: String,
    pub stopped_reason: AgentStopReason,
}

/// The 3-model agent. Each role can use a different model id.
pub struct Agent {
    client: Box<dyn Completions>,
    plan_model: String,
    build_model: String,
    route_model: String,
    tools: Tools,
    root: PathBuf,
    max_iterations: u32,
    max_build_turns: u32,
    observer: DynObserver,
}

/// Serializable checkpoint of an in-flight agent run, written to
/// [`STATE_FILE`] under the working directory after every completed
/// iteration. A crash or Ctrl-C never loses more than one iteration:
/// [`Agent::resume`] reloads this state and continues from
/// `iteration + 1` with the same plan, feedback history and stagnation
/// counters. Terminal outcomes remove the checkpoint file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub task: String,
    pub plan: Plan,
    /// Iterations completed so far; `0` means only the plan exists.
    pub iteration: u32,
    pub tool_calls: u32,
    pub final_answer: String,
    /// Continue/revise feedback history fed back to the build model.
    pub history: Vec<ChatMessage>,
    pub prev_plan: Option<Plan>,
    pub stagnant_streak: u32,
}

impl AgentState {
    /// Read the checkpoint from `root/.aether-agent-state.json`, if present.
    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(STATE_FILE);
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).map(Some).map_err(|e| {
                Error::InvalidInput(format!("corrupt state file {}: {e}", path.display()))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// Wire shape of a fenced tool invocation inside build text.
#[derive(serde::Deserialize)]
struct FencedCall {
    #[serde(default)]
    tool: String,
    #[serde(default)]
    args: Value,
}

impl Agent {
    pub fn new(
        client: Box<dyn Completions>,
        root: std::path::PathBuf,
        plan_model: String,
        build_model: String,
        route_model: String,
    ) -> Self {
        Self {
            client,
            plan_model,
            build_model,
            route_model,
            tools: Tools::new(root.clone()),
            root,
            max_iterations: 6,
            max_build_turns: 12,
            observer: std::sync::Arc::new(NoopObserver),
        }
    }

    /// Report live phase transitions to `observer` (e.g. a TUI channel).
    /// The observer is fire-and-forget: a slow consumer never stalls the loop.
    pub fn with_observer(mut self, observer: DynObserver) -> Self {
        self.observer = observer;
        self
    }

    fn emit(&self, phase: AgentPhase) {
        self.observer.on_phase(phase);
    }

    pub fn with_iteration_cap(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn with_build_turn_cap(mut self, max: u32) -> Self {
        self.max_build_turns = max;
        self
    }

    /// Enable/disable the `write_file` confirmation gate (see [`Tools::with_confirm`]).
    pub fn with_confirm(mut self, enabled: bool) -> Self {
        self.tools = self.tools.with_confirm(enabled);
        self
    }

    /// Auto-approve all `write_file` calls (non-interactive `--yes`).
    pub fn with_yes(mut self) -> Self {
        self.tools = self.tools.with_yes();
        self
    }

    /// Run the loop on `task` until the router says done or caps are hit.
    ///
    /// The initial plan is persisted to [`STATE_FILE`] immediately, then the
    /// checkpoint is rewritten after every iteration. A clean terminal
    /// outcome removes it; an error or kill leaves it behind for [`resume`].
    ///
    /// [`resume`]: Self::resume
    pub async fn run(&self, task: &str) -> Result<AgentResult> {
        self.emit(AgentPhase::Planning);
        let plan = self.plan(task).await?;
        self.emit(AgentPhase::PlanReady(render_plan(&plan)));
        let state = AgentState {
            task: task.to_string(),
            plan,
            iteration: 0,
            tool_calls: 0,
            final_answer: String::new(),
            history: Vec::new(),
            prev_plan: None,
            stagnant_streak: 0,
        };
        self.persist(&state);
        self.run_from(state).await
    }

    /// Continue a previously interrupted run from its [`AgentState`]
    /// checkpoint, picking up at `state.iteration + 1` with the same plan,
    /// history and stagnation counters.
    pub async fn resume(&self, state: AgentState) -> Result<AgentResult> {
        self.run_from(state).await
    }

    /// Shared loop body: `run` seeds it with a fresh state, `resume` reuses
    /// a persisted one. The checkpoint is written after each iteration.
    async fn run_from(&self, mut state: AgentState) -> Result<AgentResult> {
        let task = state.task.clone();
        let mut plan = state.plan.clone();
        let mut history = std::mem::take(&mut state.history);
        let mut tool_calls = state.tool_calls;
        let mut final_answer = state.final_answer.clone();
        let mut prev_plan = state.prev_plan.clone();
        let mut stagnant_streak = state.stagnant_streak;

        for iteration in state.iteration + 1..=self.max_iterations {
            self.emit(AgentPhase::BuildStarted { iteration });
            let build = self.build_round(iteration, &task, &plan, &history).await?;
            tool_calls += build.tool_calls;
            final_answer = build.summary.clone();
            self.emit(AgentPhase::BuildFinished {
                iteration,
                tool_calls: build.tool_calls,
                modified: build.modified,
                summary: build.summary.clone(),
            });

            // The build model spent its whole tool budget without producing
            // a summary — there is nothing more to route on.
            if build.capped {
                self.clear_state();
                self.emit(AgentPhase::Finished {
                    iterations: iteration,
                    tool_calls,
                    reason: AgentStopReason::BuildTurnCap,
                });
                return Ok(AgentResult {
                    plan,
                    iterations: iteration,
                    tool_calls,
                    final_answer,
                    stopped_reason: AgentStopReason::BuildTurnCap,
                });
            }

            // Stagnation detection: no observable progress means the plan is
            // identical to the previous iteration AND no tool call modified
            // any file this iteration. Two consecutive stagnant iterations
            // stop the loop early.
            let plan_unchanged = prev_plan.as_ref() == Some(&plan);
            let no_files_modified = !build.modified;
            if plan_unchanged && no_files_modified {
                stagnant_streak += 1;
            } else {
                stagnant_streak = 0;
            }
            if stagnant_streak >= 2 {
                self.clear_state();
                self.emit(AgentPhase::Finished {
                    iterations: iteration,
                    tool_calls,
                    reason: AgentStopReason::Stagnation,
                });
                return Ok(AgentResult {
                    plan,
                    iterations: iteration,
                    tool_calls,
                    final_answer,
                    stopped_reason: AgentStopReason::Stagnation,
                });
            }
            prev_plan = Some(plan.clone());

            self.emit(AgentPhase::Routing { iteration });
            let verdict = self.route(&task, &plan, &build.summary, &history).await?;
            self.emit(AgentPhase::Routed {
                iteration,
                verdict: verdict_phase(&verdict),
            });
            match verdict {
                Verdict::Done(answer) => {
                    self.clear_state();
                    self.emit(AgentPhase::Finished {
                        iterations: iteration,
                        tool_calls,
                        reason: AgentStopReason::Done,
                    });
                    return Ok(AgentResult {
                        plan,
                        iterations: iteration,
                        tool_calls,
                        final_answer: if answer.is_empty() {
                            build.summary
                        } else {
                            answer
                        },
                        stopped_reason: AgentStopReason::Done,
                    });
                }
                Verdict::Continue(feedback) => {
                    history.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!("Continue: {feedback}"),
                        ..ChatMessage::default()
                    });
                }
                Verdict::Revise(new_plan, feedback) => {
                    history.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!(
                            "Revise plan: {feedback}\nNew plan:\n{}",
                            render_plan(&new_plan)
                        ),
                        ..ChatMessage::default()
                    });
                    plan = new_plan;
                }
            }

            state.iteration = iteration;
            state.plan = plan.clone();
            state.history = history.clone();
            state.tool_calls = tool_calls;
            state.final_answer = final_answer.clone();
            state.prev_plan = prev_plan.clone();
            state.stagnant_streak = stagnant_streak;
            self.persist(&state);
        }

        self.clear_state();
        self.emit(AgentPhase::Finished {
            iterations: self.max_iterations,
            tool_calls,
            reason: AgentStopReason::IterationCap,
        });
        Ok(AgentResult {
            plan,
            iterations: self.max_iterations,
            tool_calls,
            final_answer,
            stopped_reason: AgentStopReason::IterationCap,
        })
    }

    fn state_path(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    fn persist(&self, state: &AgentState) {
        if let Ok(raw) = serde_json::to_string_pretty(state) {
            let _ = std::fs::write(self.state_path(), raw);
        }
    }

    fn clear_state(&self) {
        let _ = std::fs::remove_file(self.state_path());
    }

    /// Role 1: produce the plan.
    async fn plan(&self, task: &str) -> Result<Plan> {
        let request = self.request(
            &self.plan_model,
            vec![system(PLAN_SYSTEM), user(task)],
            None,
        );
        let reply = self.client.complete(&request).await?;
        let text = assistant_text(&reply);
        parse_plan(&text).map_err(|_| {
            Error::InvalidInput(format!("plan model returned unparseable output: {text}"))
        })
    }

    /// Role 2: execute plan steps, returning the final summary + tool stats.
    /// Emits `AgentPhase::ToolCalled` (with the write diff, if any) for
    /// every tool call so the TUI can show live file changes.
    async fn build_round(
        &self,
        iteration: u32,
        task: &str,
        plan: &Plan,
        history: &[ChatMessage],
    ) -> Result<BuildRound> {
        let mut messages: Vec<ChatMessage> = vec![
            system(&build_system_with_tools()),
            user(&format!(
                "Task: {task}\n\nPlan:\n{}\n\nTool results so far are appended below.",
                render_plan(plan)
            )),
        ];
        messages.extend_from_slice(history);

        let tool_defs = Tools::defs();
        let mut round_tool_calls: u32 = 0;
        let mut round_modified: bool = false;
        let mut summary = String::new();

        for _ in 0..self.max_build_turns {
            let request =
                self.request(&self.build_model, messages.clone(), Some(tool_defs.clone()));
            let reply = self.client.complete(&request).await?;

            let native = reply
                .choices
                .first()
                .and_then(|c| c.message.tool_calls.clone())
                .unwrap_or_default();

            if !native.is_empty() {
                for call in &native {
                    let (result, modified, diff) = self.execute_tool_call(call).await;
                    round_tool_calls += 1;
                    round_modified |= modified;
                    self.emit(AgentPhase::ToolCalled {
                        iteration,
                        name: call.function.name.clone(),
                        diff,
                    });
                    messages.push(assistant_tool_calls(vec![call.clone()]));
                    messages.push(tool_result(&call.id, &result));
                }
                continue;
            }

            let text = assistant_text(&reply);
            let fenced = extract_fenced_calls(&text)?;
            if !fenced.is_empty() {
                for call in fenced {
                    let (result, modified, diff) = match self.tools.call(&call.tool, &call.args) {
                        Ok(r) => (r.text, r.modified, r.diff),
                        Err(e) => (e.to_string(), false, None),
                    };
                    round_tool_calls += 1;
                    round_modified |= modified;
                    self.emit(AgentPhase::ToolCalled {
                        iteration,
                        name: call.tool.clone(),
                        diff,
                    });
                    messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: text.clone(),
                        ..ChatMessage::default()
                    });
                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: result,
                        tool_call_id: Some("fenced".to_string()),
                        ..ChatMessage::default()
                    });
                }
                continue;
            }

            summary = text;
            break;
        }

        let capped = summary.is_empty();
        Ok(BuildRound {
            summary,
            tool_calls: round_tool_calls,
            modified: round_modified,
            // If the loop ran out of turns, summary was never set.
            capped,
        })
    }

    async fn execute_tool_call(&self, call: &ToolCall) -> (String, bool, Option<String>) {
        let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);
        match self.tools.call(&call.function.name, &args) {
            Ok(r) => (r.text, r.modified, r.diff),
            Err(e) => (e.to_string(), false, None),
        }
    }

    /// Role 3: judge the build round.
    async fn route(
        &self,
        task: &str,
        plan: &Plan,
        summary: &str,
        history: &[ChatMessage],
    ) -> Result<Verdict> {
        let messages = vec![
            system(ROUTE_SYSTEM),
            user(&format!(
                "Task: {task}\n\nPlan:\n{}\n\nBuilder summary:\n{summary}\n\nHistory:\n{}",
                render_plan(plan),
                render_history(history)
            )),
        ];
        let request = self.request(&self.route_model, messages.clone(), None);
        let reply = self.client.complete(&request).await?;
        let text = assistant_text(&reply);
        parse_verdict(&text)
            .map_err(|_| Error::InvalidInput(format!("router returned unparseable output: {text}")))
    }

    fn request(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
    ) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            messages,
            temperature: None,
            stream: false,
            tools,
        }
    }
}

struct BuildRound {
    summary: String,
    tool_calls: u32,
    modified: bool,
    capped: bool,
}

fn system(content: &str) -> ChatMessage {
    ChatMessage {
        role: "system".to_string(),
        content: content.to_string(),
        ..ChatMessage::default()
    }
}

fn user(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: content.to_string(),
        ..ChatMessage::default()
    }
}

fn tool_result(call_id: &str, text: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".to_string(),
        content: text.to_string(),
        tool_call_id: Some(call_id.to_string()),
        ..ChatMessage::default()
    }
}

fn assistant_tool_calls(calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(calls),
        ..ChatMessage::default()
    }
}

fn assistant_text(completion: &ChatCompletion) -> String {
    completion
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default()
}

fn render_history(history: &[ChatMessage]) -> String {
    history
        .iter()
        .map(|m| format!("[{}] {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Scan build text for fenced JSON tool invocations.
fn extract_fenced_calls(text: &str) -> Result<Vec<FencedCall>> {
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(obj) = extract_json_object(rest) {
        if let Ok(call) = serde_json::from_str::<FencedCall>(obj)
            && !call.tool.is_empty()
        {
            calls.push(call);
        }
        let end = rest.find(obj).map(|i| i + obj.len()).unwrap_or(rest.len());
        rest = &rest[end..];
    }
    Ok(calls)
}

fn verdict_phase(verdict: &Verdict) -> VerdictPhase {
    match verdict {
        Verdict::Done(_) => VerdictPhase::Done,
        Verdict::Continue(_) => VerdictPhase::Continue,
        Verdict::Revise(_, _) => VerdictPhase::Revise,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_provider::{CompletionChoice, CompletionMessage};
    use std::sync::{Arc, Mutex};

    /// A scripted mock: each `complete` returns the next queued completion.
    #[derive(Clone)]
    struct MockClient {
        replies: Arc<Mutex<Vec<ChatCompletion>>>,
    }

    impl MockClient {
        fn new(replies: Vec<ChatCompletion>) -> Self {
            Self {
                replies: Arc::new(Mutex::new(replies)),
            }
        }
        fn text(content: &str) -> ChatCompletion {
            ChatCompletion {
                choices: vec![CompletionChoice {
                    message: CompletionMessage {
                        role: "assistant".to_string(),
                        content: Some(content.to_string()),
                        tool_calls: None,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: None,
            }
        }
        fn with_tools(calls: Vec<ToolCall>) -> ChatCompletion {
            ChatCompletion {
                choices: vec![CompletionChoice {
                    message: CompletionMessage {
                        role: "assistant".to_string(),
                        content: None,
                        tool_calls: Some(calls),
                    },
                    finish_reason: Some("tool_calls".to_string()),
                }],
                usage: None,
            }
        }
    }

    #[async_trait::async_trait]
    impl Completions for MockClient {
        async fn complete(&self, _request: &ChatRequest) -> Result<ChatCompletion> {
            let mut guard = self.replies.lock().unwrap();
            if guard.is_empty() {
                return Err(Error::InvalidInput("mock exhausted".to_string()));
            }
            Ok(guard.remove(0))
        }
    }

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-agent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            kind: "function".to_string(),
            function: aether_provider::ToolCallFunction {
                name: name.to_string(),
                arguments: args.to_string(),
            },
        }
    }

    #[test]
    fn extracts_fenced_calls() {
        let text = r#"Let me check.
{"tool":"read_file","args":{"path":"a.txt"}}
Then:
```json
{"tool":"list_dir","args":{"path":""}}
```
Done"#;
        let calls = extract_fenced_calls(text).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool, "read_file");
        assert_eq!(calls[1].tool, "list_dir");
    }

    #[tokio::test]
    async fn full_loop_plan_build_route_done() {
        let root = temp_root("done");
        std::fs::write(root.join("x.txt"), "hello world").unwrap();
        let mock = MockClient::new(vec![
            // plan
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read x.txt"}]}"#),
            // build: native tool call
            MockClient::with_tools(vec![tool_call("c1", "read_file", r#"{"path":"x.txt"}"#)]),
            // build: final summary (tool result fed back)
            MockClient::text("file contains hello world"),
            // route: done
            MockClient::text(r#"{"verdict":"done","final_answer":"hello world"}"#),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        );
        let result = agent.run("read x.txt").await.unwrap();
        assert_eq!(result.iterations, 1);
        assert_eq!(result.tool_calls, 1);
        assert_eq!(result.final_answer, "hello world");
        assert_eq!(result.stopped_reason, AgentStopReason::Done);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn loop_emits_phases_in_order() {
        use crate::observer::ChannelObserver;
        let root = temp_root("phases");
        std::fs::write(root.join("x.txt"), "hello world").unwrap();
        let mock = MockClient::new(vec![
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read x.txt"}]}"#),
            MockClient::with_tools(vec![tool_call("c1", "read_file", r#"{"path":"x.txt"}"#)]),
            MockClient::text("file contains hello world"),
            MockClient::text(r#"{"verdict":"done","final_answer":"hello world"}"#),
        ]);
        let (tx, rx) = std::sync::mpsc::channel();
        let result = {
            let agent = Agent::new(
                Box::new(mock),
                root.clone(),
                "m".to_string(),
                "m".to_string(),
                "m".to_string(),
            )
            .with_observer(std::sync::Arc::new(ChannelObserver(tx)));
            agent.run("read x.txt").await.unwrap()
        };
        assert_eq!(result.iterations, 1);
        let phases: Vec<AgentPhase> = rx.iter().collect();
        assert_eq!(phases[0], AgentPhase::Planning);
        assert!(matches!(&phases[1], AgentPhase::PlanReady(text) if text.contains("goal")));
        assert_eq!(phases[2], AgentPhase::BuildStarted { iteration: 1 });
        assert_eq!(
            phases[3],
            AgentPhase::ToolCalled {
                iteration: 1,
                name: "read_file".to_string(),
                diff: None
            }
        );
        assert!(matches!(
            &phases[4],
            AgentPhase::BuildFinished {
                iteration: 1,
                modified: false,
                ..
            }
        ));
        assert_eq!(phases[5], AgentPhase::Routing { iteration: 1 });
        assert_eq!(
            phases[6],
            AgentPhase::Routed {
                iteration: 1,
                verdict: VerdictPhase::Done
            }
        );
        assert!(matches!(
            &phases[7],
            AgentPhase::Finished {
                iterations: 1,
                reason: AgentStopReason::Done,
                ..
            }
        ));
        assert_eq!(phases.len(), 8);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn fenced_fallback_works_without_native_tools() {
        let root = temp_root("fenced");
        std::fs::write(root.join("y.txt"), "data").unwrap();
        let mock = MockClient::new(vec![
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read"}]}"#),
            // build returns fenced tool call in text
            MockClient::text(r#"Reading now. {"tool":"read_file","args":{"path":"y.txt"}}"#),
            // build summary after tool result
            MockClient::text("content is data"),
            // route done
            MockClient::text(r#"{"verdict":"done","final_answer":"data"}"#),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        );
        let result = agent.run("read y.txt").await.unwrap();
        assert_eq!(result.tool_calls, 1);
        assert_eq!(result.final_answer, "data");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn revise_loop_replaces_plan() {
        let root = temp_root("revise");
        std::fs::write(root.join("a.txt"), "A").unwrap();
        let mock = MockClient::new(vec![
            // plan v1
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read a.txt"}]}"#),
            // build: no tools, plain summary
            MockClient::text("nothing yet"),
            // route: revise with plan v2
            MockClient::text(
                r#"{"verdict":"revise","feedback":"read it",
                "revised_plan":{"goal":"g","steps":[{"id":1,"action":"read a.txt"}]}}"#,
            ),
            // build: tool call
            MockClient::with_tools(vec![tool_call("c2", "read_file", r#"{"path":"a.txt"}"#)]),
            // build: summary
            MockClient::text("content A"),
            // route: done
            MockClient::text(r#"{"verdict":"done","final_answer":"A"}"#),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        );
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.iterations, 2);
        assert_eq!(result.final_answer, "A");
        assert_eq!(result.stopped_reason, AgentStopReason::Done);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn iteration_cap_returns_partial() {
        let root = temp_root("cap");
        let mock = MockClient::new(vec![
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"a"}]}"#),
            MockClient::text("still working"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            MockClient::text("still working 2"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more 2"}"#),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(2);
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.iterations, 2);
        assert_eq!(result.final_answer, "still working 2");
        assert_eq!(result.stopped_reason, AgentStopReason::IterationCap);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two consecutive iterations with an unchanged plan and no file
    /// modifications must stop the loop with `Stagnation`.
    #[tokio::test]
    async fn stagnation_detected_after_two_identical_iterations() {
        let root = temp_root("stagnate");
        let mock = MockClient::new(vec![
            // plan
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"work"}]}"#),
            // iter 1: no tools, plain summary; route says continue
            MockClient::text("working on it"),
            MockClient::text(r#"{"verdict":"continue","feedback":"keep going"}"#),
            // iter 2: still no tools -> stagnant streak 1
            MockClient::text("still working"),
            MockClient::text(r#"{"verdict":"continue","feedback":"keep going"}"#),
            // iter 3: still no tools -> stagnant streak 2 -> stop
            MockClient::text("still working again"),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(6);
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.stopped_reason, AgentStopReason::Stagnation);
        assert_eq!(result.iterations, 3);
        assert_eq!(result.final_answer, "still working again");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// File modifications reset the stagnation streak even when the plan
    /// stays identical: it must take two *consecutive* stagnant iterations.
    #[tokio::test]
    async fn stagnation_resets_when_files_modified() {
        let root = temp_root("stagnatereset");
        let mock = MockClient::new(vec![
            // plan
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"write"}]}"#),
            // iter 1: write a file (modified) -> streak 0
            MockClient::with_tools(vec![tool_call(
                "c1",
                "write_file",
                r#"{"path":"a.txt","content":"v1"}"#,
            )]),
            MockClient::text("wrote a.txt"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            // iter 2: write again (modified) -> streak 0
            MockClient::with_tools(vec![tool_call(
                "c2",
                "write_file",
                r#"{"path":"a.txt","content":"v2"}"#,
            )]),
            MockClient::text("wrote a.txt again"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            // iter 3: no tools -> streak 1
            MockClient::text("nothing to change"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            // iter 4: no tools -> streak 2 -> Stagnation
            MockClient::text("nothing to change again"),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(6);
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.stopped_reason, AgentStopReason::Stagnation);
        assert_eq!(result.iterations, 4);
        // The two writes actually landed.
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "v2");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A revised plan resets the plan-unchanged condition.
    #[tokio::test]
    async fn stagnation_does_not_fire_across_plan_revision() {
        let root = temp_root("stagrevise");
        let mock = MockClient::new(vec![
            // plan v1
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"a"}]}"#),
            // iter 1: no tools; route revises to plan v2
            MockClient::text("hmm"),
            MockClient::text(
                r#"{"verdict":"revise","feedback":"try b",
                "revised_plan":{"goal":"g","steps":[{"id":1,"action":"b"}]}}"#,
            ),
            // iter 2: plan changed -> not stagnant; route continues
            MockClient::text("trying b"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            // iter 3: plan unchanged now, no tools -> streak 1
            MockClient::text("still b"),
            MockClient::text(r#"{"verdict":"continue","feedback":"more"}"#),
            // iter 4: streak 2 -> Stagnation
            MockClient::text("still b again"),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(6);
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.stopped_reason, AgentStopReason::Stagnation);
        // Revision happened at iter 1; stagnation needs iters 3+4 unchanged.
        assert_eq!(result.iterations, 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn resume_continues_after_crash() {
        let root = temp_root("resume");
        std::fs::write(root.join("f.txt"), "data").unwrap();
        // Plan + iter 1 (build+route) + iter 2 (build+route); the 3rd build
        // call hits the exhausted mock -> provider error simulates a crash.
        let crash_replies = vec![
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read f.txt"}]}"#),
            MockClient::text("working 1"),
            MockClient::text(r#"{"verdict":"continue","feedback":"keep going"}"#),
            MockClient::text("working 2"),
            MockClient::text(r#"{"verdict":"continue","feedback":"keep going"}"#),
        ];
        let agent = Agent::new(
            Box::new(MockClient::new(crash_replies)),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(6);
        let err = agent.run("task").await.unwrap_err();
        assert!(
            err.to_string().contains("mock exhausted"),
            "expected provider crash, got {err}"
        );

        let state = AgentState::load(&root)
            .unwrap()
            .expect("state file must survive the crash");
        assert_eq!(state.iteration, 2);
        assert_eq!(state.plan.goal, "g");
        assert_eq!(state.history.len(), 2);
        // Two stagnant iterations (no tools, unchanged plan) before the crash.
        assert_eq!(state.stagnant_streak, 1);
        assert_eq!(state.prev_plan, Some(state.plan.clone()));

        // Resume with a fresh client: iteration counter continues at 3, and
        // the file modification resets the carried-over stagnation streak.
        let resume_replies = vec![
            MockClient::with_tools(vec![tool_call(
                "c3",
                "write_file",
                r#"{"path":"f.txt","content":"v3"}"#,
            )]),
            MockClient::text("wrote f.txt"),
            MockClient::text(r#"{"verdict":"done","final_answer":"finished"}"#),
        ];
        let agent2 = Agent::new(
            Box::new(MockClient::new(resume_replies)),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        )
        .with_iteration_cap(6);
        let result = agent2.resume(state).await.unwrap();
        assert_eq!(result.stopped_reason, AgentStopReason::Done);
        assert_eq!(result.iterations, 3);
        assert_eq!(result.tool_calls, 1);
        assert_eq!(result.final_answer, "finished");
        assert!(
            !root.join(STATE_FILE).exists(),
            "checkpoint must be removed on clean finish"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn clean_run_removes_checkpoint() {
        let root = temp_root("cleanstate");
        std::fs::write(root.join("f.txt"), "data").unwrap();
        let mock = MockClient::new(vec![
            MockClient::text(r#"{"goal":"g","steps":[{"id":1,"action":"read f.txt"}]}"#),
            MockClient::text("working"),
            MockClient::text(r#"{"verdict":"done","final_answer":"data"}"#),
        ]);
        let agent = Agent::new(
            Box::new(mock),
            root.clone(),
            "m".to_string(),
            "m".to_string(),
            "m".to_string(),
        );
        let result = agent.run("task").await.unwrap();
        assert_eq!(result.stopped_reason, AgentStopReason::Done);
        assert!(
            !root.join(STATE_FILE).exists(),
            "checkpoint must be removed after a clean run"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn state_load_missing_returns_none() {
        let root = temp_root("nostate");
        assert!(AgentState::load(&root).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
