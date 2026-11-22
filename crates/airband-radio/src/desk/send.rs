//! His words on their way out: which computer they go to, and everything a host can do with a message of his.
use super::*;

#[uniffi::export]
impl Desk {
    /// He said who his unplaced words are for: they go to that agent, on whichever computer has it.
    pub fn choose(&self, agent: String) {
        let sent = {
            let mut state = self.shared.state.lock().unwrap();
            let to = state.stations.iter().find_map(|s| s.agents.iter().find(|a| a.id == agent).map(|_| s.radio.clone()));
            match (to, state.asking.take()) {
                (Some(radio), Some(asking)) => Some((radio, asking.message.text, asking.message.label)),
                (_, asking) => {
                    state.asking = asking;
                    None
                }
            }
        };
        if let Some((radio, text, label)) = sent {
            radio.queue_as(text, None, None, label, String::new(), vec![agent], None);
        }
        self.shared.host.changed();
    }
    /// He let his unplaced words go: they are sent to nobody.
    pub fn drop_ask(&self) {
        self.shared.state.lock().unwrap().asking = None;
        self.shared.host.changed();
    }
    /// He tapped a button under a question: the answer goes as what the button stood for, never as words to
    /// be read. Nothing is sent for an option the message does not have.
    pub fn tap(&self, message_id: String, tapped: Tapped) {
        let sent = {
            let mut state = self.shared.state.lock().unwrap();
            let text = match &tapped {
                Tapped::Option { index } => state.stations.iter().find_map(|s| s.messages.iter().find(|m| m.id == message_id)).and_then(|m| m.options.get(*index as usize).cloned()),
                Tapped::Allow | Tapped::Deny => Some(tapped.words()),
            };
            let radio = state.owner(&message_id).or((!state.stations.is_empty()).then_some(0)).map(|i| state.stations[i].radio.clone());
            text.zip(radio).inspect(|_| {
                state.answered.insert(message_id.clone());
            })
        };
        if let Some((text, radio)) = sent {
            radio.queue_as(text, Some(message_id), None, None, String::new(), Vec::new(), Some(tapped.act()));
        }
        self.shared.host.changed();
    }
    /// Words of his own sent as the answer to that message.
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
    /// computer has it. `tapped` is the button, when it was one: `text` is then the button's words for an
    /// option, and is not used for Allow and Deny. `station` is the computer the notification came from ([`OpenedNotice::station`]).
    /// Blocks for some seconds at most. If it could not be sent in that time it is kept and sent when it can
    /// be, and the error says why, in words for him when there are any.
    pub fn answer_now(&self, station: String, message_id: String, text: String, tapped: Option<Tapped>) -> Result<(), RadioError> {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            state.answered.insert(message_id.clone());
            let named = state.stations.iter().position(|s| station_key(&s.link).as_deref() == Some(station.as_str()));
            named.or(state.owner(&message_id)).map(|i| state.stations[i].radio.clone())
        };
        self.shared.host.changed();
        radio.ok_or(RadioError::Told { reason: "That computer is no longer paired.".into() })?.answer_now(match &tapped { Some(t @ (Tapped::Allow | Tapped::Deny)) => t.words(), _ => text }, message_id, tapped.map(|t| t.act()))
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
            state.stations.iter().filter(|s| s.agents.iter().any(|a| a.state == AgentState::Stalled && room.as_ref().is_none_or(|r| a.group == *r))).map(|s| s.radio.clone()).collect()
        };
        for radio in radios {
            // what the chat shows for it is what the stopped agents are told
            radio.queue_as("Keep going.".into(), None, room.clone(), None, String::new(), Vec::new(), Some(Act::Control { control: airband_core::routing::Control::Poke }));
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
            let pick = state.uploads.first().map(|u| u.station).or_else(|| pick_station(&state, "", re.as_deref(), room.as_deref(), agent.as_deref()));
            let Some(pick) = pick.filter(|i| state.stations[*i].online).or_else(|| state.stations.iter().position(|s| s.online)).or(pick) else { return };
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
    /// Take back something he said that is still waiting its turn. Blocks: call it off the main thread.
    pub fn unsend(&self, message_id: String) -> Result<(), RadioError> {
        self.radio_of(&message_id).ok_or(RadioError::Refused)?.unsend(message_id)
    }
}

