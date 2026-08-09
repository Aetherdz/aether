//! aether — A cross-platform terminal AI coding agent.
//!
//! Command surface (6 root commands):
//! - `aether ask "..."`        one-shot streaming answer
//! - `aether chat`             interactive REPL
//! - `aether agent "task"`     run the 3-model agent loop
//! - `aether agent undo [f]`   list/restore write snapshots
//! - `aether provider ...`     list / models / use (set defaults)
//! - `aether session ...`      list / show / delete / rename / resume / search / sync
//! - `aether tui`              launch the terminal UI
//!
//! Legacy names (`use`, `models`, `providers`, `sessions`, `recall`, `sync`,
//! `undo`) still parse but print a one-line deprecation notice.

mod cli;

use std::io::Write;

use aether_agent::{AgentStopReason, UndoJournal};
use aether_core::config::{load_config, update_default};
use aether_core::error::{Error, Result};
use aether_provider::{
    ChatMessage, ChatRequest, OpenAICompatibleClient, Provider, fetch_zen_models, get_provider,
    key_status, list_providers, resolve_default,
};
use aether_session::{Ledger, Recall, Session, SessionId, SessionMeta, list_sessions};
use clap::Parser;
use futures::StreamExt;

use aether_sync::Backend;
use cli::{AgentAction, Cli, Command, ProviderAction, SessionAction, SessionsAction, SyncAction};

const MODEL_SEPARATOR: &str = "────────────────────────────────────────";

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        // The TUI builds its own tokio runtime, so it must run outside the
        // runtime created below (a second Runtime::new() would panic).
        Command::Tui => cmd_tui(),
        command => {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| Error::Config(e.to_string()))?;
            rt.block_on(dispatch(command))
        }
    }
}

async fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Ask {
            question,
            provider,
            model,
        } => cmd_ask(&question, provider.as_deref(), model.as_deref()).await,
        Command::Chat { provider, model } => cmd_chat(provider.as_deref(), model.as_deref()).await,
        Command::Agent {
            action,
            task,
            provider,
            plan_model,
            build_model,
            route_model,
            iterations,
            yes,
        } => match action {
            Some(AgentAction::Undo { file }) => cmd_undo(file.as_deref()),
            None => {
                let task = task.ok_or_else(|| {
                    Error::InvalidInput(
                        "`aether agent` needs a task or a subcommand (try `aether agent undo`)"
                            .into(),
                    )
                })?;
                cmd_agent(
                    &task,
                    provider.as_deref(),
                    plan_model.as_deref(),
                    build_model.as_deref(),
                    route_model.as_deref(),
                    iterations,
                    yes,
                )
                .await
            }
        },
        Command::Provider { action } => cmd_provider(action).await,
        Command::Session { action } => cmd_session(action).await,
        Command::Use { spec } => {
            deprecate("use", "provider use");
            cmd_use(&spec)
        }
        Command::Models { provider, live } => {
            deprecate("models", "provider models");
            cmd_models(provider.as_deref(), live).await
        }
        Command::Providers => {
            deprecate("providers", "provider list");
            cmd_providers()
        }
        Command::Sessions { action } => {
            deprecate("sessions", "session");
            cmd_sessions(action).await
        }
        Command::Recall { phrase } => {
            deprecate("recall", "session search");
            cmd_recall(&phrase)
        }
        Command::Sync { action } => {
            deprecate("sync", "session sync");
            cmd_sync(action).await
        }
        Command::Undo { file } => {
            deprecate("undo", "agent undo");
            cmd_undo(file.as_deref())
        }
        Command::Tui => unreachable!("Tui is dispatched from main() before the runtime"),
    }
}

/// Print a one-line deprecation notice for a renamed command.
fn deprecate(old: &str, new: &str) {
    eprintln!(
        "note: `aether {old}` is deprecated; use `aether {new}` instead"
    );
}

/// `aether provider <list|models|use>` — provider management.
async fn cmd_provider(action: ProviderAction) -> Result<()> {
    match action {
        ProviderAction::List => cmd_providers(),
        ProviderAction::Models { provider, live } => {
            cmd_models(provider.as_deref(), live).await
        }
        ProviderAction::Use { spec } => cmd_use(&spec),
    }
}

/// `aether session <list|show|delete|rename|resume|search|sync>` — session management.
async fn cmd_session(action: SessionAction) -> Result<()> {
    match action {
        SessionAction::List => cmd_sessions(SessionsAction::List).await,
        SessionAction::Show { id } => cmd_sessions(SessionsAction::Show { id }).await,
        SessionAction::Delete { id } => cmd_sessions(SessionsAction::Delete { id }).await,
        SessionAction::Rename { id, title } => {
            cmd_sessions(SessionsAction::Rename { id, title }).await
        }
        SessionAction::Resume { id } => cmd_sessions(SessionsAction::Resume { id }).await,
        SessionAction::Search { phrase } => cmd_recall(&phrase),
        SessionAction::Sync { action } => cmd_sync(action).await,
    }
}

