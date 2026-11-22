//! What he sees: every computer's chat as one, and which of it he has read.
use super::*;

/// How many lines of the feed a view carries.
const FEED_SHOWN: usize = 80;
/// What he has read: for each agent, by callsign, the time of the last thing it said that was on screen in
/// front of him. Kept on the device in one small file, so it holds across launches. Something said after that
/// time is unread, wherever he is looking now.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ReadMarks {
    /// When this device began keeping count: what an agent said before then is not counted against him.
    pub(super) since: u64,
    pub(super) upto: HashMap<String, u64>,
    /// His main computer, by name, once he has chosen one: where his words go when nothing says which
    /// computer they are for, by its [`DeskComputer::id`]. It works out who they are for and passes them on.
    #[serde(default)]
    pub(super) main: Option<String>,
}
impl ReadMarks {
    pub(super) fn load(dir: &str) -> Self {
        let kept: Option<ReadMarks> = if dir.is_empty() { None } else { crate::kept::load_or_set_aside(&std::path::Path::new(dir).join("read.json")) };
        kept.unwrap_or_else(|| ReadMarks { since: airband_core::clock::now_ms(), upto: HashMap::new(), main: None })
    }
    pub(super) fn keep(&self, dir: &str) {
        if !dir.is_empty() {
            // what he has read, not kept, is shown as unread again after the app opens
            let _ = crate::kept::save(&std::path::Path::new(dir).join("read.json"), self);
        }
    }
    pub(super) fn unread(&self, m: &ChatMessage) -> bool {
        !m.his && !m.fyi && m.at > self.upto.get(m.agent.as_deref().unwrap_or_default()).copied().unwrap_or(self.since)
    }
}
#[uniffi::export]
impl Desk {
    /// Make one of his computers the main one, by its [`DeskComputer::id`]: where his words go when nothing
    /// says which computer they are for. Kept on the device.
    pub fn set_main_computer(&self, computer: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.read.main = Some(computer);
            state.read.keep(&self.data_dir);
        }
        self.shared.host.changed();
    }

    /// What to show. `room` narrows the feed to one team, `agent` to one agent (its id); with neither, everything.
    pub fn view(&self, room: Option<String>, agent: Option<String>) -> DeskView {
        let mut state = self.shared.state.lock().unwrap();
        let all = state.merged();
        // What this view shows, while the chat is in front of him, is read. What agents he is not looking at
        // have said stays unread, and is counted for each of them and each team below.
        if self.shared.host.watching() {
            let mut moved = false;
            for m in all.iter().filter(|m| room.as_ref().is_none_or(|r| m.room == *r) && agent.as_ref().is_none_or(|a| m.agent.as_ref() == Some(a))) {
                if state.read.unread(m) {
                    state.read.upto.insert(m.agent.clone().unwrap_or_default(), m.at);
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
            *unread_from.entry(m.agent.as_deref().unwrap_or_default()).or_default() += 1;
            *unread_in.entry(m.room.as_str()).or_default() += 1;
        }
        let unread: u32 = unread_from.values().sum();
        let agents: Vec<&ChatAgent> = state.everyone();
        // his own messages belong to the room or agent they went to
        let here = |m: &ChatMessage| {
            let about = |id: &String| agent.as_ref().is_none_or(|a| id == a) && room.as_ref().is_none_or(|r| agents.iter().any(|x| x.id == *id && x.room == *r));
            if m.his { (room.is_none() && agent.is_none()) || m.to_agents.iter().any(about) } else { room.as_ref().is_none_or(|r| m.room == *r) && agent.as_ref().is_none_or(|a| m.agent.as_ref() == Some(a)) }
        };
        let mut feed: Vec<ChatMessage> = all.iter().filter(|&m| !m.fyi && here(m)).cloned().map(|m| ChatMessage { open: state.is_open(&m.id), ..m }).collect();
        let extra = feed.len().saturating_sub(FEED_SHOWN);
        feed.drain(..extra);
        // each team, in the state of whichever of its agents most needs him
        let rank = |s: AgentState| match s {
            AgentState::Unknown => 0,
            AgentState::Idle => 1,
            AgentState::Working => 2,
            AgentState::Stalled => 3,
            AgentState::Asking => 4,
        };
        let mut rooms: Vec<DeskRoom> = Vec::new();
        // what an agent is waiting on him for: its newest question still open
        let asked = |a: &ChatAgent| all.iter().rev().find(|m| m.needs_you && m.agent.as_deref() == Some(a.id.as_str()) && state.is_open(&m.id));
        for a in &agents {
            let member = DeskMember { id: a.id.clone(), callsign: a.callsign.clone(), state: a.state, subagents: a.subagents, doing: a.doing.clone(), held: a.held, terminal: a.terminal, ends: a.ends, unread: unread_from.get(a.id.as_str()).copied().unwrap_or(0), question: asked(a).map(|m| m.text.clone()).unwrap_or_default(), question_id: asked(a).map(|m| m.id.clone()).unwrap_or_default(), harness: a.harness.clone(), badge: a.badge.clone(), model: a.model.clone().unwrap_or_default(), effort: a.effort.clone().unwrap_or_default(), number: a.number.clone(), station: if state.stations.len() > 1 { state.stations.iter().find(|s| s.agents.iter().any(|x| x.id == a.id)).map(Station::id).unwrap_or_default() } else { String::new() } };
            // what the team is doing is said in the name of whoever is doing it, without the team's own name
            let doing = a.doing.as_ref().map(|d| match Some(a.number.as_str()).filter(|rest| !rest.is_empty()) {
                Some(who) => format!("{who}: {d}"),
                None => d.clone(),
            });
            match rooms.iter_mut().find(|r| r.id == a.room) {
                Some(r) => {
                    r.agents += 1;
                    r.subagents += a.subagents;
                    if rank(a.state) > rank(r.state) {
                        r.state = a.state;
                    }
                    r.doing = r.doing.take().or(doing);
                    r.members.push(member);
                }
                None => rooms.push(DeskRoom { id: a.room.clone(), name: a.group.clone(), state: a.state, agents: 1, subagents: a.subagents, doing, members: vec![member], unread: unread_in.get(a.room.as_str()).copied().unwrap_or(0), colour: state.colour(&a.room) }),
            }
        }
        rooms.sort_by(|a, b| a.name.cmp(&b.name));
        rooms.iter_mut().for_each(|r| r.members.sort_by(|a, b| a.callsign.cmp(&b.callsign)));
        // the same quick messages from every computer, each once
        let mut quick: Vec<QuickMessage> = Vec::new();
        for q in state.stations.iter().flat_map(|s| s.quick.iter()) {
            if !quick.iter().any(|have| have.label == q.label && have.text == q.text) {
                quick.push(q.clone());
            }
        }
        DeskView {
            colours: state.colours.all(),
            paired: !state.stations.is_empty(),
            online: state.stations.iter().any(|s| s.online),
            connecting: !state.stations.iter().any(|s| s.online) && state.stations.iter().any(|s| !s.tried),
            near: state.stations.iter().any(|s| s.online) && state.stations.iter().filter(|s| s.online).all(|s| s.radio.near()),
            away: if state.stations.len() > 1 { state.stations.iter().enumerate().filter(|(_, s)| s.tried && !s.online).map(|(i, _)| state.computer_name(i)).collect() } else { Vec::new() },
            can_poke: agents.iter().any(|a| a.state == AgentState::Stalled && room.as_ref().is_none_or(|r| a.room == *r)),
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
            stations: state.stations.iter().enumerate().map(|(i, s)| DeskComputer { id: s.id(), name: state.computer_name(i) }).collect(),
            main_computer: state.main().map(|i| state.stations[i].id()),
            choose_main: state.stations.len() > 1 && state.main().is_none(),
            uploads: state.uploads.iter().map(|u| u.shown.clone()).collect(),
            asking: state.asking.as_ref().map(|asking| {
                // the tower's guesses first, in its order; then everyone else, whoever spoke last first
                let mut rest: Vec<&ChatAgent> = agents.iter().copied().filter(|a| !asking.guesses.contains(&a.id)).collect();
                let spoke = |a: &ChatAgent| state.stations.iter().flat_map(|s| s.messages.iter()).filter(|m| m.agent.as_deref() == Some(a.id.as_str())).map(|m| m.at).max().unwrap_or(0);
                rest.sort_by_key(|a| std::cmp::Reverse(spoke(a)));
                let guessed = asking.guesses.iter().filter_map(|id| agents.iter().copied().find(|a| a.id == *id));
                DeskAsk {
                    text: asking.message.label.clone().unwrap_or_else(|| asking.message.text.clone()),
                    choices: guessed.chain(rest).take(8).map(|a| DeskChoice { id: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), colour: state.colour(&a.room) }).collect(),
                }
            }),
        }
    }
    /// What was said between him and an agent just before one of its messages, oldest first: the context for
    /// a question.
    pub fn thread(&self, message_id: String) -> Vec<ChatMessage> {
        let state = self.shared.state.lock().unwrap();
        let Some((index, m)) = state.message(&message_id) else { return Vec::new() };
        let Some(agent) = m.agent.as_ref() else { return Vec::new() };
        let before: Vec<ChatMessage> = state.stations[index].messages.iter().filter(|b| b.seq < m.seq && (b.agent.as_ref() == Some(agent) || (b.his && b.to_agents.contains(agent)))).cloned().collect();
        before[before.len().saturating_sub(5)..].to_vec()
    }
}
