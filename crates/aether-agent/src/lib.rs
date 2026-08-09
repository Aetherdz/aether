//! aether-agent: the 3-model loop (plan -> build -> route) with sandboxed
//! tool execution. The CLI wires an [`Agent`] to a provider client.
//!
//! Safety features:
//! - [`tools`] sandboxes every path and snapshots replaced files
//! - [`undo`] persists snapshots under `.aether-undo/` for post-run restore
//! - [`agent`] stops early on stagnation and reports why via
//!   [`AgentStopReason`]

pub mod agent;
pub mod diff;
pub mod plan;
pub mod prompts;
pub mod route;
pub mod tools;
pub mod undo;

pub use agent::{Agent, AgentResult, AgentStopReason, Completions};
pub use diff::{DiffLine, DiffLineKind, diff_lines, render_diff};
pub use plan::Plan;
pub use route::Verdict;
pub use tools::{ConfirmPolicy, ToolResult, Tools};
pub use undo::{RestoredSnapshot, SnapshotMeta, UNDO_DIR_NAME, UndoJournal};
