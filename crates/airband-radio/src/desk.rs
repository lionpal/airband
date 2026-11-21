//! The desk: every computer he is paired with, as one. A host that shows a chat talks to this and to nothing
//! else. It keeps one [`Radio`] per computer and owns everything that is the same on any platform: merging
//! their messages, what counts as waiting on him and what as done, which computer a message of his goes to,
//! which computer has the floor on a call, and marking what he has read. What is left for a host is to draw
//! the [`DeskView`], make and capture sound, and keep the pairing links somewhere safe.
use super::*;
use airband_core::routing::{Callsign, parse_utterance};
use crate::mouth::CodingTool;
use std::collections::{HashMap, HashSet};
use std::sync::Condvar;

/// What a host provides to a desk. Sound calls arrive on a radio's own thread, one computer at a time, and may
/// block; `changed`, `status` and `note` arrive on any thread and must return quickly.
#[uniffi::export(with_foreign)]
pub trait DeskHost: Send + Sync {
    /// Play 16-bit little-endian mono PCM at `rate`, queued after anything already playing. Returns at once,
    /// except with `last`: then it returns when everything queued has been heard.
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool);
    /// Start capturing the microphone as 16-bit little-endian mono PCM at `rate`. False if this host cannot.
    fn record_start(&self, rate: u32) -> bool;
    /// The next captured audio. Blocks until there is some; empty once the microphone has been stopped.
    fn record_read(&self) -> Vec<u8>;
    fn record_stop(&self);
    /// What the desk shows has changed: ask for [`Desk::view`] again.
    fn changed(&self);
    /// For a call screen: what is happening. `saying` is always empty: nothing is said with the device's own voice.
    fn status(&self, status: RadioStatus, saying: String);
    /// Something he must be told: a message could not be placed, or expired unsent.
    fn note(&self, text: String);
    /// What he has said so far, while he is still speaking, revised as more is heard. Empty when he has stopped.
    fn hearing(&self, text: String);
    /// What the voice is saying in the clip now starting to play, and when each word is said: where each word
    /// starts in `text` (in characters) and how many milliseconds into the clip. With empty `text`: more of the
    /// same clip's timings. For showing the words as they are spoken.
    fn words(&self, text: String, starts: Vec<u32>, at_ms: Vec<u32>);
    /// Is the chat on screen in front of him right now? What arrives while it is counts as read, so a call
    /// started later does not read it out.
    fn watching(&self) -> bool;
    /// What this device can do: the engines the host itself runs models with, by the names the catalogue's
    /// model rows give, and which operating system this is. A model is offered only when its engine is in the
    /// core or named here, and the system is one its row allows. Asked once, when the desk is made.
    fn device(&self) -> HostDevice;
    /// Run a model with one of the engines `device` named. `folder` is where the model's files are. For
    /// listening `input` is a recording (16-bit little-endian mono PCM at 16 kHz) and the answer is the
    /// words, as UTF-8. For a voice it is the words, as UTF-8, then a line feed and the name of the voice, and
    /// the answer is the sound (16-bit little-endian mono PCM at the rate the model's row gives). Nothing when
    /// the host cannot. May block for as long as it takes.
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>>;
    /// A key he gave for a service, by that service's provider id; nothing when none is kept. The host keeps
    /// keys where its system keeps secrets, on this device only. The core asks for one only to use it: it is
    /// never handed to a screen.
    fn secret(&self, name: String) -> Option<String>;
    /// Keep this key under that name from now on, or with nothing, forget it.
    fn keep_secret(&self, name: String, value: Option<String>);
}

/// One team: a project and the agents working in it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskRoom {
    pub name: String,
    /// What the team is doing, the most pressing first: `asking` (one of them is waiting on him), `stalled` (one
    /// has stopped without finishing), `working`, or `idle`.
    pub state: String,
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
    /// `working`, `asking`, `stalled` or `idle`.
    pub state: String,
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
    /// The tower runs it itself, so his words reach it as his own. One that is not was started in a terminal
    /// and is only left messages from outside; the tower can take it over.
    pub held: bool,
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
    pub state: String,
}

/// How many lines of the feed a view carries.
const FEED_SHOWN: usize = 80;

pub(crate) struct Station {
    link: String,
    pub(crate) radio: Arc<Radio>,
    name: String,
    pub(crate) online: bool,
    /// It has been asked at least once and has answered or failed.
    tried: bool,
    /// When it last answered, and how many times in a row it has not since.
    answered_at: Option<std::time::Instant>,
    fails: u32,
    failing_since: Option<std::time::Instant>,
    /// What reads the agents' messages there, by name.
    reads: String,
    /// Why it could not, the last time that computer asked it. Empty while it works.
    reads_failed: String,
    agents: Vec<ChatAgent>,
    open: HashSet<String>,
    sending: Vec<Outgoing>,
    /// Sent, and not yet seen in the chat that comes back from the tower: shown as on their way until they are.
    landing: Vec<(Outgoing, std::time::Instant)>,
    quick: Vec<QuickMessage>,
    /// Each team's colour, as this computer gave it.
    colours: std::collections::HashMap<String, u8>,
    /// This computer's recent chat, oldest first.
    messages: Vec<ChatMessage>,
}

/// Words of his that no tower could place, with who the asked tower thought they might be for.
struct Asking {
    message: Outgoing,
    guesses: Vec<String>,
}

#[derive(Default)]
pub(crate) struct State {
    /// What he said that is waiting to be told who it is for.
    asking: Option<Asking>,
    /// He took back what he was saying: when the microphone closes, nothing is sent.
    discard: bool,
    pub(crate) stations: Vec<Station>,
    /// Answered here a moment ago: hidden from "waiting" before the tower confirms.
    answered: HashSet<String>,
    /// Goes up each time the set of computers changes, so threads following the old set stop.
    generation: u64,
    calling: bool,
    uploads: Vec<Upload>,
    /// Where this device can be woken with a notification (a push service's name and the device's address
    /// there), and the links of the computers that have been told.
    push: Option<(String, String)>,
    push_told: HashSet<String>,
    /// The coding tools each computer was last told he uses, by its link.
    tools_told: HashMap<String, Vec<String>>,
    banner: BannerAddresses,
    read: ReadMarks,
}

/// Where this device's banner can be changed while the app is asleep. One address puts a banner up, and every
/// computer is given it. Each computer has a banner of its own, and the address that changes it goes to that
/// computer only.
#[derive(Default)]
struct BannerAddresses {
    /// The push service's name. Empty until the host has given an address.
    provider: String,
    start: String,
    /// The address of the banner that is up for a computer, by its link. None up: no entry.
    update: HashMap<String, String>,
    /// What each computer has been told, by its link: the address that puts one up and the one that changes it.
    told: HashMap<String, (String, String)>,
}

/// What he has read: for each agent, by callsign, the time of the last thing it said that was on screen in
/// front of him. Kept on the device in one small file, so it holds across launches. Something said after that
/// time is unread, wherever he is looking now.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ReadMarks {
    /// When this device began keeping count: what an agent said before then is not counted against him.
    since: u64,
    upto: HashMap<String, u64>,
}

