//! His teams: the table of them by id, what each is called and how a name is chosen, how one is made, renamed and
//! settled at the start, and the listings of the places a new agent can be started in.
use super::*;

/// A folder as it really is on disk, links followed: two ways of writing one folder are one team. One that is
/// not there any more is read by its parts, as the system reads a path.
pub(super) fn real_folder(path: &str) -> String {
    let at = std::path::Path::new(path);
    at.canonicalize().unwrap_or_else(|_| at.components().collect()).display().to_string()
}

/// A place a new agent can be started in: what it is called, and its folder. Its team is the folder's, made
/// when an agent is started there if there is none yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Place {
    pub(super) name: String,
    pub(super) folder: String,
}

/// One folder in the recent list, shown as he would read its path.
fn recent_folder(folder: String, name: String, team: bool, tools: Vec<String>, at: u64) -> airband_core::radio::RecentFolder {
    airband_core::radio::RecentFolder { shown: shown(std::path::Path::new(&folder)), name, team, tools, at, folder }
}

impl Inner {
    /// What the team with this id is called. Empty for an id that is no team of his.
    pub(super) fn team_name(&self, id: &str) -> Option<&str> {
        self.teams.get(id).map(|t| t.name.as_str())
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
    pub(super) fn name_for_folder(&self, folder: &str) -> Option<TeamName> {
        let real = real_folder(folder);
        let here = |name: &TeamName| self.taken_here(name.as_str(), Some(&real));
        let there = |name: &TeamName| self.taken_there(name.as_str());
        let names = airband_core::routing::team_names(&real);
        let plainest = names.first()?;
        if !here(plainest) && there(plainest) {
            let on = airband_core::routing::team_on(&self.here, plainest);
            if !here(&on) && !there(&on) {
                return Some(on);
            }
        }
        // every name the folder's path gives, then each with this computer's name in front; the first that no
        // other team of his has. None when all are taken: the caller says so, and no clash is kept.
        let on_here: Vec<TeamName> = names.iter().map(|n| airband_core::routing::team_on(&self.here, n)).collect();
        names.iter().chain(on_here.iter()).find(|n| !here(n) && !there(n)).cloned()
    }
    /// Has a team on this computer this name, other than the team `except` (by id)?
    fn taken_here(&self, name: &str, except: Option<&str>) -> bool {
        self.teams.iter().any(|(id, t)| t.name == name && Some(id.as_str()) != except)
    }
    /// Has a team on another of his computers this name?
    fn taken_there(&self, name: &str) -> bool {
        self.elsewhere.iter().any(|c| c.project == name)
    }
    /// Can a team take this name: no other team of his, here (other than `except`, by id) or on another of his
    /// computers, has it?
    fn name_free(&self, name: &str, except: Option<&str>) -> bool {
        !self.taken_here(name, except) && !self.taken_there(name)
    }
    /// A name chosen for a team (by him, by a model, in the configuration) as it is kept: written as a callsign
    /// writes it, short, and no other team's, here (but `except`'s) or on another of his computers. The one
    /// check every chosen name goes through; `Err` says why it cannot be.
    pub(super) fn chosen_team(&self, name: &str, except: Option<&str>) -> Result<TeamName, String> {
        let name = TeamName::new(name)?;
        if self.name_free(name.as_str(), except) { Ok(name) } else { Err(format!("There is already a team called {name}, here or on another of your computers.")) }
    }
    /// A new team in a folder, called `name`, with `named` when he chose that name. Its id: the folder as it
    /// really is. The one way a folder's team is made.
    pub(super) fn new_folder_team(&mut self, folder: &str, name: TeamName, named: bool) -> String {
        let id = real_folder(folder);
        self.teams.insert(id.clone(), Team { name, folder: Some(id.clone()), named });
        id
    }
    /// The team for a folder: the one that works there, or a new one, named as [`Self::name_for_folder`] says.
    /// Its id. Nothing when the folder has no name to give a team.
    pub(super) fn folder_team(&mut self, folder: &str) -> Option<String> {
        if let Some(id) = self.team_at(folder) {
            return Some(id);
        }
        let name = self.name_for_folder(folder)?;
        Some(self.new_folder_team(folder, name, false))
    }
    /// A new team with no folder, called `name`, with `named` when he chose that name. A name a team already
    /// has is refused: a new team is never an old one by another way. Its id.
    pub(super) fn new_team(&mut self, name: &str, named: bool) -> Result<String, String> {
        let name = self.chosen_team(name, None)?;
        let id = format!("team-{}", config::random_hex(8));
        self.teams.insert(id.clone(), Team { name, folder: None, named });
        Ok(id)
    }
    /// The team he means by a name he gave (the name an agent was started under, a team he moved one to from a
    /// radio): the team on this computer that has exactly that name, or a new one with no folder, his. `Err`
    /// says why it can be neither.
    pub(super) fn team_he_named(&mut self, name: &str) -> Result<String, String> {
        let name = TeamName::new(name)?;
        match self.team_called(name.as_str()) {
            Some(id) => Ok(id),
            None => self.new_team(name.as_str(), true),
        }
    }
    /// A radio said which names his other computers use. A team here whose name a team there has takes another
    /// unless he gave it the name, and any agent here given one of those callsigns by number takes the next free
    /// number. Returns whether anything was renamed.
    pub(super) fn taken_elsewhere(&mut self, there: &[Callsign]) -> bool {
        self.elsewhere = there.to_vec();
        let theirs: BTreeSet<&str> = there.iter().map(|c| c.project.as_str()).collect();
        let clashing_teams: Vec<String> = self.teams.iter().filter(|(_, t)| theirs.contains(t.name.as_str()) && !t.named).map(|(id, _)| id.clone()).collect();
        let mut renamed = false;
        for id in clashing_teams {
            let Some(name) = self.teams.get(&id).map(|t| t.name.clone()) else { continue };
            let on = airband_core::routing::team_on(&self.here, &name);
            let to = if self.name_free(on.as_str(), Some(&id)) {
                Some(on)
            } else {
                // this computer's name is not enough: the folder's own path, as for any new team
                self.teams.get(&id).and_then(|t| t.folder.clone()).and_then(|f| self.name_for_folder(&f))
            };
            match to {
                Some(to) => {
                    self.set_team_name(&id, &to);
                    renamed = true;
                }
                None => log(format!("the team {name} has the name of a team on another computer, and no other name is free for it: rename one of them")),
            }
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
    pub(super) fn rename_team(&mut self, team: &str, to: &str) -> Result<TeamName, String> {
        let Some(was) = self.teams.get(team).map(|t| t.name.clone()) else {
            return Err("That team is not on this computer any more.".into());
        };
        let to = self.chosen_team(to, Some(team))?;
        self.set_team_name(team, &to);
        if let Some(t) = self.teams.get_mut(team) {
            t.named = true;
        }
        log(format!("the team {was} is now {to}"));
        Ok(to)
    }
    /// A team is called something else: its agents take the new name, the numbers and the names he gave them
    /// staying as they were, and every line of the team is sent to the radios again.
    fn set_team_name(&mut self, team: &str, to: &TeamName) {
        let Some(t) = self.teams.get_mut(team) else { return };
        if &t.name == to {
            return;
        }
        t.name = to.clone();
        // each keeps its number or the name he gave it under the new name when nobody else has that, else the next free
        let ids: Vec<String> = self.sessions.iter().filter(|s| s.team == team).map(|s| s.agent.id.clone()).collect();
        for id in ids {
            let Some(was) = self.find(&id).map(|s| s.callsign.clone()) else { continue };
            let same = Callsign { project: to.to_string(), ..was };
            let callsign = if self.callsign_free(&same, Some(&id)) { same } else { self.next_free(to.as_str(), Some(&id), &[]) };
            self.rename(&id, callsign);
        }
        let lines: Vec<String> = self.log.iter().filter(|e| e.team == team).map(|e| e.id.clone()).collect();
        for line in lines {
            self.touch(&line);
        }
    }
    /// His teams as a start finds them: what an older tower kept (`old`, `named`) settled first, so a name he
    /// gave a team the configuration lists stays his; then the configuration's teams (`listed`).
    pub(super) fn teams_at_start(&mut self, old: BTreeMap<String, String>, named: BTreeSet<String>, listed: &BTreeMap<String, String>) {
        self.settle_teams(old, named);
        self.configure_teams(listed);
    }
    /// The teams the configuration lists, by name with their folders: each is his team in that folder. The name
    /// is given when the team is made; a team the tower already keeps keeps the name it has, whether he gave it
    /// or the tower chose it to stay distinct from his other computers' teams. One whose name cannot be a team's,
    /// or that another team already has, is left out and said in the log.
    pub(super) fn configure_teams(&mut self, listed: &BTreeMap<String, String>) {
        for (name, folder) in listed {
            let id = real_folder(folder);
            if self.teams.contains_key(&id) {
                continue;
            }
            match self.chosen_team(name, Some(&id)) {
                Ok(name) => {
                    self.new_folder_team(folder, name, false);
                }
                Err(why) => log(format!("the configured team \"{name}\" is left out: {why}")),
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
            let Ok(name) = TeamName::new(&name) else {
                log(format!("a kept team called {name} is not read: a team cannot have that name"));
                continue;
            };
            match self.teams.get_mut(&id) {
                // a team already there takes the name he gave it
                Some(team) if is_named => {
                    team.name = name;
                    team.named = true;
                }
                Some(_) => {}
                None => {
                    self.new_folder_team(&folder, name, is_named);
                }
            }
        }
        // an agent kept with no team id goes in the team of its name; with none, its folder's; with neither it is
        // let go, and said
        let unteamed: Vec<(String, String, Option<String>)> = self.sessions.iter().filter(|s| !self.teams.contains_key(&s.team)).map(|s| (s.agent.id.clone(), s.callsign.project.clone(), s.agent.cwd.clone())).collect();
        for (id, name, cwd) in unteamed {
            match self.team_called(&name).or_else(|| cwd.and_then(|cwd| self.folder_team(&cwd))) {
                Some(team) => {
                    if let Some(s) = self.session(&id) {
                        s.team = team;
                    }
                }
                None => {
                    log(format!("a kept agent called {name} is let go: it has no team and no folder to give it one"));
                    self.sessions.retain(|s| s.agent.id != id);
                }
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
        let by_name: BTreeMap<String, String> = self.teams.iter().map(|(id, t)| (t.name.to_string(), id.clone())).collect();
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
    /// His teams that have a folder, by what each is called, with the folder: what the configuration lists
    /// and wherever agents have been seen. A team whose folder has gone is not offered.
    pub(super) fn folder_teams(&self) -> BTreeMap<String, String> {
        self.teams.values().filter_map(|t| t.live_folder().map(|f| (t.name.to_string(), f))).collect()
    }
    /// What a folder is called in his lists: by the team that works there, or by the name one would have, and
    /// whether there is one. Nothing for a folder with no name to give a team.
    pub(super) fn folder_called(&self, folder: &str) -> (Option<String>, bool) {
        match self.team_at(folder).and_then(|id| self.teams.get(&id)) {
            Some(team) => (Some(team.name.to_string()), true),
            None => (self.name_for_folder(folder).map(TeamName::into_string), false),
        }
    }
    /// Where a new agent can be started: his teams, then the folders his coding tools have lately had sessions
    /// in, each by the name a team there would have. Every folder is listed once; one whose name another place
    /// in the list already has is called by where it is instead, so none is left out and none is taken for another.
    pub(super) fn launchable(&self) -> Vec<Place> {
        let mut all: Vec<Place> = self.folder_teams().into_iter().map(|(name, folder)| Place { name, folder }).collect();
        for recent in self.recent().into_iter().filter(|r| !r.team && !r.name.is_empty()).take(20) {
            if all.iter().any(|p| real_folder(&p.folder) == real_folder(&recent.folder)) {
                continue;
            }
            let name = if all.iter().any(|p| p.name == recent.name) { recent.shown } else { recent.name };
            all.push(Place { name, folder: recent.folder });
        }
        all
    }
    /// The place of one of his teams, by its id, when it has a folder that is there.
    pub(super) fn team_place(&self, id: &str) -> Option<Place> {
        let team = self.teams.get(id)?;
        team.live_folder().map(|folder| Place { name: team.name.to_string(), folder })
    }
    /// The folders his coding tools have had sessions in, newest first, with his teams: one that has a team
    /// is called by it, and a team no tool has a record of comes last.
    pub(super) fn recent(&self) -> Vec<airband_core::radio::RecentFolder> {
        let teams = self.folder_teams();
        let mut all: Vec<airband_core::radio::RecentFolder> = self
            .worked
            .iter()
            .take(60)
            .cloned()
            .map(|(folder, tools, at)| {
                let (name, team) = self.folder_called(&folder);
                recent_folder(folder, name.unwrap_or_default(), team, tools, at)
            })
            .collect();
        for (name, folder) in teams {
            if !all.iter().any(|r| real_folder(&r.folder) == real_folder(&folder)) {
                all.push(recent_folder(folder, name, true, Vec::new(), 0));
            }
        }
        all
    }
}
