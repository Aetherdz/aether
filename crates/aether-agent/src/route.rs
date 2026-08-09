//! Router model output: verdict on whether the build is done, needs to
//! continue, or requires a revised plan.

use aether_core::error::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::plan::Plan;

/// What the router decides after a build round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Build succeeded; finish with the final answer.
    Done(String),
    /// Keep executing the current plan.
    Continue(String),
    /// Replace the plan and keep going.
    Revise(Plan, String),
}

/// Wire shape of the router's JSON reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteReply {
    pub verdict: String,
    #[serde(default)]
    pub feedback: String,
    #[serde(default)]
    pub revised_plan: Option<Plan>,
    #[serde(default)]
    pub final_answer: Option<String>,
}

/// Parse a router verdict from the model reply.
pub fn parse_verdict(text: &str) -> Result<Verdict> {
    let json = crate::plan::extract_json_object(text)
        .ok_or_else(|| Error::InvalidInput("router reply contains no JSON object".to_string()))?;
    let reply: RouteReply = serde_json::from_str(json)
        .map_err(|e| Error::InvalidInput(format!("router JSON invalid: {e}")))?;
    match reply.verdict.as_str() {
        "done" => Ok(Verdict::Done(
            reply
                .final_answer
                .or(Some(reply.feedback))
                .unwrap_or_default(),
        )),
        "continue" => Ok(Verdict::Continue(reply.feedback)),
        "revise" => {
            let plan = reply
                .revised_plan
                .ok_or_else(|| Error::InvalidInput("revise verdict missing revised_plan".to_string()))?;
            if plan.steps.is_empty() {
                return Err(Error::InvalidInput("revised plan has no steps".to_string()));
            }
            Ok(Verdict::Revise(plan, reply.feedback))
        }
        other => Err(Error::InvalidInput(format!("unknown verdict \"{other}\""))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_done_with_answer() {
        let v = parse_verdict(r#"{"verdict":"done","final_answer":"all green"}"#).unwrap();
        assert_eq!(v, Verdict::Done("all green".to_string()));
    }

    #[test]
    fn parses_continue() {
        let v = parse_verdict(r#"{"verdict":"continue","feedback":"more"}"#).unwrap();
        assert_eq!(v, Verdict::Continue("more".to_string()));
    }

    #[test]
    fn parses_revise_with_plan() {
        let text = r#"{"verdict":"revise","feedback":"wrong approach",
            "revised_plan":{"goal":"g2","steps":[{"id":1,"action":"b"}]}}"#;
        let v = parse_verdict(text).unwrap();
        match v {
            Verdict::Revise(plan, fb) => {
                assert_eq!(plan.goal, "g2");
                assert_eq!(fb, "wrong approach");
            }
            other => panic!("expected Revise, got {other:?}"),
        }
    }

    #[test]
    fn revises_without_plan_is_error() {
        assert!(parse_verdict(r#"{"verdict":"revise","feedback":"x"}"#).is_err());
    }

    #[test]
    fn done_falls_back_to_feedback() {
        let v = parse_verdict(r#"{"verdict":"done","feedback":"fallback"}"#).unwrap();
        assert_eq!(v, Verdict::Done("fallback".to_string()));
    }
}
