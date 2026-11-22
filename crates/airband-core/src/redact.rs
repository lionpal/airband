//! Keys and tokens that turn up in what is said are not kept or shown: a GitHub token pasted to an agent, an
//! API key in a log an agent quotes. What an agent is given is not touched; this is for the copy the chat
//! keeps, which is stored on the tower, sent to every paired radio and read by a model.
//!
//! A credential is known by its shape, which its issuer fixes: a prefix and a run of characters of a known
//! kind and length. That is not a judgement about meaning, so it is done here by rule and not by a model.
//! Something secret with no such shape (a password in a sentence) is not caught.

const DOTS: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// One shape a credential comes in, as its issuer fixes it: a row of the catalogue (`catalog/secrets/`). Rows
/// are tried in order, so a longer prefix comes before a shorter one that starts it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Shape {
    pub id: String,
    pub prefix: String,
    /// How the body after the prefix is built.
    #[serde(default)]
    pub form: Form,
    /// The characters its body is made of besides letters and digits.
    #[serde(default)]
    pub extra: String,
    /// How many characters its body (each part, for a dotted one) has at least; for a block, how soon after
    /// the prefix its `head` must come.
    pub least: usize,
    /// For a block: what follows the prefix to make it one, kept as it is.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub head: String,
    /// For a block: the marks that end it, found in this order. Everything up to the last is masked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub end: Vec<String>,
}

/// How a credential's body is built.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Form {
    /// One run of letters, digits and `extra`, starting a word.
    #[default]
    Run,
    /// One run of hexadecimal digits, wherever it is.
    Hex,
    /// Three runs of letters, digits, `-` and `_` joined by dots, the prefix part of the first, starting a word.
    Dotted,
    /// Lines from the prefix to its end marks, wherever it is.
    Block,
}

/// `text` with every credential in it replaced by its prefix and a row of dots.
pub fn redact(text: &str) -> String {
    let shapes = &crate::catalog::built().secrets;
    // nothing that could start one: the usual case, and no copy is made for it beyond the return
    if !shapes.iter().any(|s| text.contains(s.prefix.as_str())) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        // only at the start of a word: "task-force" holds no key
        let at_word = out.chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        for shape in shapes {
            if let Some((kept, skip)) = matched(shape, rest, at_word) {
                out.push_str(&rest[..kept]);
                out.push_str(DOTS);
                rest = &rest[skip..];
                continue 'scan;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Where `rest` starts a credential of this shape: how much of it is kept as it is, and how much the
/// credential takes.
fn matched(shape: &Shape, rest: &str, at_word: bool) -> Option<(usize, usize)> {
    let body = rest.strip_prefix(shape.prefix.as_str())?;
    let p = shape.prefix.len();
    let run = |s: &str, also: &[u8]| s.bytes().take_while(|b| b.is_ascii_alphanumeric() || also.contains(b)).count();
    match shape.form {
        Form::Run if at_word => Some(run(body, shape.extra.as_bytes())).filter(|n| *n >= shape.least).map(|n| (p, p + n)),
        Form::Hex => Some(body.bytes().take_while(u8::is_ascii_hexdigit).count()).filter(|n| *n >= shape.least).map(|n| (p, p + n)),
        Form::Dotted if at_word => {
            let a = p + run(body, b"-_");
            let b = a + 1 + run(rest.get(a + 1..)?, b"-_");
            let c = b + 1 + run(rest.get(b + 1..)?, b"-_");
            let long = |from: usize, to: usize| to.saturating_sub(from) >= shape.least;
            (long(0, a) && rest[a..].starts_with('.') && long(a + 1, b) && rest[b..].starts_with('.') && long(b + 1, c)).then_some((p, c))
        }
        Form::Block => {
            let head = body.find(shape.head.as_str()).filter(|i| *i < shape.least)?;
            let mut at = p + head + shape.head.len();
            let kept = at;
            for mark in &shape.end {
                at = match rest[at..].find(mark.as_str()) {
                    Some(i) => at + i + mark.len(),
                    None => return Some((kept, rest.len())),
                };
            }
            Some((kept, at))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_masked_and_nothing_else_is() {
        let token = format!("ghp_{}", "a1B2".repeat(9));
        assert_eq!(redact(&format!("use {token} for the push")), format!("use ghp_{DOTS} for the push"));
        assert_eq!(redact(&format!("key=\"AKIA{}\"", "ABCD1234EFGH5678")), format!("key=\"AKIA{DOTS}\""));
        assert_eq!(redact(&format!("sk_live_{} and sk-ant-{}", "x".repeat(24), "y-z_".repeat(8))), format!("sk_live_{DOTS} and sk-ant-{DOTS}"));
        let pem = "-----BEGIN PRIVATE KEY-----\nMIGTAgEAMBMGByqGSM49\nAgEGCCqGSM49\n-----END PRIVATE KEY-----\nafter";
        assert_eq!(redact(pem), format!("-----BEGIN PRIVATE KEY-----{DOTS}\nafter"));
        assert_eq!(redact(&format!("airband://pair?relay=x&secret={}", "ab".repeat(32))), format!("airband://pair?relay=x&secret={DOTS}"));
        assert_eq!(redact(&format!("eyJ{}.{}.{} ok", "a".repeat(20), "b".repeat(30), "c".repeat(40))), format!("eyJ{DOTS} ok"));
        // ordinary words that only look a little like one
        for plain in ["the task-force sk-8 skis", "ghp_ is how GitHub tokens start", "risk-free and mask_test_ok", "an AKIA prefix alone", "Run the payments tests", "passé: café ghp_short"] {
            assert_eq!(redact(plain), plain);
        }
    }
}