impl ReadMarks {
    fn load(dir: &str) -> Self {
        let kept = if dir.is_empty() { None } else { std::fs::read(std::path::Path::new(dir).join("read.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()) };
        kept.unwrap_or_else(|| ReadMarks { since: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0), upto: HashMap::new() })
    }
    fn keep(&self, dir: &str) {
        if !dir.is_empty() {
            let _ = std::fs::write(std::path::Path::new(dir).join("read.json"), serde_json::to_vec(self).unwrap_or_default());
        }
    }
    fn unread(&self, m: &ChatMessage) -> bool {
        m.from != "you" && !m.fyi && m.at > self.upto.get(&m.from).copied().unwrap_or(self.since)
    }
}

struct Upload {
    shown: DeskUpload,
    /// Which computer it is going to: what he sends next goes there too.
    station: usize,
    /// He took it off, or it is being sent somewhere else instead: stop sending.
    dropped: Arc<AtomicBool>,
    /// The file on the device and what to call it, kept so it can be sent to another computer if his words turn
    /// out to be for an agent there.
    file_path: String,
    name: String,
}

/// Send one attached file to a computer, keeping the tray's picture of it up to date. When it is there it is
/// armed on that computer's radio: the next thing he sends there takes it along.
fn send_file(shared: Arc<Shared>, radio: Arc<Radio>, id: String, file_path: String, name: String, dropped: Arc<AtomicBool>) {
    thread::spawn(move || {
        let last_told = Mutex::new(std::time::Instant::now());
        let step = |sent: u64, _total: u64| {
            if dropped.load(Ordering::SeqCst) {
                return false;
            }
            if let Some(u) = shared.state.lock().unwrap().uploads.iter_mut().find(|u| u.shown.id == id && Arc::ptr_eq(&u.dropped, &dropped)) {
                u.shown.sent = sent;
            }
            // the screen is told a few times a second at most, however fast the parts land
            let mut told = last_told.lock().unwrap();
            if told.elapsed() > Duration::from_millis(250) {
                *told = std::time::Instant::now();
                drop(told);
                shared.host.changed();
            }
            true
        };
        let outcome = radio.upload(&id, &file_path, &name, step);
        {
            let mut state = shared.state.lock().unwrap();
            // only if this is still the sending that counts: it may have been taken off, or started again elsewhere
            if let Some(u) = state.uploads.iter_mut().find(|u| u.shown.id == id && Arc::ptr_eq(&u.dropped, &dropped)) {
                u.shown.state = if outcome.is_ok() { "ready".into() } else { "failed".into() };
                if outcome.is_ok() {
                    radio.arm(id.clone());
                }
            }
        }
        shared.host.changed();
    });
}

impl State {
    fn owner(&self, message: &str) -> Option<usize> {
        self.stations.iter().position(|s| s.messages.iter().any(|m| m.id == message))
    }
    fn message(&self, id: &str) -> Option<(usize, &ChatMessage)> {
        self.stations.iter().enumerate().find_map(|(i, s)| s.messages.iter().find(|m| m.id == id).map(|m| (i, m)))
    }
    /// Every message, oldest first, each labelled with its computer when there are several.
    fn merged(&self) -> Vec<ChatMessage> {
        let several = self.stations.len() > 1;
        let mut all: Vec<ChatMessage> = self
            .stations
            .iter()
            .flat_map(|s| {
                s.messages.iter().map(move |m| {
                    let his = m.from == "you";
                    ChatMessage {
                        station: if several { s.name.clone() } else { String::new() },
                        // on his own message, the badge of the agent it went to, when it went to one
                        badge: match (his, m.to.as_slice()) {
                            (false, _) => airband_core::routing::badge(&m.group, &m.from),
                            (true, [one]) if !m.group.is_empty() => airband_core::routing::badge(&m.group, one),
                            _ => String::new(),
                        },
                        colour: s.colours.get(&m.group).copied().unwrap_or(0),
                        ..m.clone()
                    }
                })
            })
            .collect();
        all.sort_by_key(|m| m.at);
        // one computer paired by two links (one through the relay, one straight to it) is still one computer:
        // each of its messages is shown once
        let mut seen = HashSet::new();
        all.retain(|m| seen.insert(m.id.clone()));
        all
    }
    /// Every agent on every computer, each once.
    fn everyone(&self) -> Vec<&ChatAgent> {
        let mut seen = HashSet::new();
        self.stations.iter().flat_map(|s| s.agents.iter()).filter(|a| seen.insert(a.id.clone())).collect()
    }
    fn is_open(&self, id: &str) -> bool {
        !self.answered.contains(id) && self.stations.iter().any(|s| s.open.contains(id))
    }
}

/// On a call only one computer may have the floor: its transmission, his answer, its reply. The others wait,
/// so two never talk over each other or take his answer for theirs.
#[derive(Default)]
struct Floor {
    holder: Mutex<Option<usize>>,
    freed: Condvar,
}

impl Floor {
    fn take(&self, index: usize) {
        let mut holder = self.holder.lock().unwrap();
        while holder.is_some_and(|h| h != index) {
            holder = self.freed.wait(holder).unwrap();
        }
        *holder = Some(index);
    }
    /// Give it up if `index` has it. Returns whether `index` may show its status: it has the floor or nobody does.
    fn release(&self, index: usize, give_up: bool) -> bool {
        let mut holder = self.holder.lock().unwrap();
        if give_up && *holder == Some(index) {
            *holder = None;
            self.freed.notify_all();
        }
        holder.is_none_or(|h| h == index)
    }
    fn clear(&self) {
        *self.holder.lock().unwrap() = None;
        self.freed.notify_all();
    }
}

pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    floor: Floor,
    pub(crate) host: Arc<dyn DeskHost>,
    /// What he chose to read, listen and speak with.
    pub(crate) chooser: crate::choices::Chooser,
    /// What listens and speaks on this device, with the models loaded for it: every radio of this desk's
    /// uses the same.
    pub(crate) senses: Arc<Senses>,
    /// How a key he gives is tested before it is kept: one request to its service (`keys::check`).
    pub(crate) key_test: crate::keys::Test,
}

/// What one computer's radio calls back into: it knows which computer it is, and the desk does the rest.
struct StationHost {
    shared: Arc<Shared>,
    index: usize,
    generation: u64,
}

impl RadioHost for StationHost {
    // a phone's own voice and recognition are never used
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn told(&self, text: String) {
        // what he chose stays chosen and now says why it did not work: the screens that show it are told too
        self.shared.host.note(text);
        self.shared.host.changed();
    }
    fn unplaced(&self, message: Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.generation != self.generation {
                return;
            }
            // it was not delivered here: it is not on its way to this computer's chat
            if let Some(s) = state.stations.get_mut(self.index) {
                s.landing.retain(|(o, _)| o.id != message.id);
            }
            // the tower read his words as for an agent another computer has: they go there, addressed to it
            let there = elsewhere.as_ref().and_then(|c| state.stations.iter().enumerate().find(|(i, s)| *i != self.index && s.agents.iter().any(|a| a.callsign == *c)).map(|(_, s)| s.radio.clone()));
            match (there, elsewhere) {
                (Some(radio), Some(callsign)) => {
                    let text = format!("@{} {}", callsign.replace(' ', ""), message.text);
                    drop(state);
                    radio.queue_with(text, None, None, message.label.clone(), String::new());
                }
                (_, gone) => state.asking = Some(Asking { message, guesses: gone.into_iter().chain(ask).collect() }),
            }
        }
        self.shared.host.changed();
    }
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        self.shared.floor.take(self.index);
        self.shared.host.play(rate, pcm, last);
    }
    fn record_start(&self, rate: u32) -> bool {
        self.shared.floor.take(self.index);
        self.shared.host.record_start(rate)
    }
    fn record_read(&self) -> Vec<u8> {
        self.shared.host.record_read()
    }
    fn record_stop(&self) {
        self.shared.host.record_stop();
    }
    fn status(&self, status: RadioStatus) {
        // idle or offline ends that computer's turn
        let ends = matches!(status, RadioStatus::Idle | RadioStatus::Offline);
        // what he said has gone, and took with it the files that were waiting: they leave the tray
        if matches!(status, RadioStatus::Idle) {
            let mut state = self.shared.state.lock().unwrap();
            if let Some(waiting) = state.stations.get(self.index).map(|s| s.radio.armed()) {
                let before = state.uploads.len();
                state.uploads.retain(|u| !(u.station == self.index && u.shown.state == "ready" && !waiting.contains(&u.shown.id)));
                if state.uploads.len() != before {
                    drop(state);
                    self.shared.host.changed();
                }
            }
        }
        if self.shared.floor.release(self.index, ends) {
            self.shared.host.status(status, String::new());
        }
    }
    fn hearing(&self, text: String) {
        self.shared.host.hearing(text);
    }
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        self.shared.host.run_model(engine, folder, job, input)
    }
    fn words(&self, text: String, starts: Vec<u32>, at_ms: Vec<u32>) {
        // only the computer whose turn it is to speak says what is being said: another one's words wait with
        // its sound, or they would be shown over the voice of the one speaking
        self.shared.floor.take(self.index);
        self.shared.host.words(text, starts, at_ms);
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.generation != self.generation {
                return;
            }
            if let Some(s) = state.stations.get_mut(self.index) {
                // What has left the list was sent. It stays on screen as on its way until the chat has it (the
                // tower gives the message the same id), so it never vanishes for a moment in between. Not when
                // there is something to tell him about it: then it did not go.
                if note.is_none() {
                    // (one that was given up on and has now been let go did not go anywhere)
                    let gone: Vec<Outgoing> = s.sending.iter().filter(|was| !waiting.iter().any(|w| w.id == was.id)).cloned().collect();
                    let now = std::time::Instant::now();
                    s.landing.retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
                    s.landing.extend(gone.into_iter().map(|o| (o, now)));
                }
                s.sending = waiting;
            }
        }
        self.shared.host.changed();
        if let Some(note) = note.filter(|n| !n.is_empty()) {
            self.shared.host.note(note);
        }
    }
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

#[derive(uniffi::Object)]
pub struct Desk {
    pub(crate) shared: Arc<Shared>,
    data_dir: String,
}

