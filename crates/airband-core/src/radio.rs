//! `airband.radio/2`: what goes between the tower and a radio (the app on the phone, in the car, on a desk).
//!
//! The radio asks; the tower answers. All under `/v1/radio/`. Sent straight to the tower with
//! `Authorization: Bearer <token>`, or sealed and passed through a relay (`airband.relay/1`):
//! - `GET next`: held open up to 25 seconds. 200 with a [`Transmission`], or 204 when nothing is waiting.
//!   A transmission is words: the radio speaks them with the voice he chose on it. The tower makes no sound
//!   and hears none; what listens and what speaks are the radio's alone.
//! - `POST heard` [`Heard`]: that transmission was spoken to the end.
//! - `POST utterance` [`Utterance`] → [`Outcome`]: what he said. Answered at once: the tower has it. What came
//!   of it arrives in the feed ([`Feed::answers`]), and asking again under the same id says the same.
//! - `POST ready`: the radio is free for the next transmission.
//! - `POST presence` [`Presence`]: whether the radio is on a call and whether the chat is in front of him.
//!   Said when either changes; every other request carries it too.
//!
//! Those are call mode: one transmission at a time, spoken. Chat mode reads the same messages as a feed:
//! - `GET feed?scope=…&after=<seq>&wait=<seconds>` → [`Feed`]: everything said since `after`, each cut to a
//!   line or two. `scope` is `all`, `group:<name>` or `agent:<callsign>`.
//! - `GET message/<id>?form=detail|full` → [`More`]: the fuller spoken version, or exactly what the agent wrote.
//! - `POST utterance` is also how chat sends: leading `@tags` say who it is for (`@shop2`, `@shop`, `@all`).
//! - Starting a new agent is an utterance too: one whose `act` is a launch, for a project the feed lists.
use serde::{Deserialize, Serialize};

mod asks;
mod feed;
pub use asks::*;
pub use feed::*;

pub const SCHEMA: &str = "airband.radio/2";

/// How much a message deserves his attention. Deciding this is the tower's main job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// The agent is stopped until he answers or acts, or something broke that he must know now.
    /// Interrupts in call mode; flagged in chat.
    #[serde(rename = "needs-you", alias = "question")]
    NeedsYou,
    /// It finished a piece of work, or found or decided something he would want to know.
    #[serde(rename = "report")]
    Report,
    /// Progress chatter, an acknowledgement, a restatement: he would not miss it. Never spoken; shown
    /// dimmed in chat and still there in full if he looks.
    #[serde(rename = "fyi")]
    Fyi,
    /// It said what it would do next, or that it was carrying on, and then stopped: the work is not finished and
    /// nothing was asked of him. It needs a nudge, not an answer ("poke").
    #[serde(rename = "stalled")]
    Stalled,
    /// No model read it, so nothing says what it is. Shown as the agent wrote it and marked so; he is told of
    /// it as he is of a report, since nothing says it is routine.
    #[serde(rename = "unjudged")]
    Unjudged,
    /// A kind a newer tower uses that this build does not know. Shown like a report: never dropped, and never
    /// a reason to refuse the rest of what came with it.
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// One thing an agent has to say.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transmission {
    pub id: String,
    pub callsign: String,
    pub kind: Kind,
    /// The words to speak, callsign first.
    pub say: String,
    /// How many more are waiting behind this one.
    #[serde(default)]
    pub waiting: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Heard {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Utterance {
    pub text: String,
    /// Random, the same on every retry of this utterance: the tower takes each id once, and tells what came
    /// of it under this id.
    pub id: String,
    /// The message this answers, when the radio knows: the transmission just spoken, or the chat line he
    /// replied to. With it the tower need not work out who it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<String>,
    /// The team whose room he was looking at when he said it, by name. With it, and no tag or `re`, the
    /// tower chooses only among that team's agents. Left out, it chooses among all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// When this is a quick message: the two words on the button he tapped. `text` is then the whole
    /// instruction it stands for. The chat shows the label; the agent gets the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Files he attached, by the ids they were sent under (`POST attachment`). The agent is told where each is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    /// Agents on his screen that this tower does not have: they are on another of his computers. With them the
    /// tower reads his words against everything he can see, and can answer that they are for one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<Other>,
    /// The agents he addressed by picking them on his screen, by id. With these the tower reads nothing to
    /// find who it is for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    /// What he did, when it was a button and not words. Words are never read as one of these: text is only
    /// ever what he said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
}

/// Something he did with a button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Act {
    /// Yes or no to the request for leave that `re` names. `text` is then only what the chat shows.
    Verdict { allow: bool },
    /// One of the controls the tower answers itself.
    Control { control: crate::routing::Control },
    /// Start a new agent.
    Launch(Launch),
    /// He tapped one of the answers the message `re` offered: which, by its place in the list. `text` is its
    /// words, which are what the agent gets.
    Option { index: u32 },
}

