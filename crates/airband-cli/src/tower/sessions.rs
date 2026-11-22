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
/// A folder as it really is on disk, links followed: two ways of writing one folder are one team. One that is
/// not there any more is read by its parts, as the system reads a path.
pub(super) fn real_folder(path: &str) -> String {
    let at = std::path::Path::new(path);
    at.canonicalize().unwrap_or_else(|_| at.components().collect()).display().to_string()
}
impl Inner {
    /// What the team with this id is called. Empty for an id that is no team of his.
    pub(super) fn team_name(&self, id: &str) -> String {
        self.teams.get(id).map(|t| t.name.clone()).unwrap_or_default()
    }
    /// The id of the team that works in this folder, if one does.
    pub(super) fn team_at(&self, folder: &str) -> Option<String> {
        let real = real_folder(folder);
        self.teams.iter().find(|(_, t)| t.folder.as_deref() == Some(real.as_str())).map(|(id, _)| id.clone())
    }
    /// The id of the team called this, if one is. Names are kept distinct, so there is at most one.
    pub(super) fn team_called(&self, name: &str) -> Option<String> {
        self.teams.iter().find(|(_, t)| t.name == name).map(|(id, _)| id.clone())
    }
    /// What a new team in this folder would be called: the plainest name from its path that no other team of
    /// his has. A name another team on this computer has takes the parent folder's name in front ("work shop");
    /// one only a team on another of his computers has takes this computer's name in front instead ("laptop
    /// shop"). Nothing when the path has no name in it (the top of a disk).
    pub(super) fn name_for_folder(&self, folder: &str) -> Option<String> {
        let real = real_folder(folder);
        let here = |name: &str| self.taken_here(name, Some(&real));
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
    /// Has a team on this computer this name, other than the team `except` (by id)?
    fn taken_here(&self, name: &str, except: Option<&str>) -> bool {
        self.teams.iter().any(|(id, t)| t.name == name && Some(id.as_str()) != except)
    }
    /// Has a team on another of his computers this name?
    fn taken_there(&self, name: &str) -> bool {
        self.elsewhere.iter().any(|c| c.project == name)
    }
    /// Can a team he names take this name: no other team of his, here (other than `except`, by id) or on
    /// another of his computers, has it?
    pub(super) fn name_free(&self, name: &str, except: Option<&str>) -> bool {
        !self.taken_here(name, except) && !self.taken_there(name)
    }
    /// The team for a folder: the one that works there, or a new one, named as [`Self::name_for_folder`] says.
    /// Its id. Nothing when the folder has no name to give a team.
    pub(super) fn folder_team(&mut self, folder: &str) -> Option<String> {
        if let Some(id) = self.team_at(folder) {
            return Some(id);
        }
        let name = self.name_for_folder(folder)?;
        let id = real_folder(folder);
        self.teams.insert(id.clone(), Team { name, folder: Some(id.clone()), ..Team::default() });
        Some(id)
    }
    /// The team he means by a name he gave (the name an agent was started under, a new team he moved one to):
    /// the team that has it, or a new one with no folder. Its id. Nothing for a name that says nothing.
    pub(super) fn named_team(&mut self, name: &str) -> Option<String> {
        let name = airband_core::routing::team_name(name)?;
        if let Some(id) = self.team_called(&name) {
            return Some(id);
        }
        let id = format!("team-{}", config::random_hex(8));
        self.teams.insert(id.clone(), Team { name, folder: None, named: true, configured: false });
        Some(id)
    }
    /// The team an agent goes in, by id: the one it is in, or the team of the name it was started under, or
    /// its folder's team.
    fn team_of(&mut self, agent: &Agent) -> Option<String> {
        if let Some(s) = self.find(&agent.id) {
            return Some(s.team.clone());
        }
        match (&agent.name, &agent.cwd) {
            (Some(name), _) => self.named_team(name),
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
        // exactly as `team_of` places it, so the two never disagree
        self.find(&agent.id).is_some()
            || match (&agent.name, &agent.cwd) {
                (Some(name), _) => airband_core::routing::team_name(name).is_some(),
                (None, Some(cwd)) => self.team_at(cwd).is_some() || self.name_for_folder(cwd).is_some(),
                (None, None) => false,
            }
    }
    /// An agent reported in. A new one is given a callsign; a known one has what it reported about itself
    /// brought up to date, and takes the name he gave its session if that is new. Returns its callsign.
    pub(super) fn check_in(&mut self, agent: &Agent) -> Callsign {
        // the team it is in, or the one it joins
        let team = self.team_of(agent).unwrap_or_default();
        let team_name = self.team_name(&team);
        // the name he gave the session, when it is new to the tower or has changed
        let wanted = agent.title.as_deref().filter(|t| self.find(&agent.id).is_none_or(|s| s.agent.title.as_deref() != Some(*t))).and_then(|t| Callsign::titled(&team_name, agent.cwd.as_deref().unwrap_or_default(), t));
        if let Some(want) = &wanted {
            self.make_room(&agent.id, want, &team_name);
        }
        let free = |inner: &Inner, c: &Callsign| !inner.sessions.iter().any(|s| s.callsign == *c);
        if self.find(&agent.id).is_none() {
            let callsign = wanted.clone().filter(|c| free(self, c)).unwrap_or_else(|| {
                self.next_free(&team_name, None, &[])
            });
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            self.sessions.push(Session { agent: agent.clone(), callsign, team: team.clone(), waiting: false, last_heard: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0, queue: Vec::new(), listed: false, hooked: false, turn: None });
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
            self.session(&agent.id).unwrap().agent.name = Some(name.clone());
            if let Some(team) = self.named_team(&name) {
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
        s.callsign.clone()
    }
    /// A session is about to take the callsign `want`. If another session holds it only because of the order it
    /// arrived in, that one moves to the next free number; one that was given the name by him keeps it.
    fn make_room(&mut self, id: &str, want: &Callsign, team: &str) {
        let Some(holder) = self.sessions.iter().find(|s| s.callsign == *want && s.agent.id != id).cloned() else { return };
        if holder.agent.title.as_deref().and_then(|t| Callsign::titled(team, holder.agent.cwd.as_deref().unwrap_or_default(), t)).as_ref() == Some(want) {
            return;
        }
        let to = self.next_free(team, Some(&holder.agent.id), std::slice::from_ref(want));
        self.rename(&holder.agent.id, to);
    }
    /// The next free numbered callsign in a team: free among the names in use here (but `except`'s) and on his
    /// other computers, and none of `also`. The one rule for handing out a number.
    pub(super) fn next_free(&self, team: &str, except: Option<&str>, also: &[Callsign]) -> Callsign {
        let taken: Vec<Callsign> = self.sessions.iter().filter(|s| Some(s.agent.id.as_str()) != except).map(|s| s.callsign.clone()).chain(self.elsewhere.iter().cloned()).chain(also.iter().cloned()).collect();
        Callsign::next_in(team, &taken)
    }
    /// Put a session in another of his teams, by the team's id. Returns what it was called and what it is
    /// called now, or nothing when it is already there or there is no such team.
    pub(super) fn move_to(&mut self, id: &str, team: &str, by: Mover) -> Option<(Callsign, Callsign)> {
        let was = self.find(id)?.callsign.clone();
        let name = self.teams.get(team)?.name.clone();
        let others: Vec<Callsign> = self.sessions.iter().filter(|s| s.agent.id != id).map(|s| s.callsign.clone()).chain(self.elsewhere.iter().cloned()).collect();
        let to = was.moved_to(&name, &others)?;
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
        self.placed.insert(id.to_string(), (team.to_string(), by));
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
        // a team here whose name a team on one of those computers has takes this computer's name in front; a
        // name he gave a team is his, and stays
        let theirs: BTreeSet<&str> = there.iter().map(|c| c.project.as_str()).collect();
        let clashing_teams: Vec<String> = self.teams.iter().filter(|(_, t)| theirs.contains(t.name.as_str()) && !t.named).map(|(id, _)| id.clone()).collect();
        let renamed = !clashing_teams.is_empty();
        for id in clashing_teams {
            let name = self.team_name(&id);
            let on = airband_core::routing::team_on(&self.here, &name);
            let to = if self.name_free(&on, Some(&id)) {
                on
            } else {
                // this computer's name is not enough: the folder's own path, as for any new team
                self.teams.get(&id).and_then(|t| t.folder.clone()).and_then(|f| self.name_for_folder(&f)).unwrap_or(name)
            };
            self.set_team_name(&id, &to);
        }
        let clashing: Vec<String> = self.sessions.iter().filter(|s| s.callsign.numbered() && there.contains(&s.callsign)).map(|s| s.agent.id.clone()).collect();
        for id in &clashing {
            let Some(project) = self.find(id).map(|s| s.callsign.project.clone()) else { continue };
            let free = self.next_free(&project, None, &[]);
            self.rename(id, free);
        }
        renamed || !clashing.is_empty()
    }
    /// He renamed a team, by its id. Only what it is called changes: its agents, its folder and its colour stay
    /// with it. Returns the name it now has.
    pub(super) fn rename_team(&mut self, team: &str, to: &str) -> Result<String, String> {
        let to = airband_core::routing::chosen_team_name(to)?;
        let Some(was) = self.teams.get(team).map(|t| t.name.clone()) else {
            return Err("That team is not on this computer any more.".into());
        };
        if to != was && !self.name_free(&to, Some(team)) {
            return Err(format!("There is already a team called {to}, here or on another of your computers."));
        }
        self.set_team_name(team, &to);
        if let Some(t) = self.teams.get_mut(team) {
            t.named = true;
        }
        log(format!("the team {was} is now {to}"));
        Ok(to)
    }
    /// A team is called something else: its agents take the new name, the numbers and the names he gave them
    /// staying as they were, and every line of the team is sent to the radios again.
    fn set_team_name(&mut self, team: &str, to: &str) {
        let Some(t) = self.teams.get_mut(team) else { return };
        if t.name == to {
            return;
        }
        t.name = to.to_string();
        let ids: Vec<(String, Callsign)> = self.sessions.iter().filter(|s| s.team == team).map(|s| (s.agent.id.clone(), Callsign { project: to.to_string(), ..s.callsign.clone() })).collect();
        for (id, callsign) in ids {
            self.rename(&id, callsign);
        }
        let lines: Vec<String> = self.log.iter().filter(|e| e.team == team).map(|e| e.id.clone()).collect();
        for line in lines {
            self.touch(&line);
        }
    }
    /// The teams the configuration lists, by name with their folders: each is his team in that folder, called
    /// that unless he has renamed it since. One whose name says nothing, or that another team already has, is
    /// left out and said in the log.
    pub(super) fn configure_teams(&mut self, listed: &BTreeMap<String, String>) {
        for (name, folder) in listed {
            let Some(name) = airband_core::routing::team_name(name) else {
                log(format!("the configured team \"{name}\" is left out: its name has no letters or numbers"));
                continue;
            };
            let id = real_folder(folder);
            if self.taken_here(&name, Some(&id)) {
                log(format!("the configured team {name} is left out: another team is called that"));
                continue;
            }
            let team = self.teams.entry(id.clone()).or_insert(Team { name: name.clone(), folder: Some(id), named: false, configured: true });
            team.configured = true;
            if !team.named {
                team.name = name;
            }
        }
    }
    /// Teams go by id. State kept by a tower from before that (teams by name, `old`, with the names he gave,
    /// `named`) is read into teams once; then each agent and each line goes by its team's id, each agent nobody
    /// moved goes back to its own folder's team, and each kept folder is written as it really is on disk.
    pub(super) fn settle_teams(&mut self, old: BTreeMap<String, String>, named: BTreeSet<String>) {
        for (name, folder) in old {
            let id = real_folder(&folder);
            let is_named = named.contains(&name);
            self.teams.entry(id.clone()).or_insert(Team { name, folder: Some(id), named: is_named, configured: false });
        }
        let unteamed: Vec<(String, String)> = self.sessions.iter().filter(|s| !self.teams.contains_key(&s.team)).map(|s| (s.agent.id.clone(), s.callsign.project.clone())).collect();
        for (id, name) in unteamed {
            let team = self.team_called(&name).or_else(|| self.named_team(&name)).unwrap_or_default();
            if let Some(s) = self.session(&id) {
                s.team = team;
            }
        }
        let placed: Vec<(String, String, Mover)> = self.placed.iter().filter(|(_, (team, _))| !self.teams.contains_key(team)).map(|(a, (t, by))| (a.clone(), t.clone(), *by)).collect();
        for (agent, name, by) in placed {
            match self.team_called(&name) {
                Some(team) => {
                    self.placed.insert(agent, (team, by));
                }
                None => {
                    self.placed.remove(&agent);
                }
            }
        }
        let by_name: BTreeMap<String, String> = self.teams.iter().map(|(id, t)| (t.name.clone(), id.clone())).collect();
        for e in self.log.iter_mut().filter(|e| !e.team.is_empty() && !self.teams.contains_key(&e.team)) {
            if let Some(id) = by_name.get(&e.team) {
                e.team = id.clone();
                self.unsaved.borrow_mut().insert(e.id.clone());
            }
        }
        let strays: Vec<(String, String)> = self.sessions.iter().filter(|s| s.moved.is_none() && s.agent.name.is_none()).filter_map(|s| s.agent.cwd.clone().map(|cwd| (s.agent.id.clone(), cwd))).collect();
        for (id, cwd) in strays {
            if let Some(team) = self.folder_team(&cwd) {
                self.move_to(&id, &team, Mover::Tower);
                if let Some(s) = self.session(&id) {
                    s.moved = None;
                }
            }
        }
    }
}
