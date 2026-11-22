//! The pairing link: one string that tells a radio how to reach its tower. Hosts store it and hand it to the
//! radio unread; they know nothing about routes.
//!
//!   airband://pair?relay=https%3A%2F%2Frelay.example&secret=<64 hex>     through a relay, from anywhere
//!   airband://pair?secret=<64 hex>&at=http%3A%2F%2F192.168.1.20%3A7373   sealed the same way, with no relay: straight
//!                                                                        to the tower at its own addresses (several,
//!                                                                        with commas), on its network or a private one
//!   airband://pair?url=http%3A%2F%2F127.0.0.1%3A7373&token=<token>       unsealed, to a tower on this same machine

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pairing {
    Relay { relay: String, secret: String },
    /// No relay: every request goes straight to the tower, sealed with the keys the secret gives. `at` is where
    /// the tower was when the link was made; it says where it is now each time it is asked.
    Near { at: Vec<String>, secret: String },
    Direct { url: String, token: String },
}

/// A link's query, each value encoded: written with the same rules it is read by.
fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
}

impl Pairing {
    pub fn parse(link: &str) -> Result<Pairing, String> {
        let query = link.trim().strip_prefix("airband://pair?").ok_or("a pairing link starts with airband://pair?")?;
        let get = |name: &str| url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned()).filter(|v| !v.is_empty());
        let tidy = |u: String| u.trim_end_matches('/').to_string();
        let at = get("at").map(|at| at.split(',').map(|a| tidy(a.to_string())).filter(|a| !a.is_empty()).collect::<Vec<_>>()).filter(|at| !at.is_empty());
        match (get("relay"), get("secret"), get("url"), get("token")) {
            (Some(relay), Some(secret), _, _) => Ok(Pairing::Relay { relay: tidy(relay), secret }),
            (None, Some(secret), _, _) if at.is_some() => Ok(Pairing::Near { at: at.unwrap_or_default(), secret }),
            (_, _, Some(url), Some(token)) => Ok(Pairing::Direct { url: tidy(url), token }),
            _ => Err("that is not a whole pairing link: copy it again from your computer, or scan its code".into()),
        }
    }
    pub fn link(&self) -> String {
        match self {
            Pairing::Relay { relay, secret } => format!("airband://pair?{}", query(&[("relay", relay), ("secret", secret)])),
            Pairing::Near { at, secret } => format!("airband://pair?{}", query(&[("secret", secret), ("at", &at.join(","))])),
            Pairing::Direct { url, token } => format!("airband://pair?{}", query(&[("url", url), ("token", token)])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for p in [Pairing::Relay { relay: "https://relay.example".into(), secret: "ab".repeat(32) }, Pairing::Near { at: vec!["http://192.168.1.20:7373".into(), "http://100.64.0.7:7373".into()], secret: "cd".repeat(32) }, Pairing::Direct { url: "http://100.64.0.7:7373".into(), token: "t0k-en".into() }] {
            assert_eq!(Pairing::parse(&p.link()).unwrap(), p);
        }
        assert_eq!(Pairing::parse("airband://pair?url=http%3A%2F%2Fx%3A1%2F&token=a").unwrap(), Pairing::Direct { url: "http://x:1".into(), token: "a".into() });
    }
    #[test]
    fn refuses_what_is_not_a_pairing() {
        assert!(Pairing::parse("https://example.com/?relay=a&secret=b").is_err());
        assert!(Pairing::parse("airband://pair?relay=https%3A%2F%2Fr").is_err());
        // a secret alone reaches nothing
        assert!(Pairing::parse(&format!("airband://pair?secret={}", "ab".repeat(32))).is_err());
        assert!(Pairing::parse("airband://pair?url=%ZZ&token=").is_err());
    }
}
