//! What he sees: every computer's chat as one, and which of it he has read.
use super::*;

/// How many lines of the feed a view carries.
const FEED_SHOWN: usize = 80;
/// What he has read: for each agent, by its id, the time of the last thing it said that was on screen in
/// front of him. Kept on the device in one small file, so it holds across launches. Something said after that
/// time is unread, wherever he is looking now.
#[derive(Default)]
pub(super) struct ReadMarks {
    pub(super) marks: Marks,
    keeper: airband_io::kept::Keeper,
    /// What he is to be told about the file when the desk opens: it could not be read.
    pub(super) opened: Option<String>,
}

/// What `read.json` holds.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Marks {
    /// When this device began keeping count: what an agent said before then is not counted against him.
    pub(super) since: u64,
    pub(super) upto: HashMap<String, u64>,
    /// His main computer, once he has chosen one: where his words go when nothing says which
    /// computer they are for, by its [`DeskComputer::id`]. It works out who they are for and passes them on.
    #[serde(default)]
    pub(super) main: Option<StationId>,
    /// The picture he picked for each computer, by its [`DeskComputer::id`]. One he has not picked for is
    /// drawn by its shape.
    #[serde(default)]
    pub(super) pictures: BTreeMap<String, ComputerPicture>,
}

impl Marks {
    /// A device that has kept no count yet: it begins now.
    fn begun() -> Marks {
        Marks { since: airband_core::clock::now_ms(), ..Marks::default() }
    }
}

impl crate::kept::Rejoin for Marks {
    /// What was read since is kept over what the file held, and so is a main computer chosen since.
    fn rejoin(&mut self, since: Marks) {
        for (agent, at) in since.upto {
            let mine = self.upto.entry(agent).or_insert(at);
            *mine = (*mine).max(at);
        }
        self.since = self.since.min(since.since);
        if since.main.is_some() {
            self.main = since.main;
        }
        self.pictures.extend(since.pictures);
    }
}

impl ReadMarks {
    pub(super) fn load(dir: &str) -> Self {
        let (kept, keeper, opened) = crate::kept::open::<Option<Marks>>(crate::kept::file(dir, "read.json"), "What you have read, and your default computer,");
        ReadMarks { marks: kept.unwrap_or_else(Marks::begun), keeper, opened }
    }
    /// Keep them on the device. The words to tell him when they could not be kept.
    #[must_use]
    pub(super) fn keep(&mut self) -> Option<String> {
        crate::kept::again_as::<Option<Marks>, Marks>(&self.keeper, &mut self.marks, |found| found.flatten().unwrap_or_else(Marks::begun));
        crate::kept::told(self.keeper.save(&self.marks), "What you have read, your main computer and its picture are not kept if the app closes.")
    }
    pub(super) fn unread(&self, m: &ChatMessage) -> bool {
        !m.his && !m.fyi && m.at > self.marks.upto.get(m.agent.as_deref().unwrap_or_default()).copied().unwrap_or(self.marks.since)
    }
}
#[uniffi::export]
impl Desk {
    /// Make one of his computers the main one, by its [`DeskComputer::id`]: where his words go when nothing
    /// says which computer they are for. Kept on the device.
    pub fn set_main_computer(&self, computer: String) {
        let lost = {
            let mut state = self.shared.state.lock().unwrap();
            state.read.marks.main = Some(StationId::new(computer));
            state.read.keep()
        };
        if let Some(lost) = lost {
            self.shared.host.note(lost);
        }
        self.shared.host.changed();
    }

