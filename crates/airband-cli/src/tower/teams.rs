//! His teams as places to start agents in: the ones with a folder, the folders his coding tools have lately
//! worked in, and the names a new agent can be started by.
use super::*;

impl Inner {
    /// His teams that have a folder, by what each is called, with the folder: what the configuration lists
    /// and wherever agents have been seen. A team whose folder has gone is not offered.
    pub(super) fn folder_teams(&self) -> BTreeMap<String, String> {
        self.teams.values().filter_map(|t| t.folder.clone().filter(|f| std::path::Path::new(f).is_dir()).map(|f| (t.name.clone(), f))).collect()
    }
    /// Where a new agent can be started by name: his teams, then the folders his coding tools have lately
    /// had sessions in, each by the name a team there would have. A team's name is never taken from it.
    pub(super) fn launchable(&self) -> BTreeMap<String, String> {
        let mut all = self.folder_teams();
        for recent in self.recent().into_iter().filter(|r| !r.team && !r.name.is_empty()).take(20) {
            all.entry(recent.name).or_insert(recent.folder);
        }
        all
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
                let team = self.team_at(&folder).and_then(|id| self.teams.get(&id)).map(|t| t.name.clone());
                airband_core::radio::RecentFolder { shown: shown(std::path::Path::new(&folder)), name: team.clone().or_else(|| self.name_for_folder(&folder)).unwrap_or_default(), team: team.is_some(), tools, at, folder }
            })
            .collect();
        for (name, folder) in teams {
            if !all.iter().any(|r| real_folder(&r.folder) == real_folder(&folder)) {
                all.push(airband_core::radio::RecentFolder { shown: shown(std::path::Path::new(&folder)), name, team: true, tools: Vec::new(), at: 0, folder });
            }
        }
        all
    }
}
