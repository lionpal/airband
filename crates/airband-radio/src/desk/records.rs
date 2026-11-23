//! What a desk hands a host to draw: plain records, made new each time they are asked for.
use super::*;

/// One team: a project and the agents working in it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskRoom {
    /// The room's id: one team on one computer. What every call about the room takes.
    pub id: String,
    /// What the team is called, to show.
    pub name: String,
    /// What the team is doing: the state of whichever of its agents most needs him.
    pub state: AgentState,
    pub agents: u32,
    /// Subagents running under its agents.
    pub subagents: u32,
    /// What one of its working agents is doing right now, as "callsign: a few words". Changes often; show it
    /// small and in passing, and never read it aloud.
    pub doing: Option<String>,
    /// Its agents, each known by its id: for a list he can pick one from.
    pub members: Vec<DeskMember>,
    /// Its place in the host's list of team colours. Nothing until the phone has given its team one.
    pub colour: Option<u8>,
    /// How many things its agents have said that he has not had in front of him.
    pub unread: u32,
    /// Some agent in it is working, whatever the team's own state says.
    pub working: bool,
}
/// One agent, as a row in a list of them.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskMember {
    pub id: String,
    pub callsign: String,
    pub state: AgentState,
    pub subagents: u32,
    pub doing: Option<String>,
    /// Two characters to stand for it: "S3", "CP".
    pub badge: String,
    /// The model it runs on ("Opus 4.7") and how hard it is set to think ("high"). Each empty when not known.
    pub model: String,
    pub effort: String,
    /// What tells it from the others of its team, for a place that already shows the team: "two", "removals".
    pub number: String,
    /// The computer it is on, by its [`DeskComputer::id`], when he is paired with more than one; empty otherwise.
    pub station: String,
    /// The tower runs it itself, so his words reach it as his own. One that is not was started outside it
    /// and is only left messages from outside; the tower can take it over.
    pub held: bool,
    /// It is known to run in a terminal of his.
    pub terminal: bool,
    /// He can end it from here: the computer runs it, or its tool can put a session away.
    pub ends: bool,
    /// He can stop what it is doing now: it is at work, and run by its computer.
    pub can_stop: bool,
    /// How many things it has said that he has not had in front of him.
    pub unread: u32,
    /// What it is waiting on him for: the short form of its newest question still open. Empty when it is
    /// not waiting.
    pub question: String,
    /// The message that asked it, to answer or to have read out. Empty when it is not waiting.
    pub question_id: String,
    /// The coding tool it is: `claude-code`, `codex`, `opencode`. A host shows that tool's mark.
    pub harness: String,
    /// What that tool is called, from the catalogue; empty for a tool it does not list.
    pub tool_name: String,
}
/// What he is looking at: one team, one agent in it, or (both empty) everyone. The desk keeps it valid: a team
/// that has gone is let go of once the computers can say so.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct DeskFocus {
    pub room: Option<String>,
    pub agent: Option<String>,
}
/// What a new agent starts on without his choosing: the computer and the coding tool, each when there is only
/// one to choose. Nothing where he must choose.
#[derive(Debug, Clone, PartialEq, Default, uniffi::Record)]
pub struct NewAgentDefaults {
    pub computer: Option<String>,
    pub tool: Option<CodingTool>,
}
/// Everything a chat screen shows, for the team or the agent he is looking at (or all of them).
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskView {
    /// At least one computer is paired.
    pub paired: bool,
    /// At least one of them can be reached.
    pub online: bool,
    /// None has answered yet and at least one is still being tried for the first time: show that it is
    /// loading, not that the computer cannot be reached.
    pub connecting: bool,
    /// Every computer that can be reached is being reached straight, on its own network: he is at home with
    /// them, and nothing is going by the relay.
    pub near: bool,
    /// The computers that cannot be reached right now, by name, when he is paired with more than one: so he is
    /// told which. Empty with one computer (then `online` says it all).
    pub away: Vec<String>,
    /// The feed: reports, questions and what he said, oldest first, without the chatter not worth his attention.
    /// A message with `open` is a question still waiting on him; one with `stalled` is from an agent that stopped.
    pub feed: Vec<ChatMessage>,
    /// One per team with an agent on the radio, each known by its room id ([`DeskRoom::id`]). Not narrowed by what he is looking at.
    pub rooms: Vec<DeskRoom>,
    /// How many things agents have said that he has not had in front of him, over all of them. While the chat
    /// is on screen none of these are in what he is looking at.
    pub unread: u32,
    /// He has more than one computer: the screens say which one a thing is on, and ask which when it matters.
    pub several_computers: bool,
    /// How many of those are outside the team he is in (the one he picked, or the team of the agent he is
    /// looking at): shown on the way back to all of them.
    pub unread_elsewhere: u32,
    /// Someone here has stopped without finishing: a poke would do something.
    pub can_poke: bool,
    /// The quick messages to offer.
    pub quick: Vec<QuickMessage>,
    /// His messages still on their way.
    pub sending: Vec<DeskOutgoing>,
    /// Each paired computer, in the order they were paired.
    pub stations: Vec<DeskComputer>,
    /// Each team's place in the host's list of team colours, the same on every computer. A host keeps a copy
    /// where its notification extension can read it, and hands it to [`open_notice`].
    pub colours: std::collections::HashMap<String, u8>,
    /// His main computer, by its [`DeskComputer::id`]: the only one, or the one he chose. Nothing while he has
    /// several and has not chosen.
    pub main_computer: Option<String>,
    /// He has several computers and has not said which is the main one: ask him before anything is sent.
    pub choose_main: bool,
    /// Files he has attached and not yet sent, in the order he added them: on their way to the computer, there
    /// and waiting for his words, or failed.
    pub uploads: Vec<DeskUpload>,
    /// Something he said that could not be placed: who is it for? Shown until he picks one or lets it go.
    pub asking: Option<DeskAsk>,
    /// What his computers are to be told and not all have yet, each with the computers still without it.
    pub intents: Vec<DeskIntent>,
    /// What he is looking at now.
    pub focus: DeskFocus,
    /// What a new agent starts on when he is not asked.
    pub new_agent: NewAgentDefaults,
    /// The coding tools he uses, as the catalogue describes them, the one he uses most first.
    pub tools: Vec<CodingTool>,
    /// What to say when a computer cannot be reached: which ones, when he has several.
    pub cannot_reach: String,
}
/// Which thing his computers are to be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum IntentKind {
    /// What writes his summaries.
    Reads,
    /// Where this device can be woken.
    Push,
    /// The coding tools he uses.
    Tools,
}
/// One thing his computers are to be told, and the computers that do not have it yet.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskIntent {
    pub kind: IntentKind,
    pub computers: Vec<DeskReachLine>,
}
/// One computer that does not have it yet, by id and name, and why.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskReachLine {
    pub computer: String,
    pub name: String,
    pub state: DeskDelivery,
}
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum DeskDelivery {
    /// It is sent when that computer is reached.
    Waiting,
    /// That computer said no, or could not take it: why, in words for him.
    Refused { reason: String },
}
/// His words, waiting to be told who they are for.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskAsk {
    /// What he said (or the words on the button, for a quick message).
    pub text: String,
    /// Who it could be for: the tower's guesses first, then everyone else he can see.
    pub choices: Vec<DeskChoice>,
}
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskChoice {
    /// The agent's id: what [`Desk::choose`] takes.
    pub id: String,
    pub callsign: String,
    pub badge: String,
    pub colour: Option<u8>,
}
/// One file he attached: how far it has got ([`UploadState`]), with `sent` of `total` bytes there so far.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskUpload {
    pub id: String,
    pub name: String,
    pub sent: u64,
    pub total: u64,
    pub state: UploadState,
}
/// How far a file he attached has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UploadState {
    /// On its way to his computer.
    Sending,
    /// His computer has it: it goes with the next thing he says.
    Ready,
    /// It did not get there.
    Failed,
}
/// How one computer was last reached each way, as the record has it: a few words ("Answered.", "The relay says
/// your computer is not connected."), or nothing when the record has nothing for that way. The record keeps
/// failures and what he sent, so nothing means it has not been tried, or has not failed, since the record began.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskReach {
    /// The computer, by its [`DeskComputer::id`], and what it is called to him.
    pub id: String,
    pub computer: String,
    pub relay: Option<String>,
    pub near: Option<String>,
}
/// One folder inside another: its name, and its whole path to pass back.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskFolder {
    pub name: String,
    pub path: String,
}
/// The folders inside one of his on a computer.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskFolders {
    /// This folder's whole path, to pass back.
    pub path: String,
    /// The same as he would read it: "~/Companies".
    pub shown: String,
    /// The folder above, to pass back; nothing at his home folder.
    pub up: Option<String>,
    /// The folders inside, each with its whole path to pass back.
    pub folders: Vec<DeskFolder>,
    /// The team that works in this folder, or the name the computer would give one there.
    pub team: String,
    /// A team works here already: a new agent joins it.
    pub has_team: bool,
}
/// A folder the coding tools on a computer have had sessions in.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskRecent {
    /// Its whole path, to pass back.
    pub folder: String,
    /// The same as he would read it: "~/Companies/shop".
    pub shown: String,
    /// The team an agent started there is in: the one that already works there, or the folder's own name.
    pub name: String,
    /// There is a team in this folder already.
    pub team: bool,
    /// The coding tools that have had sessions there, as they are called.
    pub tools: Vec<String>,
    /// When the last of those sessions was, in milliseconds since 1970; 0 when no tool has a record of one.
    pub at: u64,
}

