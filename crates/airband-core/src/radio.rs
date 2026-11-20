//! `airband.radio/1`: what goes between the tower and a radio (the app on the phone, in the car, on a desk).
//!
//! The radio asks; the tower answers. All under `/v1/radio/`. Sent straight to the tower with
//! `Authorization: Bearer <token>`, or sealed and passed through a relay (`airband.relay/1`):
//! - `GET next`: held open up to 25 seconds. 200 with a [`Transmission`], or 204 when nothing is waiting.
//!   With `?voice=own` the radio makes the speech itself and the transmission carries words only.
//! - `POST heard` [`Heard`]: that transmission was spoken to the end.
//! - `POST utterance` [`Utterance`] → [`Reply`]: what he said.
//! - `POST ready`: the radio is free for the next transmission.
//! - `GET ears` → [`EarsGrant`]: what should listen: the phone's own recognition, or a service and a pass for it.
//! - `GET speech/<id>?from=<block>` → [`SpeechPart`]: a transmission's or a reply's speech, in parts, as the
//!   tower's voice produces it. 404 when the tower has no voice set up: the radio then speaks the text itself.
//!
//! Those four are call mode: one transmission at a time, spoken. Chat mode reads the same messages as a feed:
//! - `GET feed?scope=…&after=<seq>&wait=<seconds>` → [`Feed`]: everything said since `after`, each cut to a
//!   line or two. `scope` is `all`, `group:<name>` or `agent:<callsign>`.
//! - `GET message/<id>?form=detail|full` → [`More`]: the fuller spoken version, or exactly what the agent wrote.
//! - `POST utterance` is also how chat sends: leading `@tags` say who it is for (`@shop2`, `@shop`, `@all`).
//! - Starting a new agent is an utterance too: `/launch <project> <task>`, for a project the feed lists.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "airband.radio/1";

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
    /// The id of this transmission's speech, when the tower is making it (`GET speech/<id>`). Without one, or if
    /// it does not arrive in time, the radio reads `say` with its own voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Heard {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Utterance {
    pub text: String,
    /// Random, the same on every retry of this utterance: the tower delivers each id once. The message it
    /// becomes in the chat carries the same id, so a radio can match what it sent to what came back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The message this answers, when the radio knows: the transmission just spoken, or the chat line he
    /// replied to. With it the tower need not work out who it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<String>,
    /// Where he was looking when he said it: `group:<name>` for one project's room. With it, and no tag or
    /// `re`, the tower chooses only among that project's agents. Left out, it chooses among all of them.
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
}

/// An agent on another of his computers, as his screen shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Other {
    pub callsign: String,
    pub group: String,
    /// The last thing it said, in its short form. Empty if it has said nothing.
    #[serde(default)]
    pub said: String,
    /// How long ago that was, in seconds.
    #[serde(default)]
    pub ago: u64,
}

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
    /// This is the last part.
    #[serde(default)]
    pub done: bool,
    /// How long the whole file is, when the radio says. With it the parts may arrive in any order and several at
    /// once, and the file is whole when every byte of it has come; without it they must come in order and the
    /// one marked `done` ends it. A tower that takes parts in any order says so in its answer (`any: true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// A quick message: something he says all the time, sent with one tap. The label is what the button says; the
/// text is the whole instruction the agent gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quick {
    pub label: String,
    pub text: String,
}

/// One login a harness on this computer can run under: a subscription or plan with its own allowance. The tower
/// never sends anything secret about it, only what to call it and how much of it is used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// What to pass back to switch to it. Means something only to the adapter that listed it.
    pub id: String,
    /// The harness it belongs to ("claude-code").
    pub harness: String,
    /// What to show him: usually the email it signs in with.
    pub label: String,
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
/// and takes the next free number otherwise; the answer is `{callsign, team}`, what it is called now and the team
/// it is in, or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveAgent {
    /// The agent's id, as the roster gives it.
    pub agent: String,
    pub team: String,
}

/// `POST folders`: the folders inside one of his folders on this computer, for choosing where a new team
/// works. `under` is a folder's whole path; empty is his home folder. Only his home folder and what is
/// inside it can be listed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoldersAsk {
    #[serde(default)]
    pub under: String,
}

