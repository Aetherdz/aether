//! aether-session — JSONL sessions with auto-title, recall, and ledger.
//!
//! Faithful port of `aether-cli/src/session.ts` (220 LOC) so both versions
//! coexist on the same `~/.config/aether/sessions` directory:
//! same id scheme, same line shapes, same title sidecars.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use aether_core::config::sessions_dir;
use aether_core::error::{Error, Result};
use aether_core::fs::{atomic_write, ensure_dir, safe_join};
use serde::{Deserialize, Serialize};
use time::macros::format_description;

/// ISO-8601 timestamp with milliseconds (matches `new Date().toISOString()`).
const ISO_MS: &[time::format_description::FormatItem] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

/// Session id format: ISO-8601 with `:` and `.` replaced by `-`
/// (matches TS `date.toISOString().replace(/[:.]/g, "-")`).
const ID_FMT: &[time::format_description::FormatItem] = format_description!(
    "[year]-[month]-[day]T[hour]-[minute]-[second]-[subsecond digits:3]Z"
);

/// A filesystem-safe session id (also the sort key: descending = newest first).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Wrap a raw id string (validated for filesystem safety on use).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The raw id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One conversation line in a session file (role/content/ts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub ts: String,
}

/// A per-turn usage line (role `meta`, kind `usage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Aggregated stats for one session (from its meta lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionStats {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Summary row for `sessions list`.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: SessionId,
    pub ts: String,
    pub size: u64,
    pub messages: usize,
    pub title: Option<String>,
    pub stats: SessionStats,
}

/// A recall hit: the best matching line in one session.
#[derive(Debug, Clone)]
pub struct RecallHit {
    pub id: SessionId,
    pub title: Option<String>,
    pub ts: String,
    pub role: String,
    pub snippet: String,
    pub matches: usize,
}

/// Totals across all sessions (ledger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub sessions: usize,
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A raw JSONL line, dispatched by `role`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Message(SessionMessage),
    Meta(SessionMetaWithRole),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetaWithRole {
    role: String,
    kind: String,
    #[serde(flatten)]
    meta: SessionMeta,
}

/// A session handle bound to one id.
#[derive(Debug, Clone)]
pub struct Session {
    id: SessionId,
}

/// Full-session search across all session files.
pub struct Recall;

/// Cross-session usage ledger.
pub struct Ledger;

impl Session {
    /// Create a new session with a fresh timestamp id. The file is created
    /// lazily on the first append (exactly like TS: `appendToSession`).
    pub fn create() -> Result<SessionId> {
        let dir = sessions_dir()?;
        ensure_dir(&dir)?;
        let id = SessionId(
            time::OffsetDateTime::now_utc()
                .format(&ID_FMT)
                .map_err(|e| Error::InvalidInput(e.to_string()))?,
        );
        Ok(id)
    }

    /// Open an existing session by id (validates the id is filesystem-safe).
    pub fn open(id: &SessionId) -> Result<Session> {
        let _ = session_file(id)?;
        Ok(Session { id: id.clone() })
    }

    /// The bound id.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Append one message line (atomic, fsynced). Content is required;
    /// use [`Session::append_usage`] for ledger lines.
    pub fn append(&self, role: &str, content: &str) -> Result<()> {
        let path = session_file(&self.id)?;
        let line = SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            ts: now_iso()?,
        };
        append_line(&path, &serde_json::to_string(&line)?)
    }

    /// Append a usage ledger line (role `meta`).
    pub fn append_usage(&self, meta: SessionMeta) -> Result<()> {
        let path = session_file(&self.id)?;
        let line = SessionMetaWithRole {
            role: "meta".to_string(),
            kind: "usage".to_string(),
            meta,
        };
        append_line(&path, &serde_json::to_string(&line)?)
    }

    /// Title of the session: reads the `<id>.title` sidecar, auto-generating
    /// (and persisting) it from the first user message when missing.
    pub fn title(&self) -> Result<String> {
        if let Some(t) = read_title(&self.id)? {
            return Ok(t);
        }
        let title = auto_title(&self.id)?;
        write_title(&self.id, &title)?;
        Ok(title)
    }

    /// Read all lines of the session (messages + meta), in file order.
    pub fn read(&self) -> Result<Vec<Line>> {
        read_lines(&self.id)
    }

    /// Read only the conversation messages (role user/assistant).
    pub fn read_messages(&self) -> Result<Vec<SessionMessage>> {
        Ok(read_lines(&self.id)?
            .into_iter()
            .filter_map(|line| match line {
                Line::Message(m) => Some(m),
                Line::Meta(_) => None,
            })
            .collect())
    }

    /// Aggregated usage stats for this session.
    pub fn stats(&self) -> Result<SessionStats> {
        let mut stats = SessionStats::default();
        for line in self.read()? {
            if let Line::Meta(m) = line {
                stats.turns += m.meta.turns;
                stats.input_tokens += m.meta.input_tokens;
                stats.output_tokens += m.meta.output_tokens;
            }
        }
        Ok(stats)
    }

    /// Delete the session file and its title sidecar.
    pub fn delete(&self) -> Result<bool> {
        let dir = sessions_dir()?;
        let file = dir.join(format!("{}.jsonl", self.id.0));
        let title = dir.join(format!("{}.title", self.id.0));
        if !file.exists() {
            return Ok(false);
        }
        ensure_regular(&file)?;
        ensure_regular(&title)?;
        fs::remove_file(&file)?;
        if title.exists() {
            fs::remove_file(&title)?;
        }
        Ok(true)
    }

    /// Set (or replace) the session title sidecar.
    pub fn rename(&self, title: &str) -> Result<()> {
        write_title(&self.id, title)
    }

    /// Raw JSONL lines of the session file (for sync bundles).
    pub fn raw_lines(&self) -> Result<Vec<String>> {
        let path = session_file(&self.id)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        ensure_regular(&path)?;
        let raw = fs::read_to_string(&path)?;
        Ok(raw.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
    }

    /// Replace the session file with raw JSONL lines (used by sync pull).
    pub fn write_raw_lines(&self, lines: &[String]) -> Result<()> {
        let path = session_file(&self.id)?;
        ensure_regular(&path)?;
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        atomic_write(&path, content.as_bytes())
    }
}

