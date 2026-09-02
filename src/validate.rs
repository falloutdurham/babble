//! Input validation shared by the server handlers.

use crate::api;
use regex::Regex;
use std::sync::LazyLock;

static AGENT_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_-]{1,32}$").expect("agent name regex is valid"));

/// Human-readable reason a value was rejected.
pub type Invalid = String;

pub fn agent_name(name: &str) -> Result<(), Invalid> {
    if AGENT_NAME.is_match(name) {
        Ok(())
    } else {
        Err(format!(
            "agent name must match [a-z0-9_-]{{1,{}}}",
            api::MAX_NAME_LEN
        ))
    }
}

pub fn title(title: &str) -> Result<(), Invalid> {
    let title = title.trim();
    if title.is_empty() {
        return Err("title must not be empty".into());
    }
    if title.chars().count() > api::MAX_TITLE_LEN {
        return Err(format!(
            "title must be at most {} characters",
            api::MAX_TITLE_LEN
        ));
    }
    Ok(())
}

pub fn body(body: &str) -> Result<(), Invalid> {
    if body.trim().is_empty() {
        return Err("body must not be empty".into());
    }
    if body.len() > api::MAX_BODY_LEN {
        return Err(format!("body must be at most {} bytes", api::MAX_BODY_LEN));
    }
    Ok(())
}

pub fn tags(tags: &[String]) -> Result<(), Invalid> {
    if tags.len() > api::MAX_TAGS {
        return Err(format!("at most {} tags are allowed", api::MAX_TAGS));
    }
    for tag in tags {
        if tag.is_empty() || tag.chars().count() > api::MAX_TAG_LEN {
            return Err(format!(
                "each tag must be 1-{} characters",
                api::MAX_TAG_LEN
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_follow_the_charset() {
        assert!(agent_name("alice").is_ok());
        assert!(agent_name("agent-1_x").is_ok());
        assert!(agent_name("Alice").is_err());
        assert!(agent_name("").is_err());
        assert!(agent_name(&"a".repeat(33)).is_err());
    }

    #[test]
    fn titles_and_bodies_are_bounded() {
        assert!(title("hello").is_ok());
        assert!(title("   ").is_err());
        assert!(title(&"t".repeat(201)).is_err());
        assert!(body(&"b".repeat(api::MAX_BODY_LEN)).is_ok());
        assert!(body(&"b".repeat(api::MAX_BODY_LEN + 1)).is_err());
    }

    #[test]
    fn tag_lists_are_bounded() {
        assert!(tags(&["a".to_string()]).is_ok());
        assert!(tags(&vec!["x".to_string(); 11]).is_err());
        assert!(tags(&["".to_string()]).is_err());
    }
}