/// The answer: where this is, as a whole path and as he would read it ("~/Companies"), the folder above it
/// (nothing at his home folder), and the folders inside it by name, without the hidden ones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Folders {
    pub path: String,
    pub shown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<String>,
    pub folders: Vec<String>,
}

/// `POST project`: make a new team. Agents started in it work in `folder` (a whole path inside his home
/// folder), which is made first when `make` is set. The tower then starts the team's first agent with
/// nothing to do yet, if it runs agents itself. The answer is `{say}` or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub folder: String,
    #[serde(default)]
    pub make: bool,
}

/// `POST takeover`: have the tower take over an agent he started in a terminal of his own. That terminal's
/// session is ended and the same conversation carried on in a process the tower holds, so his words reach it
/// as his own. The answer is `{}` or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeOver {
    pub agent: String,
}

/// `POST stop`: stop what an agent is doing, now. Only for one the tower runs itself (`held`). Whatever he said
/// to it meanwhile, and has not taken back, it then reads together as its next turn. `{}` or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopAgent {
    pub agent: String,
}

/// `POST end`: end an agent the tower runs itself. Its input is closed and it finishes and signs off; what it
/// has done stays done. `{}` or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndAgent {
    pub agent: String,
}

/// What the Allow and Deny buttons send in reply to a request for leave: the two answers a tower takes without
/// reading. Every radio sends these and no words of its own.
pub const ALLOW: &str = "Allow";
pub const DENY: &str = "Deny";

/// `POST taken`: the names agents go by on his other computers, as his radio shows them ("shop 1"). A tower
/// gives none of its own agents a numbered name on this list, and moves one that already has one to the next
/// free number, so two agents on his screen are never called the same. A radio tells only the later of two
/// computers, so they do not both move. `{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Taken {
    pub callsigns: Vec<String>,
}

/// `POST dismiss`: he does not mean to answer this question. It stops being open: it no longer waits on him
/// in the chat, on its team or aloud. The agent is told nothing. `{}` or `{error}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dismiss {
    pub message: String,
}

/// `POST unsend`: take back something he said that is still `queued`. `{}`, or `{error}` when it has gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unsend {
    pub message: String,
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
    /// The id of `say` as speech, when the tower is making it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<String>,
    /// The tower could not tell who his words are for: the agents it could be, likeliest first. A radio with a
    /// screen offers them to tap; `say` asks the same thing in words for one without.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<String>,
    /// His words are for this agent, which another of his computers has (one of the utterance's `others`).
    /// Nothing was delivered: the radio sends them there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elsewhere: Option<String>,
}

/// `GET ears`: what should turn his speech into text, and what the phone needs for it.
///
/// `kind` is a listening dialect the phone speaks to the service itself, with `token` (a pass that expires in
/// minutes) and `set` (whatever that service has to be told: a region, a language, a model). Or it is `tower`:
/// the phone records, and sends the recording to the tower (`POST hear`), which holds the key. Or `none`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EarsGrant {
    pub kind: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub set: std::collections::BTreeMap<String, String>,
    /// Names he is likely to say (callsigns, projects), for a recogniser that can be told what to expect.
    #[serde(default)]
    pub phrases: Vec<String>,
}

impl Default for EarsGrant {
    fn default() -> Self {
        EarsGrant { kind: crate::providers::NO_EARS.into(), token: String::new(), set: Default::default(), phrases: Vec::new() }
    }
}

impl EarsGrant {
    /// One of the settings, or empty.
    pub fn get(&self, field: &str) -> &str {
        self.set.get(field).map(String::as_str).unwrap_or_default()
    }
    pub fn listens(&self) -> bool {
        self.kind != crate::providers::NO_EARS
    }
}

/// `POST reads`: choose what reads for this tower, from a radio. `kind` is a provider that reads and that a
/// person can choose (a row of the provider table); never a command of the radio's own naming. `key` is its
/// key, or empty to use one this tower already keeps for that provider. `set` is what else it is told.
/// The tower tries it with a real call first, and answers `{ reads }` (its name) or `{ error }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetReads {
    pub kind: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub set: std::collections::BTreeMap<String, String>,
}

/// `POST hear`: part of a recording, for a tower whose listening service takes a finished clip. `data` is
/// 16 kHz mono in the same compressed blocks speech comes in, base64. With `last`, the answer is [`HeardText`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HearPart {
    /// The same for every part of one recording.
    pub clip: String,
    pub data: String,
    pub samples: u32,
    #[serde(default)]
    pub last: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeardText {
    #[serde(default)]
    pub text: String,
}