/// A new agent to start: where, with what, and on what. Each is an id from a list the tower gave.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// The team it joins, by name, as the feed lists it.
    pub project: String,
    /// The coding tool, by id. Nothing is the computer's usual one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// What it should do, in his words. Empty for one that waits for his first message.
    #[serde(default)]
    pub task: String,
}

/// Where something he said stands with the tower.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    /// The tower has it, written down under its id, and is working out what to do with it. Nothing waits on
    /// that: what comes of it arrives in the feed.
    Taken,
    /// This is what came of it.
    Answered { reply: Reply },
    /// The tower would not take it. Nothing was done with it.
    Rejected { why: Rejected },
}

/// Why the tower would not take something he said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejected {
    /// It was left with the relay more than a week ago: too late to act on.
    TooOld,
    /// A reason this build does not know.
    #[serde(other)]
    Unknown,
}

/// What came of one thing he said, as the feed tells the radio that said it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settled {
    /// The utterance's id.
    pub id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// Whether a radio is there, as the radio itself says. The tower infers none of this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// He has reading aloud on: the radio takes transmissions and speaks them.
    #[serde(default)]
    pub on_call: bool,
    /// The chat is in front of him: what the radio is sent, he sees.
    #[serde(default)]
    pub showing_chat: bool,
}

/// A quick message: something he says all the time, sent with one tap. The label is what the button says; the
/// text is the whole instruction the agent gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quick {
    pub label: String,
    pub text: String,
}

/// The quick messages every tower offers. A tower adds his own after these (`quick` in its configuration).
pub fn quick_defaults() -> Vec<Quick> {
    [
        ("Screenshot", "Send me a picture of the result."),
        ("Test it", "Run the tests and fix what fails."),
        ("Double-check", "Verify it actually works before saying it's done."),
        ("Secure?", "Review what you changed for security problems and fix them."),
        ("Ship it", "Commit, push and deploy."),
        ("Status?", "What's done and what's left, in two sentences."),
    ]
    .into_iter()
    .map(|(label, text)| Quick { label: label.into(), text: text.into() })
    .collect()
}

/// The tower's answer to an utterance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reply {
    /// Words to speak back ("Sent to shop two."); empty for nothing.
    #[serde(default)]
    pub say: String,
    /// What he asked was done, whole, and `say` only confirms it. A radio with a screen already shows it
    /// (his words are in the chat), so it shows `say` only when this is false: then something did not go as
    /// he asked and he has to see it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Open the microphone again after speaking.
    #[serde(default)]
    pub listen: bool,
    /// The tower could not tell who his words are for: the agents it could be, likeliest first. A radio with a
    /// screen offers them to tap; `say` asks the same thing in words for one without.
    /// By agent id; each is one this tower has, or one of the utterance's `others`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<String>,
    /// His words are for this agent, which another of his computers has (one of the utterance's `others`).
    /// Nothing was delivered: the radio sends them there.
    /// By the agent's id, as `others` gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elsewhere: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feed_from_a_newer_tower_still_reads() {
        let feed: Feed = serde_json::from_str(r#"{"messages":[{"seq":1,"id":"a","at":0,"from":"shop one","group":"shop","kind":"celebration","text":"x","mood":"good"}],"roster":[{"callsign":"shop one","group":"shop","harness":"h","waiting":false,"reachable":true,"colour":"red"}],"seq":1,"weather":"fine"}"#).unwrap();
        assert_eq!(feed.messages[0].kind, Some(Kind::Unknown));
        let t: Transmission = serde_json::from_str(r#"{"id":"a","callsign":"shop one","kind":"celebration","say":"x"}"#).unwrap();
        assert_eq!(t.kind, Kind::Unknown);
    }
}
