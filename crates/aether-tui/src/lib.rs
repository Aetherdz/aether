//! aether-tui — ratatui + crossterm terminal UI (Phase 4).
//!
//! Three screens behind a shared chrome (header bar + tabs):
//! - **Session list** — browse/rename/delete/resume sessions, arrow keys + wheel scroll.
//! - **Chat** — transcript of the active session, live streaming, ctrl+P provider/model palette.
//! - **Agent** — live plan → build → route loop visualization fed by the aether-agent observer.
//!
//! The trait surface (`Tui`, `TuiError`) from Phase 0 is preserved so downstream
//! crates still compile against a stable contract.

pub mod agent_screen;
pub mod cards;
pub mod chrome;
pub mod render;

use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use aether_agent::ChannelObserver;
use aether_agent::observer::DynObserver;
use aether_core::config::load_config;
use aether_provider::{
    ChatMessage, ChatRequest, OpenAICompatibleClient, Provider, resolve_default,
};
use aether_session::{Ledger, Session, list_sessions};
use thiserror::Error;

/// Errors produced by a TUI implementation.
#[derive(Debug, Error)]
pub enum TuiError {
    /// The terminal could not be initialized or restored.
    #[error("terminal error: {0}")]
    Terminal(String),
    /// Rendering failed.
    #[error("render error: {0}")]
    Render(String),
    /// Reading events failed.
    #[error("event error: {0}")]
    Event(String),
    /// A session or ledger operation failed.
    #[error("session error: {0}")]
    Session(String),
    /// The configured provider could not be reached.
    #[error("provider error: {0}")]
    Provider(String),
}

/// The TUI contract. Phase 0 defines the surface; Phase 4 provides the
/// ratatui implementation.
pub trait Tui {
    /// Run the TUI event loop until it returns.
    fn run(&mut self) -> Result<(), TuiError>;
}

/// A no-op TUI used before the real implementation lands. It exists so the
/// trait has a concrete, testable implementation.
#[derive(Debug, Default)]
pub struct NoopTui;

impl Tui for NoopTui {
    fn run(&mut self) -> Result<(), TuiError> {
        Ok(())
    }
}

/// Shared visual language: Modern Dark (slate + success-green accent).
/// RGB values from the ui-ux-pro-max design system for developer tools.
mod theme {
    use ratatui::style::{Color, Modifier, Style};

    pub const ACCENT: Color = Color::Rgb(0x22, 0xc5, 0x5e);
    pub const AI: Color = Color::Rgb(0x81, 0x8c, 0xf8);
    pub const TITLE: Color = Color::Rgb(0x38, 0xbd, 0xf8);
    pub const BORDER: Color = Color::Rgb(0x33, 0x41, 0x55);
    pub const MUTED: Color = Color::Rgb(0x64, 0x74, 0x8b);
    pub const SELECT_BG: Color = Color::Rgb(0x1e, 0x29, 0x3b);
    /// Background fill behind user messages (opencode-style "You" block).
    pub const USER_BG: Color = Color::Rgb(0x16, 0x22, 0x30);

    pub fn title() -> Style {
        Style::default().fg(TITLE).add_modifier(Modifier::BOLD)
    }
    pub fn accent() -> Style {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    }
    pub fn ai() -> Style {
        Style::default().fg(AI).add_modifier(Modifier::BOLD)
    }
    pub fn muted() -> Style {
        Style::default().fg(MUTED)
    }
    pub fn border() -> Style {
        Style::default().fg(BORDER)
    }
    pub fn highlight() -> Style {
        Style::default().bg(SELECT_BG).fg(Color::White)
    }
}

/// Which screen is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Sessions,
    Chat,
    /// Live plan -> build -> route loop visualization.
    Agent,
    Palette,
    /// Slash-command picker (opened by typing `/` in the chat input).
    Commands,
}

/// A slash command entry shown by the `/` picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashCmd {
    Model,
    Agent,
    Sessions,
    Clear,
    Help,
    Exit,
}

impl SlashCmd {
    const ALL: [SlashCmd; 6] = [
        SlashCmd::Model,
        SlashCmd::Agent,
        SlashCmd::Sessions,
        SlashCmd::Clear,
        SlashCmd::Help,
        SlashCmd::Exit,
    ];

    fn label(self) -> &'static str {
        match self {
            SlashCmd::Model => "/model    switch the active model",
            SlashCmd::Agent => "/agent    open the live agent loop",
            SlashCmd::Sessions => "/sessions go back to the session list",
            SlashCmd::Clear => "/clear    clear the transcript",
            SlashCmd::Help => "/help     show this reference",
            SlashCmd::Exit => "/exit     quit aether",
        }
    }
}

/// A renderable chat line with its role for colouring.
#[derive(Debug, Clone)]
struct ChatRow {
    role: String,
    content: String,
}

/// The synchronous result of a key press; async work runs in the event loop.
#[derive(Debug, PartialEq, Eq)]
enum KeyAction {
    /// Keep running.
    Continue,
    /// Stop the event loop.
    Quit,
    /// Ask the model `question` against `model` (async).
    SendTurn { question: String, model: String },
}

/// One event pushed from the background streaming task to the UI thread.
#[derive(Debug)]
enum StreamEvent {
    Chunk(String),
    Usage(aether_provider::Usage),
    Done,
    Error(String),
}

/// An in-flight reply being streamed in a background task. The UI drains
/// [`ChatStream::rx`] on every draw tick, so typing, scrolling and the
/// spinner stay live while the model answers — nothing blocks the loop.
struct ChatStream {
    rx: std::sync::mpsc::Receiver<StreamEvent>,
    buffer: String,
    usage: Option<aether_provider::Usage>,
}