/// `GET speech/<id>?from=<block>`: the next part of a clip, as far as the tower has it. The radio asks again
/// from `next` until `done`. Each request is held a moment while the voice service is still producing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpeechPart {
    /// Samples a second.
    pub rate: u32,
    /// Blocks of IMA ADPCM (`airband_core::audio`), base64, one after another.
    pub data: String,
    /// How many samples `data` holds.
    pub samples: u32,
    /// The block to ask for next.
    pub next: u32,
    /// Nothing follows.
    pub done: bool,
    /// With the first part: the words being spoken, exactly as the voice was given them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// When the voice service says when each word is spoken: pairs of (where the word starts in `text`, counted
    /// in characters; milliseconds from the start of the clip), for the words that start within this part. A
    /// radio can show the words as they are said. Empty when the service gives no timings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<(u32, u32)>,
}

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

/// `POST push`: where this radio can be woken when it is not listening: its address at a push service. The tower
/// keeps it with the pairing it came through and uses it only for what deserves a notification. An empty
/// `token` takes the address away.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTarget {
    /// The push service ("apns" for Apple's).
    pub provider: String,
    pub token: String,
}

/// A notification, as the tower seals it for the phone (`airband.relay/1`, direction "push"). Neither the relay
/// nor the push service can read it; the phone opens it and shows it. Everything the phone needs to show it and
/// to answer it is here, because the app may not be running.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// The message's id: what an answer is sent as the answer to (`re`).
    pub id: String,
    /// The agent's callsign as the tower has it, and its team.
    pub from: String,
    #[serde(default)]
    pub group: String,
    /// The team's colour: its place in the client's list of team colours.
    #[serde(default)]
    pub colour: u8,
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
}

/// What deserves a notification: a message that needs him, or a report. Never chatter, a stall, or his own words.
pub fn deserves_notice(kind: Option<Kind>) -> bool {
    matches!(kind, Some(Kind::NeedsYou | Kind::Report))
}

/// How many bytes of a picture go in one part.
pub const PICTURE_PART: usize = 120 * 1024;