#[cfg(test)]
impl Desk {
    /// A desk with no computers, keeping what it keeps in `data_dir`.
    pub(crate) fn alone(data_dir: &str, host: Arc<dyn DeskHost>) -> Desk {
        let chooser = crate::choices::Chooser::open(data_dir, host.device());
        Desk { shared: Arc::new(Shared { state: Mutex::new(State::default()), floor: Floor::default(), host, chooser, senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: data_dir.into() }
    }
}

impl Desk {
    /// Tell every computer what it has not been told about the banner's addresses.
    fn tell_banners(&self) {
        let shared = self.shared.clone();
        thread::spawn(move || {
            let (generation, radios): (u64, Vec<Arc<Radio>>) = {
                let state = shared.state.lock().unwrap();
                (state.generation, state.stations.iter().map(|s| s.radio.clone()).collect())
            };
            for (index, radio) in radios.iter().enumerate() {
                tell_banner(&shared, radio, index, generation);
            }
        });
    }
}

/// A folder name that is the same every launch for the same link.
fn folder(link: &str) -> String {
    let mut h: u64 = 1469598103934665603;
    for b in link.bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    format!("{h:x}")
}

#[uniffi::export]
impl Desk {
    /// `data_dir` is a folder of the app's own; each computer's unsent messages, recent chat and pictures are
    /// kept in a folder under it. Nothing is connected until [`Desk::set_links`].
    #[uniffi::constructor]
    pub fn open(data_dir: String, host: Arc<dyn DeskHost>) -> Arc<Self> {
        let state = State { read: ReadMarks::load(&data_dir), ..State::default() };
        let chooser = crate::choices::Chooser::open(&data_dir, host.device());
        let senses: Arc<Senses> = Arc::default();
        // what he chose to listen and speak with is in use from the start
        chooser.apply(&*host, &senses);
        Arc::new(Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser, senses, key_test: crate::keys::check }), data_dir })
    }

    /// The computers he is paired with, as the links `airband pair` printed, one each. Replaces whatever set
    /// was there. Returns the links that were taken: one that is not a pairing link is left out. What was last
    /// seen from each is on screen at once, before any connection.
    pub fn set_links(&self, links: Vec<String>) -> Vec<String> {
        let (old, generation, calling) = {
            let mut state = self.shared.state.lock().unwrap();
            state.generation += 1;
            (std::mem::take(&mut state.stations), state.generation, state.calling)
        };
        old.iter().for_each(|s| s.radio.stop());
        self.shared.floor.clear();
        let mut made: Vec<Station> = Vec::new();
        for link in links {
            if made.iter().any(|s| s.link == link) {
                continue;
            }
            let dir = if self.data_dir.is_empty() { String::new() } else { std::path::Path::new(&self.data_dir).join(folder(&link)).display().to_string() };
            let host = Arc::new(StationHost { shared: self.shared.clone(), index: made.len(), generation });
            let Ok(radio) = Radio::start_with(link.clone(), dir, host, self.shared.senses.clone()) else { continue };
            radio.set_call_mode(calling);
            // a computer that was already there keeps what was last seen of it, so the screen does not empty while
            // it is asked again
            let was = old.iter().find(|s| s.link == link);
            made.push(Station {
                messages: radio.history(),
                radio,
                name: was.map(|s| s.name.clone()).unwrap_or_default(),
                online: was.is_some_and(|s| s.online),
                tried: false,
                answered_at: was.and_then(|s| s.answered_at),
                fails: 0,
                failing_since: None,
                reads: String::new(),
                reads_failed: String::new(),
                agents: was.map(|s| s.agents.clone()).unwrap_or_default(),
                open: was.map(|s| s.open.clone()).unwrap_or_default(),
                sending: Vec::new(),
                landing: Vec::new(),
                quick: was.map(|s| s.quick.clone()).unwrap_or_default(),
                colours: was.map(|s| s.colours.clone()).unwrap_or_default(),
                link,
            });
        }
        let taken: Vec<String> = made.iter().map(|s| s.link.clone()).collect();
        let radios: Vec<Arc<Radio>> = made.iter().map(|s| s.radio.clone()).collect();
        self.shared.state.lock().unwrap().stations = made;
        for (index, radio) in radios.into_iter().enumerate() {
            let shared = self.shared.clone();
            thread::spawn(move || follow(shared, radio, index, generation));
        }
        self.shared.host.changed();
        taken
    }

    /// The app is in front of him again. A request that was open when it went to the background is dead and
    /// would not be noticed for most of a minute, so if no computer has answered lately they are all asked
    /// afresh, now. Cheap to call whenever the app comes forward.
    pub fn wake(&self) {
        let stale = {
            let state = self.shared.state.lock().unwrap();
            // a request waits up to 25 seconds for news, so an answer older than that plus a margin is a dead line
            !state.stations.is_empty() && state.stations.iter().any(|s| s.answered_at.is_none_or(|t| t.elapsed() > Duration::from_secs(32)))
        };
        if stale {
            let links: Vec<String> = self.shared.state.lock().unwrap().stations.iter().map(|s| s.link.clone()).collect();
            self.set_links(links);
        }
    }

    /// He asked for it: ask every computer afresh, now, whatever was last heard from them.
    pub fn reconnect(&self) {
        let links: Vec<String> = self.shared.state.lock().unwrap().stations.iter().map(|s| s.link.clone()).collect();
        if !links.is_empty() {
            self.set_links(links);
        }
    }

    /// What to show. `room` narrows the feed to one team, `agent` to one callsign; with neither, everything.
    pub fn view(&self, room: Option<String>, agent: Option<String>) -> DeskView {
        let mut state = self.shared.state.lock().unwrap();
        let all = state.merged();
        // What this view shows, while the chat is in front of him, is read. What agents he is not looking at
        // have said stays unread, and is counted for each of them and each team below.
        if self.shared.host.watching() {
            let mut moved = false;
            for m in all.iter().filter(|m| room.as_ref().is_none_or(|r| m.group == *r) && agent.as_ref().is_none_or(|a| m.from == *a)) {
                if state.read.unread(m) {
                    state.read.upto.insert(m.from.clone(), m.at);
                    moved = true;
                }
            }
            if moved {
                state.read.keep(&self.data_dir);
            }
        }
        let state = state;
        let mut unread_from: HashMap<&str, u32> = HashMap::new();
        let mut unread_in: HashMap<&str, u32> = HashMap::new();
        for m in all.iter().filter(|m| state.read.unread(m)) {
            *unread_from.entry(m.from.as_str()).or_default() += 1;
            *unread_in.entry(m.group.as_str()).or_default() += 1;
        }
        let unread: u32 = unread_from.values().sum();
        let agents: Vec<&ChatAgent> = state.everyone();
        // his own messages belong to the room or agent they went to
        let here = |m: &ChatMessage| {
            let about = |name: &String| agent.as_ref().is_none_or(|a| name == a) && room.as_ref().is_none_or(|r| agents.iter().any(|x| x.callsign == *name && x.group == *r));
            if m.from == "you" { (room.is_none() && agent.is_none()) || m.to.iter().any(about) } else { room.as_ref().is_none_or(|r| m.group == *r) && agent.as_ref().is_none_or(|a| m.from == *a) }
        };
        let mut feed: Vec<ChatMessage> = all.iter().cloned().filter(|m| !m.fyi && here(m)).map(|m| ChatMessage { open: state.is_open(&m.id), ..m }).collect();
        let extra = feed.len().saturating_sub(FEED_SHOWN);
        feed.drain(..extra);
        // each team, in the state of whichever of its agents most needs him
        let rank = |s: &str| ["idle", "working", "stalled", "asking"].iter().position(|x| *x == s).unwrap_or(0);
        let mut rooms: Vec<DeskRoom> = Vec::new();
        // what an agent is waiting on him for: its newest question still open
        let asked = |a: &ChatAgent| all.iter().rev().find(|m| m.needs_you && m.from == a.callsign && state.is_open(&m.id));
        for a in &agents {
            let member = DeskMember { id: a.id.clone(), callsign: a.callsign.clone(), state: a.state.clone(), subagents: a.subagents, doing: a.doing.clone(), held: a.held, unread: unread_from.get(a.callsign.as_str()).copied().unwrap_or(0), question: asked(a).map(|m| m.text.clone()).unwrap_or_default(), question_id: asked(a).map(|m| m.id.clone()).unwrap_or_default(), harness: a.harness.clone(), badge: airband_core::routing::badge(&a.group, &a.callsign), model: a.model.clone().unwrap_or_default(), effort: a.effort.clone().unwrap_or_default(), number: a.number.clone(), station: if state.stations.len() > 1 { state.stations.iter().find(|s| s.agents.iter().any(|x| x.id == a.id)).map(|s| s.name.clone()).unwrap_or_default() } else { String::new() } };
            // what the team is doing is said in the name of whoever is doing it, without the team's own name
            let doing = a.doing.as_ref().map(|d| match a.callsign.strip_prefix(&a.group).map(str::trim).filter(|rest| !rest.is_empty()) {
                Some(who) => format!("{who}: {d}"),
                None => d.clone(),
            });
            match rooms.iter_mut().find(|r| r.name == a.group) {
                Some(r) => {
                    r.agents += 1;
                    r.subagents += a.subagents;
                    if rank(&a.state) > rank(&r.state) {
                        r.state = a.state.clone();
                    }
                    r.doing = r.doing.take().or(doing);
                    r.members.push(member);
                }
                None => rooms.push(DeskRoom { name: a.group.clone(), state: if a.state.is_empty() { "working".into() } else { a.state.clone() }, agents: 1, subagents: a.subagents, doing, members: vec![member], unread: unread_in.get(a.group.as_str()).copied().unwrap_or(0), colour: state.stations.iter().find_map(|s| s.colours.get(&a.group).copied()).unwrap_or(0) }),
            }
        }
        rooms.sort_by(|a, b| a.name.cmp(&b.name));
        rooms.iter_mut().for_each(|r| r.members.sort_by(|a, b| a.callsign.cmp(&b.callsign)));
        // the same quick messages from every computer, each once
        let mut quick: Vec<QuickMessage> = Vec::new();
        for q in state.stations.iter().flat_map(|s| s.quick.iter()) {
            if !quick.iter().any(|have| have.label == q.label) {
                quick.push(q.clone());
            }
        }
        DeskView {
            paired: !state.stations.is_empty(),
            online: state.stations.iter().any(|s| s.online),
            connecting: !state.stations.iter().any(|s| s.online) && state.stations.iter().any(|s| !s.tried),
            near: state.stations.iter().any(|s| s.online) && state.stations.iter().filter(|s| s.online).all(|s| s.radio.near()),
            away: if state.stations.len() > 1 { state.stations.iter().enumerate().filter(|(_, s)| s.tried && !s.online).map(|(i, s)| if s.name.is_empty() { format!("computer {}", i + 1) } else { s.name.clone() }).collect() } else { Vec::new() },
            can_poke: agents.iter().any(|a| a.state == "stalled" && room.as_ref().is_none_or(|r| a.group == *r)),
            feed,
            rooms,
            unread,
            quick,
            // on their way: not yet sent, then sent and not yet back in the chat (for a quarter of a minute at most)
            sending: state
                .stations
                .iter()
                .flat_map(|s| s.sending.iter().filter(|o| !s.messages.iter().any(|m| m.id == o.id)).cloned().chain(s.landing.iter().filter(|(o, at)| at.elapsed() < Duration::from_secs(15) && !s.messages.iter().any(|m| m.id == o.id)).map(|(o, _)| o.clone())))
                .collect(),
            stations: state.stations.iter().map(|s| s.name.clone()).collect(),
            reads: state.stations.iter().map(|s| s.reads.clone()).find(|r| !r.is_empty()).unwrap_or_default(),
            uploads: state.uploads.iter().map(|u| u.shown.clone()).collect(),
            asking: state.asking.as_ref().map(|asking| {
                // the tower's guesses first, in its order; then everyone else, whoever spoke last first
                let mut rest: Vec<&ChatAgent> = agents.iter().copied().filter(|a| !asking.guesses.contains(&a.callsign)).collect();
                let spoke = |a: &ChatAgent| state.stations.iter().flat_map(|s| s.messages.iter()).filter(|m| m.from == a.callsign).map(|m| m.at).max().unwrap_or(0);
                rest.sort_by_key(|a| std::cmp::Reverse(spoke(a)));
                let guessed = asking.guesses.iter().filter_map(|c| agents.iter().copied().find(|a| a.callsign == *c));
                DeskAsk {
                    text: asking.message.label.clone().unwrap_or_else(|| asking.message.text.clone()),
                    choices: guessed.chain(rest).take(8).map(|a| DeskChoice { id: a.id.clone(), callsign: a.callsign.clone(), badge: airband_core::routing::badge(&a.group, &a.callsign), colour: state.stations.iter().find_map(|s| s.colours.get(&a.group).copied()).unwrap_or(0) }).collect(),
                }
            }),
        }
    }

    /// He said who his unplaced words are for: they go to that agent, on whichever computer has it.
    pub fn choose(&self, agent: String) {
        let sent = {
            let mut state = self.shared.state.lock().unwrap();
            let to = state.stations.iter().find_map(|s| s.agents.iter().find(|a| a.id == agent).map(|a| (s.radio.clone(), a.callsign.clone())));
            match (to, state.asking.take()) {
                (Some((radio, callsign)), Some(asking)) => Some((radio, format!("@{} {}", callsign.replace(' ', ""), asking.message.text), asking.message.label)),
                (_, asking) => {
                    state.asking = asking;
                    None
                }
            }
        };
        if let Some((radio, text, label)) = sent {
            radio.queue_with(text, None, None, label, String::new());
        }
        self.shared.host.changed();
    }

    /// He let his unplaced words go: they are sent to nobody.
    pub fn drop_ask(&self) {
        self.shared.state.lock().unwrap().asking = None;
        self.shared.host.changed();
    }

    /// What was said between him and an agent just before one of its messages, oldest first: the context for
    /// a question.
    pub fn thread(&self, message_id: String) -> Vec<ChatMessage> {
        let state = self.shared.state.lock().unwrap();
        let Some((index, m)) = state.message(&message_id) else { return Vec::new() };
        let same_agent = |b: &ChatMessage| if m.agent.is_some() { b.agent == m.agent } else { b.from == m.from };
        let before: Vec<ChatMessage> = state.stations[index].messages.iter().filter(|b| b.seq < m.seq && (same_agent(b) || (b.from == "you" && b.to.contains(&m.from)))).cloned().collect();
        before[before.len().saturating_sub(5)..].to_vec()
    }

    /// One tap on an answer: send it as the answer to that message.
    pub fn answer(&self, message_id: String, text: String) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            state.answered.insert(message_id.clone());
            state.owner(&message_id).or((!state.stations.is_empty()).then_some(0)).map(|i| state.stations[i].radio.clone())
        };
        if let Some(radio) = radio {
            radio.queue(text, Some(message_id), None, None);
        }
        self.shared.host.changed();
    }

    /// An answer given from a notification: send it now, as the answer to that message, and wait until the
    /// computer has it. `station` is the computer the notification came from ([`OpenedNotice::station`]).
    /// Blocks for some seconds at most. If it could not be sent in that time it is kept and sent when it can
    /// be, and the error says why, in words for him when there are any.
    pub fn answer_now(&self, station: String, message_id: String, text: String) -> Result<(), RadioError> {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            state.answered.insert(message_id.clone());
            let named = state.stations.iter().position(|s| station_key(&s.link).as_deref() == Some(station.as_str()));
            named.or(state.owner(&message_id)).map(|i| state.stations[i].radio.clone())
        };
        self.shared.host.changed();
        radio.ok_or(RadioError::Told { reason: "That computer is no longer paired.".into() })?.answer_now(text, message_id)
    }

    /// Where this device can be woken with a notification: the push service's name ("apns") and the device's
    /// address there, as the system gave it. Every paired computer is told, now and whenever one is paired
    /// later. Call it each time the system gives the address; the same one again does nothing.
    pub fn set_push(&self, provider: String, token: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let push = Some((provider, token));
            if state.push == push {
                return;
            }
            state.push = push;
            state.push_told.clear();
        }
        let shared = self.shared.clone();
        thread::spawn(move || {
            let (generation, radios): (u64, Vec<Arc<Radio>>) = {
                let state = shared.state.lock().unwrap();
                (state.generation, state.stations.iter().map(|s| s.radio.clone()).collect())
            };
            for (index, radio) in radios.iter().enumerate() {
                tell_push(&shared, radio, index, generation);
            }
        });
    }

    /// Where a banner can be put up on this device while the app is asleep: the push service's name and the
    /// address the system gave for it. Every paired computer is told, now and whenever one is paired later.
    pub fn set_banner_start(&self, provider: String, token: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.banner.provider == provider && state.banner.start == token {
                return;
            }
            (state.banner.provider, state.banner.start) = (provider, token);
        }
        self.tell_banners();
    }

    /// A banner is up for one of his computers and this is the address that changes it, or (with an empty
    /// token) that banner has come down. `slot` is the number the banner carries
    /// ([`airband_core::relay::banner_slot`]): it says which computer. Only that computer is told.
    pub fn set_banner_update(&self, slot: u32, provider: String, token: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let Some(link) = state.stations.iter().map(|s| s.link.clone()).find(|link| station_key(link).is_some_and(|channel| airband_core::relay::banner_slot(&channel) == slot)) else { return };
            if state.banner.provider.is_empty() {
                state.banner.provider = provider;
            }
            if state.banner.update.get(&link).map(String::as_str).unwrap_or("") == token {
                return;
            }
            if token.is_empty() { state.banner.update.remove(&link) } else { state.banner.update.insert(link, token) };
        }
        self.tell_banners();
    }

    /// The computers a new agent of this team can be started on: the ones where the team already has agents,
    /// the one with the most of them first. That first one is where a new agent goes unless he says otherwise.
    pub fn computers_for(&self, room: String) -> Vec<String> {
        let state = self.shared.state.lock().unwrap();
        let mut found: Vec<(usize, &Station)> = state.stations.iter().map(|s| (s.agents.iter().filter(|a| a.group == room).count(), s)).filter(|(n, _)| *n > 0).collect();
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found.into_iter().map(|(_, s)| s.name.clone()).collect()
    }

    /// The folders inside one of his on a computer, for choosing where a new team works. `computer` is its
    /// name, or empty when he has only one; `under` is a folder's whole path, or empty for his
    /// home folder. Blocks: call it off the main thread.
    pub fn folders(&self, computer: String, under: String) -> Result<DeskFolders, RadioError> {
        let f = self.station_radio(&computer)?.folders(under)?;
        Ok(DeskFolders { path: f.path, shown: f.shown, up: f.up, folders: f.folders })
    }

    /// The folders the coding tools on a computer have had sessions in, newest first, then the teams there
    /// that no tool has a record of. `computer` is its name, or empty when he has only one. Blocks: call it
    /// off the main thread.
    pub fn recent_folders(&self, computer: String) -> Result<Vec<DeskRecent>, RadioError> {
        let tools = airband_core::catalog::current();
        let called = |id: &String| tools.tools.iter().find(|t| t.id == *id).map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
        Ok(self.station_radio(&computer)?.recent()?.folders.into_iter().map(|r| DeskRecent { tools: r.tools.iter().map(called).collect(), folder: r.folder, shown: r.shown, name: r.name, team: r.team, at: r.at }).collect())
    }

    /// Make a new team on a computer, or start another agent in one that is there: its name, and the folder
    /// its agents work in (made first with `make`).
    /// Its first agent is started there with nothing to do yet. Blocks: call it off the main thread.
    /// `tool` is the coding tool its first agent is, by id; nothing is the computer's usual one.
    /// Answers with the team's name as that computer writes it.
    pub fn new_team(&self, computer: String, name: String, folder: String, make: bool, tool: Option<String>) -> Result<String, RadioError> {
        self.station_radio(&computer)?.new_project(name, folder, make, tool.filter(|t| !t.is_empty()))
    }

    /// Start a new agent in a team, on the computer named. With none named it starts on the one computer that
    /// has that team; when several have it he is asked which, and nothing is started. It is started with
    /// nothing to do yet and shows up at once, waiting for his first word. Nothing happens for a team no
    /// computer has.
    /// `tool` is the coding tool it is, by id, with what to call that tool; nothing is the computer's usual one.
    pub fn new_agent(&self, room: String, computer: Option<String>, tool: Option<CodingTool>) {
        let radio = {
            let state = self.shared.state.lock().unwrap();
            let has = |s: &&Station| s.agents.iter().any(|a| a.group == room);
            let there: Vec<&Station> = state.stations.iter().filter(has).collect();
            match (computer.filter(|c| !c.is_empty()), there.as_slice()) {
                (Some(named), _) => there.iter().find(|s| s.name == named).map(|s| s.radio.clone()).ok_or_else(|| format!("{named} has no team called {room}.")),
                (None, [only]) => Ok(only.radio.clone()),
                (None, []) => return,
                (None, several) => Err(format!("Which computer should it start on? {} each have a team called {room}.", several.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" and "))),
            }
        };
        let radio = match radio {
            Ok(radio) => Some(radio),
            Err(ask) => {
                self.shared.host.note(ask);
                None
            }
        };
        if let Some(radio) = radio {
            // the typed command, which the tower takes as it is; the chat shows the words beside it
            match tool {
                Some(tool) => radio.queue(format!("/launch {room} tool:{}", tool.id), None, None, Some(format!("New {} agent in {room}", tool.name))),
                None => radio.queue(format!("/launch {room}"), None, None, Some(format!("New agent in {room}"))),
            }
        }
    }

    /// Send what he typed. Who it is for, in this order: tags at its start (`@shop2`, `@shop`, `@all`); the
    /// message `re` if he picked one; the `agent` he is looking at; otherwise the tower works it out from his
    /// words, among the agents of the `room` he is in or among all of them. It goes to the computer that has
    /// whoever that is.
    pub fn send(&self, text: String, re: Option<String>, room: Option<String>, agent: Option<String>) {
        self.deliver(text, None, re, room, agent);
    }

    /// Send the words of one of his messages again, to another agent: the whole of them, fetched from the
    /// computer that has the message if the chat holds only their opening. Blocks while it fetches: call it
    /// off the main thread.
    pub fn resend(&self, message_id: String, agent_id: String) {
        let (text, callsign) = {
            let state = self.shared.state.lock().unwrap();
            let found = state.stations.iter().find_map(|s| s.messages.iter().find(|m| m.id == message_id).map(|m| (m.text.clone(), m.long)));
            let callsign = state.stations.iter().flat_map(|s| s.agents.iter()).find(|a| a.id == agent_id).map(|a| a.callsign.clone());
            (found, callsign)
        };
        let (Some((text, long)), Some(callsign)) = (text, callsign) else { return };
        let text = if long { self.full(message_id).unwrap_or(text) } else { text };
        self.deliver(text, None, None, None, Some(callsign));
    }

    /// Send a quick message: the feed shows `label`, the agent gets `text`. Who it is for is worked out as for
    /// anything he types.
    pub fn quick(&self, label: String, text: String, re: Option<String>, room: Option<String>, agent: Option<String>) {
        self.deliver(text, Some(label), re, room, agent);
    }

    /// Set going again every agent that has stopped without finishing: in one team, or everywhere.
    pub fn poke(&self, room: Option<String>) {
        let radios: Vec<Arc<Radio>> = {
            let state = self.shared.state.lock().unwrap();
            state.stations.iter().filter(|s| s.agents.iter().any(|a| a.state == "stalled" && room.as_ref().is_none_or(|r| a.group == *r))).map(|s| s.radio.clone()).collect()
        };
        for radio in radios {
            radio.queue("/poke".into(), None, room.as_ref().map(|r| format!("group:{r}")), None);
        }
        self.shared.host.changed();
    }

    /// He wants to speak: open the microphone when free. What he says goes to the message `re` if he picked
    /// one; otherwise the tower works out who it is for, among the agents of `room` or all of them.
    pub fn talk(&self, re: Option<String>, room: Option<String>, agent: Option<String>) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stations.is_empty() {
                return;
            }
            if let Some(id) = &re {
                state.answered.insert(id.clone());
            }
            state.discard = false;
            // any computer that is there can do the listening; where the words go is worked out when he sends them
            let pick = state.uploads.first().map(|u| u.station).unwrap_or_else(|| pick_station(&state, "", re.as_deref(), room.as_deref(), agent.as_deref()));
            let pick = if state.stations[pick].online { pick } else { state.stations.iter().position(|s| s.online).unwrap_or(pick) };
            state.stations[pick].radio.clone()
        };
        let shared = self.shared.clone();
        radio.dictate(Box::new(move |heard| {
            shared.host.hearing(String::new());
            if std::mem::take(&mut shared.state.lock().unwrap().discard) {
                return;
            }
            match heard {
                Listened::Said(text) => deliver(&shared, text, None, re, room, agent),
                Listened::Nothing => {}
                // why was said where it was found out (`ears.rs`): nothing chosen, what he chose could not, or
                // the microphone did not open
                Listened::Cannot => {}
            }
        }));
    }

    /// Why the microphone would be opened for nothing as things are, in words for him: nothing is chosen to
    /// listen with, or what he chose cannot be used. A host asks before it opens the microphone, and says
    /// this in its place. Nothing when something can listen.
    pub fn cannot_listen(&self) -> Option<String> {
        self.shared.senses.ears.cannot()
    }

    /// He took back what he was saying. The host closes the microphone next; nothing is sent.
    pub fn cancel_talk(&self) {
        self.shared.state.lock().unwrap().discard = true;
    }

    /// Attach a file from the device (a photo, a video): it starts on its way to the computer at once, and goes
    /// to whoever his next words go to. `name` is what to call it there; its extension says what it is. Returns
    /// its id straight away; [`DeskView::uploads`] shows how far it has got.
    pub fn attach(&self, file_path: String, name: String, re: Option<String>, room: Option<String>, agent: Option<String>) -> String {
        let id = random_hex(12);
        let dropped = Arc::new(AtomicBool::new(false));
        let total = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stations.is_empty() {
                return id;
            }
            // everything attached to one message goes to one computer: the first file's
            let station = state.uploads.first().map(|u| u.station).unwrap_or_else(|| pick_station(&state, "", re.as_deref(), room.as_deref(), agent.as_deref()));
            state.uploads.push(Upload { shown: DeskUpload { id: id.clone(), name: name.clone(), sent: 0, total, state: "sending".into() }, station, dropped: dropped.clone(), file_path: file_path.clone(), name: name.clone() });
            state.stations[station].radio.clone()
        };
        self.shared.host.changed();
        send_file(self.shared.clone(), radio, id.clone(), file_path, name, dropped);
        id
    }

    /// Take an attached file off again before it is sent.
    pub fn detach(&self, id: String) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            let Some(i) = state.uploads.iter().position(|u| u.shown.id == id) else { return };
            let gone = state.uploads.remove(i);
            gone.dropped.store(true, Ordering::SeqCst);
            state.stations.get(gone.station).map(|s| s.radio.clone())
        };
        if let Some(radio) = radio {
            radio.disarm(id);
        }
        self.shared.host.changed();
    }

    /// Call mode on every computer: transmissions are spoken one at a time and the microphone opens after each.
    pub fn set_call_mode(&self, on: bool) {
        let radios: Vec<Arc<Radio>> = {
            let mut state = self.shared.state.lock().unwrap();
            state.calling = on;
            state.stations.iter().map(|s| s.radio.clone()).collect()
        };
        if !on {
            self.shared.floor.clear();
        }
        // with no voice chosen there is nothing to read aloud with: he is told now, where he turned it on
        if on && !self.shared.senses.mouth.ready() {
            self.shared.host.note(crate::voice::NOTHING_CHOSEN.into());
        }
        radios.iter().for_each(|r| r.set_call_mode(on));
    }

    /// Exactly what the agent wrote. Blocks: call it off the main thread.
    pub fn full(&self, message_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.more(message_id, true)
    }

    /// Read a whole message aloud, in the voice he chose on this device. Blocks until it has been heard or
    /// [`Desk::stop_reading`] is called: call it off the main thread. False when it was not read (he is told why).
    pub fn read_aloud(&self, message_id: String) -> bool {
        self.radio_of(&message_id).is_some_and(|radio| radio.read_aloud(&message_id))
    }

    /// Read a question to him before he answers it: who is asking, its short form, and the answers it offers,
    /// in the voice he chose on this device. Blocks until it has been heard or [`Desk::stop_reading`] is
    /// called. False when it was not read.
    pub fn read_question(&self, message_id: String) -> bool {
        self.radio_of(&message_id).is_some_and(|radio| radio.read_question(&message_id))
    }

    /// Leave off reading aloud. The host stops what is already playing.
    pub fn stop_reading(&self) {
        let radios: Vec<Arc<Radio>> = self.shared.state.lock().unwrap().stations.iter().map(|s| s.radio.clone()).collect();
        radios.iter().for_each(|r| r.hush());
    }

    /// One of a message's pictures, as a file on the device. Blocks: call it off the main thread.
    pub fn picture(&self, message_id: String, picture_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.picture(picture_id)
    }

    /// Every paired computer's accounts, in the order the computers were paired. A computer that cannot be
    /// reached, or whose airband is too old, comes back with a note and no accounts. Blocks: call it off the
    /// main thread.
    pub fn accounts(&self, fresh: bool) -> Vec<StationAccounts> {
        // a computer is always headed by a name: its own, or its place in the list until it has said its own
        let radios: Vec<(String, Arc<Radio>)> = self.shared.state.lock().unwrap().stations.iter().enumerate().map(|(i, s)| (if s.name.is_empty() { format!("computer {}", i + 1) } else { s.name.clone() }, s.radio.clone())).collect();
        // one after another, not all at once: two computers on the same account asking Anthropic in the same
        // moment is how one of them gets refused
        let mut all: Vec<StationAccounts> = radios
            .into_iter()
            .map(|(name, radio)| {
                radio.accounts(name.clone(), fresh).unwrap_or_else(|e| StationAccounts {
                    station: name,
                    accounts: Vec::new(),
                    note: Some(match e {
                        RadioError::Told { reason } => reason,
                        RadioError::Offline => "Cannot reach this computer.".into(),
                        _ => "This computer would not say.".into(),
                    }),
                    checked_at: 0,
                })
            })
            .collect();
        // the same account on two computers has one allowance: where one computer could not be told it, what
        // the other was told is shown
        let known: Vec<(String, String, Vec<AccountLimit>)> = all.iter().flat_map(|s| s.accounts.iter()).filter(|a| !a.limits.is_empty()).map(|a| (a.harness.clone(), a.label.clone(), a.limits.clone())).collect();
        for a in all.iter_mut().flat_map(|s| s.accounts.iter_mut()).filter(|a| a.limits.is_empty()) {
            if let Some((_, _, limits)) = known.iter().find(|(harness, label, _)| *harness == a.harness && *label == a.label) {
                a.limits = limits.clone();
                a.problem = None;
            }
        }
        all
    }

    /// Move one computer's sessions to another of its accounts. `station`, `harness` and `id` are as
    /// [`Desk::accounts`] gave them. Blocks: call it off the main thread.
    pub fn use_account(&self, station: String, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.name == station).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.use_account(station, harness, id)
    }

    /// Put an agent in another team: one of the teams on screen, or a new one of that name. `agent` is its id.
    /// The list shows it there at once. Blocks: call it off the main thread.
    pub fn move_agent(&self, agent: String, team: String) -> Result<(), RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.agents.iter().any(|a| a.id == agent)).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        let now = radio.move_agent(agent.clone(), team)?;
        {
            // shown as the tower now has it, without waiting for the next look at the feed
            let mut state = self.shared.state.lock().unwrap();
            for a in state.stations.iter_mut().flat_map(|s| s.agents.iter_mut()).filter(|a| a.id == agent) {
                (a.callsign, a.group) = (now[0].clone(), now[1].clone());
            }
        }
        self.shared.host.changed();
        Ok(())
    }

    /// Stop what an agent is doing, now: one its computer runs itself. What he said to it meanwhile it then
    /// reads together. Blocks: call it off the main thread.
    pub fn stop_agent(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.agents.iter().any(|a| a.id == agent)).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.stop_agent(agent)
    }

    /// End an agent its computer runs itself: it finishes and leaves the radio. Blocks: call it off the main thread.
    pub fn end_agent(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.agents.iter().any(|a| a.id == agent)).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.end_agent(agent)
    }

    /// Let a question go without answering it: it stops waiting on him, here at once and on its computer.
    /// Blocks: call it off the main thread.
    pub fn dismiss(&self, message_id: String) -> Result<(), RadioError> {
        let radio = self.radio_of(&message_id).ok_or(RadioError::Refused)?;
        self.shared.state.lock().unwrap().answered.insert(message_id.clone());
        self.shared.host.changed();
        radio.dismiss(message_id)
    }

    /// Try again a message that was not sent ([`Outgoing::failed`]). It goes after anything else still waiting.
    pub fn send_again(&self, id: String) {
        if let Some(radio) = self.sender_of(&id) {
            radio.send_again(&id);
        }
    }
    /// Let go of a message that was not sent. Returns its words, for putting back where he types.
    pub fn discard_unsent(&self, id: String) -> Option<String> {
        self.sender_of(&id).and_then(|radio| radio.discard(&id)).map(|o| o.label.unwrap_or(o.text))
    }
    /// The computer's radio that a message still waiting to go, or given up on, belongs to.
    fn sender_of(&self, id: &str) -> Option<Arc<Radio>> {
        self.shared.state.lock().unwrap().stations.iter().find(|s| s.sending.iter().any(|o| o.id == id)).map(|s| s.radio.clone())
    }

    /// For each computer: how it was last reached, or not, through the relay and at its own address on the
    /// network the phone is on. From what this phone has seen itself; nothing is asked of anyone to make it.
    pub fn reach(&self) -> Vec<DeskReach> {
        let radios: Vec<(String, Arc<Radio>)> = self.shared.state.lock().unwrap().stations.iter().map(|s| (s.name.clone(), s.radio.clone())).collect();
        radios
            .into_iter()
            .map(|(computer, radio)| {
                let (relay, near) = radio.reach();
                DeskReach { computer, relay: relay.unwrap_or_default(), near: near.unwrap_or_default() }
            })
            .collect()
    }

    /// What to copy when something is wrong with the connection: for each computer, every request that failed
    /// lately and every try at sending something he said, with the time, the way it went, how it ended and how
    /// long it took. Nothing that he or an agent said is in it, and no key or address. It is made on this
    /// device from what this device kept, and goes nowhere unless he pastes it somewhere.
    pub fn diagnostics(&self) -> String {
        let radios: Vec<(String, Arc<Radio>)> = self.shared.state.lock().unwrap().stations.iter().map(|s| (s.name.clone(), s.radio.clone())).collect();
        let mut out = vec![format!("airband {} connection record", env!("CARGO_PKG_VERSION"))];
        for (computer, radio) in radios {
            let trail = radio.link_trail();
            out.push(String::new());
            out.push(if computer.is_empty() { "This computer".to_string() } else { computer });
            if trail.is_empty() {
                out.push("nothing has failed".into());
            }
            out.extend(trail);
            // what the computer is told in passing (he heard this, he saw that) and did not get
            let lost = radio.lost();
            if !lost.is_empty() {
                out.push(format!("told in passing and not delivered, since the app was opened: {}", lost.iter().map(|(what, n)| format!("{what} {n}")).collect::<Vec<_>>().join(", ")));
            }
        }
        out.join("\n")
    }

    /// Take back something he said that is still waiting its turn. Blocks: call it off the main thread.
    pub fn unsend(&self, message_id: String) -> Result<(), RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.unsend(message_id)
    }

    /// Have its computer take over an agent he started in a terminal. Blocks: call it off the main thread.
    pub fn take_over(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.agents.iter().any(|a| a.id == agent)).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.take_over(agent)
    }

    /// Stop for good.
    pub fn stop(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.generation += 1;
        state.stations.drain(..).for_each(|s| s.radio.stop());
    }
}

