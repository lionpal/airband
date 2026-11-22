//! What a host can do with an agent or a computer: start one, move it, stop it, end it, and the accounts it runs under.
use super::*;

#[uniffi::export]
impl Desk {
    /// The computers a new agent of this team can be started on: the ones where the team already has agents,
    /// the one with the most of them first. That first one is where a new agent goes unless he says otherwise.
    pub fn computers_for(&self, room: String) -> Vec<String> {
        let state = self.shared.state.lock().unwrap();
        let mut found: Vec<(usize, &Station)> = state.stations.iter().map(|s| (s.agents.iter().filter(|a| a.group == room).count(), s)).filter(|(n, _)| *n > 0).collect();
        found.sort_by_key(|a| std::cmp::Reverse(a.0));
        found.into_iter().map(|(_, s)| s.name.clone()).collect()
    }
    /// The folders inside one of his on a computer, for choosing where a new team works. `computer` is its
    /// name, or empty when he has only one; `under` is a folder's whole path, or empty for his
    /// home folder. Blocks: call it off the main thread.
    pub fn folders(&self, computer: String, under: String) -> Result<DeskFolders, RadioError> {
        let f = self.station_radio(&computer)?.folders(under)?;
        Ok(DeskFolders { path: f.path, shown: f.shown, up: f.up, folders: f.folders, team: f.team, has_team: f.has_team })
    }
    /// The folders the coding tools on a computer have had sessions in, newest first, then the teams there
    /// that no tool has a record of. `computer` is its name, or empty when he has only one. Blocks: call it
    /// off the main thread.
    pub fn recent_folders(&self, computer: String) -> Result<Vec<DeskRecent>, RadioError> {
        let tools = airband_core::catalog::current();
        let called = |id: &String| tools.tools.iter().find(|t| t.id == *id).map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
        Ok(self.station_radio(&computer)?.recent()?.folders.into_iter().map(|r| DeskRecent { tools: r.tools.iter().map(called).collect(), folder: r.folder, shown: r.shown, name: r.name, team: r.team, at: r.at }).collect())
    }
    /// Make a new team on a computer, or start another agent in one that is there: its name, and the folder
    /// its agents work in (made first with `make`).
    /// Its first agent is started there with nothing to do yet. Blocks: call it off the main thread.
    /// `tool` is the coding tool its first agent is, by id; nothing is the computer's usual one.
    /// Answers with the team's name as that computer writes it.
    pub fn new_team(&self, computer: String, name: String, folder: String, make: bool, tool: Option<String>) -> Result<String, RadioError> {
        self.station_radio(&computer)?.new_project(name, folder, make, tool.filter(|t| !t.is_empty()))
    }
    /// Start a new agent in a team, on the computer named. With none named it starts on the one computer that
    /// has that team; when several have it he is asked which, and nothing is started. It is started with
    /// nothing to do yet and shows up at once, waiting for his first word. Nothing happens for a team no
    /// computer has.
    /// `tool` is the coding tool it is, by id, with what to call that tool; nothing is the computer's usual one.
    pub fn new_agent(&self, room: String, computer: Option<String>, tool: Option<CodingTool>) {
        let radio = {
            let state = self.shared.state.lock().unwrap();
            let has = |s: &&Station| s.agents.iter().any(|a| a.group == room);
            let there: Vec<&Station> = state.stations.iter().filter(has).collect();
            match (computer.filter(|c| !c.is_empty()), there.as_slice()) {
                (Some(named), _) => there.iter().find(|s| s.name == named).map(|s| s.radio.clone()).ok_or_else(|| format!("{named} has no team called {room}.")),
                (None, [only]) => Ok(only.radio.clone()),
                (None, []) => return,
                (None, several) => Err(format!("Which computer should it start on? {} each have a team called {room}.", several.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" and "))),
            }
        };
        let radio = match radio {
            Ok(radio) => Some(radio),
            Err(ask) => {
                self.shared.host.note(ask);
                None
            }
        };
        if let Some(radio) = radio {
            // sent as what it is; the words are only what the chat shows for it
            let shown = match &tool {
                Some(tool) => format!("New {} agent in {room}", tool.name),
                None => format!("New agent in {room}"),
            };
            let launch = airband_core::radio::Launch { project: room, tool: tool.map(|t| t.id), ..Default::default() };
            radio.queue_as(shown, None, None, None, String::new(), Vec::new(), Some(Act::Launch(launch)));
        }
    }
    /// Every paired computer's accounts, in the order the computers were paired. A computer that cannot be
    /// reached, or whose airband is too old, comes back with a note and no accounts. Blocks: call it off the
    /// main thread.
    pub fn accounts(&self, fresh: bool) -> Vec<StationAccounts> {
        // a computer is always headed by a name: its own, or its place in the list until it has said its own
        let radios = self.shared.state.lock().unwrap().named_radios();
        // one after another, not all at once: two computers on the same account asking Anthropic in the same
        // moment is how one of them gets refused
        let mut all: Vec<StationAccounts> = radios
            .into_iter()
            .map(|(name, radio)| {
                radio.accounts(name.clone(), fresh).unwrap_or_else(|e| StationAccounts {
                    station: name,
                    accounts: Vec::new(),
                    note: Some(match e {
                        RadioError::Told { reason } => reason,
                        RadioError::Offline => "Cannot reach this computer.".into(),
                        _ => "This computer would not say.".into(),
                    }),
                    checked_at: 0,
                })
            })
            .collect();
        // the same account on two computers has one allowance: where one computer could not be told it, what
        // the other was told is shown
        let known: Vec<(String, String, Vec<AccountLimit>)> = all.iter().flat_map(|s| s.accounts.iter()).filter(|a| !a.limits.is_empty()).map(|a| (a.harness.clone(), a.label.clone(), a.limits.clone())).collect();
        for a in all.iter_mut().flat_map(|s| s.accounts.iter_mut()).filter(|a| a.limits.is_empty()) {
            if let Some((_, _, limits)) = known.iter().find(|(harness, label, _)| *harness == a.harness && *label == a.label) {
                a.limits = limits.clone();
                a.problem = None;
            }
        }
        all
    }
    /// Move one computer's sessions to another of its accounts. `station`, `harness` and `id` are as
    /// [`Desk::accounts`] gave them. Blocks: call it off the main thread.
    pub fn use_account(&self, station: String, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        let radio = self.shared.state.lock().unwrap().stations.iter().find(|s| s.name == station).map(|s| s.radio.clone()).ok_or(RadioError::Refused)?;
        radio.use_account(station, harness, id)
    }
    /// Put an agent in another team: one of the teams on screen, or a new one of that name. `agent` is its id.
    /// The list shows it there at once. Blocks: call it off the main thread.
    pub fn move_agent(&self, agent: String, team: String) -> Result<(), RadioError> {
        let radio = self.agent_radio(&agent)?;
        let now = radio.move_agent(agent.clone(), team)?;
        {
            // shown as the tower now has it, without waiting for the next look at the feed
            let mut state = self.shared.state.lock().unwrap();
            for a in state.stations.iter_mut().flat_map(|s| s.agents.iter_mut()).filter(|a| a.id == agent) {
                (a.callsign, a.group) = (now[0].clone(), now[1].clone());
            }
        }
        self.shared.host.changed();
        Ok(())
    }
    /// Rename one of his teams, on the computer whose team it is. Only what it is called changes: its agents
    /// stay in it. Answers with the name it now has. Blocks: call it off the main thread.
    pub fn rename_team(&self, team: String, to: String) -> Result<String, RadioError> {
        let radio = {
            let state = self.shared.state.lock().unwrap();
            state.stations.iter().find(|s| s.agents.iter().any(|a| a.group == team)).map(|s| s.radio.clone())
        };
        let now = radio.ok_or(RadioError::Told { reason: "That team has no agents now.".into() })?.rename_team(team.clone(), to)?;
        {
            // shown as the tower now has it, without waiting for the next look at the feed
            let mut state = self.shared.state.lock().unwrap();
            state.colours.renamed(&team, &now);
            for a in state.stations.iter_mut().flat_map(|s| s.agents.iter_mut()).filter(|a| a.group == team) {
                a.callsign = airband_core::routing::Callsign { project: now.clone(), ..a.parts() }.text();
                a.group = now.clone();
            }
        }
        self.shared.host.changed();
        Ok(now)
    }
    /// Stop what an agent is doing, now: one its computer runs itself. What he said to it meanwhile it then
    /// reads together. Blocks: call it off the main thread.
    pub fn stop_agent(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.agent_radio(&agent)?;
        radio.stop_agent(agent)
    }
    /// End an agent its computer runs itself: it finishes and leaves the radio. Blocks: call it off the main thread.
    pub fn end_agent(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.agent_radio(&agent)?;
        radio.end_agent(agent)
    }
    /// Let a question go without answering it: it stops waiting on him, here at once and on its computer.
    /// Blocks: call it off the main thread.
    pub fn dismiss(&self, message_id: String) -> Result<(), RadioError> {
        let radio = self.radio_of(&message_id).ok_or(RadioError::Refused)?;
        self.shared.state.lock().unwrap().answered.insert(message_id.clone());
        self.shared.host.changed();
        radio.dismiss(message_id)
    }
    /// Have its computer take over an agent he started in a terminal. Blocks: call it off the main thread.
    pub fn take_over(&self, agent: String) -> Result<(), RadioError> {
        let radio = self.agent_radio(&agent)?;
        radio.take_over(agent)
    }
}
