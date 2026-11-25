//! The tile: the small square outside the app (on the Home Screen, and on a car's dashboard) that says what is
//! going on without opening anything. What it holds and what it shows are decided here; the host only draws
//! [`TileView`] and asks the system to draw it again when [`SharedFolder::keep_tile`] or
//! [`SharedFolder::tile_heard`] says it changed.
//!
//! The small tile (the one size a car's dashboard has) shows, in order: an agent waiting on him, with its
//! question; else the news he has not had yet, one line for each agent, newest first; else who is working and
//! on what; else nothing, and the tile is the way to talk. A larger tile shows more lines of all three, most
//! pressing first, and a line on the lock screen the most pressing one ([`TileView`]).
//!
//! Whoever learns first writes it: the app, from the chat (`Desk::tile`), and the notification extension, from a
//! notification as it opens it. A line leaves the tile the moment it stops being news, as its notification does:
//! the chat says so (`news`, his reading it, his answer), and a notification that takes another down takes the
//! line off too, and is remembered, so a copy of the chat that has not caught up yet cannot put it back.
//!
//! It shows only what a notification on his lock screen shows, and only a few lines of it.
use serde::{Deserialize, Serialize};

/// How many agents' news the small tile shows: the newest. The car's dashboard shows only the small one.
pub const TILE_NEWS: usize = 2;
/// How many working agents the small tile shows.
pub const TILE_WORKING: usize = 3;
/// How many agents' news, and how many working agents, are kept: enough for the largest tile.
pub const NEWS_KEPT: usize = 6;
pub const WORKING_KEPT: usize = 6;
/// How many lines each larger tile shows, all kinds together.
const MEDIUM: usize = 3;
const LARGE: usize = 6;
const EXTRA_LARGE: usize = 10;
/// How many agents waiting on him are kept: the first is shown, the rest counted.
const WAITING_KEPT: usize = 8;
/// How many lines that stopped being news are remembered, newest last.
const GONE_KEPT: usize = 64;
/// How long a line may be on the tile, in characters. More than fits: the tile cuts it where it runs out.
const TEXT_MOST: usize = 240;

/// One agent's line on the tile: its question to him, or its news.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TileLine {
    /// The line's id: what takes it off the tile when it stops being news. Empty for an agent waiting on him
    /// whose question is not in the chat this phone has.
    pub message: String,
    /// The agent, by its id: who a tap answers.
    pub agent: String,
    pub callsign: String,
    /// Its two characters, and its team's place in the host's list of team colours.
    pub badge: String,
    pub colour: Option<u8>,
    /// The line's words as the chat has them, without marks, on one line.
    pub text: String,
}

/// An agent at work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TileWork {
    pub agent: String,
    pub callsign: String,
    pub badge: String,
    pub colour: Option<u8>,
    /// What it is doing right now, in a few words, when its tool says.
    pub doing: Option<String>,
}

/// What the tile is to hold: the agents waiting on him, newest question first; the news, newest first, one line
/// for each agent; and who is working.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TileFacts {
    pub waiting: Vec<TileLine>,
    pub news: Vec<TileLine>,
    pub working: Vec<TileWork>,
}

/// What the tile shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TileShows {
    /// An agent is waiting on him: it and its question, and how many more are. A tap answers it.
    Waiting { line: TileLine, more: u32 },
    /// What agents said that is news to him, newest first, at most [`TILE_NEWS`].
    News { lines: Vec<TileLine> },
    /// Who is working, and on what, at most [`TILE_WORKING`].
    Working { agents: Vec<TileWork> },
    /// Nothing is going on.
    #[default]
    Quiet,
}

/// One line of a larger tile: in order, the agents waiting on him, then the news, then who is working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TileItem {
    Waiting { line: TileLine },
    News { line: TileLine },
    Working { work: TileWork },
}