/// Launch the ratatui terminal UI.
fn cmd_tui() -> Result<()> {
    use aether_tui::{RatatuiTui, Tui};
    let mut tui = RatatuiTui::new().map_err(|e| Error::Config(e.to_string()))?;
    tui.run().map_err(|e| Error::Config(e.to_string()))
}

/// Resolve the effective provider + model; print any fallback notice.
fn resolve(
    config: &aether_core::config::AetherConfig,
    provider: Option<&str>,
    model: Option<&str>,
) -> (OpenAICompatibleClient, String) {
    let resolved = resolve_default(config, provider, model);
    if let Some(notice) = &resolved.notice {
        eprintln!("{notice}");
    }
    let client = resolved
        .provider
        .client()
        .expect("failed to build provider client");
    (client, resolved.model)
}

async fn cmd_ask(question: &str, provider: Option<&str>, model: Option<&str>) -> Result<()> {
    let config = load_config()?;
    let (client, model) = resolve(&config, provider, model);
    stream_print(&client, &model, question).await
}

async fn cmd_chat(provider: Option<&str>, model: Option<&str>) -> Result<()> {
    let config = load_config()?;
    let (client, model) = resolve(&config, provider, model);

    println!("aether chat — model {model} (exit: Ctrl-D or /exit)");
    let mut reader = rustyline::DefaultEditor::new().map_err(|e| Error::Config(e.to_string()))?;
    loop {
        let readline = reader.readline("aether> ");
        let line = match readline {
            Ok(line) => line.trim().to_string(),
            Err(_) => break,
        };
        if line.is_empty() {
            continue;
        }
        if line == "/exit" || line == "quit" || line == "exit" {
            break;
        }
        println!("\n{MODEL_SEPARATOR}");
        stream_print(&client, &model, &line).await?;
        println!("\n{MODEL_SEPARATOR}\n");
    }
    Ok(())
}

/// Stream one user message to the model and print the text deltas
/// (also captures `usage` and renders `reasoning_content` deltas).
async fn stream_print(client: &OpenAICompatibleClient, model: &str, content: &str) -> Result<()> {
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            ..ChatMessage::default()
        }],
        temperature: None,
        stream: true,
        tools: None,
    };
    let chunks = client.stream_chat(&request).await?;
    futures::pin_mut!(chunks);
    let mut out = std::io::stdout().lock();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if let Some(text) = &chunk.content {
            out.write_all(text.as_bytes())?;
            out.flush()?;
        }
    }
    println!();
    Ok(())
}

fn cmd_use(spec: &str) -> Result<()> {
    let (provider, model) = match spec.split_once('/') {
        Some((p, m)) => (p, Some(m)),
        None => (spec, None),
    };
    let config = load_config()?;
    if get_provider(provider, &config).is_none() {
        eprintln!("warn: provider \"{provider}\" is not configured; it will fall back to zen");
    }
    update_default(provider, model)?;
    println!(
        "default set to {provider}{}",
        model.map(|m| format!("/{m}")).unwrap_or_default()
    );
    Ok(())
}

async fn cmd_models(provider: Option<&str>, live: bool) -> Result<()> {
    let config = load_config()?;
    let p = provider
        .map(|p| get_provider(p, &config))
        .unwrap_or_else(|| Some(resolve_default(&config, None, None).provider))
        .ok_or_else(|| Error::Provider("provider not found".into()))?;

    let mut models = p.static_models.clone();
    if p.kind == "zen" || live {
        match fetch_zen_models().await {
            Ok(fetched) => models = fetched,
            Err(_) => {
                eprintln!("warn: live fetch failed, using static list");
            }
        }
    }
    for m in &models {
        println!("{m}");
    }
    Ok(())
}

fn cmd_providers() -> Result<()> {
    let config = load_config()?;
    for p in list_providers(&config) {
        let status = key_status(&p);
        println!("{}  key: {}  :: {}", p.id, status.as_str(), p.description);
    }
    Ok(())
}

