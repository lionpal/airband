//! The chat as a tower gives it: its messages, the agents on the roster, what is waiting, and the notice that
//! wakes the phone.
use super::*;

/// Why something an agent said was not read out to him on a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotRead {
    /// The judge found nothing in it for his ears.
    Routine,
    /// Its agent said something newer before he heard it.
    Superseded,
    /// It was waiting on him, and he has since spoken to its agent, or the agent has gone back to work.
    Answered,
    /// He let the question go.
    Dismissed,
    /// It was waiting on him, and its agent has gone.
    AgentGone,
    /// It was said long before he turned reading aloud on.
    Stale,
    /// The request for leave it put to him is over.
    PermissionEnded,
}

/// An agent on another of his computers, as his screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Other {
    /// The agent's id on the computer that has it: what the tower answers with when the words are for it.
    pub id: String,
    pub callsign: String,
    pub group: String,
    /// Its team's id on the computer that has it: what is kept and sent about the team. `group` is only shown.
    pub team: crate::ids::TeamId,
    /// Its two characters, as that computer gave them.
    #[serde(default)]
    pub badge: String,
    /// The last thing it said, in its short form. Empty if it has said nothing.
    #[serde(default)]
    pub said: String,
    /// How long ago that was, in seconds.
    #[serde(default)]
    pub ago: u64,
}

/// A notification, as the tower seals it for the phone (`airband.relay/1`, direction "push"). Neither the relay
/// nor the push service can read it; the phone opens it and shows it. Everything the phone needs to show it and
/// to answer it is here, because the app may not be running.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// The message's id: what an answer is sent as the answer to (`re`).
    pub id: String,
    /// The agent's callsign as a screen shows it, and its team.
    pub from: String,
    /// The agent's id, the same as in the roster.
    #[serde(default)]
    pub agent: Option<String>,
    /// Its two characters.
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub group: String,
    /// Its team's id on that computer. `group` is only shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<crate::ids::TeamId>,
    /// What this computer is called.
    #[serde(default)]
    pub station: String,
    /// The short version of the message.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// Answers he can give with one tap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// It is a request for permission: the only answers are Allow, Deny, or his own words, which refuse it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    /// `text` is the opening of something longer. On a request for permission it means no Allow is offered from
    /// the lock screen: he reads all of what would run first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
    /// An agent is held, waiting on his answer to this: a request for leave, or a question put through its
    /// tool's own asking. Shown so it breaks through a focus he has on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub holding: bool,
    /// What this line asked of him is over without his answer from here: it takes the place of the
    /// notification that asked, quietly, with nothing left to answer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub over: bool,
    /// When the tool stops waiting on it, in seconds since 1970, if the tool said. Past it the notification is
    /// worth nothing, and is not kept for a phone that is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
}

/// What deserves a notification: a message that needs him, a report, or one no model read. Never chatter, a
/// stall, or his own words.
pub fn deserves_notice(kind: Option<Kind>) -> bool {
    matches!(kind, Some(Kind::NeedsYou | Kind::Report | Kind::Unjudged))
}

/// How many bytes of a picture go in one part.
pub const PICTURE_PART: usize = 120 * 1024;

/// A picture that goes with a message: one the agent looked at or made for him. The tower holds a compressed
/// copy (JPEG, 1600 pixels at most on its long side); `picture/<id>` has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// How large the copy is, so a client can decide when to fetch it.
    pub bytes: u32,
}

/// `GET picture/<id>?from=<byte>`: the next part of a picture's file. The radio asks again from `next` until
/// `done`. In parts because one sealed message through a relay has a size limit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PicturePart {
    /// Base64.
    pub data: String,
    pub next: u32,
    pub done: bool,
}