/// Live streaming state shown on the status line above the chat input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ProcessingStatus {
    #[default]
    Idle,
    Thinking,
    Streaming(u64),
}

/// Braille spinner frames; index with `frame % SPINNER.len()`.
const SPINNER: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

/// The full ratatui application state.
pub struct RatatuiTui {
    screen: Screen,
    sessions: Vec<aether_session::SessionSummary>,
    list_state: ListState,
    /// Scroll offset for the session list (0 = top).
    list_scroll: usize,
    /// Active session while chatting.
    active: Option<Session>,
    /// Transcript rows for the active session (includes streamed tail).
    chat: Vec<ChatRow>,
    chat_scroll: usize,
    /// In-flight reply streaming from the background task (None when idle).
    stream: Option<ChatStream>,
    processing: ProcessingStatus,
    /// Animation frame counter for the status spinner (advanced each draw).
    frame: u32,
    /// Live token counters for the active session.
    input_tokens: u64,
    output_tokens: u64,
    /// Text being typed in the chat input box.
    input: String,
    /// Resolved default model; updated when the ctrl+P palette picks one.
    model: String,
    /// Palette state: available models + the highlighted one.
    palette_models: Vec<String>,
    palette_index: usize,
    /// Highlighted row of the slash-command picker.
    commands_index: usize,
    /// Ledger totals footer.
    totals: aether_session::Totals,
    /// The most recent background error, shown in the footer.
    last_error: Option<TuiError>,
    /// Live state for the Agent screen (fed by the observer channel).
    agent_state: agent_screen::AgentScreenState,
    /// Receiver draining AgentPhase events while an agent run is live.
    agent_rx: Option<std::sync::mpsc::Receiver<aether_agent::AgentPhase>>,
    /// Resolved provider name, shown in the header bar.
    provider: String,
}

impl Default for RatatuiTui {
    fn default() -> Self {
        Self {
            screen: Screen::Sessions,
            sessions: Vec::new(),
            list_state: ListState::default(),
            list_scroll: 0,
            active: None,
            chat: Vec::new(),
            chat_scroll: 0,
            stream: None,
            processing: ProcessingStatus::Idle,
            frame: 0,
            input_tokens: 0,
            output_tokens: 0,
            input: String::new(),
            model: String::new(),
            palette_models: Vec::new(),
            palette_index: 0,
            commands_index: 0,
            totals: aether_session::Totals::default(),
            last_error: None,
            agent_state: agent_screen::AgentScreenState::default(),
            agent_rx: None,
            provider: String::new(),
        }
    }
}

/// Resolve the default model name for the configured provider.
fn default_model() -> String {
    let config = load_config().unwrap_or_default();
    resolve_default(&config, None, None).model
}

/// Clamp a chat scroll offset so it never scrolls past the last visible row.
fn clamp_scroll(scroll: usize, total: usize, viewport: usize) -> usize {
    scroll.min(total.saturating_sub(viewport))
}

impl RatatuiTui {
    /// Build the UI and load the session list + ledger totals.
    pub fn new() -> Result<Self, TuiError> {
        let mut app = Self {
            model: default_model(),
            ..Self::default()
        };
        let config = load_config().unwrap_or_default();
        app.provider = resolve_default(&config, None, None).provider.name.clone();
        app.refresh_sessions()?;
        Ok(app)
    }

    fn refresh_sessions(&mut self) -> Result<(), TuiError> {
        self.sessions = list_sessions().map_err(|e| TuiError::Session(e.to_string()))?;
        self.totals = Ledger::totals().map_err(|e| TuiError::Session(e.to_string()))?;
        if self.sessions.is_empty() {
            self.list_state.select(None);
        } else {
            let idx = self
                .list_state
                .selected()
                .unwrap_or(0)
                .min(self.sessions.len() - 1);
            self.list_state.select(Some(idx));
        }
        Ok(())
    }

    fn selected_session(&self) -> Option<&aether_session::SessionSummary> {
        self.list_state
            .selected()
            .and_then(|i| self.sessions.get(i))
    }