/// What each size of tile shows: the small one (the only one a car's dashboard has) shows one kind of thing; a
/// larger one shows more lines of all kinds, most pressing first; a line on the lock screen shows the first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TileView {
    pub small: TileShows,
    pub medium: Vec<TileItem>,
    pub large: Vec<TileItem>,
    pub extra_large: Vec<TileItem>,
    /// The most pressing line, for the lock screen. Nothing when nothing is going on.
    pub top: Option<TileItem>,
    /// How many agents are waiting on him, all told.
    pub waiting: u32,
}

/// What the shared folder keeps for the tile: the facts, the lines that stopped being news, and what that
/// makes the tile show, which is all the tile reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TileKept {
    pub(crate) facts: TileFacts,
    pub(crate) gone: Vec<String>,
    pub(crate) shows: TileView,
}

impl TileFacts {
    /// What the tile shows of these.
    fn shows(&self) -> TileShows {
        if let Some((line, rest)) = self.waiting.split_first() {
            return TileShows::Waiting { line: line.clone(), more: rest.len() as u32 };
        }
        if !self.news.is_empty() {
            return TileShows::News { lines: self.news.iter().take(TILE_NEWS).cloned().collect() };
        }
        if !self.working.is_empty() {
            return TileShows::Working { agents: self.working.iter().take(TILE_WORKING).cloned().collect() };
        }
        TileShows::Quiet
    }
    /// Every line a larger tile could show, most pressing first.
    fn items(&self) -> Vec<TileItem> {
        let waiting = self.waiting.iter().map(|l| TileItem::Waiting { line: l.clone() });
        let news = self.news.iter().map(|l| TileItem::News { line: l.clone() });
        let working = self.working.iter().map(|w| TileItem::Working { work: w.clone() });
        waiting.chain(news).chain(working).collect()
    }
    /// What each size of tile shows of these.
    fn view(&self) -> TileView {
        let items = self.items();
        let first = |n: usize| items.iter().take(n).cloned().collect::<Vec<_>>();
        TileView { small: self.shows(), medium: first(MEDIUM), large: first(LARGE), extra_large: first(EXTRA_LARGE), top: items.first().cloned(), waiting: self.waiting.len() as u32 }
    }
    /// Without the lines that stopped being news, and no more than is kept.
    fn without(mut self, gone: &[String]) -> TileFacts {
        let news = |l: &TileLine| l.message.is_empty() || !gone.contains(&l.message);
        self.waiting.retain(news);
        self.news.retain(news);
        self.waiting.truncate(WAITING_KEPT);
        self.news.truncate(NEWS_KEPT);
        self.working.truncate(WORKING_KEPT);
        self
    }
}

impl TileKept {
    /// The facts the app has from the chat, in place of what was kept, but for lines that stopped being news.
    pub(crate) fn with_chat(&self, facts: TileFacts) -> TileKept {
        TileKept::of(facts, self.gone.clone())
    }
    /// What an opened notification changes: a question puts its agent first among the ones waiting, news puts
    /// its line first among the news; either way its agent has stopped working. One that takes a line down
    /// takes it off the tile, and it is remembered as gone.
    pub(crate) fn heard(&self, opening: &crate::NoticeOpening) -> TileKept {
        let (mut facts, mut gone) = (self.facts.clone(), self.gone.clone());
        match opening {
            crate::NoticeOpening::Opened { notice: n } => {
                let line = TileLine { message: n.message.clone(), agent: n.agent.clone(), callsign: n.title.clone(), badge: n.badge.clone(), colour: n.colour, text: words(&n.body) };
                let list = if n.question { &mut facts.waiting } else { &mut facts.news };
                list.retain(|l| l.agent != line.agent);
                list.insert(0, line);
                facts.working.retain(|w| w.agent != n.agent);
            }
            crate::NoticeOpening::Withdrawn { message, .. } => {
                gone.retain(|g| g != message);
                gone.push(message.clone());
                let extra = gone.len().saturating_sub(GONE_KEPT);
                gone.drain(..extra);
            }
            crate::NoticeOpening::Plain { .. } => {}
        }
        TileKept::of(facts, gone)
    }
    fn of(facts: TileFacts, gone: Vec<String>) -> TileKept {
        let facts = facts.without(&gone);
        TileKept { shows: facts.view(), facts, gone }
    }
}

