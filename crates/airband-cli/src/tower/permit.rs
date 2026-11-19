//! Permission, asked from the phone. A harness that would stop and ask at the keyboard can ask the tower first:
//! the request goes into the feed as a question with Allow and Deny, and his answer goes back to the harness.
//!
//! What this must never do is decide. The tower passes on his answer and nothing else: there is no default, no
//! "whatever you think best", and nothing a model says counts as a yes. The line he reads is built from what the
//! harness sent, word for word, not rewritten, because it is what will run on his computer. If no radio is
//! there to be asked, the harness is told so at once and asks in its own way.
use super::*;
use airband_core::agent::{PermissionAsk, PermissionState};

/// A radio that has not asked for the feed for this long is not there to be asked.
const NOBODY_AFTER: Duration = Duration::from_secs(90);

/// A harness asks. The question goes into the feed, and the id comes back for fetching the answer.
pub(super) fn ask(app: &App, ask: PermissionAsk) -> PermissionState {
    let nobody = PermissionState { state: "nobody".into(), ..PermissionState::default() };
    if ask.schema != agent::SCHEMA || app.cfg.permission_wait == 0 {
        return nobody;
    }
    let mut inner = app.inner.lock().unwrap();
    if !inner.last_reader.is_some_and(|t| t.elapsed() < NOBODY_AFTER) && !inner.connected() {
        return nobody;
    }
    inner.check_in(&ask.agent);
    let agent_id = ask.agent.id.clone();
    let session = {
        let s = inner.session(&agent_id).unwrap();
        s.waiting = true;
        s.clone()
    };
    // one request at a time from each agent: an earlier one it has moved on from is withdrawn
    inner.permits.retain(|_, p| p.agent_id != agent_id || p.decided.is_some());
    inner.drop_pending(&agent_id, None);
    let (id, entry_id) = (config::random_hex(8), config::random_hex(8));
    let what = ask.what.trim();
    let line = format!("Wants to {what}. Allow?");
    log(format!("{}: asks permission to {what}", session.callsign.text()));
    let detail = if ask.detail.trim().is_empty() { line.clone() } else { ask.detail.clone() };
    inner.record(Entry { short: Some(line.clone()), options: vec!["Allow".into(), "Deny".into()], permit: Some(id.clone()), ..Entry::said(entry_id.clone(), &session, Some(Kind::NeedsYou), detail.clone()) });
    // on a call it is read out like any other question
    inner.pending.push(Item { id: entry_id.clone(), agent_id: agent_id.clone(), notice: false, full: detail, kind: Some(Kind::NeedsYou), say: Some(line), detail: None, created: now(), asked: None });
    inner.permits.insert(id.clone(), Permit { agent_id, entry: entry_id, decided: None, asked: Instant::now() });
    app.save(&inner);
    app.kick.notify_one();
    PermissionState { id, state: "pending".into(), message: String::new(), wait_seconds: app.cfg.permission_wait }
}

