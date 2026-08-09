//! aetherdz-sync — cross-device session sync (faithful port of `sync.ts`).
//!
//! Backends:
//! - **folder**: a directory containing `aether-sessions.json` (bundles).
//! - **gist**: a GitHub gist with a single file `aether-sessions.json`.
//!
//! Merge semantics (identical to the TS original):
//! - `merge_lines`: concatenate, sort by `ts`, drop exact duplicates.
//! - `merge_bundles`: union of session ids, title `local ?? remote`, lines merged.
//! - `write_sessions_from_bundle`: existing session files are line-merged
//!   (never overwritten); new ids are written fresh. Returns written/merged counts.

use std::collections::HashMap;
use std::path::PathBuf;

use aetherdz_core::config::config_dir;
use aetherdz_core::error::{Error, Result};
use aetherdz_core::fs::{atomic_write, ensure_dir};
use aetherdz_session::{list_sessions, Session, SessionId};
use serde::{Deserialize, Serialize};

/// Bundle file name inside a gist or folder (must match `sync.ts`).
pub const GIST_FILENAME: &str = "aether-sessions.json";
/// Bundle format version (matches `sync.ts` `BUNDLE_VERSION`).
pub const BUNDLE_VERSION: u64 = 1;
/// GitHub API base URL (matches `sync.ts`).
const GIST_API: &str = "https://api.github.com";

/// Persisted sync state (`~/.config/aether/sync.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default)]
    pub device_id: String,
}

/// One session inside a bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEntry {
    pub title: Option<String>,
    pub lines: Vec<String>,
}

/// A full sync bundle (v1): all sessions of one device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionBundle {
    pub v: u64,
    pub device_id: String,
    pub sessions: HashMap<String, SessionEntry>,
}

/// A resolved backend.
#[derive(Debug, Clone)]
pub enum Backend {
    Gist { id: String },
    Folder { path: PathBuf },
}

/// Pull result: how many sessions were written fresh vs line-merged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub written: usize,
    pub merged: usize,
}

/// Line-level merge: concatenate, sort by `ts`, drop exact duplicates.
pub fn merge_lines(a: &[String], b: &[String]) -> Vec<String> {
    let mut combined: Vec<&String> = a.iter().chain(b.iter()).collect();
    combined.sort_by_key(|l| line_timestamp(l));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(combined.len());
    for line in combined {
        if seen.insert(line.as_str()) {
            out.push(line.clone());
        }
    }
    out
}

/// Bundle merge: union of ids, title `local ?? remote`, lines merged.
pub fn merge_bundles(local: &SessionBundle, remote: &SessionBundle) -> SessionBundle {
    let mut ids: Vec<&String> = local.sessions.keys().collect();
    for id in remote.sessions.keys() {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.sort();
    let mut sessions = HashMap::with_capacity(ids.len());
    for id in ids {
        let entry = match (local.sessions.get(id), remote.sessions.get(id)) {
            (Some(l), Some(r)) => SessionEntry {
                title: l.title.clone().or_else(|| r.title.clone()),
                lines: merge_lines(&l.lines, &r.lines),
            },
            (Some(l), None) => l.clone(),
            (None, Some(r)) => r.clone(),
            (None, None) => continue,
        };
        sessions.insert(id.clone(), entry);
    }
    SessionBundle {
        v: BUNDLE_VERSION,
        device_id: local.device_id.clone(),
        sessions,
    }
}

/// Push local sessions into a backend bundle; pull and merge it back.
pub async fn push(device_id: &str, backend: &Backend) -> Result<SyncReport> {
    let local = local_bundle(device_id)?;
    let report = SyncReport {
        written: local.sessions.len(),
        merged: 0,
    };
    match fetch_bundle(backend).await? {
        Some(remote) => write_bundle(backend, &merge_bundles(&local, &remote)).await?,
        None => write_bundle(backend, &local).await?,
    }
    Ok(report)
}

/// Fetch the backend bundle and merge its sessions into the local store.
pub async fn pull(backend: &Backend) -> Result<SyncReport> {
    match fetch_bundle(backend).await? {
        Some(bundle) => write_sessions_from_bundle(&bundle),
        None => Ok(SyncReport::default()),
    }
}

/// Load sync state from `~/.config/aether/sync.json` (defaults when absent/corrupt).
pub fn load_state() -> Result<SyncState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(SyncState::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    match serde_json::from_str(&raw) {
        Ok(s) => Ok(s),
        Err(_) => Ok(SyncState::default()),
    }
}

/// Persist sync state atomically.
pub fn save_state(state: &SyncState) -> Result<()> {
    let path = state_path()?;
    if let Some(dir) = path.parent() {
        ensure_dir(dir)?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    atomic_write(&path, raw.as_bytes())
}
/// Read the GitHub token from env or `~/.config/aether/github-token`.
pub fn github_token() -> Result<Option<String>> {
    match std::env::var("AETHER_GITHUB_TOKEN") {
        Ok(t) if !t.trim().is_empty() => return Ok(Some(t)),
        _ => {}
    }
    let file = config_dir()?.join("github-token");
    if file.exists() {
        let t = std::fs::read_to_string(&file)?;
        let t = t.trim();
        if !t.is_empty() {
            return Ok(Some(t.to_string()));
        }
    }
    Ok(None)
}

/// Device id in the same shape as the TS original:
/// `device-<Date.now().toString(36)>-<random36>`.
pub fn generate_device_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Cheap uniqueness without pulling in a rng dep: hash the nanosecond
    // counter plus the process id into a base-36 tail.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tail = (nanos ^ u128::from(std::process::id())) & 0xFFFF_FFFF;
    format!("device-{ms:x}-{tail:x}")
}

/// Build a bundle from every local session.
pub fn local_bundle(device_id: &str) -> Result<SessionBundle> {
    let mut sessions = HashMap::new();
    for summary in list_sessions()? {
        let session = Session::open(&summary.id)?;
        sessions.insert(
            summary.id.as_str().to_string(),
            SessionEntry {
                title: summary.title.clone(),
                lines: session.raw_lines()?,
            },
        );
    }
    Ok(SessionBundle {
        v: BUNDLE_VERSION,
        device_id: device_id.to_string(),
        sessions,
    })
}

/// Write every bundle session into the local store. Existing files are
/// line-merged (never overwritten); missing ids are created fresh.
/// Returns (written, merged).
pub fn write_sessions_from_bundle(bundle: &SessionBundle) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    for (id, entry) in &bundle.sessions {
        let sid = SessionId::new(id.clone());
        let session = Session::open(&sid)?;
        let existing = session.raw_lines()?;
        let (final_lines, mode) = if existing.is_empty() {
            (entry.lines.clone(), 0usize)
        } else {
            let merged = merge_lines(&existing, &entry.lines);
            if merged.len() == existing.len() && merged == existing {
                (merged, 2usize) // no change
            } else {
                (merged, 1usize)
            }
        };
        match mode {
            0 => report.written += 1,
            1 => report.merged += 1,
            _ => {}
        }
        session.write_raw_lines(&final_lines)?;
        if let Some(title) = &entry.title {
            session.rename(title)?;
        }
    }
    Ok(report)
}