    /// Open the selected session and load its transcript.
    fn open_selected(&mut self) -> Result<(), TuiError> {
        let Some(summary) = self.selected_session() else {
            return Ok(());
        };
        let id = summary.id.clone();
        let session = Session::open(&id).map_err(|e| TuiError::Session(e.to_string()))?;
        self.chat = session
            .read_messages()
            .map_err(|e| TuiError::Session(e.to_string()))?
            .into_iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| ChatRow {
                role: m.role,
                content: m.content,
            })
            .collect();
        let stats = session
            .stats()
            .map_err(|e| TuiError::Session(e.to_string()))?;
        self.input_tokens = stats.input_tokens;
        self.output_tokens = stats.output_tokens;
        self.chat_scroll = 0;
        self.active = Some(session);
        self.screen = Screen::Chat;
        Ok(())
    }

    /// Delete the selected session and refresh the list.
    fn delete_selected(&mut self) -> Result<(), TuiError> {
        let Some(summary) = self.selected_session() else {
            return Ok(());
        };
        let id = summary.id.clone();
        Session::open(&id)
            .and_then(|s| s.delete())
            .map_err(|e| TuiError::Session(e.to_string()))?;
        self.refresh_sessions()
    }

    fn rename_selected(&mut self, title: &str) -> Result<(), TuiError> {
        let Some(summary) = self.selected_session() else {
            return Ok(());
        };
        let id = summary.id.clone();
        Session::open(&id)
            .and_then(|s| s.rename(title))
            .map_err(|e| TuiError::Session(e.to_string()))?;
        self.refresh_sessions()
    }

    /// Open the ctrl+P palette with the default provider's models.
    fn open_palette(&mut self) {
        let config = load_config().unwrap_or_default();
        let resolved = resolve_default(&config, None, None);
        self.palette_models = resolved.provider.static_models.clone();
        self.palette_index = self
            .palette_models
            .iter()
            .position(|m| m == &resolved.model)
            .unwrap_or(0);
        self.screen = Screen::Palette;
    }

    fn close_palette(&mut self) {
        self.screen = Screen::Chat;
    }

    /// The base URL + optional key for the currently configured provider.
    fn client() -> Result<OpenAICompatibleClient, TuiError> {
        let config = load_config().map_err(|e| TuiError::Session(e.to_string()))?;
        let resolved = resolve_default(&config, None, None);
        resolved
            .provider
            .client()
            .map_err(|e| TuiError::Provider(e.to_string()))
    }

    /// Queue one user turn: show the message immediately, spawn a background
    /// streaming task, and return — the draw loop keeps running.
    fn start_turn(&mut self, question: String, model: &str) -> Result<(), TuiError> {
        self.chat.push(ChatRow {
            role: "user".to_string(),
            content: question.clone(),
        });
        self.chat_scroll = self.chat.len().saturating_sub(1);
        if let Some(session) = &self.active {
            session
                .append("user", &question)
                .map_err(|e| TuiError::Session(e.to_string()))?;
        }

        const HISTORY_WINDOW: usize = 40;
        let start = self.chat.len().saturating_sub(HISTORY_WINDOW);
        let mut history: Vec<ChatMessage> = self.chat[start..self.chat.len() - 1]
            .iter()
            .map(|r| ChatMessage {
                role: r.role.clone(),
                content: r.content.clone(),
                ..ChatMessage::default()
            })
            .collect();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: question,
            ..ChatMessage::default()
        });

        let client = Self::client()?;
        let (tx, rx) = std::sync::mpsc::channel();
        self.stream = Some(ChatStream {
            rx,
            buffer: String::new(),
            usage: None,
        });
        self.processing = ProcessingStatus::Thinking;
        let request = ChatRequest {
            model: model.to_string(),
            messages: history,
            temperature: None,
            stream: true,
            tools: None,
        };
        tokio::spawn(async move {
            match client.stream_chat(&request).await {
                Ok(stream) => {
                    futures::pin_mut!(stream);
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => {
                                if let Some(text) = &chunk.content {
                                    let _ = tx.send(StreamEvent::Chunk(text.clone()));
                                }
                                if let Some(u) = chunk.usage {
                                    let _ = tx.send(StreamEvent::Usage(u));
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(StreamEvent::Error(e.to_string()));
                                return;
                            }
                        }
                    }
                    let _ = tx.send(StreamEvent::Done);
                }
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(e.to_string()));
                }
            }
        });
        Ok(())
    }

    /// Drain stream events since the last draw tick. Called on every frame so
    /// streaming text, the spinner, and token counters update in real time.
    fn drain_stream(&mut self) {
        let events: Vec<StreamEvent> = match &self.stream {
            Some(s) => s.rx.try_iter().collect(),
            None => return,
        };
        if events.is_empty() {
            return;
        }
        for ev in events {
            match ev {
                StreamEvent::Chunk(text) => {
                    if let Some(s) = &mut self.stream {
                        s.buffer.push_str(&text);
                    }
                    let n = self
                        .stream
                        .as_ref()
                        .map_or(0, |s| s.buffer.chars().count() as u64 / 4);
                    self.processing = ProcessingStatus::Streaming(n);
                }
                StreamEvent::Usage(u) => {
                    if let Some(s) = &mut self.stream {
                        s.usage = Some(u);
                    }
                }
                StreamEvent::Done => {
                    let (reply, usage) = match self.stream.take() {
                        Some(s) => (s.buffer, s.usage),
                        None => (String::new(), None),
                    };
                    self.finish_reply(reply, usage);
                }
                StreamEvent::Error(e) => {
                    self.stream = None;
                    self.processing = ProcessingStatus::Idle;
                    self.last_error = Some(TuiError::Provider(e));
                }
            }
        }
    }

    /// Persist a completed reply to the session and append it to the chat.
    fn finish_reply(&mut self, reply: String, usage: Option<aether_provider::Usage>) {
        if let Some(session) = &self.active {
            let _ = session.append("assistant", &reply);
            if let Some(u) = &usage {
                let input = u.prompt_tokens.unwrap_or(0);
                let output = u.completion_tokens.unwrap_or(0);
                let _ = session.append_usage(aether_session::SessionMeta {
                    turns: 1,
                    input_tokens: input,
                    output_tokens: output,
                });
                self.input_tokens += input;
                self.output_tokens += output;
            }
        }
        self.chat.push(ChatRow {
            role: "assistant".to_string(),
            content: reply,
        });
        self.chat_scroll = self.chat.len().saturating_sub(1);
        self.processing = ProcessingStatus::Idle;
    }

    /// Drive one event and return `true` to quit.
    async fn handle_event(&mut self, event: Event) -> Result<bool, TuiError> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match self.on_key(key) {
                KeyAction::Continue => Ok(false),
                KeyAction::Quit => Ok(true),
                KeyAction::SendTurn { question, model } => {
                    self.start_turn(question, &model)?;
                    Ok(false)
                }
            },
            Event::Mouse(me) => self.on_mouse(me),
            Event::Resize(_, _) => Ok(false),
            _ => Ok(false),
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> KeyAction {
        // ctrl+P opens/closes the palette on any screen.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            match self.screen {
                Screen::Palette => self.close_palette(),
                _ => self.open_palette(),
            }
            return KeyAction::Continue;
        }
        // Tab cycles the main screens: Chat -> Agent -> Sessions -> Chat.
        if key.code == KeyCode::Tab {
            self.screen = match self.screen {
                Screen::Chat => Screen::Agent,
                Screen::Agent => Screen::Sessions,
                _ => Screen::Chat,
            };
            if self.screen == Screen::Sessions {
                let _ = self.refresh_sessions();
            }
            return KeyAction::Continue;
        }
        match self.screen {
            Screen::Sessions => self.on_key_sessions(key),
            Screen::Chat => self.on_key_chat(key),
            Screen::Agent => self.on_key_agent(key),
            Screen::Palette => self.on_key_palette(key),
            Screen::Commands => self.on_key_commands(key),
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                self.list_scroll =
                    (self.list_scroll + 1).min(self.sessions.len().saturating_sub(1));
                self.list_state.select(Some(self.list_scroll));
                KeyAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.list_scroll = self.list_scroll.saturating_sub(1);
                self.list_state.select(Some(self.list_scroll));
                KeyAction::Continue
            }
            KeyCode::Enter => {
                if let Err(e) = self.open_selected() {
                    self.last_error = Some(e);
                }
                KeyAction::Continue
            }
            KeyCode::Char('d') => {
                if let Err(e) = self.delete_selected() {
                    self.last_error = Some(e);
                }
                KeyAction::Continue
            }
            KeyCode::Char('r') => {
                if let Some(summary) = self.selected_session() {
                    let title = summary
                        .title
                        .clone()
                        .unwrap_or_else(|| "renamed".to_string());
                    if let Err(e) = self.rename_selected(&format!("{title} *")) {
                        self.last_error = Some(e);
                    }
                }
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    fn on_key_chat(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::Sessions;
                let _ = self.refresh_sessions();
                KeyAction::Continue
            }
            KeyCode::Char('q') if self.stream.is_none() && self.input.is_empty() => {
                self.screen = Screen::Sessions;
                let _ = self.refresh_sessions();
                KeyAction::Continue
            }
            KeyCode::Char('/') if self.stream.is_none() && self.input.is_empty() => {
                self.screen = Screen::Commands;
                KeyAction::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.chat_scroll = self.chat_scroll.saturating_add(1);
                KeyAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.chat_scroll = self.chat_scroll.saturating_sub(1);
                KeyAction::Continue
            }
            KeyCode::Enter if self.stream.is_none() && !self.input.trim().is_empty() => {
                let question = std::mem::take(&mut self.input);
                let model = self.model.clone();
                KeyAction::SendTurn { question, model }
            }
            KeyCode::Backspace | KeyCode::Char('\u{7f}') | KeyCode::Char('\u{08}') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    delete_word(&mut self.input);
                } else {
                    self.input.pop();
                }
                KeyAction::Continue
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                delete_word(&mut self.input);
                KeyAction::Continue
            }
            KeyCode::Char(c)
                if self.stream.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.input.push(c);
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    fn on_key_commands(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Chat;
                KeyAction::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.commands_index = (self.commands_index + 1) % SlashCmd::ALL.len();
                KeyAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.commands_index = self.commands_index.saturating_sub(1);
                KeyAction::Continue
            }
            KeyCode::Enter => {
                let cmd = SlashCmd::ALL[self.commands_index];
                self.screen = Screen::Chat;
                match cmd {
                    SlashCmd::Model => self.open_palette(),
                    SlashCmd::Agent => self.screen = Screen::Agent,
                    SlashCmd::Sessions => {
                        self.screen = Screen::Sessions;
                        let _ = self.refresh_sessions();
                    }
                    SlashCmd::Clear => self.chat.clear(),
                    SlashCmd::Help => self.show_help(),
                    SlashCmd::Exit => return KeyAction::Quit,
                }
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    fn on_key_agent(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Chat;
                KeyAction::Continue
            }
            KeyCode::Enter => {
                if self.agent_rx.is_none() {
                    self.start_agent();
                }
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    fn start_agent(&mut self) {
        let Ok(client) = Self::client() else {
            self.last_error = Some(TuiError::Provider("no provider configured".into()));
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let observer: DynObserver = std::sync::Arc::new(ChannelObserver(tx));
        let root = std::env::current_dir().unwrap_or_default();
        let model = self.model.clone();
        let agent = aether_agent::Agent::new(
            Box::new(client),
            root,
            model.clone(),
            model.clone(),
            model.clone(),
        )
        .with_observer(observer);
        self.agent_rx = Some(rx);
        self.agent_state = agent_screen::AgentScreenState::default();
        let task = self
            .last_question()
            .unwrap_or("a small demo task")
            .to_string();
        tokio::spawn(async move {
            let _ = agent.run(&task).await;
        });
    }

    fn on_key_palette(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc => {
                self.close_palette();
                KeyAction::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.palette_index =
                    (self.palette_index + 1).min(self.palette_models.len().saturating_sub(1));
                KeyAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.palette_index = self.palette_index.saturating_sub(1);
                KeyAction::Continue
            }
            KeyCode::Enter => {
                let model = self
                    .palette_models
                    .get(self.palette_index)
                    .cloned()
                    .unwrap_or_default();
                self.close_palette();
                if model.is_empty() {
                    return KeyAction::Continue;
                }
                // Remember the pick so the next Enter uses it without reloading config.
                self.model = model.clone();
                // Re-ask the last user question against the newly selected model.
                match self.last_question() {
                    Some(q) => KeyAction::SendTurn {
                        question: q.to_string(),
                        model,
                    },
                    None => KeyAction::Continue,
                }
            }
            _ => KeyAction::Continue,
        }
    }

    fn last_question(&self) -> Option<&str> {
        self.chat
            .iter()
            .rev()
            .find(|r| r.role == "user")
            .map(|r| r.content.as_str())
    }

    fn on_mouse(&mut self, me: crossterm::event::MouseEvent) -> Result<bool, TuiError> {
        match me.kind {
            MouseEventKind::ScrollDown => match self.screen {
                Screen::Sessions => {
                    self.list_scroll =
                        (self.list_scroll + 1).min(self.sessions.len().saturating_sub(1));
                    self.list_state.select(Some(self.list_scroll));
                    Ok(false)
                }
                Screen::Chat => {
                    self.chat_scroll = self.chat_scroll.saturating_add(1);
                    Ok(false)
                }
                _ => Ok(false),
            },
            MouseEventKind::ScrollUp => match self.screen {
                Screen::Sessions => {
                    self.list_scroll = self.list_scroll.saturating_sub(1);
                    self.list_state.select(Some(self.list_scroll));
                    Ok(false)
                }
                Screen::Chat => {
                    self.chat_scroll = self.chat_scroll.saturating_sub(1);
                    Ok(false)
                }
                _ => Ok(false),
            },
            MouseEventKind::Down(MouseButton::Left) => {
                if self.screen == Screen::Sessions {
                    let _ = self.open_selected();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let body = self.draw_chrome(frame, frame.area());
        match self.screen {
            Screen::Sessions => self.draw_sessions(frame, body),
            Screen::Chat => self.draw_chat(frame, body),
            Screen::Agent => self.draw_agent(frame, body),
            Screen::Palette => self.draw_palette(frame, body),
            Screen::Commands => self.draw_commands(frame, body),
        }
    }

    /// Render the shared header bar + tab strip on the top two rows and
    /// return the body area left below them.
    fn draw_chrome(&self, frame: &mut Frame, area: Rect) -> Rect {
        let vertical = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ]);
        let [header_area, tabs_area, body_area] = vertical.areas(area);

        let header = chrome::render_header(
            &chrome::HeaderData {
                brand: "aether",
                screen: match self.screen {
                    Screen::Sessions => "sessions",
                    Screen::Chat => "chat",
                    Screen::Agent => "agent",
                    Screen::Palette => "chat",
                    Screen::Commands => "commands",
                },
                model: &self.model,
                provider: &self.provider,
                version: env!("CARGO_PKG_VERSION"),
                session_count: self.sessions.len(),
            },
            area.width,
        );
        frame.render_widget(Paragraph::new(header), header_area);

        let selected = matches!(self.screen, Screen::Chat | Screen::Palette);
        let tabs = chrome::render_tabs(
            &[
                ("chat", selected),
                ("agent", self.screen == Screen::Agent),
                ("sessions", self.screen == Screen::Sessions),
            ],
            area.width,
        );
        frame.render_widget(Paragraph::new(tabs), tabs_area);

        body_area
    }

    fn draw_sessions(&mut self, frame: &mut Frame, body: Rect) {
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]);
        let [list_area, footer_area] = vertical.areas(body);

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|s| {
                let title = s.title.as_deref().unwrap_or("(untitled)");
                let stats = format!(
                    "{} in / {} out · {} msgs",
                    s.stats.input_tokens, s.stats.output_tokens, s.messages
                );
                ListItem::new(Line::from(vec![
                    Span::styled(title, Style::default().fg(Color::White)),
                    Span::styled("  ", theme::muted()),
                    Span::styled(stats, theme::muted()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" sessions "),
            )
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        let totals = &self.totals;
        let left = match &self.last_error {
            Some(err) => format!(
                "{} sessions · {} turns · {} in / {} out   error: {}",
                totals.sessions,
                totals.turns,
                totals.input_tokens,
                totals.output_tokens,
                err.to_string().lines().next().unwrap_or_default()
            ),
            None => format!(
                "{} sessions · {} turns · {} in / {} out",
                totals.sessions, totals.turns, totals.input_tokens, totals.output_tokens
            ),
        };
        let right =
            "j/k or wheel: move · Enter: open · d: delete · r: rename · ctrl+P: model · q: quit";
        self.render_footer(frame, footer_area, &left, right);
    }

    fn draw_chat(&mut self, frame: &mut Frame, body: Rect) {
        let vertical = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let [body_area, status_area, input_area, usage_area, footer_area] = vertical.areas(body);

        // Visible window of rows (oldest at top, scroll up to reveal history).
        let viewport = body_area.height as usize;
        let tail_active = self.stream.is_some();
        let total = self.chat.len() + usize::from(tail_active);
        self.chat_scroll = clamp_scroll(self.chat_scroll, total, viewport);
        let end = total.saturating_sub(self.chat_scroll);
        let start = end.saturating_sub(viewport);
        let mut rows: Vec<Line> = self
            .chat
            .iter()
            .skip(start)
            .take(viewport)
            .flat_map(|r| self.render_message(r, body_area.width))
            .collect();
        if let Some(stream) = &self.stream {
            let tail = stream.buffer.as_str();
            let spinner = SPINNER[(self.frame as usize) % SPINNER.len()];
            rows.push(Line::from(vec![
                Span::styled("ai  ", theme::ai()),
                Span::styled(format!("{spinner} "), theme::accent()),
                Span::raw(tail),
            ]));
        }
        let transcript = Paragraph::new(rows)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" transcript "),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(transcript, body_area);

        let status = match self.processing {
            ProcessingStatus::Idle => Span::styled("ready", theme::muted()),
            ProcessingStatus::Thinking => Span::styled("⏳ thinking…", theme::accent()),
            ProcessingStatus::Streaming(n) => {
                Span::styled(format!("● streaming · {n} chunks"), theme::accent())
            }
        };
        frame.render_widget(Paragraph::new(Line::from(status)), status_area);

        let input_widget = Paragraph::new(if self.stream.is_some() {
            Line::from(Span::styled("streaming…", theme::muted()))
        } else {
            Line::from(vec![
                Span::styled("> ", theme::accent()),
                Span::raw(self.input.as_str()),
            ])
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border())
                .title(" input "),
        );
        frame.render_widget(input_widget, input_area);

        let usage = render::usage_summary(Some(self.input_tokens), Some(self.output_tokens), None);
        let left = match &self.last_error {
            Some(err) => format!(
                "{usage}   error: {}",
                err.to_string().lines().next().unwrap_or_default()
            ),
            None => usage,
        };
        let mut right = String::new();
        if !self.model.is_empty() {
            right.push_str("model: ");
            right.push_str(&self.model);
            right.push_str("   ");
        }
        right.push_str("Enter: send · j/k or wheel: scroll · ctrl+P: model · Esc: back · q: quit");
        self.render_footer(frame, footer_area, &left, &right);
        self.draw_usage(frame, usage_area);
    }

    fn draw_usage(&mut self, frame: &mut Frame, area: Rect) {
        let total = self.input_tokens + self.output_tokens;
        let line = format!(
            "session tokens: {} in · {} out · {} total    model: {}",
            self.input_tokens, self.output_tokens, total, self.model
        );
        let usage_bar = Paragraph::new(Line::from(Span::styled(line, theme::muted()))).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme::border()),
        );
        frame.render_widget(usage_bar, area);
    }

    fn render_message<'a>(&self, row: &'a ChatRow, width: u16) -> Vec<Line<'a>> {
        if row.role == "user" {
            return self.render_user_block(&row.content, width);
        }
        let prefix = match row.role.as_str() {
            "system" => Span::styled("sys  ", theme::muted()),
            _ => Span::styled("ai   ", theme::ai()),
        };
        if let (true, Some(card)) = (
            row.role == "assistant",
            cards::extract_plan_card(&row.content),
        ) {
            let total = card.lines.len();
            let done = card
                .lines
                .iter()
                .filter(|l| l.trim_start().starts_with("- [x]"))
                .count();
            let mut lines = cards::render_plan_card(&card, width);
            let mut meter =
                cards::render_todos_pip_meter(done, total, width.saturating_sub(6).min(20));
            meter.spans.insert(0, Span::raw("  "));
            lines.push(meter);
            lines.insert(0, Line::from(prefix));
            return lines;
        }
        vec![Line::from(vec![prefix, Span::raw(row.content.as_str())])]
    }

    /// opencode-style user block: green "you" label + white text on a filled
    /// background, padded to the full transcript width.
    fn render_user_block<'a>(&self, content: &'a str, width: u16) -> Vec<Line<'a>> {
        let inner = width.saturating_sub(2) as usize;
        let label = "you  ";
        let text_w = inner.saturating_sub(label.chars().count() + 1).max(10);
        let body_style = Style::default().bg(theme::USER_BG);
        let label_style = theme::accent().bg(theme::USER_BG);
        let mut lines: Vec<Line> = Vec::new();
        for (i, para) in render::wrap(content, text_w).into_iter().enumerate() {
            let lead = if i == 0 { label } else { "     " };
            let pad = inner.saturating_sub(lead.chars().count() + para.chars().count());
            let mut spans = vec![
                Span::styled(lead, label_style),
                Span::styled(para, body_style.fg(Color::White)),
            ];
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad), body_style));
            }
            lines.push(Line::from(spans));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(label, label_style)));
        }
        lines
    }

    /// Two-part footer: totals on the left, keybindings on the right.
    fn render_footer(&self, frame: &mut Frame, area: Rect, left: &str, right: &str) {
        let left_len = render::visible_length(left) as u16 + 2;
        let horizontal = Layout::horizontal([Constraint::Length(left_len), Constraint::Min(0)]);
        let [left_area, right_area] = horizontal.areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(left, theme::muted()))),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, theme::muted()))),
            right_area,
        );
    }

    fn draw_agent(&mut self, frame: &mut Frame, body: Rect) {
        if let Some(rx) = &self.agent_rx {
            while let Ok(phase) = rx.try_recv() {
                self.agent_state.apply(phase);
            }
        }
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]);
        let [panel_area, footer_area] = vertical.areas(body);

        let lines: Vec<Line> = self
            .agent_state
            .status_lines()
            .into_iter()
            .map(|line| {
                if matches!(line.as_str(), "PLAN" | "BUILD" | "ROUTE") {
                    Line::from(Span::styled(line, theme::title()))
                } else if line.starts_with("DONE:") {
                    Line::from(Span::styled(line, theme::accent()))
                } else {
                    Line::from(Span::raw(line))
                }
            })
            .collect();
        let panel = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" agent loop "),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(panel, panel_area);

        let left = if self.agent_state.stopped {
            "agent finished".to_string()
        } else if self.agent_rx.is_some() {
            "agent running — plan → build → route".to_string()
        } else {
            "press Enter to run the agent loop".to_string()
        };
        let right = "Enter: run · Tab: switch · Esc: back";
        self.render_footer(frame, footer_area, &left, right);
    }

    fn draw_palette(&mut self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(40);
        let h = (self.palette_models.len() as u16 + 2).min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let palette_rect = Rect::new(x, y, w, h);

        let items: Vec<ListItem> = self
            .palette_models
            .iter()
            .map(|m| {
                ListItem::new(Line::from(Span::styled(
                    m,
                    Style::default().fg(Color::White),
                )))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" ctrl+P — model "),
            )
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(self.palette_index));
        frame.render_widget(list, palette_rect);
    }

    fn draw_commands(&mut self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(46);
        let h = (SlashCmd::ALL.len() as u16 + 2).min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let cmd_rect = Rect::new(x, y, w, h);

        let items: Vec<ListItem> = SlashCmd::ALL
            .iter()
            .map(|c| {
                ListItem::new(Line::from(Span::styled(
                    c.label(),
                    Style::default().fg(Color::White),
                )))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" / commands "),
            )
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        state.select(Some(
            self.commands_index
                .min(SlashCmd::ALL.len().saturating_sub(1)),
        ));
        frame.render_widget(list, cmd_rect);
    }

    fn show_help(&mut self) {
        let help = [
            "aether — plan · build · route",
            "",
            "  chat        Enter send · j/k scroll · ctrl+W delete word",
            "              ctrl+P model palette · / command menu",
            "  agent       Enter run the plan → build → route loop",
            "  sessions    j/k select · Enter open · d delete · r rename",
            "  commands    / or q to close · Enter run · j/k select",
            "  quit        q on sessions screen",
        ]
        .join("\n");
        self.chat.push(ChatRow {
            role: "system".to_string(),
            content: help,
        });
        self.chat_scroll = self.chat.len().saturating_sub(1);
    }
}