    /// Draw one of his computers, by its [`DeskComputer::id`], with this picture from now on, whatever its shape.
    pub fn set_computer_picture(&self, computer: String, picture: ComputerPicture) {
        let lost = {
            let mut state = self.shared.state.lock().unwrap();
            state.read.marks.pictures.insert(computer, picture);
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
            // he looks somewhere else: a message opened from a notification is no longer what he is on
            state.opening = None;
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
                    state.read.marks.upto.insert(m.agent.clone().unwrap_or_default(), m.at);
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
        let stoppable = rooms.iter().filter(|r| room.as_ref().is_none_or(|x| r.id == *x)).flat_map(|r| r.members.iter()).filter(|m| m.can_stop && agent.as_ref().is_none_or(|a| m.id == *a)).cloned().collect();
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
            opening: state.opening.as_ref().map(|o| opening_state(&state, &all, o)),
            focus: DeskFocus { room: room.clone(), agent: agent.clone() },
            stoppable,
            new_agent: NewAgentDefaults { computer: (state.computers.len() == 1).then(|| state.computers[0].id().to_string()), tool: (used.len() == 1).then(|| used[0].clone()) },
            tools: used,
            cannot_reach: cannot_reach(&state),
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
            several_computers: state.computers.len() > 1,
            unread_elsewhere: {
                // the team he is in: the one he picked, or the one of the agent he is looking at
                let team = room.clone().or_else(|| agent.as_ref().and_then(|id| agents.iter().find(|a| a.id == *id)).map(|a| a.room.clone()));
                unread_from.values().sum::<u32>() - team.and_then(|t| unread_in.get(t.as_str()).copied()).unwrap_or(0)
            },
            quick,
            sending: sending(&state, &self.shared.senses.takes, room.as_deref(), agent.as_deref()),
            no_way_in: agent.as_ref().and_then(|id| agents.iter().find(|a| a.id == *id)).filter(|a| !a.reaches).map(|a| DeskNoWayIn { agent: a.id.clone(), line: no_way_in(&a.callsign), take_over: a.take_over }),
            takes: takes_shown(&state, &self.shared.senses.takes),
            stations: state.computers.iter().map(|s| state.shown(s)).collect(),
            main_computer: state.main().map(|s| s.id().to_string()),
            // asked only once every computer can be named to him: each is, from its pairing link on
            choose_main: state.computers.len() > 1 && state.main().is_none() && state.computers.iter().all(|s| !state.computer_name(&s.id()).trim().is_empty()),
            uploads: state.uploads.iter().map(|u| u.shown.clone()).collect(),
            intents: self.intents_view(&state),
            commands: super::slash::view(&state),
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
        AgentState::Idle => 1,
        AgentState::Working => 2,
        AgentState::Stalled => 3,
        AgentState::Asking => 4,
    };
    let mut rooms: Vec<DeskRoom> = Vec::new();
    // what an agent is waiting on him for: its newest question still open
    let asked = |a: &ChatAgent| all.iter().rev().find(|m| m.needs_you && m.agent.as_deref() == Some(a.id.as_str()) && state.is_open(&m.id));
    for a in agents {
        let member = DeskMember { take_over: a.take_over, tool_name: crate::services::tool_name(&a.harness).unwrap_or_default(), id: a.id.clone(), callsign: a.callsign.clone(), state: a.state, subagents: a.subagents, doing: a.doing.clone(), held: a.held, can_stop: a.held && a.state == AgentState::Working, move_to: Vec::new(), terminal: a.terminal, ends: a.ends, unread: unread_from.get(a.id.as_str()).copied().unwrap_or(0), question: asked(a).map(|m| m.text.clone()).unwrap_or_default(), question_id: asked(a).map(|m| m.id.clone()).unwrap_or_default(), harness: a.harness.clone(), badge: a.badge.clone(), model: a.model.clone().unwrap_or_default(), effort: a.effort.clone().unwrap_or_default(), number: a.number.clone(), station: if state.computers.len() > 1 { state.computers.iter().find(|s| s.agents.iter().any(|x| x.id == a.id)).map(|s| s.id().to_string()).unwrap_or_default() } else { String::new() } };
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
                r.working |= a.state == AgentState::Working;
                r.members.push(member);
            }
            None => rooms.push(DeskRoom { id: a.room.clone(), name: a.group.clone(), state: a.state, agents: 1, subagents: a.subagents, doing, members: vec![member], unread: unread_in.get(a.room.as_str()).copied().unwrap_or(0), working: a.state == AgentState::Working, colour: state.colour(&a.room) }),
        }
    }
    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    rooms.iter_mut().for_each(|r| r.members.sort_by(|a, b| a.callsign.cmp(&b.callsign)));
    // an agent moves only to another team on its own computer
    let on: Vec<(String, Vec<String>)> = rooms.iter().map(|r| (r.id.clone(), r.members.iter().map(|m| m.station.clone()).collect())).collect();
    for r in rooms.iter_mut() {
        let id = r.id.clone();
        for m in r.members.iter_mut() {
            m.move_to = on.iter().filter(|(other, stations)| *other != id && stations.contains(&m.station)).map(|(other, _)| other.clone()).collect();
        }
    }
    rooms
}

/// What the place he types says of an agent his words cannot reach.
pub(super) fn no_way_in(callsign: &str) -> String {
    format!("airband can\u{2019}t send messages to {callsign}.")
}

