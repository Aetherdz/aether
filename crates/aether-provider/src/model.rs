//! Static model catalog, mirroring `aether-cli/src/providers/models.ts`.

/// Zen models verified to work WITHOUT any API key.
pub const ZEN_FREE_MODELS: &[&str] = &[
    "big-pickle",
    "deepseek-v4-flash-free",
    "laguna-s-2.1-free",
    "ling-3.0-flash-free",
    "ling-3.0-tiny-free",
    "longcat-2.0-free",
    "mimo-v2.5-free",
    "nemotron-3-ultra-free",
    "north-mini-code-free",
];

/// Static per-provider model lists (fallback when a live fetch fails).
pub const STATIC_MODELS: &[(&str, &[&str])] = &[
    (
        "zen",
        &[
            "deepseek-v4-flash-free",
            "deepseek-v4-flash",
            "deepseek-v4-pro",
            "mimo-v2.5-free",
            "north-mini-code-free",
            "nemotron-3-ultra-free",
            "big-pickle",
            "laguna-s-2.1-free",
            "ling-3.0-flash-free",
            "ling-3.0-tiny-free",
            "longcat-2.0-free",
            "qwen3.6-plus",
            "minimax-m3",
            "claude-sonnet-5",
            "claude-opus-4-7",
            "claude-fable-5",
            "gpt-5.2",
            "gemini-3.6-flash",
            "kimi-k2.6",
            "grok-4.5",
        ],
    ),
    ("openai", &["gpt-4o", "gpt-4o-mini", "o3-mini"]),
    ("ollama", &["llama3.2", "qwen2.5-coder", "deepseek-coder"]),
    (
        "anthropic",
        &["claude-sonnet-4-5", "claude-sonnet-4", "claude-haiku-4-5"],
    ),
    (
        "google",
        &["gemini-3-flash", "gemini-3-pro", "gemini-2.5-flash"],
    ),
    ("deepseek", &["deepseek-chat", "deepseek-reasoner"]),
    (
        "openrouter",
        &["deepseek/deepseek-chat", "anthropic/claude-sonnet-4-5"],
    ),
    (
        "groq",
        &[
            "llama-3.3-70b-versatile",
            "mixtral-8x7b-instruct",
            "deepseek-r1-distill-llama-70b",
        ],
    ),
    (
        "mistral",
        &[
            "mistral-large-latest",
            "codestral-latest",
            "mistral-small-latest",
        ],
    ),
    ("xai", &["grok-4", "grok-3", "grok-2"]),
    ("cerebras", &["llama3.1-8b", "llama3.3-70b"]),
    (
        "togetherai",
        &["deepseek-ai/DeepSeek-V3", "Qwen/Qwen2.5-72B-Instruct"],
    ),
    (
        "fireworks",
        &["accounts/fireworks/models/llama-v3p3-70b-instruct"],
    ),
    ("perplexity", &["sonar-pro", "sonar"]),
    ("moonshot", &["kimi-k2", "moonshot-v1-8k"]),
    ("minimax", &["MiniMax-M2", "MiniMax-Text-01"]),
    (
        "huggingface",
        &[
            "qwen/Qwen2.5-72B-Instruct",
            "meta-llama/Llama-3.3-70B-Instruct",
        ],
    ),
    ("lmstudio", &["local-model"]),
    ("github", &["gpt-4.1", "gpt-4o-mini"]),
];

/// Default model per provider.
pub const DEFAULT_MODELS: &[(&str, &str)] = &[
    ("zen", "deepseek-v4-flash-free"),
    ("openai", "gpt-4o"),
    ("ollama", "llama3.2"),
];

/// Static model list for a provider id.
pub fn static_models(provider_id: &str) -> Vec<String> {
    STATIC_MODELS
        .iter()
        .find(|(id, _)| *id == provider_id)
        .map(|(_, models)| models.iter().map(|m| (*m).to_string()).collect())
        .unwrap_or_default()
}

/// Default model for a provider id, if known.
pub fn default_model(provider_id: &str) -> Option<String> {
    DEFAULT_MODELS
        .iter()
        .find(|(id, _)| *id == provider_id)
        .map(|(_, m)| (*m).to_string())
}

/// Whether a model id is in the zen free (no-key) set.
pub fn is_zen_free_model(model: &str) -> bool {
    ZEN_FREE_MODELS.contains(&model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zen_default_is_free() {
        assert!(is_zen_free_model("deepseek-v4-flash-free"));
        assert!(!is_zen_free_model("claude-sonnet-5"));
    }

    #[test]
    fn static_models_known() {
        assert!(static_models("zen").contains(&"deepseek-v4-flash-free".to_string()));
        assert!(static_models("nope").is_empty());
    }
}
