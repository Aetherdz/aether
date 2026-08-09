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
    /// List, show, resume, rename or delete sessions
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Search past sessions by keyword
    Recall {
        /// The phrase to search for
        phrase: String,
    },
    /// Sync sessions across devices (gist or folder backend)
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Launch the interactive ratatui terminal UI
    Tui,
}

#[derive(Debug, Subcommand)]
pub enum SessionsAction {
    /// List all sessions (newest first)
    List,
    /// Show the full transcript of a session
    Show {
        /// Session id
        id: String,
    },
    /// Delete a session and its title
    Delete {
        /// Session id
        id: String,
    },
    /// Rename a session
    Rename {
        /// Session id
        id: String,
        /// New title
        title: String,
    },
    /// Resume a session in the interactive chat
    Resume {
        /// Session id
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SyncAction {
    /// Set up a folder backend and persist the state
    SetupFolder {
        /// Local directory that holds aether-sessions.json
        path: String,
    },
    /// Set up a gist backend and persist the state
    SetupGist {
        /// GitHub gist id
        id: String,
    },
    /// Push local sessions to the backend (pull + merge + push)
    Push,
    /// Pull the backend bundle and merge sessions into the local store
    Pull,
    /// Show the current sync state and token presence
    Status,
}