/// Fetch the backend bundle, or `None` when the backend has none yet.
async fn fetch_bundle(backend: &Backend) -> Result<Option<SessionBundle>> {
    match backend {
        Backend::Folder { path } => {
            let file = path.join(GIST_FILENAME);
            if !file.exists() {
                return Ok(None);
            }
            let raw = std::fs::read_to_string(&file)?;
            let bundle: SessionBundle = serde_json::from_str(&raw)?;
            Ok(Some(bundle))
        }
        Backend::Gist { id } => {
            let token = github_token()?
                .ok_or_else(|| Error::Config("no GitHub token: set AETHER_GITHUB_TOKEN or write ~/.config/aether/github-token".into()))?;
            let client = http_client()?;
            let res = client
                .get(format!("{GIST_API}/gists/{id}"))
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "aether")
                .send()
                .await
                .map_err(|e| Error::Network(e.to_string()))?;
            if !res.status().is_success() {
                return Err(Error::Network(format!(
                    "gist fetch failed: HTTP {}",
                    res.status()
                )));
            }
            let json: serde_json::Value = res
                .json()
                .await
                .map_err(|e| Error::Network(e.to_string()))?;
            match json["files"][GIST_FILENAME]["content"].as_str() {
                Some(content) => {
                    let bundle: SessionBundle = serde_json::from_str(content)?;
                    Ok(Some(bundle))
                }
                None => Ok(None),
            }
        }
    }
}

/// Write a bundle to a backend (folder = atomic file write, gist = PATCH).
async fn write_bundle(backend: &Backend, bundle: &SessionBundle) -> Result<()> {
    match backend {
        Backend::Folder { path } => {
            ensure_dir(path)?;
            let file = path.join(GIST_FILENAME);
            let raw = serde_json::to_string_pretty(bundle)?;
            atomic_write(&file, raw.as_bytes())
        }
        Backend::Gist { id } => {
            let token = github_token()?
                .ok_or_else(|| Error::Config("no GitHub token: set AETHER_GITHUB_TOKEN or write ~/.config/aether/github-token".into()))?;
            let client = http_client()?;
            let content = serde_json::to_string(bundle)?;
            let body = serde_json::json!({
                "files": { GIST_FILENAME: { "content": content } }
            });
            let res = client
                .patch(format!("{GIST_API}/gists/{id}"))
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "aether")
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Network(e.to_string()))?;
            if !res.status().is_success() {
                return Err(Error::Network(format!(
                    "gist update failed: HTTP {}",
                    res.status()
                )));
            }
            Ok(())
        }
    }
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .build()
        .map_err(|e| Error::Network(e.to_string()))
}

/// Parse the `ts` field of a JSONL line into epoch millis (0 when absent/invalid).
fn line_timestamp(line: &str) -> i64 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return 0;
    };
    let Some(ts) = v.get("ts").and_then(|t| t.as_str()) else {
        return 0;
    };
    let Ok(dt) = time::OffsetDateTime::parse(
        ts,
        &time::format_description::well_known::Rfc3339,
    ) else {
        return 0;
    };
    dt.unix_timestamp() * 1000 + i64::from(dt.millisecond())
}