async fn cmd_sessions(action: SessionsAction) -> Result<()> {
    match action {
        SessionsAction::List => {
            let sessions = list_sessions()?;
            if sessions.is_empty() {
                println!("no sessions yet");
                return Ok(());
            }
            for s in sessions {
                let title = s.title.unwrap_or_else(|| "(untitled)".to_string());
                println!(
                    "{}  {} msgs  in:{} out:{}  {}",
                    s.id, s.messages, s.stats.input_tokens, s.stats.output_tokens, title
                );
            }
            let totals = Ledger::totals()?;
            println!(
                "\n{} sessions · {} turns · {} in / {} out",
                totals.sessions, totals.turns, totals.input_tokens, totals.output_tokens
            );
            Ok(())
        }
        SessionsAction::Show { id } => {
            let id = SessionId::new(id.clone());
            let session = Session::open(&id)?;
            let messages = session.read_messages()?;
            if messages.is_empty() {
                println!("session {} is empty", id.as_str());
                return Ok(());
            }
            println!("title: {}", session.title()?);
            for m in messages {
                println!("[{}] {}", m.role, m.content);
            }
            Ok(())
        }
        SessionsAction::Delete { id } => {
            let id = SessionId::new(id.clone());
            let session = Session::open(&id)?;
            match session.delete()? {
                true => println!("deleted {}", id.as_str()),
                false => println!("session {} not found", id.as_str()),
            }
            Ok(())
        }
        SessionsAction::Rename { id, title } => {
            let id = SessionId::new(id.clone());
            let session = Session::open(&id)?;
            session.rename(&title)?;
            println!("renamed {} -> {title}", id.as_str());
            Ok(())
        }
        SessionsAction::Resume { id } => {
            let id = SessionId::new(id.clone());
            let session = Session::open(&id)?;
            let history = session
                .read_messages()?
                .into_iter()
                .map(|m| ChatMessage {
                    role: m.role,
                    content: m.content,
                    ..ChatMessage::default()
                })
                .collect::<Vec<_>>();
            println!("resuming {} — {}", session.id(), session.title()?);
            cmd_chat_with_history(session, history).await
        }
    }
}

fn cmd_recall(phrase: &str) -> Result<()> {
    let hits = Recall::search(phrase, 10)?;
    if hits.is_empty() {
        println!("no matches for \"{phrase}\"");
        return Ok(());
    }
    for h in hits {
        let title = h.title.unwrap_or_else(|| "(untitled)".to_string());
        println!("[{}] {} ({} matches)", h.id, title, h.matches);
        println!("  {}\n", h.snippet);
    }
    Ok(())
}

async fn cmd_sync(action: SyncAction) -> Result<()> {
    use aether_sync::{generate_device_id, github_token, load_state, pull, push, save_state};
    match action {
        SyncAction::SetupFolder { path } => {
            let mut state = load_state()?;
            if state.device_id.is_empty() {
                state.device_id = generate_device_id();
            }
            state.backend = Some("folder".to_string());
            state.folder = Some(path.clone());
            save_state(&state)?;
            println!("sync enabled: folder {path}");
            Ok(())
        }
        SyncAction::SetupGist { id } => {
            let mut state = load_state()?;
            if state.device_id.is_empty() {
                state.device_id = generate_device_id();
            }
            state.backend = Some("gist".to_string());
            state.gist_id = Some(id.clone());
            save_state(&state)?;
            println!("sync enabled: gist {id}");
            Ok(())
        }
        SyncAction::Push => {
            let state = load_state()?;
            let backend = resolve_backend(&state)?;
            let report = push(&state.device_id, &backend).await?;
            println!("pushed {} sessions", report.written);
            Ok(())
        }
        SyncAction::Pull => {
            let state = load_state()?;
            let backend = resolve_backend(&state)?;
            let report = pull(&backend).await?;
            println!(
                "pulled: {} written, {} merged",
                report.written, report.merged
            );
            Ok(())
        }
        SyncAction::Status => {
            let state = load_state()?;
            let backend = state.backend.clone().unwrap_or_else(|| "off".to_string());
            println!("backend: {backend}");
            if let Some(gist) = &state.gist_id {
                println!("gist id: {gist}");
            }
            if let Some(folder) = &state.folder {
                println!("folder: {folder}");
            }
            println!("device: {}", state.device_id);
            if backend == "gist" {
                println!(
                    "token: {}",
                    if github_token()?.is_some() {
                        "present"
                    } else {
                        "missing"
                    }
                );
            }
            Ok(())
        }
    }
}

fn resolve_backend(state: &aether_sync::SyncState) -> Result<Backend> {
    match state.backend.as_deref() {
        Some("folder") => match &state.folder {
            Some(path) => Ok(Backend::Folder { path: path.into() }),
            None => Err(Error::InvalidInput("folder backend has no path; re-run setup".into())),
        },
        Some("gist") => match &state.gist_id {
            Some(id) => Ok(Backend::Gist { id: id.clone() }),
            None => Err(Error::InvalidInput("gist backend has no id; re-run setup".into())),
        },
        _ => Err(Error::InvalidInput(
            "sync is off — run `aether sync setup folder <path>` or `aether sync setup gist <id>` first"
                .into(),
        )),
    }
}

