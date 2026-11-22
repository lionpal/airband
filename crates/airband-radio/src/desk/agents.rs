//! What a host can do with an agent or a computer: start one, move it, stop it, end it, and the accounts it runs under.
use super::*;

#[uniffi::export]
impl Desk {
    /// The folders inside one of his on a computer, for choosing where a new team works. `computer` is its
    /// [`DeskComputer::id`], or empty when he has only one; `under` is a folder's whole path, or empty for his
    /// home folder. Blocks: call it off the main thread.
    pub fn folders(&self, computer: String, under: String) -> Result<DeskFolders, RadioError> {
        let f = self.station_radio(&computer)?.folders(under)?;
        Ok(DeskFolders { path: f.path, shown: f.shown, up: f.up, folders: f.folders.into_iter().map(|f| DeskFolder { name: f.name, path: f.path }).collect(), team: f.team, has_team: f.has_team })
    }
    /// The folders the coding tools on a computer have had sessions in, newest first, then the teams there
    /// that no tool has a record of. `computer` is its [`DeskComputer::id`], or empty when he has only one. Blocks: call it
    /// off the main thread.
    pub fn recent_folders(&self, computer: String) -> Result<Vec<DeskRecent>, RadioError> {
        let tools = airband_core::catalog::current();
        let called = |id: &String| tools.tools.iter().find(|t| t.id == *id).map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
        Ok(self.station_radio(&computer)?.recent()?.folders.into_iter().map(|r| DeskRecent { tools: r.tools.iter().map(called).collect(), folder: r.folder, shown: r.shown, name: r.name, team: r.team.is_some(), at: r.at }).collect())
    }
    /// Make a new team on a computer, or start another agent in one that is there: its name as he wrote it,
    /// and the folder its agents work in (with `make`, the folder a new one called by that name is made in).
    /// Its first agent is started there with nothing to do yet. Blocks: call it off the main thread.
    /// `tool` is the coding tool its first agent is, by id; nothing is the computer's usual one.
    /// `computer` is its [`DeskComputer::id`], or empty when he has only one. Answers with the team's room on
    /// his screen ([`DeskRoom::id`]).
    pub fn new_team(&self, computer: String, name: String, folder: String, make: bool, tool: Option<String>) -> Result<String, RadioError> {
        let (radio, station) = self.station_of(&computer)?;
        let room = room_id(&station, &radio.new_project(name, folder, make, tool.filter(|t| !t.is_empty()))?);
        // he is shown the team at once, and it stays open, empty, until its first agent shows up
        {
            let mut state = self.shared.state.lock().unwrap();
            state.focus = Focus { room: Some(room.clone()), agent: None, made: Some(room.clone()) };
        }
        self.shared.host.changed();
        Ok(room)
    }
    /// Start a new agent in a team: `room` is the room on his screen, as [`DeskRoom::id`] gives it. It is
    /// started with nothing to do yet and shows up at once, waiting for his first word. Nothing happens for a
    /// room no computer has now.
    /// `tool` is the coding tool it is, by id, with what to call that tool; nothing is the computer's usual one.
    pub fn new_agent(&self, room: String, tool: Option<CodingTool>) {
        let Some((radio, team, name)) = self.shared.state.lock().unwrap().room(&room) else { return };
        // sent as what it is; the words are only what the chat shows for it
        let shown = match &tool {
            Some(tool) => format!("New {} agent in {name}", tool.name),
            None => format!("New agent in {name}"),
        };
        let launch = airband_core::radio::Launch { team: team.into(), tool: tool.map(|t| t.id), ..Default::default() };
        radio.queue_as(Outgoing::of(shown).with_act(Some(&Act::Launch(launch))), Carry::Nothing);
    }
    /// Every paired computer's accounts, in the order the computers were paired. A computer that cannot be
    /// reached, or whose airband is too old, comes back with a note and no accounts. Blocks: call it off the
    /// main thread.
    pub fn accounts(&self, fresh: bool) -> Vec<StationAccounts> {
        // a computer is always headed by a name: its own, or its place in the list until it has said its own
        let radios = self.shared.state.lock().unwrap().named_radios();
        // one after another, not all at once: two computers on the same account asking Anthropic in the same
        // moment is how one of them gets refused
        let told: Vec<(DeskComputer, Result<airband_core::radio::Accounts, RadioError>)> = radios.into_iter().map(|(computer, radio)| { let got = radio.accounts(fresh); (computer, got) }).collect();
        let answered: Vec<(DeskComputer, airband_core::radio::Accounts)> = told.iter().filter_map(|(c, got)| got.as_ref().ok().map(|a| (c.clone(), a.clone()))).collect();
        let mut filled = StationAccounts::across(&answered).into_iter();
        told.into_iter()
            .map(|(computer, got)| match got {
                Ok(_) => filled.next().expect("one for each that answered"),
                Err(e) => StationAccounts {
                    station: computer.id,
                    name: computer.name,
                    accounts: Vec::new(),
                    note: Some(match e {
                        RadioError::Told { reason } => reason,
                        RadioError::Offline => "Cannot reach this computer.".into(),
                        _ => "This computer would not say.".into(),
                    }),
                    checked_at: 0,
                },
            })
            .collect()
    }
    /// Move one computer's sessions to another of its accounts. `station`, `harness` and `id` are as
    /// [`Desk::accounts`] gave them. Blocks: call it off the main thread.
    pub fn use_account(&self, station: String, harness: String, id: String) -> Result<StationAccounts, RadioError> {
        let radio = self.station_radio(&station)?;
        let computer = self.shared.state.lock().unwrap().named_radios().into_iter().map(|(c, _)| c).find(|c| c.id == station).ok_or(RadioError::Offline)?;
        radio.use_account(&computer, harness, id)
    }
    /// Put an agent in another team: a room on screen with that agent's computer ([`DeskRoom::id`]), or a new
    /// team of the name he gave. `agent` is its id. The list shows it there at once. Blocks: call it off the
    /// main thread.
    pub fn move_agent(&self, agent: String, to: DeskTeamPick) -> Result<(), RadioError> {
        let radio = self.agent_radio(&agent)?;
        let pick = match to {
            DeskTeamPick::Room { room } => {
                let state = self.shared.state.lock().unwrap();
                let (there, team, _) = state.room(&room).ok_or(crate::choices::told(NO_TEAM))?;
                if !Arc::ptr_eq(&there, &radio) {
                    return Err(RadioError::Told { reason: "An agent can only move to a team on its own computer.".into() });
                }
                airband_core::radio::TeamPick::Team { id: team }
            }
            DeskTeamPick::New { name } => airband_core::radio::TeamPick::New { name },
        };
        let now = radio.move_agent(agent.clone(), pick)?;
        {
            // shown as the tower now has it, without waiting for the next look at the feed
            let mut state = self.shared.state.lock().unwrap();
            for s in state.computers.iter_mut().filter(|s| s.agents.iter().any(|a| a.id == agent)) {
                for a in s.agents.iter_mut().filter(|a| a.id == agent) {
                    a.call(now.callsign.clone());
                    a.team = now.team.to_string();
                }
                s.settle_rooms();
            }
        }
        self.shared.host.changed();
        Ok(())
    }
    /// Rename one of his teams: `room` is its room on his screen ([`DeskRoom::id`]). Only what it is called
    /// changes: its agents, its room and its colour stay. Answers with the name it now has. Blocks: call it
    /// off the main thread.
    pub fn rename_team(&self, room: String, to: String) -> Result<String, RadioError> {
        let (radio, team, _) = self.shared.state.lock().unwrap().room(&room).ok_or(crate::choices::told(NO_TEAM))?;
        let now = radio.rename_team(team, to)?;
        {
            // shown as the tower now has it, without waiting for the next look at the feed
            let mut state = self.shared.state.lock().unwrap();
            for a in state.computers.iter_mut().flat_map(|s| s.agents.iter_mut()).filter(|a| a.room == room) {
                a.call(airband_core::routing::Callsign { project: now.clone(), ..a.parts() });
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
