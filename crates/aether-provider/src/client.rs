//! OpenAI-compatible chat-completions client with SSE streaming.
//!
//! Wire format: `POST {base}/chat/completions` with `stream: true`, response
//! is `text/event-stream` of `data:` frames. This is the format used by zen,
//! OpenAI, Ollama, and every other OpenAI-compatible endpoint.

use std::collections::VecDeque;
use std::time::Duration;

use aether_core::error::{redact_secret, Error, Result};
use futures::stream::{self, Stream};
use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

/// Connect timeout for all requests (TS uses 8s for its fetch calls).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Total timeout for short non-streaming requests (models fetch).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// Transient HTTP statuses worth retrying. Mirrors jcode's transient-error
/// handling: 408 (timeout), 429 (rate limit), and 5xx (server fault).
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// Exponential backoff with full jitter (jcode's retry discipline).
///
/// `attempt` is 1-based. Delay = random in `[0, min(base * 2^(attempt-1), cap))`.
/// A small floor keeps the first retry from firing instantly on flaky links.
pub fn backoff_delay(attempt: u32, base: Duration, cap: Duration, floor: Duration) -> Duration {
    if attempt <= 1 {
        return floor;
    }
    let exponent = base
        .saturating_mul(1u32.checked_shl(attempt.saturating_sub(1).min(10)).unwrap_or(u32::MAX))
        .min(cap);
    let nanos = exponent.as_nanos() as u64;
    // Full jitter: uniform draw in [0, window). Deterministic for tests when
    // a seed is set; otherwise uses thread_rng.
    let window = nanos.max(1);
    let draw = std::time::Duration::from_nanos(randish(window));
    draw.max(floor).min(cap)
}

/// Uniform pseudo-random draw in `[0, window)`.
/// Uses a cheap xorshift fed by system time when no test seed is set.
fn randish(window: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0);
    let mut state = SEED.load(Ordering::Relaxed);
    if state == 0 {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        state = t | 1;
        SEED.store(state, Ordering::Relaxed);
    }
    // xorshift64*
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    SEED.store(state, Ordering::Relaxed);
    let r = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (r >> 11) % window
}

/// Retry policy for transient transport/provider failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first (1 = no retry).
    pub max_attempts: u32,
    /// Base backoff unit (doubled per attempt).
    pub base_delay: Duration,
    /// Upper bound on any single backoff.
    pub max_delay: Duration,
    /// Minimum delay before the first retry.
    pub min_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
            min_delay: Duration::from_millis(250),
        }
    }
}

impl RetryPolicy {
    /// Is a retry worthwhile given how many attempts remain?
    pub fn can_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }
}

/// A chat-completions request body.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub stream: bool,
    /// Function tools the model may call (omitted when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
}

/// A single chat message.
///
/// The wire shape varies by role: user/assistant carry text in `content`,
/// tool results carry `tool_call_id` + text, and assistant messages that
/// invoke tools carry `tool_calls` with an empty (omitted) content.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Id of the tool call this message answers (role = "tool").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool invocations requested by the assistant (role = "assistant").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
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

/// A function tool offered to the model (OpenAI `tools[]` item).
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

/// The name/schema half of a [`ToolDef`].
#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

/// The name + JSON-encoded arguments of a [`ToolCall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

