//! Model-response parsing: tolerate code fences and surrounding prose, then
//! demand valid JSON between the first `{` and the last `}`. No auto-retry —
//! the error carries a sample and the caller decides.

use serde::Deserialize;

use crate::EngineError;

#[derive(Debug, Deserialize)]
pub struct RawGroups {
    #[serde(default)]
    pub groups: Vec<RawGroup>,
}

#[derive(Debug, Deserialize)]
pub struct RawGroup {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub classes: Vec<String>,
    /// Unknown values fall back to "focus" downstream (when in doubt, focus).
    #[serde(default = "default_effort")]
    pub effort: String,
    #[serde(default)]
    pub reason: String,
}

fn default_effort() -> String {
    "focus".to_string()
}

/// The span the grouping parser reads: the first `{` to the last `}`,
/// inclusive, whatever prose or fence surrounds it. `None` when the text has
/// no such pair.
///
/// Public because `dfr agents --probe` judges an agent's reply by exactly
/// this rule. A probe that read more leniently would pass an agent the
/// pipeline then rejects, minutes in.
pub fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end >= start).then(|| &text[start..=end])
}

pub fn parse_response(text: &str) -> Result<RawGroups, EngineError> {
    let Some(span) = json_object(text) else {
        return Err(err("no JSON object in response", text));
    };
    serde_json::from_str(span).map_err(|e| err(&e.to_string(), text))
}

fn err(msg: &str, text: &str) -> EngineError {
    let sample: String = text.chars().take(300).collect();
    EngineError::GroupingParse {
        msg: msg.to_string(),
        sample,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json_parses() {
        let r = parse_response(r#"{"groups": [{"label": "x", "classes": ["C0"]}]}"#).unwrap();
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].effort, "focus");
    }

    #[test]
    fn fenced_json_parses() {
        let r = parse_response(
            "```json\n{\"groups\": [{\"label\": \"x\", \"classes\": [\"C0\"], \"effort\": \"skim\"}]}\n```",
        )
        .unwrap();
        assert_eq!(r.groups[0].effort, "skim");
    }

    #[test]
    fn prose_wrapped_json_parses() {
        let r = parse_response(
            "Here is the grouping you asked for:\n{\"groups\": []}\nHope that helps!",
        )
        .unwrap();
        assert!(r.groups.is_empty());
    }

    #[test]
    fn the_object_span_is_inclusive_and_ignores_what_surrounds_it() {
        assert_eq!(json_object("```json\n{\"a\": 1}\n```"), Some("{\"a\": 1}"));
        assert_eq!(
            json_object("x {\"a\": {\"b\": 2}} y"),
            Some("{\"a\": {\"b\": 2}}")
        );
        assert_eq!(json_object("no braces"), None);
        assert_eq!(json_object("} before {"), None);
    }

    #[test]
    fn garbage_errors_with_sample() {
        match parse_response("I cannot help with that.") {
            Err(EngineError::GroupingParse { sample, .. }) => {
                assert!(sample.contains("cannot help"));
            }
            other => panic!("expected GroupingParse, got {other:?}"),
        }
    }
}
