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

use aether_core::error::{Error, Result};
use aether_provider::{
    ChatCompletion, ChatMessage, ChatRequest, ToolCall, ToolDef,
};
use serde_json::Value;

use crate::plan::{extract_json_object, parse_plan, Plan};
use crate::prompts::{build_system_with_tools, render_plan, PLAN_SYSTEM, ROUTE_SYSTEM};
use crate::route::{parse_verdict, Verdict};
use crate::tools::Tools;

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

/// Outcome of one full agent run.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub plan: Plan,
    pub iterations: u32,
    pub tool_calls: u32,
    pub final_answer: String,
}

/// The 3-model agent. Each role can use a different model id.
pub struct Agent {
    client: Box<dyn Completions>,
    plan_model: String,
    build_model: String,
    route_model: String,
    tools: Tools,
    max_iterations: u32,
    max_build_turns: u32,
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
            tools: Tools::new(root),
            max_iterations: 6,
            max_build_turns: 12,
        }
    }

    pub fn with_iteration_cap(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn with_build_turn_cap(mut self, max: u32) -> Self {
        self.max_build_turns = max;
        self
    }

    /// Run the loop on `task` until the router says done or caps are hit.
    pub async fn run(&self, task: &str) -> Result<AgentResult> {
        let plan = self.plan(task).await?;
        let mut plan = plan;
        let mut history: Vec<ChatMessage> = Vec::new();
        let mut tool_calls: u32 = 0;
        let mut final_answer = String::new();

        for iteration in 1..=self.max_iterations {
            let build = self.build_round(task, &plan, &history).await?;
            tool_calls += build.tool_calls;
            final_answer = build.summary.clone();

            let verdict = self.route(task, &plan, &build.summary, &history).await?;
            match verdict {
                Verdict::Done(answer) => {
                    return Ok(AgentResult {
                        plan,
                        iterations: iteration,
                        tool_calls,
                        final_answer: if answer.is_empty() {
                            build.summary
                        } else {
                            answer
                        },
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
                        content: format!("Revise plan: {feedback}\nNew plan:\n{}", render_plan(&new_plan)),
                        ..ChatMessage::default()
                    });
                    plan = new_plan;
                }
            }
        }

        Ok(AgentResult {
            plan,
            iterations: self.max_iterations,
            tool_calls,
            final_answer,
        })
    }

    /// Role 1: produce the plan.
    async fn plan(&self, task: &str) -> Result<Plan> {
        let request = self.request(
            &self.plan_model,
            vec![
                system(PLAN_SYSTEM),
                user(task),
            ],
            None,
        );
        let reply = self.client.complete(&request).await?;
        let text = assistant_text(&reply);
        parse_plan(&text).map_err(|_| {
            Error::InvalidInput(format!("plan model returned unparseable output: {text}"))
        })
    }

    /// Role 2: execute plan steps, returning the final summary + tool stats.
    async fn build_round(
        &self,
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
        let mut summary = String::new();

        for _ in 0..self.max_build_turns {
            let request = self.request(&self.build_model, messages.clone(), Some(tool_defs.clone()));
            let reply = self.client.complete(&request).await?;

            let native = reply
                .choices
                .first()
                .and_then(|c| c.message.tool_calls.clone())
                .unwrap_or_default();

            if !native.is_empty() {
                for call in &native {
                    let result = self.execute_tool_call(call).await;
                    round_tool_calls += 1;
                    messages.push(assistant_tool_calls(vec![call.clone()]));
                    messages.push(tool_result(&call.id, &result));
                }
                continue;
            }

            let text = assistant_text(&reply);
            let fenced = extract_fenced_calls(&text)?;
            if !fenced.is_empty() {
                for call in fenced {
                    let result = self.tools.call(&call.tool, &call.args).map(|r| r.text);
                    let result = match result {
                        Ok(t) => t,
                        Err(e) => e.to_string(),
                    };
                    round_tool_calls += 1;
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

        Ok(BuildRound {
            summary,
            tool_calls: round_tool_calls,
        })
    }

    async fn execute_tool_call(&self, call: &ToolCall) -> String {
        let args: Value = serde_json::from_str(&call.function.arguments)
            .unwrap_or(Value::Null);
        match self.tools.call(&call.function.name, &args) {
            Ok(r) => r.text,
            Err(e) => e.to_string(),
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
        parse_verdict(&text).map_err(|_| {
            Error::InvalidInput(format!("router returned unparseable output: {text}"))
        })
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
            MockClient::text(r#"{"verdict":"revise","feedback":"read it",
                "revised_plan":{"goal":"g","steps":[{"id":1,"action":"read a.txt"}]}}"#),
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
        let _ = std::fs::remove_dir_all(&root);
    }
}
