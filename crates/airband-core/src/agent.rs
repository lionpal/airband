//! `airband.agent/1`: what an agent's harness tells the tower, and how the tower can speak back to it.
//!
//! One JSON document per event, `POST /v1/agent/events`. An adapter is whatever turns a harness's own
//! notifications into these: `airband hook <tool>` does it for a coding tool with hooks; `airband report` does it from any
//! script. The adapters are the tower's own binary, so this is read as written: a `type` or `kind` the tower
//! does not have is refused, never skipped.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "airband.agent/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Always `airband.agent/1`.
    pub schema: String,
    /// Random, made when the event is: a report sent again after a timeout is the same report, and the tower
    /// takes it once.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub agent: Agent,
    pub event: Event,
    /// How it came to be reported, when the reporter says. A tool that is heard two ways (its hooks and its
    /// own server) is heard through its hooks once they have reported, and this is how the tower knows they have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
}

impl AgentEvent {
    /// A new event of this agent's, with a fresh id.
    pub fn new(agent: Agent, event: Event) -> Self {
        AgentEvent { schema: SCHEMA.into(), id: crate::encode::random_hex(12), agent, event, via: None }
    }
    /// Reported this way.
    pub fn via(mut self, via: Via) -> Self {
        self.via = Some(via);
        self
    }
}

/// How an event came to be reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// The tool's own hook ran `airband hook`.
    Hook,
}

/// Who is reporting. Sent in full with every event, so the tower can restart without losing anyone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Stable for the life of the agent's session; unique on this machine.
    pub id: String,
    /// Which coding tool runs it, by the id the tool's catalogue row gives it. For display and for adapters; the
    /// tower has no rules per tool.
    pub harness: String,
    /// The folder it works in. Its last part names the callsign unless `name` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The spoken name to use instead of the folder's ("shop" → "Shop two").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The name he gave this session in the harness itself ("shop 2", "warehouse removals"), when he gave
    /// one. The tower uses it as the callsign in place of a number handed out in order of arrival.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The harness process; when it dies the agent is dropped. Without it the agent stays until it signs off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// How to get a message to it. Without one the agent can be heard but not answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver: Option<Deliver>,
    /// Anything else an adapter wants on record about the agent: its model, a branch, a ticket, what kind of
    /// agent it is. The tower keeps it and shows it; it has no rules about what is in it. This is where a new
    /// kind of agent puts what makes it different, without a new protocol version.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub meta: serde_json::Map<String, serde_json::Value>,
}

/// A way to put text in front of an agent as its next instruction. Its debug form never shows a socket's token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Deliver {
    /// A socket on this machine that takes a turn as one line of JSON ([`SocketLine`]), after a line that shows
    /// `token` when it asks for one.
    Socket {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// Run a command with the message on its standard input. `AIRBAND_AGENT_ID` is set in its environment.
    Command { argv: Vec<String> },
    /// The tower started this agent and holds its input: his words are written there as his own turn, with
    /// nothing around them. Nothing else can reach it this way.
    Held,
    /// Type the message into a terminal and press Enter: works for any harness that runs in one. `terminal`
    /// is its row in `catalog/terminals/`, `pane` the pane there.
    Keys { terminal: String, pane: String },
    /// Through the agent's own tool, by whatever way that tool takes a turn of his into a session that is
    /// already open (its queue, its server). The tower's adapter for `name` knows how.
    Harness { name: String, session: String },
}

impl std::fmt::Debug for Deliver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Deliver::Socket { path, token } => f.debug_struct("Socket").field("path", path).field("token", &token.as_deref().map(crate::redact::Shown)).finish(),
            Deliver::Command { argv } => f.debug_struct("Command").field("argv", argv).finish(),
            Deliver::Held => f.write_str("Held"),
            Deliver::Keys { terminal, pane } => f.debug_struct("Keys").field("terminal", terminal).field("pane", pane).finish(),
            Deliver::Harness { name, session } => f.debug_struct("Harness").field("name", name).field("session", session).finish(),
        }
    }
}

