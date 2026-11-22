//! What he sees: every computer's chat as one, and which of it he has read.
use super::*;

/// How many lines of the feed a view carries.
const FEED_SHOWN: usize = 80;
/// What he has read: for each agent, by its id, the time of the last thing it said that was on screen in
/// front of him. Kept on the device in one small file, so it holds across launches. Something said after that
/// time is unread, wherever he is looking now.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ReadMarks {
    /// When this device began keeping count: what an agent said before then is not counted against him.
    pub(super) since: u64,
    pub(super) upto: HashMap<String, u64>,
    /// His main computer, once he has chosen one: where his words go when nothing says which
    /// computer they are for, by its [`DeskComputer::id`]. It works out who they are for and passes them on.
    #[serde(default)]
    pub(super) main: Option<StationId>,
    #[serde(skip)]
    keeper: airband_io::kept::Keeper,
    /// What he is to be told about the file when the desk opens: it could not be read.
    #[serde(skip)]
    pub(super) opened: Option<String>,
}
impl ReadMarks {
    pub(super) fn load(dir: &str) -> Self {
        let (kept, keeper, opened) = crate::kept::open::<Option<ReadMarks>>(crate::kept::file(dir, "read.json"), "What you have read, and your default computer,");
        ReadMarks { keeper, opened, ..kept.unwrap_or_else(|| ReadMarks { since: airband_core::clock::now_ms(), ..ReadMarks::default() }) }
    }
    /// Keep them on the device. The words to tell him when they could not be kept.
    #[must_use]
    pub(super) fn keep(&self) -> Option<String> {
        crate::kept::told(self.keeper.save(self), "What you have read, and your main computer, are not kept if the app closes.")
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
        let lost = {
            let mut state = self.shared.state.lock().unwrap();
            state.read.main = Some(StationId::from(computer));
            state.read.keep()
        };
        if let Some(lost) = lost {
            self.shared.host.note(lost);
        }
        self.shared.host.changed();
    }

    /// Look at one team (its room id), one agent (its id), or with neither, everyone. Whatever he was looking at
    /// before is let go. The next [`Desk::view`] shows it.
    pub fn look(&self, room: Option<String>, agent: Option<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let made = state.focus.made.clone();
            state.focus = Focus { room: room.filter(|r| !r.is_empty()), agent: agent.filter(|a| !a.is_empty()), made };
        }
        self.shared.host.changed();
    }

    /// What he is looking at ([`Desk::look`]) is in front of him: the messages it shows are read. What agents
    /// he is not looking at have said stays unread. Nothing while the chat is not in front of him. Call it
    /// before [`Desk::view`], each time the screen is drawn.
    pub fn shown(&self) {
        let lost = {
            let mut state = self.shared.state.lock().unwrap();
            state.keep_focus();
            if !self.shared.host.watching() {
                return;
            }
            let (room, agent) = (state.focus.room.clone(), state.focus.agent.clone());
            let all = state.merged();
            let mut moved = false;
            for m in all.iter().filter(|m| room.as_ref().is_none_or(|r| m.room == *r) && agent.as_ref().is_none_or(|a| m.agent.as_ref() == Some(a))) {
                if state.read.unread(m) {
                    state.read.upto.insert(m.agent.clone().unwrap_or_default(), m.at);
                    moved = true;
                }
            }
            if moved { state.read.keep() } else { None }
        };
        // told once the screen's state is let go: the host may ask for a view again when told
        if let Some(lost) = lost {
            self.shared.host.note(lost);
        }
    }

    /// What to show, for what he is looking at ([`Desk::look`]): one team, one agent, or everything. Changes
    /// nothing: what it shows is marked read by [`Desk::shown`].
    pub fn view(&self) -> DeskView {
        let state = self.shared.state.lock().unwrap();
        let (room, agent) = (state.focus.room.clone(), state.focus.agent.clone());
        let all = state.merged();
        let mut unread_from: HashMap<&str, u32> = HashMap::new();
        let mut unread_in: HashMap<&str, u32> = HashMap::new();
        for m in all.iter().filter(|m| state.read.unread(m)) {
            *unread_from.entry(m.agent.as_deref().unwrap_or_default()).or_default() += 1;
            *unread_in.entry(m.room.as_str()).or_default() += 1;
        }
        let agents: Vec<&ChatAgent> = state.everyone();
        // his own messages belong to the room or agent they went to
        let here = |m: &ChatMessage| {
            let about = |id: &String| agent.as_ref().is_none_or(|a| id == a) && room.as_ref().is_none_or(|r| agents.iter().any(|x| x.id == *id && x.room == *r));
            if m.his { (room.is_none() && agent.is_none()) || m.to_agents.iter().any(about) } else { room.as_ref().is_none_or(|r| m.room == *r) && agent.as_ref().is_none_or(|a| m.agent.as_ref() == Some(a)) }
        };
        let mut feed: Vec<ChatMessage> = all.iter().filter(|&m| !m.fyi && here(m)).cloned().map(|m| ChatMessage { open: state.is_open(&m.id), ..m }).collect();
        let extra = feed.len().saturating_sub(FEED_SHOWN);
        feed.drain(..extra);
        let rooms = rooms(&state, &agents, &all, &unread_from, &unread_in);
        // the same quick messages from every computer, each once
        let mut quick: Vec<QuickMessage> = Vec::new();
        for q in state.computers.iter().flat_map(|s| s.quick.iter()) {
            if !quick.iter().any(|have| have.label == q.label && have.text == q.text) {
                quick.push(q.clone());
            }
        }
        let tools = crate::services::coding_tools();
        let used: Vec<crate::services::CodingTool> = self.shared.chooser.tools().iter().filter_map(|id| tools.iter().find(|t| t.id == *id).cloned()).collect();
        let away: Vec<String> = if state.computers.len() > 1 { state.computers.iter().filter(|s| s.tried && !s.online).map(|s| state.computer_name(&s.id())).collect() } else { Vec::new() };
        DeskView {
            focus: DeskFocus { room: room.clone(), agent: agent.clone() },
            new_agent: NewAgentDefaults { computer: (state.computers.len() == 1).then(|| state.computers[0].id().to_string()), tool: (used.len() == 1).then(|| used[0].clone()) },
            tools: used,
            cannot_reach: if away.is_empty() { "Can\u{2019}t reach your computer".to_string() } else { format!("Can\u{2019}t reach {}", away.join(", ")) },
            colours: state.colours.all(),
            paired: !state.computers.is_empty(),
            online: state.computers.iter().any(|s| s.online),
            connecting: !state.computers.iter().any(|s| s.online) && state.computers.iter().any(|s| !s.tried),
            near: state.computers.iter().any(|s| s.online) && state.computers.iter().filter(|s| s.online).all(|s| s.radio.near()),
            away,
            can_poke: agents.iter().any(|a| a.state == AgentState::Stalled && room.as_ref().is_none_or(|r| a.room == *r)),
            feed,
            rooms,
            unread: unread_from.values().sum(),
            quick,
            sending: sending(&state),
            stations: state.computers.iter().map(|s| DeskComputer { id: s.id().to_string(), name: state.computer_name(&s.id()) }).collect(),
            main_computer: state.main().map(|s| s.id().to_string()),
            choose_main: state.computers.len() > 1 && state.main().is_none(),
            uploads: state.uploads.iter().map(|u| u.shown.clone()).collect(),
            intents: self.intents_view(&state),
            asking: state.asking.as_ref().map(|asking| asked(&state, &agents, asking)),
        }
    }
    /// What was said between him and an agent just before one of its messages, oldest first: the context for
    /// a question.
    pub fn thread(&self, message_id: String) -> Vec<ChatMessage> {
        let state = self.shared.state.lock().unwrap();
        let Some((computer, m)) = state.message(&message_id) else { return Vec::new() };
        let Some(agent) = m.agent.as_ref() else { return Vec::new() };
        let before: Vec<ChatMessage> = computer.messages.iter().filter(|b| b.seq < m.seq && (b.agent.as_ref() == Some(agent) || (b.his && b.to_agents.contains(agent)))).cloned().collect();
        before[before.len().saturating_sub(5)..].to_vec()
    }
}