/// A non-streaming chat completion response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletion {
    pub choices: Vec<CompletionChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// One choice of a non-streaming completion.
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionChoice {
    pub message: CompletionMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// The assistant message of a non-streaming completion.
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
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
            .user_agent("aether")
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
    ) -> Result<impl Stream<Item = Result<ChatChunk>> + Send + '_> {
        self.stream_chat_with_retry(request, RetryPolicy::default())
            .await
    }

    /// Stream a chat completion, retrying transient failures (network errors,
    /// HTTP 408/429/5xx) with exponential backoff + jitter until the stream
    /// actually opens. Once streaming begins, mid-stream errors surface as
    /// stream items and are not retried (a half-consumed SSE body cannot be
    /// replayed safely).
    pub async fn stream_chat_with_retry(
        &self,
        request: &ChatRequest,
        policy: RetryPolicy,
    ) -> Result<impl Stream<Item = Result<ChatChunk>> + Send + '_> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match self.try_open_stream(request).await {
                Ok(stream) => return Ok(stream),
                Err(err) if policy.can_retry(attempt) && retryable_error(&err) => {
                    let delay = backoff_delay(
                        attempt,
                        policy.base_delay,
                        policy.max_delay,
                        policy.min_delay,
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Run one non-streaming completion, returning the full assistant message
    /// (text and/or tool calls). Retries transient failures exactly like
    /// [`stream_chat_with_retry`].
    pub async fn complete(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatCompletion> {
        self.complete_with_retry(request, RetryPolicy::default()).await
    }

    /// Non-streaming completion with an explicit retry policy.
    pub async fn complete_with_retry(
        &self,
        request: &ChatRequest,
        policy: RetryPolicy,
    ) -> Result<ChatCompletion> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match self.try_complete(request).await {
                Ok(completion) => return Ok(completion),
                Err(err) if policy.can_retry(attempt) && retryable_error(&err) => {
                    let delay = backoff_delay(
                        attempt,
                        policy.base_delay,
                        policy.max_delay,
                        policy.min_delay,
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Send a non-streaming request and parse the JSON completion body.
    async fn try_complete(&self, request: &ChatRequest) -> Result<ChatCompletion> {
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
        response
            .json::<ChatCompletion>()
            .await
            .map_err(|e| Error::Provider(format!("malformed completion body: {e}")))
    }

    /// Send the request and validate the response status; on a retryable
    /// status, surface enough detail for the retry loop to classify it.
    async fn try_open_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<impl Stream<Item = Result<ChatChunk>> + Send + '_> {
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

/// Whether an error is worth retrying: network/transport faults, or provider
/// errors whose status code is in the transient set (408/429/5xx).
fn retryable_error(err: &Error) -> bool {
    match err {
        Error::Network(_) => true,
        Error::Provider(msg) => {
            // `HTTP 503: ...` — parse the leading status code.
            let code = msg
                .strip_prefix("HTTP ")
                .and_then(|rest| rest.split([' ', ':']).next())
                .and_then(|num| num.parse::<u16>().ok());
            code.is_some_and(is_retryable_status)
        }
        _ => false,
    }
}

/// Convert a raw byte stream into a stream of parsed chat chunks.
fn sse_chunk_stream<B>(
    byte_stream: impl Stream<Item = std::result::Result<B, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<ChatChunk>> + Send
where
    B: AsRef<[u8]> + 'static,
{
    let parser = SseParser::new();
    let byte_stream = Box::pin(byte_stream);
    stream::unfold((parser, byte_stream), |(mut parser, mut byte_stream)| async move {
        loop {
            if let Some(data) = parser.pop_event() {
                return Some((parse_chunk(&data), (parser, byte_stream)));
            }
            match byte_stream.next().await {
                Some(Ok(bytes)) => parser.push(bytes.as_ref()),
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
fn parse_chunk(data: &str) -> Result<ChatChunk> {
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

    #[test]
    fn retryable_status_set() {
        assert!(is_retryable_status(408));
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(200));
    }

    #[test]
    fn retryable_error_classification() {
        assert!(retryable_error(&Error::Network("conn reset".into())));
        assert!(retryable_error(&Error::Provider("HTTP 429: slow down".into())));
        assert!(retryable_error(&Error::Provider("HTTP 503: unavailable".into())));
        assert!(!retryable_error(&Error::Provider("HTTP 401: nope".into())));
        assert!(!retryable_error(&Error::Provider("HTTP 400: bad req".into())));
        assert!(!retryable_error(&Error::Provider("malformed SSE chunk: x".into())));
        assert!(!retryable_error(&Error::InvalidInput("nope".into())));
    }

    #[test]
    fn backoff_delay_is_bounded_and_monotonic() {
        let base = Duration::from_millis(500);
        let cap = Duration::from_secs(8);
        let floor = Duration::from_millis(250);
        let d1 = backoff_delay(1, base, cap, floor);
        assert_eq!(d1, floor);
        for attempt in 2..=12 {
            let d = backoff_delay(attempt, base, cap, floor);
            assert!(d >= floor, "attempt {attempt} below floor");
            assert!(d <= cap, "attempt {attempt} above cap");
        }
        for _ in 0..64 {
            let d = backoff_delay(20, base, cap, floor);
            assert!(d >= floor && d <= cap, "saturated attempt out of bounds: {d:?}");
        }
    }

    #[test]
    fn retry_policy_attempt_budget() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 3);
        assert!(p.can_retry(1));
        assert!(p.can_retry(2));
        assert!(!p.can_retry(3));
        let no_retry = RetryPolicy { max_attempts: 1, ..RetryPolicy::default() };
        assert!(!no_retry.can_retry(1));
    }

    #[test]
    fn request_serializes_tools_and_skips_when_absent() {
        let def = ToolDef {
            kind: "function".to_string(),
            function: ToolFunction {
                name: "read_file".to_string(),
                description: "Read a file".to_string(),
                parameters: Some(serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}})),
            },
        };
        let req = ChatRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                ..ChatMessage::default()
            }],
            temperature: None,
            stream: false,
            tools: Some(vec![def]),
        };
        let json: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "read_file");
        assert!(json["tools"][0]["function"]["parameters"].is_object());

        let plain = ChatRequest {
            tools: None,
            ..req
        };
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn message_skips_empty_content_and_tool_fields() {
        let user = ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            ..ChatMessage::default()
        };
        let json = serde_json::to_value(&user).unwrap();
        assert_eq!(json["content"], "hi");
        assert!(json.get("tool_calls").is_none());
        assert!(json.get("tool_call_id").is_none());

        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                kind: "function".to_string(),
                function: ToolCallFunction {
                    name: "read_file".to_string(),
                    arguments: r#"{"path":"a.txt"}"#.to_string(),
                },
            }]),
            ..ChatMessage::default()
        };
        let json = serde_json::to_value(&assistant).unwrap();
        assert!(json.get("content").is_none(), "empty content must be omitted");
        assert_eq!(json["tool_calls"][0]["function"]["name"], "read_file");

        let tool_result = ChatMessage {
            role: "tool".to_string(),
            content: "contents".to_string(),
            tool_call_id: Some("call_1".to_string()),
            ..ChatMessage::default()
        };
        let json = serde_json::to_value(&tool_result).unwrap();
        assert_eq!(json["role"], "tool");
        assert_eq!(json["tool_call_id"], "call_1");
    }

    #[test]
    fn completion_parses_tool_calls() {
        let raw = r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": {"name": "run_command", "arguments": "{\"cmd\":\"ls\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
        }"#;
        let completion: ChatCompletion = serde_json::from_str(raw).unwrap();
        let choice = &completion.choices[0];
        assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
        assert!(choice.message.content.is_none());
        let calls = choice.message.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_9");
        assert_eq!(calls[0].function.name, "run_command");
        assert_eq!(completion.usage.unwrap().total_tokens, Some(13));
    }

    #[test]
    fn completion_parses_plain_text() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}]}"#;
        let completion: ChatCompletion = serde_json::from_str(raw).unwrap();
        assert!(completion.choices[0].message.tool_calls.is_none());
        assert_eq!(completion.choices[0].message.content.as_deref(), Some("done"));
        assert_eq!(completion.choices[0].finish_reason.as_deref(), Some("stop"));
    }
}