/// A harness asks, on its agent's behalf, whether the agent may do something it needs permission for. Posted to
/// `/v1/agent/permission`; the answer is then fetched from `/v1/agent/permission/<id>`. The tower puts it to him
/// as a question with Allow and Deny, and never decides it itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAsk {
    /// Always `airband.agent/1`.
    pub schema: String,
    pub agent: Agent,
    /// What the agent wants to do, as words that finish "wants to …": "run `npm run deploy`". This is what he
    /// reads before he says yes, so it must say exactly what will happen, not a summary of it.
    pub what: String,
    /// Everything about the request, for when he wants to see all of it: the tool and its whole input.
    #[serde(default)]
    pub detail: String,
    /// Set when the agent is not asking leave to do something but asking him something, through a tool of its
    /// harness that would otherwise put the question on its own screen. It is put to him as a question, with
    /// these answers to tap if it offered any, and whatever he answers goes back as its reply: there is nothing
    /// to allow or deny.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Asked>,
}

/// A question an agent put through its harness's own asking tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

/// Where a request for leave stands, for the harness that asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leave {
    /// No answer yet: ask again.
    Pending,
    /// His answer.
    Allow,
    Deny,
    /// No radio is there to ask, so the harness should ask in its own way at once.
    Nobody,
    /// The tower no longer knows this request.
    #[default]
    Gone,
}

/// Where a permission request stands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionState {
    /// For asking again.
    #[serde(default)]
    pub id: String,
    pub state: Leave,
    /// With `deny`: his own words, if he gave a reason. The agent should be told them.
    #[serde(default)]
    pub message: String,
    /// With `pending`: how many seconds in all the harness should keep waiting before asking in its own way.
    #[serde(default)]
    pub wait_seconds: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    /// The agent exists. Any other event also checks it in.
    Checkin,
    /// It has been given something to do; whatever it last said is stale.
    Working,
    /// It finished a turn and this is what it wrote. `kind` is set only when the harness knows whether it is
    /// blocked; otherwise the tower works it out.
    Said {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<crate::radio::Kind>,
        /// The tool's own id for the turn this ends, when it has one. A tool that can be heard from two ways
        /// (its hooks and its server) says the same turn by both: the tower takes what a turn said once.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<String>,
    },
    /// It needs him at the keyboard (a permission prompt, a login): something a spoken reply cannot do.
    Notice { text: String },
    /// It looked at a picture, or made one for him: a file on the tower's machine. The tower keeps a small copy
    /// and shows it with the next thing the agent says. `shown` means the agent meant him to see it (it sent
    /// him the file), not only that it looked at it itself.
    Picture {
        path: String,
        #[serde(default)]
        shown: bool,
    },
    /// Its tool made something that lives at a web address (it published an artifact), and says so itself:
    /// where it is, what it is called, and which kind of thing it is by the id of that kind's row
    /// (`links.rs`). The tower shows it with the next thing the agent says.
    Made {
        address: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
    /// It started a subagent to do part of the work, or one of its subagents finished. For showing how much is
    /// going on under one agent; nothing is decided from it.
    Subagent {
        /// False when one starts, true when one finishes.
        #[serde(default)]
        done: bool,
        /// What the subagent was asked to do, in a few words.
        #[serde(default)]
        what: String,
    },
    /// What it is doing right now, in a few words ("Running the tests"). Shown while it works and never kept:
    /// it is not something it said. A harness that cannot be asked sends these as it goes.
    Activity {
        #[serde(default)]
        text: String,
    },
    /// The model it is running on and how hard it is set to think, as a person would say them ("Opus 4.7",
    /// "high"). Shown beside its name. Sent when known and again when either changes.
    Model {
        #[serde(default)]
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    /// Its turn is over and it wrote nothing: it was stopped, it had nothing to say, or the turn failed. It
    /// is waiting from now, like after anything it says. `why` is set when the turn failed, in the tool's own
    /// words, and he is told.
    Stopped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
    /// It has taken in what the tower gave it under `receipt` (`airband_core::agent::Deliver::Held`): the
    /// messages that delivery carried are marked read. For a tool whose own record does not name the receipt.
    Took { receipt: String },
    /// It is gone.
    Signoff,
}

/// What it wrote and what it needs him for are words from his session: their debug form says how long they
/// are, never what they say. Every other kind is shown as it goes on the wire.
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::redact::Withheld;
        match self {
            Event::Said { text, kind, turn } => f.debug_struct("Said").field("text", &Withheld(text)).field("kind", kind).field("turn", turn).finish(),
            Event::Notice { text } => f.debug_struct("Notice").field("text", &Withheld(text)).finish(),
            other => f.write_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_a_kind_it_does_not_have() {
        let j = r#"{"schema":"airband.agent/1",
            "agent":{"id":"a1","harness":"codex","cwd":"/x/shop","deliver":{"kind":"keys","terminal":"tmux","pane":"%3"},"meta":{"model":"gpt-5","branch":"fix/retry"}},
            "event":{"type":"said","text":"Done.","kind":"report"}}"#;
        let e: AgentEvent = serde_json::from_str(j).unwrap();
        assert_eq!(e.agent.deliver, Some(Deliver::Keys { terminal: "tmux".into(), pane: "%3".into() }));
        assert_eq!(e.agent.meta["model"], "gpt-5");
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: Some(crate::radio::Kind::Report), turn: None });
        for odd in [
            r#"{"schema":"airband.agent/1","agent":{"id":"a1","harness":"x","deliver":{"kind":"carrier-pigeon"}},"event":{"type":"checkin"}}"#,
            r#"{"schema":"airband.agent/1","agent":{"id":"a1","harness":"x"},"event":{"type":"molting"}}"#,
            r#"{"schema":"airband.agent/1","agent":{"id":"a1","harness":"x"},"event":{"type":"said","text":"Done.","kind":"celebration"}}"#,
        ] {
            assert!(serde_json::from_str::<AgentEvent>(odd).is_err(), "{odd}");
        }
    }
}

