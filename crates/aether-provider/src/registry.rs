//! Provider registry and default resolution.
//!
//! Mirrors `aether-cli/src/providers/registry.ts`: built-in providers plus
//! custom providers from config, with the same graceful-fallback semantics
//! in [`resolve_default`].

use std::time::Duration;

use aether_core::config::{AetherConfig, CustomProviderConfig, DEFAULT_MODEL, DEFAULT_PROVIDER};
use aether_core::error::{Error, Result};

use crate::model::{is_zen_free_model, static_models};
use crate::provider::{OpenAIProvider, Pricing};

/// Zen endpoint base URL (from `registry.ts`).
pub const ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";
/// Zen live models endpoint (from `models.ts`).
pub const ZEN_MODELS_URL: &str = "https://opencode.ai/zen/v1/models";

/// The zen provider: free OpenAI-compatible endpoint, no API key required.
pub fn zen_provider() -> OpenAIProvider {
    OpenAIProvider {
        id: "zen".to_string(),
        name: "OpenCode Zen".to_string(),
        kind: "zen".to_string(),
        base_url: ZEN_BASE_URL.to_string(),
        key_env: None,
        needs_key: false,
        free: Pricing::Free,
        description: "Free OpenAI-compatible endpoint, no API key required".to_string(),
        default_model: "deepseek-v4-flash-free".to_string(),
        static_models: static_models("zen"),
    }
}

/// OpenAI hosted models (requires `OPENAI_API_KEY`).
pub fn openai_provider() -> OpenAIProvider {
    OpenAIProvider {
        id: "openai".to_string(),
        name: "OpenAI".to_string(),
        kind: "openai".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        key_env: Some("OPENAI_API_KEY".to_string()),
        needs_key: true,
        free: Pricing::Paid,
        description: "OpenAI hosted models".to_string(),
        default_model: "gpt-4o".to_string(),
        static_models: static_models("openai"),
    }
}

/// One row of a built-in provider descriptor (mirrors `registry.ts`).
struct BuiltinSpec {
    id: &'static str,
    name: &'static str,
    kind: &'static str,
    key_env: Option<&'static str>,
    free: Pricing,
    description: &'static str,
    default_model: &'static str,
    needs_key: bool,
}

