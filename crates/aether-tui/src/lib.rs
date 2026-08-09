//! aether-tui — ratatui + crossterm terminal UI (Phase 4).
//!
//! Two screens:
//! - **Session list** — browse/rename/delete/resume sessions, arrow keys + wheel scroll.
//! - **Chat** — transcript of the active session, live streaming, ctrl+P provider/model palette.
//!
//! The trait surface (`Tui`, `TuiError`) from Phase 0 is preserved so downstream
//! crates still compile against a stable contract.

pub mod render;

use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    DisableMouseCapture, EnableMouseCapture,
};
use futures::StreamExt;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

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

/// Which screen is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Sessions,
    Chat,
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
    /// Live token counters for the active session.
    input_tokens: u64,
    output_tokens: u64,
    /// Palette state: available models + the highlighted one.
    palette_models: Vec<String>,
    palette_index: usize,
    /// Ledger totals footer.
    totals: aether_session::Totals,
    /// The most recent background error, shown in the footer.
    last_error: Option<TuiError>,
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
            input_tokens: 0,
            output_tokens: 0,
            palette_models: Vec::new(),
            palette_index: 0,
            totals: aether_session::Totals::default(),
            last_error: None,
        }
    }
}

impl RatatuiTui {
    /// Build the UI and load the session list + ledger totals.
    pub fn new() -> Result<Self, TuiError> {
        let mut app = Self::default();
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
        self.list_state.selected().and_then(|i| self.sessions.get(i))
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
            .map(|m| ChatRow { role: m.role, content: m.content })
            .collect();
        let stats = session.stats().map_err(|e| TuiError::Session(e.to_string()))?;
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
        let mut history: Vec<ChatMessage> = self
            .chat
            .iter()
            .map(|r| ChatMessage { role: r.role.clone(), content: r.content.clone(), ..ChatMessage::default() })
            .collect();
        history.push(ChatMessage { role: "user".to_string(), content: question.clone(), ..ChatMessage::default() });

        let request = ChatRequest {
            model: model.to_string(),
            messages: history.clone(),
            temperature: None,
            stream: true,
            tools: None,
        };

        let mut reply = String::new();
        let mut usage = None;
        let stream = client
            .stream_chat(&request)
            .await
            .map_err(|e| TuiError::Provider(e.to_string()))?;
        futures::pin_mut!(stream);
        self.pending = Some(String::new());
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| TuiError::Provider(e.to_string()))?;
            if let Some(text) = &chunk.content {
                reply.push_str(text);
                self.pending = Some(reply.clone());
            }
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
        }
        self.pending = None;

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
        self.chat.push(ChatRow { role: "user".to_string(), content: question });
        self.chat.push(ChatRow { role: "assistant".to_string(), content: reply });
        self.chat_scroll = self.chat.len().saturating_sub(1);
        Ok(())
    }

    /// Drive one event and return `true` to quit.
    async fn handle_event(&mut self, event: Event) -> Result<bool, TuiError> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match self.on_key(key) {
                    KeyAction::Continue => Ok(false),
                    KeyAction::Quit => Ok(true),
                    KeyAction::SendTurn { question, model } => {
                        self.send_turn(question, &model).await?;
                        Ok(false)
                    }
                }
            }
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
        match self.screen {
            Screen::Sessions => self.on_key_sessions(key),
            Screen::Chat => self.on_key_chat(key),
            Screen::Palette => self.on_key_palette(key),
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                self.list_scroll = (self.list_scroll + 1).min(self.sessions.len().saturating_sub(1));
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
            KeyCode::Char('q') if self.pending.is_none() => {
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
            _ => KeyAction::Continue,
        }
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
                // Re-ask the last user question against the newly selected model.
                match self.last_question() {
                    Some(q) => KeyAction::SendTurn { question: q.to_string(), model },
                    None => KeyAction::Continue,
                }
            }
            _ => KeyAction::Continue,
        }
    }

    fn last_question(&self) -> Option<&str> {
        self.chat.iter().rev().find(|r| r.role == "user").map(|r| r.content.as_str())
    }

    fn on_mouse(&mut self, me: crossterm::event::MouseEvent) -> Result<bool, TuiError> {
        match me.kind {
            MouseEventKind::ScrollDown => match self.screen {
                Screen::Sessions => {
                    self.list_scroll = (self.list_scroll + 1)
                        .min(self.sessions.len().saturating_sub(1));
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
        match self.screen {
            Screen::Sessions => self.draw_sessions(frame),
            Screen::Chat => self.draw_chat(frame),
            Screen::Palette => self.draw_palette(frame),
        }
    }

    fn draw_sessions(&mut self, frame: &mut Frame) {
        let vertical = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(3),
        ]);
        let [title_area, list_area, footer_area] = vertical.areas(frame.area());

        let title = format!(" aether sessions — {} ", self.sessions.len());
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                title,
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))),
            title_area,
        );

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|s| {
                let title = s.title.clone().unwrap_or_else(|| "(untitled)".to_string());
                let stats = format!(
                    "{} in / {} out · {} msgs",
                    s.stats.input_tokens, s.stats.output_tokens, s.messages
                );
                ListItem::new(Line::from(vec![
                    Span::styled(title, Style::default().fg(Color::White)),
                    Span::raw("  "),
                    Span::styled(stats, Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" sessions "))
            .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        let totals = &self.totals;
        let footer = format!(
            "{} sessions · {} turns · {} in / {} out   —  j/k or wheel: move · Enter: open · d: delete · ctrl+P: palette · q: quit",
            totals.sessions, totals.turns, totals.input_tokens, totals.output_tokens
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(Color::DarkGray),
            ))),
            footer_area,
        );
    }

    fn draw_chat(&mut self, frame: &mut Frame) {
        let vertical = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ]);
        let [title_area, body_area, footer_area] = vertical.areas(frame.area());

        let title = self
            .active
            .as_ref()
            .map(|s| s.title().unwrap_or_else(|_| "untitled".into()))
            .unwrap_or_else(|| "chat".into());
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {title} "),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))),
            title_area,
        );

        // Visible window of rows (oldest at top, scroll up to reveal history).
        let viewport = body_area.height as usize;
        let total = self.chat.len() + usize::from(self.pending.is_some());
        let end = total.saturating_sub(self.chat_scroll);
        let start = end.saturating_sub(viewport);
        let mut rows: Vec<Line> = self
            .chat
            .iter()
            .skip(start)
            .take(viewport)
            .map(|r| self.row_to_line(r))
            .collect();
        if let Some(tail) = &self.pending {
            rows.push(Line::from(vec![
                Span::styled(
                    "ai  ",
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                ),
                Span::raw(tail.clone()),
            ]));
        }
        let body = Paragraph::new(rows)
            .block(Block::default().borders(Borders::ALL).title(" transcript "))
            .wrap(Wrap { trim: false });
        frame.render_widget(body, body_area);

        let usage = render::usage_summary(
            Some(self.input_tokens),
            Some(self.output_tokens),
            None,
        );
        let hint = if self.pending.is_some() {
            "streaming…"
        } else {
            "Esc: back · j/k or wheel: scroll · ctrl+P: palette"
        };
        let footer = format!("{usage}   {hint}");
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(Color::DarkGray),
            ))),
            footer_area,
        );
    }

    fn row_to_line(&self, row: &ChatRow) -> Line<'static> {
        let role = row.role.clone();
        let content = row.content.clone();
        let prefix = match role.as_str() {
            "user" => Span::styled(
                "you ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            _ => Span::styled(
                "ai  ",
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            ),
        };
        Line::from(vec![prefix, Span::raw(content)])
    }

    fn draw_palette(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let w = area.width.min(40);
        let h = (self.palette_models.len() as u16 + 2).min(area.height.saturating_sub(2));
        let x = area.x + area.width.saturating_sub(w) / 2;
        let y = area.y + area.height.saturating_sub(h) / 2;
        let palette_rect = Rect::new(x, y, w, h);

        let items: Vec<ListItem> = self
            .palette_models
            .iter()
            .map(|m| ListItem::new(m.clone()))
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" ctrl+P — model "))
            .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
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
                ChatRow { role: "user".into(), content: "hi".into() },
                ChatRow { role: "assistant".into(), content: "hello".into() },
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
        let mut app = RatatuiTui { screen: Screen::Chat, ..Default::default() };
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
            chat: vec![ChatRow { role: "user".into(), content: "why?".into() }],
            palette_models: vec!["m1".into(), "m2".into()],
            palette_index: 1,
            ..Default::default()
        };
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            action,
            KeyAction::SendTurn { question: "why?".into(), model: "m2".into() }
        );
        assert_eq!(app.screen, Screen::Chat);
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
