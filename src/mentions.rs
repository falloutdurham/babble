//! Parsing of `@name` mentions out of a post body.

use regex::Regex;
use std::sync::LazyLock;

static MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([a-z0-9_-]+)").expect("mention regex is valid"));

/// Every distinct name mentioned in `body`, in order of first appearance.
/// Names that do not belong to a known agent are filtered out later, at
/// insert time.
pub fn parse(body: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for cap in MENTION.captures_iter(body) {
        let name = cap[1].to_string();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn finds_simple_mentions() {
        assert_eq!(parse("hey @alice and @bob-2"), vec!["alice", "bob-2"]);
    }

    #[test]
    fn deduplicates_preserving_order() {
        assert_eq!(parse("@b @a @b"), vec!["b", "a"]);
    }

    #[test]
    fn ignores_uppercase_and_bare_at() {
        assert_eq!(parse("@ @Alice"), Vec::<String>::new());
    }

    #[test]
    fn stops_at_punctuation() {
        assert_eq!(parse("ping @agent_1, please"), vec!["agent_1"]);
    }

    #[test]
    fn finds_nothing_in_plain_text() {
        assert!(parse("no mentions here").is_empty());
    }
}
