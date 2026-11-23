//! His words on their way out: which computer they go to, and everything a host can do with a message of his.
use super::*;

#[uniffi::export]
impl Desk {
    /// He said who his unplaced words are for: they go to that agent, on whichever computer has it.
    pub fn choose(&self, agent: String) {
        let asking = {
            let mut state = self.shared.state.lock().unwrap();
            if state.with_agent(&agent).is_some() { state.asking.take() } else { None }
        };
        if let Some(asking) = asking {
            place(&self.shared, &asking.at, asking.message, agent);
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
        let Some(radio) = self.owner_or_say(&message_id) else { return };
        let sent = {
            let mut state = self.shared.state.lock().unwrap();
            let text = match &tapped {
                Tapped::Option { index } => state.message(&message_id).and_then(|(_, m)| m.options.get(*index as usize).cloned()),
                Tapped::Allow | Tapped::Deny => Some(tapped.words()),
            };
            text.map(|text| {
                state.answered.insert(message_id.clone());
                (text, radio)
            })
        };
        if let Some((text, radio)) = sent {
            radio.queue_as(Outgoing { re: Some(message_id), ..Outgoing::of(text) }.with_act(Some(&tapped.act())), Carry::Nothing);
        }
        self.shared.host.changed();
    }
    /// Words of his own sent as the answer to that message.
    pub fn answer(&self, message_id: String, text: String) {
        let Some(radio) = self.owner_or_say(&message_id) else { return };
        {
            let mut state = self.shared.state.lock().unwrap();
            state.answered.insert(message_id.clone());
            // the files that have arrived on that computer leave with his answer: they leave the tray too
            if let Some(there) = state.owner(&message_id).map(|s| s.id()) {
                state.files_went(&there, &[]);
            }
        }
        radio.queue(text, Some(message_id), None, None);
        self.shared.host.changed();
    }
    /// An answer given from a notification: send it now, as the answer to that message, and wait until the
    /// computer has it. `tapped` is the button, when it was one: `text` is then the button's words for an
    /// option, and is not used for Allow and Deny. `station` is the computer the notification came from
    /// ([`OpenedNotice::station`]): the answer goes there and nowhere else.
    /// Blocks for some seconds at most. What became of it says whether it went, is kept to go later, will never
    /// go, or had no computer to go to, with the words he is told.
    pub fn answer_now(&self, station: String, message_id: String, text: String, tapped: Option<Tapped>) -> AnswerOutcome {
        let radio = self.shared.state.lock().unwrap().computer(&StationId::new(station.as_str())).map(|s| s.radio.clone());
        let Some(radio) = radio else { return AnswerOutcome::NotPaired { why: UNPAIRED.into() } };
        let words = match &tapped { Some(t @ (Tapped::Allow | Tapped::Deny)) => t.words(), _ => text };
        let outcome = radio.answer_now(words, message_id.clone(), tapped.map(|t| t.act()));
        // the question stops waiting on him only when his answer is given, now or later
        if outcome.given() {
            self.shared.state.lock().unwrap().answered.insert(message_id);
        }
        self.shared.host.changed();
        outcome
    }
    /// Send what he typed. Who it is for, in this order: tags at its start (`@shop2`, `@shop`, `@all`); the
    /// message `re` if he picked one; the `agent` he is looking at; otherwise the tower works it out from his
    /// words, among the agents of the `room` he is in or among all of them. It goes to the computer that has
    /// whoever that is.
    pub fn send(&self, text: String, re: Option<String>, room: Option<String>, agent: Option<String>) {
        self.deliver(text, None, re, room, agent);
    }
    /// Send the words of one of his messages again, to another agent: the whole of them, fetched from the
    /// computer that has the message if the chat holds only their opening. Nothing goes when the whole of them
    /// cannot be fetched: an opening is never sent as the whole. Blocks while it fetches: call it off the main
    /// thread.
    pub fn resend(&self, message_id: String, agent_id: String) -> Result<(), RadioError> {
        let (found, radio) = {
            let state = self.shared.state.lock().unwrap();
            (state.message(&message_id).map(|(_, m)| (m.text.clone(), m.long)), state.radio_with_agent(&agent_id))
        };
        let (text, long) = found.ok_or_else(|| RadioError::gone("message"))?;
        let radio = radio.ok_or_else(|| RadioError::gone("agent"))?;
        let text = if long { self.full(message_id)? } else { text };
        // to the agent he picked, whatever tags the words start with: he chose who, so nothing in them is read
        radio.queue_as(Outgoing { to: vec![agent_id], ..Outgoing::of(text) }, Carry::Nothing);
        self.shared.host.changed();
        Ok(())
    }
    /// Send a quick message: the feed shows `label`, the agent gets `text`. Who it is for is worked out as for
    /// anything he types.
    pub fn quick(&self, label: String, text: String, re: Option<String>, room: Option<String>, agent: Option<String>) {
        self.deliver(text, Some(label), re, room, agent);
    }
    /// Set going again every agent that has stopped without finishing: in one team, or everywhere.
    pub fn poke(&self, room: Option<String>) {
        // each computer with a stopped agent, and in one room, that room's team there by its id
        let radios: Vec<(Arc<Radio>, Option<String>)> = {
            let state = self.shared.state.lock().unwrap();
            state.computers.iter().filter_map(|s| s.agents.iter().find(|a| a.state == AgentState::Stalled && room.as_ref().is_none_or(|r| a.room == *r)).map(|a| (s.radio.clone(), room.as_ref().map(|_| a.team.clone())))).collect()
        };
        for (radio, team) in radios {
            // what the chat shows for it is what the stopped agents are told
            radio.queue_as(Outgoing { scope: team, ..Outgoing::of("Keep going.".into()) }.with_act(Some(&Act::Control { control: airband_core::routing::Control::Poke })), Carry::Nothing);
        }
        self.shared.host.changed();
    }
    /// Try again a message that was not sent ([`Outgoing::failed`]). It goes after anything else still waiting.
    /// Words sent to several computers at once are one line: each copy given up on is tried again.
    pub fn send_again(&self, id: String) {
        for radio in self.senders_of(&id) {
            radio.send_again(&id);
        }
    }
    /// Let go of a message that was not sent (every copy given up on, for words sent to several computers).
    /// Returns its words, for putting back where he types.
    pub fn discard_unsent(&self, id: String) -> Option<String> {
        let gone = self.senders_of(&id).into_iter().filter_map(|radio| radio.discard(&id)).last().map(|o| o.label.unwrap_or(o.text));
        // what he said aloud goes with its recording
        if gone.is_some() {
            self.shared.senses.takes.discard(&id);
        }
        gone
    }
    /// Take back something he said that is still waiting its turn. Blocks: call it off the main thread.
    /// Words sent to several computers at once are taken back from each.
    pub fn unsend(&self, message_id: String) -> Result<(), RadioError> {
        let radios: Vec<Arc<Radio>> = self.shared.state.lock().unwrap().computers.iter().filter(|s| s.messages.iter().any(|m| m.id == message_id)).map(|s| s.radio.clone()).collect();
        if radios.is_empty() {
            return Err(RadioError::gone("message"));
        }
        // each is asked, whatever the others answer
        let answers: Vec<Result<(), RadioError>> = radios.iter().map(|radio| radio.unsend(message_id.clone())).collect();
        answers.into_iter().collect()
    }
}

impl Desk {
    pub(super) fn deliver(&self, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>) {
        deliver(&self.shared, text, label, re, room, agent, None);
    }
}

/// Send something of his: typed, tapped or said. Who it is for, and so which computer it goes to, is worked out
/// from what he can see; the tower it goes to is told about the agents the other computers have, so it can say
/// the words are for one of those.
///
/// `said`: words he said aloud, by the take they were heard from. They go under its id, and the take is told
/// whether they went into an outbox, or could not go anywhere and wait for him in the chat.
pub(super) fn deliver(shared: &Arc<Shared>, text: String, label: Option<String>, re: Option<String>, room: Option<String>, agent: Option<String>, said: Option<String>) {
    let takes = &shared.senses.takes;
    let unsent = || {
        if let Some(take) = &said {
            takes.unsent(take);
            shared.host.changed();
        }
    };
    let line = |text: String| Outgoing { id: said.clone().unwrap_or_else(|| random_hex(12)), ..Outgoing::of(text) };
    let text = text.trim().to_string();
    let (radio, text, re, scope, others, to) = {
        let mut state = shared.state.lock().unwrap();
        // nothing typed is still something to send when files have arrived: the files themselves. Files still
        // on their way are not yet anything to send
        if state.computers.is_empty() || (text.is_empty() && !state.uploads.iter().any(|u| u.shown.state == UploadState::Ready)) {
            drop(state);
            unsent();
            return;
        }
        // something new said: whatever was waiting to be placed is let go
        state.asking = None;
        // tags are the one thing he types that addresses: `@shop2`, `@all`, read as the tower reads them; words
        // that only start with an @ ("@home is broken") address nobody
        let tagging = tagged_on(&state, &text);
        let tagged = !tagging.is_empty();
        let re = re.filter(|_| !tagged);
        if let Some(id) = &re {
            state.answered.insert(id.clone());
        }
        // a tag that names agents on several computers (`@all`, a team on two of them) goes to each of them
        if tagging.len() > 1 {
            drop(state);
            if let Some(take) = &said {
                takes.sent(take);
            }
            to_each(shared, &tagging, line(text), label);
            return;
        }
        // looking at one agent, with nobody tagged and no message picked: it is for that agent
        let aimed = matches!((&agent, tagged, &re), (Some(_), false, None));
        // with several computers and no main one chosen the app asks him first, and nothing is sent until then
        let Some(wanted) = pick_station(&state, &text, re.as_deref(), room.as_deref(), agent.as_deref()) else {
            drop(state);
            unsent();
            return;
        };
        // Files he attached are already on one computer. Words with no address go there with them. Words he
        // addressed (a tag, a message he picked, the agent he is looking at) go where they are addressed, and
        // if that is another computer the files are sent there first: the words wait for them.
        let pick = match files_at(&state) {
            Some(at) if at != wanted && (tagged || aimed || re.is_some()) => {
                let words = AfterFiles { text: text.clone(), label: label.clone(), re: re.clone(), room: room.clone(), agent: agent.clone(), said: said.clone() };
                if !move_files(shared, &mut state, &at, &wanted, words) {
                    drop(state);
                    unsent();
                    return;
                }
                drop(state);
                shared.host.changed();
                return;
            }
            Some(at) => at,
            None => wanted,
        };
        // the ones that have arrived leave with this message
        state.files_went(&pick, &[]);
        let Some(computer) = state.computer(&pick) else {
            drop(state);
            unsent();
            return;
        };
        // the agent he is looking at, by id: the tower reads nothing to find who it is for
        let to: Vec<String> = match (&agent, aimed) {
            (Some(id), true) => computer.agents.iter().find(|a| a.id == *id).map(|a| a.id.clone()).into_iter().collect(),
            _ => Vec::new(),
        };
        // the room's team on that computer, by its id
        let scope = room.as_ref().filter(|_| re.is_none() && agent.is_none() && !tagged).and_then(|r| computer.agents.iter().find(|a| a.room == *r)).map(|a| a.team.clone());
        // when the tower has to work out who it is for, it is told who else is on his screen
        let others = if tagged || aimed || re.is_some() || state.computers.len() < 2 { Vec::new() } else { others(&state, &pick, room.as_deref()) };
        (computer.radio.clone(), text, re, scope, others, to)
    };
    if let Some(take) = &said {
        takes.sent(take);
    }
    radio.queue_as(Outgoing { re, scope, label, others, to, ..line(text) }, Carry::Answer);
    shared.host.changed();
}
/// Words of his that reached the computer `from` and no agent, now for `agent`: they go to the computer that
/// has it, addressed to it. The files they took go too when that is the computer they are on; otherwise they
/// stay there, for the next thing he sends to it, and he is told.
pub(super) fn place(shared: &Arc<Shared>, from: &StationId, message: Outgoing, agent: String) {
    let found = {
        let state = shared.state.lock().unwrap();
        let from = state.computer(from);
        state.with_agent(&agent).map(|to| (to.radio.clone(), from.map(|f| (f.id() == to.id(), f.radio.clone(), state.computer_name(&f.id())))))
    };
    let Some((radio, origin)) = found else { return };
    // the files go with the words when they are on that computer; the words still answer what they answered
    let mut attachments = Vec::new();
    if !message.attachments.is_empty() {
        match origin {
            Some((true, _, _)) => attachments = message.attachments,
            Some((false, origin, origin_name)) => {
                message.attachments.into_iter().for_each(|f| origin.arm(f));
                shared.host.note(format!("The files you attached are on {origin_name}. They go with the next thing you send there."));
            }
            None => shared.host.note("The files you attached were on a computer that is no longer paired. Your words went without them.".into()),
        }
    }
    radio.queue_as(Outgoing { label: message.label, to: vec![agent], answers: message.answers, attachments, ..Outgoing::of(message.text) }, Carry::Nothing);
}
/// The agents on his screen that computers other than `pick` have, each with the last thing it said: for the
/// tower his words are going to. In one room, only that room's.
fn others(state: &State, pick: &StationId, room: Option<&str>) -> Vec<airband_core::radio::Other> {
    let now = now_ms();
    let mut seen = HashSet::new();
    state.computer(pick).iter().flat_map(|s| s.agents.iter()).for_each(|a| {
        seen.insert(a.id.clone());
    });
    state
        .computers
        .iter()
        .filter(|s| s.id() != *pick)
        .flat_map(|s| s.agents.iter().map(move |a| (s, a)))
        .filter(|(_, a)| room.is_none_or(|r| a.room == r) && seen.insert(a.id.clone()))
        .map(|(s, a)| {
            let last = s.messages.iter().rev().find(|m| m.agent.as_deref() == Some(a.id.as_str()) && !m.fyi);
            airband_core::radio::Other { id: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), group: a.group.clone(), team: airband_core::ids::TeamId::new(a.team.clone()), said: last.map(|m| m.text.chars().take(240).collect()).unwrap_or_default(), ago: last.map(|m| now.saturating_sub(m.at) / 1000).unwrap_or(0) }
        })
        .collect()
}
/// Every computer that has someone his words tag, read the way the tower reads tags, in his list's order.
/// Nothing when they tag nobody.
fn tagged_on(state: &State, text: &str) -> Vec<StationId> {
    state.computers.iter().filter(|s| {
        let live: Vec<Callsign> = s.agents.iter().map(|a| a.parts()).collect();
        !parse_utterance(text, &live).to.is_empty()
    }).map(Computer::id).collect()
}
/// Words whose tags name agents on each of these computers: each of them is sent them, and its tower reads
/// the tags for its own agents. Files he attached go with the copy to the computer they are on; when that is
/// none of these, they stay there for the next thing he sends to it, and he is told.
fn to_each(shared: &Arc<Shared>, computers: &[StationId], line: Outgoing, label: Option<String>) {
    let (radios, stranded) = {
        let mut state = shared.state.lock().unwrap();
        let files = files_at(&state);
        if let Some(at) = files.as_ref().filter(|at| computers.contains(at)) {
            state.files_went(&at.clone(), &[]);
        }
        let stranded = files.filter(|at| !computers.contains(at)).map(|at| state.computer_name(&at));
        let radios: Vec<Arc<Radio>> = computers.iter().filter_map(|id| state.computer(id).map(|s| s.radio.clone())).collect();
        (radios, stranded)
    };
    // One line of his, under one id, sent to each: the chat shows it once whichever computers have it. Each
    // copy takes its own: the files that reached that computer, the question it last asked him back.
    let line = Outgoing { label, ..line };
    for radio in radios {
        radio.queue_as(line.clone(), Carry::Answer);
    }
    if let Some(name) = stranded {
        shared.host.note(format!("The files you attached are on {name}. They go with the next thing you send there."));
    }
    shared.host.changed();
}
/// Which computer a message of his goes to, by its id.
pub(super) fn pick_station(state: &State, text: &str, re: Option<&str>, room: Option<&str>, agent: Option<&str>) -> Option<StationId> {
    // the computer that has the message he is answering
    if let Some(s) = re.and_then(|id| state.owner(id)) {
        return Some(s.id());
    }
    // the computer that has whoever he tagged, read the way the tower will, when that is one computer. Tags
    // on several are sent to each (`to_each`): none of them is picked here
    match tagged_on(state, text).as_slice() {
        [] => {}
        [only] => return Some(only.clone()),
        _ => return None,
    }
    if let Some(s) = agent.and_then(|id| state.with_agent(id)) {
        return Some(s.id());
    }
    // the room he is looking at, when only one computer has agents in it
    if let Some(r) = room {
        let mut with = state.computers.iter().filter(|s| s.agents.iter().any(|a| a.room == r)).map(Computer::id);
        if let (Some(only), None) = (with.next(), with.next()) {
            return Some(only);
        }
    }
    // Nothing says: his main computer, which works out who it is for and passes it on. With several computers
    // and none chosen there is no answer, and he is asked to choose (`DeskView::choose_main`).
    state.main().map(Computer::id)
}

impl Desk {
    /// The radio of the computer that has a message; when none has it now, nothing, and he is told why.
    fn owner_or_say(&self, message_id: &str) -> Option<Arc<Radio>> {
        let radio = self.radio_of(message_id);
        if radio.is_none() {
            self.shared.host.note(UNPAIRED.into());
        }
        radio
    }
}
