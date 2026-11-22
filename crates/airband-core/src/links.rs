//! The things an agent makes that live at a web address: a pull request, an issue, a published artifact. A
//! client shows each as a card that says what it is, what it is called and how it stands, and opens it.
//!
//! What a thing is comes from one of two typed sources, never from the words around it:
//! - the agent's own tool says so when it makes one ([`crate::agent::Event::Made`]), or
//! - its address has the shape of a row in the catalogue's `links` table ([`LinkKind`]): a host, and a path
//!   with named parts. That is the address's own structure, as the site that issues it defines it.
//!
//! An address no row fits is a plain link. Each kind is one file under `catalog/links/`.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One kind of thing, as its row in the catalogue has it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LinkKind {
    pub id: String,
    /// What it is called where he sees one: "Pull request".
    pub name: String,
    /// The mark shown beside it, by the name the platform's symbol set knows it by.
    pub symbol: String,
    /// The host its addresses are at. One that starts with `*.` takes any name under the rest.
    pub host: String,
    /// The path, one entry per part: a word that must be there as written, or a named part in braces
    /// (`{repo}`, and `{number:digits}` for one that is all digits).
    pub path: Vec<String>,
    /// Whether the address may go on past those parts (a pull request's `/files`).
    #[serde(default)]
    pub more: bool,
    /// What to call one before its title is known, from the named parts: `{repo} #{number}`.
    #[serde(default)]
    pub label: String,
    /// How the computer can ask what it is called and how it stands. Without one, the card says only what
    /// the agent's tool said about it, or nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

/// A program on his computer that knows about one of these, signed in as him. It is given the named parts
/// of the address and prints one line of JSON: `{"title": …, "state": …}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Ask {
    pub program: String,
    /// Its arguments, each with `{part}` filled from the address.
    pub args: Vec<String>,
    /// Every state the program can print, and how each is shown. One not listed is not shown.
    #[serde(default)]
    pub states: Vec<StateRow>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct StateRow {
    /// The state as the program prints it.
    pub is: String,
    /// The same in his words: "Merged".
    pub name: String,
    pub tone: Tone,
    /// It will not change again: there is no need to ask any more.
    #[serde(default)]
    pub last: bool,
}

/// How a state should look, for a client to pick a colour by.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    /// Open and well.
    Good,
    /// Finished as hoped.
    Done,
    /// Ended without being finished.
    Stopped,
    /// Something is wrong with it.
    Trouble,
    /// Not ready yet.
    Waiting,
    #[default]
    #[serde(other)]
    Plain,
}

/// How a thing stands, as a card shows it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LinkState {
    pub name: String,
    pub tone: Tone,
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
    /// A few words to call it by when it has no title: the site it is at, or `shop #12`.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<LinkState>,
}