/// One line in the chat: something an agent said, cut down, or something he said.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Position in the tower's log; pass the highest one seen as `after`.
    pub seq: u64,
    pub id: String,
    /// Milliseconds since 1970.
    pub at: u64,
    /// A callsign, or "you".
    pub from: String,
    /// The group (the project's name) this belongs to; empty for a message of his to several groups.
    pub group: String,
    /// For his messages: the callsigns it went to.
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// The short version.
    pub text: String,
    /// There is more than `text` says: `message/<id>` has it.
    #[serde(default)]
    pub long: bool,
    /// Answers he can give with one tap, when the agent asked something he can answer by picking: up to three
    /// short labels, in the order the agent put them. Sending one as an utterance with `re` set to this message answers it.
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
    /// Web links the agent wrote (an artifact, a pull request), which the short version leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// It needs him and nothing has answered it yet: the agent is still stopped, he has not replied, and the
    /// agent has said nothing since. These are what an inbox shows, and what he is reminded of.
    #[serde(default)]
    pub open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    /// The agent's id: stable while it runs. What messages refer to.
    #[serde(default)]
    pub id: String,
    pub callsign: String,
    pub group: String,
    pub harness: String,
    /// Finished a turn and waiting on him.
    pub waiting: bool,
    /// What it is doing, in one word a client can show as a colour: `working`; `asking` (it has a question open
    /// for him); `stalled` (it stopped without finishing or asking: poke it); `idle` (it finished and reported).
    #[serde(default)]
    pub state: String,
    /// How many subagents it has running, when its harness reports them.
    #[serde(default)]
    pub subagents: u32,
    /// A reply can be delivered to it.
    pub reachable: bool,
    /// What it is doing right now, in a few words, while it is working and that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// The tower runs this agent itself and holds its input, so what he says reaches it as his own words. One
    /// he started in a terminal of his own is reached as a message from outside, which it treats with more care.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    /// The model it runs on, as a person would say it ("Opus 4.7"), when its harness can tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How hard it is set to think ("high"), when its harness has such a setting and can tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Feed {
    /// Always `airband.radio/1` from a tower that says; a client need not check it, but can.
    #[serde(default)]
    pub schema: String,
    /// The log's change count as of this answer: pass it back as `since`. Lower than the one a client holds
    /// means the tower's log was reset, and the client should start again from nothing.
    #[serde(default)]
    pub rev: u64,
    pub messages: Vec<Message>,
    pub roster: Vec<RosterEntry>,
    /// The log's latest position, whether or not this scope had anything new.
    pub seq: u64,
    /// Projects a new agent can be launched in (`/launch <project> <task>` as an utterance).
    #[serde(default)]
    pub projects: Vec<String>,
    /// The quick messages to offer: the standard ones, then his own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<Quick>,
    /// What this tower's computer is called ("mini", "macbook"), for a client paired with more than one.
    #[serde(default)]
    pub station: String,
    /// Addresses a radio may try to reach this tower at without the relay, when it is on the same network:
    /// `http://<address>:<port>`. A radio paired through a relay posts the same sealed request it would give
    /// the relay to `<address>/v1/sealed/<channel>` and gets the sealed answer back, so nothing on that network
    /// can read it or stand in for the tower. Empty when the tower offers none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct: Vec<String>,
    /// This tower has a key of its own for the listening service. Only that it has one: never the key.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listens: bool,
    /// What reads the agents' messages on this tower and makes the short of them, by its provider's name for
    /// showing. Empty when nothing does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reads: String,
    /// The ids of every message that needs him and is still unanswered, whether or not it is in `messages`:
    /// the whole inbox, current as of this answer.
    #[serde(default)]
    pub open: Vec<String>,
    /// A number that changes whenever anything in `roster` does. Pass it back as `roster=` and a waiting
    /// `GET feed` also returns when the roster changes, not only when a message does.
    #[serde(default)]
    pub roster_rev: u64,
    /// Each team's colour, as a number from 0: its place in the client's own list of team colours (it wraps
    /// round if there are more teams than colours). Given once and kept, so a team keeps its colour.
    #[serde(default)]
    pub colours: std::collections::BTreeMap<String, u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct More {
    pub text: String,
}

/// Does a message belong in a scope? `all`, `group:<name>`, `agent:<callsign>`.
pub fn in_scope(scope: &str, m: &Message) -> bool {
    match scope.split_once(':') {
        Some(("group", g)) => m.group == g || m.to.iter().any(|c| c.rsplit_once(' ').is_some_and(|(project, _)| project == g)),
        Some(("agent", a)) => m.from == a || m.to.iter().any(|c| c == a),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(from: &str, group: &str, to: &[&str]) -> Message {
        Message { seq: 1, id: "x".into(), at: 0, from: from.into(), group: group.into(), to: to.iter().map(|s| s.to_string()).collect(), kind: None, text: String::new(), long: false, options: Vec::new(), recommended: None, read: None, own_session: false, permission: false, pictures: Vec::new(), links: Vec::new(), agent: None, rev: 0, open: false, queued: false, seen: None, cancelled: false }
    }
    #[test]
    fn a_feed_from_a_newer_tower_still_reads() {
        let feed: Feed = serde_json::from_str(r#"{"messages":[{"seq":1,"id":"a","at":0,"from":"shop one","group":"shop","kind":"celebration","text":"x","mood":"good"}],"roster":[{"callsign":"shop one","group":"shop","harness":"h","waiting":false,"reachable":true,"colour":"red"}],"seq":1,"weather":"fine"}"#).unwrap();
        assert_eq!(feed.messages[0].kind, Some(Kind::Unknown));
        let t: Transmission = serde_json::from_str(r#"{"id":"a","callsign":"shop one","kind":"celebration","say":"x"}"#).unwrap();
        assert_eq!(t.kind, Kind::Unknown);
    }
    #[test]
    fn scopes() {
        let said = m("shop two", "shop", &[]);
        let mine = m("you", "", &["shop two", "backoffice one"]);
        assert!(in_scope("all", &said) && in_scope("group:shop", &said) && in_scope("agent:shop two", &said));
        assert!(!in_scope("group:backoffice", &said) && !in_scope("agent:shop one", &said));
        assert!(in_scope("group:backoffice", &mine) && in_scope("agent:shop two", &mine) && !in_scope("agent:shop one", &mine));
    }
}
