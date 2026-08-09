//! aether-core — shared types, config, prompt building, and fs utilities.
//!
//! This crate has no network or async dependencies; it is the leaf that
//! every other aether crate builds on.

pub mod config;
pub mod error;
pub mod fs;
pub mod prompt;

pub use config::{
    AetherConfig, CustomProviderConfig, Providers, DEFAULT_MODEL, DEFAULT_PROVIDER,
};
pub use error::{redact_secret, Error, Result};
pub use prompt::{Prompt, SYSTEM_PROMPT};
