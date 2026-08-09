//! aether-provider: provider trait + OpenAI-compatible SSE streaming.
//!
//! Feature-gated providers: `zen` (default), `openai`, `ollama`. All speak
//! the OpenAI-compatible chat-completions wire format in Phase 0.

pub mod client;
pub mod model;
pub mod provider;
pub mod registry;

pub use client::{
    ChatChunk, ChatCompletion, ChatMessage, ChatRequest, CompletionChoice, CompletionMessage,
    OpenAICompatibleClient, ToolCall, ToolCallFunction, ToolDef, ToolFunction, Usage,
};
pub use provider::{OpenAIProvider, Pricing, Provider};
pub use registry::{
    fetch_zen_models, get_provider, key_is_set, key_status, list_providers, normalize_defaults,
    resolve_default, zen_provider, KeyStatus, ResolvedModel, ZEN_BASE_URL, ZEN_MODELS_URL,
};