impl Desk {
    fn deliver(&self, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>) {
        deliver(&self.shared, text, label, re, room, agent);
    }

    /// The computer called this. With no name it is the one computer he has; with several he has to say
    /// which, and none is picked for him.
    fn station_radio(&self, computer: &str) -> Result<Arc<Radio>, RadioError> {
        let state = self.shared.state.lock().unwrap();
        let found = match (computer.is_empty(), state.stations.as_slice()) {
            (true, [only]) => Some(only),
            (true, [_, _, ..]) => return Err(RadioError::Told { reason: "Choose which computer first.".into() }),
            _ => state.stations.iter().find(|s| s.name == computer),
        };
        found.filter(|s| s.online).map(|s| s.radio.clone()).ok_or(RadioError::Offline)
    }
    fn radio_of(&self, message: &str) -> Option<Arc<Radio>> {
        let state = self.shared.state.lock().unwrap();
        state.owner(message).map(|i| state.stations[i].radio.clone())
    }
}

/// Send something of his: typed, tapped or said. Who it is for, and so which computer it goes to, is worked out
/// from what he can see; the tower it goes to is told about the agents the other computers have, so it can say
/// the words are for one of those.
fn deliver(shared: &Arc<Shared>, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let (radio, text, re, scope, others) = {
        let mut state = shared.state.lock().unwrap();
        if state.stations.is_empty() {
            return;
        }
        // something new said: whatever was waiting to be placed is let go
        state.asking = None;
        let tagged = text.starts_with('@') || text.starts_with('/');
        let re = re.filter(|_| !tagged);
        if let Some(id) = &re {
            state.answered.insert(id.clone());
        }
        // looking at one agent, with nobody tagged and no message picked: it is for that agent
        let aimed = matches!((&agent, tagged, &re), (Some(_), false, None));
        let wanted = pick_station(&state, &text, re.as_deref(), room.as_deref(), agent.as_deref());
        // Files he attached are already on one computer. Words with no address go there with them. Words he
        // addressed (a tag, a message he picked, the agent he is looking at) go where they are addressed, and
        // if that is another computer the files are sent there first: the words wait for them.
        let files_at = state.uploads.first().map(|u| u.station);
        let pick = match files_at {
            Some(at) if at != wanted && (tagged || aimed || re.is_some()) => {
                let to = state.stations[wanted].radio.clone();
                let mut moved = Vec::new();
                for u in state.uploads.iter_mut() {
                    u.dropped.store(true, Ordering::SeqCst);
                    u.dropped = Arc::new(AtomicBool::new(false));
                    moved.push((u.station, u.shown.id.clone(), u.file_path.clone(), u.name.clone(), u.dropped.clone()));
                    u.station = wanted;
                    u.shown.sent = 0;
                    u.shown.state = "sending".into();
                }
                for (was, id, file_path, name, dropped) in moved {
                    if let Some(old) = state.stations.get(was) {
                        old.radio.disarm(id.clone());
                    }
                    send_file(shared.clone(), to.clone(), id, file_path, name, dropped);
                }
                // send the words when the files are there (or have failed, or he gave up on them)
                let (again, text, label, re, room, agent) = (shared.clone(), text.clone(), label.clone(), re.clone(), room.clone(), agent.clone());
                thread::spawn(move || {
                    for _ in 0..1200 {
                        thread::sleep(Duration::from_millis(250));
                        if !again.state.lock().unwrap().uploads.iter().any(|u| u.shown.state == "sending") {
                            break;
                        }
                    }
                    deliver(&again, text, label, re, room, agent);
                });
                drop(state);
                shared.host.changed();
                return;
            }
            Some(at) => at,
            None => wanted,
        };
        // the ones that have arrived leave with this message
        state.uploads.retain(|u| !(u.station == pick && u.shown.state == "ready"));
        let text = match (&agent, aimed) {
            (Some(callsign), true) => format!("@{} {text}", callsign.replace(' ', "")),
            _ => text,
        };
        let scope = room.clone().filter(|_| re.is_none() && agent.is_none() && !tagged).map(|r| format!("group:{r}"));
        // when the tower has to work out who it is for, it is told who else is on his screen
        let others = if tagged || aimed || re.is_some() || state.stations.len() < 2 { String::new() } else { others(&state, pick, room.as_deref()) };
        (state.stations[pick].radio.clone(), text, re, scope, others)
    };
    radio.queue_with(text, re, scope, label, others);
    shared.host.changed();
}

