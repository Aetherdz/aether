//! System prompt building. Mirrors `aether-cli/src/prompt.ts`.

/// The shared system prompt, byte-for-byte identical to the TS source.
pub const SYSTEM_PROMPT: &str = "You are aether, a terminal coding agent. Be concise, precise, and professional. Never use emojis. Prefer short, direct answers. When asked to write code, write complete, working code.";

/// Prompt builder. Phase 0 only needs the shared system prompt; the builder
/// exists so later phases can layer agent profiles or extra instructions.
pub struct Prompt;

impl Prompt {
    /// Build the default system prompt.
    pub fn build() -> String {
        SYSTEM_PROMPT.to_string()
    }

    /// Build the system prompt with additional instructions appended.
    pub fn build_with(extra: &str) -> String {
        let extra = extra.trim();
        if extra.is_empty() {
            Self::build()
        } else {
            format!("{SYSTEM_PROMPT}\n\nAdditional instructions:\n{extra}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Prompt, SYSTEM_PROMPT};

    #[test]
    fn build_returns_shared_prompt() {
        assert_eq!(Prompt::build(), SYSTEM_PROMPT);
    }

    #[test]
    fn build_with_appends_instructions() {
        let out = Prompt::build_with("  be terse  ");
        assert!(out.starts_with(SYSTEM_PROMPT));
        assert!(out.ends_with("be terse"));
    }
}
