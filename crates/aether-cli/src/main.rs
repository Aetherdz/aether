//! aether — A cross-platform terminal AI coding agent.
//!
//! Phase 0 ships exactly five commands:
//! - `aether ask "..."`        one-shot streaming answer
//! - `aether chat`             interactive REPL
//! - `aether use provider[/model]`  set defaults
//! - `aether models [--live]`  list models
//! - `aether providers`        list providers with key status

mod cli;

use std::io::Write;

use aether_core::config::{load_config, update_default};
use aether_core::error::{Error, Result};
use aether_provider::{
    fetch_zen_models, get_provider, key_status, list_providers, resolve_default, ChatMessage,
    ChatRequest, OpenAICompatibleClient, Provider,
};
use aether_session::{list_sessions, Ledger, Recall, Session, SessionId, SessionMeta};
use clap::Parser;
use futures::StreamExt;

use cli::{Cli, Command, SessionsAction, SyncAction};
use aether_sync::Backend;

const MODEL_SEPARATOR: &str = "────────────────────────────────────────";

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ask { question, provider, model } => {
            cmd_ask(&question, provider.as_deref(), model.as_deref()).await
        }
        Command::Chat { provider, model } => cmd_chat(provider.as_deref(), model.as_deref()).await,
        Command::Use { spec } => cmd_use(&spec),
        Command::Models { provider, live } => cmd_models(provider.as_deref(), live).await,
        Command::Providers => cmd_providers(),
        Command::Sessions { action } => cmd_sessions(action).await,
        Command::Recall { phrase } => cmd_recall(&phrase),
        Command::Sync { action } => cmd_sync(action).await,
        Command::Tui => cmd_tui(),
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
    let mut reader = rustyline::DefaultEditor::new()
        .map_err(|e| Error::Config(e.to_string()))?;
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
        }],
        temperature: None,
        stream: true,
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
        println!(
            "{}  key: {}  :: {}",
            p.id,
            status.as_str(),
            p.description
        );
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
                .map(|m| ChatMessage { role: m.role, content: m.content })
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
    use aether_sync::{
        generate_device_id, github_token, load_state, pull, push, save_state,
    };
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
            let backend = state
                .backend
                .clone()
                .unwrap_or_else(|| "off".to_string());
            println!("backend: {backend}");
            if let Some(gist) = &state.gist_id {
                println!("gist id: {gist}");
            }
            if let Some(folder) = &state.folder {
                println!("folder: {folder}");
            }
            println!("device: {}", state.device_id);
            if backend == "gist" {
                println!("token: {}", if github_token()?.is_some() { "present" } else { "missing" });
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
    let mut reader = rustyline::DefaultEditor::new()
        .map_err(|e| Error::Config(e.to_string()))?;
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
        messages.push(ChatMessage { role: "user".to_string(), content: line.clone() });
        println!("\n{MODEL_SEPARATOR}");
        let (reply, usage) = stream_collect(&client, &model, &messages).await?;
        messages.push(ChatMessage { role: "assistant".to_string(), content: reply.clone() });
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
