//! The `Provider` trait and the concrete OpenAI-compatible provider struct.

use aether_core::error::Result;

use crate::client::OpenAICompatibleClient;

/// Pricing tier, used by the `providers` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pricing {
    Free,
    Paid,
    FreePaid,
}

impl Pricing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Paid => "paid",
            Self::FreePaid => "free/paid",
        }
    }
}

/// A chat provider. Phase 0 providers all speak the OpenAI-compatible wire
/// format; the trait exists so later phases can add non-compatible providers
/// (Anthropic, Google) behind the same surface.
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn kind(&self) -> &str;
    fn base_url(&self) -> &str;
    fn key_env(&self) -> Option<&str>;
    fn needs_key(&self) -> bool;
    fn free(&self) -> Pricing;
    fn description(&self) -> &str;
    fn default_model(&self) -> &str;
    fn static_models(&self) -> &[String];

    /// Build a streaming client for this provider, reading the API key from
    /// the environment when one is required.
    fn client(&self) -> Result<OpenAICompatibleClient>;
}

/// Concrete OpenAI-compatible provider.
#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub key_env: Option<String>,
    pub needs_key: bool,
    pub free: Pricing,
    pub description: String,
    pub default_model: String,
    pub static_models: Vec<String>,
}

impl Provider for OpenAIProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &str {
        &self.kind
    }
    fn base_url(&self) -> &str {
        &self.base_url
    }
    fn key_env(&self) -> Option<&str> {
        self.key_env.as_deref()
    }
    fn needs_key(&self) -> bool {
        self.needs_key
    }
    fn free(&self) -> Pricing {
        self.free
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn default_model(&self) -> &str {
        &self.default_model
    }
    fn static_models(&self) -> &[String] {
        &self.static_models
    }

    fn client(&self) -> Result<OpenAICompatibleClient> {
        let api_key = self
            .key_env
            .as_deref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|v| !v.is_empty());
        OpenAICompatibleClient::new(self.base_url.clone(), api_key)
    }
}
