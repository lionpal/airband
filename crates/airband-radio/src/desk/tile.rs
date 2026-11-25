//! What the tile is to hold, from what this phone has of the chat ([`crate::tile`] decides what it shows).
use super::*;
use crate::tile::{TileFacts, TileLine, TileWork, words};

#[uniffi::export]
impl Desk {
    /// What the tile is to hold, from the chat as this phone has it: each agent waiting on him with its open
    /// question, newest first; the news he has not read, newest first, one line for each agent; and who is
    /// working, with what. The host keeps it in the shared folder ([`SharedFolder::keep_tile`]) when it changes.
    pub fn tile(&self) -> TileFacts {
        let state = self.shared.state.lock().unwrap();
        let all = state.merged();
        let agents = state.everyone();
        let asking: Vec<&&ChatAgent> = agents.iter().filter(|a| a.state == AgentState::Asking).collect();
        let line = |m: &ChatMessage| {
            let agent = agents.iter().find(|a| m.agent.as_ref() == Some(&a.id));
            TileLine {
                message: m.id.clone(),
                agent: m.agent.clone().unwrap_or_default(),
                callsign: agent.map_or_else(|| m.from.clone(), |a| a.callsign.clone()),
                badge: m.badge.clone(),
                colour: m.colour,
                text: words(&m.text),
            }
        };
        // each agent waiting on him, with its newest question still open
        let mut waiting: Vec<(u64, TileLine)> = asking
            .iter()
            .map(|a| match all.iter().rev().find(|m| m.needs_you && m.agent.as_deref() == Some(a.id.as_str()) && state.is_open(&m.id)) {
                Some(m) => (m.at, line(m)),
                None => (0, TileLine { message: String::new(), agent: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), colour: state.colour(&a.room), text: String::new() }),
            })
            .collect();
        waiting.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        // the news: an agent's latest line that is still news and that he has not read, but for the agents
        // already on the tile waiting on him
        let mut news: Vec<TileLine> = Vec::new();
        for m in all.iter().rev().filter(|m| m.news && !m.his && state.read.unread(m)) {
            let Some(agent) = m.agent.as_deref() else { continue };
            if asking.iter().any(|a| a.id == agent) || news.iter().any(|l| l.agent == agent) {
                continue;
            }
            news.push(line(m));
            if news.len() == crate::tile::NEWS_KEPT {
                break;
            }
        }
        let mut working: Vec<TileWork> = agents
            .iter()
            .filter(|a| a.state == AgentState::Working)
            .map(|a| TileWork { agent: a.id.clone(), callsign: a.callsign.clone(), badge: a.badge.clone(), colour: state.colour(&a.room), doing: a.doing.clone() })
            .collect();
        working.sort_by(|a, b| a.callsign.cmp(&b.callsign));
        working.truncate(crate::tile::WORKING_KEPT);
        TileFacts { waiting: waiting.into_iter().map(|(_, l)| l).collect(), news, working }
    }
}