impl Recall {
    /// Case-insensitive keyword search over all sessions. Returns the best
    /// matching line per session, sorted by match count, bounded by `limit`.
    pub fn search(query: &str, limit: usize) -> Result<Vec<RecallHit>> {
        let q = query.to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for s in list_sessions()? {
            let mut best: Option<RecallHit> = None;
            for m in read_lines(&s.id)? {
                let Line::Message(msg) = m else { continue };
                if msg.role != "user" && msg.role != "assistant" {
                    continue;
                }
                let hay = msg.content.to_lowercase();
                let Some(idx) = hay.find(&q) else { continue };
                let matches = hay.matches(&q).count();
                let start = idx.saturating_sub(40);
                let end = (idx + q.len() + 60).min(msg.content.len());
                let mut snippet = String::new();
                if start > 0 {
                    snippet.push('…');
                }
                snippet.push_str(&msg.content[start..end]);
                if end < msg.content.len() {
                    snippet.push('…');
                }
                let candidate = RecallHit {
                    id: s.id.clone(),
                    title: s.title.clone(),
                    ts: s.ts.clone(),
                    role: msg.role.clone(),
                    snippet,
                    matches,
                };
                if best.as_ref().is_none_or(|b| candidate.matches > b.matches) {
                    best = Some(candidate);
                }
            }
            if let Some(hit) = best {
                hits.push(hit);
            }
        }
        hits.sort_by_key(|h| std::cmp::Reverse(h.matches));
        hits.truncate(limit);
        Ok(hits)
    }
}

impl Ledger {
    /// Totals across all sessions.
    pub fn totals() -> Result<Totals> {
        let sessions = list_sessions()?;
        let mut totals = Totals {
            sessions: sessions.len(),
            ..Totals::default()
        };
        for s in sessions {
            totals.turns += s.stats.turns;
            totals.input_tokens += s.stats.input_tokens;
            totals.output_tokens += s.stats.output_tokens;
        }
        Ok(totals)
    }
}

/// List all sessions, newest first (id descending, as TS does).
pub fn list_sessions() -> Result<Vec<SessionSummary>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".jsonl") else { continue };
        let id = SessionId(id.to_string());
        let st = entry.metadata()?;
        let lines = read_lines(&id)?;
        let mut stats = SessionStats::default();
        let mut messages = 0usize;
        for line in &lines {
            match line {
                Line::Message(_) => messages += 1,
                Line::Meta(m) => {
                    stats.turns += m.meta.turns;
                    stats.input_tokens += m.meta.input_tokens;
                    stats.output_tokens += m.meta.output_tokens;
                }
            }
        }
        out.push(SessionSummary {
            ts: id.0.replace('-', ":"),
            id: id.clone(),
            size: st.len(),
            messages,
            title: read_title(&id)?,
            stats,
        });
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// The session file path for an id (validates filesystem safety).
fn session_file(id: &SessionId) -> Result<PathBuf> {
    let dir = sessions_dir()?;
    ensure_dir(&dir)?;
    let file_name = format!("{}.jsonl", id.0);
    safe_join(&dir, &file_name)
}

/// The `<id>.title` sidecar path (validates filesystem safety).
fn title_path(id: &SessionId) -> Result<PathBuf> {
    let dir = sessions_dir()?;
    let file_name = format!("{}.title", id.0);
    safe_join(&dir, &file_name)
}

