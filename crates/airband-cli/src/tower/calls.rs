//! Each radio's place in a call, and what came of what he said from it.
use super::*;

/// One radio's place in a call. Each radio has its own, so a phone and a desktop app on a call at the same time
/// do not take each other's transmissions or answer each other's questions.
#[derive(Default)]
pub(super) struct Call {
    /// The transmission the next `GET next` from this radio will take.
    pub(super) outbox: Option<Transmission>,
    pub(super) ready: bool,
    pub(super) epoch: u64,
    /// He said "stand by".
    pub(super) held: bool,
    /// The agent that last spoke on this radio or was last addressed from it.
    pub(super) focus: Option<String>,
    /// Whether it is there.
    pub(super) presence: Presence,
    /// What it took to read out and has not answered for yet (heard, not heard, or ready for the next). If the
    /// answer that carried it was lost on the way, it is given the same thing again.
    pub(super) handed: Option<Transmission>,
    /// When this radio last did anything at all: a radio that only reads the feed and types never polls.
    pub(super) seen: Option<Instant>,
    /// Words he said that the tower could not place, with their files, held under the id of the question it
    /// asked him back ("Who is that for?"). Only an utterance that answers that id takes them.
    pub(super) asked: Option<(String, String, String)>,
    /// An idea for a new agent whose project the tower could not work out, held under the id of the question
    /// it asked him back ("Which project?").
    pub(super) unplaced_idea: Option<(String, Idea)>,
    /// How far this radio has the chat on screen: the highest change count it has asked for what follows of,
    /// while it was showing the chat. A line changed after this is one he may not have seen.
    pub(super) has: u64,
    /// When he last joined (turned listening on), in milliseconds since 1970. A call starts from there: he is
    /// not read what was asked before it.
    pub(super) began: u64,
    /// It was handed something and no sound came out of it: nothing more is handed to it before this moment,
    /// so a voice that is not answering does not run the whole queue past him in silence.
    pub(super) quiet_until: Option<Instant>,
}
/// Whether a radio is there: what it last said of itself (the radio protocol's `Presence`, which every request
/// of its carries), and when. The tower infers nothing from which requests it makes. Every rule that turns on
/// "is he there" reads this, by the one clock [`THERE`]: who can be handed something to read out, whether there
/// is anyone to ask for leave, whether a notification is needed, whether it is worth asking the harnesses what
/// their agents are doing.
#[derive(Default)]
pub(super) struct Presence {
    said: airband_core::radio::Presence,
    /// When it last said so. A radio that stops (the app is closed, the connection is gone) says nothing more,
    /// and what it last said stops counting.
    at: Option<Instant>,
}
impl Presence {
    /// It has been heard from lately: there is a radio to put a question to.
    pub(super) fn there(&self) -> bool {
        self.at.is_some_and(|t| t.elapsed() < THERE)
    }
    /// It is on a call: reading aloud is on.
    pub(super) fn on_call(&self) -> bool {
        self.there() && self.said.on_call
    }
    /// The chat is in front of him on it.
    pub(super) fn showing_chat(&self) -> bool {
        self.there() && self.said.showing_chat
    }
    /// The radio says where it stands. True when that puts it on a call it was not on.
    pub(super) fn says(&mut self, said: airband_core::radio::Presence) -> bool {
        let was = self.on_call();
        (self.said, self.at) = (said, Some(Instant::now()));
        !was && self.on_call()
    }
}
impl Call {
    pub(super) fn connected(&self) -> bool {
        self.presence.on_call()
    }
    pub(super) fn can_send(&self) -> bool {
        self.connected() && self.ready && !self.held && self.outbox.is_none() && self.quiet_until.is_none_or(|t| Instant::now() >= t)
    }
}
/// How many outcomes are kept for the feed. One that has gone from here is still told to a radio that asks
/// again under its id: the database has every answer.
pub(super) const SETTLED_KEEP: usize = 200;
impl Inner {
    /// Is any radio there?
    pub(super) fn connected(&self) -> bool {
        self.calls.values().any(Call::connected)
    }
    /// Is any radio there at all? Then there is someone to put a question to, and someone to see what the
    /// agents are doing.
    pub(super) fn following(&self) -> bool {
        self.calls.values().any(|c| c.presence.there())
    }
    /// What came of something he said: told to the radio that said it, in its feed.
    pub(super) fn settle(&mut self, radio: &str, id: &str, outcome: airband_core::radio::Outcome) {
        self.rev += 1;
        self.settled.push_back((self.rev, radio.to_string(), airband_core::radio::Settled { id: id.to_string(), outcome }));
        while self.settled.len() > SETTLED_KEEP {
            self.settled.pop_front();
        }
    }
    /// The outcomes a radio has not been told: those since the change count it asks from, or all that are
    /// kept for one starting from nothing.
    pub(super) fn settled_for(&self, radio: &str, since: Option<u64>) -> Vec<airband_core::radio::Settled> {
        self.settled.iter().filter(|(rev, to, _)| to == radio && *rev > since.unwrap_or(0)).map(|(_, _, s)| s.clone()).collect()
    }
    /// A radio says where it stands, as it does with every request. One that has just come on a call is free
    /// to be handed something, and what it had when it was last on one is still to be heard.
    pub(super) fn radio_says(&mut self, radio: &str, said: airband_core::radio::Presence) {
        if self.call(radio).presence.says(said) {
            log(format!("{radio} is on a call"));
            let call = self.call(radio);
            (call.began, call.ready, call.outbox) = (now_ms(), true, None);
            self.give_back(radio);
        }
    }
    /// The agent this radio last heard or spoke to, while it is still on the radio.
    pub(super) fn focused(&self, radio: &str) -> Option<&Session> {
        self.calls.get(radio).and_then(|c| c.focus.as_deref()).and_then(|f| self.find(f))
    }
    pub(super) fn call(&mut self, radio: &str) -> &mut Call {
        let call = self.calls.entry(radio.to_string()).or_default();
        call.seen = Some(Instant::now());
        call
    }
    /// A radio that is on a call and free to take a transmission.
    pub(super) fn free_radio(&self) -> Option<String> {
        let mut free: Vec<&String> = self.calls.iter().filter(|(_, c)| c.can_send()).map(|(id, _)| id).collect();
        free.sort();
        free.first().map(|id| id.to_string())
    }
}