/// Where a request stands. Held open for up to `wait` seconds while there is no answer.
pub(super) async fn state(app: &Arc<App>, id: &str, wait: u64) -> PermissionState {
    let until = Instant::now() + Duration::from_secs(wait.min(25));
    loop {
        {
            let mut inner = app.inner.lock().unwrap();
            let Some(permit) = inner.permits.get(id) else { return PermissionState { id: id.into(), state: "gone".into(), ..PermissionState::default() } };
            if let Some((allow, message)) = permit.decided.clone() {
                // handed over: the harness has its answer
                inner.permits.remove(id);
                return PermissionState { id: id.into(), state: if allow { "allow" } else { "deny" }.into(), message, wait_seconds: 0 };
            }
        }
        if Instant::now() >= until {
            return PermissionState { id: id.into(), state: "pending".into(), message: String::new(), wait_seconds: app.cfg.permission_wait };
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The harness has stopped waiting and is asking in its own way. The question in the feed is no longer one he
/// can answer from here.
pub(super) fn withdraw(app: &App, id: &str) {
    let mut inner = app.inner.lock().unwrap();
    let Some(permit) = inner.permits.remove(id) else { return };
    if let Some(e) = inner.log.iter_mut().find(|e| e.id == permit.entry) {
        e.permit = None;
        e.options.clear();
        e.short = e.short.as_ref().map(|s| s.replace(" Allow?", " It is asking at the keyboard now."));
    }
    inner.pending.retain(|p| p.id != permit.entry);
    inner.touch(&permit.entry);
    app.save(&inner);
}

/// What his words say about a request: yes, no, or neither.
fn verdict(text: &str) -> Option<bool> {
    let said: String = text.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
    let said = said.trim();
    const YES: [&str; 14] = ["allow", "allow it", "yes", "yeah", "yep", "yup", "ok", "okay", "go ahead", "do it", "approve", "approved", "sure", "fine"];
    const NO: [&str; 8] = ["deny", "deny it", "no", "nope", "dont", "do not", "stop", "refuse"];
    if YES.contains(&said) {
        Some(true)
    } else if NO.contains(&said) {
        Some(false)
    } else {
        None
    }
}

/// Is what he just said the answer to a request for permission? It is when he answered that question itself
/// (`re`), or when exactly one request is waiting and he said a plain yes or no. Otherwise nothing here
/// touches it. Words of his own on the question itself refuse it, and are passed on as the reason.
pub(super) fn answer(app: &App, text: &str, re: Option<&str>) -> Option<Reply> {
    let say = |s: &str| Some(Reply { say: s.into(), listen: false, ..Reply::default() });
    let mut inner = app.inner.lock().unwrap();
    let waiting = |p: &Permit| p.decided.is_none();
    let on_question = re.and_then(|r| inner.permits.iter().find(|(_, p)| p.entry == r && waiting(p)).map(|(id, _)| id.clone()));
    let id = match on_question {
        Some(id) => id,
        None => {
            // not aimed at a question: only a plain yes or no, and only when it can mean one thing
            if re.is_some() || verdict(text).is_none() {
                return None;
            }
            let mut open = inner.permits.iter().filter(|(_, p)| waiting(p));
            let (first, more) = (open.next().map(|(id, _)| id.clone()), open.next().is_some());
            match (first, more) {
                (Some(id), false) => id,
                _ => return None,
            }
        }
    };
    // handing the decision back is not an answer to "may I": he has to say which
    if text.trim().trim_end_matches('.').eq_ignore_ascii_case("do whatever you think is best") {
        return say("That one needs a yes or a no from you.");
    }
    let (allow, reason) = match verdict(text) {
        Some(allow) => (allow, String::new()),
        None => (false, text.trim().to_string()),
    };
    let agent_id = {
        let permit = inner.permits.get_mut(&id).unwrap();
        permit.decided = Some((allow, reason.clone()));
        permit.agent_id.clone()
    };
    let entry = inner.permits[&id].entry.clone();
    if let Some(e) = inner.log.iter_mut().find(|e| e.id == entry) {
        e.permit = None;
    }
    inner.pending.retain(|p| p.id != entry);
    let Some(session) = inner.find(&agent_id).cloned() else { return say("That agent has gone.") };
    inner.session(&agent_id).unwrap().waiting = false;
    log(format!("← radio: \"{text}\" → {} is {} {}", session.callsign.text(), if allow { "allowed" } else { "refused" }, if reason.is_empty() { String::new() } else { "with his reason".into() }));
    let shown = if allow { "Allow".to_string() } else if reason.is_empty() { "Deny".into() } else { reason };
    inner.record(Entry::his(config::random_hex(8), &[session.clone()], shown));
    inner.touch(&entry);
    app.save(&inner);
    say(if allow { "Allowed." } else { "Refused." })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_yes_or_no_is_a_verdict() {
        assert_eq!(verdict("Allow"), Some(true));
        assert_eq!(verdict("go ahead."), Some(true));
        assert_eq!(verdict("No!"), Some(false));
        assert_eq!(verdict("Don't"), Some(false));
        assert_eq!(verdict("use the staging database instead"), None);
        assert_eq!(verdict("yes but only on staging"), None);
        assert_eq!(verdict("Do whatever you think is best."), None);
    }
}