/// Atomic, fsynced append of one JSON line.
///
/// Rejects symlinked session files before writing so a malicious symlink
/// cannot redirect the append outside the sessions dir.
fn append_line(path: &PathBuf, line: &str) -> Result<()> {
    ensure_regular(path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Reject symlinks: a session file must be a regular file.
///
/// Missing files are allowed (they are created on first append).
fn ensure_regular(path: &std::path::Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(Error::InvalidInput(format!(
                    "refusing to touch symlink: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Read and parse every line of a session file (skips malformed lines).
fn read_lines(id: &SessionId) -> Result<Vec<Line>> {
    let path = session_file(id)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    ensure_regular(&path)?;
    let raw = fs::read_to_string(&path)?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<Line>(line) {
            out.push(parsed);
        }
    }
    Ok(out)
}

/// Read the title sidecar, if present and non-empty.
fn read_title(id: &SessionId) -> Result<Option<String>> {
    let path = title_path(id)?;
    if !path.exists() {
        return Ok(None);
    }
    ensure_regular(&path)?;
    let t = fs::read_to_string(&path)?.trim().to_string();
    Ok((!t.is_empty()).then_some(t))
}

/// Write the title sidecar (atomic).
fn write_title(id: &SessionId, title: &str) -> Result<()> {
    let path = title_path(id)?;
    ensure_regular(&path)?;
    atomic_write(&path, format!("{title}\n").as_bytes())
}

/// Auto-title from the first user message: trimmed, whitespace-collapsed,
/// capped at 60 chars (exactly the TS heuristic). Falls back to the id.
fn auto_title(id: &SessionId) -> Result<String> {
    for m in read_lines(id)? {
        let Line::Message(msg) = m else { continue };
        if msg.role == "user" {
            let collapsed: String = msg.content.split_whitespace().collect::<Vec<_>>().join(" ");
            let t = collapsed.trim();
            if !t.is_empty() {
                let end = t.chars().count().min(60);
                return Ok(t.chars().take(end).collect());
            }
        }
    }
    Ok(id.0.clone())
}

/// Current UTC time as an ISO-8601 string with milliseconds.
fn now_iso() -> Result<String> {
    time::OffsetDateTime::now_utc()
        .format(&ISO_MS)
        .map_err(|e| Error::InvalidInput(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::config::config_dir;
    use std::sync::Mutex;

    /// Serialize env-mutating tests (AETHER_CONFIG_DIR is process-global).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_env(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-session-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // SAFETY: tests are serialized by ENV_LOCK, so no other thread reads
        // AETHER_CONFIG_DIR while it is being mutated.
        unsafe { std::env::set_var("AETHER_CONFIG_DIR", &dir) };
        let _ = fs::create_dir_all(dir.join("sessions"));
        dir
    }

    #[test]
    fn create_append_read_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = test_env("roundtrip");
        let id = Session::create().unwrap();
        let s = Session::open(&id).unwrap();
        s.append("user", "hello world").unwrap();
        s.append("assistant", "hi there").unwrap();
        let msgs = s.read_messages().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello world");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "hi there");
        assert!(!msgs[0].ts.is_empty());
        // Id has the TS shape: ISO with : and . replaced by -.
        assert_eq!(id.as_str().chars().filter(|c| *c == '-').count(), 5);
        assert!(!id.as_str().contains(':'));
        let _ = fs::remove_dir_all(&dir);
        let _ = config_dir();
    }

    #[test]
    fn auto_title_generated_once_and_stable() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = test_env("title");
        let id = Session::create().unwrap();
        let s = Session::open(&id).unwrap();
        s.append("user", "fix the login bug in auth.rs please").unwrap();
        let t1 = s.title().unwrap();
        let t2 = s.title().unwrap();
        assert_eq!(t1, t2);
        assert_eq!(t1, "fix the login bug in auth.rs please");
        assert!(dir.join("sessions").join(format!("{}.title", id.as_str())).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recall_finds_keyword_in_one_session_not_another() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = test_env("recall");
        let a = Session::open(&Session::create().unwrap()).unwrap();
        a.append("user", "we use postgres with an ORM called diesel").unwrap();
        let b = Session::open(&Session::create().unwrap()).unwrap();
        b.append("user", "the frontend is plain react with no backend").unwrap();
        let hits = Recall::search("diesel", 8).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, *a.id());
        assert!(hits[0].snippet.contains("diesel"));
        // Case-insensitive.
        let hits = Recall::search("DIESEL", 8).unwrap();
        assert_eq!(hits.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ledger_totals_across_sessions() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = test_env("ledger");
        let a = Session::open(&Session::create().unwrap()).unwrap();
        a.append_usage(SessionMeta { turns: 1, input_tokens: 100, output_tokens: 20 }).unwrap();
        a.append_usage(SessionMeta { turns: 1, input_tokens: 50, output_tokens: 30 }).unwrap();
        let b = Session::open(&Session::create().unwrap()).unwrap();
        b.append_usage(SessionMeta { turns: 2, input_tokens: 300, output_tokens: 40 }).unwrap();
        let totals = Ledger::totals().unwrap();
        assert_eq!(totals.sessions, 2);
        assert_eq!(totals.turns, 4);
        assert_eq!(totals.input_tokens, 450);
        assert_eq!(totals.output_tokens, 90);
        let _ = fs::remove_dir_all(&dir);
    }
}
