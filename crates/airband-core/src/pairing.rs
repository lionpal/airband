//! The pairing link: one string that tells a radio how to reach its tower, and what that computer is called.
//! Hosts store it and hand it to the radio unread; they know nothing about routes.
//!
//!   airband://pair?relay=https%3A%2F%2Frelay.example&secret=<64 hex>     through a relay, from anywhere
//!   airband://pair?secret=<64 hex>&at=http%3A%2F%2F192.168.1.20%3A7373   sealed the same way, with no relay: straight
//!                                                                        to the tower at its own addresses (several,
//!                                                                        with commas), on its network or a private one
//!   airband://pair?url=http%3A%2F%2F127.0.0.1%3A7373&token=<token>       unsealed, to a tower on this same machine
//!
//! Every link ends `&name=<the computer's name>`: a phone calls the computer that from the moment it is paired.
//! A computer whose system gives its model's identifier adds it (`&model=Mac16,10`): the phone shows the
//! computer's shape from it (`catalog::shape_of`). One whose system gives none adds nothing.

/// Its debug form says where it reaches and never the secret or the token.
#[derive(Clone, PartialEq, Eq)]
pub enum Pairing {
    Relay { relay: String, secret: String },
    /// No relay: every request goes straight to the tower, sealed with the keys the secret gives. `at` is where
    /// the tower was when the link was made; it says where it is now each time it is asked.
    Near { at: Vec<String>, secret: String },
    Direct { url: String, token: String },
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::redact::Shown;
        match self {
            Pairing::Relay { relay, secret } => f.debug_struct("Relay").field("relay", relay).field("secret", &Shown(secret)).finish(),
            Pairing::Near { at, secret } => f.debug_struct("Near").field("at", at).field("secret", &Shown(secret)).finish(),
            Pairing::Direct { url, token } => f.debug_struct("Direct").field("url", url).field("token", &Shown(token)).finish(),
        }
    }
}

/// Why a string is not a pairing link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotALink {
    /// It does not start `airband://pair?`.
    NotOne,
    /// It does not say how to reach the computer.
    NoWay,
    /// It does not say what the computer is called.
    NoName,
}

impl std::fmt::Display for NotALink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NotALink::NotOne => "a pairing link starts with airband://pair?",
            NotALink::NoWay => "that is not a whole pairing link: copy it again from your computer, or scan its code",
            NotALink::NoName => "that pairing link does not say what its computer is called: pair again from your computer",
        })
    }
}

/// A link's query, each value encoded: written with the same rules it is read by.
fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
}