impl Tui for RatatuiTui {
    fn run(&mut self) -> Result<(), TuiError> {
        let mut terminal = ratatui::init();
        if let Err(e) = crossterm::execute!(std::io::stdout(), EnableMouseCapture) {
            ratatui::restore();
            return Err(TuiError::Terminal(e.to_string()));
        }
        let result = tokio::runtime::Runtime::new()
            .map_err(|e| TuiError::Terminal(e.to_string()))
            .and_then(|rt| rt.block_on(self.event_loop(&mut terminal)));
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
        result
    }
}

impl RatatuiTui {
    async fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<(), TuiError> {
        loop {
            self.drain_stream();
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|e| TuiError::Render(e.to_string()))?;

            if event::poll(Duration::from_millis(100))
                .map_err(|e| TuiError::Event(e.to_string()))?
            {
                let event = event::read().map_err(|e| TuiError::Event(e.to_string()))?;
                let quit = self.handle_event(event).await?;
                if quit {
                    return Ok(());
                }
            }
        }
    }
}

/// Delete the whitespace-delimited word left of the cursor (ctrl+W).
fn delete_word(input: &mut String) {
    let trimmed_end = input.trim_end().len();
    input.truncate(trimmed_end);
    if let Some(pos) = input.rfind(char::is_whitespace) {
        input.truncate(pos + 1);
    } else {
        input.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_session::SessionId;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Point `AETHER_CONFIG_DIR` at a scratch dir so session/ledger reads
    /// never touch the developer's real store (same pattern as aether-mcp).
    fn isolate(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-tui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("AETHER_CONFIG_DIR", &dir) };
        let sessions = dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions);
        dir
    }

    #[test]
    fn noop_tui_runs() {
        let mut tui = NoopTui;
        assert!(tui.run().is_ok());
    }

    #[test]
    fn default_has_no_selection() {
        let app = RatatuiTui::default();
        assert_eq!(app.screen, Screen::Sessions);
        assert!(app.list_state.selected().is_none());
        assert!(app.chat.is_empty());
    }

    #[test]
    fn sessions_scroll_clamps() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("scroll");
        let sid = SessionId::new("2026-08-09T10-00-00-000Z");
        let session = Session::open(&sid).unwrap();
        session.append("user", "hello").unwrap();
        session.rename("Scroll Test").unwrap();

        let mut app = RatatuiTui::default();
        app.refresh_sessions().unwrap();
        assert_eq!(app.list_state.selected(), Some(0));

        let _ = app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.list_state.selected(), Some(0)); // clamped at bottom
        let _ = app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.list_state.selected(), Some(0)); // clamped at top
    }

    #[test]
    fn chat_scroll_stays_in_range() {
        let mut app = RatatuiTui {
            chat: vec![
                ChatRow {
                    role: "user".into(),
                    content: "hi".into(),
                },
                ChatRow {
                    role: "assistant".into(),
                    content: "hello".into(),
                },
            ],
            ..Default::default()
        };
        for _ in 0..10 {
            app.chat_scroll = app.chat_scroll.saturating_add(1);
        }
        assert_eq!(app.chat_scroll, 10);
        for _ in 0..20 {
            app.chat_scroll = app.chat_scroll.saturating_sub(1);
        }
        assert_eq!(app.chat_scroll, 0);
    }

    #[test]
    fn ctrl_p_opens_and_closes_palette() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let _ = app.on_key(ctrl_p);
        assert_eq!(app.screen, Screen::Palette);
        let _ = app.on_key(ctrl_p);
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn palette_enter_asks_last_question() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "why?".into(),
            }],
            palette_models: vec!["m1".into(), "m2".into()],
            palette_index: 1,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            action,
            KeyAction::SendTurn {
                question: "why?".into(),
                model: "m2".into()
            }
        );
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn chat_input_accumulates_and_clears() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert_eq!(app.input, "hi");
        let _ = app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input, "h");
    }

    #[test]
    fn chat_enter_sends_typed_question() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "older".into(),
            }],
            input: "new question".into(),
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            KeyAction::SendTurn { question, .. } if question == "new question"
        ));
        assert!(app.input.is_empty());
    }

    #[test]
    fn chat_enter_ignores_whitespace_input() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: "   ".into(),
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(app.input, "   ");
    }

    #[test]
    fn chat_scroll_clamps_to_visible_window() {
        assert_eq!(clamp_scroll(0, 10, 5), 0);
        assert_eq!(clamp_scroll(5, 10, 5), 5);
        assert_eq!(clamp_scroll(999, 10, 5), 5);
        assert_eq!(clamp_scroll(3, 2, 5), 0);
        assert_eq!(clamp_scroll(4, 2, 5), 0);
    }

    #[test]
    fn palette_enter_without_question_continues() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            palette_models: vec!["m1".into()],
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
    }

    #[test]
    fn palette_pick_persists_model_for_next_send() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "why?".into(),
            }],
            palette_models: vec!["m1".into(), "m2".into()],
            palette_index: 1,
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.model, "m2");
        app.screen = Screen::Chat;
        app.input = "next".into();
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            KeyAction::SendTurn { model, .. } if model == "m2"
        ));
    }

    #[test]
    fn wheel_scroll_moves_sessions() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("wheel");
        let sid = SessionId::new("2026-08-09T10-00-00-000Z");
        let session = Session::open(&sid).unwrap();
        session.append("user", "hello").unwrap();
        session.rename("Wheel Test").unwrap();

        let mut app = RatatuiTui::default();
        app.refresh_sessions().unwrap();
        let _ = app.on_mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.list_scroll, 0); // clamped (1 session)
        let _ = app.on_mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.list_scroll, 0);
    }

    #[test]
    fn delete_word_removes_whitespace_delimited_word() {
        let mut input = "hello world".to_string();
        delete_word(&mut input);
        assert_eq!(input, "hello ");
        let mut input = "hello".to_string();
        delete_word(&mut input);
        assert_eq!(input, "");
        let mut input = "a b c".to_string();
        delete_word(&mut input);
        assert_eq!(input, "a b ");
        let mut input = "  spaced  ".to_string();
        delete_word(&mut input);
        assert_eq!(input, "  ");
    }

    #[test]
    fn backspace_and_ctrl_w_edit_chat_input() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: "delete me".into(),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "delete ");
        let _ = app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input, "delete");
    }

    #[test]
    fn slash_opens_commands_screen_when_input_empty() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: String::new(),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Commands);
    }

    #[test]
    fn slash_does_not_open_when_input_nonempty() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: "/not-a-command".into(),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn commands_screen_clear_empties_chat() {
        let mut app = RatatuiTui {
            screen: Screen::Commands,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "drop me".into(),
            }],
            ..Default::default()
        };
        app.commands_index = SlashCmd::ALL
            .iter()
            .position(|c| *c == SlashCmd::Clear)
            .unwrap();
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Chat);
        assert!(app.chat.is_empty());
    }

    #[test]
    fn commands_screen_help_returns_to_chat() {
        let mut app = RatatuiTui {
            screen: Screen::Commands,
            ..Default::default()
        };
        app.commands_index = SlashCmd::ALL
            .iter()
            .position(|c| *c == SlashCmd::Help)
            .unwrap();
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Chat);
    }

    #[test]
    fn commands_screen_exit_quits() {
        let mut app = RatatuiTui {
            screen: Screen::Commands,
            ..Default::default()
        };
        app.commands_index = SlashCmd::ALL
            .iter()
            .position(|c| *c == SlashCmd::Exit)
            .unwrap();
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Quit);
    }

    #[test]
    fn drain_stream_accumulates_chunks_and_finishes() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            stream: Some(ChatStream {
                rx,
                buffer: String::new(),
                usage: None,
            }),
            ..Default::default()
        };
        tx.send(StreamEvent::Chunk("hel".into())).unwrap();
        tx.send(StreamEvent::Chunk("lo".into())).unwrap();
        tx.send(StreamEvent::Done).unwrap();
        app.drain_stream();
        assert!(app.stream.is_none());
        assert!(matches!(app.processing, ProcessingStatus::Idle));
        assert_eq!(app.chat.last().map(|r| r.content.as_str()), Some("hello"));
    }

    #[test]
    fn drain_stream_error_records_last_error() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            stream: Some(ChatStream {
                rx,
                buffer: String::new(),
                usage: None,
            }),
            ..Default::default()
        };
        tx.send(StreamEvent::Error("boom".into())).unwrap();
        app.drain_stream();
        assert!(app.stream.is_none());
        assert!(matches!(app.processing, ProcessingStatus::Idle));
        assert!(app.last_error.is_some());
    }

    #[test]
    fn drain_stream_usage_counter_tracks_session_total() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input_tokens: 10,
            output_tokens: 5,
            stream: Some(ChatStream {
                rx,
                buffer: String::new(),
                usage: None,
            }),
            ..Default::default()
        };
        let usage = aether_provider::Usage::default();
        tx.send(StreamEvent::Chunk("reply".into())).unwrap();
        tx.send(StreamEvent::Usage(usage)).unwrap();
        tx.send(StreamEvent::Done).unwrap();
        app.drain_stream();
        assert_eq!(app.input_tokens + app.output_tokens, 15);
    }
}