/// What is on its way: not yet sent, then sent and not yet back in the chat (for [`super::LANDING`] at most),
/// and what was not delivered, which stays until he lets it go or sends it again and it is. Each is shown where
/// he sent it from, and with any agent it was for: looking at everyone, all of it.
/// Words sent to several computers at once (a tag on agents of each) are one line under one id: shown once,
/// and not at all once the chat has them from one of those computers, unless a copy was not delivered, which
/// is shown so he can send it again or let it go, with why for every copy that was not.
fn sending(state: &State, takes: &crate::takes::Takes, room: Option<&str>, agent: Option<&str>) -> Vec<DeskOutgoing> {
    let in_chat = |id: &str| state.computers.iter().any(|s| s.messages.iter().any(|m| m.id == id));
    let agents = state.everyone();
    let room_of = |id: &str| agents.iter().find(|a| a.id == id).map(|a| a.room.clone());
    let named = |id: &str| agents.iter().find(|a| a.id == id).map(|a| a.callsign.clone()).unwrap_or_else(|| "an agent that has gone".into());
    // the agents it is about: the ones it was for, and the ones it did not reach
    let about = |o: &Outgoing| -> Vec<String> { o.to.iter().cloned().chain(o.sent_from.agent.clone()).chain(o.undelivered.iter().flat_map(|u| u.missed().into_iter().map(|m| m.agent.clone()))).collect() };
    let here = |o: &Outgoing| match (room, agent) {
        (None, None) => true,
        (_, Some(a)) => about(o).iter().any(|x| x == a),
        (Some(r), None) => o.sent_from.room.as_deref() == Some(r) || about(o).iter().any(|x| room_of(x).as_deref() == Some(r)),
    };
    let mut shown: Vec<(Outgoing, Vec<String>)> = Vec::new();
    let waiting = state.computers.iter().flat_map(|s| s.radio.unsent().into_iter().chain(s.landing.iter().filter(|(_, at)| at.elapsed() < super::LANDING).map(|(o, _)| o.clone())));
    for o in waiting.filter(|o| (o.undelivered.is_some() || !in_chat(&o.id)) && here(o)) {
        let why = o.undelivered.is_some().then(|| crate::outbox::not_delivered_why(&o, &named));
        match shown.iter_mut().find(|(have, _)| have.id == o.id) {
            // the copy not delivered is the one he has to act on, and every such copy says why
            Some((have, whys)) => {
                if let Some(why) = why.filter(|w| !whys.contains(w)) {
                    whys.push(why);
                }
                if have.undelivered.is_none() && o.undelivered.is_some() {
                    *have = o;
                }
            }
            None => shown.push((o, why.into_iter().collect())),
        }
    }
    shown
        .iter()
        .map(|(o, whys)| {
            // the agent it did not reach for want of a way in, when its computer can take it over
            let take_over = o.undelivered.iter().flat_map(|u| u.missed()).filter(|m| m.why == airband_core::radio::NotReachable::NoWayIn).map(|m| m.agent.clone()).find(|id| agents.iter().any(|a| a.id == *id && a.take_over));
            let not_delivered = o.undelivered.is_some().then(|| format!("Not delivered. {}", whys.join(" ")).trim_end().to_string());
            DeskOutgoing { recording: takes.kept(&o.id), not_delivered, take_over, ..DeskOutgoing::from(o) }
        })
        .collect()
}

/// What he said aloud that is not on its way, each in a line: not heard, being heard again, or heard and not
/// sent. Words in an outbox are shown by it ([`sending`]); words whose outbox is gone (that computer was
/// unpaired) are not sent.
fn takes_shown(state: &State, takes: &crate::takes::Takes) -> Vec<DeskTake> {
    use crate::takes::Stage;
    const NOT_SENT: &str = "Not sent.";
    let waiting = |id: &str| state.computers.iter().any(|s| s.radio.unsent().iter().any(|o| o.id == id));
    takes
        .all()
        .into_iter()
        .filter_map(|t| {
            let line = match &t.stage {
                Stage::Unheard { why } => format!("Not heard. {}", why.told()),
                Stage::Hearing { .. } => "Listening again.".into(),
                Stage::Unsent { .. } => NOT_SENT.into(),
                Stage::Sent { .. } if !waiting(&t.id) => NOT_SENT.into(),
                Stage::Recording | Stage::Heard { .. } | Stage::Sent { .. } => return None,
            };
            Some(DeskTake { busy: matches!(t.stage, Stage::Hearing { .. }), id: t.id, line })
        })
        .collect()
}

/// What he is told of the computers that cannot be talked to: each that refuses this phone by what it said,
/// and the ones that do not answer as out of reach.
fn cannot_reach(state: &State) -> String {
    let several = state.computers.len() > 1;
    let refusing: Vec<String> = state.computers.iter().filter(|c| c.tried && !c.online).filter_map(|c| c.refused.as_ref().map(|why| if several { format!("{}: {why}", state.computer_name(&c.id())) } else { why.clone() })).collect();
    let silent: Vec<String> = state.computers.iter().filter(|c| c.tried && !c.online && c.refused.is_none()).map(|c| state.computer_name(&c.id())).collect();
    let silent = match (several, silent.is_empty()) {
        (_, true) if !refusing.is_empty() => None,
        (true, false) => Some(format!("Can\u{2019}t reach {}.", silent.join(", "))),
        _ => Some(crate::UNREACHED.to_string()),
    };
    refusing.into_iter().chain(silent).collect::<Vec<_>>().join(" ")
}

/// Who his words that reached no agent may be for: the tower's guesses first, in its order; then everyone
/// else, whoever spoke last first.
fn asked(state: &State, agents: &[&ChatAgent], asking: &Asking) -> DeskAsk {
    let mut rest: Vec<&ChatAgent> = agents.iter().copied().filter(|a| !asking.guesses.contains(&a.id)).collect();
    let spoke = |a: &ChatAgent| state.computers.iter().flat_map(|s| s.messages.iter()).filter(|m| m.agent.as_deref() == Some(a.id.as_str())).map(|m| m.at).max().unwrap_or(0);
    rest.sort_by_key(|a| std::cmp::Reverse(spoke(a)));
    let guessed = asking.guesses.iter().filter_map(|id| agents.iter().copied().find(|a| a.id == *id));
    DeskAsk {
        text: asking.message.shown(),
        choices: guessed.chain(rest).take(8).map(|a| DeskChoice { id: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), colour: state.colour(&a.room) }).collect(),
    }
}
