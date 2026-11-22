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
    /// This computer's recent chat, oldest first.
    messages: Vec<ChatMessage>,
}
impl Station {
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
    pub(crate) stations: Vec<Station>,
    /// Answered here a moment ago: hidden from "waiting" before the tower confirms.
    answered: HashSet<String>,
    /// Goes up each time the set of computers changes, so threads following the old set stop.
    generation: u64,
    calling: bool,
    uploads: Vec<Upload>,
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
        self.stations.iter().map(|s| s.radio.clone()).collect()
    }
    /// The computer that has this agent, by the agent's id: its place among them.
    /// Where a computer is among them now, by its id ([`Station::id`]).
    pub(super) fn station_at(&self, id: &str) -> Option<usize> {
        self.stations.iter().position(|s| s.id() == id)
    }
    fn station_with_agent(&self, id: &str) -> Option<usize> {
        self.stations.iter().position(|s| s.agents.iter().any(|a| a.id == id))
    }
    /// The radio of the computer that has this agent, by the agent's id.
    fn radio_with_agent(&self, id: &str) -> Option<Arc<Radio>> {
        self.station_with_agent(id).map(|i| self.stations[i].radio.clone())
    }
    /// What a computer is called wherever one is named to him: the name it gave itself, and for one that
    /// gave none, "Your computer" when it is the only one and its place among them when there are several.
    pub(super) fn computer_name(&self, index: usize) -> String {
        match self.stations.get(index).map(|s| s.name.as_str()).filter(|n| !n.is_empty()) {
            Some(name) => name.to_string(),
            None if self.stations.len() == 1 => "Your computer".to_string(),
            None => format!("Computer {}", index + 1),
        }
    }
    /// Every computer's radio with its id and what it is called.
    fn named_radios(&self) -> Vec<(DeskComputer, Arc<Radio>)> {
        (0..self.stations.len()).map(|i| (DeskComputer { id: self.stations[i].id(), name: self.computer_name(i) }, self.stations[i].radio.clone())).collect()
    }
    /// The colour a team is shown in.
    fn colour(&self, group: &str) -> Option<u8> {
        self.colours.of(group)
    }
    /// His main computer: the only one, or the one he chose, while it is still paired.
    pub(crate) fn main(&self) -> Option<usize> {
        match self.stations.len() {
            0 => None,
            1 => Some(0),
            _ => self.read.main.as_deref().and_then(|id| self.station_at(id)),
        }
    }
    /// A room on his screen ([`DeskRoom::id`]): the radio of the computer that has it, the team's id there,
    /// and what the team is called. Nothing for a room no agent is in now.
    fn room(&self, room: &str) -> Option<(Arc<Radio>, String, String)> {
        self.stations.iter().find_map(|s| s.agents.iter().find(|a| a.room == room).map(|a| (s.radio.clone(), a.team.clone(), a.group.clone())))
    }
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
        self.stations.iter().flat_map(|s| s.agents.iter()).filter(|a| seen.insert(a.id.clone())).collect()
    }
    fn is_open(&self, id: &str) -> bool {
        !self.answered.contains(id) && self.stations.iter().any(|s| s.open.contains(id))
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
        let state = State { read: ReadMarks::load(&data_dir), colours: Colours::open(&data_dir), ..State::default() };
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
            let host: Arc<dyn RadioHost> = Arc::new(StationHost { shared: self.shared.clone(), index: made.len(), generation });
            // a computer that was already there keeps what was last seen of it, so the screen does not empty while
            // it is asked again, and what he was about to send there
            let was = old.iter().find(|s| s.link == link);
            let kept = was.map(|s| s.radio.kept()).unwrap_or_else(|| Kept::open(&dir, host.clone()));
            let Ok(radio) = Radio::start_with(link.clone(), &dir, host, self.shared.senses.clone(), kept) else { continue };
            radio.set_call_mode(calling);
            let sending = radio.unsent();
            let mut station = Station {
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
                // what is still waiting to go, or was given up on, is shown from the start: nothing else says so
                // until it is next tried
                sending,
                landing: Vec::new(),
                quick: was.map(|s| s.quick.clone()).unwrap_or_default(),
                link,
            };
            // the lines kept on the phone are in their rooms before any computer answers
            station.settle_rooms();
            made.push(station);
        }
        let taken: Vec<String> = made.iter().map(|s| s.link.clone()).collect();
        let radios: Vec<Arc<Radio>> = made.iter().map(|s| s.radio.clone()).collect();
        {
            let mut state = self.shared.state.lock().unwrap();
            let paired: HashSet<String> = made.iter().map(|s| s.id()).collect();
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
            state.stations = made;
        }
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
    /// Stop for good.
    pub fn stop(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.generation += 1;
        state.stations.drain(..).for_each(|s| s.radio.stop());
    }
}

impl Desk {
    /// The computer called this. With no name it is the one computer he has; with several he has to say
    /// which, and none is picked for him.
    fn station_radio(&self, computer: &str) -> Result<Arc<Radio>, RadioError> {
        self.station_of(computer).map(|(radio, _)| radio)
    }
    /// The same, with what the computer is known by in the rooms of his screen.
    fn station_of(&self, computer: &str) -> Result<(Arc<Radio>, String), RadioError> {
        let state = self.shared.state.lock().unwrap();
        let found = match (computer.is_empty(), state.stations.as_slice()) {
            (true, [only]) => Some(only),
            (true, [_, _, ..]) => return Err(RadioError::Told { reason: "Choose which computer first.".into() }),
            _ => state.stations.iter().find(|s| s.id() == computer),
        };
        found.filter(|s| s.online).map(|s| (s.radio.clone(), station_id(&s.link))).ok_or(RadioError::Offline)
    }
    fn radio_of(&self, message: &str) -> Option<Arc<Radio>> {
        let state = self.shared.state.lock().unwrap();
        state.owner(message).map(|i| state.stations[i].radio.clone())
    }
    /// The radio of the computer that has this agent, or a refusal when none does.
    fn agent_radio(&self, agent: &str) -> Result<Arc<Radio>, RadioError> {
        self.shared.state.lock().unwrap().radio_with_agent(agent).ok_or(RadioError::Refused)
    }
    /// The computer's radio that a message still waiting to go, or given up on, belongs to.
    fn sender_of(&self, id: &str) -> Option<Arc<Radio>> {
        self.shared.state.lock().unwrap().stations.iter().find(|s| s.sending.iter().any(|o| o.id == id)).map(|s| s.radio.clone())
    }
}
