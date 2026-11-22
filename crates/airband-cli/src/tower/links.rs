//! What the tower knows about the things at the web addresses in the chat: a pull request, an issue, a
//! published artifact (`airband_core::links` says how an address gets its kind).
//!
//! Two things are kept here, in `links.json` beside the tower's other files:
//! - what each address is called and how it stands, as last heard. An agent's tool says so when it makes one
//!   ([`Event::Made`]); for a kind whose row has a program to ask (`gh`, signed in as him on this computer),
//!   the tower asks while the message is recent and the thing can still change. Nothing is sent anywhere
//!   else, and a card says only what was heard.
//! - which messages a made thing goes with, when the agent did not write its address out.
use super::*;
use airband_core::links::{Link, LinkKind, LinkState};
use std::collections::BTreeMap;

/// How long after a message a thing in it is still asked about.
const ASKED_FOR: u64 = 48 * 3600;
/// A message this new has its things asked about every minute; an older one every ten.
const NEW: u64 = 6 * 3600;
const OFTEN: u64 = 60;
const SELDOM: u64 = 600;
/// How long the program is given to answer.
const PATIENCE: Duration = Duration::from_secs(20);
/// How many are asked about in one round, newest first: a chat full of links does not start a hundred programs.
const AT_ONCE: usize = 4;
/// What is known about an address is dropped when nothing has asked about it or said it for this long.
const KEPT_FOR: u64 = 30 * 24 * 3600;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
struct Known {
    /// The kind its maker's tool said it is, by the id of that kind's row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// How it stands, as the asked program printed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    /// When its maker's tool last said it, in seconds.
    #[serde(default)]
    at: u64,
    /// When a program was last asked about it, whether or not it answered.
    #[serde(default)]
    asked: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub(super) struct Book {
    #[serde(default)]
    known: BTreeMap<String, Known>,
    /// By message: the things its agent's tool made before saying it, whose addresses are not in its words.
    #[serde(default)]
    with: BTreeMap<String, Vec<String>>,
    /// By agent: what its tool has made since it last spoke.
    #[serde(skip)]
    pending: HashMap<String, Vec<String>>,
}

fn file() -> std::path::PathBuf {
    config::dir().join("links.json")
}

/// What a program is to be asked about one address.
pub(super) struct Question {
    address: String,
    program: String,
    args: Vec<String>,
}

impl Book {
    pub(super) fn load() -> Book {
        let mut book: Book = airband_core::kept::load(&file()).unwrap_or_default();
        let oldest = airband_core::clock::now_secs().saturating_sub(KEPT_FOR);
        book.known.retain(|_, k| k.at.max(k.asked) >= oldest);
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
        if title.is_some() {
            known.title = title;
        }
        if kind.is_some() {
            known.kind = kind;
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
        let known = self.known.get(address);
        let mut link = airband_core::links::link(kinds, address, known.and_then(|k| k.kind.as_deref()));
        let Some(known) = known else { return link };
        link.title = known.title.clone();
        // a state the row does not list is not shown: the card says nothing it was not told how to say
        link.state = kinds.iter().find(|k| k.id == link.kind).and_then(|k| k.ask.as_ref()).zip(known.state.as_ref()).and_then(|(ask, state)| ask.states.iter().find(|s| &s.is == state)).map(|s| LinkState { name: s.name.clone(), tone: s.tone });
        link
    }
    /// What to ask now, given each address in a recent message and that message's age in seconds.
    fn due(&self, kinds: &[LinkKind], recent: &[(String, u64)], now: u64) -> Vec<Question> {
        let mut out: Vec<Question> = Vec::new();
        for (address, age) in recent {
            let Some((kind, named)) = airband_core::links::kind_of(kinds, address) else { continue };
            let Some(ask) = &kind.ask else { continue };
            let known = self.known.get(address);
            // one that has reached a state it will not leave is not asked about again
            let settled = known.and_then(|k| k.state.as_ref()).is_some_and(|state| ask.states.iter().any(|s| &s.is == state && s.last));
            let every = if *age < NEW { OFTEN } else { SELDOM };
            let asked = known.map(|k| k.asked).unwrap_or(0);
            if settled || *age > ASKED_FOR || now.saturating_sub(asked) < every || out.iter().any(|q| &q.address == address) {
                continue;
            }
            let Some(args) = ask.args.iter().map(|a| airband_core::links::fill(a, &named)).collect::<Option<Vec<String>>>() else { continue };
            out.push(Question { address: address.clone(), program: ask.program.clone(), args });
            if out.len() == AT_ONCE {
                break;
            }
        }
        out
    }
    /// What a program answered. True when the card changes by it.
    fn heard(&mut self, address: &str, title: Option<String>, state: Option<String>, now: u64) -> bool {
        let known = self.known.entry(address.to_string()).or_default();
        let before = (known.title.clone(), known.state.clone());
        known.asked = now;
        if title.is_some() {
            known.title = title;
        }
        if state.is_some() {
            known.state = state;
        }
        before != (known.title.clone(), known.state.clone())
    }
}

/// Run the program a row names and read the one line of JSON it prints. It is found where his own programs
/// are; a tower started by the system has a short path, so the usual places are added.
async fn ask(q: &Question) -> Option<(Option<String>, Option<String>)> {
    let path = format!("{}:/opt/homebrew/bin:/usr/local/bin", std::env::var("PATH").unwrap_or_default());
    let run = tokio::process::Command::new(&q.program).args(&q.args).env("PATH", path).env("AIRBAND_INTERNAL", "1").env("NO_COLOR", "1").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).output();
    let out = tokio::time::timeout(PATIENCE, run).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let said: Value = serde_json::from_slice(&out.stdout).ok()?;
    let text = |k: &str| said[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(|t| t.chars().take(200).collect::<String>());
    Some((text("title"), text("state")))
}

/// For as long as the tower runs: ask about the things in recent messages, and send a message again when
/// what is known about one of its things has changed.
pub(super) async fn keep_asking(app: Arc<App>) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let catalog = airband_core::catalog::current();
        let now = airband_core::clock::now_secs();
        let due = {
            let inner = app.inner.lock().unwrap();
            let recent: Vec<(String, u64)> = inner.log.iter().rev().filter(|e| e.from != "you").map(|e| (e, now.saturating_sub(e.at / 1000))).take_while(|(_, age)| *age <= ASKED_FOR).flat_map(|(e, age)| inner.links.addresses(&e.id, &e.full).into_iter().map(move |a| (a, age))).collect();
            inner.links.due(&catalog.links, &recent, now)
        };
        for q in due {
            let answer = ask(&q).await;
            let mut inner = app.inner.lock().unwrap();
            let (title, state) = answer.clone().unwrap_or_default();
            // one that could not be asked about (no such program, not signed in, no network) is tried again
            // later, not at once
            let changed = inner.links.heard(&q.address, title, state, now);
            inner.links.keep();
            if answer.is_none() {
                log(format!("links: {} could not say how one stands", q.program));
            }
            if changed {
                let with: Vec<String> = inner.log.iter().filter(|e| inner.links.addresses(&e.id, &e.full).contains(&q.address)).map(|e| e.id.clone()).collect();
                with.iter().for_each(|id| inner.touch(id));
                app.save(&inner);
            }
        }
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
        book.made("a1", "https://claude.ai/artifact/def", None, None);
        book.said("a1", "m1", "Published at https://claude.ai/artifact/def.");
        // it went with that message, and does not go with the next
        book.said("a1", "m2", "Done.");
        assert!(book.links(&kinds(), "m2", "Done.").is_empty());
        let cards = book.links(&kinds(), "m1", "Published at https://claude.ai/artifact/def.");
        assert_eq!(cards.iter().map(|l| l.address.as_str()).collect::<Vec<_>>(), ["https://claude.ai/artifact/def", "https://claude.ai/artifact/abc"]);
        assert_eq!((cards[1].name.as_str(), cards[1].title.as_deref()), ("Artifact", Some("Plan")));
        assert_eq!(cards[0].title, None);
    }

