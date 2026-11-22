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
impl Inner {
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
        // what it reports now; for one already here that says neither this time, what it said before
        let named = agent.name.clone().or(agent.cwd.clone()).or_else(|| self.find(&agent.id).and_then(|s| s.agent.name.clone().or(s.agent.cwd.clone()))).unwrap_or_default();
        // the name he gave the session, when it is new to the tower or has changed
        let wanted = agent.title.as_deref().filter(|t| self.find(&agent.id).is_none_or(|s| s.agent.title.as_deref() != Some(*t))).and_then(|t| Callsign::titled(&named, t)).map(|mut want| {
            // one that was moved to another team is renamed there, not back in its folder's
            if let Some(s) = self.find(&agent.id).filter(|s| s.moved.is_some()) {
                want.project = s.callsign.project.clone();
            }
            want
        });
        if let Some(want) = &wanted {
            self.make_room(&agent.id, want, &named);
        }
        let free = |inner: &Inner, c: &Callsign| !inner.sessions.iter().any(|s| s.callsign == *c);
        if self.find(&agent.id).is_none() {
            let callsign = wanted.clone().filter(|c| free(self, c)).unwrap_or_else(|| {
                // the next free number, among the names in use here and on his other computers (as his radio
                // shows them: "shop 2" there takes "shop two" here)
                let mut taken = self.callsigns();
                let mut c = Callsign::next_for(&named, &taken);
                while self.elsewhere.iter().any(|e| airband_core::routing::shown(&e.text()) == airband_core::routing::shown(&c.text())) {
                    taken.push(c.clone());
                    c = Callsign::next_for(&named, &taken);
                }
                c
            });
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            // remember where this project lives, so a new agent can be launched there later
            if let Some(cwd) = agent.cwd.clone().filter(|_| agent.name.is_none()) {
                self.projects.insert(callsign.project.clone(), cwd);
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
            let team = Callsign::next_for(&name, &[]).project;
            self.session(&agent.id).unwrap().agent.name = Some(name);
            self.move_to(&agent.id, &team, Mover::Tower);
        }
        self.colour_teams();
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
    fn make_room(&mut self, id: &str, want: &Callsign, named: &str) {
        let Some(holder) = self.sessions.iter().find(|s| s.callsign == *want && s.agent.id != id).cloned() else { return };
        if holder.agent.title.as_deref().and_then(|t| Callsign::titled(named, t)).as_ref() == Some(want) {
            return;
        }
        let mut taken = self.callsigns();
        taken.push(want.clone());
        self.rename(&holder.agent.id, Callsign::next_for(named, &taken));
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
        self.colour_teams();
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
    /// Every team on the radio has a colour: a new one takes the first that no team has, and when they have all
    /// been taken, the one fewest teams share.
    pub(super) fn colour_teams(&mut self) {
        const COLOURS: u8 = 6;
        let teams: Vec<String> = self.sessions.iter().map(|s| s.callsign.project.clone()).collect();
        for team in teams {
            if !self.colours.contains_key(&team) {
                let next = (0..COLOURS).min_by_key(|c| self.colours.values().filter(|have| *have == c).count()).unwrap_or(0);
                self.colours.insert(team, next);
            }
        }
    }
    pub(super) fn callsigns(&self) -> Vec<Callsign> {
        self.sessions.iter().map(|s| s.callsign.clone()).collect()
    }
    /// A radio said which names his other computers use. Any agent here that was given one of them by number
    /// takes the next free number. Returns whether anything was renamed.
    pub(super) fn taken_elsewhere(&mut self, shown: &[String]) -> bool {
        use airband_core::routing::shown as as_shown;
        // as callsigns: the team is everything before the last word
        self.elsewhere = shown.iter().filter_map(|c| c.rsplit_once(' ')).map(|(project, number)| Callsign { project: project.to_string(), number: number.to_string(), named: false }).collect();
        let clashing: Vec<String> = self.sessions.iter().filter(|s| s.callsign.numbered() && shown.iter().any(|c| *c == as_shown(&s.callsign.text()))).map(|s| s.agent.id.clone()).collect();
        for id in &clashing {
            let Some(project) = self.find(id).map(|s| s.callsign.project.clone()) else { continue };
            // a name nobody has, here or there: numbers are compared as the radio shows them
            let mut n = 1;
            let free = loop {
                let c = Callsign::next_for(&project, &[]);
                let candidate = Callsign { project: c.project.clone(), number: airband_core::routing::number_name(n), named: false };
                let text = as_shown(&candidate.text());
                if !shown.contains(&text) && !self.sessions.iter().any(|s| as_shown(&s.callsign.text()) == text) {
                    break candidate;
                }
                n += 1;
            };
            self.rename(id, free);
        }
        !clashing.is_empty()
    }
}