impl Pairing {
    /// How a link reaches its computer. A link that does not also say what the computer is called is not one.
    pub fn parse(link: &str) -> Result<Pairing, NotALink> {
        let query = link.trim().strip_prefix("airband://pair?").ok_or(NotALink::NotOne)?;
        let get = |name: &str| url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned()).filter(|v| !v.is_empty());
        let tidy = |u: String| u.trim_end_matches('/').to_string();
        let at = get("at").map(|at| at.split(',').map(|a| tidy(a.to_string())).filter(|a| !a.is_empty()).collect::<Vec<_>>()).filter(|at| !at.is_empty());
        let pairing = match (get("relay"), get("secret"), get("url"), get("token")) {
            (Some(relay), Some(secret), _, _) => Pairing::Relay { relay: tidy(relay), secret },
            (None, Some(secret), _, _) if at.is_some() => Pairing::Near { at: at.unwrap_or_default(), secret },
            (_, _, Some(url), Some(token)) => Pairing::Direct { url: tidy(url), token },
            _ => return Err(NotALink::NoWay),
        };
        get("name").ok_or(NotALink::NoName)?;
        Ok(pairing)
    }
    /// What the computer a link pairs with is called: what a phone calls it from the moment it is paired.
    pub fn computer(link: &str) -> Result<String, NotALink> {
        Pairing::parse(link)?;
        let query = link.trim().strip_prefix("airband://pair?").ok_or(NotALink::NotOne)?;
        url::form_urlencoded::parse(query.as_bytes()).find(|(k, v)| k == "name" && !v.is_empty()).map(|(_, v)| v.into_owned()).ok_or(NotALink::NoName)
    }
    /// The identifier of the model of the computer a link pairs with, as its system gave it. Empty when the link
    /// carries none.
    pub fn model(link: &str) -> String {
        let query = link.trim().strip_prefix("airband://pair?").unwrap_or_default();
        url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "model").map(|(_, v)| v.into_owned()).unwrap_or_default()
    }
    /// The link to the computer called `computer`, whose model goes by the identifier `model` (empty when its
    /// system gives none). A computer with no name has no link: Err.
    pub fn link(&self, computer: &str, model: &str) -> Result<String, NotALink> {
        if computer.trim().is_empty() {
            return Err(NotALink::NoName);
        }
        let way = match self {
            Pairing::Relay { relay, secret } => query(&[("relay", relay), ("secret", secret)]),
            Pairing::Near { at, secret } => query(&[("secret", secret), ("at", &at.join(","))]),
            Pairing::Direct { url, token } => query(&[("url", url), ("token", token)]),
        };
        let model = if model.is_empty() { String::new() } else { format!("&{}", query(&[("model", model)])) };
        Ok(format!("airband://pair?{way}{model}&{}", query(&[("name", computer)])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for p in [Pairing::Relay { relay: "https://relay.example".into(), secret: "ab".repeat(32) }, Pairing::Near { at: vec!["http://192.168.1.20:7373".into(), "http://100.64.0.7:7373".into()], secret: "cd".repeat(32) }, Pairing::Direct { url: "http://100.64.0.7:7373".into(), token: "t0k-en".into() }] {
            let link = p.link("Studio Mac", "Mac16,9").unwrap();
            assert_eq!(Pairing::parse(&link).unwrap(), p);
            assert_eq!(Pairing::computer(&link).unwrap(), "Studio Mac");
            // the model goes and comes back exactly as the system gave it
            assert_eq!(Pairing::model(&link), "Mac16,9");
            assert_eq!(crate::catalog::shape_of(&Pairing::model(&link)), Some(crate::catalog::Shape::Studio));
            // a computer whose system gives no model: the link carries none, and says nothing of a shape
            let plain = p.link("Studio Mac", "").unwrap();
            assert!(!plain.contains("model="));
            assert_eq!((Pairing::parse(&plain).unwrap(), Pairing::model(&plain)), (p.clone(), String::new()));
        }
        assert_eq!(Pairing::parse("airband://pair?url=http%3A%2F%2Fx%3A1%2F&token=a&name=x").unwrap(), Pairing::Direct { url: "http://x:1".into(), token: "a".into() });
    }
    #[test]
    fn a_link_always_says_what_its_computer_is_called() {
        let relay = Pairing::Relay { relay: "https://relay.example".into(), secret: "ab".repeat(32) };
        assert_eq!(relay.link(" ", "Mac16,10"), Err(NotALink::NoName));
        let nameless = format!("airband://pair?relay=https%3A%2F%2Frelay.example&secret={}", "ab".repeat(32));
        assert_eq!(Pairing::parse(&nameless), Err(NotALink::NoName));
        assert_eq!(Pairing::computer(&nameless), Err(NotALink::NoName));
        // where a link reaches its computer is not its name
        let direct = "airband://pair?url=http%3A%2F%2Fstudio.local%3A7373&token=t";
        assert_eq!(Pairing::computer(direct), Err(NotALink::NoName));
    }
    #[test]
    fn refuses_what_is_not_a_pairing() {
        assert_eq!(Pairing::parse("https://example.com/?relay=a&secret=b&name=x"), Err(NotALink::NotOne));
        assert_eq!(Pairing::parse("airband://pair?relay=https%3A%2F%2Fr&name=x"), Err(NotALink::NoWay));
        // a secret alone reaches nothing
        assert_eq!(Pairing::parse(&format!("airband://pair?secret={}&name=x", "ab".repeat(32))), Err(NotALink::NoWay));
        assert_eq!(Pairing::parse("airband://pair?url=%ZZ&token=&name=x"), Err(NotALink::NoWay));
    }
}
