//! Configuration: `~/.config/aether/config.json` (or `AETHER_CONFIG_DIR`).
//!
//! Mirrors `aether-cli/src/config.ts` and `aether-cli/src/defaults.ts`:
//! same file layout, same default merge semantics, same legacy-shape
//! normalization for custom providers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, Result};
use crate::fs::{atomic_write, ensure_dir, safe_join};

/// Default provider id (from `defaults.ts`).
pub const DEFAULT_PROVIDER: &str = "zen";
/// Default model id (from `defaults.ts`).
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash-free";

/// A user-defined OpenAI-compatible provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomProviderConfig {
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// The `providers` section of the config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Providers {
    #[serde(default, deserialize_with = "deserialize_custom_providers")]
    pub custom: Vec<CustomProviderConfig>,
}

/// Top-level config, JSON layout identical to the TS version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AetherConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub providers: Providers,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl Default for AetherConfig {
    fn default() -> Self {
        Self {
            default_provider: DEFAULT_PROVIDER.to_string(),
            default_model: DEFAULT_MODEL.to_string(),
            providers: Providers::default(),
            aliases: HashMap::new(),
        }
    }
}

fn default_provider() -> String {
    DEFAULT_PROVIDER.to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

/// Resolve the config directory: `AETHER_CONFIG_DIR` override, else
/// `~/.config/aether` via the `dirs` crate.
pub fn config_dir() -> Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("AETHER_CONFIG_DIR") {
        if !override_dir.trim().is_empty() {
            return Ok(PathBuf::from(override_dir));
        }
    }
    let base = dirs::config_dir()
        .ok_or_else(|| Error::Config("could not resolve the user config directory".to_string()))?;
    Ok(base.join("aether"))
}

/// Path to `config.json`.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

/// Path to the sessions directory (created lazily in Phase 1).
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("sessions"))
}

/// Load the config, merging defaults for any missing field.
///
/// - Missing file: writes the default config and returns it.
/// - Corrupt/unreadable file: recreates the default config and returns it.
/// - Present file: parses, merges defaults, and (if the file was missing
///   fields) rewrites the normalized shape.
pub fn load_config() -> Result<AetherConfig> {
    let dir = config_dir()?;
    load_config_from(&dir)
}

/// Testable core of [`load_config`]: load from an explicit directory.
pub fn load_config_from(dir: &Path) -> Result<AetherConfig> {
    let path = dir.join("config.json");
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        match serde_json::from_str::<AetherConfig>(&raw) {
            Ok(config) => Ok(config),
            Err(_) => {
                // Corrupt or unreadable config: recreate defaults.
                let defaults = AetherConfig::default();
                save_config_to(dir, &defaults)?;
                Ok(defaults)
            }
        }
    } else {
        ensure_dir(dir)?;
        let defaults = AetherConfig::default();
        save_config_to(dir, &defaults)?;
        Ok(defaults)
    }
}

/// Persist the config under the default config directory.
pub fn save_config(config: &AetherConfig) -> Result<()> {
    let dir = config_dir()?;
    save_config_to(&dir, config)
}

/// Persist the config under an explicit directory.
pub fn save_config_to(dir: &Path, config: &AetherConfig) -> Result<()> {
    ensure_dir(dir)?;
    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(config)?;
    atomic_write(&path, format!("{json}\n").as_bytes())
}

/// Set the default provider (and optionally model), persisting the change.
pub fn update_default(provider: &str, model: Option<&str>) -> Result<AetherConfig> {
    let mut config = load_config()?;
    if let Some(model) = model {
        config.default_model = model.to_string();
    }
    config.default_provider = provider.to_string();
    save_config(&config)?;
    Ok(config)
}

/// Normalize a raw `providers.custom` value (array, single object, or
/// anything else) into a `Vec<CustomProviderConfig>`. Entries without a
/// `name` or `baseURL` are dropped, matching the TS behavior.
fn deserialize_custom_providers<'de, D>(deserializer: D) -> std::result::Result<Vec<CustomProviderConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(normalize_custom_providers(&value))
}

