//! The desk: every computer he is paired with, as one. A host that shows a chat talks to this and to nothing
//! else. It keeps one [`Radio`] per computer and owns everything that is the same on any platform: merging
//! their messages, what counts as waiting on him and what as done, which computer a message of his goes to,
//! which computer has the floor on a call, and marking what he has read. What is left for a host is to draw
//! the [`DeskView`], make and capture sound, and keep the pairing links somewhere safe.
use super::*;
use airband_core::routing::{Callsign, parse_utterance};
use std::collections::HashSet;
use std::sync::Condvar;

/// What a host provides to a desk. Sound calls arrive on a radio's own thread, one computer at a time, and may
/// block; `changed`, `status` and `note` arrive on any thread and must return quickly.
#[uniffi::export(with_foreign)]
pub trait DeskHost: Send + Sync {
    /// Say this aloud with the device's own voice; return when it has finished.
    fn speak(&self, text: String);
    /// Open the microphone and return what he said, by the device's own recognition, or nothing.
    fn listen(&self) -> Option<String>;
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
    /// For a call screen: what is happening, and the words being said by the device's own voice ("" if none).
    fn status(&self, status: RadioStatus, saying: String);
    /// Something he must be told: a message could not be placed, or expired unsent.
    fn note(&self, text: String);
    /// Is the chat on screen in front of him right now? What arrives while it is counts as read, so a call
    /// started later does not read it out.
    fn watching(&self) -> bool;
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
}

/// Everything a chat screen shows, for the team or the agent he is looking at (or all of them).
#[derive(Debug, Clone, uniffi::Record)]
pub struct DeskView {
    /// At least one computer is paired.
    pub paired: bool,
    /// At least one of them can be reached.
    pub online: bool,
    /// The feed: reports, questions and what he said, oldest first, without the chatter not worth his attention.
    /// A message with `open` is a question still waiting on him; one with `stalled` is from an agent that stopped.
    pub feed: Vec<ChatMessage>,
    /// One per project with an agent on the radio, by name. Not narrowed by what he is looking at.
    pub rooms: Vec<DeskRoom>,
    /// Someone here has stopped without finishing: a poke would do something.
    pub can_poke: bool,
    /// The quick messages to offer.
    pub quick: Vec<QuickMessage>,
    /// His messages still on their way.
    pub sending: Vec<Outgoing>,
    /// What each paired computer is called.
    pub stations: Vec<String>,
}

/// How many lines of the feed a view carries.
const FEED_SHOWN: usize = 80;

struct Station {
    link: String,
    radio: Arc<Radio>,
    name: String,
    online: bool,
    agents: Vec<ChatAgent>,
    open: HashSet<String>,
    sending: Vec<Outgoing>,
    quick: Vec<QuickMessage>,
    /// Each team's colour, as this computer gave it.
    colours: std::collections::HashMap<String, u8>,
    /// This computer's recent chat, oldest first.
    messages: Vec<ChatMessage>,
}

#[derive(Default)]
struct State {
    stations: Vec<Station>,
    /// Answered here a moment ago: hidden from "waiting" before the tower confirms.
    answered: HashSet<String>,
    /// Goes up each time the set of computers changes, so threads following the old set stop.
    generation: u64,
    calling: bool,
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
                        badge: if his { String::new() } else { airband_core::routing::badge(&m.group, &m.from) },
                        colour: s.colours.get(&m.group).copied().unwrap_or(0),
                        ..m.clone()
                    }
                })
            })
            .collect();
        all.sort_by_key(|m| m.at);
        all
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

struct Shared {
    state: Mutex<State>,
    floor: Floor,
    host: Arc<dyn DeskHost>,
}

/// What one computer's radio calls back into: it knows which computer it is, and the desk does the rest.
struct StationHost {
    shared: Arc<Shared>,
    index: usize,
    generation: u64,
}

impl RadioHost for StationHost {
    fn speak(&self, text: String) {
        self.shared.floor.take(self.index);
        self.shared.host.status(RadioStatus::Speaking { callsign: String::new() }, text.clone());
        self.shared.host.speak(text);
    }
    fn listen(&self) -> Option<String> {
        self.shared.floor.take(self.index);
        self.shared.host.listen()
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
        if self.shared.floor.release(self.index, ends) {
            self.shared.host.status(status, String::new());
        }
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.generation != self.generation {
                return;
            }
            if let Some(s) = state.stations.get_mut(self.index) {
                s.sending = waiting;
            }
        }
        self.shared.host.changed();
        if let Some(note) = note {
            self.shared.host.note(note);
        }
    }
}

