//! Each radio's place in a call, and what came of what he said from it.
use super::*;

/// Words he said that the tower could not place, with their files, held until he says who they are for.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Asked {
    /// The question asked him back ("Who is that for?"); only an answer to it takes these words.
    pub(super) question: airband_core::ids::QuestionId,
    pub(super) words: String,
    pub(super) files: String,
}

/// An idea for a new agent held until he says which project it is for.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Unplaced {
    /// The question asked him back ("Which project?").
    pub(super) question: airband_core::ids::QuestionId,
    pub(super) idea: Idea,
}

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
    /// What he calls it, for the log and `airband status`: its pairing's name (a call is kept under the pairing's
    /// channel).
    pub(super) name: Option<String>,
    /// What it took to read out and has not answered for yet (heard, not heard, or ready for the next). If the
    /// answer that carried it was lost on the way, it is given the same thing again.
    pub(super) handed: Option<Transmission>,
    /// When this radio last did anything at all: a radio that only reads the feed and types never polls.
    pub(super) seen: Option<Instant>,
    /// Words he said that the tower could not place, with their files, each held under the id of the question
    /// it asked him back ("Who is that for?"). Only an utterance that answers that id takes them; a later
    /// question never puts an earlier one out.
    asked: Vec<Asked>,
    /// Ideas for a new agent whose project the tower could not work out, each held under the id of the
    /// question it asked him back ("Which project?").
    unplaced: Vec<Unplaced>,
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
/// How many questions put back to him one radio holds answers open for. Past that the oldest is let go.
const OPEN_QUESTIONS: usize = 8;

impl Call {
    /// Hold words he said under the question asked about them.
    pub(super) fn ask(&mut self, asked: Asked) {
        self.asked.push(asked);
        let extra = self.asked.len().saturating_sub(OPEN_QUESTIONS);
        self.asked.drain(..extra);
    }
    /// The words held for "Who is that for?" when these words answer that question, by its id. Read, not
    /// taken: they stay until what he said is settled ([`Call::resolve_asked`]).
    pub(super) fn asked_for(&self, answers: Option<&str>) -> Option<Asked> {
        self.asked.iter().find(|a| answers == Some(a.question.as_str())).cloned()
    }
    /// The question is settled: what it held is let go.
    pub(super) fn resolve_asked(&mut self, question: &airband_core::ids::QuestionId) {
        self.asked.retain(|a| a.question != *question);
    }
    /// Hold an idea under the question asked about where it goes.
    pub(super) fn hold_unplaced(&mut self, unplaced: Unplaced) {
        self.unplaced.push(unplaced);
        let extra = self.unplaced.len().saturating_sub(OPEN_QUESTIONS);
        self.unplaced.drain(..extra);
    }
    /// The idea held for "Which project?" when these words answer that question, by its id. Read, not taken.
    pub(super) fn unplaced_for(&self, answers: Option<&str>) -> Option<Unplaced> {
        self.unplaced.iter().find(|u| answers == Some(u.question.as_str())).cloned()
    }
    /// The question is settled: the idea is let go.
    pub(super) fn resolve_unplaced(&mut self, question: &airband_core::ids::QuestionId) {
        self.unplaced.retain(|u| u.question != *question);
    }
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
/// What he calls the radio a call is kept under: its pairing's name, once it has said where it stands.
pub(super) fn name_of(calls: &HashMap<String, Call>, radio: &str) -> String {
    calls.get(radio).and_then(|c| c.name.clone()).unwrap_or_else(|| radio.to_string())
}
impl Inner {
    /// What he calls this radio.
    pub(super) fn radio_name(&self, radio: &str) -> String {
        name_of(&self.calls, radio)
    }
    /// A radio says where it stands, with the name he calls it by.
    pub(super) fn radio_named_says(&mut self, radio: &str, name: String, said: airband_core::radio::Presence) {
        self.call(radio).name = Some(name);
        self.radio_says(radio, said);
    }
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
            log(format!("{} is on a call", self.radio_name(radio)));
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
    /// A radio that is on a call and free to take a transmission: of several, the one he showed was there
    /// most lately, never one picked by the order of their names.
    pub(super) fn free_radio(&self) -> Option<String> {
        self.calls.iter().filter(|(_, c)| c.can_send()).max_by_key(|(_, c)| c.seen).map(|(id, _)| id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::ids::QuestionId;

    #[test]
    fn a_later_question_never_puts_out_one_still_waiting_for_his_answer() {
        let mut call = Call::default();
        let idea = |task: &str| Idea { tool: None, project: None, model: None, effort: None, task: task.into() };
        call.hold_unplaced(Unplaced { question: QuestionId::new("q1"), idea: idea("fix login") });
        call.hold_unplaced(Unplaced { question: QuestionId::new("q2"), idea: idea("write docs") });
        assert_eq!(call.unplaced_for(Some("q1")).unwrap().idea.task, "fix login", "the first is still there to answer");
        call.resolve_unplaced(&QuestionId::new("q1"));
        assert!(call.unplaced_for(Some("q1")).is_none());
        assert_eq!(call.unplaced_for(Some("q2")).unwrap().idea.task, "write docs");
        call.ask(Asked { question: QuestionId::new("a1"), words: "ship it".into(), files: String::new() });
        call.ask(Asked { question: QuestionId::new("a2"), words: "hold on".into(), files: String::new() });
        assert_eq!(call.asked_for(Some("a1")).unwrap().words, "ship it");
        assert!(call.asked_for(None).is_none(), "words that answer no question take nothing");
    }
}