impl Desk {
    pub(super) fn deliver(&self, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>) {
        deliver(&self.shared, text, label, re, room, agent);
    }
}

/// Send something of his: typed, tapped or said. Who it is for, and so which computer it goes to, is worked out
/// from what he can see; the tower it goes to is told about the agents the other computers have, so it can say
/// the words are for one of those.
pub(super) fn deliver(shared: &Arc<Shared>, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>) {
    let text = text.trim().to_string();
    let (radio, text, re, scope, others, to) = {
        let mut state = shared.state.lock().unwrap();
        // nothing typed is still something to send when files are attached: the files themselves
        if state.stations.is_empty() || (text.is_empty() && state.uploads.is_empty()) {
            return;
        }
        // something new said: whatever was waiting to be placed is let go
        state.asking = None;
        // tags are the one thing he types that addresses: `@shop2`, `@all`
        let tagged = text.starts_with('@');
        let re = re.filter(|_| !tagged);
        if let Some(id) = &re {
            state.answered.insert(id.clone());
        }
        // looking at one agent, with nobody tagged and no message picked: it is for that agent
        let aimed = matches!((&agent, tagged, &re), (Some(_), false, None));
        // with several computers and no main one chosen the app asks him first, and nothing is sent until then
        let Some(wanted) = pick_station(&state, &text, re.as_deref(), room.as_deref(), agent.as_deref()) else { return };
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
                    u.shown.state = UploadState::Sending;
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
                        if !again.state.lock().unwrap().uploads.iter().any(|u| u.shown.state == UploadState::Sending) {
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
        state.uploads.retain(|u| !(u.station == pick && u.shown.state == UploadState::Ready));
        // the agent he is looking at, by id: the tower reads nothing to find who it is for
        let to: Vec<String> = match (&agent, aimed) {
            (Some(callsign), true) => state.stations[pick].agents.iter().find(|a| a.callsign == *callsign).map(|a| a.id.clone()).into_iter().collect(),
            _ => Vec::new(),
        };
        let scope = room.clone().filter(|_| re.is_none() && agent.is_none() && !tagged);
        // when the tower has to work out who it is for, it is told who else is on his screen
        let others = if tagged || aimed || re.is_some() || state.stations.len() < 2 { String::new() } else { others(&state, pick, room.as_deref()) };
        (state.stations[pick].radio.clone(), text, re, scope, others, to)
    };
    radio.queue_as(text, re, scope, label, others, to, None);
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
            let last = s.messages.iter().rev().find(|m| m.agent.as_deref() == Some(a.id.as_str()) && !m.fyi);
            airband_core::radio::Other { callsign: a.callsign.clone(), group: a.group.clone(), said: last.map(|m| m.text.chars().take(240).collect()).unwrap_or_default(), ago: last.map(|m| now.saturating_sub(m.at) / 1000).unwrap_or(0) }
        })
        .collect();
    if list.is_empty() { String::new() } else { serde_json::to_string(&list).unwrap_or_default() }
}
/// Which computer a message of his goes to.
pub(super) fn pick_station(state: &State, text: &str, re: Option<&str>, room: Option<&str>, agent: Option<&str>) -> Option<usize> {
    // the computer that has the message he is answering
    if let Some(i) = re.and_then(|id| state.owner(id)) {
        return Some(i);
    }
    // the computer that has whoever he tagged: read the tags the way the tower will
    if text.starts_with('@') {
        let named = state.stations.iter().position(|s| {
            let live: Vec<Callsign> = s.agents.iter().map(|a| Callsign { project: a.group.clone(), number: a.number.clone(), named: a.named }).collect();
            !parse_utterance(text, &live).to.is_empty()
        });
        if let Some(i) = named {
            return Some(i);
        }
    }
    if let Some(i) = agent.and_then(|c| state.stations.iter().position(|s| s.agents.iter().any(|a| a.callsign == c))) {
        return Some(i);
    }
    // the room he is looking at, when only one computer has agents in it
    if let Some(r) = room {
        let mut with = state.stations.iter().enumerate().filter(|(_, s)| s.agents.iter().any(|a| a.group == r)).map(|(i, _)| i);
        if let (Some(only), None) = (with.next(), with.next()) {
            return Some(only);
        }
    }
    // Nothing says: his main computer, which works out who it is for and passes it on. With several computers
    // and none chosen there is no answer, and he is asked to choose (`DeskView::choose_main`).
    state.main()
}
