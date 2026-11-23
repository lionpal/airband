//! How the command line and the adapters reach the tower on this machine ([`TowerHere`]). Apart: asking him for
//! leave on a tool's behalf and waiting for his answer (`asking`), and posting an event, kept in a folder while
//! the tower is away and handed over when it is back (`spool`).
mod asking;
mod spool;
pub(crate) use asking::*;
pub(crate) use spool::*;
use super::*;
use airband_core::radio::wire::{self, Nothing, WaitQuery, at};
use airband_io::http::Said;

/// The tower on this computer, asked with its own token. Every status comes back as an answer, for the caller
/// to read; only a tower that cannot be reached is an error. The one way the command line and the hooks reach it.
pub(crate) struct TowerHere {
    client: airband_io::http::Client,
    base: String,
    auth: String,
}

impl TowerHere {
    pub(crate) fn here(cfg: &config::Config, timeout: std::time::Duration) -> TowerHere {
        TowerHere::at(cfg.port, &cfg.token, timeout)
    }
    /// The same, from the port and token alone (`config::reach`), with no keys opened.
    pub(crate) fn reached(reach: &config::Reach, timeout: std::time::Duration) -> TowerHere {
        TowerHere::at(reach.port, &reach.token, timeout)
    }
    fn at(port: u16, token: &str, timeout: std::time::Duration) -> TowerHere {
        TowerHere { client: airband_io::http::Client::new(timeout), base: format!("http://127.0.0.1:{port}"), auth: format!("Bearer {token}") }
    }
    // Each says which version of the radio protocol it speaks: the radio's routes are refused without it, and
    // the agent's routes do not read it.
    /// The tower's own page about itself, read as the type the tower writes it from. Nothing when the tower is
    /// not running, did not take the request, or answered in another shape.
    pub(crate) fn status(&self) -> Option<crate::tower::Status> {
        let res = self.client.send(airband_io::http::Ask::get(format!("{}{}", self.base, crate::tower::STATUS_PATH)).header("authorization", &self.auth).header(airband_core::radio::SCHEMA_HEADER, airband_core::radio::SCHEMA));
        taken(res)?.body_mut().read_json::<crate::tower::Status>().ok()
    }
    /// One of the tower's typed routes: the path, method and answer all come from the endpoint.
    pub(crate) fn call<E: wire::Endpoint>(&self, id: Option<&str>, query: &E::Query, body: Option<&E::Body>) -> Result<Said<E::Answer>, airband_io::http::Error> {
        airband_io::http::call::<E>(&self.client, &self.base, &[("authorization", &self.auth), (airband_core::radio::SCHEMA_HEADER, airband_core::radio::SCHEMA)], id, query, body)
    }
}

/// A request's answer, when it was taken (200): the one place a plain request's status is read that way, for the
/// tower here and for any service a command or an adapter asks.
pub(crate) fn taken(res: Result<airband_io::http::Response, airband_io::http::Error>) -> Option<airband_io::http::Response> {
    res.ok().filter(|r| r.status().as_u16() == 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_request_of_one_session_is_put_to_him_once_the_first_is_answered() {
        let mut asking = Asking::default();
        assert!(asking.take("thread"));
        assert!(!asking.take("thread"), "one at a time");
        asking.over("thread", "call_1", Some(&Answer::Said(true, String::new())));
        // the session still waits, now on another request: that one is put to him at once, the first never again
        assert!(asking.take("thread"));
        assert!(asking.done("call_1") && !asking.done("call_2"));
        asking.keep_only(|_| false);
        assert!(!asking.done("call_1"));
    }

    #[test]
    fn an_answer_that_could_not_be_given_to_the_tool_is_put_to_him_again() {
        let mut asking = Asking::default();
        assert!(asking.take("r1"));
        asking.over("r1", "r1", None);
        assert!(!asking.done("r1"));
        assert!(asking.again.contains_key("r1"));
    }

    #[test]
    fn one_whose_wait_is_over_is_not_kept_for_ever() {
        let mut asking = Asking::default();
        asking.again.insert("gone".into(), std::time::Instant::now());
        asking.over("r2", "r2", None);
        assert!(!asking.again.contains_key("gone") && asking.again.contains_key("r2"));
    }

    #[test]
    fn a_kept_report_the_tower_refuses_never_holds_up_the_ones_after_it() {
        let dir = std::env::temp_dir().join(format!("airband-spool-{}", config::random_hex(4)));
        let event = |id: &str| AgentEvent::new(crate::adapters::agent_with(id, "test"), airband_core::agent::Event::Checkin);
        assert_eq!(spool(&dir, &event("refused")), Posted::Kept);
        assert_eq!(spool(&dir, &event("taken")), Posted::Kept);
        let took = std::sync::Mutex::new(Vec::new());
        drain(&dir, |ev| if ev.agent.id == "refused" { Err("no team".into()) } else { took.lock().unwrap().push(ev.agent.id); Ok(()) });
        assert_eq!(*took.lock().unwrap(), ["taken"]);
        assert_eq!(airband_io::files::entries(&dir).unwrap().len(), 0);
        airband_io::files::remove_folder(&dir).ok();
    }

    #[test]
    fn a_kept_report_still_being_written_is_neither_taken_nor_counted() {
        let dir = std::env::temp_dir().join(format!("airband-spool-{}", config::random_hex(4)));
        let event = AgentEvent::new(crate::adapters::agent_with("a", "test"), airband_core::agent::Event::Checkin);
        assert_eq!(spool(&dir, &event), Posted::Kept);
        // one being written beside it, under the hidden name its writer renames it from
        let part = dir.join(".00000000000000000001-abcd.json.part-1234");
        std::fs::write(&part, serde_json::to_vec(&event).unwrap()).unwrap();
        let took = std::sync::Mutex::new(0);
        drain(&dir, |_| { *took.lock().unwrap() += 1; Ok(()) });
        assert_eq!(*took.lock().unwrap(), 1);
        assert!(part.exists(), "left to its writer");
        airband_io::files::remove_folder(&dir).ok();
    }

    #[test]
    fn an_event_the_tower_did_not_take_is_kept_and_said_to_be() {
        let dir = std::env::temp_dir().join(format!("airband-spool-{}", config::random_hex(4)));
        let event = AgentEvent::new(crate::adapters::agent_with("a", "test"), airband_core::agent::Event::Checkin);
        assert_eq!(spool(&dir, &event), Posted::Kept);
        assert_eq!(airband_io::files::entries(&dir).unwrap().len(), 1);
        airband_io::files::remove_folder(&dir).ok();
    }
}
