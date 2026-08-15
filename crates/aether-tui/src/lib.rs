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
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

use aether_agent::ChannelObserver;
use aether_agent::observer::DynObserver;
use aether_core::config::{AetherConfig, CustomProviderConfig, load_config, save_config};
use aether_provider::{
    ChatMessage, ChatRequest, OpenAICompatibleClient, Provider, list_providers, resolve_default,
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
/// Semantic roles — green = brand/you/positive (the single accent), indigo =
/// AI/agent, sky = info/titles, slate = borders/muted, red = danger.
/// RGB values from the ui-ux-pro-max design system for developer tools.
mod theme {
    use ratatui::style::{Color, Modifier, Style};

    pub const ACCENT: Color = Color::Rgb(0x22, 0xc5, 0x5e);
    pub const AI: Color = Color::Rgb(0x81, 0x8c, 0xf8);
    pub const TITLE: Color = Color::Rgb(0x38, 0xbd, 0xf8);
    pub const BORDER: Color = Color::Rgb(0x33, 0x41, 0x55);
    pub const MUTED: Color = Color::Rgb(0x64, 0x74, 0x8b);
    pub const DANGER: Color = Color::Rgb(0xef, 0x44, 0x44);
    pub const SELECT_BG: Color = Color::Rgb(0x1e, 0x29, 0x3b);
    /// Background fill behind user messages (opencode-style "You" block).
    pub const USER_BG: Color = Color::Rgb(0x16, 0x22, 0x30);
    /// Background fill behind fenced code blocks in the transcript.
    pub const CODE_BG: Color = Color::Rgb(0x0f, 0x17, 0x22);

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
    pub fn danger() -> Style {
        Style::default().fg(DANGER).add_modifier(Modifier::BOLD)
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
    /// Sequential input flow for adding a custom model (from the palette).
    AddModel,
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

/// Which field the add-model flow is currently editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddModelField {
    BaseUrl,
    ApiKeyEnv,
    ModelName,
}

/// Sequential input state for the "add custom model" flow: one field at a
/// time, Esc cancels at any step, Enter advances (validated on commit).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AddModelState {
    field: AddModelField,
    base_url: String,
    api_key_env: String,
    model_name: String,
}

impl AddModelState {
    /// The string being edited for the current field.
    fn current(&mut self) -> &mut String {
        match self.field {
            AddModelField::BaseUrl => &mut self.base_url,
            AddModelField::ApiKeyEnv => &mut self.api_key_env,
            AddModelField::ModelName => &mut self.model_name,
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

/// Redraw tick: the event loop polls input with this timeout and redraws on
/// every iteration, so the spinner animates while the model is thinking.
const TICK_MS: u64 = 100;

/// The right usage/session sidebar is only drawn when the body is at least
/// this wide, so the transcript never drops below ~40 columns.
const SIDEBAR_MIN_BODY_WIDTH: u16 = 72;
/// Fixed width of the right usage/session sidebar column. 32 fits the
/// longest line (`in 1.2K · out 3.4K · total 4.6K`, 31 chars) plus the
/// left border without truncation.
const SIDEBAR_WIDTH: u16 = 32;

/// Pick the spinner frame for animation tick `frame` (wraps around).
fn spinner_frame(frame: u32) -> char {
    SPINNER[(frame as usize) % SPINNER.len()]
}

/// Status line above the chat input: spinner + state label while processing,
/// `ready` when idle. Pure — unit-testable without a terminal.
fn status_line(processing: ProcessingStatus, frame: u32) -> Line<'static> {
    match processing {
        ProcessingStatus::Idle => Line::from(Span::styled("ready", theme::muted())),
        ProcessingStatus::Thinking => {
            let spinner = spinner_frame(frame);
            Line::from(Span::styled(
                format!("{spinner} thinking…"),
                theme::accent(),
            ))
        }
        ProcessingStatus::Streaming(n) => {
            let spinner = spinner_frame(frame);
            Line::from(Span::styled(
                format!("{spinner} streaming · {n} chunks"),
                theme::accent(),
            ))
        }
    }
}

/// Live `plan -> build -> route` badge for the chat status line while an
/// agent run is active: the current stage is highlighted in accent, the
/// other two are muted. Pure — unit-testable without a terminal.
fn agent_badge(phase: agent_screen::ScreenPhase) -> Line<'static> {
    let active = match phase {
        agent_screen::ScreenPhase::Planning | agent_screen::ScreenPhase::PlanReady => 0,
        agent_screen::ScreenPhase::BuildStarted
        | agent_screen::ScreenPhase::ToolCalled
        | agent_screen::ScreenPhase::BuildFinished => 1,
        agent_screen::ScreenPhase::Routing | agent_screen::ScreenPhase::Routed => 2,
        agent_screen::ScreenPhase::Idle | agent_screen::ScreenPhase::Finished => usize::MAX,
    };
    let stages = ["plan", "build", "route"];
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, stage) in stages.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" -> ", theme::muted()));
        }
        let style = if i == active {
            theme::accent()
        } else {
            theme::muted()
        };
        spans.push(Span::styled(*stage, style));
    }
    Line::from(spans)
}

/// Welcome screen for an empty chat: the aether logo centered in the
/// transcript, a `plan · build · route` tagline on the wordmark line, and a
/// muted hint line — vertically centered inside the transcript's inner area.
/// Pure — unit-testable without a terminal.
fn welcome_rows(width: u16, height: u16) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2) as usize;
    let mut rows: Vec<Line<'static>> = Vec::new();
    let logo = chrome::render_logo();
    let last = logo.len() - 1;
    for (i, logo_line) in logo.into_iter().enumerate() {
        let mut spans = logo_line.spans;
        if i == last {
            spans.push(Span::styled(" — plan · build · route", theme::muted()));
        }
        let text_len: usize = spans.iter().map(|s| s.width()).sum();
        let pad = inner.saturating_sub(text_len) / 2;
        let mut full = vec![Span::raw(" ".repeat(pad))];
        full.extend(spans);
        rows.push(Line::from(full));
    }
    rows.push(Line::from(""));
    let hint = "type a message to start · / for commands · ctrl+P for model";
    let pad = inner.saturating_sub(hint.chars().count()) / 2;
    rows.push(Line::from(vec![
        Span::raw(" ".repeat(pad)),
        Span::styled(hint, theme::muted()),
    ]));
    let inner_h = height.saturating_sub(2) as usize;
    let top = inner_h.saturating_sub(rows.len()) / 2;
    let mut padded = Vec::with_capacity(top + rows.len());
    padded.extend(std::iter::repeat_with(|| Line::from("")).take(top));
    padded.extend(rows);
    padded
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
    palette_state: ListState,
    /// Highlighted row of the slash-command picker.
    commands_index: usize,
    commands_state: ListState,
    /// Ledger totals footer.
    totals: aether_session::Totals,
    /// The most recent background error, shown in the footer.
    last_error: Option<TuiError>,
    /// Live state for the Agent screen (fed by the observer channel).
    agent_state: agent_screen::AgentScreenState,
    /// Receiver draining AgentPhase events while an agent run is live.
    agent_rx: Option<std::sync::mpsc::Receiver<aether_agent::AgentPhase>>,
    /// Iteration cap of the running agent loop (0 = unknown/unlimited).
    agent_cap: u32,
    /// Resolved provider name, shown in the header bar.
    provider: String,
    /// Quit-confirmation dialog is open (ctrl+C toggles it; Esc cancels).
    show_quit: bool,
    /// In-flight add-model flow (None when not active).
    add_model: Option<AddModelState>,
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
            palette_state: ListState::default(),
            commands_index: 0,
            commands_state: ListState::default(),
            totals: aether_session::Totals::default(),
            last_error: None,
            agent_state: agent_screen::AgentScreenState::default(),
            agent_rx: None,
            agent_cap: 0,
            provider: String::new(),
            show_quit: false,
            add_model: None,
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

/// Style one diff line for the BUILD panel: `+` additions in accent
/// (green), `-` removals in danger (red), context lines in muted.
fn diff_line_span(line: &str) -> Span<'static> {
    let style = if line.starts_with("+ ") {
        theme::accent()
    } else if line.starts_with("- ") {
        theme::danger()
    } else {
        theme::muted()
    };
    Span::styled(format!("    {line}"), style)
}

