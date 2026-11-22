//! What crosses to the host for the chat: its lines, the agents in it, their links and pictures, and the
//! words for why a line was not read aloud.
use super::*;

/// Why something an agent said was not read out to him while he had reading aloud on, in words for him.
pub(crate) fn not_read(why: airband_core::radio::NotRead) -> &'static str {
    use airband_core::radio::NotRead;
    match why {
        NotRead::Routine => "Not read aloud. There was nothing in it you needed to hear.",
        NotRead::Superseded => "Not read aloud. The agent said something newer first.",
        NotRead::Answered => "Not read aloud. You had already answered the agent, or it had gone back to work.",
        NotRead::Dismissed => "Not read aloud. You let this question go.",
        NotRead::AgentGone => "Not read aloud. The agent had stopped running.",
        NotRead::Stale => "Not read aloud. It was said before you turned reading on.",
        NotRead::PermissionEnded => "Not read aloud. The request was over before its turn came.",
        NotRead::Unknown => "Not read aloud.",
    }
}

impl ChatAgent {
    /// Called this now: every form of its name shown, from the typed parts, as the feed would give them.
    pub(crate) fn call(&mut self, c: airband_core::routing::Callsign) {
        (self.callsign, self.badge, self.number) = (c.shown(), c.badge(), c.shown_rest());
        (self.group, self.number_kept, self.named) = (c.project, c.number, c.named);
    }
    /// Its callsign as the typed parts it was made from.
    pub fn parts(&self) -> airband_core::routing::Callsign {
        airband_core::routing::Callsign { project: self.group.clone(), number: self.number_kept.clone(), named: self.named }
    }
}

pub(crate) fn chat(m: airband_core::radio::Message) -> ChatMessage {
    ChatMessage { seq: m.seq, id: m.id, at: m.at, his: m.his, from: m.from, badge: m.badge, to_agents: m.to_agents, group: m.group, team: m.team, room: String::new(), to: m.to, needs_you: m.kind == Some(Kind::NeedsYou), fyi: m.kind == Some(Kind::Fyi) || m.cancelled, stalled: m.kind == Some(Kind::Stalled), unjudged: if m.kind == Some(Kind::Unjudged) { "Shown as the agent wrote it. No summary was made.".into() } else { String::new() }, long: m.long, text: m.text, queued: m.queued, seen: m.seen.unwrap_or(0), options: m.options, recommended: m.recommended, permission: m.permission, pictures: m.pictures.into_iter().map(|p| ChatPicture { id: p.id, width: p.width, height: p.height }).collect(), links: m.links.into_iter().map(chat_link).collect(), agent: m.agent, station: String::new(), computer: String::new(), open: m.open, colour: None, read: m.read, own_session: m.own_session, not_read: m.not_read.map(not_read).unwrap_or_default().to_string() }
}

pub(crate) fn chat_feed(feed: Feed) -> ChatFeed {
    ChatFeed {
        seq: feed.seq,
        messages: feed.messages.into_iter().map(chat).collect(),
        agents: feed.roster.into_iter().map(|r| {
            let mut agent = ChatAgent { id: r.id, callsign: String::new(), badge: String::new(), group: String::new(), team: r.team, room: String::new(), number: String::new(), number_kept: String::new(), named: false, harness: r.harness, waiting: r.waiting, state: r.state.into(), subagents: r.subagents, doing: r.doing, held: r.held, terminal: r.terminal, ends: r.ends, model: r.model, effort: r.effort };
            agent.call(r.callsign);
            agent
        }).collect(),
        quick: feed.quick.into_iter().map(|q| QuickMessage { label: q.label, text: q.text }).collect(),
        open: feed.open,
        reads_failed: feed.reads_failed.map(|trouble| trouble.said(&feed.reads)).unwrap_or_default(),
        reads: feed.reads,
        station: feed.station,
    }
}