/// The full built-in provider catalog, same set and order as the TS registry.
const BUILTINS: &[BuiltinSpec] = &[
    BuiltinSpec {
        id: "zen",
        name: "OpenCode Zen",
        kind: "zen",
        key_env: None,
        needs_key: false,
        free: Pricing::Free,
        description: "Free OpenAI-compatible endpoint, no API key required",
        default_model: "deepseek-v4-flash-free",
    },
    BuiltinSpec {
        id: "openai",
        name: "OpenAI",
        kind: "openai",
        key_env: Some("OPENAI_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "OpenAI hosted models",
        default_model: "gpt-4o",
    },
    BuiltinSpec {
        id: "anthropic",
        name: "Anthropic Claude",
        kind: "anthropic",
        key_env: Some("ANTHROPIC_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "Anthropic Claude models",
        default_model: "claude-sonnet-4-5",
    },
    BuiltinSpec {
        id: "google",
        name: "Google Gemini",
        kind: "google",
        key_env: Some("GOOGLE_GENERATIVE_AI_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Google Gemini models (free tier available)",
        default_model: "gemini-3-flash",
    },
    BuiltinSpec {
        id: "deepseek",
        name: "DeepSeek",
        kind: "deepseek",
        key_env: Some("DEEPSEEK_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "DeepSeek hosted models",
        default_model: "deepseek-chat",
    },
    BuiltinSpec {
        id: "openrouter",
        name: "OpenRouter",
        kind: "openrouter",
        key_env: Some("OPENROUTER_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Aggregated models incl. free tiers",
        default_model: "deepseek/deepseek-chat",
    },
    BuiltinSpec {
        id: "ollama",
        name: "Ollama (local)",
        kind: "ollama",
        key_env: None,
        needs_key: false,
        free: Pricing::Free,
        description: "Local Ollama server, no key",
        default_model: "llama3.2",
    },
    BuiltinSpec {
        id: "groq",
        name: "Groq",
        kind: "groq",
        key_env: Some("GROQ_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Fast inference on Llama/other open models",
        default_model: "llama-3.3-70b-versatile",
    },
    BuiltinSpec {
        id: "mistral",
        name: "Mistral",
        kind: "mistral",
        key_env: Some("MISTRAL_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Mistral Large/Small + Codestral",
        default_model: "mistral-large-latest",
    },
    BuiltinSpec {
        id: "xai",
        name: "xAI",
        kind: "xai",
        key_env: Some("XAI_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "Grok models by xAI",
        default_model: "grok-4",
    },
    BuiltinSpec {
        id: "cerebras",
        name: "Cerebras",
        kind: "cerebras",
        key_env: Some("CEREBRAS_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Ultra-fast Llama on Cerebras hardware",
        default_model: "llama3.1-8b",
    },
    BuiltinSpec {
        id: "togetherai",
        name: "Together AI",
        kind: "togetherai",
        key_env: Some("TOGETHER_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Open models incl. DeepSeek, Llama, Qwen",
        default_model: "deepseek-ai/DeepSeek-V3",
    },
    BuiltinSpec {
        id: "fireworks",
        name: "Fireworks",
        kind: "fireworks",
        key_env: Some("FIREWORKS_API_KEY"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Fast open-model inference",
        default_model: "accounts/fireworks/models/llama-v3p3-70b-instruct",
    },
    BuiltinSpec {
        id: "perplexity",
        name: "Perplexity",
        kind: "perplexity",
        key_env: Some("PERPLEXITY_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "Sonar models with web-grounded answers",
        default_model: "sonar-pro",
    },
    BuiltinSpec {
        id: "moonshot",
        name: "Moonshot",
        kind: "moonshot",
        key_env: Some("MOONSHOT_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "Kimi models (long context)",
        default_model: "kimi-k2",
    },
    BuiltinSpec {
        id: "minimax",
        name: "MiniMax",
        kind: "minimax",
        key_env: Some("MINIMAX_API_KEY"),
        needs_key: true,
        free: Pricing::Paid,
        description: "MiniMax M2/Text models",
        default_model: "MiniMax-M2",
    },
    BuiltinSpec {
        id: "huggingface",
        name: "HuggingFace",
        kind: "huggingface",
        key_env: Some("HF_TOKEN"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Open models via HF Inference Router",
        default_model: "qwen/Qwen2.5-72B-Instruct",
    },
    BuiltinSpec {
        id: "lmstudio",
        name: "LM Studio",
        kind: "lmstudio",
        key_env: None,
        needs_key: false,
        free: Pricing::Free,
        description: "Local LM Studio server, no key",
        default_model: "local-model",
    },
    BuiltinSpec {
        id: "github",
        name: "GitHub Models",
        kind: "github",
        key_env: Some("GITHUB_TOKEN"),
        needs_key: true,
        free: Pricing::FreePaid,
        description: "Free GPT models via GitHub Models",
        default_model: "gpt-4.1",
    },
];

/// Build the provided catalog preserving the TS order.
pub fn builtin_providers() -> Vec<OpenAIProvider> {
    // zen is special-cased first (default provider), then the rest.
    let mut providers = vec![zen_provider()];
    providers.extend(
        BUILTINS
            .iter()
            .filter(|b| b.id != "zen")
            .map(|b| OpenAIProvider {
                id: b.id.to_string(),
                name: b.name.to_string(),
                kind: b.kind.to_string(),
                base_url: platform_base_url(b.id, b.kind),
                key_env: b.key_env.map(str::to_string),
                needs_key: b.needs_key,
                free: b.free,
                description: b.description.to_string(),
                default_model: b.default_model.to_string(),
                static_models: static_models(b.id),
            }),
    );
    providers
}

/// Base URL for a built-in provider id (mirrors TS `baseUrl`).
fn platform_base_url(id: &str, kind: &str) -> String {
    match id {
        "anthropic" => "https://api.anthropic.com/v1".to_string(),
        "google" => "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
        "deepseek" => "https://api.deepseek.com/v1".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        "ollama" => "http://localhost:11434/api".to_string(),
        "groq" => "https://api.groq.com/openai/v1".to_string(),
        "mistral" => "https://api.mistral.ai/v1".to_string(),
        "xai" => "https://api.x.ai/v1".to_string(),
        "cerebras" => "https://api.cerebras.ai/v1".to_string(),
        "togetherai" => "https://api.together.xyz/v1".to_string(),
        "fireworks" => "https://api.fireworks.ai/inference/v1".to_string(),
        "perplexity" => "https://api.perplexity.ai/v1".to_string(),
        "moonshot" => "https://api.moonshot.cn/v1".to_string(),
        "minimax" => "https://api.minimax.chat/v1".to_string(),
        "huggingface" => "https://router.huggingface.co/v1".to_string(),
        "lmstudio" => "http://localhost:1234/v1".to_string(),
        "github" => "https://models.github.ai/inference".to_string(),
        _ => format!("https://{kind}.aether.invalid/v1"),
    }
}

/// All providers: built-ins plus custom providers from the config.
pub fn list_providers(config: &AetherConfig) -> Vec<OpenAIProvider> {
    let mut providers = builtin_providers();
    for custom in &config.providers.custom {
        providers.push(custom_provider(custom));
    }
    providers
}

/// Look up a provider by id.
pub fn get_provider(id: &str, config: &AetherConfig) -> Option<OpenAIProvider> {
    list_providers(config).into_iter().find(|p| p.id == id)
}

fn custom_provider(cfg: &CustomProviderConfig) -> OpenAIProvider {
    let default_model = cfg
        .default_model
        .clone()
        .or_else(|| cfg.models.first().cloned())
        .unwrap_or_default();
    OpenAIProvider {
        id: cfg.name.clone(),
        name: cfg.name.clone(),
        kind: "custom".to_string(),
        base_url: cfg.base_url.clone(),
        key_env: cfg.api_key_env.clone(),
        needs_key: false,
        free: Pricing::Paid,
        description: "Custom OpenAI-compatible endpoint".to_string(),
        default_model,
        static_models: cfg.models.clone(),
    }
}

/// Key status shown by the `providers` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Set,
    NotSet,
    Local,
    None,
}

impl KeyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::NotSet => "not-set",
            Self::Local => "local",
            Self::None => "none",
        }
    }
}

/// Whether the provider's key is present in the environment.
pub fn key_is_set(provider: &OpenAIProvider) -> bool {
    if !provider.needs_key {
        return true;
    }
    match provider.key_env.as_deref() {
        Some(env) => std::env::var(env).map(|v| !v.is_empty()).unwrap_or(false),
        None => false,
    }
}

/// Key status for the `providers` command.
pub fn key_status(provider: &OpenAIProvider) -> KeyStatus {
    if provider.kind == "ollama" {
        return KeyStatus::Local;
    }
    if provider.key_env.is_none() {
        return KeyStatus::None;
    }
    if key_is_set(provider) {
        KeyStatus::Set
    } else {
        KeyStatus::NotSet
    }
}

/// The resolved provider + model after applying fallbacks.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider: OpenAIProvider,
    pub model: String,
    pub notice: Option<String>,
}

/// Resolve the effective provider and model, applying the same graceful
/// fallbacks as the TS `resolveDefault`:
/// - unknown provider -> zen
/// - provider needs a key that is not set -> zen
/// - paid zen model without `OPENCODE_ZEN_API_KEY` -> free zen model
/// - model not in the provider's known list -> provider default
pub fn resolve_default(
    config: &AetherConfig,
    override_provider: Option<&str>,
    override_model: Option<&str>,
) -> ResolvedModel {
    let wanted_provider = override_provider.unwrap_or(&config.default_provider);
    let wanted_model = override_model
        .map(str::to_string)
        .unwrap_or_else(|| config.default_model.clone());

    let Some(provider) = get_provider(wanted_provider, config) else {
        let zen = zen_provider();
        let model = if wanted_model.is_empty() {
            zen.default_model.clone()
        } else {
            wanted_model
        };
        return ResolvedModel {
            provider: zen,
            model,
            notice: Some(format!(
                "provider \"{wanted_provider}\" is not configured; using zen"
            )),
        };
    };

    if provider.needs_key && !key_is_set(&provider) {
        let zen = zen_provider();
        let model = if wanted_model.is_empty() {
            zen.default_model.clone()
        } else {
            wanted_model
        };
        let env = provider.key_env.as_deref().unwrap_or("API key");
        return ResolvedModel {
            provider: zen,
            model,
            notice: Some(format!(
                "{} has no {env} set; falling back to zen",
                provider.id
            )),
        };
    }

    if provider.kind == "zen"
        && !is_zen_free_model(&wanted_model)
        && std::env::var("OPENCODE_ZEN_API_KEY").is_err()
    {
        let model = provider.default_model.clone();
        let notice = format!(
            "model \"{wanted_model}\" on zen needs OPENCODE_ZEN_API_KEY; using free \"{model}\""
        );
        return ResolvedModel {
            provider,
            model,
            notice: Some(notice),
        };
    }

    if provider.kind != "zen"
        && !provider.static_models.is_empty()
        && !provider.static_models.contains(&wanted_model)
    {
        let chosen = if provider.default_model.is_empty() {
            provider.static_models[0].clone()
        } else {
            provider.default_model.clone()
        };
        if chosen != wanted_model {
            let notice = format!(
                "model \"{wanted_model}\" not available on {}; using \"{chosen}\"",
                provider.id
            );
            return ResolvedModel {
                provider,
                model: chosen,
                notice: Some(notice),
            };
        }
    }

    ResolvedModel {
        provider,
        model: wanted_model,
        notice: None,
    }
}

/// Provider-aware default normalization (mirrors `normalizeDefaults`).
/// Returns `true` if the config was mutated.
pub fn normalize_defaults(config: &mut AetherConfig) -> bool {
    let Some(provider) = get_provider(&config.default_provider, config) else {
        config.default_provider = DEFAULT_PROVIDER.to_string();
        config.default_model = DEFAULT_MODEL.to_string();
        return true;
    };
    if provider.kind != "zen"
        && !provider.static_models.is_empty()
        && !provider.static_models.contains(&config.default_model)
    {
        config.default_model = if provider.default_model.is_empty() {
            provider.static_models[0].clone()
        } else {
            provider.default_model.clone()
        };
        return true;
    }
    false
}

/// Fetch the live zen model list (10-minute cache would come with a cache
/// layer; Phase 0 fetches directly and falls back to static on failure).
pub async fn fetch_zen_models() -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(8))
        .user_agent("aether")
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;
    let response = client
        .get(ZEN_MODELS_URL)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Provider(format!(
            "zen models endpoint returned HTTP {}",
            response.status()
        )));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    let ids = extract_model_ids(&value);
    if ids.is_empty() {
        return Err(Error::Provider(
            "zen models endpoint returned no models".to_string(),
        ));
    }
    Ok(ids)
}

/// Extract model ids from the three accepted zen response shapes.
fn extract_model_ids(value: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    let arrays = [
        value.as_array(),
        value.get("data").and_then(serde_json::Value::as_array),
        value.get("models").and_then(serde_json::Value::as_array),
    ];
    for array in arrays.into_iter().flatten() {
        for entry in array {
            if let Some(s) = entry.as_str() {
                ids.push(s.to_string());
            } else if let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::config::AetherConfig;

    fn config() -> AetherConfig {
        AetherConfig::default()
    }

    #[test]
    fn resolve_default_uses_config() {
        let mut c = config();
        c.default_provider = "zen".to_string();
        c.default_model = "deepseek-v4-flash-free".to_string();
        let resolved = resolve_default(&c, None, None);
        assert_eq!(resolved.provider.id, "zen");
        assert_eq!(resolved.model, "deepseek-v4-flash-free");
        assert!(resolved.notice.is_none());
    }

    #[test]
    fn resolve_default_falls_back_on_unknown_provider() {
        let mut c = config();
        c.default_provider = "ghost".to_string();
        let resolved = resolve_default(&c, None, None);
        assert_eq!(resolved.provider.id, "zen");
        assert!(resolved.notice.is_some());
    }

    #[test]
    fn resolve_default_falls_back_when_key_missing() {
        // openai needs OPENAI_API_KEY; absent -> zen.
        let mut c = config();
        c.default_provider = "openai".to_string();
        c.default_model = "gpt-4o".to_string();
        // SAFETY: test-only; single-threaded test, no concurrent env access.
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        let resolved = resolve_default(&c, None, None);
        assert_eq!(resolved.provider.id, "zen");
        assert!(resolved.notice.is_some());
    }

    #[test]
    fn normalize_defaults_resets_unknown_provider() {
        let mut c = config();
        c.default_provider = "ghost".to_string();
        assert!(normalize_defaults(&mut c));
        assert_eq!(c.default_provider, DEFAULT_PROVIDER);
        assert_eq!(c.default_model, DEFAULT_MODEL);
    }

    #[test]
    fn extract_zen_shapes() {
        let arr = serde_json::json!(["a", "b"]);
        assert_eq!(
            extract_model_ids(&arr),
            vec!["a".to_string(), "b".to_string()]
        );
        let obj = serde_json::json!({"data": [{"id": "x"}, {"id": "y"}]});
        assert_eq!(
            extract_model_ids(&obj),
            vec!["x".to_string(), "y".to_string()]
        );
        let obj2 = serde_json::json!({"models": ["m1"]});
        assert_eq!(extract_model_ids(&obj2), vec!["m1".to_string()]);
    }
}