/// The agents on his screen that computers other than `pick` have, each with the last thing it said: for the
/// tower his words are going to. In one room, only that room's.
fn others(state: &State, pick: usize, room: Option<&str>) -> String {
    let now = now_ms();
    let mut seen = HashSet::new();
    state.stations[pick].agents.iter().for_each(|a| {
        seen.insert(a.id.clone());
    });
    let list: Vec<airband_core::radio::Other> = state
        .stations
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != pick)
        .flat_map(|(_, s)| s.agents.iter().map(move |a| (s, a)))
        .filter(|(_, a)| room.is_none_or(|r| a.group == r) && seen.insert(a.id.clone()))
        .map(|(s, a)| {
            let last = s.messages.iter().rev().find(|m| m.from == a.callsign && !m.fyi);
            airband_core::radio::Other { callsign: a.callsign.clone(), group: a.group.clone(), said: last.map(|m| m.text.chars().take(240).collect()).unwrap_or_default(), ago: last.map(|m| now.saturating_sub(m.at) / 1000).unwrap_or(0) }
        })
        .collect();
    if list.is_empty() { String::new() } else { serde_json::to_string(&list).unwrap_or_default() }
}

/// Which computer a message of his goes to.
fn pick_station(state: &State, text: &str, re: Option<&str>, room: Option<&str>, agent: Option<&str>) -> usize {
    // the computer that has the message he is answering
    if let Some(i) = re.and_then(|id| state.owner(id)) {
        return i;
    }
    // the computer that has whoever he tagged: read the tags the way the tower will
    if text.starts_with('@') {
        let named = state.stations.iter().position(|s| {
            let live: Vec<Callsign> = s.agents.iter().map(|a| Callsign { project: a.group.clone(), number: a.callsign.strip_prefix(&a.group).unwrap_or(&a.callsign).trim().to_string() }).collect();
            !parse_utterance(text, &live).to.is_empty()
        });
        if let Some(i) = named {
            return i;
        }
    }
    if let Some(i) = agent.and_then(|c| state.stations.iter().position(|s| s.agents.iter().any(|a| a.callsign == c))) {
        return i;
    }
    // Nothing says. Go by what is on his screen: the computer of the latest thing in the room he is looking at
    // (or anywhere), whether an agent said it or he did. A reply follows what was just said; a second message
    // of his follows his first. The tower there is told who else he can see, and passes it on if it is for them.
    let here = |s: &Station, m: &ChatMessage| match room {
        None => true,
        Some(r) if m.from == "you" => m.to.iter().any(|c| s.agents.iter().any(|a| a.callsign == *c && a.group == r)),
        Some(r) => m.group == r,
    };
    let last = state.stations.iter().enumerate().filter_map(|(i, s)| s.messages.iter().rev().find(|m| !m.fyi && here(s, m)).map(|m| (m.at, i))).max();
    if let Some((_, i)) = last {
        return i;
    }
    if let Some(i) = room.and_then(|r| state.stations.iter().position(|s| s.agents.iter().any(|a| a.group == r))) {
        return i;
    }
    0
}