async fn cmd_chat_with_history(session: Session, history: Vec<ChatMessage>) -> Result<()> {
    let config = load_config()?;
    let (client, model) = resolve(&config, None, None);

    println!("model {model} (exit: Ctrl-D or /exit)");
    let mut reader = rustyline::DefaultEditor::new().map_err(|e| Error::Config(e.to_string()))?;
    let mut messages = history;
    loop {
        let readline = reader.readline("aether> ");
        let line = match readline {
            Ok(line) => line.trim().to_string(),
            Err(_) => break,
        };
        if line.is_empty() {
            continue;
        }
        if line == "/exit" || line == "quit" || line == "exit" {
            break;
        }
        session.append("user", &line)?;
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: line.clone(),
            ..ChatMessage::default()
        });
        println!("\n{MODEL_SEPARATOR}");
        let (reply, usage) = stream_collect(&client, &model, &messages).await?;
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: reply.clone(),
            ..ChatMessage::default()
        });
        session.append("assistant", &reply)?;
        if let Some(u) = usage {
            session.append_usage(SessionMeta {
                turns: 1,
                input_tokens: u.prompt_tokens.unwrap_or(0),
                output_tokens: u.completion_tokens.unwrap_or(0),
            })?;
        }
        println!("\n{MODEL_SEPARATOR}\n");
    }
    Ok(())
}

async fn cmd_agent(
    task: &str,
    provider: Option<&str>,
    plan_model: Option<&str>,
    build_model: Option<&str>,
    route_model: Option<&str>,
    iterations: u32,
    yes: bool,
) -> Result<()> {
    let config = load_config()?;
    let (client, model) = resolve(&config, provider, None);
    let plan_model = plan_model.unwrap_or(&model).to_string();
    let build_model = build_model.unwrap_or(&model).to_string();
    let route_model = route_model.unwrap_or(&model).to_string();
    let cwd = std::env::current_dir().map_err(|e| Error::Config(e.to_string()))?;

    let mut agent =
        aether_agent::Agent::new(Box::new(client), cwd, plan_model, build_model, route_model)
            .with_iteration_cap(iterations);
    if yes {
        agent = agent.with_yes();
    } else {
        agent = agent.with_confirm(true);
    }

    println!("aether agent — plan: build: route (max {iterations} iterations)\n");
    let result = agent.run(task).await?;
    println!("plan: {}", result.plan.goal);
    for step in &result.plan.steps {
        println!("  {}. {}", step.id, step.action);
    }
    let status = match result.stopped_reason {
        AgentStopReason::Done => format!(
            "done in {} iterations · {} tool calls",
            result.iterations, result.tool_calls
        ),
        AgentStopReason::IterationCap => format!(
            "stopped after {} iterations: iteration cap reached",
            result.iterations
        ),
        AgentStopReason::Stagnation => format!(
            "stopped after {} iterations: no progress detected (plan unchanged, no files modified)",
            result.iterations
        ),
        AgentStopReason::BuildTurnCap => format!(
            "stopped after {} iterations: build model exhausted its tool budget",
            result.iterations
        ),
    };
    println!("\n[{status}]\n");
    println!("{}", result.final_answer);
    Ok(())
}

/// `aether undo` — list or restore snapshots persisted by the agent.
fn cmd_undo(file: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().map_err(|e| Error::Config(e.to_string()))?;
    let journal = UndoJournal::new(&cwd);
    match file {
        Some(rel) => {
            let restored = journal.restore(rel)?;
            println!(
                "restored {rel} from snapshot {} ({} bytes)",
                restored.seq, restored.bytes
            );
        }
        None => {
            let snaps = journal.list()?;
            if snaps.is_empty() {
                println!("no snapshots in {}", journal.dir().display());
                return Ok(());
            }
            println!("snapshots in {}:", journal.dir().display());
            for s in snaps {
                println!("{}  {}  ({} bytes)", s.seq, s.rel, s.bytes);
            }
        }
    }
    Ok(())
}

async fn stream_collect(
    client: &OpenAICompatibleClient,
    model: &str,
    messages: &[ChatMessage],
) -> Result<(String, Option<aether_provider::Usage>)> {
    let request = ChatRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        temperature: None,
        stream: true,
        tools: None,
    };
    let chunks = client.stream_chat(&request).await?;
    futures::pin_mut!(chunks);
    let mut out = std::io::stdout().lock();
    let mut reply = String::new();
    let mut usage = None;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if let Some(text) = &chunk.content {
            out.write_all(text.as_bytes())?;
            out.flush()?;
            reply.push_str(text);
        }
        if chunk.usage.is_some() {
            usage = chunk.usage;
        }
    }
    println!();
    Ok((reply, usage))
}