/// Which team he moved an agent to: a room on his screen ([`DeskRoom::id`]), or a new team of a name he gave.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum DeskTeamPick {
    Room { room: String },
    New { name: String },
}

/// One paired computer: what every call that names a computer takes (`id`), and what it is called to him.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeskComputer {
    /// The same every time, from its pairing; nothing secret.
    pub id: String,
    /// Its own name once it has said one, or "Your computer", or its place in the list.
    pub name: String,
}

/// One of his messages on its way, as a host shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskOutgoing {
    /// Its id: what [`Desk::send_again`] and [`Desk::discard_unsent`] are given.
    pub id: String,
    /// What it shows as: a quick message's button words, or what he typed.
    pub shown: String,
    /// How far it has got.
    pub stage: crate::Stage,
    /// It is no longer being tried, and why, in a sentence. He can send it again or let it go.
    pub failed: Option<String>,
    /// While it is still being tried and has been for a while: how the tries are going, in sentences ("Trying.
    /// The relay says your computer is not connected. 3 tries.").
    pub trying: Option<String>,
}

impl From<&crate::Outgoing> for DeskOutgoing {
    fn from(o: &crate::Outgoing) -> Self {
        // how the tries go is worth showing only once they have gone on for ten seconds and said something
        let trying = (o.failed.is_none() && !o.trail.is_empty() && o.last_tried >= o.at + 10_000)
            .then(|| format!("Trying. {}{}", o.trail.join(" "), if o.tries > 1 { format!(" {} tries.", o.tries) } else { String::new() }));
        DeskOutgoing { id: o.id.clone(), shown: o.shown(), stage: o.stage, failed: o.failed.clone(), trying }
    }
}
