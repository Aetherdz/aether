//! clap derive CLI surface. Six root commands:
//! `ask`, `chat`, `agent`, `tui`, `provider`, `session`.
//! Legacy command names (`use`, `models`, `providers`, `sessions`, `recall`,
//! `sync`, `undo`) still parse but are hidden and print a deprecation notice.

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
    /// Run the 3-model agent loop (plan -> build -> route) on a task
    #[command(subcommand_precedence_over_arg = true)]
    Agent {
        /// Agent subcommand (e.g. undo); otherwise the task is positional
        #[command(subcommand)]
        action: Option<AgentAction>,
        /// The task to accomplish
        task: Option<String>,
        /// Provider id
        #[arg(short, long)]
        provider: Option<String>,
        /// Plan model id (defaults to the resolved model)
        #[arg(long)]
        plan_model: Option<String>,
        /// Build model id (defaults to the resolved model)
        #[arg(long)]
        build_model: Option<String>,
        /// Route model id (defaults to the resolved model)
        #[arg(long)]
        route_model: Option<String>,
        /// Max loop iterations before giving up
        #[arg(short, long, default_value_t = 6)]
        iterations: u32,
        /// Auto-approve write_file changes without prompting (non-interactive)
        #[arg(long)]
        yes: bool,
    },
    /// Launch the interactive ratatui terminal UI
    Tui,
    /// Manage providers: list, models, use (set default)
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    /// List, show, resume, rename, delete or search sessions (sync via subcommand)
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Set the default provider and/or model (e.g. aether use zen/claude-sonnet-5)
    #[command(hide = true)]
    Use {
        /// Provider spec: `provider` or `provider/model`
        spec: String,
    },
    /// List models for a provider (default provider if omitted; zen fetches live)
    #[command(hide = true)]
    Models {
        /// Provider id
        provider: Option<String>,
        /// Force a live fetch from the zen models endpoint
        #[arg(short, long)]
        live: bool,
    },
    /// List all providers with key status
    #[command(hide = true)]
    Providers,
    /// List, show, resume, rename or delete sessions
    #[command(hide = true)]
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Search past sessions by keyword
    #[command(hide = true)]
    Recall {
        /// The phrase to search for
        phrase: String,
    },
    /// Sync sessions across devices (gist or folder backend)
    #[command(hide = true)]
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// List or restore write snapshots saved by the agent under .aether-undo/
    #[command(hide = true)]
    Undo {
        /// Restore this relative file to its most recent snapshot (omit to list)
        file: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// List or restore write snapshots saved by the agent under .aether-undo/
    Undo {
        /// Restore this relative file to its most recent snapshot (omit to list)
        file: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProviderAction {
    /// List all providers with key status
    List,
    /// List models for a provider (default provider if omitted; zen fetches live)
    Models {
        /// Provider id
        provider: Option<String>,
        /// Force a live fetch from the zen models endpoint
        #[arg(short, long)]
        live: bool,
    },
    /// Set the default provider and/or model (e.g. aether provider use zen/claude-sonnet-5)
    Use {
        /// Provider spec: `provider` or `provider/model`
        spec: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionAction {
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
    /// Search past sessions by keyword
    Search {
        /// The phrase to search for
        phrase: String,
    },
    /// Sync sessions across devices (gist or folder backend)
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("aether").chain(args.iter().copied()))
            .expect("args should parse")
            .command
    }

    #[test]
    fn new_surface_parses() {
        assert!(matches!(parse(&["ask", "hi"]), Command::Ask { .. }));
        assert!(matches!(parse(&["chat"]), Command::Chat { .. }));
        assert!(matches!(parse(&["tui"]), Command::Tui));
        assert!(matches!(
            parse(&["provider", "list"]),
            Command::Provider { .. }
        ));
        assert!(matches!(
            parse(&["session", "list"]),
            Command::Session { .. }
        ));
    }

    #[test]
    fn agent_task_and_undo_both_parse() {
        match parse(&["agent", "fix the bug"]) {
            Command::Agent {
                task: Some(t), action: None, ..
            } => assert_eq!(t, "fix the bug"),
            other => panic!("expected Agent with task, got {other:?}"),
        }
        match parse(&["agent", "undo", "src/main.rs"]) {
            Command::Agent {
                task: None,
                action: Some(AgentAction::Undo { file }),
                ..
            } => assert_eq!(file.as_deref(), Some("src/main.rs")),
            other => panic!("expected Agent undo, got {other:?}"),
        }
        // Bare `agent` with no task and no subcommand parses (handled in main).
        assert!(matches!(parse(&["agent"]), Command::Agent { .. }));
    }

    #[test]
    fn deprecated_aliases_still_resolve() {
        assert!(matches!(parse(&["use", "zen"]), Command::Use { .. }));
        assert!(matches!(
            parse(&["models", "zen"]),
            Command::Models { .. }
        ));
        assert!(matches!(parse(&["providers"]), Command::Providers));
        assert!(matches!(parse(&["recall", "auth"]), Command::Recall { .. }));
        assert!(matches!(parse(&["undo", "file.rs"]), Command::Undo { .. }));
        assert!(matches!(parse(&["sync", "status"]), Command::Sync { .. }));
    }

    #[test]
    fn deprecated_sessions_actions_map_one_to_one() {
        assert!(matches!(
            parse(&["sessions", "list"]),
            Command::Sessions { .. }
        ));
        assert!(matches!(
            parse(&["sessions", "show", "abc"]),
            Command::Sessions { .. }
        ));
    }
}
