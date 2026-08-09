//! aether-core — shared types, config, prompt building, and fs utilities.
//!
//! This crate has no network or async dependencies; it is the leaf that
//! every other aether crate builds on.

pub mod config;
pub mod error;
pub mod fs;
pub mod prompt;

pub use config::{AetherConfig, CustomProviderConfig, DEFAULT_MODEL, DEFAULT_PROVIDER, Providers};
pub use error::{Error, Result, redact_secret};
pub use prompt::{Prompt, SYSTEM_PROMPT};
