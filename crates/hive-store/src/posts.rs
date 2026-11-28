use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

/// A marker line by which a yes on a post would run a command (`pair:`,
/// `onboard:`, `lead-role:`, `toolkit:`, `goal:`): the owner's yes on a post
/// carrying one authorizes that command (ADR-021).
static COMMAND_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(pair|onboard|lead-role|toolkit|goal):[ \t]*\S").unwrap()
});

/// The first line of `text` that is a command marker ([`COMMAND_MARKER`]),
/// trimmed, or None. The board refuses an agent's yes on such a post, and a
/// goal draft must never carry one: the owner's Accept would authorize it.
pub fn command_marker(text: &str) -> Option<String> {
    let m = COMMAND_MARKER.find(text)?;
    let start = text[..m.start()].rfind('\n').map_or(0, |i| i + 1);
    let end = text[m.start()..]
        .find('\n')
        .map_or(text.len(), |i| m.start() + i);
    Some(text[start..end].trim().to_string())
}

pub fn post_for_owner(extra: &str) -> Result<bool> {
    let v: Value = serde_json::from_str(extra).map_err(|e| anyhow!("post extra: {e}"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow!("post extra must be a JSON object"))?;
    let Some(flag) = obj.get("for_owner") else {
        return Ok(false);
    };
    match flag {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(anyhow!("post extra: for_owner must be true or false")),
        },
        _ => Err(anyhow!("post extra: for_owner must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_marker_finds_the_line_a_yes_would_authorize() {
        assert_eq!(
            super::command_marker("Make it fast.\n  pair: box\nWhy: x").as_deref(),
            Some("pair: box")
        );
        assert_eq!(
            super::command_marker("goal: amend\nx").as_deref(),
            Some("goal: amend")
        );
        assert!(super::command_marker("onboard: shop").is_some());
        assert!(super::command_marker("toolkit:\tadd").is_some());
        // Mid-line, bare, or another word: no marker.
        assert_eq!(super::command_marker("We pair: boxes later."), None);
        assert_eq!(super::command_marker("pair:"), None);
        assert_eq!(super::command_marker("Pair: box"), None);
        assert_eq!(super::command_marker("Measure: weekly"), None);
    }

    #[test]
    fn post_for_owner() {
        let _serial = crate::test_helpers::serial();
        for (name, extra, want, want_error) in [
            ("missing", r#"{}"#, false, false),
            (
                "unrelated metadata",
                r#"{"mentions":["human"],"ticket":"owner-review"}"#,
                false,
                false,
            ),
            ("true", r#"{"for_owner":true}"#, true, false),
            ("false", r#"{"for_owner":false}"#, false, false),
            ("frontmatter true", r#"{"for_owner":"true"}"#, true, false),
            (
                "frontmatter false",
                r#"{"for_owner":"false"}"#,
                false,
                false,
            ),
            ("normalized true", r#"{"for_owner":" TRUE "}"#, true, false),
            (
                "normalized false",
                r#"{"for_owner":" FALSE "}"#,
                false,
                false,
            ),
            ("number", r#"{"for_owner":1}"#, false, true),
            ("zero", r#"{"for_owner":0}"#, false, true),
            ("null flag", r#"{"for_owner":null}"#, false, true),
            ("object", r#"{"for_owner":{}}"#, false, true),
            ("array flag", r#"{"for_owner":[]}"#, false, true),
            ("invalid text", r#"{"for_owner":"yes"}"#, false, true),
            ("empty text", r#"{"for_owner":""}"#, false, true),
            ("invalid json", r#"{"for_owner":"#, false, true),
            ("array metadata", r#"[]"#, false, true),
        ] {
            let res = super::post_for_owner(extra);
            let got = *res.as_ref().unwrap_or(&false);
            assert!(
                res.is_err() == want_error && got == want,
                "{name}: PostForOwner({extra}) = {got}, {res:?}; want {want}, error={want_error}"
            );
        }
    }
}
