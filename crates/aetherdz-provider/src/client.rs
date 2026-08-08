//! OpenAI-compatible chat-completions client with SSE streaming.
//!
//! Wire format: `POST {base}/chat/completions` with `stream: true`, response
//! is `text/event-stream` of `data:` frames. This is the format used by zen,
//! OpenAI, Ollama, and every other OpenAI-compatible endpoint.

use std::collections::VecDeque;
use std::time::Duration;

use aetherdz_core::error::{redact_secret, Error, Result};
use futures::stream::{self, Stream};
use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;

/// Connect timeout for all requests (TS uses 8s for its fetch calls).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Total timeout for short non-streaming requests (models fetch).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// A chat-completions request body.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub stream: bool,
}

/// A single chat message (string content only in Phase 0).
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Token usage reported by the provider (present on the final chunk).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

/// One parsed SSE chunk from a streaming response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatChunk {
    pub content: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// OpenAI-compatible streaming client.
///
/// The API key is stored but never printed: [`Debug`] redacts it.
pub struct OpenAICompatibleClient {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl std::fmt::Debug for OpenAICompatibleClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompatibleClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_deref().map(redact_secret))
            .finish_non_exhaustive()
    }
}

impl OpenAICompatibleClient {
    /// Build a client for `base_url` with an optional bearer token.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("aetherdz")
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into(),
            api_key,
            http,
        })
    }

    /// Stream a chat completion. Yields text deltas as they arrive.
    pub async fn stream_chat(
        &self,
        request: &ChatRequest,
    ) -> Result<impl Stream<Item = Result<ChatChunk, Error>> + Send + '_> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut builder = self.http.post(&url).header(CONTENT_TYPE, "application/json");
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .json(request)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("HTTP {status}: {body}")));
        }
        Ok(sse_chunk_stream(response.bytes_stream()))
    }
}

/// Convert a raw byte stream into a stream of parsed chat chunks.
fn sse_chunk_stream(
    byte_stream: impl Stream<Item = Result<reqwest::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<ChatChunk, Error>> + Send {
    let mut parser = SseParser::new();
    let mut byte_stream = Box::pin(byte_stream);
    stream::unfold((parser, byte_stream), |(mut parser, mut byte_stream)| async move {
        loop {
            if let Some(data) = parser.pop_event() {
                return Some((parse_chunk(&data), (parser, byte_stream)));
            }
            match byte_stream.next().await {
                Some(Ok(bytes)) => parser.push(&bytes),
                Some(Err(e)) => {
                    return Some((Err(Error::Network(e.to_string())), (parser, byte_stream)));
                }
                None => {
                    parser.finish();
                    if let Some(data) = parser.pop_event() {
                        return Some((parse_chunk(&data), (parser, byte_stream)));
                    }
                    return None;
                }
            }
        }
    })
}

/// Parse one SSE `data:` payload into a chat chunk.
fn parse_chunk(data: &str) -> Result<ChatChunk, Error> {
    if data == "[DONE]" {
        return Ok(ChatChunk {
            finish_reason: Some("stop".to_string()),
            ..ChatChunk::default()
        });
    }
    let value: serde_json::Value =
        serde_json::from_str(data).map_err(|e| Error::Provider(format!("malformed SSE chunk: {e}")))?;
    let usage = value
        .get("usage")
        .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());
    let Some(choice) = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|c| c.first())
    else {
        // Usage-only or empty chunk; nothing to emit.
        return Ok(ChatChunk { usage, ..ChatChunk::default() });
    };
    let content = choice
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let finish_reason = choice
        .get("finish_reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok(ChatChunk {
        content,
        finish_reason,
        usage,
    })
}

/// Incremental SSE parser. Handles chunks split across network boundaries
/// and both `\n\n` and `\r\n\r\n` event terminators.
struct SseParser {
    buffer: Vec<u8>,
    events: VecDeque<String>,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            events: VecDeque::new(),
        }
    }

    /// Feed raw bytes; complete events become available via [`pop_event`].
    fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        self.drain_events();
    }

    /// Flush any trailing event at end of stream.
    fn finish(&mut self) {
        if !self.buffer.is_empty() {
            if let Some(data) = parse_sse_event(&self.buffer) {
                self.events.push_back(data);
            }
            self.buffer.clear();
        }
    }

    /// Pop the next complete event payload, if any.
    fn pop_event(&mut self) -> Option<String> {
        self.events.pop_front()
    }

    fn drain_events(&mut self) {
        while let Some(end) = find_event_end(&self.buffer) {
            let event: Vec<u8> = self.buffer.drain(..=end).collect();
            if let Some(data) = parse_sse_event(&event) {
                self.events.push_back(data);
            }
        }
    }
}

/// Index of the last byte of the first complete SSE event block.
fn find_event_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 1);
        }
    }
    for i in 0..buf.len().saturating_sub(3) {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i + 3);
        }
    }
    None
}

/// Extract the `data:` payload from one SSE event block.
fn parse_sse_event(event: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(event);
    let mut data = String::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest);
    }
    if data.is_empty() {
        None
    } else {
        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_handles_split_chunks() {
        let mut parser = SseParser::new();
        // First event split across two pushes.
        parser.push(b"data: {\"a\":1}\n\n");
        assert_eq!(parser.pop_event().as_deref(), Some("{\"a\":1}"));
        // Second event arrives in fragments.
        parser.push(b"data: {\"b\"");
        assert!(parser.pop_event().is_none());
        parser.push(b":2}\n\ndata: [DONE]\n\n");
        assert_eq!(parser.pop_event().as_deref(), Some("{\"b\":2}"));
        assert_eq!(parser.pop_event().as_deref(), Some("[DONE]"));
        assert!(parser.pop_event().is_none());
    }

    #[test]
    fn sse_parser_handles_crlf() {
        let mut parser = SseParser::new();
        parser.push(b"data: hello\r\n\r\n");
        assert_eq!(parser.pop_event().as_deref(), Some("hello"));
    }

    #[test]
    fn sse_parser_ignores_non_data_lines() {
        let mut parser = SseParser::new();
        parser.push(b": comment\nid: 1\ndata: payload\n\n");
        assert_eq!(parser.pop_event().as_deref(), Some("payload"));
    }

    #[test]
    fn parse_chunk_extracts_delta() {
        let chunk = parse_chunk(
            r#"{"id":"x","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        )
        .unwrap();
        assert_eq!(chunk.content.as_deref(), Some("Hel"));
        assert!(chunk.finish_reason.is_none());
    }

    #[test]
    fn parse_chunk_handles_done() {
        let chunk = parse_chunk("[DONE]").unwrap();
        assert_eq!(chunk.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_chunk_handles_usage_only() {
        let chunk = parse_chunk(r#"{"usage":{"prompt_tokens":10,"completion_tokens":5}}"#).unwrap();
        assert!(chunk.content.is_none());
        assert_eq!(chunk.usage.unwrap().prompt_tokens, Some(10));
    }

    #[test]
    fn debug_redacts_api_key() {
        let client = OpenAICompatibleClient::new(
            "https://example.com/v1",
            Some("sk-super-secret-key-123456".to_string()),
        )
        .unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("sk-super-secret-key-123456"));
        assert!(debug.contains("sk…56"));
    }
}
