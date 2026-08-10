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

/// Live streaming state shown on the status line above the chat input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ProcessingStatus {
    #[default]
    Idle,
    Thinking,
    Streaming(u64),
}

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
    /// Streaming buffer while a response is arriving.
    pending: Option<String>,
    processing: ProcessingStatus,
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
            pending: None,
            processing: ProcessingStatus::Idle,
            input_tokens: 0,
            output_tokens: 0,
            input: String::new(),
            model: String::new(),
            palette_models: Vec::new(),
            palette_index: 0,
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
        let mut app = Self::default();
        app.model = default_model();
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

    /// Send one user turn; stream the reply into `self.chat` and persist.
    async fn send_turn(&mut self, question: String, model: &str) -> Result<(), TuiError> {
        let client = Self::client()?;
        // Context window: the last 40 rows bounds RAM + token spend on long chats.
        const HISTORY_WINDOW: usize = 40;
        let start = self.chat.len().saturating_sub(HISTORY_WINDOW);
        let mut history: Vec<ChatMessage> = self.chat[start..]
            .iter()
            .map(|r| ChatMessage {
                role: r.role.clone(),
                content: r.content.clone(),
                ..ChatMessage::default()
            })
            .collect();
        history.push(ChatMessage {
            role: "user".to_string(),
            content: question.clone(),
            ..ChatMessage::default()
        });

        let request = ChatRequest {
            model: model.to_string(),
            messages: history,
            temperature: None,
            stream: true,
            tools: None,
        };

        self.processing = ProcessingStatus::Thinking;
        let mut usage = None;
        let stream = match client.stream_chat(&request).await {
            Ok(stream) => stream,
            Err(e) => {
                self.processing = ProcessingStatus::Idle;
                return Err(TuiError::Provider(e.to_string()));
            }
        };
        futures::pin_mut!(stream);
        // Stream straight into the pending buffer: one allocation, no
        // per-chunk clones (a long reply used to be copied on every chunk).
        self.pending = Some(String::new());
        let mut tokens = 0u64;
        {
            let buffer = self.pending.as_mut().expect("pending just set");
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        self.processing = ProcessingStatus::Idle;
                        return Err(TuiError::Provider(e.to_string()));
                    }
                };
                tokens += 1;
                self.processing = ProcessingStatus::Streaming(tokens);
                if let Some(text) = &chunk.content {
                    buffer.push_str(text);
                }
                if chunk.usage.is_some() {
                    usage = chunk.usage;
                }
            }
        }
        self.processing = ProcessingStatus::Idle;
        let reply = self.pending.take().unwrap_or_default();

        if let Some(session) = &self.active {
            session
                .append("user", &question)
                .map_err(|e| TuiError::Session(e.to_string()))?;
            session
                .append("assistant", &reply)
                .map_err(|e| TuiError::Session(e.to_string()))?;
            if let Some(u) = usage {
                let input = u.prompt_tokens.unwrap_or(0);
                let output = u.completion_tokens.unwrap_or(0);
                session
                    .append_usage(aether_session::SessionMeta {
                        turns: 1,
                        input_tokens: input,
                        output_tokens: output,
                    })
                    .map_err(|e| TuiError::Session(e.to_string()))?;
                self.input_tokens += input;
                self.output_tokens += output;
            }
        }
        self.chat.push(ChatRow {
            role: "user".to_string(),
            content: question,
        });
        self.chat.push(ChatRow {
            role: "assistant".to_string(),
            content: reply,
        });
        self.chat_scroll = self.chat.len().saturating_sub(1);
        Ok(())
    }

    /// Drive one event and return `true` to quit.
    async fn handle_event(&mut self, event: Event) -> Result<bool, TuiError> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match self.on_key(key) {
                KeyAction::Continue => Ok(false),
                KeyAction::Quit => Ok(true),
                KeyAction::SendTurn { question, model } => {
                    self.send_turn(question, &model).await?;
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
            KeyCode::Char('q') if self.pending.is_none() && self.input.is_empty() => {
                self.screen = Screen::Sessions;
                let _ = self.refresh_sessions();
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
            KeyCode::Enter if self.pending.is_none() && !self.input.trim().is_empty() => {
                let question = std::mem::take(&mut self.input);
                let model = self.model.clone();
                KeyAction::SendTurn { question, model }
            }
            KeyCode::Backspace => {
                self.input.pop();
                KeyAction::Continue
            }
            KeyCode::Char(c)
                if self.pending.is_none() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.input.push(c);
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
        ]);
        let [body_area, status_area, input_area, footer_area] = vertical.areas(body);

        // Visible window of rows (oldest at top, scroll up to reveal history).
        let viewport = body_area.height as usize;
        let total = self.chat.len() + usize::from(self.pending.is_some());
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
        if let Some(tail) = &self.pending {
            rows.push(Line::from(vec![
                Span::styled("ai  ", theme::ai()),
                Span::raw(tail.as_str()),
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

        let input_widget = Paragraph::new(if self.pending.is_some() {
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
    }

    fn render_message<'a>(&self, row: &'a ChatRow, width: u16) -> Vec<Line<'a>> {
        let prefix = match row.role.as_str() {
            "user" => Span::styled("you  ", theme::accent()),
            "system" => Span::styled("sys  ", theme::muted()),
            _ => Span::styled("ai   ", theme::ai()),
        };
        if row.role == "assistant" {
            if let Some(card) = cards::extract_plan_card(&row.content) {
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
        }
        vec![Line::from(vec![prefix, Span::raw(row.content.as_str())])]
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
}
