//! The agents on the radio, as the state holds them: found by id, let go when gone, and what each is doing.
use super::*;

impl Inner {
    pub(in crate::tower) fn session(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.agent.id == id)
    }
    pub(in crate::tower) fn find(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.agent.id == id)
    }
    /// The agent that said a logged message, if it is still on the radio.
    pub(in crate::tower) fn sender(&self, e: &Entry) -> Option<&Session> {
        e.agent.as_deref().and_then(|id| self.find(id))
    }
    /// What the one who said a logged message is called now, as it is said: its callsign while it is here, and
    /// what the line was written down as coming from after it has gone.
    pub(in crate::tower) fn sender_name(&self, e: &Entry) -> String {
        self.sender(e).map(|s| s.callsign.text()).unwrap_or_else(|| e.from.clone())
    }
    pub(in crate::tower) fn reap(&mut self) -> bool {
        let dead: Vec<String> = self.sessions.iter().filter(|s| s.agent.deliver != Some(Deliver::Held) && s.agent.pid.is_some_and(|p| !alive(p))).map(|s| s.agent.id.clone()).collect();
        for id in &dead {
            // by its id: its callsign carries its team's name
            log(format!("agent {} is gone", super::super::short_id(id)));
            self.remove(id);
        }
        // a permission request nobody is waiting on any more: its line stops offering an answer
        let over: Vec<(String, permit::Ended)> = self.permits.iter().filter(|(_, p)| !p.live()).map(|(id, p)| (id.clone(), p.unwaited())).collect();
        let closed = !over.is_empty();
        for (id, ended) in over {
            self.close_permit(&id, ended);
        }
        // a radio not heard from in a day has nothing worth keeping
        self.calls.retain(|_, c| c.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(86_400)));
        let teams = self.drop_unused_teams();
        !dead.is_empty() || closed || teams
    }
    /// What an agent is doing.
    ///
    /// One part of this is a rule and not a fact: an agent that stopped to wait for background work of its own
    /// counts as working for [`STILL_ITS_OWN_WORK_MS`] and as stalled after. No tool says whether that work is
    /// still running.
    pub(in crate::tower) fn state_of(&self, s: &Session, open: &HashSet<String>) -> AgentState {
        if !s.waiting {
            return AgentState::Working;
        }
        // It stopped only to wait for something of its own that is still running: that is still work, for a
        // while. If nothing has come of it after twenty minutes it is not coming back by itself: it has
        // stalled, and a poke is offered.
        if let Some(e) = self.log.iter().rev().find(|e| e.agent.as_deref() == Some(s.agent.id.as_str())).filter(|e| e.background) {
            return if now_ms().saturating_sub(e.at) < STILL_ITS_OWN_WORK_MS { AgentState::Working } else { AgentState::Stalled };
        }
        match self.last_said(&s.agent.id) {
            Some(e) if open.contains(&e.id) => AgentState::Asking,
            Some(e) if e.kind == Some(Kind::Stalled) => AgentState::Stalled,
            _ => AgentState::Idle,
        }
    }
    /// A number that changes when anything a radio shows about the agents does: who is there, what each is
    /// called, its state, what it is doing.
    /// `carried`: the coding tools a launcher here carries on ([`crate::tower::held::carried_on`]).
    pub(in crate::tower) fn roster_rev(&self, carried: &[String]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.roster_entries(carried)).unwrap_or_default().hash(&mut h);
        // never 0: that is what a radio sends before it has seen one
        h.finish() | 1
    }
    /// The agents that have stopped without finishing or asking: the ones a poke is for.
    pub(in crate::tower) fn stalled(&self) -> Vec<Session> {
        let open = self.open_ids();
        self.sessions.iter().filter(|s| self.state_of(s, &open) == AgentState::Stalled).cloned().collect()
    }
    /// Every agent on the radio. `carried`: the coding tools a launcher here carries on, which says whether
    /// one with no way in can be taken over.
    pub(in crate::tower) fn roster_entries(&self, carried: &[String]) -> Vec<RosterEntry> {
        let open = self.open_ids();
        let reach = |s: &Session| match s.agent.deliver {
            Some(_) => Reach::Reaches,
            None => Reach::NoWayIn { take_over: crate::tower::held::can_take_over(self, s, carried) },
        };
        self.sessions.iter().map(|s| RosterEntry { reach: reach(s), id: s.agent.id.clone(), callsign: s.callsign.clone(), team: s.team.clone(), harness: s.agent.harness.clone(), waiting: s.waiting, state: self.state_of(s, &open), subagents: s.subagents, doing: s.doing.clone().filter(|_| !s.waiting), held: s.agent.deliver == Some(Deliver::Held), terminal: matches!(s.agent.deliver, Some(Deliver::Keys { .. } | Deliver::Socket { .. })), ends: match &s.agent.deliver { Some(Deliver::Held) => true, Some(Deliver::Harness { name, .. }) => crate::adapters::puts_away(name), _ => false }, model: s.model.clone(), effort: s.effort.clone() }).collect()
    }
    pub(in crate::tower) fn roster(&self) -> String {
        if self.sessions.is_empty() {
            return "No agents are running.".into();
        }
        let names = |waiting: bool| self.sessions.iter().filter(|s| s.waiting == waiting).map(|s| s.callsign.text()).collect::<Vec<_>>().join(", ");
        let (waiting, working) = (names(true), names(false));
        [(!waiting.is_empty()).then(|| format!("Waiting on you: {waiting}.")), (!working.is_empty()).then(|| format!("Working: {working}."))].into_iter().flatten().collect::<Vec<_>>().join(" ")
    }
}
