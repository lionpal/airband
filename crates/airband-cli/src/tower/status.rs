//! The tower's own page about itself, `/v1/status`, for `airband status` and the test suites: one shape, read
//! and written as the same types on both ends.
use super::*;

/// Where the tower serves this page: the one place the path is written, for the route and for `airband status`.
pub(crate) const STATUS_PATH: &str = "/v1/status";

/// How the tower stands.
#[derive(Serialize, Deserialize)]
pub(crate) struct Status {
    /// A radio is on a call.
    pub(crate) phone: bool,
    /// A radio has said to stand by.
    pub(crate) held: bool,
    /// The radios on a call, each by the name its pairing was given (a radio that came with the tower's own token
    /// is named by what it said it is called).
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
        app.reap();
        let cfg = app.cfg();
        let inner = app.read();
        let open = inner.open_ids();
        Status {
            phone: inner.connected(),
            held: inner.calls.values().any(|c| c.held),
            radios: inner.calls.iter().filter(|(_, c)| c.connected()).map(|(id, _)| super::push::radio_name(&cfg, id)).collect(),
            answering: inner.answering.len(),
            projects: inner.launchable().into_iter().map(|p| (p.name, p.folder)).collect(),
            kept: app.store.counts().map(|(messages, events)| Kept { messages, events }),
            sessions: inner.sessions.iter().map(|s| StatusAgent { callsign: s.callsign.text(), harness: s.agent.harness.clone(), cwd: s.agent.cwd.clone(), state: inner.state_of(s, &open), reachable: s.agent.deliver.is_some(), meta: s.agent.meta.clone() }).collect(),
            pending: inner.unheard().map(|e| Pending { callsign: inner.sender_name(e), kind: e.kind, say: e.short.clone(), aloud: e.aloud.clone() }).collect(),
            aloud: inner.aloud_trail.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radio_on_a_call_is_shown_by_its_pairings_name_not_its_channel() {
        let app = super::super::http::tests::app("status-names");
        let pairing = config::PairedRadio { name: "phone".into(), secret: "ab".repeat(32) };
        let channel = super::super::push::channel_of(&pairing).unwrap();
        app.settings.change(|cfg| cfg.pairings.push(pairing.clone())).unwrap();
        {
            let mut inner = app.change();
            let _ = inner.call(&channel).presence.says(airband_core::radio::Presence { on_call: true, showing_chat: false });
            let _ = inner.call("desk phone").presence.says(airband_core::radio::Presence { on_call: true, showing_chat: false });
        }
        let mut shown = Status::of(&app).radios;
        shown.sort();
        assert_eq!(shown, vec!["desk phone".to_string(), "phone".to_string()]);
        // the log calls it by its id, never by the name he gave it
        assert_eq!(super::super::Aloud::Heard { radio: channel.clone(), at: 0 }.words(), format!("heard on radio {}", &channel[..8]));
    }
}
