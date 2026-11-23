//! What a radio asks of a tower besides talking: accounts, teams and folders, an agent stopped or taken over, what
//! reads for it, where to wake the phone.
use super::*;

/// `POST attachment`: one part of a file he is attaching. Parts are sent in order; the answer says how many bytes
/// the tower now has, which is where the next part starts. One sent again lands in the same place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentPart {
    /// Random hex, chosen by the radio, the same for every part of the file. An utterance names it.
    pub id: String,
    /// What the file is called. The tower keeps the extension and makes the rest safe.
    pub name: String,
    /// Where in the file this part starts.
    pub from: u64,
    /// The bytes, base64: 120 KB of them at most, to fit one sealed message.
    pub data: String,
    /// How long the whole file is. The parts may arrive in any order and several at once, and the file is
    /// whole when every byte of it has come.
    pub total: u64,
}

/// One login a harness on this computer can run under: a subscription or plan with its own allowance. The tower
/// never sends anything secret about it, only what to call it and how much of it is used.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// What to pass back to switch to it. Means something only to the adapter that listed it.
    pub id: String,
    /// The harness it belongs to ("claude-code").
    pub harness: String,
    /// What to show him: usually the email it signs in with, or a key's last four characters. Only to show:
    /// two computers' accounts are the same one by [`Account::identity`], never by this.
    pub label: String,
    /// Who it is, as its tool records it.
    #[serde(default)]
    pub identity: crate::identity::AccountIdentity,
    /// It is the one every session of that harness is using now.
    #[serde(default)]
    pub live: bool,
    /// Its allowances, the longest window first. Empty when they could not be read.
    #[serde(default)]
    pub limits: Vec<Limit>,
    /// Why it cannot be used or checked, in words for him. Nothing when all is well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// The label is usually his email: its debug form says whether there is one, never what it is.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").field("id", &self.id).field("harness", &self.harness).field("label", &crate::redact::Shown(&self.label)).field("identity", &self.identity).field("live", &self.live).field("limits", &self.limits).field("problem", &self.problem).finish()
    }
}

/// One allowance of an account: how much is used and when it starts again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// What the window is called: "week", "5 hours".
    pub name: String,
    /// From 0 to 1.
    pub used: f64,
    /// When it starts again, in milliseconds since 1970, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<u64>,
}

/// `GET accounts`: every account the tower can switch between, for every harness that has them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Accounts {
    pub schema: String,
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Something he should know about the list as a whole: why it is empty, or why it cannot be switched here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When the allowances were read, in milliseconds since 1970.
    #[serde(default)]
    pub checked_at: u64,
}

/// `POST accounts/use`: make this account the one its harness's sessions use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseAccount {
    pub harness: String,
    pub id: String,
}

/// `POST team`: put an agent in another team, one that exists or a new one. The agent keeps a name he gave it
/// and takes the next free number otherwise; the answer is [`Moved`] (what it is called now, and the id of the team
/// it is in), or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveAgent {
    /// The agent's id, as the roster gives it.
    pub agent: String,
    pub to: TeamPick,
}

/// The answer to [`MoveAgent`]: what the agent is called now, as its typed parts, and its team's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub callsign: crate::routing::Callsign,
    pub team: crate::ids::TeamId,
}

/// Which team an agent goes to: one of his teams on that computer, by its id, or a new one he named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamPick {
    Team { id: crate::ids::TeamId },
    New { name: String },
}

/// `POST folders`: the folders inside one of his folders on this computer, for choosing where a new team
/// works. `under` is a folder's whole path; empty is his home folder, where the walk starts. Any folder
/// on the computer can be listed, up to the top and into other disks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoldersAsk {
    #[serde(default)]
    pub under: String,
}

/// One folder inside another: its name, and its whole path as the computer writes it, to pass back.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub path: String,
}

/// The answer: where this is, as a whole path and as he would read it ("~/Companies"), the folder above it
/// (nothing at the top of the disk), and the folders inside it, without the hidden ones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Folders {
    pub path: String,
    pub shown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<String>,
    pub folders: Vec<Folder>,
    /// The team that works in this folder, or the name a team there would have.
    #[serde(default)]
    pub team: String,
    /// A team works here already: a new agent joins it.
    #[serde(default)]
    pub has_team: bool,
}

/// `GET recent`: the folders the coding tools on this computer have had sessions in, newest first, for
/// choosing where a new agent works without walking there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub folders: Vec<RecentFolder>,
}

/// One of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentFolder {
    /// Its whole path, to pass back.
    pub folder: String,
    /// The same as he would read it: "~/Companies/shop".
    pub shown: String,
    /// The team an agent started there is in: the one that already works there, or else the folder's own name.
    pub name: String,
    /// The id of the team that already works in this folder, when there is one.
    #[serde(default)]
    pub team: Option<crate::ids::TeamId>,
    /// The coding tools that have had sessions there, by id.
    #[serde(default)]
    pub tools: Vec<String>,
    /// When the last of those sessions was, in milliseconds since 1970. 0 when no tool has a record of one.
    #[serde(default)]
    pub at: u64,
}

/// `POST project`: make a new team, or start another agent in one that is there. Agents started in it work
/// in `folder` (a whole path, anywhere on the computer), which is made first when `make` is set. The tower then starts an agent there with
/// nothing to do yet, if it runs agents itself. The answer is a [`ProjectMade`], or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProject {
    /// The team's name as he wrote it; the tower makes it a team's name. With `make`, the new folder is
    /// called exactly this.
    pub name: String,
    /// The folder its agents work in, or with `make` the folder the new one is made inside.
    pub folder: String,
    #[serde(default)]
    pub make: bool,
    /// The coding tool its first agent is, by id; the tower's usual one when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// What `POST team/rename` answers: the name the team now has, as the tower writes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRenamed {
    pub team: String,
}

