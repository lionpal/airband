//! The agents on the radio: who each one is, what it is called and which team it is in, and how one joins,
//! is renamed, moves between teams and leaves.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Session {
    pub(super) agent: Agent,
    pub(super) callsign: Callsign,
    /// The team it is in, by the team's id ([`Inner::teams`]). Its callsign carries what that team is called.
    #[serde(default)]
    pub(super) team: String,
    /// It finished a turn and is waiting on him.
    pub(super) waiting: bool,
    /// The line of the last thing he heard from it, for "say more" and "repeat".
    #[serde(default)]
    pub(super) last_heard: Option<String>,
    /// Pictures it has touched since it last spoke; they go with what it says next.
    #[serde(default)]
    pub(super) pictures: Vec<pictures::Seen>,
    /// Subagents it has running.
    #[serde(default)]
    pub(super) subagents: u32,
    /// What it is doing right now, while it works: its harness's words for its latest step. Never kept.
    #[serde(skip)]
    pub(super) doing: Option<String>,
    /// The model it runs on and how hard it is set to think, as last told. Kept, so one that has been waiting
    /// since before the tower restarted still shows them.
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) effort: Option<String>,
    /// Who put it in a team other than its folder's, if anyone did. One he placed stays where he put it.
    #[serde(default)]
    pub(super) moved: Option<Mover>,
    /// How many times it has reported. The tower looks at where it belongs on some of them.
    #[serde(default)]
    pub(super) reports: u32,
    /// How far into its harness's record the tower has read what he typed to it at its own keyboard.
    #[serde(default)]
    pub(super) told_upto: u64,
    /// For one the tower runs itself: what he has said to it while it was in the middle of something, by the
    /// id of the chat line and the words. Held here, where he can still take it back, until it stops.
    #[serde(default)]
    pub(super) queue: Vec<(String, String)>,
    /// Its own tool has listed it among its open sessions. One of these that the tool lists no longer has
    /// gone, whether or not it said so.
    #[serde(default)]
    pub(super) listed: bool,
    /// It has reported from outside the tower, by its tool's hooks. One that has not is heard only through
    /// its tool's own server, where it has one.
    #[serde(default)]
    pub(super) hooked: bool,
    /// The last turn of its own it was heard to end, by its tool's id for the turn, when the tool gives one.
    /// The same turn told a second time, by another way the tool has of saying it, is not taken again.
    #[serde(default)]
    pub(super) turn: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(super) enum Mover {
    Him,
    Tower,
}
impl Inner {
    /// The team an agent goes in, by id: the one it is in; where it was put before it went away; the team of the
    /// name it was started under; or its folder's team. A team found by the name is written down as where it
    /// was put, so the team keeps it when he renames the team.
    fn team_of(&mut self, agent: &Agent) -> Option<String> {
        if let Some(s) = self.find(&agent.id) {
            return Some(s.team.clone());
        }
        if let Some((team, _)) = self.placed.get(&agent.id).filter(|(team, _)| self.teams.contains_key(team)) {
            return Some(team.clone());
        }
        match (&agent.name, &agent.cwd) {
            (Some(name), _) => {
                let team = self.team_started_under(name)?;
                self.put(&agent.id, &team, Mover::Tower);
                Some(team)
            }
            (None, Some(cwd)) => self.folder_team(cwd),
            (None, None) => None,
        }
    }
    /// Write down which team an agent was put in, by whom. Only the latest few hundred are remembered.
    fn put(&mut self, id: &str, team: &str, by: Mover) {
        self.placed.insert(id.to_string(), (team.to_string(), by));
        while self.placed.len() > 400 {
            let Some(first) = self.placed.keys().next().cloned() else { break };
            self.placed.remove(&first);
        }
    }
    pub(super) fn remove(&mut self, agent_id: &str) {
        // it has gone: what it was waiting on him for goes with it, and what it reported is still to be heard
        self.drop_asks(agent_id, Dropped::AgentGone);
        self.sessions.retain(|s| s.agent.id != agent_id);
        for call in self.calls.values_mut() {
            if call.focus.as_deref() == Some(agent_id) {
                call.focus = None;
            }
        }
    }
    /// An agent reported in. A new one is given a callsign; a known one has what it reported about itself
    /// brought up to date, and takes the name he gave its session if that is new. Returns its callsign, or
    /// nothing for one no team can be found or made for: a team is called by the name the agent was started
    /// under, or by its folder's own name, and no name is made up for one with neither. It is not taken in.
    pub(super) fn check_in(&mut self, agent: &Agent) -> Option<Callsign> {
        // the team it is in, or the one it joins
        let Some(team) = self.team_of(agent) else {
            log(format!("an agent of {} was not taken in: no team can be found or made for it", agent.harness));
            return None;
        };
        let team_name = self.team_name(&team);
        // the name he gave the session, when it is new to the tower or has changed
        let wanted = agent.title.as_deref().filter(|t| self.find(&agent.id).is_none_or(|s| s.agent.title.as_deref() != Some(*t))).and_then(|t| Callsign::titled(&team_name, agent.cwd.as_deref().unwrap_or_default(), t));
        if let Some(want) = &wanted {
            self.make_room(&agent.id, want, &team_name);
        }
        if self.find(&agent.id).is_none() {
            let callsign = wanted.clone().filter(|c| self.callsign_free(c, Some(&agent.id))).unwrap_or_else(|| {
                self.next_free(&team_name, None, &[])
            });
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            self.sessions.push(Session { agent: agent.clone(), callsign, team: team.clone(), waiting: false, last_heard: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0, queue: Vec::new(), listed: false, hooked: false, turn: None });
            // one that was moved to another team before it went away is back there, by whoever put it there
            if let Some(by) = self.placed.get(&agent.id).filter(|(placed, _)| *placed == team).map(|(_, by)| *by)
                && let Some(s) = self.session(&agent.id) {
                    s.moved = Some(by);
                }
        } else if let Some(want) = wanted.filter(|c| self.callsign_free(c, Some(&agent.id))) {
            self.rename(&agent.id, want);
        }
        // one first heard of from its tool's list of open sessions comes without the name it was started
        // under; its own first report has it, and it goes to the team of that name
        if let Some(name) = agent.name.clone().filter(|_| self.find(&agent.id).is_some_and(|s| s.agent.name.is_none() && s.moved != Some(Mover::Him))) {
            self.session(&agent.id).unwrap().agent.name = Some(name.clone());
            if let Some(team) = self.team_started_under(&name) {
                self.move_to(&agent.id, &team, Mover::Tower);
            }
        }
        let s = self.session(&agent.id).unwrap();
        // an adapter that cannot see something on this event must not erase what an earlier one reported
        if agent.title.is_some() {
            s.agent.title = agent.title.clone();
        }
        if agent.deliver.is_some() {
            s.agent.deliver = agent.deliver.clone();
        }
        if agent.pid.is_some() {
            s.agent.pid = agent.pid;
        }
        if !agent.meta.is_empty() {
            s.agent.meta = agent.meta.clone();
        }
        Some(s.callsign.clone())
    }
    /// A session is about to take the callsign `want`. If another session holds it only because of the order it
    /// arrived in, that one moves to the next free number; one that was given the name by him keeps it.
    fn make_room(&mut self, id: &str, want: &Callsign, team_name: &str) {
        let Some(holder) = self.sessions.iter().find(|s| s.callsign == *want && s.agent.id != id).cloned() else { return };
        if holder.agent.title.as_deref().and_then(|t| Callsign::titled(team_name, holder.agent.cwd.as_deref().unwrap_or_default(), t)).as_ref() == Some(want) {
            return;
        }
        let to = self.next_free(team_name, Some(&holder.agent.id), std::slice::from_ref(want));
        self.rename(&holder.agent.id, to);
    }
    /// The callsigns in use: by every agent here but `except` (by id), and on his other computers.
    fn taken_callsigns(&self, except: Option<&str>) -> Vec<Callsign> {
        self.sessions.iter().filter(|s| Some(s.agent.id.as_str()) != except).map(|s| s.callsign.clone()).chain(self.elsewhere.iter().cloned()).collect()
    }
    /// Can the agent `except` (by id) be called this: no other agent here or on his other computers is?
    fn callsign_free(&self, callsign: &Callsign, except: Option<&str>) -> bool {
        !self.taken_callsigns(except).contains(callsign)
    }
    /// The next free numbered callsign in the team called `team_name`: free among the callsigns in use (but
    /// `except`'s), and none of `also`. The one rule for handing out a number.
    pub(super) fn next_free(&self, team_name: &str, except: Option<&str>, also: &[Callsign]) -> Callsign {
        let taken: Vec<Callsign> = self.taken_callsigns(except).into_iter().chain(also.iter().cloned()).collect();
        Callsign::next_in(team_name, &taken)
    }
    /// Put a session in another of his teams, by the team's id. Returns what it was called and what it is
    /// called now, or nothing when it is already there or there is no such team.
    pub(super) fn move_to(&mut self, id: &str, team: &str, by: Mover) -> Option<(Callsign, Callsign)> {
        let was = self.find(id)?.callsign.clone();
        let name = self.teams.get(team)?.name.clone();
        let to = was.moved_to(&name, &self.taken_callsigns(Some(id)))?;
        if self.find(id)?.team == team {
            // already there: if it was he who said so, it now stays
            if by == Mover::Him {
                self.session(id)?.moved = Some(Mover::Him);
            }
            return None;
        }
        self.rename(id, to.clone());
        let s = self.session(id)?;
        s.team = team.to_string();
        s.moved = Some(by);
        self.put(id, team, by);
        Some((was, to))
    }
    /// Call a session something else. Its messages are found by its id, so nothing has to be carried over;
    /// they are marked changed so every radio shows the new name.
    pub(super) fn rename(&mut self, id: &str, to: Callsign) {
        let Some(s) = self.session(id) else { return };
        if s.callsign == to {
            return;
        }
        log(format!("{} is now {}", s.callsign.text(), to.text()));
        s.callsign = to;
        let touched: Vec<String> = self.log.iter().filter(|e| e.agent.as_deref() == Some(id) || e.to_agents.iter().any(|a| a == id)).map(|e| e.id.clone()).collect();
        for entry in touched {
            self.touch(&entry);
        }
    }
    /// An agent carried on in a new process keeps the name it had, when nobody has taken it meanwhile. Renamed
    /// the way every rename is: written in the log, and every line of it sent to the radios again.
    pub(super) fn keep_name(&mut self, id: &str, name: Callsign) -> bool {
        if !self.callsign_free(&name, Some(id)) {
            return false;
        }
        self.rename(id, name);
        true
    }
    pub(super) fn callsigns(&self) -> Vec<Callsign> {
        self.sessions.iter().map(|s| s.callsign.clone()).collect()
    }
}