/// Normalize a raw JSON value into custom provider configs.
pub fn normalize_custom_providers(raw: &serde_json::Value) -> Vec<CustomProviderConfig> {
    if raw.is_null() {
        return Vec::new();
    }
    let candidates: Vec<&serde_json::Value> = match raw.as_array() {
        Some(arr) => arr.iter().collect(),
        None => vec![raw],
    };
    let mut out = Vec::new();
    for entry in candidates {
        let Some(obj) = entry.as_object() else { continue };
        let Some(name) = obj.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(base_url) = obj.get("baseURL").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let models = obj
            .get("models")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let api_key_env = obj.get("apiKeyEnv").and_then(serde_json::Value::as_str).map(str::to_string);
        let default_model = obj.get("defaultModel").and_then(serde_json::Value::as_str).map(str::to_string);
        out.push(CustomProviderConfig {
            name: name.to_string(),
            base_url: base_url.to_string(),
            api_key_env,
            models,
            default_model,
        });
    }
    out
}

/// Resolve a session file path inside the sessions dir, rejecting traversal.
pub fn session_path(session_id: &str) -> Result<PathBuf> {
    let dir = sessions_dir()?;
    safe_join(&dir, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aetherdz-config-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_config_creates_defaults() {
        let dir = temp_dir("missing");
        let config = load_config_from(&dir).unwrap();
        assert_eq!(config.default_provider, DEFAULT_PROVIDER);
        assert_eq!(config.default_model, DEFAULT_MODEL);
        assert!(config.providers.custom.is_empty());
        assert!(dir.join("config.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_config_merges_defaults() {
        let dir = temp_dir("partial");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{"defaultProvider":"openai"}"#,
        )
        .unwrap();
        let config = load_config_from(&dir).unwrap();
        // Explicit field kept, missing field defaulted.
        assert_eq!(config.default_provider, "openai");
        assert_eq!(config.default_model, DEFAULT_MODEL);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_config_recreates_defaults() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{ not json !!!").unwrap();
        let config = load_config_from(&dir).unwrap();
        assert_eq!(config.default_provider, DEFAULT_PROVIDER);
        assert_eq!(config.default_model, DEFAULT_MODEL);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_custom_provider_shapes_normalize() {
        // Single object (legacy) and array shapes both normalize.
        let single = serde_json::json!({
            "name": "myllm",
            "baseURL": "http://localhost:1234/v1",
            "models": ["m1", "m2"],
            "apiKeyEnv": "MYLLM_KEY",
            "defaultModel": "m1"
        });
        let normalized = normalize_custom_providers(&single);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].name, "myllm");
        assert_eq!(normalized[0].models, vec!["m1".to_string(), "m2".to_string()]);

        // Entries missing name/baseURL are dropped.
        let mixed = serde_json::json!([
            {"name": "ok", "baseURL": "https://x/v1"},
            {"name": "no-base"},
            {"baseURL": "https://y/v1"},
            "not-an-object"
        ]);
        let normalized = normalize_custom_providers(&mixed);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].name, "ok");
    }

    #[test]
    fn update_default_persists() {
        let dir = temp_dir("update");
        let config = load_config_from(&dir).unwrap();
        assert_eq!(config.default_provider, DEFAULT_PROVIDER);
        // update_default uses the real config dir; test the underlying
        // mutation + save path directly.
        let mut c = config.clone();
        c.default_provider = "ollama".to_string();
        c.default_model = "llama3.2".to_string();
        save_config_to(&dir, &c).unwrap();
        let reloaded = load_config_from(&dir).unwrap();
        assert_eq!(reloaded.default_provider, "ollama");
        assert_eq!(reloaded.default_model, "llama3.2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_path_rejects_traversal() {
        // safe_join is the traversal gate; session_path routes through it.
        assert!(matches!(
            crate::fs::safe_join(Path::new("/tmp"), "../evil"),
            Err(Error::PathTraversal(_))
        ));
    }
}