/// One line in the chat: something an agent said, cut down, or something he said.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Position in the tower's log; pass the highest one seen as `after`.
    pub seq: u64,
    pub id: String,
    /// Milliseconds since 1970.
    pub at: u64,
    /// What to show it as coming from: its sender's callsign as a screen shows it.
    pub from: String,
    /// Two characters for whoever said it, or for the one agent his message went to, while that agent is here.
    /// Empty once it has gone.
    #[serde(default)]
    pub badge: String,
    /// He said it. A client goes by this and never by what `from` reads.
    #[serde(default)]
    pub his: bool,
    /// What the team this belongs to is called, to show; empty for a message of his to several teams.
    pub group: String,
    /// That team's id on this computer: what views and colours go by. None when `group` is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<crate::ids::TeamId>,
    /// For his messages: the callsigns it went to, as a screen shows them.
    #[serde(default)]
    pub to: Vec<String>,
    /// For his messages: the ids of the agents it went to.
    #[serde(default)]
    pub to_agents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// The short version.
    pub text: String,
    /// There is more than `text` says: `message/<id>` has it.
    #[serde(default)]
    pub long: bool,
    /// Answers he can give with one tap, when the agent asked something he can answer by picking: short labels,
    /// in the order the agent put them (up to three from a message, up to four from a question its tool put). Sending one as an utterance with `re` set to this message answers it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Which of `options` the agent itself recommended, when it said so. Nothing otherwise: a client must not
    /// make one answer look preferred just because it is listed first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<u32>,
    /// The message holds something he asked to see or has to read whole before he can decide (a draft email, a
    /// plan, a list): what it is, in two or three words ("the refund email"). A client offers the whole message
    /// to read under that name. Nothing when a line is enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<String>,
    /// The agent says it will take this answer only from him in its own session, not from a message passed on
    /// to it. For an agent the tower does not run, an answer from a radio is such a message: a client says so,
    /// and offers to take the agent over, after which his words reach it as his own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own_session: bool,
    /// Something he said that is waiting its turn: the agent is in the middle of something, and the tower
    /// holds this until it stops. He can still take it back (`POST unsend`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub queued: bool,
    /// For something he said: when the agent it went to took it in, in milliseconds since 1970, once that is
    /// known. An agent in the middle of something may not read a message for a long while after it is delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<u64>,
    /// He took it back before it was delivered. A radio stops showing it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// This is a request for permission to do one thing, and its answer lets that thing happen on his computer.
    /// The only answers are the two given (allow, deny) or his own words, which refuse it and are passed on as
    /// the reason. A client must not offer a "do what you think best" answer on it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub permission: bool,
    /// Pictures that go with it, the ones the agent meant him to see first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<Picture>,
    /// The agent that said it: the same id as in the roster, and the same for as long as that agent runs.
    /// `from` is only what it is called, which can change and can later belong to another agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// When this message last changed (its short version was made, its sender was renamed), as a count that
    /// only goes up. Pass the feed's `rev` back as `since` to get everything changed after it.
    #[serde(default)]
    pub rev: u64,
    /// The web addresses that go with it: ones the agent wrote, and things its tool said it made (an artifact,
    /// a pull request). The short version leaves them out, so a client shows each as a card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<crate::links::Link>,
    /// It needs him and nothing has answered it yet: the agent is still stopped, he has not replied, and the
    /// agent has said nothing since. These are what an inbox shows, and what he is reminded of.
    #[serde(default)]
    pub open: bool,
    /// It was to be read out to him while he had reading aloud on, and was not: why. A client marks the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_read: Option<NotRead>,
    /// What airband says about the agent in its own voice, with the tool's words after it: the agent is held at
    /// its keyboard, or its turn failed. `text` already starts with it; a client may mark the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Note>,
}

/// Something airband says about an agent in its own voice, never in the agent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Note {
    /// The tool says the agent needs him at its keyboard.
    AtKeyboard,
    /// The tool says it asks him leave at its keyboard for something (with no word of what).
    LeaveAtKeyboard,
    /// The agent's turn failed and it stopped.
    TurnFailed,
    /// The tower moved the agent to this team, where its work belongs. The words are the name it had before.
    Moved,
}