/// Tell one computer where this device can be woken, if it has not been told. A computer that cannot be reached
/// or does not take it is told the next time round.
fn tell_push(shared: &Arc<Shared>, radio: &Arc<Radio>, index: usize, generation: u64) {
    let (link, push) = {
        let state = shared.state.lock().unwrap();
        let Some(station) = state.stations.get(index).filter(|_| state.generation == generation) else { return };
        let Some(push) = state.push.clone().filter(|_| !state.push_told.contains(&station.link)) else { return };
        (station.link.clone(), push)
    };
    if radio.push_target(&push.0, &push.1).is_ok() {
        let mut state = shared.state.lock().unwrap();
        // unless the address changed while this one was on its way
        if state.push.as_ref() == Some(&push) {
            state.push_told.insert(link);
        }
    }
}

/// Tell one computer which coding tools he uses, if it has not been told or he has since changed them: a new
/// agent he asks for without naming a tool is started with the one he uses most.
fn tell_tools(shared: &Arc<Shared>, radio: &Arc<Radio>, index: usize, generation: u64) {
    let tools = shared.chooser.tools();
    let link = {
        let state = shared.state.lock().unwrap();
        let Some(station) = state.stations.get(index).filter(|_| state.generation == generation) else { return };
        if state.tools_told.get(&station.link) == Some(&tools) {
            return;
        }
        station.link.clone()
    };
    if radio.uses_tools(&tools).is_ok() {
        shared.state.lock().unwrap().tools_told.insert(link, tools);
    }
}