#[derive(uniffi::Object)]
pub struct Desk {
    shared: Arc<Shared>,
    data_dir: String,
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
        Arc::new(Desk { shared: Arc::new(Shared { state: Mutex::new(State::default()), floor: Floor::default(), host }), data_dir })
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
            let Ok(radio) = Radio::start(link.clone(), dir, host) else { continue };
            radio.set_call_mode(calling);
            made.push(Station { link, messages: radio.history(), radio, name: String::new(), online: false, agents: Vec::new(), open: HashSet::new(), sending: Vec::new(), quick: Vec::new(), colours: Default::default() });
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

    /// What to show. `room` narrows the feed to one team, `agent` to one callsign; with neither, everything.
    pub fn view(&self, room: Option<String>, agent: Option<String>) -> DeskView {
        let state = self.shared.state.lock().unwrap();
        let agents: Vec<&ChatAgent> = state.stations.iter().flat_map(|s| s.agents.iter()).collect();
        // his own messages belong to the room or agent they went to
        let here = |m: &ChatMessage| {
            let about = |name: &String| agent.as_ref().is_none_or(|a| name == a) && room.as_ref().is_none_or(|r| agents.iter().any(|x| x.callsign == *name && x.group == *r));
            if m.from == "you" { (room.is_none() && agent.is_none()) || m.to.iter().any(about) } else { room.as_ref().is_none_or(|r| m.group == *r) && agent.as_ref().is_none_or(|a| m.from == *a) }
        };
        let mut feed: Vec<ChatMessage> = state.merged().into_iter().filter(|m| !m.fyi && here(m)).map(|m| ChatMessage { open: state.is_open(&m.id), ..m }).collect();
        let extra = feed.len().saturating_sub(FEED_SHOWN);
        feed.drain(..extra);
        // each team, in the state of whichever of its agents most needs him
        let rank = |s: &str| ["idle", "working", "stalled", "asking"].iter().position(|x| *x == s).unwrap_or(0);
        let mut rooms: Vec<DeskRoom> = Vec::new();
        for a in &agents {
            let member = DeskMember { id: a.id.clone(), callsign: a.callsign.clone(), state: a.state.clone(), subagents: a.subagents, doing: a.doing.clone(), badge: airband_core::routing::badge(&a.group, &a.callsign) };
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
                None => rooms.push(DeskRoom { name: a.group.clone(), state: if a.state.is_empty() { "working".into() } else { a.state.clone() }, agents: 1, subagents: a.subagents, doing, members: vec![member], colour: state.stations.iter().find_map(|s| s.colours.get(&a.group).copied()).unwrap_or(0) }),
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
            can_poke: agents.iter().any(|a| a.state == "stalled" && room.as_ref().is_none_or(|r| a.group == *r)),
            feed,
            rooms,
            quick,
            sending: state.stations.iter().flat_map(|s| s.sending.clone()).collect(),
            stations: state.stations.iter().map(|s| s.name.clone()).collect(),
        }
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

    /// Send what he typed. Who it is for, in this order: tags at its start (`@shop2`, `@shop`, `@all`); the
    /// message `re` if he picked one; the `agent` he is looking at; otherwise the tower works it out from his
    /// words, among the agents of the `room` he is in or among all of them. It goes to the computer that has
    /// whoever that is.
    pub fn send(&self, text: String, re: Option<String>, room: Option<String>, agent: Option<String>) {
        self.deliver(text, None, re, room, agent);
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
    pub fn talk(&self, re: Option<String>, room: Option<String>) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stations.is_empty() {
                return;
            }
            if let Some(id) = &re {
                state.answered.insert(id.clone());
            }
            let pick = pick_station(&state, "", re.as_deref(), room.as_deref(), None);
            state.stations[pick].radio.clone()
        };
        let scope = room.filter(|_| re.is_none()).map(|r| format!("group:{r}"));
        radio.talk(re, scope);
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
        radios.iter().for_each(|r| r.set_call_mode(on));
    }

    /// Exactly what the agent wrote. Blocks: call it off the main thread.
    pub fn full(&self, message_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.more(message_id, true)
    }

