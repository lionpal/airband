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
}
impl ReadMarks {
    pub(super) fn load(dir: &str) -> Self {
        let kept = if dir.is_empty() { None } else { airband_core::kept::load(&std::path::Path::new(dir).join("read.json")) };
        kept.unwrap_or_else(|| ReadMarks { since: airband_core::clock::now_ms(), upto: HashMap::new() })
    }
    pub(super) fn keep(&self, dir: &str) {
        if !dir.is_empty() {
            let _ = airband_core::kept::save(&std::path::Path::new(dir).join("read.json"), self);
        }
    }
    pub(super) fn unread(&self, m: &ChatMessage) -> bool {
        m.from != "you" && !m.fyi && m.at > self.upto.get(&m.from).copied().unwrap_or(self.since)
    }
}
#[uniffi::export]
impl Desk {
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
                None => rooms.push(DeskRoom { name: a.group.clone(), state: if a.state.is_empty() { "working".into() } else { a.state.clone() }, agents: 1, subagents: a.subagents, doing, members: vec![member], unread: unread_in.get(a.group.as_str()).copied().unwrap_or(0), colour: state.colour(&a.group) }),
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
            away: if state.stations.len() > 1 { state.stations.iter().enumerate().filter(|(_, s)| s.tried && !s.online).map(|(i, _)| state.computer_name(i)).collect() } else { Vec::new() },
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
                    choices: guessed.chain(rest).take(8).map(|a| DeskChoice { id: a.id.clone(), callsign: a.callsign.clone(), badge: airband_core::routing::badge(&a.group, &a.callsign), colour: state.colour(&a.group) }).collect(),
                }
            }),
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
}