/// Tell one computer where this device's banner can be put up and changed, if what it was told is not what is
/// so now. A computer that cannot be reached is told the next time round.
fn tell_banner(shared: &Arc<Shared>, radio: &Arc<Radio>, index: usize, generation: u64) {
    use airband_core::radio::BannerAddress;
    let (link, provider, now, told) = {
        let state = shared.state.lock().unwrap();
        let Some(station) = state.stations.get(index).filter(|_| state.generation == generation && !state.banner.provider.is_empty()) else { return };
        let now = (state.banner.start.clone(), state.banner.update.get(&station.link).cloned().unwrap_or_default());
        (station.link.clone(), state.banner.provider.clone(), now, state.banner.told.get(&station.link).cloned().unwrap_or_default())
    };
    for (which, token, was) in [(BannerAddress::Start, &now.0, &told.0), (BannerAddress::Update, &now.1, &told.1)] {
        if token == was || radio.banner_target(&provider, which, token).is_err() {
            continue;
        }
        let mut state = shared.state.lock().unwrap();
        let told = state.banner.told.entry(link.clone()).or_default();
        match which {
            BannerAddress::Start => told.0 = token.clone(),
            BannerAddress::Update => told.1 = token.clone(),
        }
    }
}

/// What a notification says once this device has opened it: everything a host needs to show it and to send an
/// answer to it, with the app running or not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OpenedNotice {
    /// The computer it came from, for [`Desk::answer_now`].
    pub station: String,
    /// The message it is about.
    pub message: String,
    /// Who sent it, as the app shows the name.
    pub title: String,
    /// The computer's name, when he has more than one.
    pub subtitle: String,
    pub body: String,
    /// The agent's two characters, and its team's place in the host's list of team colours.
    pub badge: String,
    pub colour: u8,
    /// The agent's team and its name: where the app opens when he taps the notification.
    pub room: String,
    pub agent: String,
    /// It is a question to him (not a report).
    pub question: bool,
    /// Answers to offer as buttons.
    pub options: Vec<String>,
    /// It asks permission: the buttons are Allow and Deny, and words of his own refuse it.
    pub permission: bool,
}

/// What names a computer to this device, from its pairing link: nothing secret, the same every time.
fn station_key(link: &str) -> Option<String> {
    match Pairing::parse(link).ok()? {
        Pairing::Relay { secret, .. } | Pairing::Near { secret, .. } => Keys::derive(&secret).ok().map(|k| k.channel),
        Pairing::Direct { .. } => None,
    }
}

/// Open a sealed notification with whichever pairing sealed it. `links` are the pairing links the host keeps;
/// `sealed` is the notification as the push service delivered it. Nothing when none of them opens it. This
/// needs no desk: a host calls it from wherever its system hands it a notification.
/// What a button answers a request for leave with: `true` for the one that allows. A host's buttons send this
/// and have no words of their own for it.
#[uniffi::export]
pub fn leave_answer(allow: bool) -> String {
    if allow { airband_core::radio::ALLOW } else { airband_core::radio::DENY }.to_string()
}

#[uniffi::export]
pub fn open_notice(links: Vec<String>, sealed: String) -> Option<OpenedNotice> {
    use airband_core::routing::{badge, shown};
    let sealed = airband_core::audio::base64_decode(&sealed)?;
    let several = links.len() > 1;
    links.iter().find_map(|link| {
        let Pairing::Relay { secret, .. } = Pairing::parse(link).ok()? else { return None };
        let keys = Keys::derive(&secret).ok()?;
        let n: airband_core::radio::Notice = serde_json::from_slice(&keys.open(Direction::Push, &sealed).ok()?).ok()?;
        Some(OpenedNotice {
            station: keys.channel,
            message: n.id,
            title: shown(&n.from),
            subtitle: if several { n.station } else { String::new() },
            body: n.text,
            badge: badge(&n.group, &n.from),
            colour: n.colour,
            room: n.group,
            agent: shown(&n.from),
            question: n.kind == Some(Kind::NeedsYou),
            options: if n.permission { Vec::new() } else { n.options },
            permission: n.permission,
        })
    })
}

