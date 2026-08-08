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

use aetherdz_core::config::{load_config, update_default};
use aetherdz_core::error::{Error, Result};
use aetherdz_provider::{
    fetch_zen_models, get_provider, key_status, list_providers, resolve_default, ChatMessage,
    ChatRequest, OpenAICompatibleClient, Provider,
};
use clap::Parser;
use futures::StreamExt;

use cli::{Cli, Command};

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
    }
}

/// Resolve the effective provider + model; print any fallback notice.
fn resolve(
    config: &aetherdz_core::config::AetherConfig,
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