/// Lines for the right usage/session sidebar (opencode-style): live token
/// counters, the current session, model/provider, and an honest MCP note.
/// Pure — unit-testable without a terminal. `context_window` of 0 omits the
/// context percentage (the TUI has no context-window source; `draw_chat`
/// passes 0).
fn sidebar_lines(
    input: u64,
    output: u64,
    context_window: u64,
    session: Option<&aether_session::SessionSummary>,
    model: &str,
    provider: &str,
) -> Vec<String> {
    let mut lines = vec![chrome::usage_line(input, output, context_window)];
    match session {
        Some(s) => {
            lines.push(s.title.as_deref().unwrap_or("(untitled)").to_string());
            lines.push(format!("{} turns · {} messages", s.stats.turns, s.messages));
            lines.push(format!(
                "in {} · out {}",
                chrome::format_tokens(s.stats.input_tokens),
                chrome::format_tokens(s.stats.output_tokens)
            ));
        }
        None => lines.push("no session".to_string()),
    }
    lines.push(String::new());
    lines.push(format!("model: {model}"));
    lines.push(format!("provider: {provider}"));
    lines.push("mcp: server crate (external)".to_string());
    lines
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

    /// Open the ctrl+P palette with the default provider's static models
    /// plus every custom model from the config, and reset any in-flight
    /// add-model flow. The cursor starts on the currently active model.
    fn open_palette(&mut self) {
        let config = load_config().unwrap_or_default();
        let resolved = resolve_default(&config, None, None);
        let mut models = resolved.provider.static_models.clone();
        for custom in &config.providers.custom {
            for m in &custom.models {
                if !models.contains(m) {
                    models.push(m.clone());
                }
            }
        }
        self.palette_models = models;
        self.palette_index = self
            .palette_models
            .iter()
            .position(|m| m == &self.model)
            .unwrap_or(0);
        self.add_model = None;
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

    /// Build a client for the provider that serves `model` — custom
    /// providers from the config included — falling back to the default
    /// provider when no provider lists the model. This is what makes a
    /// palette pick of a custom model actually talk to the custom endpoint.
    fn client_for_model(model: &str) -> Result<OpenAICompatibleClient, TuiError> {
        let config = load_config().map_err(|e| TuiError::Session(e.to_string()))?;
        for provider in list_providers(&config) {
            if provider.static_models.iter().any(|m| m == model) {
                return provider
                    .client()
                    .map_err(|e| TuiError::Provider(e.to_string()));
            }
        }
        let resolved = resolve_default(&config, None, None);
        resolved
            .provider
            .client()
            .map_err(|e| TuiError::Provider(e.to_string()))
    }

    /// Queue one user turn: show the message immediately, spawn a background
    /// streaming task, and return — the draw loop keeps running.
    fn start_turn(&mut self, question: String, model: &str) -> Result<(), TuiError> {
        // Flip to Thinking before any fallible I/O or spawn: the very next
        // draw (within the ~100 ms tick) shows the spinner, even when the
        // first streamed chunk is seconds away.
        self.processing = ProcessingStatus::Thinking;
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

        let client = Self::client_for_model(model)?;
        let (tx, rx) = std::sync::mpsc::channel();
        self.stream = Some(ChatStream {
            rx,
            buffer: String::new(),
            usage: None,
        });
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
                    let input = u.prompt_tokens.unwrap_or(0);
                    let output = u.completion_tokens.unwrap_or(0);
                    self.input_tokens += input;
                    self.output_tokens += output;
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

    /// Fold any pending AgentPhase events into `agent_state`. Called on
    /// every draw of both the Chat and Agent screens, so both stay live
    /// from the same observer channel.
    fn drain_agent_events(&mut self) {
        if let Some(rx) = &self.agent_rx {
            while let Ok(phase) = rx.try_recv() {
                self.agent_state.apply(phase);
            }
        }
    }

    /// Persist a completed reply to the session and append it to the chat.
    /// Token counters were already updated live by [`Self::drain_stream`]
    /// when the `Usage` event arrived, so only the ledger row is written here.
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

    /// Clear the chat transcript and reset the scroll (ctrl+L, /clear).
    fn clear_transcript(&mut self) {
        self.chat.clear();
        self.chat_scroll = 0;
    }

    fn on_key(&mut self, key: KeyEvent) -> KeyAction {
        // ctrl+C toggles the quit-confirmation dialog on any screen; it
        // never quits directly (opencode behavior).
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.show_quit = !self.show_quit;
            return KeyAction::Continue;
        }
        // While the quit dialog is open it swallows every other key.
        if self.show_quit {
            return self.on_key_quit_dialog(key);
        }
        // ctrl+P opens/closes the palette on any screen.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            match self.screen {
                Screen::Palette => self.close_palette(),
                _ => self.open_palette(),
            }
            return KeyAction::Continue;
        }
        // ctrl+L clears the chat transcript on any screen (opencode-style).
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
            self.clear_transcript();
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
            Screen::AddModel => self.on_key_add_model(key),
        }
    }

    /// Keys while the quit-confirmation dialog is open: Esc cancels,
    /// Enter or 'q' confirms, everything else is ignored.
    fn on_key_quit_dialog(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc => {
                self.show_quit = false;
                KeyAction::Continue
            }
            KeyCode::Enter | KeyCode::Char('q') => KeyAction::Quit,
            _ => KeyAction::Continue,
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Char('q') => KeyAction::Quit,
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
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                    SlashCmd::Clear => self.clear_transcript(),
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
        self.agent_cap = 6;
        let task = self
            .last_question()
            .unwrap_or("a small demo task")
            .to_string();
        tokio::spawn(async move {
            let _ = agent.run(&task).await;
        });
    }

    fn on_key_palette(&mut self, key: KeyEvent) -> KeyAction {
        // The list is the models plus one trailing "add custom model" entry.
        let total = self.palette_models.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.close_palette();
                KeyAction::Continue
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.palette_index = (self.palette_index + 1) % total;
                KeyAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.palette_index = (self.palette_index + total - 1) % total;
                KeyAction::Continue
            }
            KeyCode::Enter => {
                if self.palette_index == self.palette_models.len() {
                    // "add custom model" entry: start the sequential input flow.
                    self.add_model = Some(AddModelState {
                        field: AddModelField::BaseUrl,
                        base_url: String::new(),
                        api_key_env: String::new(),
                        model_name: String::new(),
                    });
                    self.screen = Screen::AddModel;
                    return KeyAction::Continue;
                }
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

    /// Keys while the add-model flow is active: typing appends to the
    /// current field, Enter advances (validated), Esc cancels back to the
    /// palette without persisting anything.
    fn on_key_add_model(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Esc => {
                self.add_model = None;
                self.screen = Screen::Palette;
                KeyAction::Continue
            }
            KeyCode::Enter => self.advance_add_model(),
            KeyCode::Backspace | KeyCode::Char('\u{7f}') | KeyCode::Char('\u{08}') => {
                if let Some(state) = &mut self.add_model {
                    state.current().pop();
                }
                KeyAction::Continue
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(state) = &mut self.add_model {
                    state.current().push(c);
                }
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    /// Advance the add-model flow one field; on the last field, validate and
    /// persist the new custom provider, then return to the palette with the
    /// new model listed. Empty base URL / model name are refused (the flow
    /// stays on the offending field).
    fn advance_add_model(&mut self) -> KeyAction {
        let Some(state) = &mut self.add_model else {
            return KeyAction::Continue;
        };
        match state.field {
            AddModelField::BaseUrl => {
                if state.base_url.trim().is_empty() {
                    return KeyAction::Continue;
                }
                state.field = AddModelField::ApiKeyEnv;
            }
            AddModelField::ApiKeyEnv => {
                state.field = AddModelField::ModelName;
            }
            AddModelField::ModelName => {
                if state.model_name.trim().is_empty() {
                    return KeyAction::Continue;
                }
                let base_url = state.base_url.trim().to_string();
                let model_name = state.model_name.trim().to_string();
                let api_key_env = state.api_key_env.trim();
                let api_key_env = (!api_key_env.is_empty()).then(|| api_key_env.to_string());
                self.commit_custom_provider(base_url, api_key_env, model_name);
            }
        }
        KeyAction::Continue
    }

    /// Persist a custom provider (base URL + optional key env + model name)
    /// to the config: appends a new provider or extends an existing one
    /// with the same derived name, then reopens the palette with the new
    /// model listed and the cursor on it.
    fn commit_custom_provider(
        &mut self,
        base_url: String,
        api_key_env: Option<String>,
        model_name: String,
    ) {
        let name = provider_name_from_base_url(&base_url);
        let mut config = load_config().unwrap_or_default();
        match config.providers.custom.iter_mut().find(|p| p.name == name) {
            Some(existing) => {
                existing.base_url = base_url;
                if api_key_env.is_some() {
                    existing.api_key_env = api_key_env;
                }
                if !existing.models.contains(&model_name) {
                    existing.models.push(model_name.clone());
                }
            }
            None => config.providers.custom.push(CustomProviderConfig {
                name,
                base_url,
                api_key_env,
                models: vec![model_name.clone()],
                default_model: None,
            }),
        }
        persist_config(&config);
        self.add_model = None;
        self.open_palette();
        if let Some(pos) = self.palette_models.iter().position(|m| m == &model_name) {
            self.palette_index = pos;
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

    /// Best-effort terminal tab/window title (OSC 0 via crossterm `SetTitle`),
    /// mirroring opencode/claude-code: page first, then the active session
    /// name. Some terminals ignore the escape sequence — errors are swallowed.
    fn update_title(&self) {
        let session_title = self.active.as_ref().and_then(|s| s.title().ok());
        let title = title_for(self.screen, session_title.as_deref(), &self.model);
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(title));
    }

    fn draw(&mut self, frame: &mut Frame) {
        // Advance the spinner animation every frame; the event loop redraws
        // on a ~100 ms tick, so the spinner stays live while thinking.
        self.frame = self.frame.wrapping_add(1);
        // Every frame reflects the latest screen + session, so one call here
        // covers all transitions (Tab cycling, Enter/Esc, slash commands, ...).
        self.update_title();
        let body = self.draw_chrome(frame, frame.area());
        match self.screen {
            Screen::Sessions => self.draw_sessions(frame, body),
            Screen::Chat => self.draw_chat(frame, body),
            Screen::Agent => self.draw_agent(frame, body),
            Screen::Palette => self.draw_palette(frame, body),
            Screen::Commands => self.draw_commands(frame, body),
            Screen::AddModel => self.draw_add_model(frame, body),
        }
        // The quit dialog is an overlay drawn on top of the active screen.
        if self.show_quit {
            self.draw_quit_dialog(frame, body);
        }
    }

    /// Render the centered quit-confirmation overlay (opencode style).
    fn draw_quit_dialog(&self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(46);
        let h = 5.min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let dialog_rect = Rect::new(x, y, w, h);

        let dialog = Paragraph::new("ctrl+C / Enter / q — quit   ·   Esc — cancel")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" quit aether? "),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(dialog, dialog_rect);
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
                    Screen::AddModel => "chat",
                },
                model: &self.model,
                provider: &self.provider,
                version: env!("CARGO_PKG_VERSION"),
                session_count: self.sessions.len(),
            },
            area.width,
        );
        frame.render_widget(Paragraph::new(header), header_area);

        let selected = matches!(
            self.screen,
            Screen::Chat | Screen::Palette | Screen::AddModel
        );
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

        let inner = list_area.width.saturating_sub(2) as usize;
        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|s| {
                let title = s.title.as_deref().unwrap_or("(untitled)");
                let stats = format!(
                    "{} in / {} out · {} msgs",
                    chrome::format_tokens(s.stats.input_tokens),
                    chrome::format_tokens(s.stats.output_tokens),
                    s.messages
                );
                let title_max = inner.saturating_sub(stats.chars().count() + 2);
                let title = chrome::truncate(title, title_max.saturating_sub(1));
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
        let (left, left_style) = match &self.last_error {
            Some(err) => (
                format!(
                    "error: {}",
                    err.to_string().lines().next().unwrap_or_default()
                ),
                theme::danger(),
            ),
            None => (
                format!(
                    "{} sessions · {} turns · {} in / {} out",
                    totals.sessions, totals.turns, totals.input_tokens, totals.output_tokens
                ),
                theme::muted(),
            ),
        };
        let right =
            "j/k or wheel: move · Enter: open · d: delete · r: rename · ctrl+P: model · q: quit";
        self.render_footer_styled(frame, footer_area, &left, right, left_style);
        self.render_footer(frame, footer_area, &left, right);
    }

    fn draw_chat(&mut self, frame: &mut Frame, body: Rect) {
        self.drain_agent_events();
        // The usage/session sidebar is a full-height right rail (opencode
        // style): it spans the transcript + status + input + usage rows, so
        // those stay aligned with the transcript column. The footer is the
        // only full-width row. Skipped on narrow terminals so the transcript
        // keeps the full width.
        let rail = Rect::new(body.x, body.y, body.width, body.height.saturating_sub(1));
        let (left_col, sidebar_area) = if rail.width >= SIDEBAR_MIN_BODY_WIDTH {
            let horizontal =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(SIDEBAR_WIDTH)]);
            let [left, sidebar] = horizontal.areas(rail);
            (left, Some(sidebar))
        } else {
            (rail, None)
        };
        let vertical = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let [body_area, status_area, input_area, usage_area, footer_area] =
            vertical.areas(left_col);
        let footer_area = Rect::new(body.x, footer_area.y, body.width, 1);

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
            let spinner = spinner_frame(self.frame);
            rows.push(Line::from(vec![
                Span::styled("ai  ", theme::ai()),
                Span::styled(format!("{spinner} "), theme::accent()),
                Span::raw(tail),
            ]));
        }
        if rows.is_empty() {
            rows = welcome_rows(body_area.width, body_area.height);
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

        if total > viewport {
            let sb_area = Rect::new(
                body_area.right().saturating_sub(1),
                body_area.y,
                1,
                body_area.height,
            );
            let mut sb_state = ScrollbarState::new(total).position(end.saturating_sub(viewport));
            frame.render_stateful_widget(
                Scrollbar::default()
                    .orientation(ScrollbarOrientation::VerticalRight)
                    .style(theme::border()),
                sb_area,
                &mut sb_state,
            );
        }

        if let Some(sidebar_area) = sidebar_area {
            let lines = sidebar_lines(
                self.input_tokens,
                self.output_tokens,
                0, // no context-window source in the TUI — usage_line omits the pct
                self.selected_session(),
                &self.model,
                &self.provider,
            );
            let sidebar = Paragraph::new(
                lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| {
                        let style = if i == 1 {
                            Style::default().fg(Color::White)
                        } else {
                            theme::muted()
                        };
                        Line::from(Span::styled(l.as_str(), style))
                    })
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(theme::border())
                    .title(" usage "),
            );
            frame.render_widget(sidebar, sidebar_area);
        }

        let status = if let Some(err) = &self.last_error {
            Line::from(vec![
                Span::styled("error:", theme::danger()),
                Span::raw(format!(
                    " {}",
                    err.to_string().lines().next().unwrap_or_default()
                )),
            ])
        } else if self.agent_rx.is_some() {
            let mut line = agent_badge(self.agent_state.phase);
            line.spans.push(Span::raw("   "));
            line.spans
                .extend(status_line(self.processing, self.frame).spans);
            line
        } else {
            status_line(self.processing, self.frame)
        };
        frame.render_widget(Paragraph::new(status), status_area);

        let inner = input_area.width.saturating_sub(2) as usize;
        let shown = tail_with_ellipsis(&self.input, inner.saturating_sub(2));
        let input_widget = Paragraph::new(Line::from(vec![
            Span::styled("> ", theme::accent()),
            Span::raw(shown),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border())
                .title(" input "),
        );
        frame.render_widget(input_widget, input_area);

        let usage = render::usage_summary(Some(self.input_tokens), Some(self.output_tokens), None);
        let mut right = String::new();
        if !self.model.is_empty() {
            right.push_str("model: ");
            right.push_str(&self.model);
            right.push_str("   ");
        }
        right.push_str("Enter: send · j/k or wheel: scroll · ctrl+P: model · Esc: back · q: quit");
        self.render_footer(frame, footer_area, &usage, &right);
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
                lines.insert(0, Line::from(Span::styled("ai   ", theme::ai())));
                return lines;
            }
            return self.render_assistant_message(&row.content, width);
        }
        vec![Line::from(vec![
            Span::styled("sys  ", theme::muted()),
            Span::raw(row.content.as_str()),
        ])]
    }

    /// Render an assistant reply, highlighting fenced code blocks on a filled
    /// background with preserved indentation instead of mangling them by wrap.
    fn render_assistant_message<'a>(&self, content: &'a str, width: u16) -> Vec<Line<'a>> {
        let inner = width.saturating_sub(2) as usize;
        let mut lines: Vec<Line> = Vec::new();
        let mut rest = content;
        while let Some(open) = rest.find("```") {
            let before = &rest[..open];
            if !before.trim().is_empty() {
                for para in render::wrap(before.trim_end(), inner.saturating_sub(5).max(10)) {
                    lines.push(Line::from(vec![
                        Span::styled("ai   ", theme::ai()),
                        Span::raw(para),
                    ]));
                }
            }
            let after_open = &rest[open + 3..];
            let end = after_open.find("```").unwrap_or(after_open.len());
            let block = &after_open[..end];
            let (lang, code) = match block.find('\n') {
                Some(nl) => (&block[..nl], &block[nl + 1..]),
                None => (block, ""),
            };
            let code_style = Style::default().bg(theme::CODE_BG).fg(Color::White);
            lines.push(Line::from(Span::styled(
                format!("  {lang} "),
                theme::ai().bg(theme::CODE_BG),
            )));
            for code_line in code.lines() {
                let indent = code_line
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect::<String>();
                let content_part = &code_line[indent.len()..];
                for para in render::wrap(content_part, inner.saturating_sub(indent.len()).max(10)) {
                    let pad = inner.saturating_sub(indent.len() + para.chars().count());
                    let mut spans = vec![Span::styled(indent.clone(), code_style)];
                    spans.push(Span::styled(para, code_style));
                    if pad > 0 {
                        spans.push(Span::styled(" ".repeat(pad), code_style));
                    }
                    lines.push(Line::from(spans));
                }
            }
            rest = if end < after_open.len() {
                &after_open[end + 3..]
            } else {
                ""
            };
        }
        if !rest.trim().is_empty() {
            for para in render::wrap(rest.trim_end(), inner.saturating_sub(5).max(10)) {
                lines.push(Line::from(vec![
                    Span::styled("ai   ", theme::ai()),
                    Span::raw(para),
                ]));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled("ai   ", theme::ai())));
        }
        lines
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
        self.render_footer_styled(frame, area, left, right, theme::muted());
    }

    /// Two-part footer with a custom style for the left segment.
    fn render_footer_styled(
        &self,
        frame: &mut Frame,
        area: Rect,
        left: &str,
        right: &str,
        left_style: Style,
    ) {
        let left_len = render::visible_length(left) as u16 + 2;
        let horizontal = Layout::horizontal([Constraint::Length(left_len), Constraint::Min(0)]);
        let [left_area, right_area] = horizontal.areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(left, left_style))),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, theme::muted()))),
            right_area,
        );
    }

    fn draw_agent(&mut self, frame: &mut Frame, body: Rect) {
        self.drain_agent_events();
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]);
        let [panel_area, footer_area] = vertical.areas(body);
        let horizontal = Layout::horizontal([
            Constraint::Percentage(34),
            Constraint::Percentage(38),
            Constraint::Percentage(28),
        ]);
        let [plan_area, build_area, route_area] = horizontal.areas(panel_area);

        self.draw_agent_panel(frame, plan_area, "PLAN", theme::title());
        self.draw_agent_panel(frame, build_area, "BUILD", theme::accent());
        self.draw_agent_panel(frame, route_area, "ROUTE", theme::ai());

        let left = if self.agent_state.stopped {
            "agent finished".to_string()
        } else if self.agent_rx.is_some() {
            let cap = self.agent_cap;
            let iter = self.agent_state.current_iteration;
            if cap > 0 {
                format!("agent running — iteration {iter}/{cap} · plan → build → route")
            } else {
                "agent running — plan → build → route".to_string()
            }
        } else {
            "press Enter to run the agent loop".to_string()
        };
        let right = "Enter: run · Tab: switch · Esc: back";
        self.render_footer(frame, footer_area, &left, right);
    }

    /// Render one agent panel (PLAN / BUILD / ROUTE) as a bordered box with a
    /// colored header, progress meter for BUILD, and the state lines inside.
    fn draw_agent_panel(&self, frame: &mut Frame, area: Rect, title: &str, header_style: Style) {
        let state = &self.agent_state;
        let mut lines: Vec<Line> = Vec::new();

        if title == "PLAN" {
            if state.plan_text.is_empty() {
                lines.push(Line::from(Span::styled("  planning…", theme::muted())));
            } else {
                for plan_line in state.plan_text.lines().take(agent_screen::MAX_PLAN_LINES) {
                    lines.push(Line::from(Span::raw(format!("  {}", plan_line))));
                }
            }
        } else if title == "BUILD" {
            let cap = self.agent_cap;
            if cap > 0 {
                let pct = (state.current_iteration as f32 / cap as f32).clamp(0.0, 1.0);
                let width = 18usize;
                let filled = (pct * width as f32).round() as usize;
                let bar = format!(
                    "  [{}] {}%",
                    "█".repeat(filled) + &"░".repeat(width - filled),
                    (pct * 100.0).round() as u32
                );
                lines.push(Line::from(Span::styled(bar, theme::accent())));
            }
            lines.push(Line::from(Span::styled(
                format!("  iteration: {}", state.current_iteration),
                theme::muted(),
            )));
            lines.push(Line::from(Span::styled(
                format!("  tool calls: {}", state.total_tool_calls),
                theme::muted(),
            )));
            match &state.last_tool_name {
                Some(name) => lines.push(Line::from(Span::raw(format!("  last tool: {name}")))),
                None => lines.push(Line::from(Span::styled("  last tool: —", theme::muted()))),
            }
            if let Some(diff) = &state.last_diff {
                for diff_line in diff.lines().take(agent_screen::MAX_DIFF_LINES) {
                    lines.push(Line::from(diff_line_span(diff_line)));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                format!(
                    "  done {} · continue {} · revise {}",
                    state.verdict_counters.done,
                    state.verdict_counters.continue_,
                    state.verdict_counters.revise
                ),
                theme::muted(),
            )));
            match state.current_verdict {
                Some(v) => lines.push(Line::from(Span::styled(
                    format!("  current: {}", agent_screen::verdict_label(v)),
                    theme::accent(),
                ))),
                None => lines.push(Line::from(Span::styled(
                    "  current: waiting",
                    theme::muted(),
                ))),
            }
        }

        if let Some(reason) = state.stop_reason {
            lines.push(Line::from(Span::styled(
                format!("DONE: {}", agent_screen::stop_reason_label(reason)),
                theme::accent(),
            )));
        }

        // Live phase status in the panel header: the panel that owns the
        // current stage shows `· planning` / `· building` / `· routing` in
        // its role color, so the active stage is visible at a glance.
        let phase_label = match (title, state.phase) {
            (
                "PLAN",
                agent_screen::ScreenPhase::Planning | agent_screen::ScreenPhase::PlanReady,
            ) => Some("planning"),
            (
                "BUILD",
                agent_screen::ScreenPhase::BuildStarted
                | agent_screen::ScreenPhase::ToolCalled
                | agent_screen::ScreenPhase::BuildFinished,
            ) => Some("building"),
            ("ROUTE", agent_screen::ScreenPhase::Routing | agent_screen::ScreenPhase::Routed) => {
                Some("routing")
            }
            _ => None,
        };
        let title_line = match phase_label {
            Some(label) => Line::from(vec![
                Span::styled(format!(" {title} "), header_style),
                Span::styled(format!("· {label} "), header_style),
            ]),
            None => Line::from(Span::styled(format!(" {title} "), header_style)),
        };
        let panel = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(title_line),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(panel, area);
    }

    fn draw_palette(&mut self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(40);
        let h = (self.palette_models.len() as u16 + 3).min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let palette_rect = Rect::new(x, y, w, h);

        let inner = w.saturating_sub(2) as usize;
        let mut items: Vec<ListItem> = self
            .palette_models
            .iter()
            .map(|m| {
                let line = if m == &self.model {
                    let suffix = "  (active)";
                    let max = inner.saturating_sub(2 + suffix.chars().count());
                    Line::from(vec![
                        Span::styled(
                            chrome::truncate(m, max.saturating_sub(1)),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(suffix, theme::accent()),
                    ])
                } else {
                    let max = inner.saturating_sub(2);
                    Line::from(Span::styled(
                        chrome::truncate(m, max.saturating_sub(1)),
                        Style::default().fg(Color::White),
                    ))
                };
                ListItem::new(line)
            })
            .collect();
        items.push(ListItem::new(Line::from(Span::styled(
            "+ add custom model",
            theme::accent(),
        ))));
        let select = self.palette_index.min(items.len().saturating_sub(1));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" ctrl+P — model "),
            )
            .highlight_style(theme::highlight())
            .highlight_symbol("▸ ");
        self.palette_state.select(Some(select));
        frame.render_stateful_widget(list, palette_rect, &mut self.palette_state);
    }

    /// Render the add-model dialog: the current field prompt, the value
    /// typed so far, and a hint line (Enter advances, Esc cancels).
    fn draw_add_model(&self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(60);
        let h = 7.min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let rect = Rect::new(x, y, w, h);

        let (prompt, value) = match &self.add_model {
            Some(state) => match state.field {
                AddModelField::BaseUrl => ("provider base URL", state.base_url.as_str()),
                AddModelField::ApiKeyEnv => (
                    "API key env var name (Enter for none)",
                    state.api_key_env.as_str(),
                ),
                AddModelField::ModelName => ("model name", state.model_name.as_str()),
            },
            None => ("", ""),
        };
        let inner = w.saturating_sub(2) as usize;
        let prompt_prefix = format!("{prompt}: ");
        let value_max = inner.saturating_sub(prompt_prefix.chars().count());
        let value = chrome::truncate(value, value_max.saturating_sub(1));
        let lines = vec![
            Line::from(Span::styled("add custom model", theme::title())),
            Line::from(""),
            Line::from(vec![
                Span::styled(prompt_prefix, theme::accent()),
                Span::raw(value),
            ]),
            Line::from(""),
            Line::from(Span::styled("Enter: next · Esc: cancel", theme::muted())),
        ];
        let dialog = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(" /model — add custom "),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(dialog, rect);
    }

    fn draw_commands(&mut self, frame: &mut Frame, body: Rect) {
        let w = body.width.min(46);
        let h = (SlashCmd::ALL.len() as u16 + 2).min(body.height.saturating_sub(2));
        let x = body.x + body.width.saturating_sub(w) / 2;
        let y = body.y + body.height.saturating_sub(h) / 2;
        let cmd_rect = Rect::new(x, y, w, h);

        let inner = w.saturating_sub(2) as usize;
        let items: Vec<ListItem> = SlashCmd::ALL
            .iter()
            .map(|c| {
                let max = inner.saturating_sub(2);
                ListItem::new(Line::from(Span::styled(
                    chrome::truncate(c.label(), max.saturating_sub(1)),
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
        self.commands_state.select(Some(
            self.commands_index
                .min(SlashCmd::ALL.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(list, cmd_rect, &mut self.commands_state);
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
            "  quit        ctrl+C confirm · Esc cancel · q quit",
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
        // Best-effort: capture the pre-app title so it can be restored on
        // exit; a non-responding terminal must never block startup.
        let original_title = capture_original_title(TITLE_QUERY_TIMEOUT);
        if let Err(e) = crossterm::execute!(std::io::stdout(), EnableMouseCapture) {
            restore_title(original_title.as_deref());
            ratatui::restore();
            return Err(TuiError::Terminal(e.to_string()));
        }
        // Show the title immediately, before the first frame.
        self.update_title();
        let result = tokio::runtime::Runtime::new()
            .map_err(|e| TuiError::Terminal(e.to_string()))
            .and_then(|rt| rt.block_on(self.event_loop(&mut terminal)));
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
        restore_title(original_title.as_deref());
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

            if event::poll(Duration::from_millis(TICK_MS))
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

/// Persist the config so custom providers survive the next load.
///
/// `aether_core::config::save_config` serializes `base_url` as `baseUrl`,
/// but `load_config`'s legacy normalizer only accepts the TS shape
/// `baseURL`, so a Rust-written config would silently drop every custom
/// provider on the next read. Patch the key back to `baseURL` after saving
/// (a no-op when the key is already `baseURL`). Best-effort: a failed save
/// or patch leaves the previous config untouched.
fn persist_config(config: &AetherConfig) {
    let _ = save_config(config);
    let Ok(path) = aether_core::config::config_path() else {
        return;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let patched = raw.replace("\"baseUrl\":", "\"baseURL\":");
    if patched != raw {
        let _ = aether_core::fs::atomic_write(&path, patched.as_bytes());
    }
}

/// Derive a provider name from a base URL host (with port), so adding a
/// second model to the same endpoint extends the existing provider instead
/// of duplicating it. Falls back to `custom` when no host is present.
fn provider_name_from_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme.split('/').next().unwrap_or("custom").trim();
    if host.is_empty() {
        "custom".to_string()
    } else {
        host.to_string()
    }
}

/// Keep the most recent `max` chars of `s`, prefixing a `…` when anything
/// was cut — the chat input shows the tail of long text so the chars being
/// typed right now stay visible. Pure — unit-testable without a terminal.
fn tail_with_ellipsis(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let tail: String = s.chars().skip(count - keep).collect();
    format!("…{tail}")
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

/// Build the terminal tab/window title (opencode pattern: page first, then
/// the active session). Pure — no I/O — so it is unit-testable without a
/// terminal. Palette and Commands are chat overlays, so they share the chat
/// title (session name when one is active, plain `chat` otherwise).
fn title_for(screen: Screen, session_title: Option<&str>, model: &str) -> String {
    const MAX_SESSION_TITLE: usize = 40;
    let session = session_title
        .map(|t| truncate_title(t, MAX_SESSION_TITLE))
        .filter(|t| !t.is_empty());
    match screen {
        Screen::Sessions => "aether — sessions".to_string(),
        Screen::Agent => "aether — agent loop".to_string(),
        Screen::Chat | Screen::Palette | Screen::Commands | Screen::AddModel => match session {
            Some(title) => format!("aether — {title} | {model}"),
            None => "aether — chat".to_string(),
        },
    }
}

/// Truncate a session title to at most `max` chars, appending `…` when cut.
fn truncate_title(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        return title.to_string();
    }
    let mut out: String = title.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// How long to wait for the terminal's reply to the title query before
/// giving up and proceeding without a title to restore.
const TITLE_QUERY_TIMEOUT: Duration = Duration::from_millis(120);
/// Upper bound on the title reply we'll buffer (guards against a chatty
/// terminal streaming garbage back after the query).
const TITLE_REPLY_MAX_BYTES: usize = 4096;

/// Best-effort capture of the terminal's title before the app overwrites it,
/// so `run()` can restore the original on exit. Writes the XTerm title query
/// (`\x1B[21t`), flushes, reads the reply on a background thread and waits at
/// most `timeout` for it. A non-tty stdout, a terminal that doesn't answer,
/// or an unparseable reply all yield `None` without blocking the main path.
/// The reader thread may keep blocking on stdin afterwards, which is
/// acceptable for a one-off at startup.
fn capture_original_title(timeout: Duration) -> Option<String> {
    use std::io::{IsTerminal, Read, Write};

    if !std::io::stdout().is_terminal() {
        return None;
    }
    let mut stdout = std::io::stdout();
    if stdout.write_all(b"\x1B[21t").is_err() || stdout.flush().is_err() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while buf.len() < TITLE_REPLY_MAX_BYTES {
            match stdin.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    buf.push(byte[0]);
                    if parse_title_response(&buf).is_some() {
                        break;
                    }
                }
            }
        }
        let _ = tx.send(buf);
    });
    rx.recv_timeout(timeout)
        .ok()
        .and_then(|buf| parse_title_response(&buf))
}

/// Best-effort restore of the terminal title captured at startup. A no-op
/// when capture yielded `None`; errors are swallowed so a terminal that
/// ignores `SetTitle` never breaks exit.
fn restore_title(original: Option<&str>) {
    if let Some(title) = original {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(title));
    }
}

/// Locate `needle` in `haystack`, returning its byte offset.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Parse a terminal's reply to the XTerm title query (`\x1B[21t`) into the
/// window title string. XTerm answers with `\x1B]l<title>\x1B\\`; some
/// terminals use the OSC 0 (`\x1B]0;<title>\x07`, icon name) or OSC 2
/// (`\x1B]2;<title>\x07`, window title) forms instead. The reply may carry
/// stray bytes around it, so each variant is located anywhere in `raw`. An
/// empty title (or no recognizable reply) yields `None`.
fn parse_title_response(raw: &[u8]) -> Option<String> {
    const VARIANTS: &[(&[u8], &[u8])] = &[
        (b"\x1B]l", b"\x1B\\"), // XTerm title reply (ST terminator)
        (b"\x1B]0;", b"\x07"),  // OSC 0 icon name
        (b"\x1B]2;", b"\x07"),  // OSC 2 window title
    ];
    for (open, close) in VARIANTS {
        let Some(start) = find_bytes(raw, open) else {
            continue;
        };
        let payload = &raw[start + open.len()..];
        let Some(end) = find_bytes(payload, close) else {
            continue;
        };
        let title = String::from_utf8_lossy(&payload[..end]).trim().to_string();
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_session::SessionId;

    /// Point `AETHER_CONFIG_DIR` at a scratch dir so session/ledger reads
    /// never touch the developer's real store (serialized across the
    /// workspace via aether_core::testutil).
    fn isolate(tag: &str) -> std::path::PathBuf {
        let dir = aether_core::testutil::test_env(&format!("tui-{tag}"));
        let sessions = dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions);
        dir
    }

    /// Feed one plain character at a time through `on_key` (used to type
    /// into the add-model flow fields).
    fn type_text(app: &mut RatatuiTui, text: &str) {
        for c in text.chars() {
            let _ = app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    /// Concatenate every cell symbol of a drawn buffer (assert on rendered
    /// text without a terminal).
    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
        }
        text
    }

    /// Foreground color of the badge span whose content is exactly `word`.
    fn badge_fg(line: &Line<'static>, word: &str) -> Option<Color> {
        line.spans
            .iter()
            .find(|s| s.content.as_ref() == word)
            .and_then(|s| s.style.fg)
    }

    #[test]
    fn diff_line_span_colors_by_prefix() {
        let add = diff_line_span("+ new line");
        assert_eq!(add.style.fg, Some(theme::ACCENT));
        let remove = diff_line_span("- old line");
        assert_eq!(remove.style.fg, Some(theme::DANGER));
        let context = diff_line_span("  unchanged");
        assert_eq!(context.style.fg, Some(theme::MUTED));
    }

    #[test]
    fn palette_and_commands_render_highlight_symbol() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            palette_models: vec!["model-a".to_string(), "model-b".to_string()],
            palette_index: 1,
            ..Default::default()
        };

        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        for screen in [Screen::Palette, Screen::Commands] {
            app.screen = screen;
            terminal
                .draw(|frame| app.draw(frame))
                .expect("draw must not fail");
            let buffer = terminal.backend().buffer();
            let mut found = false;
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    if let Some(cell) = buffer.cell((x, y))
                        && cell.symbol() == "▸"
                    {
                        found = true;
                    }
                }
            }
            assert!(found, "{screen:?} must render the ▸ highlight symbol");
        }
    }

    #[test]
    fn error_renders_plain_text_not_symbol() {
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        for screen in [Screen::Sessions, Screen::Chat] {
            let mut app = RatatuiTui {
                screen,
                last_error: Some(TuiError::Provider("boom".into())),
                ..Default::default()
            };
            terminal
                .draw(|frame| app.draw(frame))
                .expect("draw must not fail");
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    if let Some(cell) = buffer.cell((x, y)) {
                        text.push_str(cell.symbol());
                    }
                }
            }
            assert!(
                text.contains("error:"),
                "{screen:?} must render 'error:' text, got: {text:?}"
            );
            assert!(
                !text.contains('✗'),
                "{screen:?} must not render the ✗ glyph, got: {text:?}"
            );
        }
    }

    #[test]
    fn status_indicators_are_plain_text() {
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        for (processing, expected) in [
            (ProcessingStatus::Thinking, "thinking"),
            (ProcessingStatus::Streaming(3), "streaming"),
        ] {
            let mut app = RatatuiTui {
                screen: Screen::Chat,
                processing,
                ..Default::default()
            };
            terminal
                .draw(|frame| app.draw(frame))
                .expect("draw must not fail");
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    if let Some(cell) = buffer.cell((x, y)) {
                        text.push_str(cell.symbol());
                    }
                }
            }
            assert!(
                text.contains(expected),
                "status must show '{expected}', got: {text:?}"
            );
            assert!(
                !text.contains('⏳') && !text.contains('●'),
                "status must not render symbol glyphs, got: {text:?}"
            );
        }
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
        let _g = aether_core::testutil::lock_env();
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
        let _g = aether_core::testutil::lock_env();
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
    fn ctrl_backspace_deletes_one_word_at_a_time() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: "foo bar ".into(),
            ..Default::default()
        };
        let ctrl_bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL);
        let _ = app.on_key(ctrl_bs);
        assert_eq!(app.input, "foo ");
        let _ = app.on_key(ctrl_bs);
        assert_eq!(app.input, "");
    }

    #[test]
    fn ctrl_backspace_on_empty_or_whitespace_input_is_safe() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input: "   ".into(),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
        assert_eq!(app.input, "");
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
        assert_eq!(app.input, "");
    }

    #[test]
    fn ctrl_l_clears_transcript_and_stays_on_screen() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "keep me? no".into(),
            }],
            chat_scroll: 3,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(app.screen, Screen::Chat);
        assert!(app.chat.is_empty());
        assert_eq!(app.chat_scroll, 0);
    }

    #[test]
    fn ctrl_l_clears_from_any_screen() {
        let mut app = RatatuiTui {
            screen: Screen::Sessions,
            chat: vec![ChatRow {
                role: "assistant".into(),
                content: "stale".into(),
            }],
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert_eq!(app.screen, Screen::Sessions);
        assert!(app.chat.is_empty());
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
    fn ctrl_c_toggles_quit_dialog_on_sessions() {
        let mut app = RatatuiTui {
            screen: Screen::Sessions,
            ..Default::default()
        };
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let action = app.on_key(ctrl_c);
        assert_eq!(action, KeyAction::Continue);
        assert!(app.show_quit);
        let action = app.on_key(ctrl_c);
        assert_eq!(action, KeyAction::Continue);
        assert!(!app.show_quit);
    }

    #[test]
    fn ctrl_c_never_quits_on_any_screen() {
        for screen in [Screen::Sessions, Screen::Chat, Screen::Agent] {
            let mut app = RatatuiTui {
                screen,
                ..Default::default()
            };
            let action = app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
            assert_eq!(action, KeyAction::Continue);
            assert!(app.show_quit);
        }
    }

    #[test]
    fn esc_on_sessions_does_not_quit() {
        let mut app = RatatuiTui {
            screen: Screen::Sessions,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert!(!app.show_quit);
    }

    #[test]
    fn sessions_q_still_quits() {
        let mut app = RatatuiTui {
            screen: Screen::Sessions,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Quit);
    }

    #[test]
    fn quit_dialog_esc_cancels() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            show_quit: true,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert!(!app.show_quit);
    }

    #[test]
    fn quit_dialog_enter_confirms() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            show_quit: true,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Quit);
    }

    #[test]
    fn quit_dialog_q_confirms() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            show_quit: true,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Quit);
    }

    #[test]
    fn quit_dialog_ignores_other_keys() {
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            show_quit: true,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert!(app.show_quit);
        assert_eq!(app.screen, Screen::Chat);
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

    #[test]
    fn usage_updates_counters_live_before_done() {
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
        let usage = aether_provider::Usage {
            prompt_tokens: Some(12),
            completion_tokens: Some(7),
            ..aether_provider::Usage::default()
        };
        tx.send(StreamEvent::Usage(usage)).unwrap();
        app.drain_stream();
        assert_eq!(app.input_tokens, 12);
        assert_eq!(app.output_tokens, 7);
        assert!(app.stream.is_some()); // not finished yet
    }

    #[test]
    fn assistant_message_renders_code_block_with_background() {
        let app = RatatuiTui::default();
        let lines = app.render_assistant_message(
            "explain:\n```rust\nfn main() {\n    hi();\n}\n```\ndone",
            40,
        );
        assert!(lines.len() >= 5);
        let code_style = Style::default().bg(theme::CODE_BG);
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.style.bg == Some(theme::CODE_BG)))
        );
        let has_lang = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("rust")));
        assert!(has_lang, "code block should show its language label");
        assert_eq!(code_style.bg, Some(theme::CODE_BG));
    }

    #[test]
    fn title_for_sessions_screen_shows_page() {
        let title = title_for(Screen::Sessions, None, "gpt-4o");
        assert!(title.contains("sessions"));
        assert!(title.starts_with("aether"));
    }

    #[test]
    fn title_for_chat_with_session_shows_session_and_model() {
        let title = title_for(Screen::Chat, Some("Fix the auth bug"), "gpt-4o");
        assert!(title.contains("Fix the auth bug"));
        assert!(title.contains("gpt-4o"));
        assert!(title.contains("aether"));
    }

    #[test]
    fn title_for_long_session_title_is_truncated() {
        let long = "this session title is way longer than forty characters for sure";
        assert!(long.chars().count() > 40);
        let title = title_for(Screen::Chat, Some(long), "gpt-4o");
        let session = title
            .split(" | ")
            .next()
            .unwrap()
            .strip_prefix("aether — ")
            .unwrap();
        assert!(session.chars().count() <= 40);
        assert!(session.ends_with('…'));
    }

    #[test]
    fn title_for_agent_screen_shows_agent_loop() {
        let title = title_for(Screen::Agent, Some("Fix the auth bug"), "gpt-4o");
        assert!(title.contains("agent"));
    }

    #[test]
    fn title_for_chat_without_session_shows_chat() {
        let title = title_for(Screen::Chat, None, "gpt-4o");
        assert_eq!(title, "aether — chat");
    }

    #[test]
    fn title_for_palette_and_commands_follow_chat() {
        for screen in [Screen::Palette, Screen::Commands] {
            let with_session = title_for(screen, Some("Fix the auth bug"), "gpt-4o");
            assert!(with_session.contains("Fix the auth bug"));
            let without = title_for(screen, None, "gpt-4o");
            assert_eq!(without, "aether — chat");
        }
    }

    #[test]
    fn title_for_truncation_boundary_keeps_short_titles() {
        let exact = "x".repeat(40);
        let title = title_for(Screen::Chat, Some(&exact), "gpt-4o");
        assert!(title.contains(&exact));
        let one_over = "x".repeat(41);
        let title = title_for(Screen::Chat, Some(&one_over), "gpt-4o");
        let session = title
            .split(" | ")
            .next()
            .unwrap()
            .strip_prefix("aether — ")
            .unwrap();
        assert_eq!(session.chars().count(), 40);
        assert!(session.ends_with('…'));
    }

    #[test]
    fn update_title_is_best_effort_and_never_panics() {
        let app = RatatuiTui::default();
        app.update_title(); // writes OSC to captured test stdout; must not panic
    }

    #[test]
    fn spinner_frame_wraps_and_animates() {
        assert_eq!(spinner_frame(0), SPINNER[0]);
        assert_eq!(spinner_frame(1), SPINNER[1]);
        assert_eq!(spinner_frame(SPINNER.len() as u32), SPINNER[0]); // wraps
        assert_eq!(spinner_frame(SPINNER.len() as u32 + 2), SPINNER[2]);
        assert_ne!(spinner_frame(3), spinner_frame(4)); // consecutive ticks differ
        assert!(SPINNER.contains(&spinner_frame(42)));
    }

    #[test]
    fn status_line_thinking_renders_spinner_and_label() {
        let line = status_line(ProcessingStatus::Thinking, 0);
        let text = line.to_string();
        assert!(
            text.contains(spinner_frame(0)),
            "thinking line must show the spinner, got {text:?}"
        );
        assert!(
            text.contains("thinking…"),
            "thinking line must show the label, got {text:?}"
        );
        let next = status_line(ProcessingStatus::Thinking, 1).to_string();
        assert_ne!(text, next, "spinner must animate across ticks");
    }

    #[test]
    fn status_line_streaming_renders_spinner_and_count() {
        let line = status_line(ProcessingStatus::Streaming(3), 2);
        let text = line.to_string();
        assert!(
            text.contains(spinner_frame(2)),
            "streaming line must show the spinner, got {text:?}"
        );
        assert!(
            text.contains("streaming · 3 chunks"),
            "streaming line must show the chunk count, got {text:?}"
        );
    }

    #[test]
    fn status_line_idle_is_ready() {
        assert_eq!(status_line(ProcessingStatus::Idle, 7).to_string(), "ready");
    }

    #[test]
    fn start_turn_sets_thinking_immediately() {
        let _g = aether_core::testutil::lock_env();
        isolate("think-now");
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            app.start_turn("hello".to_string(), "some-model")
                .expect("start_turn must succeed with the default config");
        });
        assert!(
            matches!(app.processing, ProcessingStatus::Thinking),
            "processing must be Thinking the moment start_turn returns, before any stream event"
        );
        assert!(
            app.stream.is_some(),
            "stream channel must be open while thinking"
        );
    }

    #[test]
    fn typing_appends_while_processing_and_enter_stays_gated() {
        for processing in [ProcessingStatus::Thinking, ProcessingStatus::Streaming(1)] {
            let (_tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
            let mut app = RatatuiTui {
                screen: Screen::Chat,
                processing,
                stream: Some(ChatStream {
                    rx,
                    buffer: String::new(),
                    usage: None,
                }),
                ..Default::default()
            };
            let _ = app.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
            let _ = app.on_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
            assert_eq!(app.input, "hi", "typing must append while {processing:?}");
            let _ = app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            assert_eq!(app.input, "h", "backspace must work while {processing:?}");
            let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(
                action,
                KeyAction::Continue,
                "Enter must stay gated while {processing:?}"
            );
            assert_eq!(app.input, "h", "gated Enter must not clear the input");
        }
    }

    #[test]
    fn draw_advances_animation_frame_per_tick() {
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let first = app.frame;
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(
            app.frame,
            first + 1,
            "each redraw tick must advance the spinner frame"
        );
        assert_ne!(
            spinner_frame(first),
            spinner_frame(first + 1),
            "consecutive ticks must show different spinner frames"
        );
    }
    #[test]
    fn palette_renders_custom_models_from_config() {
        use ratatui::backend::TestBackend;

        let _g = aether_core::testutil::lock_env();
        let dir = isolate("palette-custom");
        let mut config = aether_core::config::AetherConfig::default();
        config.default_provider = "myllm".to_string();
        config.providers.custom.push(CustomProviderConfig {
            name: "myllm".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: Some("MYLLM_KEY".into()),
            models: vec!["my-custom-model".into()],
            default_model: None,
        });
        config.providers.custom.push(CustomProviderConfig {
            name: "other".into(),
            base_url: "https://other.example/v1".into(),
            api_key_env: None,
            models: vec!["other-model".into()],
            default_model: None,
        });
        persist_config(&config);

        let mut app = RatatuiTui::default();
        app.open_palette();
        assert!(
            app.palette_models.iter().any(|m| m == "my-custom-model"),
            "palette state must include custom models from the config"
        );
        assert!(app.palette_models.iter().any(|m| m == "other-model"));

        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
        }
        assert!(
            text.contains("my-custom-model"),
            "rendered palette must show the custom model, got: {text:?}"
        );
        assert!(
            text.contains("+ add custom model"),
            "rendered palette must show the add-model entry, got: {text:?}"
        );
    }

    #[test]
    fn open_palette_includes_static_and_custom_models() {
        let _g = aether_core::testutil::lock_env();
        isolate("palette-mix");
        let mut config = aether_core::config::AetherConfig::default();
        config.providers.custom.push(CustomProviderConfig {
            name: "myllm".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: None,
            models: vec!["custom-x".into()],
            default_model: None,
        });
        persist_config(&config);

        let mut app = RatatuiTui::default();
        app.open_palette();
        assert!(
            app.palette_models
                .iter()
                .any(|m| m == "deepseek-v4-flash-free"),
            "static models of the default provider must stay listed"
        );
        assert_eq!(
            app.palette_models.last().map(String::as_str),
            Some("custom-x")
        );
        let unique: std::collections::HashSet<&String> = app.palette_models.iter().collect();
        assert_eq!(
            unique.len(),
            app.palette_models.len(),
            "no duplicate models"
        );
    }

    #[test]
    fn palette_selection_wraps_around() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            palette_models: vec!["m1".into(), "m2".into()],
            palette_index: 0,
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.palette_index, 1);
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.palette_index, 2); // the trailing add-model entry
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.palette_index, 0); // wraps to the top
        let _ = app.on_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert_eq!(app.palette_index, 2); // wraps to the bottom
        let _ = app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.palette_index, 0);
        let _ = app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.palette_index, 2);
    }

    #[test]
    fn palette_enter_on_add_entry_opens_add_flow_and_esc_cancels() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            palette_models: vec!["m1".into()],
            palette_index: 1, // the trailing add-model entry
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(app.screen, Screen::AddModel);
        assert_eq!(
            app.add_model.as_ref().map(|s| s.field),
            Some(AddModelField::BaseUrl)
        );

        let action = app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(app.screen, Screen::Palette);
        assert!(app.add_model.is_none(), "Esc must cancel the add flow");
    }

    #[test]
    fn add_model_flow_persists_config_and_appears_in_list() {
        let _g = aether_core::testutil::lock_env();
        let dir = isolate("add-model");
        let mut app = RatatuiTui {
            screen: Screen::AddModel,
            add_model: Some(AddModelState {
                field: AddModelField::BaseUrl,
                base_url: String::new(),
                api_key_env: String::new(),
                model_name: String::new(),
            }),
            ..Default::default()
        };
        type_text(&mut app, "http://localhost:9999/v1");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.add_model.as_ref().map(|s| s.field),
            Some(AddModelField::ApiKeyEnv)
        );
        type_text(&mut app, "MY_KEY");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.add_model.as_ref().map(|s| s.field),
            Some(AddModelField::ModelName)
        );
        type_text(&mut app, "my-model");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.screen, Screen::Palette);
        assert!(app.add_model.is_none());
        let pos = app
            .palette_models
            .iter()
            .position(|m| m == "my-model")
            .expect("the new model must appear in the palette list");
        assert_eq!(app.palette_index, pos, "cursor must land on the new model");

        let reloaded = aether_core::config::load_config_from(&dir).unwrap();
        let custom = reloaded
            .providers
            .custom
            .iter()
            .find(|p| p.name == "localhost:9999")
            .expect("config.json must contain the new custom provider");
        assert_eq!(custom.base_url, "http://localhost:9999/v1");
        assert_eq!(custom.api_key_env.as_deref(), Some("MY_KEY"));
        assert_eq!(custom.models, vec!["my-model".to_string()]);
    }

    #[test]
    fn add_model_flow_empty_key_env_stores_none() {
        let _g = aether_core::testutil::lock_env();
        let dir = isolate("add-model-nokey");
        let mut app = RatatuiTui {
            screen: Screen::AddModel,
            add_model: Some(AddModelState {
                field: AddModelField::BaseUrl,
                base_url: String::new(),
                api_key_env: String::new(),
                model_name: String::new(),
            }),
            ..Default::default()
        };
        type_text(&mut app, "http://localhost:9999/v1");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // skip the key env
        type_text(&mut app, "m");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let reloaded = aether_core::config::load_config_from(&dir).unwrap();
        let custom = reloaded
            .providers
            .custom
            .iter()
            .find(|p| p.name == "localhost:9999")
            .expect("config.json must contain the new custom provider");
        assert_eq!(
            custom.api_key_env, None,
            "an empty key env must be stored as None (no key sent)"
        );
    }

    #[test]
    fn add_model_flow_merges_into_existing_provider() {
        let _g = aether_core::testutil::lock_env();
        let dir = isolate("add-model-merge");
        let mut config = aether_core::config::AetherConfig::default();
        config.providers.custom.push(CustomProviderConfig {
            name: "localhost:9999".into(),
            base_url: "http://localhost:9999/v1".into(),
            api_key_env: Some("OLD_KEY".into()),
            models: vec!["existing-model".into()],
            default_model: None,
        });
        persist_config(&config);

        let mut app = RatatuiTui {
            screen: Screen::AddModel,
            add_model: Some(AddModelState {
                field: AddModelField::BaseUrl,
                base_url: String::new(),
                api_key_env: String::new(),
                model_name: String::new(),
            }),
            ..Default::default()
        };
        type_text(&mut app, "http://localhost:9999/v1");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        type_text(&mut app, "NEW_KEY");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        type_text(&mut app, "new-model");
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let reloaded = aether_core::config::load_config_from(&dir).unwrap();
        assert_eq!(
            reloaded.providers.custom.len(),
            1,
            "same endpoint must merge, not duplicate"
        );
        let custom = &reloaded.providers.custom[0];
        assert_eq!(
            custom.models,
            vec!["existing-model".to_string(), "new-model".to_string()]
        );
        assert_eq!(custom.api_key_env.as_deref(), Some("NEW_KEY"));
    }

    #[test]
    fn add_model_flow_refuses_empty_base_url() {
        let mut app = RatatuiTui {
            screen: Screen::AddModel,
            add_model: Some(AddModelState {
                field: AddModelField::BaseUrl,
                base_url: String::new(),
                api_key_env: String::new(),
                model_name: String::new(),
            }),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.add_model.as_ref().map(|s| s.field),
            Some(AddModelField::BaseUrl),
            "empty base URL must be refused"
        );
        assert_eq!(app.screen, Screen::AddModel);
    }

    #[test]
    fn add_model_flow_refuses_empty_model_name() {
        let mut app = RatatuiTui {
            screen: Screen::AddModel,
            add_model: Some(AddModelState {
                field: AddModelField::ModelName,
                base_url: "http://localhost:9999/v1".into(),
                api_key_env: String::new(),
                model_name: String::new(),
            }),
            ..Default::default()
        };
        let _ = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.add_model.as_ref().map(|s| s.field),
            Some(AddModelField::ModelName),
            "empty model name must be refused"
        );
        assert_eq!(app.screen, Screen::AddModel);
    }

    #[test]
    fn palette_select_custom_model_sends_turn_with_it() {
        let mut app = RatatuiTui {
            screen: Screen::Palette,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "why?".into(),
            }],
            palette_models: vec!["m1".into(), "my-custom-model".into()],
            palette_index: 1,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            action,
            KeyAction::SendTurn {
                question: "why?".into(),
                model: "my-custom-model".into()
            }
        );
        assert_eq!(app.screen, Screen::Chat);
        assert_eq!(app.model, "my-custom-model");
    }

    #[test]
    fn client_for_model_uses_custom_provider_client() {
        let _g = aether_core::testutil::lock_env();
        let dir = isolate("client-for-model");
        let mut config = aether_core::config::AetherConfig::default();
        config.providers.custom.push(CustomProviderConfig {
            name: "localhost:9999".into(),
            base_url: "http://localhost:9999/v1".into(),
            api_key_env: Some("MY_KEY".into()),
            models: vec!["my-model".into()],
            default_model: None,
        });
        persist_config(&config);

        let client = RatatuiTui::client_for_model("my-model").unwrap();
        let debug = format!("{client:?}");
        assert!(
            debug.contains("http://localhost:9999/v1"),
            "a custom model must be served by its own endpoint, got: {debug}"
        );
    }

    #[test]
    fn client_for_model_falls_back_to_default_provider() {
        let _g = aether_core::testutil::lock_env();
        isolate("client-fallback");
        let client = RatatuiTui::client_for_model("no-such-model").unwrap();
        let debug = format!("{client:?}");
        assert!(
            debug.contains("opencode.ai/zen/v1"),
            "an unknown model must fall back to the default provider, got: {debug}"
        );
    }

    #[test]
    fn provider_name_from_base_url_derives_host() {
        assert_eq!(
            provider_name_from_base_url("http://localhost:1234/v1"),
            "localhost:1234"
        );
        assert_eq!(
            provider_name_from_base_url("https://api.example.com/v1"),
            "api.example.com"
        );
        assert_eq!(provider_name_from_base_url("  https://x.io  "), "x.io");
        assert_eq!(provider_name_from_base_url(""), "custom");
        assert_eq!(provider_name_from_base_url("///"), "custom");
    }

    #[test]
    fn agent_badge_shows_all_stages_with_plan_highlighted() {
        let line = agent_badge(agent_screen::ScreenPhase::Planning);
        assert_eq!(line.to_string(), "plan -> build -> route");
        assert_eq!(badge_fg(&line, "plan"), Some(theme::ACCENT));
        assert_eq!(badge_fg(&line, "build"), Some(theme::MUTED));
        assert_eq!(badge_fg(&line, "route"), Some(theme::MUTED));
    }

    #[test]
    fn agent_badge_highlight_moves_through_stages() {
        let plan = agent_badge(agent_screen::ScreenPhase::PlanReady);
        assert_eq!(badge_fg(&plan, "plan"), Some(theme::ACCENT));
        let build = agent_badge(agent_screen::ScreenPhase::BuildStarted);
        assert_eq!(badge_fg(&build, "plan"), Some(theme::MUTED));
        assert_eq!(badge_fg(&build, "build"), Some(theme::ACCENT));
        for phase in [
            agent_screen::ScreenPhase::ToolCalled,
            agent_screen::ScreenPhase::BuildFinished,
        ] {
            let line = agent_badge(phase);
            assert_eq!(
                badge_fg(&line, "build"),
                Some(theme::ACCENT),
                "{phase:?} must highlight build"
            );
        }
        for phase in [
            agent_screen::ScreenPhase::Routing,
            agent_screen::ScreenPhase::Routed,
        ] {
            let line = agent_badge(phase);
            assert_eq!(
                badge_fg(&line, "route"),
                Some(theme::ACCENT),
                "{phase:?} must highlight route"
            );
        }
        assert_ne!(
            build.spans, plan.spans,
            "the highlight must move between stages"
        );
    }

    #[test]
    fn agent_badge_idle_and_finished_are_all_muted() {
        for phase in [
            agent_screen::ScreenPhase::Idle,
            agent_screen::ScreenPhase::Finished,
        ] {
            let line = agent_badge(phase);
            assert_eq!(line.to_string(), "plan -> build -> route");
            for word in ["plan", "build", "route"] {
                assert_eq!(
                    badge_fg(&line, word),
                    Some(theme::MUTED),
                    "{phase:?}: {word} must be muted"
                );
            }
        }
    }

    #[test]
    fn drain_agent_events_folds_phases_into_agent_state() {
        let (tx, rx) = std::sync::mpsc::channel::<aether_agent::AgentPhase>();
        let mut app = RatatuiTui {
            agent_rx: Some(rx),
            ..Default::default()
        };
        tx.send(aether_agent::AgentPhase::BuildStarted { iteration: 2 })
            .unwrap();
        app.drain_agent_events();
        assert_eq!(
            app.agent_state.phase,
            agent_screen::ScreenPhase::BuildStarted
        );
        assert_eq!(app.agent_state.current_iteration, 2);

        tx.send(aether_agent::AgentPhase::Routing { iteration: 2 })
            .unwrap();
        app.drain_agent_events();
        assert_eq!(app.agent_state.phase, agent_screen::ScreenPhase::Routing);
        assert_eq!(app.agent_state.current_iteration, 2);
    }

    #[test]
    fn draw_chat_shows_live_agent_badge_when_agent_runs() {
        use ratatui::backend::TestBackend;

        let (tx, rx) = std::sync::mpsc::channel::<aether_agent::AgentPhase>();
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            agent_rx: Some(rx),
            agent_state: agent_screen::AgentScreenState {
                phase: agent_screen::ScreenPhase::BuildStarted,
                ..Default::default()
            },
            ..Default::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("plan -> build -> route"),
            "chat status must show the badge while an agent runs, got: {text:?}"
        );
        assert!(
            text.contains("ready"),
            "the normal status text must follow the badge, got: {text:?}"
        );

        // A phase arriving on the channel updates the badge on the next draw.
        tx.send(aether_agent::AgentPhase::Routing { iteration: 1 })
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(
            app.agent_state.phase,
            agent_screen::ScreenPhase::Routing,
            "draw_chat must drain agent events so the badge stays live"
        );
    }

    #[test]
    fn draw_chat_without_agent_has_no_pattern_badge() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains("plan -> build -> route"),
            "no agent run must mean no pattern badge, got: {text:?}"
        );
        assert!(text.contains("ready"));
    }

    #[test]
    fn draw_agent_updates_phase_via_shared_drain() {
        use ratatui::backend::TestBackend;

        let (tx, rx) = std::sync::mpsc::channel::<aether_agent::AgentPhase>();
        let mut app = RatatuiTui {
            screen: Screen::Agent,
            agent_rx: Some(rx),
            ..Default::default()
        };
        tx.send(aether_agent::AgentPhase::BuildFinished {
            iteration: 1,
            tool_calls: 3,
            modified: false,
            summary: "round done".to_string(),
        })
        .unwrap();
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(
            app.agent_state.phase,
            agent_screen::ScreenPhase::BuildFinished,
            "the Agent screen must keep updating via the shared drain"
        );
        assert_eq!(app.agent_state.total_tool_calls, 3);
    }

    #[test]
    fn sidebar_lines_formats_tokens_and_session() {
        let session = aether_session::SessionSummary {
            id: SessionId::new("s1"),
            ts: "2026-08-15".into(),
            size: 0,
            messages: 3,
            title: Some("fix auth".into()),
            stats: aether_session::SessionStats {
                turns: 2,
                input_tokens: 1000,
                output_tokens: 2000,
            },
        };
        let lines = sidebar_lines(1234, 3400, 0, Some(&session), "gpt-4o-mini", "openai");
        assert_eq!(lines[0], "in 1.2K · out 3.4K · total 4.6K");
        assert_eq!(lines[1], "fix auth");
        assert_eq!(lines[2], "2 turns · 3 messages");
        assert_eq!(lines[3], "in 1K · out 2K");
        assert!(lines.iter().any(|l| l == "model: gpt-4o-mini"));
        assert!(lines.iter().any(|l| l == "provider: openai"));
        assert!(lines.iter().any(|l| l == "mcp: server crate (external)"));
    }

    #[test]
    fn sidebar_lines_falls_back_to_untitled_and_no_session() {
        let session = aether_session::SessionSummary {
            id: SessionId::new("s2"),
            ts: "2026-08-15".into(),
            size: 0,
            messages: 0,
            title: None,
            stats: aether_session::SessionStats::default(),
        };
        let lines = sidebar_lines(0, 0, 0, Some(&session), "m", "p");
        assert_eq!(lines[1], "(untitled)");
        let lines = sidebar_lines(0, 0, 0, None, "m", "p");
        assert_eq!(lines[1], "no session");
        assert!(!lines.iter().any(|l| l.contains("turns")));
    }

    #[test]
    fn sidebar_lines_context_percentage_follows_usage_line() {
        let with_ctx = sidebar_lines(12_000, 34_000, 100_000, None, "m", "p");
        assert!(with_ctx[0].starts_with("in 12K · out 34K · total 46K"));
        assert!(with_ctx[0].ends_with("% of ctx"));
        let without_ctx = sidebar_lines(12_000, 34_000, 0, None, "m", "p");
        assert_eq!(without_ctx[0], "in 12K · out 34K · total 46K");
        assert!(!without_ctx[0].contains("% of ctx"));
    }

    #[test]
    fn draw_chat_renders_sidebar_with_usage_and_session() {
        use ratatui::backend::TestBackend;

        let session = aether_session::SessionSummary {
            id: SessionId::new("s3"),
            ts: "2026-08-15".into(),
            size: 0,
            messages: 5,
            title: Some("my session".into()),
            stats: aether_session::SessionStats {
                turns: 4,
                input_tokens: 1000,
                output_tokens: 2000,
            },
        };
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut app = RatatuiTui {
            screen: Screen::Chat,
            sessions: vec![session],
            list_state,
            input_tokens: 1234,
            output_tokens: 3400,
            model: "gpt-4o-mini".into(),
            provider: "openai".into(),
            ..Default::default()
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("in 1.2K · out 3.4K · total 4.6K"));
        assert!(text.contains("my session"));
        assert!(text.contains("4 turns · 5 messages"));
        assert!(text.contains("model: gpt-4o-mini"));
        assert!(text.contains("provider: openai"));
        assert!(text.contains("mcp: server crate (external)"));
    }

    #[test]
    fn draw_chat_narrow_terminal_skips_sidebar() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            screen: Screen::Chat,
            input_tokens: 1234,
            output_tokens: 3400,
            model: "gpt-4o-mini".into(),
            ..Default::default()
        };
        let backend = TestBackend::new(40, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("transcript"));
        assert!(
            !text.contains("1.2K"),
            "sidebar must be skipped when the body is too narrow"
        );
    }

    #[test]
    fn draw_chat_sidebar_keeps_scrollbar_on_transcript_column() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            screen: Screen::Chat,
            chat: (0..50)
                .map(|i| ChatRow {
                    role: "assistant".into(),
                    content: format!("line {i}"),
                })
                .collect(),
            chat_scroll: 10,
            input_tokens: 1234,
            output_tokens: 3400,
            ..Default::default()
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer_text(buffer);
        assert!(
            text.contains('█'),
            "scrollbar thumb must render when the chat overflows"
        );
        // The sidebar column (rightmost) must never host the scrollbar.
        for y in 0..buffer.area.height {
            if let Some(cell) = buffer.cell((99, y)) {
                assert_ne!(
                    cell.symbol(),
                    "█",
                    "scrollbar must stay on the transcript column, not the sidebar"
                );
            }
        }
    }

    #[test]
    fn parse_title_response_xt_variant() {
        assert_eq!(
            parse_title_response(b"\x1B]lmy-shell\x1B\\"),
            Some("my-shell".to_string())
        );
    }

    #[test]
    fn parse_title_response_osc0_variant() {
        assert_eq!(
            parse_title_response(b"\x1B]0;my-shell\x07"),
            Some("my-shell".to_string())
        );
    }

    #[test]
    fn parse_title_response_osc2_variant() {
        assert_eq!(
            parse_title_response(b"\x1B]2;my-shell\x07"),
            Some("my-shell".to_string())
        );
    }

    #[test]
    fn parse_title_response_empty_is_none() {
        assert_eq!(parse_title_response(b"\x1B]l\x1B\\"), None);
        assert_eq!(parse_title_response(b"\x1B]0;\x07"), None);
        assert_eq!(parse_title_response(b"\x1B]2;\x07"), None);
    }

    #[test]
    fn parse_title_response_no_reply_is_none() {
        assert_eq!(parse_title_response(b""), None);
        assert_eq!(parse_title_response(b"\x1B[21t"), None);
        assert_eq!(parse_title_response(b"\x1B]0;"), None);
        assert_eq!(parse_title_response(b"not a title reply"), None);
    }

    #[test]
    fn parse_title_response_ignores_stray_bytes() {
        assert_eq!(
            parse_title_response(b"\x1B[?2026h\x1B]2;my-shell\x07\x1B[?2026l"),
            Some("my-shell".to_string())
        );
    }

    #[test]
    fn welcome_rows_centers_logo_and_hints() {
        let rows = welcome_rows(40, 20);
        let text: Vec<String> = rows.iter().map(|l| l.to_string()).collect();
        assert!(
            text.iter().any(|l| l.contains('▲')),
            "logo rows must render"
        );
        assert!(text.iter().any(|l| l.contains("aether")));
        assert!(text.iter().any(|l| l.contains("plan · build · route")));
        assert!(text.iter().any(|l| l.contains("type a message to start")));
        // Vertically centered: leading rows are blank padding.
        assert!(rows[0].to_string().trim().is_empty());
        // Horizontally centered: the first logo row has leading spaces.
        let logo_row = text.iter().find(|l| l.contains('▲')).unwrap();
        assert!(logo_row.starts_with(' '));
    }

    #[test]
    fn tail_with_ellipsis_keeps_short_input_unchanged() {
        assert_eq!(tail_with_ellipsis("hi", 5), "hi");
        assert_eq!(tail_with_ellipsis("", 5), "");
    }

    #[test]
    fn tail_with_ellipsis_shows_most_recent_chars() {
        assert_eq!(tail_with_ellipsis("hello world", 5), "…orld");
        assert_eq!(tail_with_ellipsis("hello world", 1), "…");
        assert_eq!(tail_with_ellipsis("hello world", 0), "…");
    }

    #[test]
    fn draw_chat_empty_shows_welcome_logo() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            screen: Screen::Chat,
            ..Default::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains('▲'),
            "an empty chat must show the welcome logo, got: {text:?}"
        );
        assert!(text.contains("type a message to start"));
    }

    #[test]
    fn draw_chat_with_messages_hides_welcome() {
        use ratatui::backend::TestBackend;

        let mut app = RatatuiTui {
            screen: Screen::Chat,
            chat: vec![ChatRow {
                role: "user".into(),
                content: "hi".into(),
            }],
            ..Default::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains('▲'),
            "a chat with messages must not show the welcome, got: {text:?}"
        );
    }
}
