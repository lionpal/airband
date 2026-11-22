//! How the command line and the adapters reach the tower on this machine ([`TowerHere`]): posting an event (kept
//! in a folder while the tower is away, and handed over when it is back), and asking him for leave on a tool's
//! behalf and waiting for his answer.
use super::*;

/// The tower on this computer, asked with its own token. Every status comes back as an answer, for the caller
/// to read; only a tower that cannot be reached is an error. The one way the command line and the hooks reach it.
pub(crate) struct TowerHere {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

impl TowerHere {
    pub(crate) fn here(cfg: &config::Config, timeout: std::time::Duration) -> TowerHere {
        TowerHere { agent: airband_core::net::agent(timeout), base: format!("http://127.0.0.1:{}", cfg.port), auth: format!("Bearer {}", cfg.token) }
    }
    pub(crate) fn get(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent.get(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
    pub(crate) fn post(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.agent.post(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
    pub(crate) fn delete(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent.delete(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
}

/// A request's answer, when the tower took it (200).
pub(crate) fn taken(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Option<ureq::http::Response<ureq::Body>> {
    res.ok().filter(|r| r.status().as_u16() == 200)
}

/// Requests for leave a watcher puts to him itself, each by a key (the request's id, or its session's): one is
/// put to him once at a time; one that found nobody there is tried again after a while; one answered or left
/// is not put again while it is still waiting.
#[derive(Default)]
pub(crate) struct Asking {
    busy: std::collections::HashSet<String>,
    done: std::collections::HashSet<String>,
    again: std::collections::HashMap<String, std::time::Instant>,
}

impl Asking {
    /// How long one that found nobody there waits before it is tried again.
    const AGAIN: Duration = Duration::from_secs(20);
    /// Take this one to put to him now. False while it is being put, once it was, or before it is due again.
    pub(crate) fn take(&mut self, key: &str) -> bool {
        let due = !self.busy.contains(key) && !self.done.contains(key) && self.again.get(key).is_none_or(|at| *at <= std::time::Instant::now());
        if due {
            self.busy.insert(key.to_string());
        }
        due
    }
    /// What came of putting it to him. Nothing is a try that could not be made (the tool could not be followed).
    pub(crate) fn over(&mut self, key: &str, came: Option<&Answer>) {
        self.busy.remove(key);
        match came {
            Some(Answer::Nobody) | None => {
                self.again.insert(key.to_string(), std::time::Instant::now() + Self::AGAIN);
            }
            Some(_) => {
                self.again.remove(key);
                self.done.insert(key.to_string());
            }
        }
    }
    /// Forget the ones put to him that are no longer waiting: one waiting again is a new request.
    pub(crate) fn keep_only(&mut self, waiting: impl Fn(&str) -> bool) {
        self.done.retain(|k| waiting(k));
    }
}

/// What came of putting a request to him.
pub(crate) enum Answer {
    /// He answered: yes or no, and with a no the reason he gave, if any.
    Said(bool, String),
    /// Nobody was there to ask (no tower, or no radio), so nothing was put in front of anyone.
    Nobody,
    /// It was put to him and he did not answer in the time the tower allows, or it was answered elsewhere.
    Unanswered,
}

/// Put a permission request to him through the tower, and wait for his answer. Nothing when there is none to be
/// had: no tower, no radio there to ask, or he did not answer in the time the tower allows. Then the harness
/// asks at the keyboard as it always has. Waiting never turns into a yes.
pub(crate) fn ask_him(request: &PermissionAsk) -> Option<(bool, String)> {
    match ask_him_while(request, None) {
        Answer::Said(allow, reason) => Some((allow, reason)),
        Answer::Nobody | Answer::Unanswered => None,
    }
}

/// The same, for a request that can also be answered somewhere else meanwhile (at the tool's own screen):
/// `still` is asked every few seconds whether the tool is still waiting, and when it is not the question is
/// taken back.
pub(crate) fn ask_him_while(request: &PermissionAsk, still: Option<&dyn Fn() -> bool>) -> Answer {
    let Some(cfg) = config::read() else { return Answer::Nobody };
    const BASE: &str = "/v1/agent/permission";
    let here = TowerHere::here(&cfg, Duration::from_secs(40));
    let asked: Option<PermissionState> = taken(here.post(BASE).send_json(request)).and_then(|mut res| res.body_mut().read_json().ok());
    let Some(asked) = asked.filter(|a| a.state == airband_core::agent::Leave::Pending) else { return Answer::Nobody };
    let until = std::time::Instant::now() + Duration::from_secs(asked.wait_seconds.max(5));
    while std::time::Instant::now() < until {
        if still.is_some_and(|still| !still()) {
            break;
        }
        let left = until.saturating_duration_since(std::time::Instant::now()).as_secs().clamp(1, if still.is_some() { 3 } else { 20 });
        let now: Option<PermissionState> = taken(here.get(&format!("{BASE}/{}?wait={left}", asked.id)).call()).and_then(|mut res| res.body_mut().read_json().ok());
        // the tower went away mid-wait: stop waiting and let the harness ask
        let Some(now) = now else { return Answer::Unanswered };
        use airband_core::agent::Leave;
        match now.state {
            Leave::Allow => return Answer::Said(true, String::new()),
            Leave::Deny => return Answer::Said(false, now.message),
            Leave::Pending => {}
            Leave::Nobody | Leave::Gone => return Answer::Unanswered,
        }
    }
    // he did not answer, or it was answered elsewhere: take the question back, so the feed does not offer an
    // answer that no longer does anything
    let _ = here.delete(&format!("{BASE}/{}", asked.id)).call();
    Answer::Unanswered
}

/// What came of posting an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Posted {
    /// The tower took it.
    Taken,
    /// The tower did not answer, and it is kept to be handed over when the tower is back.
    Kept,
    /// The tower did not answer, and it could not be kept.
    Lost,
}

/// Post one event to the tower on this machine. One the tower did not take (it is being restarted or
/// updated) is kept in a folder and handed over when the tower is back: a report is not lost because it was
/// made in those few seconds.
pub(crate) fn post(event: &AgentEvent) -> Posted {
    if send(event) {
        return Posted::Taken;
    }
    spool(&config::dir().join("spool"), event)
}

/// Keep an event the tower did not take in `dir`, under a name that sorts by when it was made. A full folder
/// (500 waiting) or one that cannot be written keeps nothing.
fn spool(dir: &std::path::Path, event: &AgentEvent) -> Posted {
    let Ok(()) = std::fs::create_dir_all(dir) else { return Posted::Lost };
    let Ok(waiting) = std::fs::read_dir(dir).map(|d| d.count()) else { return Posted::Lost };
    if waiting >= 500 {
        return Posted::Lost;
    }
    let Ok(at) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else { return Posted::Lost };
    let Ok(body) = serde_json::to_vec(event) else { return Posted::Lost };
    match config::write_private(&dir.join(format!("{:020}-{}.json", at.as_nanos(), config::random_hex(4))), body) {
        Ok(_) => Posted::Kept,
        Err(_) => Posted::Lost,
    }
}

fn send(event: &AgentEvent) -> bool {
    let Some(cfg) = config::read() else { return false };
    taken(TowerHere::here(&cfg, Duration::from_millis(1500)).post("/v1/agent/events").send_json(event)).is_some()
}

/// The tower takes what was kept for it, oldest first, by `take`: it is the tower that calls this, and it
/// takes each as it takes any event. Stops at the first it does not take, so order holds.
/// Something kept for more than ten minutes, or that does not read, is thrown away: the tower was away for
/// longer than an update takes, and what an agent said then is no longer news to act on.
pub(crate) fn drain_spool(take: impl Fn(AgentEvent) -> bool) {
    let Ok(dir) = std::fs::read_dir(config::dir().join("spool")) else { return };
    let mut kept: Vec<std::path::PathBuf> = dir.flatten().map(|f| f.path()).collect();
    kept.sort();
    for path in kept {
        let old = std::fs::metadata(&path).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age > Duration::from_secs(600)));
        if let Some(event) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<AgentEvent>(&b).ok()).filter(|_| !old)
            && !take(event) {
                return;
            }
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_the_tower_did_not_take_is_kept_and_said_to_be() {
        let dir = std::env::temp_dir().join(format!("airband-spool-{}", config::random_hex(4)));
        let event = AgentEvent { schema: airband_core::agent::SCHEMA.into(), agent: airband_core::agent::Agent { id: "a".into(), harness: "test".into(), cwd: None, name: None, title: None, pid: None, deliver: None, meta: Default::default() }, event: airband_core::agent::Event::Checkin, via: None };
        assert_eq!(spool(&dir, &event), Posted::Kept);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