    /// One of a message's pictures, as a file on the device. Blocks: call it off the main thread.
    pub fn picture(&self, message_id: String, picture_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.picture(picture_id)
    }

    /// Every paired computer's accounts, in the order the computers were paired. A computer that cannot be
    /// reached, or whose airband is too old, comes back with a note and no accounts. Blocks: call it off the
    /// main thread.
    pub fn accounts(&self, fresh: bool) -> Vec<StationAccounts> {
        let radios: Vec<(String, Arc<Radio>)> = self.shared.state.lock().unwrap().stations.iter().map(|s| (s.name.clone(), s.radio.clone())).collect();
        radios
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
            .collect()
    }

    /// Move one computer's sessions to another of its accounts. `station`, `harness` and `id` are as
    /// [`Desk::accounts`] gave them. Blocks: call it off the main thread.
    pub fn use_account(&self, station: String, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.name == station).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.use_account(station, harness, id)
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
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let (radio, text, re, scope) = {
            let mut state = self.shared.state.lock().unwrap();
            if state.stations.is_empty() {
                return;
            }
            let tagged = text.starts_with('@') || text.starts_with('/');
            let re = re.filter(|_| !tagged);
            if let Some(id) = &re {
                state.answered.insert(id.clone());
            }
            let pick = pick_station(&state, &text, re.as_deref(), room.as_deref(), agent.as_deref());
            // looking at one agent, with nobody tagged and no message picked: it is for that agent
            let text = match (&agent, tagged, &re) {
                (Some(callsign), false, None) => format!("@{} {text}", callsign.replace(' ', "")),
                _ => text,
            };
            let scope = room.filter(|_| re.is_none() && agent.is_none() && !tagged).map(|r| format!("group:{r}"));
            (state.stations[pick].radio.clone(), text, re, scope)
        };
        radio.queue(text, re, scope, label);
        self.shared.host.changed();
    }

    fn radio_of(&self, message: &str) -> Option<Arc<Radio>> {
        let state = self.shared.state.lock().unwrap();
        state.owner(message).map(|i| state.stations[i].radio.clone())
    }
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
    if let Some(i) = room.and_then(|r| state.stations.iter().position(|s| s.agents.iter().any(|a| a.group == r))) {
        return i;
    }
    // nothing says: the computer that last had something to say
    let last = state.stations.iter().enumerate().filter_map(|(i, s)| s.messages.iter().rev().find(|m| m.from != "you").map(|m| (m.at, i))).max();
    last.map(|(_, i)| i).unwrap_or(0)
}

/// Keeps one computer's part of the chat current. One request is always open with its tower.
fn follow(shared: Arc<Shared>, radio: Arc<Radio>, index: usize, generation: u64) {
    let mut first = true;
    loop {
        // the first answer must come at once: it is what says the tower is there and who is on it
        let got = radio.follow(if first { 0 } else { 25 });
        let mut seen: Vec<String> = Vec::new();
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
                    station.online = true;
                    station.name = feed.station.clone();
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
                Err(_) => station.online = false,
            }
            // once the tower agrees a message is answered, stop tracking it here
            let now_open: HashSet<String> = state.stations.iter().flat_map(|s| s.open.iter().cloned()).collect();
            if now_open != open {
                state.answered.retain(|id| now_open.contains(id));
            }
        }
        shared.host.changed();
        if !seen.is_empty() {
            radio.seen(seen);
        }
        if got.is_err() {
            thread::sleep(Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;
    impl RadioHost for Quiet {
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
    }

    fn msg(id: &str, seq: u64, from: &str, group: &str, needs: bool) -> ChatMessage {
        ChatMessage { seq, id: id.into(), at: seq, from: from.into(), group: group.into(), to: Vec::new(), needs_you: needs, fyi: false, stalled: false, text: id.into(), long: false, options: Vec::new(), recommended: None, permission: false, pictures: Vec::new(), links: Vec::new(), agent: Some(from.into()), station: String::new(), open: needs, badge: String::new(), colour: 0, read: None }
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
            agents: agents.iter().map(|(c, g)| ChatAgent { id: c.to_string(), callsign: c.to_string(), group: g.to_string(), harness: "h".into(), waiting: false, reachable: true, state: "working".into(), subagents: 0, doing: None }).collect(),
            open: messages.iter().filter(|m| m.needs_you).map(|m| m.id.clone()).collect(),
            sending: Vec::new(),
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