fn state_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("sync.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Point AETHER_CONFIG_DIR at a fresh temp dir (serialized: env is global).
    fn isolate(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aetherdz-sync-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("AETHER_CONFIG_DIR", &dir) };
        let sessions = dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions);
        dir
    }

    fn line(ts: &str, content: &str) -> String {
        serde_json::json!({ "role": "user", "content": content, "ts": ts }).to_string()
    }

    #[test]
    fn merge_lines_sorts_and_dedups() {
        let a = vec![
            line("2026-08-09T10:00:00.000Z", "first"),
            line("2026-08-09T10:00:02.000Z", "dup"),
        ];
        let b = vec![
            line("2026-08-09T10:00:01.000Z", "second"),
            line("2026-08-09T10:00:02.000Z", "dup"),
        ];
        let m = merge_lines(&a, &b);
        assert_eq!(m.len(), 3, "duplicate must be dropped: {m:?}");
        assert!(m[0].contains("\"first\""), "ts-sorted: {m:?}");
        assert!(m[1].contains("\"second\""));
        assert!(m[2].contains("\"dup\""));
    }

    #[test]
    fn merge_bundles_unions_ids_and_titles() {
        let local = SessionBundle {
            v: BUNDLE_VERSION,
            device_id: "device-a".into(),
            sessions: HashMap::from([
                (
                    "s1".into(),
                    SessionEntry {
                        title: Some("local title".into()),
                        lines: vec![line("2026-08-09T10:00:00.000Z", "l1")],
                    },
                ),
                (
                    "s3".into(),
                    SessionEntry {
                        title: None,
                        lines: vec![],
                    },
                ),
            ]),
        };
        let remote = SessionBundle {
            v: BUNDLE_VERSION,
            device_id: "device-b".into(),
            sessions: HashMap::from([
                (
                    "s1".into(),
                    SessionEntry {
                        title: None,
                        lines: vec![line("2026-08-09T10:00:01.000Z", "r1")],
                    },
                ),
                (
                    "s2".into(),
                    SessionEntry {
                        title: Some("remote only".into()),
                        lines: vec![line("2026-08-09T10:00:02.000Z", "r2")],
                    },
                ),
            ]),
        };
        let m = merge_bundles(&local, &remote);
        assert_eq!(m.sessions.len(), 3);
        // Title: local wins on conflict; remote-only keeps remote title.
        assert_eq!(m.sessions["s1"].title.as_deref(), Some("local title"));
        assert_eq!(m.sessions["s2"].title.as_deref(), Some("remote only"));
        assert_eq!(m.sessions["s3"].title, None);
        // Lines merged + sorted for s1.
        assert_eq!(m.sessions["s1"].lines.len(), 2);
        assert!(m.sessions["s1"].lines[0].contains("\"l1\""));
        assert!(m.sessions["s1"].lines[1].contains("\"r1\""));
        assert_eq!(m.device_id, "device-a");
    }

    #[test]
    fn state_save_load_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("state");
        let state = SyncState {
            backend: Some("folder".into()),
            folder: Some("/tmp/remote".into()),
            gist_id: None,
            device_id: "device-test".into(),
        };
        save_state(&state).unwrap();
        let loaded = load_state().unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn folder_push_pull_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = isolate("roundtrip");
        let remote = dir.join("remote");
        let backend = Backend::Folder {
            path: remote.clone(),
        };

        // Create one local session with two lines.
        let sid = SessionId::new("2026-08-09T10-00-00-000Z");
        let session = Session::open(&sid).unwrap();
        session.append("user", "hello sync").unwrap();
        session.append("assistant", "hello back").unwrap();
        session.rename("Sync Test").unwrap();

        // Push to folder backend.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let report = rt.block_on(push("device-push", &backend)).unwrap();
        assert_eq!(report.written, 1);
        assert!(remote.join(GIST_FILENAME).exists());

        // Wipe local sessions, then pull back.
        let sessions = dir.join("sessions");
        let _ = std::fs::remove_dir_all(&sessions);
        let _ = std::fs::create_dir_all(&sessions);
        let report = rt.block_on(pull(&backend)).unwrap();
        assert_eq!(report.written, 1);

        let restored = Session::open(&sid).unwrap();
        assert_eq!(restored.raw_lines().unwrap().len(), 2);
        assert_eq!(restored.title().unwrap(), "Sync Test");
    }

    #[test]
    fn device_id_shape_and_token_file() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = isolate("device");
        let id = generate_device_id();
        assert!(id.starts_with("device-"), "{id}");
        assert!(id.len() > "device-".len() + 4);

        // Token file fallback.
        let tok = "ghp_test_token_123";
        std::fs::write(dir.join("github-token"), format!("{tok}\n")).unwrap();
        assert_eq!(github_token().unwrap().as_deref(), Some(tok));
    }
}
