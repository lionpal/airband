//! The agents on the radio: who each one is, what it is called and which team it is in, and how one joins,
//! is renamed, moves between teams and leaves.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Session {
    pub(super) agent: Agent,
    pub(super) callsign: Callsign,
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
/// A folder as it really is on disk, links followed: two ways of writing one folder are one team. One that is
/// not there any more is read by its parts, as the system reads a path.
pub(super) fn real_folder(path: &str) -> String {
    let at = std::path::Path::new(path);
    at.canonicalize().unwrap_or_else(|_| at.components().collect()).display().to_string()
}
impl Inner {
    /// The team the agents in a folder go in: the one that folder already has, or else the plainest name from
    /// its path that no other team of his has. A name another folder's team on this computer has takes the
    /// parent folder's name in front ("work shop"); one only a team on another of his computers has takes this
    /// computer's name in front instead ("laptop shop"). Nothing when the path has no name in it (the top of a
    /// disk). Nothing here is kept: `check_in` keeps the team it gives.
    pub(super) fn folder_team(&self, folder: &str) -> Option<String> {
        if let Some(team) = self.team_at(folder) {
            return Some(team);
        }
        let real = real_folder(folder);
        let here = |name: &str| self.taken_here(name, &real);
        let there = |name: &str| self.taken_there(name);
        let names = airband_core::routing::team_names(&real);
        let plainest = names.first()?;
        if !here(plainest) && there(plainest) {
            let on = airband_core::routing::team_on(&self.here, plainest);
            if !here(&on) && !there(&on) {
                return Some(on);
            }
        }
        names.iter().find(|n| !here(n) && !there(n)).cloned().or_else(|| names.last().map(|n| airband_core::routing::team_on(&self.here, n)))
    }
    /// Has a team on this computer this name, other than the one in this folder: another folder's, or one with
    /// no folder (a name he gave, a team an agent was moved to)?
    fn taken_here(&self, name: &str, real: &str) -> bool {
        match self.projects.get(name).or(self.configured.get(name)) {
            Some(f) => real_folder(f) != real,
            None => self.sessions.iter().any(|s| s.callsign.project == name),
        }
    }
    /// Has a team on another of his computers this name?
    fn taken_there(&self, name: &str) -> bool {
        self.elsewhere.iter().any(|c| c.project == name)
    }
    /// The team that works in this folder, if one does.
    pub(super) fn team_at(&self, folder: &str) -> Option<String> {
        let real = real_folder(folder);
        self.projects.iter().chain(self.configured.iter()).find(|(_, f)| real_folder(f) == real).map(|(name, _)| name.clone())
    }
    /// The team an agent goes in: the one it is in, or the name it was started under, or its folder's team.
    fn team_of(&self, agent: &Agent) -> Option<String> {
        if let Some(s) = self.find(&agent.id) {
            return Some(s.callsign.project.clone());
        }
        match (&agent.name, &agent.cwd) {
            (Some(name), _) => airband_core::routing::team_name(name),
            (None, Some(cwd)) => self.folder_team(cwd),
            (None, None) => None,
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
    /// Can this agent be put in a team? A team is called by the name the agent was started under, or by its
    /// folder's own name. One with neither is turned away and its tool told why: no name is made up for it.
    pub(super) fn placeable(&self, agent: &Agent) -> bool {
        self.find(&agent.id).is_some() || agent.name.as_deref().or(agent.cwd.as_deref()).and_then(airband_core::routing::team_name).is_some()
    }
    /// An agent reported in. A new one is given a callsign; a known one has what it reported about itself
    /// brought up to date, and takes the name he gave its session if that is new. Returns its callsign.
    pub(super) fn check_in(&mut self, agent: &Agent) -> Callsign {
        // the team it is in, or the one it joins
        let team = self.team_of(agent).unwrap_or_default();
        // the name he gave the session, when it is new to the tower or has changed
        let wanted = agent.title.as_deref().filter(|t| self.find(&agent.id).is_none_or(|s| s.agent.title.as_deref() != Some(*t))).and_then(|t| Callsign::titled(&team, agent.cwd.as_deref().unwrap_or_default(), t));
        if let Some(want) = &wanted {
            self.make_room(&agent.id, want, &team);
        }
        let free = |inner: &Inner, c: &Callsign| !inner.sessions.iter().any(|s| s.callsign == *c);
        if self.find(&agent.id).is_none() {
            let callsign = wanted.clone().filter(|c| free(self, c)).unwrap_or_else(|| {
                // the next free number, among the names in use here and on his other computers
                let mut taken = self.callsigns();
                let mut c = Callsign::next_in(&team, &taken);
                while self.elsewhere.contains(&c) {
                    taken.push(c.clone());
                    c = Callsign::next_in(&team, &taken);
                }
                c
            });
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            // the folder's team is kept, so its agents stay together and a new one can be launched there later
            if let Some(cwd) = agent.cwd.as_deref().filter(|_| agent.name.is_none()) {
                self.projects.entry(team.clone()).or_insert_with(|| real_folder(cwd));
            }
            self.sessions.push(Session { agent: agent.clone(), callsign, waiting: false, last_heard: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0, queue: Vec::new(), listed: false, hooked: false, turn: None });
            // one that was moved to another team before it went away goes back there
            if let Some((team, by)) = self.placed.get(&agent.id).cloned() {
                self.move_to(&agent.id, &team, by);
            }
        } else if let Some(want) = wanted.filter(|c| free(self, c)) {
            self.rename(&agent.id, want);
        }
        // one first heard of from its tool's list of open sessions comes without the name it was started
        // under; its own first report has it, and it goes to the team of that name
        if let Some(name) = agent.name.clone().filter(|_| self.find(&agent.id).is_some_and(|s| s.agent.name.is_none() && s.moved != Some(Mover::Him))) {
            let team = airband_core::routing::team_name(&name).unwrap_or_default();
            self.session(&agent.id).unwrap().agent.name = Some(name);
            self.move_to(&agent.id, &team, Mover::Tower);
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
        s.callsign.clone()
    }
    /// A session is about to take the callsign `want`. If another session holds it only because of the order it
    /// arrived in, that one moves to the next free number; one that was given the name by him keeps it.
    fn make_room(&mut self, id: &str, want: &Callsign, team: &str) {
        let Some(holder) = self.sessions.iter().find(|s| s.callsign == *want && s.agent.id != id).cloned() else { return };
        if holder.agent.title.as_deref().and_then(|t| Callsign::titled(team, holder.agent.cwd.as_deref().unwrap_or_default(), t)).as_ref() == Some(want) {
            return;
        }
        let mut taken = self.callsigns();
        taken.push(want.clone());
        self.rename(&holder.agent.id, Callsign::next_in(team, &taken));
    }
    /// Put a session in another team, one that exists or a new one. Returns what it was called and what it is
    /// called now, or nothing when it is already there or the team's name says nothing.
    pub(super) fn move_to(&mut self, id: &str, team: &str, by: Mover) -> Option<(Callsign, Callsign)> {
        let was = self.find(id)?.callsign.clone();
        let others: Vec<Callsign> = self.sessions.iter().filter(|s| s.agent.id != id).map(|s| s.callsign.clone()).collect();
        let to = was.moved_to(team, &others)?;
        if to.project == was.project {
            // already there: if it was he who said so, it now stays
            if by == Mover::Him {
                self.session(id)?.moved = Some(Mover::Him);
            }
            return None;
        }
        self.rename(id, to.clone());
        self.session(id)?.moved = Some(by);
        self.placed.insert(id.to_string(), (to.project.clone(), by));
        // only the latest few hundred are remembered
        while self.placed.len() > 400 {
            let Some(first) = self.placed.keys().next().cloned() else { break };
            self.placed.remove(&first);
        }
        Some((was, to))
    }
    /// Call a session something else. Its messages are found by its id, so nothing has to be carried over;
    /// they are marked changed so every radio shows the new name.
    fn rename(&mut self, id: &str, to: Callsign) {
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
        if self.sessions.iter().any(|s| s.agent.id != id && s.callsign == name) {
            return false;
        }
        self.rename(id, name);
        true
    }
    pub(super) fn callsigns(&self) -> Vec<Callsign> {
        self.sessions.iter().map(|s| s.callsign.clone()).collect()
    }
    /// A radio said which names his other computers use. Any agent here that was given one of them by number
    /// takes the next free number. Returns whether anything was renamed.
    pub(super) fn taken_elsewhere(&mut self, there: &[Callsign]) -> bool {
        self.elsewhere = there.to_vec();
        // a folder's team here whose name a team on one of those computers has takes this computer's name in
        // front; a name he gave a team is his, and stays
        let theirs: BTreeSet<&str> = there.iter().map(|c| c.project.as_str()).collect();
        let clashing_teams: Vec<(String, String)> = self.projects.iter().filter(|(name, _)| theirs.contains(name.as_str()) && !self.named_teams.contains(*name)).map(|(n, f)| (n.clone(), f.clone())).collect();
        let renamed = !clashing_teams.is_empty();
        for (name, folder) in clashing_teams {
            let real = real_folder(&folder);
            let on = airband_core::routing::team_on(&self.here, &name);
            let to = if !self.taken_here(&on, &real) && !self.taken_there(&on) {
                on
            } else {
                // this computer's name is not enough: the folder's own path, as for any new team
                self.projects.remove(&name);
                let to = self.folder_team(&folder).unwrap_or_else(|| name.clone());
                self.projects.insert(name.clone(), folder.clone());
                to
            };
            self.projects.remove(&name);
            self.projects.insert(to.clone(), folder);
            self.rename_team_to(&name, &to);
        }
        let clashing: Vec<String> = self.sessions.iter().filter(|s| s.callsign.numbered() && there.contains(&s.callsign)).map(|s| s.agent.id.clone()).collect();
        for id in &clashing {
            let Some(project) = self.find(id).map(|s| s.callsign.project.clone()) else { continue };
            // a name nobody has, here or there
            let mut n = 1;
            let free = loop {
                let candidate = Callsign { project: project.clone(), number: airband_core::routing::number_name(n), named: false };
                if !there.contains(&candidate) && !self.sessions.iter().any(|s| s.callsign == candidate) {
                    break candidate;
                }
                n += 1;
            };
            self.rename(id, free);
        }
        renamed || !clashing.is_empty()
    }
    /// He renamed a team. Only what it is called changes: its agents, its folder and its colour stay with it.
    /// Returns the name it now has.
    pub(super) fn rename_team(&mut self, team: &str, to: &str) -> Result<String, String> {
        let Some(to) = airband_core::routing::team_name(to).filter(|n| n.chars().count() <= 24) else {
            return Err("A team\u{2019}s name has to say something, and be short.".into());
        };
        if !self.projects.contains_key(team) && !self.sessions.iter().any(|s| s.callsign.project == team) {
            return Err(format!("There is no team called {team}."));
        }
        if to != team && (self.projects.contains_key(&to) || self.configured.contains_key(&to) || self.sessions.iter().any(|s| s.callsign.project == to)) {
            return Err(format!("There is already a team called {to}."));
        }
        if let Some(folder) = self.projects.remove(team) {
            self.projects.insert(to.clone(), folder);
        }
        self.rename_team_to(team, &to);
        self.named_teams.insert(to.clone());
        log(format!("the team {team} is now {to}"));
        Ok(to)
    }
    /// Every agent of a team, and what is kept by its name, goes under another name: the numbers and the names
    /// he gave them stay as they were.
    fn rename_team_to(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let ids: Vec<(String, Callsign)> = self.sessions.iter().filter(|s| s.callsign.project == from).map(|s| (s.agent.id.clone(), Callsign { project: to.to_string(), ..s.callsign.clone() })).collect();
        for (id, callsign) in ids {
            self.rename(&id, callsign);
        }
        for (team, _) in self.placed.values_mut() {
            if team == from {
                *team = to.to_string();
            }
        }
        if self.named_teams.remove(from) {
            self.named_teams.insert(to.to_string());
        }
    }
    /// Teams go by folder. Kept state from before that could have put two folders that read alike in one team:
    /// each agent that nobody moved goes back to its own folder's team, and each kept folder is written as it
    /// really is on disk.
    pub(super) fn settle_teams(&mut self) {
        for folder in self.projects.values_mut() {
            *folder = real_folder(folder);
        }
        let strays: Vec<(String, String)> = self.sessions.iter().filter(|s| s.moved.is_none() && s.agent.name.is_none()).filter_map(|s| s.agent.cwd.clone().map(|cwd| (s.agent.id.clone(), cwd))).collect();
        for (id, cwd) in strays {
            let Some(team) = self.folder_team(&cwd) else { continue };
            self.projects.entry(team.clone()).or_insert_with(|| real_folder(&cwd));
            let Some(was) = self.find(&id).map(|s| s.callsign.clone()).filter(|c| c.project != team) else { continue };
            let others: Vec<Callsign> = self.sessions.iter().filter(|s| s.agent.id != id).map(|s| s.callsign.clone()).collect();
            if let Some(to) = was.moved_to(&team, &others) {
                self.rename(&id, to);
            }
        }
    }
}