/// What `POST project` answers: what to tell him, and the team the agent is in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMade {
    pub say: String,
    /// The team's name, as the tower writes it.
    pub team: String,
    /// The team's id on that computer.
    pub id: String,
}

/// `POST takeover`: have the tower take over an agent he started in a terminal of his own. That terminal's
/// session is ended and the same conversation carried on in a process the tower holds, so his words reach it
/// as his own. The answer is `Done`, or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeOver {
    pub agent: String,
}

/// `POST stop`: stop what an agent is doing, now. Only for one the tower runs itself (`held`). Whatever he said
/// to it meanwhile, and has not taken back, it then reads together as its next turn. `Done`, or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopAgent {
    pub agent: String,
}

/// `POST end`: end an agent the tower runs itself. Its input is closed and it finishes and signs off; what it
/// has done stays done. `Done`, or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndAgent {
    pub agent: String,
}

/// The words on the two buttons that answer a request for leave, and what the chat shows for each. They are
/// never read: the answer itself is [`Act::Verdict`].
pub const ALLOW: &str = "Allow";

pub const DENY: &str = "Deny";

/// `POST team/rename`: he renamed one of his teams on this computer. Only what it is called changes: its agents,
/// folder and colour stay with it. [`TeamRenamed`] (the name it now has), or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameTeam {
    /// The team's id on that computer.
    pub team: crate::ids::TeamId,
    pub to: String,
}

/// `POST taken`: the names agents go by on his other computers, as his radio shows them ("shop 1"). A tower
/// gives none of its own agents a numbered name on this list, and moves one that already has one to the next
/// free number, so two agents on his screen are never called the same. A radio tells only the later of two
/// computers, so they do not both move. `{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Taken {
    pub callsigns: Vec<crate::routing::Callsign>,
}

/// `POST dismiss`: he does not mean to answer this question. It stops being open: it no longer waits on him
/// in the chat, on its team or aloud. The agent is told nothing. `Done`, or a [`wire::Refusal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dismiss {
    pub message: String,
}

/// `POST unsend`: take back something he said that is still `queued`. `Done`, or a [`wire::Refusal`] when it has gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unsend {
    pub message: String,
}

/// `POST reads`: choose what reads for this tower, from a radio. `kind` and `offer` name one of the tower's
/// own list (`ReadsOffer::provider` and `ReadsOffer::offer`): something that reads and that a person can
/// choose, never a command of the radio's own naming. `key` is its provider's key, or empty to use one this
/// tower already keeps for that provider. `set` is what else it is told.
/// The tower tries it with a real call first, and answers `{ reads }` (its name) or `{ error }`.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SetReads {
    pub kind: String,
    pub offer: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub set: std::collections::BTreeMap<String, String>,
}

/// Written out for debugging: whether a key was given, never the key, and the names of what is set.
impl std::fmt::Debug for SetReads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetReads").field("kind", &self.kind).field("offer", &self.offer).field("key", &crate::redact::Shown(&self.key)).field("set", &self.set.keys().collect::<Vec<_>>()).finish()
    }
}

/// Which of a tower's offers: the provider and the offer, both by id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferName {
    pub provider: String,
    pub offer: String,
}

/// `GET reads`: what can read for this tower, from the tower's own catalogue and in the order to show it,
/// and which of them reads now (`now`; nothing when nothing does). A radio offers these and sends one back
/// by its ids (`SetReads`): it never has to match a name of its own against the tower's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadsOffered {
    #[serde(default)]
    pub now: Option<OfferName>,
    /// Why what reads now could not, the last time it was asked. Nothing while it works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<crate::providers::Trouble>,
    #[serde(default)]
    pub offers: Vec<ReadsOffer>,
}

/// One thing that can read for a tower.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadsOffer {
    /// The provider's id. One key serves everything a provider offers.
    pub provider: String,
    /// The offer's id among that provider's.
    pub offer: String,
    pub label: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub price: String,
    /// One of the few shown before the rest.
    #[serde(default)]
    pub preferred: bool,
    /// It runs on that computer and costs him nothing.
    #[serde(default)]
    pub free: bool,
    /// The coding tool whose sign-in on that computer it uses, by that tool's id. Empty when it uses none.
    #[serde(default)]
    pub login: String,
    /// It takes a key.
    #[serde(default)]
    pub key: bool,
    /// The tower already keeps a key for it. Only that it has one: never the key.
    #[serde(default)]
    pub has_key: bool,
    #[serde(default)]
    pub key_name: String,
    #[serde(default)]
    pub keys_at: String,
    #[serde(default)]
    pub asks: Vec<ReadsAsk>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadsAsk {
    pub field: String,
    pub question: String,
    #[serde(default)]
    pub usual: String,
}

/// `POST tools`: the coding tools he uses, by the ids their rows give them, the one he uses most first. He
/// chooses them on his phone. A new agent he asks for without naming a tool is started with the first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsesTools {
    pub tools: Vec<String>,
}

/// `POST brief`: how the model that writes his summaries is to write them, in his own words
/// (`crate::summary`), or nothing for the usual text. The rest of what that model is told stays the tower's.
/// Never logged.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryBrief {
    #[serde(default)]
    pub text: Option<String>,
}

impl std::fmt::Debug for SummaryBrief {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SummaryBrief").field("his", &self.text.is_some()).finish()
    }
}

/// `POST push`: where this radio can be woken when it is not listening: its address at a push service. The tower
/// keeps it with the pairing it came through and uses it only for what deserves a notification. An empty
/// `token` takes the address away.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTarget {
    /// The push service.
    pub provider: crate::relay::PushService,
    pub token: String,
}
