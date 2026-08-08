//! aetherdz-tui — placeholder for the Phase 4 ratatui terminal UI.
//!
//! Phase 0 ships only the trait surface so downstream crates can code
//! against a stable interface. The full ratatui + crossterm implementation
//! (mouse wheel, ctrl+p palette, live token ledger) arrives in Phase 4.

use thiserror::Error;

/// Errors produced by a TUI implementation.
#[derive(Debug, Error)]
pub enum TuiError {
    /// The terminal could not be initialized or restored.
    #[error("terminal error: {0}")]
    Terminal(String),
    /// Rendering failed.
    #[error("render error: {0}")]
    Render(String),
}

/// The TUI contract. Phase 0 defines the surface; Phase 4 provides the
/// ratatui implementation.
pub trait Tui {
    /// Run the TUI event loop until it returns.
    fn run(&mut self) -> Result<(), TuiError>;
}

/// A no-op TUI used before the real implementation lands. It exists so the
/// trait has a concrete, testable implementation.
#[derive(Debug, Default)]
pub struct NoopTui;

impl Tui for NoopTui {
    fn run(&mut self) -> Result<(), TuiError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_tui_runs() {
        let mut tui = NoopTui;
        assert!(tui.run().is_ok());
    }
}