/// Keeps one computer's part of the chat current. One request is always open with its tower.
fn follow(shared: Arc<Shared>, radio: Arc<Radio>, index: usize, generation: u64) {
    let mut first = true;
    let mut told_taken: Option<Vec<String>> = None;
    loop {
        // a computer that has not been told where this device can be woken is told as soon as it is reached
        tell_push(&shared, &radio, index, generation);
        tell_banner(&shared, &radio, index, generation);
        tell_tools(&shared, &radio, index, generation);
        // the tower is told whether the chat is in front of him: only then does having it count as seeing it
        radio.set_away(!shared.host.watching());
        // the first answer must come at once: it is what says the tower is there and who is on it
        let got = radio.follow(if first { 0 } else { 25 });
        let mut seen: Vec<String> = Vec::new();
        // this computer has just been reached, or says it now reads with something else
        let mut reached = false;
        // what reads there has just stopped working, in words for him
        let mut reads_trouble: Option<String> = None;
        {
            let mut state = shared.state.lock().unwrap();
            if state.generation != generation {
                return;
            }
            let calling = state.calling;
            let open: HashSet<String> = state.stations.iter().flat_map(|s| s.open.iter().cloned()).collect();
            let Some(station) = state.stations.get_mut(index) else { return };
            match &got {
                Ok(feed) => {
                    first = false;
                    station.tried = true;
                    station.fails = 0;
                    station.failing_since = None;
                    station.answered_at = Some(std::time::Instant::now());
                    reached = !station.online || station.reads != feed.reads || station.reads_failed != feed.reads_failed;
                    station.online = true;
                    station.name = feed.station.clone();
                    station.reads = feed.reads.clone();
                    // what reads there has stopped working: he is told once, when it stops, and each message
                    // comes as it was written until it works again
                    if !feed.reads_failed.is_empty() && station.reads_failed != feed.reads_failed {
                        reads_trouble = Some(format!("Summaries are not working on {}. {} You are getting each message as it was written. To change what makes them, open Settings, then Summaries.", if feed.station.is_empty() { "your computer" } else { feed.station.as_str() }, feed.reads_failed));
                    }
                    station.reads_failed = feed.reads_failed.clone();
                    station.agents = feed.agents.clone();
                    station.open = feed.open.iter().cloned().collect();
                    station.quick = feed.quick.clone();
                    station.colours = feed.colours.clone();
                    // the radio keeps the chat itself, each line in its current form
                    station.messages = radio.history();
                    // on screen in front of him now: read, so a call started later does not read it out
                    if !calling && shared.host.watching() {
                        seen = feed.messages.iter().filter(|m| m.from != "you" && !m.needs_you).map(|m| m.id.clone()).collect();
                    }
                }
                // one request lost is not a computer out of reach: it is asked again at once, and only a second
                // failure says so
                Err(_) => {
                    station.fails += 1;
                    // a computer being restarted is back in a few seconds: it is only said to be out of reach
                    // when it has been for a while (or has never once answered)
                    let long = station.failing_since.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(10);
                    if station.fails >= 2 && (long || station.answered_at.is_none()) {
                        station.tried = true;
                        station.online = false;
                    }
                }
            }
            // once the tower agrees a message is answered, stop tracking it here
            let now_open: HashSet<String> = state.stations.iter().flat_map(|s| s.open.iter().cloned()).collect();
            if now_open != open {
                state.answered.retain(|id| now_open.contains(id));
            }
        }
        // the names he is likely to say, for whatever listens on this device: his agents and their teams
        if got.is_ok() {
            let names: Vec<String> = shared.state.lock().unwrap().stations.iter().flat_map(|s| s.agents.iter().flat_map(|a| [a.callsign.clone(), a.group.clone()])).collect();
            shared.senses.ears.expect(names);
        }
        shared.host.changed();
        if let Some(trouble) = reads_trouble {
            shared.host.note(trouble);
        }
        if !seen.is_empty() {
            radio.seen(seen);
        }
        // What waits to write his summaries goes to it, and it is asked what it can read with: on a thread of
        // its own, because the computer tries a choice with a real call before it answers.
        if reached {
            let (shared, radio) = (shared.clone(), radio.clone());
            thread::spawn(move || crate::choices::computer_reached(&shared, &radio));
        }
        // Two computers number their agents without knowing of each other. The later one in his list is told
        // the names the earlier ones use, whenever those change, and keeps off them.
        if got.is_ok() && index > 0 {
            let elsewhere: Vec<String> = {
                let state = shared.state.lock().unwrap();
                let mut names: Vec<String> = state.stations.iter().take(index).flat_map(|s| s.agents.iter().map(|a| a.callsign.clone())).collect();
                names.sort();
                names.dedup();
                names
            };
            if told_taken.as_ref() != Some(&elsewhere) && radio.taken(&elsewhere) {
                told_taken = Some(elsewhere);
            }
        }
        if got.is_err() {
            let fails = shared.state.lock().unwrap().stations.get(index).map(|s| s.fails).unwrap_or(1);
            thread::sleep(Duration::from_millis(if fails <= 1 { 300 } else { 2500 }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;
    impl RadioHost for Quiet {
        fn told(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
        fn speak(&self, _: String) {}
        fn listen(&self) -> Option<String> {
            None
        }
        fn status(&self, _: RadioStatus) {}
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
        fn hearing(&self, _: String) {}
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }

    fn msg(id: &str, seq: u64, from: &str, group: &str, needs: bool) -> ChatMessage {
        ChatMessage { seq, id: id.into(), at: seq, from: from.into(), group: group.into(), to: Vec::new(), needs_you: needs, fyi: false, stalled: false, text: id.into(), long: false, queued: false, seen: 0, options: Vec::new(), recommended: None, permission: false, pictures: Vec::new(), links: Vec::new(), agent: Some(from.into()), station: String::new(), open: needs, badge: String::new(), colour: 0, read: None, own_session: false, not_read: String::new() }
    }
    fn station(name: &str, port: u16, agents: &[(&str, &str)], messages: Vec<ChatMessage>) -> Station {
        let link = format!("airband://pair?url=http%3A%2F%2F127.0.0.1%3A{port}&token=t");
        let radio = Radio::start(link.clone(), String::new(), Arc::new(Quiet)).unwrap();
        radio.stop();
        Station {
            link,
            radio,
            name: name.into(),
            online: true,
            tried: true,
            answered_at: None,
            fails: 0,
            failing_since: None,
            reads: String::new(),
            reads_failed: String::new(),
            agents: agents.iter().map(|(c, g)| ChatAgent { id: c.to_string(), callsign: c.to_string(), group: g.to_string(), number: String::new(), harness: "h".into(), waiting: false, reachable: true, state: "working".into(), subagents: 0, doing: None, held: false, model: None, effort: None }).collect(),
            open: messages.iter().filter(|m| m.needs_you).map(|m| m.id.clone()).collect(),
            sending: Vec::new(),
            landing: Vec::new(),
            quick: Vec::new(),
            colours: Default::default(),
            messages,
        }
    }
    fn two() -> State {
        State {
            stations: vec![
                station("mini", 1, &[("shop two", "shop"), ("backoffice login fix", "backoffice")], vec![msg("q1", 1, "shop two", "shop", true), msg("d1", 2, "backoffice login fix", "backoffice", false)]),
                station("macbook", 2, &[("ff one", "ff")], vec![msg("q2", 3, "ff one", "ff", true)]),
            ],
            ..State::default()
        }
    }

    struct Watching(std::sync::atomic::AtomicBool);
    impl DeskHost for Watching {
        fn secret(&self, _: String) -> Option<String> {
            None
        }
        fn keep_secret(&self, _: String, _: Option<String>) {}
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn changed(&self) {}
        fn status(&self, _: RadioStatus, _: String) {}
        fn note(&self, _: String) {}
        fn hearing(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn watching(&self) -> bool {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn device(&self) -> HostDevice {
            HostDevice::default()
        }
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn a_computer_is_never_picked_for_him() {
        struct Noted(Mutex<Vec<String>>);
        impl DeskHost for Noted {
            fn secret(&self, _: String) -> Option<String> {
                None
            }
            fn keep_secret(&self, _: String, _: Option<String>) {}
            fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
            fn record_start(&self, _: u32) -> bool {
                false
            }
            fn record_read(&self) -> Vec<u8> {
                Vec::new()
            }
            fn record_stop(&self) {}
            fn changed(&self) {}
            fn status(&self, _: RadioStatus, _: String) {}
            fn note(&self, text: String) {
                self.0.lock().unwrap().push(text);
            }
            fn hearing(&self, _: String) {}
            fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
            fn watching(&self) -> bool {
                true
            }
            fn device(&self) -> HostDevice {
                HostDevice::default()
            }
            fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
                None
            }
        }
        let host = Arc::new(Noted(Mutex::new(Vec::new())));
        // two computers that each have a team called shop
        let state = State { stations: vec![station("mini", 1, &[("shop two", "shop")], Vec::new()), station("macbook", 2, &[("shop one", "shop")], Vec::new())], ..State::default() };
        let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: String::new() };
        // with none named he is asked which, and one that has no such team is not swapped for one that has
        desk.new_agent("shop".into(), None, None);
        desk.new_agent("shop".into(), Some("laptop".into()), None);
        // a team no computer has: nothing happens, and nothing is said
        desk.new_agent("garden".into(), None, None);
        let notes = host.0.lock().unwrap().clone();
        assert_eq!(notes, vec!["Which computer should it start on? mini and macbook each have a team called shop.".to_string(), "laptop has no team called shop.".to_string()]);
        // the same for anything else asked of a computer he did not name
        assert!(matches!(desk.station_radio(""), Err(RadioError::Told { .. })));
        assert!(matches!(desk.station_radio("laptop"), Err(RadioError::Offline)));
    }

    #[test]
    fn the_connection_record_he_can_copy_holds_nothing_said_and_nothing_secret() {
        let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
        let dir = std::env::temp_dir().join(format!("airband-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // paired through a relay nobody is at, so what he says fails and is written down
        let secret = "ef".repeat(32);
        let link = format!("airband://pair?relay=http%3A%2F%2F127.0.0.1%3A9&secret={secret}");
        let radio = Radio::start(link.clone(), dir.display().to_string(), Arc::new(Quiet)).unwrap();
        radio.queue("the door code is 4471".into(), None, None, None);
        for _ in 0..200 {
            if radio.link_trail().iter().any(|l| l.contains("/v1/radio/utterance")) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        radio.seen(vec!["m-private-id".into()]);
        let mut state = State { stations: vec![station("mini", 1, &[], vec![msg("an agent's own words", 1, "shop two", "shop", false)])], ..State::default() };
        state.stations[0].radio = radio.clone();
        state.stations[0].link = link;
        let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: dir.display().to_string() };
        let record = desk.diagnostics();
        // what is in it: which computer, each try, the way it went and how it ended, and what was told in passing and lost
        assert!(record.starts_with("airband ") && record.contains("\nmini\n"), "{record}");
        assert!(record.contains("POST /v1/radio/utterance by relay: there was no connection to the relay"), "{record}");
        assert!(record.contains("told in passing and not delivered, since the app was opened: heard 1"), "{record}");
        // what is not: anything he or an agent said, an id of a message, the pairing's secret or anything made
        // from it, or an address
        let keys = Keys::derive(&secret).unwrap();
        for private in ["door code", "4471", "an agent's own words", "m-private-id", secret.as_str(), keys.channel.as_str(), keys.radio_token.as_str(), keys.pull_token.as_str(), "127.0.0.1", "http"] {
            assert!(!record.contains(private), "the record holds {private:?}: {record}");
        }
        radio.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_he_is_not_looking_at_stays_unread_and_is_counted() {
        let host = Arc::new(Watching(std::sync::atomic::AtomicBool::new(true)));
        let dir = std::env::temp_dir().join(format!("airband-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let desk = Desk { shared: Arc::new(Shared { state: Mutex::new(two()), floor: Floor::default(), host: host.clone(), chooser: crate::choices::Chooser::open("", HostDevice::default()), senses: Arc::default(), key_test: crate::keys::pretend }), data_dir: dir.display().to_string() };
        let counts = |v: &DeskView| (v.unread, v.rooms.iter().map(|r| (r.name.clone(), r.unread)).collect::<Vec<_>>());
        // looking at one team: what it said is read, and the other two teams' one thing each is not
        let v = desk.view(Some("shop".into()), None);
        assert_eq!(counts(&v), (2, vec![("backoffice".into(), 1), ("ff".into(), 1), ("shop".into(), 0)]));
        assert_eq!(v.rooms[0].members[0].unread, 1);
        // something new from the team in front of him is read as it comes; from another, it is one more
        {
            let mut state = desk.shared.state.lock().unwrap();
            state.stations[0].messages.push(msg("r2", 5, "shop two", "shop", false));
            state.stations[1].messages.push(msg("r3", 6, "ff one", "ff", false));
        }
        assert_eq!(counts(&desk.view(Some("shop".into()), None)).0, 3);
        // with the chat not on screen nothing is read: one agent of a team opened behind a sheet clears nothing
        host.0.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(desk.view(Some("ff".into()), Some("ff one".into())).unread, 3);
        host.0.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(counts(&desk.view(Some("ff".into()), Some("ff one".into()))), (1, vec![("backoffice".into(), 1), ("ff".into(), 0), ("shop".into(), 0)]));
        // what was read is kept on the device: a desk opened again on the same folder starts from there
        assert_eq!(ReadMarks::load(&dir.display().to_string()).upto.get("ff one"), Some(&6));
        // everything at once
        assert_eq!(desk.view(None, None).unread, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_notification_is_opened_by_the_pairing_that_sealed_it_and_no_other() {
        use airband_core::radio::Notice;
        let link = |secret: &str| Pairing::Relay { relay: "https://relay.example".into(), secret: secret.into() }.link();
        let (mini, laptop, stranger) = (airband_core::relay::new_secret(), airband_core::relay::new_secret(), airband_core::relay::new_secret());
        let keys = Keys::derive(&laptop).unwrap();
        let seal = |n: &Notice| airband_core::audio::base64_encode(&keys.seal(Direction::Push, &serde_json::to_vec(n).unwrap()));
        let question = Notice { id: "m1".into(), from: "shop three".into(), group: "shop".into(), colour: 2, station: "laptop".into(), text: "Rewrite or leave?".into(), kind: Some(Kind::NeedsYou), options: vec!["Rewrite".into(), "Leave".into()], permission: false };
        let got = open_notice(vec![link(&mini), link(&laptop)], seal(&question)).unwrap();
        assert_eq!((got.title.as_str(), got.subtitle.as_str(), got.body.as_str(), got.badge.as_str(), got.colour), ("shop 3", "laptop", "Rewrite or leave?", "S3", 2));
        assert_eq!((got.station.as_str(), got.message.as_str(), got.room.as_str(), got.agent.as_str(), got.question), (keys.channel.as_str(), "m1", "shop", "shop 3", true));
        assert_eq!(got.options, ["Rewrite", "Leave"]);
        // one computer: its name says nothing he does not know
        assert_eq!(open_notice(vec![link(&laptop)], seal(&question)).unwrap().subtitle, "");
        // a permission card offers its own two answers, not the tower's list
        let card = open_notice(vec![link(&laptop)], seal(&Notice { permission: true, options: vec!["Allow".into(), "Deny".into()], ..question.clone() })).unwrap();
        assert!(card.permission && card.options.is_empty());
        // not by another pairing, not as an ordinary answer from the tower, not when it is not base64 at all
        assert!(open_notice(vec![link(&mini), link(&stranger)], seal(&question)).is_none());
        let answer = airband_core::audio::base64_encode(&keys.seal(Direction::ToRadio, &serde_json::to_vec(&question).unwrap()));
        assert!(open_notice(vec![link(&laptop)], answer).is_none());
        assert!(open_notice(vec![link(&laptop)], "not sealed".into()).is_none());
        assert_eq!(station_key(&link(&laptop)), Some(keys.channel.clone()));
    }

    #[test]
    fn a_message_goes_to_the_computer_that_has_who_it_is_for() {
        let state = two();
        // the message he is answering
        assert_eq!(pick_station(&state, "yes", Some("q2"), None, None), 1);
        // a tag, in every form the tower accepts
        assert_eq!(pick_station(&state, "@ff1 stop", None, None, None), 1);
        assert_eq!(pick_station(&state, "@ffone stop", None, None, None), 1);
        assert_eq!(pick_station(&state, "@shop2 go", None, None, None), 0);
        assert_eq!(pick_station(&state, "@backoffice ship it", None, None, None), 0);
        // the room or the agent he is looking at
        assert_eq!(pick_station(&state, "go", None, Some("ff"), None), 1);
        assert_eq!(pick_station(&state, "go", None, None, Some("backoffice login fix")), 0);
        // nothing says: whoever spoke last
        assert_eq!(pick_station(&state, "go", None, None, None), 1);
    }

    #[test]
    fn waiting_is_what_is_open_and_not_just_answered() {
        let mut state = two();
        assert!(state.is_open("q1") && state.is_open("q2") && !state.is_open("d1"));
        state.answered.insert("q1".into());
        assert!(!state.is_open("q1"));
        // labelled with its computer, because there are two
        assert_eq!(state.merged().iter().map(|m| (m.id.as_str(), m.station.as_str())).collect::<Vec<_>>(), [("q1", "mini"), ("d1", "mini"), ("q2", "macbook")]);
    }
}