    #[test]
    fn a_thing_is_asked_about_while_it_can_change_and_its_message_is_recent() {
        let kinds = kinds();
        let mut book = Book::default();
        let pull = "https://github.com/a/shop/pull/12".to_string();
        let recent = vec![(pull.clone(), 30), ("https://example.com/x".to_string(), 30), ("https://claude.ai/artifact/abc".to_string(), 30)];
        let due = book.due(&kinds, &recent, 1_000_000);
        // only the one whose row has a program to ask
        assert_eq!(due.len(), 1);
        assert_eq!((due[0].program.as_str(), due[0].args[..5].join(" ")), ("gh", "pr view 12 --repo a/shop".to_string()));
        // just asked: not again for a minute
        assert!(book.heard(&pull, Some("Fix refunds".into()), Some("OPEN".into()), 1_000_000));
        assert!(!book.heard(&pull, Some("Fix refunds".into()), Some("OPEN".into()), 1_000_000));
        assert!(book.due(&kinds, &recent, 1_000_030).is_empty());
        assert_eq!(book.due(&kinds, &recent, 1_000_061).len(), 1);
        let card = book.link(&kinds, &pull);
        assert_eq!((card.title.as_deref(), card.state.as_ref().map(|s| s.name.as_str())), (Some("Fix refunds"), Some("Open")));
        // merged: it will not change again
        assert!(book.heard(&pull, None, Some("MERGED".into()), 1_000_100));
        assert!(book.due(&kinds, &recent, 2_000_000).is_empty());
        // an old message's things are left alone, and a state no row lists is not shown
        assert!(Book::default().due(&kinds, &[(pull.clone(), ASKED_FOR + 1)], 1_000_000).is_empty());
        book.heard(&pull, None, Some("SOMETHING_NEW".into()), 1_000_200);
        assert_eq!(book.link(&kinds, &pull).state, None);
    }
}
