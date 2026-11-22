//! What the tower knows about the things at the web addresses in the chat: a pull request, a published page
//! (`airband_core::links`: a thing is a kind only when the tool that made it said so).
//!
//! Kept here, in `links.json` beside the tower's other files:
//! - each thing a tool said it made, by its address: its kind and what it is called ([`Event::Made`]);
//! - which messages a thing goes with when the agent did not write its address out.
//!
//! An address in a message is a card only when it is exactly the address of such a thing. Any other is a
//! plain link. Nothing is asked of anyone about an address, and nothing is read from how one is spelled.
use super::*;
use airband_core::links::{Link, LinkKind};
use std::collections::BTreeMap;

/// What is known about an address is dropped when nothing has said it for this long.
const KEPT_FOR: u64 = 30 * 24 * 3600;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
struct Known {
    /// The kind it is, by the id of that kind's row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// When it was last said, in seconds.
    #[serde(default)]
    at: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(super) struct Book {
    #[serde(default)]
    known: BTreeMap<String, Known>,
    /// By message: the things that go with it whose addresses are not in its words.
    #[serde(default)]
    with: BTreeMap<String, Vec<String>>,
    /// By agent: what its tool has made since it last spoke.
    #[serde(skip)]
    pending: HashMap<String, Vec<String>>,
}

fn file() -> std::path::PathBuf {
    config::dir().join("links.json")
}

impl Book {
    pub(super) fn load() -> Book {
        let mut book: Book = airband_core::kept::load(&file()).unwrap_or_default();
        let oldest = airband_core::clock::now_secs().saturating_sub(KEPT_FOR);
        book.known.retain(|_, k| k.at >= oldest);
        book
    }
    fn keep(&self) {
        // a test's book is not the tower's
        if cfg!(test) {
            return;
        }
        if let Err(e) = airband_core::kept::save(&file(), self) {
            log(format!("links: what is known about them could not be written ({e})"));
        }
    }
    /// An agent's tool made something. It goes with the next thing that agent says.
    pub(super) fn made(&mut self, agent: &str, address: &str, title: Option<String>, kind: Option<String>) {
        let known = self.known.entry(address.to_string()).or_default();
        known.at = airband_core::clock::now_secs();
        if kind.is_some() {
            known.kind = kind;
        }
        if title.is_some() {
            known.title = title;
        }
        let pending = self.pending.entry(agent.to_string()).or_default();
        pending.retain(|a| a != address);
        pending.push(address.to_string());
        self.keep();
    }
    /// The agent said something: what its tool made since it last spoke goes with this message, unless its
    /// words already have the address.
    pub(super) fn said(&mut self, agent: &str, message: &str, words: &str) {
        let Some(made) = self.pending.remove(agent) else { return };
        let written = airband_core::speech::links(words);
        let rest: Vec<String> = made.into_iter().filter(|a| !written.contains(a)).collect();
        if !rest.is_empty() {
            self.with.insert(message.to_string(), rest);
            // only the newest few hundred messages are ever shown
            while self.with.len() > 400 {
                self.with.pop_first();
            }
            self.keep();
        }
    }
    /// The addresses that go with a message: the ones in its words, then what its agent's tool made.
    fn addresses(&self, message: &str, words: &str) -> Vec<String> {
        let mut all = airband_core::speech::links(words);
        for a in self.with.get(message).into_iter().flatten() {
            if !all.contains(a) {
                all.push(a.clone());
            }
        }
        all
    }
    /// The cards for a message.
    pub(super) fn links(&self, kinds: &[LinkKind], message: &str, words: &str) -> Vec<Link> {
        self.addresses(message, words).iter().map(|address| self.link(kinds, address)).collect()
    }
    fn link(&self, kinds: &[LinkKind], address: &str) -> Link {
        // exactly this address, as the tool gave it: nothing is read from how it is spelled
        let known = self.known.get(address);
        airband_core::links::link(kinds, address, known.and_then(|k| k.kind.as_deref()), known.and_then(|k| k.title.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> Vec<LinkKind> {
        airband_core::catalog::built().links.clone()
    }

    #[test]
    fn what_a_tool_made_goes_with_the_next_thing_its_agent_says() {
        let mut book = Book::default();
        book.made("a1", "https://claude.ai/artifact/abc", Some("Plan".into()), Some("claude-artifact".into()));
        book.made("a1", "https://github.com/a/shop/pull/12", Some("a/shop #12".into()), Some("pull-request".into()));
        book.said("a1", "m1", "The fix is at https://github.com/a/shop/pull/12 and the plan is published.");
        // they went with that message, and do not go with the next
        book.said("a1", "m2", "Done.");
        assert!(book.links(&kinds(), "m2", "Done.").is_empty());
        let cards = book.links(&kinds(), "m1", "The fix is at https://github.com/a/shop/pull/12 and the plan is published.");
        assert_eq!(cards.iter().map(|l| (l.name.as_str(), l.title.as_deref())).collect::<Vec<_>>(), [("Pull request", Some("a/shop #12")), ("Artifact", Some("Plan"))]);
    }

    #[test]
    fn an_address_no_tool_said_anything_about_is_a_plain_link_however_it_reads() {
        let mut book = Book::default();
        book.made("a1", "https://github.com/a/shop/pull/12", None, Some("pull-request".into()));
        let cards = book.links(&kinds(), "m9", "See https://github.com/a/shop/pull/13 and https://github.com/a/shop/pull/12/files and https://github.com/a/shop/pull/12.");
        assert_eq!(cards.iter().map(|l| (l.name.as_str(), l.host.as_str())).collect::<Vec<_>>(), [("", "github.com"), ("", "github.com"), ("Pull request", "github.com")]);
    }
}
