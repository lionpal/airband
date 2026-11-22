//! The desk: every computer he is paired with, as one. A host that shows a chat talks to this and to nothing
//! else. It keeps one [`Radio`] per computer and owns everything that is the same on any platform: merging
//! their messages, what counts as waiting on him and what as done, which computer a message of his goes to,
//! which computer has the floor on a call, and marking what he has read. What is left for a host is to draw
//! the [`DeskView`], make and capture sound, and keep the pairing links somewhere safe.
use super::*;
use airband_core::routing::{Callsign, parse_utterance};
use crate::services::CodingTool;
use std::collections::{HashMap, HashSet};
use std::sync::Condvar;
mod agents;
mod colours;
mod follow;
mod ids;
mod notice;
mod reading;
mod records;
mod rules;
mod send;
mod talk;
mod station_host;
mod tell;
#[cfg(test)]
mod tests;
mod uploads;
mod view;

pub use notice::*;
use ids::*;

pub use rules::*;
pub use records::*;
use colours::Colours;
use follow::*;
use send::*;
use station_host::*;
pub use airband_core::relay::PushService;
use tell::*;
use uploads::*;
use view::*;

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
    /// For a call screen: what is happening.
    fn status(&self, status: RadioStatus);
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
    /// words, as UTF-8. For a voice it is JSON, `{"text": the words, "voice": the voice's name}`, and
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
/// One computer he is paired with, for as long as he is: what was last seen of it, and the connection to it
/// now. The computer outlives its connections: asking it afresh (`reconnect`, `wake`) replaces only the
/// connection, and what is kept for him on its behalf (the radio's [`Kept`]) passes to the next one.
pub(crate) struct Computer {
    link: String,
    /// The connection to it now: replaced on every reconnect, with what it keeps handed on.
    pub(crate) radio: Arc<Radio>,
    /// Which connection this is: a thread of an older one finds it changed and stops.
    epoch: u64,
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
    /// What its radio last said was waiting to go: only for telling what has since left. What is shown as
    /// waiting is read from what the radio keeps on this device, so it is there from launch.
    was_waiting: Vec<Outgoing>,
    /// Sent, and not yet seen in the chat that comes back from the tower: shown as on their way until they are.
    landing: Vec<(Outgoing, std::time::Instant)>,
    quick: Vec<QuickMessage>,
    /// This computer's recent chat, oldest first.
    messages: Vec<ChatMessage>,
}
/// What he is told when what he acted on is on a computer he is no longer paired with.
pub(super) const UNPAIRED: &str = "That computer is no longer paired.";
/// What he is told when the team he picked has no agents left, so no computer has it.
pub(super) const NO_TEAM: &str = "That team has no agents now.";
impl Computer {
    /// What this computer is known by on this device ([`station_id`]).
    pub(crate) fn id(&self) -> String {
        station_id(&self.link)
    }
    /// Each of its agents and messages is put in its room on his screen: this computer and its team, as one id.
    pub(crate) fn settle_rooms(&mut self) {
        let sid = station_id(&self.link);
        for a in &mut self.agents {
            a.room = room_id(&sid, &a.team);
        }
        for m in &mut self.messages {
            m.room = room_id(&sid, &m.team);
        }
    }
}
/// Words of his that no tower could place, with who the asked tower thought they might be for.
struct Asking {
    message: Outgoing,
    guesses: Vec<String>,
    /// The computer the words reached, which has the files they took with them, by its id.
    at: String,
}
#[derive(Default)]
pub(crate) struct State {
    /// What he said that is waiting to be told who it is for.
    asking: Option<Asking>,
    /// He took back what he was saying: when the microphone closes, nothing is sent.
    discard: bool,
    pub(crate) computers: Vec<Computer>,
    /// Answered here a moment ago: hidden from "waiting" before the tower confirms.
    answered: HashSet<String>,
    /// The last connection's number: each connection to a computer gets the next ([`Computer::epoch`]).
    epochs: u64,
    calling: bool,
    uploads: Vec<Upload>,
    /// Words of his waiting for the files he attached to reach another computer: sent when the last of them
    /// has got there, failed, or been taken off ([`uploads::move_files`]).
    after_files: Option<AfterFiles>,
    /// Where this device can be woken with a notification (a push service's name and the device's address
    /// there), and the links of the computers that have been told.
    push: Option<(PushService, String)>,
    /// Each team's colour, chosen here across every computer.
    pub(crate) colours: Colours,
    push_told: HashSet<String>,
    /// The coding tools each computer was last told he uses, by its link.
    tools_told: HashMap<String, Vec<String>>,
    read: ReadMarks,
}
impl State {
    /// Every computer's radio, in the computers' order.
    fn radios(&self) -> Vec<Arc<Radio>> {
        self.computers.iter().map(|s| s.radio.clone()).collect()
    }
    /// A computer by its id ([`Computer::id`]).
    pub(super) fn computer(&self, id: &str) -> Option<&Computer> {
        self.computers.iter().find(|s| s.id() == id)
    }
    /// A computer by its id, while `epoch` is still its connection: for the threads of one connection.
    fn live(&mut self, id: &str, epoch: u64) -> Option<&mut Computer> {
        self.computers.iter_mut().find(|s| s.epoch == epoch && s.id() == id)
    }
    /// The computer that has this agent, by the agent's id.
    fn with_agent(&self, id: &str) -> Option<&Computer> {
        self.computers.iter().find(|s| s.agents.iter().any(|a| a.id == id))
    }
    /// The radio of the computer that has this agent, by the agent's id.
    fn radio_with_agent(&self, id: &str) -> Option<Arc<Radio>> {
        self.with_agent(id).map(|s| s.radio.clone())
    }
    /// What a computer is called wherever one is named to him, by its id: the name it gave itself, and for
    /// one that never gave one, what its pairing link calls it.
    pub(super) fn computer_name(&self, id: &str) -> String {
        let Some(computer) = self.computer(id) else { return String::new() };
        if !computer.name.is_empty() {
            return computer.name.clone();
        }
        // one that has never said its name: what its pairing link calls it, or where the link reaches it
        match airband_core::pairing::Pairing::computer_in(&computer.link) {
            Some(name) => name,
            None if self.computers.len() == 1 => "Your computer".to_string(),
            None => "New computer".to_string(),
        }
    }
    /// Every computer's radio with its id and what it is called.
    fn named_radios(&self) -> Vec<(DeskComputer, Arc<Radio>)> {
        self.computers.iter().map(|s| (DeskComputer { id: s.id(), name: self.computer_name(&s.id()) }, s.radio.clone())).collect()
    }
    /// The colour a team is shown in.
    fn colour(&self, group: &str) -> Option<u8> {
        self.colours.of(group)
    }
    /// His main computer: the only one, or the one he chose, while it is still paired.
    pub(crate) fn main(&self) -> Option<&Computer> {
        match self.computers.as_slice() {
            [] => None,
            [only] => Some(only),
            _ => self.read.main.as_deref().and_then(|id| self.computer(id)),
        }
    }
    /// A room on his screen ([`DeskRoom::id`]): the radio of the computer that has it, the team's id there,
    /// and what the team is called. Nothing for a room no agent is in now.
    fn room(&self, room: &str) -> Option<(Arc<Radio>, String, String)> {
        self.computers.iter().find_map(|s| s.agents.iter().find(|a| a.room == room).map(|a| (s.radio.clone(), a.team.clone(), a.group.clone())))
    }
    /// The computer a message is in.
    fn owner(&self, message: &str) -> Option<&Computer> {
        self.computers.iter().find(|s| s.messages.iter().any(|m| m.id == message))
    }
    fn message(&self, id: &str) -> Option<(&Computer, &ChatMessage)> {
        self.computers.iter().find_map(|s| s.messages.iter().find(|m| m.id == id).map(|m| (s, m)))
    }
    /// Every message, oldest first, each labelled with its computer when there are several.
    fn merged(&self) -> Vec<ChatMessage> {
        let several = self.computers.len() > 1;
        let mut all: Vec<ChatMessage> = self
            .computers
            .iter()
            .flat_map(|s| {
                s.messages.iter().map(move |m| {
                    ChatMessage {
                        station: if several { s.id() } else { String::new() },
                        colour: self.colours.of(&m.room),
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
        self.computers.iter().flat_map(|s| s.agents.iter()).filter(|a| seen.insert(a.id.clone())).collect()
    }
    fn is_open(&self, id: &str) -> bool {
        !self.answered.contains(id) && self.computers.iter().any(|s| s.open.contains(id))
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
    /// What reaches a computer in place of the network, by its pairing link: a stand-in tower in tests.
    /// Nothing in the app, where every computer is reached the way its link says.
    pub(crate) dial: Option<Dial>,
}
/// A stand-in for the network to a computer, by its pairing link ([`Shared::dial`]).
pub(crate) type Dial = Arc<dyn Fn(&str) -> Arc<dyn crate::link::Transport> + Send + Sync>;
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
        Desk { shared: Arc::new(Shared { state: Mutex::new(State::default()), floor: Floor::default(), host, chooser, senses: Arc::default(), key_test: crate::keys::pretend, dial: None }), data_dir: data_dir.into() }
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
        let device = host.device();
        let state = State { read: ReadMarks::load(&data_dir), colours: Colours::open(&data_dir, device.team_colours), ..State::default() };
        let chooser = crate::choices::Chooser::open(&data_dir, device);
        let senses: Arc<Senses> = Arc::default();
        // what he chose to listen and speak with is in use from the start
        chooser.apply(&*host, &senses);
        Arc::new(Desk { shared: Arc::new(Shared { state: Mutex::new(state), floor: Floor::default(), host, chooser, senses, key_test: crate::keys::check, dial: None }), data_dir })
    }
    /// The computers he is paired with, as the links `airband pair` printed, one each. Replaces whatever set
    /// was there: a computer that stays is left as it is, connection and all; one no longer listed is let go;
    /// a new one is connected. Returns the links that were taken: one that is not a pairing link is left out.
    /// What was last seen from each is on screen at once, before any connection.
    pub fn set_links(&self, links: Vec<String>) -> Vec<String> {
        let mut wanted: Vec<String> = Vec::new();
        links.into_iter().for_each(|l| if !wanted.contains(&l) { wanted.push(l) });
        let (calling, new): (bool, Vec<(String, u64)>) = {
            let mut state = self.shared.state.lock().unwrap();
            let fresh: Vec<String> = wanted.iter().filter(|l| !state.computers.iter().any(|s| &s.link == *l)).cloned().collect();
            let numbered = fresh.into_iter().map(|l| { state.epochs += 1; (l, state.epochs) }).collect();
            (state.calling, numbered)
        };
        // each new computer is connected outside the lock: what it last showed is read from this device
        let mut made: Vec<Computer> = Vec::new();
        for (link, epoch) in new {
            let Ok(radio) = self.connect(&link, epoch, None) else { continue };
            radio.set_call_mode(calling);
            // what it last said it is called, kept on the phone: known before it answers again
            let name = radio.store.saved.lock().unwrap().computer.clone();
            let was_waiting = radio.unsent();
            let mut computer = Computer {
                messages: radio.history(),
                radio,
                epoch,
                name,
                online: false,
                tried: false,
                answered_at: None,
                fails: 0,
                failing_since: None,
                reads: String::new(),
                reads_failed: String::new(),
                agents: Vec::new(),
                open: HashSet::new(),
                was_waiting,
                landing: Vec::new(),
                quick: Vec::new(),
                link,
            };
            // the lines kept on the phone are in their rooms before any computer answers
            computer.settle_rooms();
            made.push(computer);
        }
        let started: Vec<(String, u64, Arc<Radio>)> = made.iter().map(|s| (s.id(), s.epoch, s.radio.clone())).collect();
        let (taken, gone) = {
            let mut state = self.shared.state.lock().unwrap();
            let mut was = std::mem::take(&mut state.computers);
            let mut kept: Vec<Computer> = Vec::new();
            for link in &wanted {
                if let Some(at) = was.iter().position(|s| &s.link == link) {
                    kept.push(was.remove(at));
                } else if let Some(at) = made.iter().position(|s| &s.link == link) {
                    kept.push(made.remove(at));
                }
            }
            let paired: HashSet<String> = kept.iter().map(|s| s.id()).collect();
            // files on a computer he no longer has, and words that reached it, go nowhere else
            state.uploads.retain(|u| {
                let keep = paired.contains(&u.station);
                if !keep {
                    u.dropped.store(true, Ordering::SeqCst);
                }
                keep
            });
            if state.asking.as_ref().is_some_and(|a| !paired.contains(&a.at)) {
                state.asking = None;
            }
            let taken: Vec<String> = kept.iter().map(|s| s.link.clone()).collect();
            state.computers = kept;
            (taken, was)
        };
        gone.iter().for_each(|s| s.radio.stop());
        for (id, epoch, radio) in started {
            let shared = self.shared.clone();
            thread::spawn(move || follow(shared, radio, id, epoch));
        }
        self.shared.host.changed();
        taken
    }
    /// The app is in front of him again. A request that was open when it went to the background is dead and
    /// would not be noticed for most of a minute, so a computer that has not answered lately is asked afresh,
    /// now. Cheap to call whenever the app comes forward.
    pub fn wake(&self) {
        let stale: Vec<String> = {
            let state = self.shared.state.lock().unwrap();
            // a request waits up to 25 seconds for news, so an answer older than that plus a margin is a dead line
            state.computers.iter().filter(|s| s.answered_at.is_none_or(|t| t.elapsed() > Duration::from_secs(32))).map(|s| s.id()).collect()
        };
        self.reconnect_these(stale);
    }
    /// He asked for it: ask every computer afresh, now, whatever was last heard from them.
    pub fn reconnect(&self) {
        let all: Vec<String> = self.shared.state.lock().unwrap().computers.iter().map(|s| s.id()).collect();
        self.reconnect_these(all);
    }
}

impl Desk {
    /// A new connection to the computer this link pairs, numbered `epoch`, handed what the last one kept for
    /// him (`kept`), or opening it from this device when there was none.
    fn connect(&self, link: &str, epoch: u64, kept: Option<Kept>) -> Result<Arc<Radio>, RadioError> {
        let dir = if self.data_dir.is_empty() { String::new() } else { std::path::Path::new(&self.data_dir).join(folder(link)).display().to_string() };
        let host: Arc<dyn RadioHost> = Arc::new(ComputerHost { shared: self.shared.clone(), station: station_id(link), epoch });
        let kept = kept.unwrap_or_else(|| Kept::open(&dir, host.clone()));
        let transport = self.shared.dial.as_ref().map(|dial| dial(link));
        Radio::start_with(link.to_string(), &dir, host, self.shared.senses.clone(), kept, transport)
    }
    /// A new connection for each of these computers, by id. Everything else about each computer stays: what was
    /// last seen of it, and what is kept for him on its behalf.
    fn reconnect_these(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let (calling, plan): (bool, Vec<(String, String, u64, Kept)>) = {
            let mut state = self.shared.state.lock().unwrap();
            let plan = ids.iter().filter_map(|id| state.computer(id).map(|s| (id.clone(), s.link.clone(), s.radio.kept()))).collect::<Vec<_>>();
            let plan = plan.into_iter().map(|(id, link, kept)| { state.epochs += 1; (id, link, state.epochs, kept) }).collect();
            (state.calling, plan)
        };
        self.shared.floor.clear();
        for (id, link, epoch, kept) in plan {
            let Ok(radio) = self.connect(&link, epoch, Some(kept)) else { continue };
            radio.set_call_mode(calling);
            let old = {
                let mut state = self.shared.state.lock().unwrap();
                let Some(computer) = state.computers.iter_mut().find(|s| s.id() == id) else {
                    radio.stop();
                    continue;
                };
                computer.epoch = epoch;
                std::mem::replace(&mut computer.radio, radio.clone())
            };
            old.stop();
            let shared = self.shared.clone();
            thread::spawn(move || follow(shared, radio, id, epoch));
        }
        self.shared.host.changed();
    }
    /// Stop for good.
    #[cfg(test)]
    pub(crate) fn stop(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.epochs += 1;
        state.computers.drain(..).for_each(|s| s.radio.stop());
    }
    /// The computer called this. With no name it is the one computer he has; with several he has to say
    /// which, and none is picked for him.
    fn station_radio(&self, computer: &str) -> Result<Arc<Radio>, RadioError> {
        self.station_of(computer).map(|(radio, _)| radio)
    }
    /// The same, with what the computer is known by in the rooms of his screen.
    fn station_of(&self, computer: &str) -> Result<(Arc<Radio>, String), RadioError> {
        let state = self.shared.state.lock().unwrap();
        let found = match (computer.is_empty(), state.computers.as_slice()) {
            (true, [only]) => Some(only),
            (true, [_, _, ..]) => return Err(RadioError::Told { reason: "Choose which computer first.".into() }),
            _ => state.computers.iter().find(|s| s.id() == computer),
        };
        found.filter(|s| s.online).map(|s| (s.radio.clone(), station_id(&s.link))).ok_or(RadioError::Offline)
    }
    fn radio_of(&self, message: &str) -> Option<Arc<Radio>> {
        self.shared.state.lock().unwrap().owner(message).map(|s| s.radio.clone())
    }
    /// The radio of the computer that has this agent, or a refusal when none does.
    fn agent_radio(&self, agent: &str) -> Result<Arc<Radio>, RadioError> {
        self.shared.state.lock().unwrap().radio_with_agent(agent).ok_or(RadioError::Refused)
    }
    /// The computer's radio that a message still waiting to go, or given up on, belongs to: what each keeps
    /// on this device says, so it is known from launch.
    fn sender_of(&self, id: &str) -> Option<Arc<Radio>> {
        self.shared.state.lock().unwrap().computers.iter().find(|s| s.radio.unsent().iter().any(|o| o.id == id)).map(|s| s.radio.clone())
    }
}