/// The lines a [`Deliver::Socket`] takes, one JSON object to a line: the shape the variant is defined by. Its
/// debug form shows neither the token nor his words, only how long they are.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SocketLine {
    /// Shown first, when the socket asks for a token.
    Auth { token: String },
    /// His words, as a turn; `uuid` is the id the session writes on its record of it, which marks it read.
    User { uuid: String, message: SocketTurn },
}

impl std::fmt::Debug for SocketLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketLine::Auth { token } => f.debug_struct("Auth").field("token", &crate::redact::Shown(token)).finish(),
            SocketLine::User { uuid, message } => f.debug_struct("User").field("uuid", uuid).field("role", &message.role).field("characters", &message.content.chars().count()).finish(),
        }
    }
}

/// A turn on a [`SocketLine::User`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketTurn {
    pub role: SocketRole,
    pub content: String,
}

/// Who a [`SocketTurn`] is from: always him.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketRole {
    User,
}

impl SocketLine {
    /// The lines to write for `words`, under the id `uuid`, with the token first when there is one.
    pub fn lines(token: Option<&str>, uuid: &str, words: &str) -> String {
        let auth = token.map(|t| SocketLine::Auth { token: t.to_string() });
        let user = SocketLine::User { uuid: uuid.to_string(), message: SocketTurn { role: SocketRole::User, content: words.to_string() } };
        auth.into_iter().chain([user]).map(|l| serde_json::to_string(&l).unwrap_or_default() + "\n").collect()
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;

    #[test]
    fn a_socket_turn_is_written_as_its_lines_say() {
        let text = SocketLine::lines(Some("t"), "u1", "hi");
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0], serde_json::from_str::<serde_json::Value>(r#"{"type":"auth","token":"t"}"#).unwrap());
        assert_eq!(lines[1], serde_json::from_str::<serde_json::Value>(r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"hi"}}"#).unwrap());
        assert_eq!(SocketLine::lines(None, "u1", "hi").lines().count(), 1);
    }
}
