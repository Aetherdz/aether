//! clap derive CLI surface. Phase 0 ships exactly five commands:
//! `ask`, `chat`, `use`, `models`, `providers`.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aether",
    version,
    about = "A cross-platform terminal AI coding agent. Building tools that break assumptions."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Ask a one-shot question and stream the answer
    Ask {
        /// The question to ask
        question: String,
        /// Provider id
        #[arg(short, long)]
        provider: Option<String>,
        /// Model id
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Run an interactive chat REPL
    Chat {
        /// Provider id
        #[arg(short, long)]
        provider: Option<String>,
        /// Model id
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Set the default provider and/or model (e.g. aether use zen/claude-sonnet-5)
    Use {
        /// Provider spec: `provider` or `provider/model`
        spec: String,
    },
    /// List models for a provider (default provider if omitted; zen fetches live)
    Models {
        /// Provider id
        provider: Option<String>,
        /// Force a live fetch from the zen models endpoint
        #[arg(short, long)]
        live: bool,
    },
    /// List all providers with key status
    Providers,
}
