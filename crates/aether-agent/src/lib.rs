//! aether-agent: the 3-model loop (plan -> build -> route) with sandboxed
//! tool execution. The CLI wires an [`Agent`] to a provider client.

pub mod agent;
pub mod plan;
pub mod prompts;
pub mod route;
pub mod tools;

pub use agent::{Agent, AgentResult, Completions};
pub use plan::Plan;
pub use route::Verdict;
pub use tools::Tools;
