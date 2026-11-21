//! Making written text speakable.
use regex::{Captures, Regex};
use std::sync::LazyLock;

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static TICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]*)`").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w.@~-]*(?:/[\w.@-]+)+").unwrap());
static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\w{1,5}$").unwrap());
static LEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*[#>*-]+\s*").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static BLOCKY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"```|\n\s*[-*|#]|\n.*\n.*\n").unwrap());

/// Strip what cannot be said aloud: code blocks, markdown marks, links, and all of a path but its last name.
pub fn spoken(text: &str) -> String {
    let t = FENCE.replace_all(text, " ");
    let t = TICK.replace_all(&t, "$1");
    let t = LINK.replace_all(&t, "$1");
    let t = URL.replace_all(&t, "a link");
    let t = PATH.replace_all(&t, |c: &Captures| {
        let p = &c[0];
        let last = p.rsplit('/').next().unwrap_or(p);
        // "and/or" is not a path: a path has two slashes, or ends in a file extension
        if p.matches('/').count() < 2 && !EXT.is_match(last) {
            return p.to_string();
        }
        EXT.replace(last, "").replace(['-', '_'], " ")
    });
    let t = LEAD.replace_all(&t, "");
    let t = t.replace(['*', '~', '|'], "").replace('_', " ");
    SPACE.replace_all(&t, " ").trim().to_string()
}

/// True when a message is short plain prose that can be read out as written, with no model in between.
pub fn reads_as_written(text: &str, max_chars: usize) -> bool {
    text.chars().count() <= max_chars && !BLOCKY.is_match(text) && !PATH.find_iter(text).any(|m| m.as_str().matches('/').count() >= 2)
}

static WEB_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`\]\)]+"#).unwrap());

/// The web links in a message, in order, each once, four at most. The short version of a message drops
/// them, so a client shows these beside it: an artifact, a pull request, a preview.
pub fn links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in WEB_LINK.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_']).to_string();
        if url.len() > 10 && !out.contains(&url) && out.len() < 4 {
            out.push(url);
        }
    }
    out
}

/// The first two sentences, for when the summary could not be made.
pub fn opening(text: &str) -> String {
    let s = spoken(text);
    let mut out = String::new();
    let mut ends = 0;
    for (i, ch) in s.char_indices() {
        out.push(ch);
        let boundary = matches!(ch, '.' | '?' | '!') && s[i + ch.len_utf8()..].starts_with(' ');
        if boundary {
            ends += 1;
        }
        if ends == 2 || out.chars().count() >= 300 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_marks_and_paths() {
        assert_eq!(
            spoken("**Done.** See `scripts/check.mjs` and /Users/c/shop/units/mito-content.json\n- one"),
            "Done. See check and mito content one"
        );
    }
    #[test]
    fn leaves_slashed_words() {
        assert_eq!(spoken("rewrite and/or drop [the doc](https://x.y/z)"), "rewrite and/or drop the doc");
    }
    #[test]
    fn drops_code_blocks() {
        assert_eq!(spoken("Ran it:\n```\nnpm test\n```\nAll green."), "Ran it: All green.");
    }
    #[test]
    fn short_prose_reads_as_written() {
        assert!(reads_as_written("Both questions are rewritten. Want me to commit?", 220));
        assert!(!reads_as_written("Done:\n- one\n- two", 220));
        assert!(!reads_as_written("See /Users/c/shop/x.json", 220));
    }
    #[test]
    fn links_are_found_once_without_their_punctuation() {
        let text = "Published: https://claude.ai/artifact/abc123. See [the PR](https://github.com/a/b/pull/7), again https://claude.ai/artifact/abc123 and `http://localhost:3000/x`.";
        assert_eq!(links(text), ["https://claude.ai/artifact/abc123", "https://github.com/a/b/pull/7", "http://localhost:3000/x"]);
        assert!(links("No links, only a path /Users/c/x.png").is_empty());
    }
    #[test]
    fn opening_is_two_sentences() {
        assert_eq!(opening("One. Two? Three. Four."), "One. Two?");
    }
}