/// A named part's value, as it may appear in an address and be handed to a program: letters, digits, dots,
/// dashes and underscores, starting with a letter or digit, so it can never be read as a flag.
fn fits(value: &str, digits: bool) -> bool {
    let mut chars = value.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok && value.len() <= 100 && value.chars().all(|c| if digits { c.is_ascii_digit() } else { c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') })
}

/// An address taken apart: its host, without a port, and the parts of its path.
fn parts(address: &str) -> Option<(String, Vec<&str>)> {
    let rest = address.strip_prefix("https://").or_else(|| address.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    // an address that carries a login is not one a site issues for a thing
    if host.is_empty() || host.contains('@') {
        return None;
    }
    let host = host.split(':').next()?.to_ascii_lowercase();
    Some((host, path.split('/').filter(|p| !p.is_empty()).collect()))
}

impl LinkKind {
    /// The named parts of an address of this kind. Nothing when the address is not one.
    pub fn takes(&self, address: &str) -> Option<BTreeMap<String, String>> {
        let (host, path) = parts(address)?;
        let at_host = match self.host.strip_prefix("*.") {
            Some(under) => host.strip_suffix(under).is_some_and(|name| name.ends_with('.') && name.len() > 1),
            None => host == self.host,
        };
        if !at_host || path.len() < self.path.len() || (path.len() > self.path.len() && !self.more) {
            return None;
        }
        let mut named = BTreeMap::new();
        for (want, has) in self.path.iter().zip(&path) {
            match want.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
                Some(part) => {
                    let (name, class) = part.split_once(':').unwrap_or((part, ""));
                    if !fits(has, class == "digits") {
                        return None;
                    }
                    named.insert(name.to_string(), has.to_string());
                }
                None if want == has => {}
                None => return None,
            }
        }
        Some(named)
    }
}

/// `{part}` in a template filled from an address's named parts. Nothing when it names a part that is not there.
pub fn fill(template: &str, named: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        // a brace that names no part is the program's own (a filter it is given), and is kept as written
        let close = rest[open..].find('}').map(|c| open + c);
        match close.and_then(|close| named.get(&rest[open + 1..close]).map(|v| (close, v))) {
            Some((close, value)) => {
                out.push_str(&rest[..open]);
                out.push_str(value);
                rest = &rest[close + 1..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    Some(out)
}

/// The site an address is at, for calling a plain link by: `example.com`.
pub fn site(address: &str) -> String {
    parts(address).map(|(host, _)| host.strip_prefix("www.").map(String::from).unwrap_or(host)).unwrap_or_else(|| address.to_string())
}

/// The row an address is a kind of, and its named parts. The first row that takes it.
pub fn kind_of<'a>(kinds: &'a [LinkKind], address: &str) -> Option<(&'a LinkKind, BTreeMap<String, String>)> {
    kinds.iter().find_map(|k| k.takes(address).map(|named| (k, named)))
}

/// What a card shows for an address before anything has been asked about it. `said` is the kind its maker's
/// tool gave it, by id, when a tool did: that comes before the shape of the address.
pub fn link(kinds: &[LinkKind], address: &str, said: Option<&str>) -> Link {
    let by_shape = kind_of(kinds, address);
    let kind = said.and_then(|id| kinds.iter().find(|k| k.id == id)).or(by_shape.as_ref().map(|(k, _)| *k));
    match kind {
        Some(k) => {
            let named = by_shape.filter(|(shape, _)| shape.id == k.id).map(|(_, named)| named).unwrap_or_default();
            let label = fill(&k.label, &named).filter(|l| !l.is_empty() && !l.contains('{')).unwrap_or_else(|| site(address));
            Link { address: address.to_string(), kind: k.id.clone(), name: k.name.clone(), symbol: k.symbol.clone(), label, title: None, state: None }
        }
        None => Link { address: address.to_string(), label: site(address), ..Link::default() },
    }
}

/// Is every row one that can be used safely? A name for a part is plain, a program is a bare name (found
/// where his programs are, never a path a row chose), and an argument that starts as a part cannot be a flag
/// because a part never starts with a dash.
pub fn check(kinds: &[LinkKind]) -> Result<(), String> {
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    for k in kinds {
        let host = k.host.strip_prefix("*.").unwrap_or(&k.host);
        if !plain(&k.id) || k.name.is_empty() || host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
            return Err(format!("{}: not a kind of link with a name and a host", k.id));
        }
        // a kind with nothing in its path would take every address at its host
        if k.path.is_empty() && k.host.strip_prefix("*.").is_none() {
            return Err(format!("{}: its path has no parts", k.id));
        }
        if let Some(ask) = &k.ask {
            if !plain(&ask.program) {
                return Err(format!("{}: the program it asks is not a bare name", k.id));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> Vec<LinkKind> {
        crate::catalog::built().links.clone()
    }

    #[test]
    fn an_address_is_the_kind_whose_shape_it_has_and_nothing_else() {
        let kinds = kinds();
        let of = |a: &str| kind_of(&kinds, a).map(|(k, _)| k.id.clone());
        assert_eq!(of("https://github.com/a/shop/pull/12").as_deref(), Some("github-pull"));
        assert_eq!(of("https://github.com/a/shop/pull/12/files#diff-1").as_deref(), Some("github-pull"));
        assert_eq!(of("https://GitHub.com/a/shop/issues/3?x=1").as_deref(), Some("github-issue"));
        assert_eq!(of("https://claude.ai/artifact/abc123").as_deref(), Some("claude-artifact"));
        // the same words somewhere else in an address are not its shape
        assert_eq!(of("https://example.com/github.com/a/shop/pull/12"), None);
        assert_eq!(of("https://github.com/a/shop/pull/new/branch"), None);
        assert_eq!(of("https://github.com/a/shop"), None);
        assert_eq!(of("https://user@github.com/a/shop/pull/12"), None);
        // a part that could be read as a flag is not a part
        assert_eq!(of("https://github.com/--web/shop/pull/12"), None);
    }

    #[test]
    fn a_card_is_called_by_its_parts_until_it_has_a_title() {
        let kinds = kinds();
        let pull = link(&kinds, "https://github.com/a/shop/pull/12", None);
        assert_eq!((pull.name.as_str(), pull.label.as_str()), ("Pull request", "shop #12"));
        let plain = link(&kinds, "https://www.example.com/x/y", None);
        assert_eq!((plain.kind.as_str(), plain.label.as_str()), ("", "example.com"));
        // what the tool that made it said it is comes before the shape of its address; a kind no row has
        // leaves the shape to say
        assert_eq!(link(&kinds, "https://example.com/made/1", Some("claude-artifact")).name, "Artifact");
        assert_eq!(link(&kinds, "https://claude.ai/artifact/abc123", Some("no-such-kind")).name, "Artifact");
    }

    #[test]
    fn what_a_program_is_asked_is_filled_from_the_address_and_keeps_its_own_braces() {
        let kinds = kinds();
        let (k, named) = kind_of(&kinds, "https://github.com/a/shop/pull/12").unwrap();
        let args: Vec<String> = k.ask.as_ref().unwrap().args.iter().map(|a| fill(a, &named).unwrap()).collect();
        assert!(args.contains(&"12".to_string()) && args.contains(&"a/shop".to_string()));
        assert_eq!(fill("{title, n: {number}}", &named).unwrap(), "{title, n: 12}");
    }

    #[test]
    fn every_row_is_safe_to_use() {
        check(&kinds()).unwrap();
        let mut bad = kinds();
        bad[0].ask.as_mut().unwrap().program = "/tmp/gh".into();
        assert!(check(&bad).is_err());
    }
}
