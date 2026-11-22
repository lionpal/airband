//! What a desk hands a host to draw: plain records, made new each time they are asked for.
use super::*;

/// One team: a project and the agents working in it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskRoom {
    pub name: String,
    /// What the team is doing: the state of whichever of its agents most needs him.
    pub state: AgentState,
    pub agents: u32,
    /// Subagents running under its agents.
    pub subagents: u32,
    /// What one of its working agents is doing right now, as "callsign: a few words". Changes often; show it
    /// small and in passing, and never read it aloud.
    pub doing: Option<String>,
    /// Its agents, by callsign: for a list he can pick one from.
    pub members: Vec<DeskMember>,
    /// Its place in the host's list of team colours.
    pub colour: u8,
    /// How many things its agents have said that he has not had in front of him.
    pub unread: u32,
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
    /// The computer it is on, when he is paired with more than one; empty otherwise.
    pub station: String,
    /// The tower runs it itself, so his words reach it as his own. One that is not was started outside it
    /// and is only left messages from outside; the tower can take it over.
    pub held: bool,
    /// It is known to run in a terminal of his.
    pub terminal: bool,
    /// He can end it from here: the computer runs it, or its tool can put a session away.
    pub ends: bool,
    /// How many things it has said that he has not had in front of him.
    pub unread: u32,
    /// What it is waiting on him for: the short form of its newest question still open. Empty when it is
    /// not waiting.
    pub question: String,
    /// The message that asked it, to answer or to have read out. Empty when it is not waiting.
    pub question_id: String,
    /// The coding tool it is: `claude-code`, `codex`, `opencode`. A host shows that tool's mark.
    pub harness: String,
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
    /// One per project with an agent on the radio, by name. Not narrowed by what he is looking at.
    pub rooms: Vec<DeskRoom>,
    /// How many things agents have said that he has not had in front of him, over all of them. While the chat
    /// is on screen none of these are in what he is looking at.
    pub unread: u32,
    /// Someone here has stopped without finishing: a poke would do something.
    pub can_poke: bool,
    /// The quick messages to offer.
    pub quick: Vec<QuickMessage>,
    /// His messages still on their way.
    pub sending: Vec<Outgoing>,
    /// What each paired computer is called.
    pub stations: Vec<String>,
    /// Each team's place in the host's list of team colours, the same on every computer. A host keeps a copy
    /// where its notification extension can read it, and hands it to [`open_notice`].
    pub colours: std::collections::HashMap<String, u8>,
    /// His main computer, by name: the only one, or the one he chose. Nothing while he has several and has
    /// not chosen.
    pub main_computer: Option<String>,
    /// He has several computers and has not said which is the main one: ask him before anything is sent.
    pub choose_main: bool,
    /// What reads the agents' messages and makes the short of them, by name: on the first computer that says.
    /// Empty when none does.
    pub reads: String,
    /// Files he has attached and not yet sent, in the order he added them: on their way to the computer, there
    /// and waiting for his words, or failed.
    pub uploads: Vec<DeskUpload>,
    /// Something he said that could not be placed: who is it for? Shown until he picks one or lets it go.
    pub asking: Option<DeskAsk>,
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
    pub colour: u8,
}
/// One file he attached. `state` is `sending` (with `sent` of `total` bytes there so far), `ready` (it goes with
/// the next thing he says or sends) or `failed`.
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
/// How one computer was last reached. Each is a few words ("answered", "the relay says your computer is not
/// connected"), or empty when that way has not been tried or has only ever worked.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DeskReach {
    pub computer: String,
    pub relay: String,
    pub near: String,
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
    /// The folders inside, by name.
    pub folders: Vec<String>,
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