/// Each team, in the state of whichever of its agents most needs him, with its agents.
fn rooms(state: &State, agents: &[&ChatAgent], all: &[ChatMessage], unread_from: &HashMap<&str, u32>, unread_in: &HashMap<&str, u32>) -> Vec<DeskRoom> {
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
    for a in agents {
        let member = DeskMember { tool_name: crate::services::tool_name(&a.harness).unwrap_or_default(), id: a.id.clone(), callsign: a.callsign.clone(), state: a.state, subagents: a.subagents, doing: a.doing.clone(), held: a.held, terminal: a.terminal, ends: a.ends, unread: unread_from.get(a.id.as_str()).copied().unwrap_or(0), question: asked(a).map(|m| m.text.clone()).unwrap_or_default(), question_id: asked(a).map(|m| m.id.clone()).unwrap_or_default(), harness: a.harness.clone(), badge: a.badge.clone(), model: a.model.clone().unwrap_or_default(), effort: a.effort.clone().unwrap_or_default(), number: a.number.clone(), station: if state.computers.len() > 1 { state.computers.iter().find(|s| s.agents.iter().any(|x| x.id == a.id)).map(|s| s.id().to_string()).unwrap_or_default() } else { String::new() } };
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
    rooms
}

/// What is on its way: not yet sent, then sent and not yet back in the chat (for a quarter of a minute at most).
fn sending(state: &State) -> Vec<DeskOutgoing> {
    state
        .computers
        .iter()
        .flat_map(|s| s.radio.unsent().into_iter().filter(|o| !s.messages.iter().any(|m| m.id == o.id)).chain(s.landing.iter().filter(|(o, at)| at.elapsed() < Duration::from_secs(15) && !s.messages.iter().any(|m| m.id == o.id)).map(|(o, _)| o.clone())))
        .map(|o| DeskOutgoing::from(&o))
        .collect()
}

/// Who his words that reached no agent may be for: the tower's guesses first, in its order; then everyone
/// else, whoever spoke last first.
fn asked(state: &State, agents: &[&ChatAgent], asking: &Asking) -> DeskAsk {
    let mut rest: Vec<&ChatAgent> = agents.iter().copied().filter(|a| !asking.guesses.contains(&a.id)).collect();
    let spoke = |a: &ChatAgent| state.computers.iter().flat_map(|s| s.messages.iter()).filter(|m| m.agent.as_deref() == Some(a.id.as_str())).map(|m| m.at).max().unwrap_or(0);
    rest.sort_by_key(|a| std::cmp::Reverse(spoke(a)));
    let guessed = asking.guesses.iter().filter_map(|id| agents.iter().copied().find(|a| a.id == *id));
    DeskAsk {
        text: asking.message.label.clone().unwrap_or_else(|| asking.message.text.clone()),
        choices: guessed.chain(rest).take(8).map(|a| DeskChoice { id: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), colour: state.colour(&a.room) }).collect(),
    }
}
