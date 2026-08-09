//! System prompt building. Mirrors `aether-cli/src/prompt.ts` with the
//! file-override layers of jcode: a project `system-prompt.md` fully
//! replaces the built-in, and a `prompt-overlay.md` appends guidance.

use std::path::{Path, PathBuf};

use crate::config::config_dir;

/// The shared system prompt, byte-for-byte identical to the TS source.
pub const SYSTEM_PROMPT: &str = "You are aether, a terminal coding agent. Be concise, precise, and professional. Never use emojis. Prefer short, direct answers. When asked to write code, write complete, working code.";

/// Prompt builder with layered file overrides.
///
/// Layer precedence, from highest to lowest:
/// 1. `<project>/.aether/system-prompt.md` — full replacement (project scope)
/// 2. `<config>/system-prompt.md` — full replacement (user scope)
/// 3. Built-in [`SYSTEM_PROMPT`]
///
/// When a replacement exists, `prompt-overlay.md` (project then user scope)
/// is appended so users can add guidance without copying the whole prompt.
pub struct Prompt;

impl Prompt {
    /// Build the effective system prompt for the current directory:
    /// replacement (if any) + overlay (if any).
    pub fn build() -> String {
        Self::build_in(Path::new("."), config_dir().ok())
    }

    /// Build the system prompt with additional instructions appended
    /// (over and above any file overlay).
    pub fn build_with(extra: &str) -> String {
        let extra = extra.trim();
        if extra.is_empty() {
            Self::build()
        } else {
            format!("{}\n\nAdditional instructions:\n{extra}", Self::build())
        }
    }

    /// Core builder with injectable project/config dirs (testable).
    pub(crate) fn build_in(project: &Path, config: Option<PathBuf>) -> String {
        let base = Self::load_base(project, config.as_deref());
        let overlay = Self::load_overlay(project, config.as_deref())
            .map(|o| format!("\n\n{o}"))
            .unwrap_or_default();
        format!("{base}{overlay}")
    }

    /// The base prompt: first replacement file found, else the built-in.
    fn load_base(project: &Path, config: Option<&Path>) -> String {
        Self::replacement_candidates(project, config)
            .into_iter()
            .find_map(|p| read_nonempty(&p))
            .unwrap_or_else(|| SYSTEM_PROMPT.to_string())
    }

    /// Optional overlay text: first overlay file found, else none.
    fn load_overlay(project: &Path, config: Option<&Path>) -> Option<String> {
        Self::overlay_candidates(project, config)
            .into_iter()
            .find_map(|p| read_nonempty(&p))
    }

    /// Candidate replacement files, most specific first.
    fn replacement_candidates(project: &Path, config: Option<&Path>) -> Vec<PathBuf> {
        let mut paths = vec![project.join(".aether/system-prompt.md")];
        if let Some(dir) = config {
            paths.push(dir.join("system-prompt.md"));
        }
        paths
    }

    /// Candidate overlay files, most specific first.
    fn overlay_candidates(project: &Path, config: Option<&Path>) -> Vec<PathBuf> {
        let mut paths = vec![project.join(".aether/prompt-overlay.md")];
        if let Some(dir) = config {
            paths.push(dir.join("prompt-overlay.md"));
        }
        paths
    }
}

/// Read a file as trimmed text if it exists and is non-empty.
fn read_nonempty(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-prompt-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_project(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn build_returns_shared_prompt_without_files() {
        assert_eq!(Prompt::build(), SYSTEM_PROMPT);
    }

    #[test]
    fn build_with_appends_instructions() {
        let out = Prompt::build_with("  be terse  ");
        assert!(out.ends_with("be terse"));
    }

    #[test]
    fn project_replacement_wins() {
        let dir = temp_dir("repl");
        write_project(&dir, ".aether/system-prompt.md", "You are a custom agent.");
        assert_eq!(Prompt::build_in(&dir, None), "You are a custom agent.");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlay_appends_after_base() {
        let dir = temp_dir("ov");
        write_project(&dir, ".aether/prompt-overlay.md", "Always cite files.");
        let out = Prompt::build_in(&dir, None);
        assert!(out.starts_with(SYSTEM_PROMPT));
        assert!(out.ends_with("Always cite files."));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_replacement_beats_builtin_project_only_overlay() {
        let project = temp_dir("proj");
        let config = temp_dir("conf");
        write_project(&config, "system-prompt.md", "You are the user-scope agent.");
        write_project(&project, ".aether/prompt-overlay.md", "Be terse.");
        let out = Prompt::build_in(&project, Some(config.clone()));
        assert!(out.starts_with("You are the user-scope agent."));
        assert!(out.ends_with("Be terse."));
        let _ = fs::remove_dir_all(&project);
        let _ = fs::remove_dir_all(&config);
    }

    #[test]
    fn empty_file_is_treated_as_absent() {
        let dir = temp_dir("empty");
        write_project(&dir, ".aether/system-prompt.md", "   ");
        assert_eq!(Prompt::build_in(&dir, None), SYSTEM_PROMPT);
        let _ = fs::remove_dir_all(&dir);
    }
}