/// One line of the chat: something an agent said, cut to a line or two, or something he said.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatMessage {
    pub seq: u64,
    pub id: String,
    /// Milliseconds since 1970.
    pub at: u64,
    /// What to show it as coming from: a callsign, or a label for him.
    pub from: String,
    /// He said it.
    pub his: bool,
    /// What its team is called, to show.
    pub group: String,
    /// Its team's id on the computer that has it.
    pub team: String,
    /// The room it belongs to on his screen: that computer and that team, as one id. What views and colours
    /// go by. Empty for a message of his to several teams.
    pub room: String,
    /// For his messages: who it went to.
    pub to: Vec<String>,
    /// For his messages: the ids of the agents it went to.
    pub to_agents: Vec<String>,
    /// The agent is stopped until he answers or acts.
    pub needs_you: bool,
    /// Not worth his attention: show it dimmed or folded away.
    pub fyi: bool,
    /// The agent stopped here without finishing or asking: it needs a poke.
    pub stalled: bool,
    /// No model read it, so it is shown as the agent wrote it: the words that say so, or empty.
    pub unjudged: String,
    pub text: String,
    /// It is shown folded to its first lines, with a tap to open it: there is more than `text` says (`more(id)`
    /// has it), or `text` itself runs past a few lines. The one rule for this.
    pub long: bool,
    /// His own words, waiting for the agent to stop what it is doing. He can still take them back.
    pub queued: bool,
    /// For his own words: when the agent took them in, in milliseconds since 1970; 0 while that is not known.
    pub seen: u64,
    /// Answers he can give with one tap: show each as a button and send the one tapped with `re` set to this message.
    pub options: Vec<String>,
    /// Which of `options` the agent itself recommended, if it said. Mark only that one; with none, mark none.
    pub recommended: Option<u32>,
    /// A request for permission to do one thing on his computer. Offer its two answers and his own words, and
    /// never a "do what you think best".
    pub permission: bool,
    /// Pictures that go with it, the ones the agent meant him to see first. `picture(id)` fetches one.
    pub pictures: Vec<ChatPicture>,
    /// The things at web addresses that go with it (a pull request, an artifact, a plain link). The short text
    /// leaves them out: show each as a card that opens its address.
    pub links: Vec<ChatLink>,
    /// The agent that said it: stable while that agent runs, whatever it is called. Nothing for his own messages.
    pub agent: Option<String>,
    /// Which computer it came from, by its [`crate::DeskComputer::id`], when he is paired with several and a
    /// [`Desk`] merged them. Empty otherwise.
    pub station: String,
    /// That computer's name, to show beside who said it, when he is paired with several. Empty otherwise.
    pub computer: String,
    /// It needs him and nothing has answered it yet. An inbox shows these.
    pub open: bool,
    /// Two characters for who said it ("S3", "CP"), and its team's place in the host's list of team colours.
    /// A [`Desk`] fills these in; empty and 0 otherwise, and always for his own messages.
    pub badge: String,
    pub colour: Option<u8>,
    /// It holds something to read whole, called this ("the refund email"): offer the full message under that
    /// name. [`Desk::full`] fetches it.
    pub read: Option<String>,
    /// The agent takes this answer only from him in its own session. If the tower does not run that agent,
    /// an answer from here will not count: say so, and offer to take it over.
    pub own_session: bool,
    /// It was to be read out to him while he had reading aloud on, and was not: why, in a sentence. Empty for
    /// every other line. Mark the line, and show the sentence when he asks.
    pub not_read: String,
}

/// Something at a web address that goes with a message. One whose maker's tool said what it is has a `kind`
/// and usually a `title`: show it as a card. One nothing was said about has neither: show it as a small
/// button called by its `host`. A host draws every kind the same way; it has no list of kinds of its own.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChatLink {
    /// Where a tap goes. Opened as an ordinary address, the phone hands it to the app that owns such
    /// addresses when one is installed, and to the browser otherwise.
    pub address: String,
    /// What kind of thing it is, in his words ("Pull request"). Empty when no tool said.
    pub kind: String,
    /// The name of the mark to show beside it, in the platform's symbol set. Empty when no tool said.
    pub symbol: String,
    /// What it is called, as the tool that made it said. Empty when it did not.
    pub title: String,
    /// The site it is at: "github.com".
    pub host: String,
}

pub(crate) fn chat_link(l: airband_core::links::Link) -> ChatLink {
    ChatLink { address: l.address, kind: l.name, symbol: l.symbol, title: l.title.unwrap_or_default(), host: l.host }
}

/// A picture an agent looked at or made for him. The size is known before the picture is fetched, so a host
/// can lay it out at once.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatPicture {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatAgent {
    /// Stable while the agent runs.
    pub id: String,
    pub callsign: String,
    /// Its two characters.
    pub badge: String,
    /// What its team is called, to show.
    pub group: String,
    /// Its team's id on the computer that has it.
    pub team: String,
    /// Its room on his screen: that computer and that team, as one id.
    pub room: String,
    /// What tells it from the others of its team: the part of its callsign after the team's name.
    pub number: String,
    /// That part is a name he gave the session, not a number his computer handed out.
    pub named: bool,
    /// That part as his computer keeps it (a number as its word), for telling his other computers which names
    /// are taken.
    pub number_kept: String,
    pub harness: String,
    pub waiting: bool,
    pub state: AgentState,
    /// How many subagents it has running.
    pub subagents: u32,
    /// What it is doing right now, in a few words, while it works and its harness can say.
    pub doing: Option<String>,
    /// The tower runs it itself, so his words reach it as his own.
    pub held: bool,
    /// It is known to run in a terminal of his. Not known for an agent in its own tool's app.
    pub terminal: bool,
    /// He can end it from here.
    pub ends: bool,
    /// The model it runs on ("Opus 4.7") and how hard it is set to think ("high"), when its harness can say.
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Something he says all the time, as a button: `label` is what the button says, `text` what is sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuickMessage {
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatFeed {
    pub messages: Vec<ChatMessage>,
    pub agents: Vec<ChatAgent>,
    /// Pass this as `after` next time.
    pub seq: u64,
    /// The quick messages to offer: the standard ones, then his own.
    pub quick: Vec<QuickMessage>,
    /// Ids of every message still waiting on him, current as of this answer. The inbox is exactly these.
    pub open: Vec<String>,
    /// What the tower's computer is called, for an app paired with more than one.
    pub station: String,
    /// What reads the agents' messages on that computer, by name. Empty when nothing does.
    pub reads: String,
    /// Why what reads there could not, the last time it was asked, in words for him. Empty while it works.
    pub reads_failed: String,
}
