//! Plan model output: a task decomposed into ordered steps.

use aether_core::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: u32,
    pub action: String,
}

/// The full plan produced by the plan model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub steps: Vec<Step>,
}

/// Extract the first balanced JSON object from a model reply.
///
/// Tolerates fenced code blocks and prose around the JSON, which is the
/// realistic output of instruct-tuned models.
pub fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a `Plan` from a model reply (optionally inside ```json fences).
pub fn parse_plan(text: &str) -> Result<Plan> {
    let json = extract_json_object(text)
        .ok_or_else(|| Error::InvalidInput("plan reply contains no JSON object".to_string()))?;
    let plan: Plan = serde_json::from_str(json)
        .map_err(|e| Error::InvalidInput(format!("plan JSON invalid: {e}")))?;
    if plan.steps.is_empty() {
        return Err(Error::InvalidInput("plan has no steps".to_string()));
    }
    for step in &plan.steps {
        if step.action.trim().is_empty() {
            return Err(Error::InvalidInput(format!(
                "step {} has no action",
                step.id
            )));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_json() {
        let text = r#"Here is the plan:
{"goal":"ship","steps":[{"id":1,"action":"build"},{"id":2,"action":"test"}]}
Done."#;
        let plan = parse_plan(text).unwrap();
        assert_eq!(plan.goal, "ship");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[1].action, "test");
    }

    #[test]
    fn extracts_from_fenced_block() {
        let text = "```json\n{\"goal\":\"g\",\"steps\":[{\"id\":1,\"action\":\"a\"}]}\n```";
        let plan = parse_plan(text).unwrap();
        assert_eq!(plan.steps[0].id, 1);
    }

    #[test]
    fn rejects_no_json() {
        assert!(parse_plan("no plan here, sorry").is_err());
    }

    #[test]
    fn rejects_empty_steps() {
        assert!(parse_plan(r#"{"goal":"g","steps":[]}"#).is_err());
    }

    #[test]
    fn rejects_nested_object_early_close() {
        // A `}` inside a string must not close the object.
        let text = r#"{"goal":"a}","steps":[{"id":1,"action":"x"}]}"#;
        let plan = parse_plan(text).unwrap();
        assert_eq!(plan.goal, "a}");
    }
}
