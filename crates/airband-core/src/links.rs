//! The things an agent makes that live at a web address: a pull request, a published page. A client shows
//! each as a card that says what it is and what it is called, and opens it.
//!
//! What a thing is comes only from the tool that made it, which says so in its own records
//! ([`crate::agent::Event::Made`]): never from how an address reads or from the words around it. An address
//! in a message that no tool said anything about is a plain link, shown by the site it is at. Each kind a
//! tool can name is one file under `catalog/links/`: what to call it and the mark to show beside it.
use serde::{Deserialize, Serialize};

/// One kind of thing, as its row in the catalogue has it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LinkKind {
    pub id: String,
    /// What it is called where he sees one: "Pull request".
    pub name: String,
    /// The mark shown beside it, by the name the platform's symbol set knows it by.
    pub symbol: String,
}

/// A web address in a message, and what is known about what is there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Link {
    pub address: String,
    /// The row it is a kind of, by id. Empty for a plain link.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// What that kind is called: "Pull request". Empty for a plain link.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub symbol: String,
    /// The site it is at (`example.com`), for showing a plain link by.
    pub host: String,
    /// What it is called, when the tool that made it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The site an address is at, as the address itself is built: its host, read by the rules for web addresses.
/// Empty when it is not a web address.
pub fn site(address: &str) -> String {
    url::Url::parse(address).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default()
}

/// What is shown for an address. `kind` is what the tool that made it said it is, by the id of a row; with
/// none, or one no row has, it is a plain link.
pub fn link(kinds: &[LinkKind], address: &str, kind: Option<&str>, title: Option<String>) -> Link {
    let host = site(address);
    match kind.and_then(|id| kinds.iter().find(|k| k.id == id)) {
        Some(k) => Link { address: address.to_string(), kind: k.id.clone(), name: k.name.clone(), symbol: k.symbol.clone(), host, title },
        None => Link { address: address.to_string(), host, title, ..Link::default() },
    }
}

/// Is every row one that can be shown: an id, a name and a mark?
pub fn check(kinds: &[LinkKind]) -> Result<(), String> {
    match kinds.iter().find(|k| k.id.is_empty() || k.name.is_empty() || k.symbol.is_empty()) {
        Some(k) => Err(format!("a kind of link without an id, a name or a mark: \"{}\"", k.id)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_a_kind_only_when_its_maker_said_so() {
        let kinds = crate::catalog::built().links.clone();
        check(&kinds).unwrap();
        // however its address reads, nothing said what this is
        let unsaid = link(&kinds, "https://github.com/a/shop/pull/12", None, None);
        assert_eq!((unsaid.kind.as_str(), unsaid.name.as_str(), unsaid.host.as_str()), ("", "", "github.com"));
        let said = link(&kinds, "https://github.com/a/shop/pull/12", Some("pull-request"), Some("a/shop #12".into()));
        assert_eq!((said.name.as_str(), said.title.as_deref()), ("Pull request", Some("a/shop #12")));
        // a kind no row has is no kind
        assert_eq!(link(&kinds, "https://example.com/x", Some("no-such-kind"), None).name, "");
    }

    #[test]
    fn the_site_is_the_host_of_the_address_as_addresses_are_built() {
        assert_eq!(site("https://user:pw@Docs.Example.com:8443/a/b?x=https://other.example/#y"), "docs.example.com");
        assert_eq!(site("http://localhost:3000/x"), "localhost");
        assert_eq!(site("not an address"), "");
    }
}
