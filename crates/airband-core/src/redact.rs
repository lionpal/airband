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
    /// The characters its body is made of besides letters and digits.
    #[serde(default)]
    pub extra: String,
    /// How many characters its body has at least.
    pub least: usize,
}

/// `text` with every credential in it replaced by its prefix and a row of dots.
pub fn redact(text: &str) -> String {
    let shapes = &crate::catalog::built().secrets;
    // nothing that could start one: the usual case, and no copy is made for it beyond the return
    if !shapes.iter().any(|s| text.contains(s.prefix.as_str())) && !text.contains("PRIVATE KEY-----") && !text.contains("secret=") && !text.contains("eyJ") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        // a private key, from its first line to its last
        if rest.starts_with("-----BEGIN ")
            && let Some(head) = rest.find("PRIVATE KEY-----").filter(|i| *i < 40) {
                let end = rest.find("-----END ").and_then(|e| rest[e..].find("KEY-----").map(|k| e + k + 8)).unwrap_or(rest.len());
                out.push_str(&rest[..head + 16]);
                out.push_str(DOTS);
                rest = &rest[end..];
                continue;
            }
        // only at the start of a word: "task-force" holds no key
        let at_word = out.chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if at_word {
            for Shape { prefix, extra, least, .. } in shapes {
                if let Some(body) = rest.strip_prefix(prefix.as_str()) {
                    let n = body.bytes().take_while(|b| b.is_ascii_alphanumeric() || extra.as_bytes().contains(b)).count();
                    if n >= *least {
                        out.push_str(prefix);
                        out.push_str(DOTS);
                        rest = &body[n..];
                        continue 'scan;
                    }
                }
            }
            // a signed token: three runs of base64 joined by dots, the first of them a JSON header
            if rest.starts_with("eyJ") {
                let run = |s: &str| s.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_').count();
                let a = run(rest);
                if a >= 10 && rest[a..].starts_with('.') {
                    let b = run(&rest[a + 1..]);
                    if b >= 10 && rest[a + 1 + b..].starts_with('.') {
                        let c = run(&rest[a + b + 2..]);
                        if c >= 10 {
                            out.push_str("eyJ");
                            out.push_str(DOTS);
                            rest = &rest[a + b + c + 2..];
                            continue;
                        }
                    }
                }
            }
        }
        // a pairing link's secret: the link is the key to his agents
        if let Some(body) = rest.strip_prefix("secret=") {
            let n = body.bytes().take_while(|b| b.is_ascii_hexdigit()).count();
            if n == 64 {
                out.push_str("secret=");
                out.push_str(DOTS);
                rest = &body[n..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
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
