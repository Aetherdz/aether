//! Agent screen — a pure, UI-framework-free view of the aether agent loop.
//!
//! [`AgentScreenState`] folds [`AgentPhase`] events coming from the
//! `aether-agent` observer interface into display state, then renders
//! human-readable status lines for the three-panel layout (PLAN / BUILD /
//! ROUTE). It performs no I/O and depends only on `std` plus the observer
//! types from `aether-agent`, so it can be unit-tested without a terminal.
//!
//! # Panels
//!
//! * **PLAN** — status: `(empty)`, `planning…`, or the first 3 plan lines.
//! * **BUILD** — iteration, total tool calls, last tool name, or `waiting`.
//! * **ROUTE** — verdict counters plus the current verdict, or `waiting`.
//! * A trailing `DONE: <reason>` line once the loop has stopped.

use aether_agent::{AgentPhase, AgentStopReason, VerdictPhase};

/// Longest line rendered in a panel before it is truncated with `…`.
const MAX_LINE: usize = 100;

/// Maximum diff lines rendered under the last tool in the BUILD panel.
pub const MAX_DIFF_LINES: usize = 12;

/// How many plan lines are shown in the PLAN panel.
pub const MAX_PLAN_LINES: usize = 3;

/// Tally of route verdicts seen so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerdictCounters {
    pub done: u32,
    pub continue_: u32,
    pub revise: u32,
}

/// Display phase of the screen — a lightweight, `Copy` mirror of
/// [`AgentPhase`] that only needs to drive the panel text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScreenPhase {
    /// No events received yet.
    #[default]
    Idle,
    /// The plan model is producing the initial plan.
    Planning,
    /// The initial plan was produced and is shown.
    PlanReady,
    /// A build round started.
    BuildStarted,
    /// A tool call was executed inside a build round.
    ToolCalled,
    /// A build round finished.
    BuildFinished,
    /// The route model is judging the build round.
    Routing,
    /// The route model returned a verdict.
    Routed,
    /// The loop terminated.
    Finished,
}

/// Live state of the three-panel agent screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentScreenState {
    /// Display phase (mirrors the latest [`AgentPhase`]).
    pub phase: ScreenPhase,
    /// Full plan text from the latest [`AgentPhase::PlanReady`].
    pub plan_text: String,
    /// Iteration number of the latest build/route round.
    pub current_iteration: u32,
    /// Total tool calls reported by the latest build round.
    pub total_tool_calls: u32,
    /// Name of the most recent tool call.
    pub last_tool_name: Option<String>,
    /// Rendered +/- diff of the most recent `write_file` call, if any.
    pub last_diff: Option<String>,
    /// Summary text from the latest build round.
    pub last_summary: String,
    /// Verdict tally across all routed rounds.
    pub verdict_counters: VerdictCounters,
    /// Verdict of the most recent routed round.
    pub current_verdict: Option<VerdictPhase>,
    /// Whether the loop has terminated.
    pub stopped: bool,
    /// Why the loop terminated (set together with `stopped`).
    pub stop_reason: Option<AgentStopReason>,
}

impl AgentScreenState {
    /// Fold one loop event into the screen state.
    pub fn apply(&mut self, phase: AgentPhase) {
        match phase {
            AgentPhase::Planning => self.phase = ScreenPhase::Planning,
            AgentPhase::PlanReady(text) => {
                self.phase = ScreenPhase::PlanReady;
                self.plan_text = text;
            }
            AgentPhase::BuildStarted { iteration } => {
                self.phase = ScreenPhase::BuildStarted;
                self.current_iteration = iteration;
            }
            AgentPhase::ToolCalled {
                iteration,
                name,
                diff,
            } => {
                self.phase = ScreenPhase::ToolCalled;
                self.current_iteration = iteration;
                self.last_tool_name = Some(name);
                self.last_diff = diff;
            }
            AgentPhase::BuildFinished {
                iteration,
                tool_calls,
                modified: _,
                summary,
            } => {
                self.phase = ScreenPhase::BuildFinished;
                self.current_iteration = iteration;
                self.total_tool_calls = tool_calls;
                self.last_summary = summary;
            }
            AgentPhase::Routing { iteration } => {
                self.phase = ScreenPhase::Routing;
                self.current_iteration = iteration;
            }
            AgentPhase::Routed { iteration, verdict } => {
                self.phase = ScreenPhase::Routed;
                self.current_iteration = iteration;
                self.current_verdict = Some(verdict);
                match verdict {
                    VerdictPhase::Done => self.verdict_counters.done += 1,
                    VerdictPhase::Continue => self.verdict_counters.continue_ += 1,
                    VerdictPhase::Revise => self.verdict_counters.revise += 1,
                }
            }
            AgentPhase::Finished {
                iterations,
                tool_calls,
                reason,
            } => {
                self.phase = ScreenPhase::Finished;
                self.current_iteration = iterations;
                self.total_tool_calls = tool_calls;
                self.stopped = true;
                self.stop_reason = Some(reason);
            }
        }
    }

