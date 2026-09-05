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

/// A reaction has to stay a reaction. Rejecting ASCII letters and digits keeps
/// the field from becoming a second, unattributed comment box, while leaving
/// every real emoji — including multi-codepoint ones like a skin-toned thumb or
/// a flag — perfectly usable.
pub fn emoji(emoji: &str) -> Result<(), Invalid> {
    if emoji.is_empty() {
        return Err("a reaction cannot be empty".into());
    }
    if emoji.len() > api::MAX_EMOJI_LEN {
        return Err(format!(
            "a reaction must be at most {} bytes",
            api::MAX_EMOJI_LEN
        ));
    }
    if emoji.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("a reaction must be an emoji, not text".into());
    }
    if emoji.chars().any(char::is_whitespace) {
        return Err("a reaction must not contain whitespace".into());
    }
    if emoji.chars().any(char::is_control) {
        return Err("a reaction must not contain control characters".into());
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
    fn reactions_must_be_symbols() {
        for good in [
            "\u{1f44d}",
            "\u{2764}\u{fe0f}",
            "\u{1f44d}\u{1f3fd}",
            "\u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}",
            "\u{2705}",
        ] {
            assert!(emoji(good).is_ok(), "rejected {good}");
        }
        for bad in [
            "",
            "lgtm",
            ":+1:",
            "\u{1f44d} \u{1f44e}",
            "\u{1f44d}\n",
            "1",
        ] {
            assert!(emoji(bad).is_err(), "accepted {bad:?}");
        }
        assert!(emoji(&"\u{1f44d}".repeat(20)).is_err());
    }

    #[test]
    fn tag_lists_are_bounded() {
        assert!(tags(&["a".to_string()]).is_ok());
        assert!(tags(&vec!["x".to_string(); 11]).is_err());
        assert!(tags(&["".to_string()]).is_err());
    }
}
