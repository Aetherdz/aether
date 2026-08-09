//! aetherdz-mcp — MCP server exposing aether session/recall/sync tools.
//!
//! Transports:
//! - stdio (`serve_stdio`)
//! - Streamable HTTP / SSE (`serve_http`)

use aetherdz_session::{Recall, Session, SessionId};
use aetherdz_sync::{github_token, load_state, pull, push, resolve_backend};
use rmcp::{
    ErrorData, ServiceExt,
    handler::server::wrapper::Parameters,
    model::ErrorCode,
    schemars, tool, tool_router,
};
use serde::Deserialize;
use serde_json::json;

/// The `#[tool_router(server_handler)]` macro generates a static `Self::tool_router()`
/// used by the auto-generated `ServerHandler` impl, so the struct itself needs no field.
#[derive(Debug, Clone, Default)]
pub struct AetherServer;

impl AetherServer {
    pub fn new() -> Self {
        Self
    }
}

fn internal(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowSessionParams {
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    pub phrase: String,
}

#[tool_router(server_handler)]
impl AetherServer {
    /// List all sessions (id, title, ts, message count).
    #[tool(description = "List all aether sessions")]
    fn list_sessions(&self) -> Result<String, ErrorData> {
        let sessions = aetherdz_session::list_sessions().map_err(|e| internal(e.to_string()))?;
        let rows: Vec<_> = sessions
            .iter()
            .map(|s| {
                json!({
                    "id": s.id.as_str(),
                    "title": s.title,
                    "ts": s.ts,
                    "messages": s.messages,
                    "size": s.size,
                })
            })
            .collect();
        serde_json::to_string(&rows).map_err(|e| internal(e.to_string()))
    }

    /// Show the full transcript of one session.
    #[tool(description = "Show the transcript of a session by id")]
    fn show_session(
        &self,
        Parameters(ShowSessionParams { id }): Parameters<ShowSessionParams>,
    ) -> Result<String, ErrorData> {
        let session =
            Session::open(&SessionId::new(id.clone())).map_err(|e| internal(e.to_string()))?;
        let messages = session.read_messages().map_err(|e| internal(e.to_string()))?;
        let rows: Vec<_> = messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content, "ts": m.ts }))
            .collect();
        serde_json::to_string(&rows).map_err(|e| internal(e.to_string()))
    }

    /// Keyword search over past sessions.
    #[tool(description = "Search past sessions by keyword")]
    fn recall(
        &self,
        Parameters(RecallParams { phrase }): Parameters<RecallParams>,
    ) -> Result<String, ErrorData> {
        let hits = Recall::search(&phrase, 10).map_err(|e| internal(e.to_string()))?;
        let rows: Vec<_> = hits
            .iter()
            .map(|h| {
                json!({
                    "id": h.id.as_str(),
                    "title": h.title,
                    "role": h.role,
                    "snippet": h.snippet,
                    "matches": h.matches,
                })
            })
            .collect();
        serde_json::to_string(&rows).map_err(|e| internal(e.to_string()))
    }

    /// Current sync configuration and token presence.
    #[tool(description = "Show sync status (backend, device, token)")]
    fn sync_status(&self) -> Result<String, ErrorData> {
        let state = load_state().map_err(|e| internal(e.to_string()))?;
        let token_present = github_token().map(|t| t.is_some()).unwrap_or(false);
        let body = json!({
            "backend": state.backend,
            "gistId": state.gist_id,
            "folder": state.folder,
            "deviceId": state.device_id,
            "tokenPresent": token_present,
        });
        serde_json::to_string(&body).map_err(|e| internal(e.to_string()))
    }

    /// Push local sessions to the configured sync backend.
    #[tool(description = "Push local sessions to the sync backend")]
    async fn sync_push(&self) -> Result<String, ErrorData> {
        let state = load_state().map_err(|e| internal(e.to_string()))?;
        let backend = resolve_backend(&state).map_err(|e| internal(e.to_string()))?;
        let report = push(&state.device_id, &backend)
            .await
            .map_err(|e| internal(e.to_string()))?;
        let body = json!({ "written": report.written, "merged": report.merged });
        serde_json::to_string(&body).map_err(|e| internal(e.to_string()))
    }

    /// Pull the sync backend bundle and merge into the local store.
    #[tool(description = "Pull sessions from the sync backend")]
    async fn sync_pull(&self) -> Result<String, ErrorData> {
        let state = load_state().map_err(|e| internal(e.to_string()))?;
        let backend = resolve_backend(&state).map_err(|e| internal(e.to_string()))?;
        let report = pull(&backend).await.map_err(|e| internal(e.to_string()))?;
        let body = json!({ "written": report.written, "merged": report.merged });
        serde_json::to_string(&body).map_err(|e| internal(e.to_string()))
    }
}

/// Serve over stdio until the peer closes the pipe.
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = AetherServer::new();
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

/// Serve the Streamable HTTP / SSE transport on `addr` (e.g. "127.0.0.1:8080").
pub async fn serve_http(addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };

    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["localhost", "127.0.0.1"])
        .with_json_response(false);

    let service: StreamableHttpService<AetherServer, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(AetherServer::new()), Default::default(), config);

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn isolate(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aetherdz-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("AETHER_CONFIG_DIR", &dir) };
        let sessions = dir.join("sessions");
        let _ = std::fs::create_dir_all(&sessions);
        dir
    }

    fn seed_session() {
        let sid = SessionId::new("2026-08-09T10-00-00-000Z");
        let session = Session::open(&sid).unwrap();
        session.append("user", "hello mcp").unwrap();
        session.append("assistant", "hello back").unwrap();
        session.rename("Mcp Test").unwrap();
    }

    #[tokio::test]
    async fn list_sessions_and_show_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("list");
        let server = AetherServer::new();
        let listed = server.list_sessions().unwrap();
        let rows: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 0);

        seed_session();
        let listed = server.list_sessions().unwrap();
        let rows: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);

        let shown = server
            .show_session(Parameters(ShowSessionParams {
                id: "2026-08-09T10-00-00-000Z".into(),
            }))
            .unwrap();
        let msgs: serde_json::Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(msgs.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn recall_finds_seeded_content() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("recall");
        seed_session();
        let server = AetherServer::new();
        let hits = server
            .recall(Parameters(RecallParams {
                phrase: "hello mcp".into(),
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&hits).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sync_status_reports_off() {
        let _g = ENV_LOCK.lock().unwrap();
        isolate("status");
        let server = AetherServer::new();
        let status = server.sync_status().unwrap();
        let v: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(v["backend"], serde_json::Value::Null);
        assert_eq!(v["tokenPresent"], serde_json::Value::Bool(false));
    }
}
