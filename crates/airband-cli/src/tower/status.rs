//! The tower's own page about itself, `/v1/status`, for `airband status` and the test suites: one shape, read
//! and written as the same types on both ends.
use super::*;

/// How the tower stands.
#[derive(Serialize, Deserialize)]
pub(crate) struct Status {
    /// A radio is on a call.
    pub(crate) phone: bool,
    /// A radio has said to stand by.
    pub(crate) held: bool,
    pub(crate) radios: Vec<String>,
    /// How many things he said are being worked out right now.
    pub(crate) answering: usize,
    /// Where a new agent can be started, by name, with the folder.
    pub(crate) projects: BTreeMap<String, String>,
    /// How many messages and raw events the database holds, when it could say.
    pub(crate) kept: Option<Kept>,
    pub(crate) sessions: Vec<StatusAgent>,
    /// What is waiting to be heard.
    pub(crate) pending: Vec<Pending>,
    /// The last changes to what is read out, each with what it was, what it became and why.
    pub(crate) aloud: VecDeque<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Kept {
    pub(crate) messages: u64,
    pub(crate) events: u64,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StatusAgent {
    pub(crate) callsign: String,
    pub(crate) harness: String,
    pub(crate) cwd: Option<String>,
    pub(crate) state: AgentState,
    /// His words can be got to it.
    pub(crate) reachable: bool,
    pub(crate) meta: serde_json::Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Pending {
    /// Who said it, as it is called now.
    pub(crate) callsign: String,
    /// What it was read as, when it has been read.
    pub(crate) kind: Option<Kind>,
    pub(crate) say: Option<String>,
    pub(crate) aloud: Aloud,
}

impl Status {
    pub(super) fn of(app: &App) -> Status {
        let mut inner = app.passing();
        inner.reap();
        let open = inner.open_ids();
        Status {
            phone: inner.connected(),
            held: inner.calls.values().any(|c| c.held),
            radios: inner.calls.iter().filter(|(_, c)| c.connected()).map(|(id, _)| id.clone()).collect(),
            answering: inner.answering.len(),
            projects: inner.launchable().into_iter().map(|p| (p.name, p.folder)).collect(),
            kept: app.store.counts().map(|(messages, events)| Kept { messages, events }),
            sessions: inner.sessions.iter().map(|s| StatusAgent { callsign: s.callsign.text(), harness: s.agent.harness.clone(), cwd: s.agent.cwd.clone(), state: inner.state_of(s, &open), reachable: s.agent.deliver.is_some(), meta: s.agent.meta.clone() }).collect(),
            pending: inner.unheard().map(|e| Pending { callsign: inner.sender_name(e), kind: e.kind, say: e.short.clone(), aloud: e.aloud.clone() }).collect(),
            aloud: inner.aloud_trail.clone(),
        }
    }
}