    /// Render the three panels as human-readable lines.
    ///
    /// The result is a flat list of lines; the caller may split it into
    /// panels on the `PLAN` / `BUILD` / `ROUTE` headers.
    pub fn status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        // ---- PLAN panel ----
        lines.push("PLAN".to_string());
        if self.plan_text.is_empty() {
            match self.phase {
                ScreenPhase::Planning => lines.push("  planning…".to_string()),
                _ => lines.push("  (empty)".to_string()),
            }
        } else {
            for plan_line in self.plan_text.lines().take(MAX_PLAN_LINES) {
                lines.push(format!("  {}", truncate(plan_line, MAX_LINE)));
            }
        }

        // ---- BUILD panel ----
        lines.push("BUILD".to_string());
        if self.current_iteration == 0 && self.last_tool_name.is_none() {
            lines.push("  waiting".to_string());
        } else {
            lines.push(format!("  iteration: {}", self.current_iteration));
            lines.push(format!("  tool calls: {}", self.total_tool_calls));
            match &self.last_tool_name {
                Some(name) => lines.push(format!("  last tool: {}", truncate(name, MAX_LINE))),
                None => lines.push("  last tool: —".to_string()),
            }
            if let Some(diff) = &self.last_diff {
                for line in diff.lines().take(MAX_DIFF_LINES) {
                    lines.push(format!("    {}", truncate(line, MAX_LINE)));
                }
            }
        }

        // ---- ROUTE panel ----
        lines.push("ROUTE".to_string());
        lines.push(format!(
            "  done: {}  continue: {}  revise: {}",
            self.verdict_counters.done,
            self.verdict_counters.continue_,
            self.verdict_counters.revise
        ));
        match self.current_verdict {
            Some(verdict) => lines.push(format!("  current: {}", verdict_label(verdict))),
            None => lines.push("  current: waiting".to_string()),
        }

        // ---- Final status ----
        if let Some(reason) = self.stop_reason {
            lines.push(format!("DONE: {}", stop_reason_label(reason)));
        }

        lines
    }
}

/// Human-readable label for a route verdict.
pub fn verdict_label(verdict: VerdictPhase) -> &'static str {
    match verdict {
        VerdictPhase::Done => "Done",
        VerdictPhase::Continue => "Continue",
        VerdictPhase::Revise => "Revise",
    }
}

/// Human-readable label for a stop reason.
pub fn stop_reason_label(reason: AgentStopReason) -> &'static str {
    match reason {
        AgentStopReason::Done => "Done",
        AgentStopReason::IterationCap => "IterationCap",
        AgentStopReason::Stagnation => "Stagnation",
        AgentStopReason::BuildTurnCap => "BuildTurnCap",
    }
}

