//! Live observation of the 3-model loop (Phase 4, "agent screen").
//!
//! The agent loop runs in its own task; a [`TuiObserver`]-style consumer in
//! another thread needs to know what phase the loop is in *while* it runs,
//! not just when it finishes. This module exposes a tiny synchronous
//! callback surface the loop calls at each phase transition:
//!
//! ```text
//! Planning -> PlanReady
//!   └─ per iteration: BuildStarted -> ToolCalled* -> BuildFinished
//!        -> Routing -> Routed
//! Finished
//! ```
//!
//! Consumers implement [`AgentObserver`] (e.g. pushing into an `mpsc`
//! channel that a TUI event loop drains). The observer is never awaited, so
//! a slow consumer cannot stall the loop — callbacks are fire-and-forget.

use std::sync::Arc;

use crate::agent::AgentStopReason;

/// One transition of the 3-model loop, reported live to observers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPhase {
    /// The plan model is producing the initial plan.
    Planning,
    /// The initial plan was produced (text form, for display).
    PlanReady(String),
    /// A build round started.
    BuildStarted { iteration: u32 },
    /// A tool call was executed inside a build round.
    /// `diff` carries the rendered +/- preview for `write_file` calls
    /// (a file-changing tool); `None` for every other tool.
    ToolCalled {
        iteration: u32,
        name: String,
        diff: Option<String>,
    },
    /// A build round finished, with its summary and stats.
    BuildFinished {
        iteration: u32,
        tool_calls: u32,
        modified: bool,
        summary: String,
    },
    /// The route model is judging the build round.
    Routing { iteration: u32 },
    /// The route model returned a verdict.
    Routed {
        iteration: u32,
        verdict: VerdictPhase,
    },
    /// The loop terminated.
    Finished {
        iterations: u32,
        tool_calls: u32,
        reason: AgentStopReason,
    },
}

/// Lightweight, `Send`-friendly view of a [`crate::route::Verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictPhase {
    Done,
    Continue,
    Revise,
}

/// Receives live phase transitions from an agent loop.
///
/// Implementations must be cheap and never block: the loop calls
/// [`AgentObserver::on_phase`] synchronously at each transition.
pub trait AgentObserver: Send + Sync {
    /// Called at every phase transition of the loop.
    fn on_phase(&self, phase: AgentPhase);
}

/// An observer that does nothing — the default for [`crate::Agent`].
pub struct NoopObserver;

impl AgentObserver for NoopObserver {
    fn on_phase(&self, _phase: AgentPhase) {}
}

/// An observer that forwards every phase to a closure.
pub struct FnObserver<F>(pub F)
where
    F: Fn(AgentPhase) + Send + Sync;

impl<F> AgentObserver for FnObserver<F>
where
    F: Fn(AgentPhase) + Send + Sync,
{
    fn on_phase(&self, phase: AgentPhase) {
        (self.0)(phase);
    }
}

/// Convenience: an observer that forwards to a `std::sync::mpsc::Sender`.
pub struct ChannelObserver(pub std::sync::mpsc::Sender<AgentPhase>);

impl AgentObserver for ChannelObserver {
    fn on_phase(&self, phase: AgentPhase) {
        // Fire-and-forget: a full channel must never stall the agent loop.
        let _ = self.0.send(phase);
    }
}

/// Type-erased observer handle stored on [`crate::Agent`].
pub type DynObserver = Arc<dyn AgentObserver>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_observer_accepts_all_phases() {
        let obs = NoopObserver;
        obs.on_phase(AgentPhase::Planning);
        obs.on_phase(AgentPhase::Finished {
            iterations: 3,
            tool_calls: 7,
            reason: AgentStopReason::Done,
        });
    }

    #[test]
    fn fn_observer_receives_phases() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let obs = FnObserver(move |p| seen2.lock().unwrap().push(p));
        obs.on_phase(AgentPhase::PlanReady("plan".into()));
        let got = seen.lock().unwrap().clone();
        assert_eq!(got, vec![AgentPhase::PlanReady("plan".into())]);
    }

    #[test]
    fn channel_observer_forwards_in_order() {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let obs = ChannelObserver(tx);
            obs.on_phase(AgentPhase::Planning);
            obs.on_phase(AgentPhase::BuildStarted { iteration: 1 });
        }
        let got: Vec<AgentPhase> = rx.iter().collect();
        assert_eq!(
            got,
            vec![
                AgentPhase::Planning,
                AgentPhase::BuildStarted { iteration: 1 }
            ]
        );
    }
}
