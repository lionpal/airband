//! Getting his words to an agent. The ways themselves are an edge of the tower (`edges::deliver`); this is
//! the name the rest of the tower calls them by.
use super::*;
use edges::deliver::Deliverer;

/// A new id for one delivery, in the form a tool's own records keep ids in.
pub(super) fn receipt() -> String {
    airband_core::encode::uuid_v4()
}

/// Get `content` to this agent by `how`. `lines` are the chat lines these words are.
pub(super) async fn deliver(app: &Arc<App>, how: &Deliver, agent_id: &str, content: &str, lines: &[String]) -> Result<(), String> {
    edges::deliver::AgentWays(app.clone()).deliver(how, agent_id, content, lines).await
}