/// Truncate `text` to at most `max` characters, appending `…` when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_shows_empty_panels() {
        let state = AgentScreenState::default();
        let lines = state.status_lines();

        assert!(lines.iter().any(|l| l == "PLAN"));
        assert!(lines.iter().any(|l| l == "  (empty)"));
        assert!(lines.iter().any(|l| l == "BUILD"));
        assert!(lines.iter().any(|l| l == "  waiting"));
        assert!(lines.iter().any(|l| l == "ROUTE"));
        assert!(
            !lines.iter().any(|l| l.starts_with("DONE:")),
            "no DONE line while idle"
        );

        assert_eq!(state.phase, ScreenPhase::Idle);
        assert!(!state.stopped);
        assert_eq!(state.plan_text, "");
        assert_eq!(state.current_iteration, 0);
        assert_eq!(state.total_tool_calls, 0);
        assert!(state.last_tool_name.is_none());
    }

    #[test]
    fn planning_then_plan_ready_sets_plan_and_shows_truncated_plan() {
        let mut state = AgentScreenState::default();

        state.apply(AgentPhase::Planning);
        assert_eq!(state.phase, ScreenPhase::Planning);
        assert!(state.status_lines().iter().any(|l| l == "  planning…"));

        state.apply(AgentPhase::PlanReady(
            "goal g\nstep1\nstep2\nstep3".to_string(),
        ));
        assert_eq!(state.phase, ScreenPhase::PlanReady);
        assert_eq!(state.plan_text, "goal g\nstep1\nstep2\nstep3");

        let lines = state.status_lines();
        assert!(lines.iter().any(|l| l == "  goal g"));
        assert!(lines.iter().any(|l| l == "  step1"));
        assert!(lines.iter().any(|l| l == "  step2"));
        assert!(
            !lines.iter().any(|l| l == "  step3"),
            "only first 3 plan lines shown"
        );
    }

    #[test]
    fn long_plan_lines_are_truncated() {
        let mut state = AgentScreenState::default();
        let long_line = "x".repeat(120);
        state.apply(AgentPhase::PlanReady(format!("{long_line}\nshort")));

        let lines = state.status_lines();
        let long = lines
            .iter()
            .find(|l| l.ends_with('…'))
            .expect("long line truncated with …");
        assert!(long.starts_with("  "));
        assert_eq!(
            long.chars().count(),
            MAX_LINE + 2,
            "2-space prefix + 100-char line"
        );
        assert!(lines.iter().any(|l| l == "  short"));
    }

    #[test]
    fn build_round_updates_build_panel() {
        let mut state = AgentScreenState::default();

        state.apply(AgentPhase::BuildStarted { iteration: 1 });
        assert_eq!(state.current_iteration, 1);

        state.apply(AgentPhase::ToolCalled {
            iteration: 1,
            name: "read_file".to_string(),
            diff: None,
        });
        assert_eq!(state.last_tool_name.as_deref(), Some("read_file"));
        assert_eq!(state.last_diff, None);

        state.apply(AgentPhase::BuildFinished {
            iteration: 1,
            tool_calls: 3,
            modified: false,
            summary: "done work".to_string(),
        });
        assert_eq!(state.current_iteration, 1);
        assert_eq!(state.total_tool_calls, 3);
        assert_eq!(state.last_tool_name.as_deref(), Some("read_file"));
        assert_eq!(state.last_summary, "done work");

        let lines = state.status_lines();
        assert!(lines.iter().any(|l| l == "  iteration: 1"));
        assert!(lines.iter().any(|l| l == "  tool calls: 3"));
        assert!(lines.iter().any(|l| l == "  last tool: read_file"));
        assert!(!lines.iter().any(|l| l == "  waiting"));
    }

    #[test]
    fn write_diff_is_rendered_in_build_panel() {
        let mut state = AgentScreenState::default();
        state.apply(AgentPhase::ToolCalled {
            iteration: 1,
            name: "write_file".to_string(),
            diff: Some("  old line\n+ new line\n".to_string()),
        });
        let lines = state.status_lines();
        assert!(lines.iter().any(|l| l == "    + new line"));
        assert!(lines.iter().any(|l| l == "      old line"));
        let lines = state.status_lines();
        assert_eq!(lines.iter().filter(|l| l.starts_with("    ")).count(), 2);
    }

    #[test]
    fn routed_increments_verdict_counters() {
        let mut state = AgentScreenState::default();

        state.apply(AgentPhase::Routed {
            iteration: 1,
            verdict: VerdictPhase::Done,
        });
        assert_eq!(state.verdict_counters.done, 1);
        assert_eq!(state.verdict_counters.continue_, 0);
        assert_eq!(state.verdict_counters.revise, 0);
        assert_eq!(state.current_verdict, Some(VerdictPhase::Done));
        assert!(state.status_lines().iter().any(|l| l == "  current: Done"));

        state.apply(AgentPhase::Routed {
            iteration: 2,
            verdict: VerdictPhase::Revise,
        });
        assert_eq!(state.verdict_counters.done, 1);
        assert_eq!(state.verdict_counters.continue_, 0);
        assert_eq!(state.verdict_counters.revise, 1);
        assert_eq!(state.current_verdict, Some(VerdictPhase::Revise));
        assert!(
            state
                .status_lines()
                .iter()
                .any(|l| l == "  done: 1  continue: 0  revise: 1")
        );
        assert!(
            state
                .status_lines()
                .iter()
                .any(|l| l == "  current: Revise")
        );
    }

    #[test]
    fn finished_sets_stopped_and_reason() {
        let mut state = AgentScreenState::default();
        state.apply(AgentPhase::Finished {
            iterations: 3,
            tool_calls: 5,
            reason: AgentStopReason::IterationCap,
        });

        assert!(state.stopped);
        assert_eq!(state.stop_reason, Some(AgentStopReason::IterationCap));
        assert_eq!(state.current_iteration, 3);
        assert_eq!(state.total_tool_calls, 5);
        assert!(
            state
                .status_lines()
                .iter()
                .any(|l| l == "DONE: IterationCap")
        );
    }
}