impl Note {
    /// The line, with the tool's own words after it. The one place these words are written.
    pub fn said(self, text: &str) -> String {
        match self {
            Note::AtKeyboard | Note::LeaveAtKeyboard => format!("Needs you at the keyboard: {text}"),
            Note::TurnFailed => format!("Its turn failed, and it has stopped. {text}"),
            Note::Moved => format!("Moved here from {text}: what it is working on belongs to this team."),
        }
    }
}

/// What an agent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    /// It has a question open for him.
    Asking,
    /// It stopped without finishing or asking: a poke sets it going again.
    Stalled,
    /// It finished and reported.
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    /// The agent's id: stable while it runs. What messages refer to.
    pub id: String,
    /// What it is called, as its typed parts: its team's name, and what tells it from the others of its team
    /// (a number the tower handed out, or a name he gave it). Every form it is shown in is made from these.
    pub callsign: crate::routing::Callsign,
    /// Its team's id on this computer: what views, colours, moves and renames go by. The callsign's team name
    /// is only shown.
    pub team: crate::ids::TeamId,
    pub harness: String,
    /// Finished a turn and waiting on him.
    pub waiting: bool,
    /// What it is doing.
    pub state: AgentState,
    /// How many subagents it has running, when its harness reports them.
    #[serde(default)]
    pub subagents: u32,
    /// What it is doing right now, in a few words, while it is working and that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// The tower runs this agent itself and holds its input, so what he says reaches it as his own words. One
    /// he started in a terminal of his own is reached as a message from outside, which it treats with more care.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    /// It runs in a terminal of his: the tower types into its pane, or it is a session its tool started at its
    /// command line (for a tool whose sessions are listed only from there). False says nothing of where one
    /// runs that the tower does not run: another tool's session may be in that tool's own app.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminal: bool,
    /// He can end it from a phone: the tower runs it, or its tool has a way to put a session away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ends: bool,
    /// The model it runs on, as a person would say it ("Opus 4.7"), when its harness can tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How hard it is set to think ("high"), when its harness has such a setting and can tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Whether his words can reach it.
    pub reach: crate::radio::Reach,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Feed {
    /// Always [`crate::radio::SCHEMA`].
    pub schema: String,
    /// The log's change count as of this answer: pass it back as `since`. Lower than the one a client holds
    /// means the tower's log was reset, and the client should start again from nothing.
    #[serde(default)]
    pub rev: u64,
    pub messages: Vec<Message>,
    pub roster: Vec<RosterEntry>,
    /// The log's latest position, whether or not this scope had anything new.
    pub seq: u64,
    /// The quick messages to offer: the standard ones, then his own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<Quick>,
    /// What this tower's computer is called ("mini", "macbook"), for a client paired with more than one.
    #[serde(default)]
    pub station: String,
    /// The identifier of this tower's computer's model, as its system gives it: what the radio shows the
    /// computer's shape by. Empty when the system gives none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Addresses a radio may try to reach this tower at without the relay, when it is on the same network:
    /// `http://<address>:<port>`. A radio paired through a relay posts the same sealed request it would give
    /// the relay to `<address>/v1/sealed/<channel>` and gets the sealed answer back, so nothing on that network
    /// can read it or stand in for the tower. Empty when the tower offers none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct: Vec<String>,
    /// What reads the agents' messages on this tower and makes the short of them, by its provider's name for
    /// showing. Empty when nothing does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reads: String,
    /// Why what reads could not, the last time it was asked. Nothing while it works. Until it does again he
    /// gets each message as it was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads_failed: Option<crate::providers::Trouble>,
    /// The ids of every message that needs him and is still unanswered, whether or not it is in `messages`:
    /// the whole inbox, current as of this answer.
    #[serde(default)]
    pub open: Vec<String>,
    /// A number that changes whenever anything in `roster` does. Pass it back as `roster=` and a waiting
    /// `GET feed` also returns when the roster changes, not only when a message does.
    #[serde(default)]
    pub roster_rev: u64,
    /// What has come of things this radio said, since the change count it asked from: each under its
    /// utterance's id. One may be told more than once; a radio that has dealt with an id ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Settled>,
}