/// A line's words for the tile: as the chat has them, without marks, on one line, cut to what could ever fit.
pub(crate) fn words(text: &str) -> String {
    airband_core::prose::clip(&airband_core::prose::plain(text), TEXT_MOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(message: &str, agent: &str, text: &str) -> TileLine {
        TileLine { message: message.into(), agent: agent.into(), callsign: format!("{agent} 1"), badge: "S1".into(), colour: Some(1), text: text.into() }
    }
    fn work(agent: &str, doing: Option<&str>) -> TileWork {
        TileWork { agent: agent.into(), callsign: agent.into(), badge: "W1".into(), colour: None, doing: doing.map(String::from) }
    }
    fn notice(message: &str, agent: &str, question: bool, body: &str) -> crate::NoticeOpening {
        crate::NoticeOpening::Opened { notice: crate::OpenedNotice {
            station: "c".into(), message: message.into(), title: format!("{agent} 1"), subtitle: String::new(), body: body.into(), badge: "S1".into(), colour: Some(1),
            room: String::new(), agent: agent.into(), question, options: Vec::new(), permission: false, buttons: crate::NoticeButtonSet::Reply, thread: String::new(),
            level: crate::NoticeLevel::Active, tag: String::new(), rev: 1, replacing: false,
        } }
    }
    fn withdrawn(message: &str) -> crate::NoticeOpening {
        crate::NoticeOpening::Withdrawn { message: message.into(), tag: String::new(), why: airband_core::radio::Withdrawn::Heard }
    }

    #[test]
    fn a_question_comes_before_news_and_news_before_work_and_nothing_is_quiet() {
        let all = TileFacts { waiting: vec![line("q1", "shop", "Ship it?"), line("q2", "site", "Which colour?")], news: vec![line("r1", "docs", "Done.")], working: vec![work("ops", Some("Running the tests"))] };
        assert_eq!(all.shows(), TileShows::Waiting { line: line("q1", "shop", "Ship it?"), more: 1 });
        let news = TileFacts { waiting: Vec::new(), ..all.clone() };
        assert_eq!(news.shows(), TileShows::News { lines: vec![line("r1", "docs", "Done.")] });
        let working = TileFacts { news: Vec::new(), ..news };
        assert_eq!(working.shows(), TileShows::Working { agents: vec![work("ops", Some("Running the tests"))] });
        assert_eq!(TileFacts::default().shows(), TileShows::Quiet);
    }

    #[test]
    fn a_notification_puts_its_line_on_and_the_one_that_takes_it_down_takes_it_off_for_good() {
        let start = TileKept::default().with_chat(TileFacts { working: vec![work("shop", Some("Fixing the cart")), work("site", None)], ..TileFacts::default() });
        // a report: the news, and its agent is no longer at work
        let told = start.heard(&notice("r1", "shop", false, "The cart **works** again."));
        assert_eq!(told.shows.small, TileShows::News { lines: vec![TileLine { text: "The cart works again.".into(), ..line("r1", "shop", "") }] });
        assert_eq!(told.facts.working, vec![work("site", None)]);
        // a question goes first, over the news
        let asked = told.heard(&notice("q1", "site", true, "Keep the old page?"));
        assert!(matches!(&asked.shows.small, TileShows::Waiting { line, more: 0 } if line.message == "q1" && line.text == "Keep the old page?"));
        // answered: the news shows again
        let answered = asked.heard(&withdrawn("q1"));
        assert!(matches!(&answered.shows.small, TileShows::News { lines } if lines[0].message == "r1"));
        // heard: nothing left but quiet
        let heard = answered.heard(&withdrawn("r1"));
        assert_eq!(heard.shows, TileView::default());
        // a chat that has not caught up yet cannot put either back
        let stale = heard.with_chat(TileFacts { waiting: vec![line("q1", "site", "Keep the old page?")], news: vec![line("r1", "shop", "The cart works again.")], working: Vec::new() });
        assert_eq!(stale.shows, TileView::default());
    }

    #[test]
    fn a_larger_tile_shows_more_lines_of_every_kind_most_pressing_first() {
        let facts = TileFacts {
            waiting: vec![line("q1", "shop", "Ship it?"), line("q2", "site", "Which page?")],
            news: (0..4).map(|n| line(&format!("r{n}"), &format!("n{n}"), "Done.")).collect(),
            working: (0..3).map(|n| work(&format!("w{n}"), Some("Testing"))).collect(),
        };
        let view = facts.view();
        assert!(matches!(&view.small, TileShows::Waiting { more: 1, .. }));
        let kinds = |items: &[TileItem]| items.iter().map(|i| match i { TileItem::Waiting { .. } => 'q', TileItem::News { .. } => 'n', TileItem::Working { .. } => 'w' }).collect::<String>();
        assert_eq!(kinds(&view.medium), "qqn");
        assert_eq!(kinds(&view.large), "qqnnnn");
        assert_eq!(kinds(&view.extra_large), "qqnnnnwww");
        assert_eq!(view.top, Some(TileItem::Waiting { line: line("q1", "shop", "Ship it?") }));
        assert_eq!(view.waiting, 2);
    }

    #[test]
    fn a_newer_line_from_the_same_agent_takes_the_place_of_its_older_one() {
        let one = TileKept::default().heard(&notice("r1", "shop", false, "First."));
        let two = one.heard(&notice("r2", "shop", false, "Second."));
        assert_eq!(two.facts.news.iter().map(|l| l.message.as_str()).collect::<Vec<_>>(), vec!["r2"]);
    }

    #[test]
    fn what_is_kept_stays_small() {
        let mut kept = TileKept::default();
        for n in 0..GONE_KEPT + 5 {
            kept = kept.heard(&notice(&format!("r{n}"), &format!("a{n}"), n % 2 == 0, "x"));
            kept = kept.heard(&withdrawn(&format!("old{n}")));
        }
        assert_eq!(kept.gone.len(), GONE_KEPT);
        assert!(kept.facts.waiting.len() <= WAITING_KEPT && kept.facts.news.len() <= NEWS_KEPT);
        assert!(words(&"word ".repeat(200)).chars().count() <= TEXT_MOST);
    }

    /// What the tile reads, in the shape it reads it: the host draws from this and nothing else.
    #[test]
    fn the_tile_reads_a_kind_and_its_fields() {
        let shows = TileShows::Waiting { line: line("q1", "shop", "Ship it?"), more: 2 };
        assert_eq!(serde_json::to_string(&shows).unwrap(), r#"{"kind":"waiting","line":{"message":"q1","agent":"shop","callsign":"shop 1","badge":"S1","colour":1,"text":"Ship it?"},"more":2}"#);
        assert_eq!(serde_json::to_string(&TileShows::Working { agents: vec![work("ops", None)] }).unwrap(), r#"{"kind":"working","agents":[{"agent":"ops","callsign":"ops","badge":"W1","colour":null,"doing":null}]}"#);
        assert_eq!(serde_json::to_string(&TileShows::Quiet).unwrap(), r#"{"kind":"quiet"}"#);
        assert_eq!(serde_json::to_string(&TileItem::Working { work: work("ops", Some("Testing")) }).unwrap(), r#"{"kind":"working","work":{"agent":"ops","callsign":"ops","badge":"W1","colour":null,"doing":"Testing"}}"#);
        assert_eq!(serde_json::to_string(&TileView::default()).unwrap(), r#"{"small":{"kind":"quiet"},"medium":[],"large":[],"extra_large":[],"top":null,"waiting":0}"#);
    }
}
