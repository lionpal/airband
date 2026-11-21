//! Permission, asked from the phone. A harness that would stop and ask at the keyboard can ask the tower first:
//! the request goes into the feed as a question with Allow and Deny, and his answer goes back to the harness.
//!
//! What this must never do is decide. The tower passes on his answer and nothing else: there is no default, no
//! "whatever you think best", and nothing a model says counts as a yes. The line he reads is built from what the
//! harness sent, word for word, not rewritten, because it is what will run on his computer. If no radio is
//! there to be asked, the harness is told so at once and asks in its own way.
use super::*;
use airband_core::agent::{PermissionAsk, PermissionState};

/// A harness asks. The question goes into the feed, and the id comes back for fetching the answer.
pub(super) fn ask(app: &Arc<App>, ask: PermissionAsk) -> PermissionState {
    let nobody = PermissionState { state: "nobody".into(), ..PermissionState::default() };
    if ask.schema != agent::SCHEMA || app.cfg.permission_wait == 0 {
        return nobody;
    }
    let mut inner = app.inner.lock().unwrap();
    // no radio is following the chat or on a call: there is nobody to ask
    if !inner.following() && !inner.connected() {
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
    let earlier: Vec<String> = inner.permits.iter().filter(|(_, p)| p.agent_id == agent_id && p.decided.is_none()).map(|(id, _)| id.clone()).collect();
    for id in earlier {
        inner.close_permit(&id, Ended::Replaced);
    }
    inner.no_longer_waiting(&agent_id);
    let (id, entry_id) = (config::random_hex(8), config::random_hex(8));
    let what = ask.what.trim();
    // a question of the agent's is put as the question it is, with its own answers; anything else as "may I"
    let (line, options, detail) = match &ask.question {
        Some(q) => {
            log(format!("{}: asks him a question", session.callsign.text()));
            let choices: Vec<String> = q.choices.iter().filter(|c| c.chars().count() <= 28).take(4).cloned().collect();
            let all = if q.choices.is_empty() { q.text.clone() } else { format!("{}\n\n{}", q.text, q.choices.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")) };
            // a long question is shown by its opening; all of it is one tap away, as for any message
            let line = if q.text.chars().count() > 220 { format!("{}…", q.text.chars().take(219).collect::<String>().trim_end()) } else { q.text.clone() };
            (line, if choices.len() == q.choices.len() { choices } else { Vec::new() }, all)
        }
        None => {
            log(format!("{}: asks permission to {what}", session.callsign.text()));
            let line = format!("Wants to {what}.{ASKS}");
            let detail = if ask.detail.trim().is_empty() { line.clone() } else { ask.detail.clone() };
            (line, vec![ALLOW.into(), DENY.into()], detail)
        }
    };
    // on a call it is read out like any other question
    inner.record(Entry { short: Some(line), options, permit: Some(id.clone()), leave: ask.question.is_none(), aloud: Aloud::Waiting, ..Entry::said(entry_id.clone(), &session, Some(Kind::NeedsYou), detail) });
    inner.permits.insert(id.clone(), Permit { agent_id, entry: entry_id.clone(), decided: None, asked: Instant::now(), polled: Instant::now(), question: ask.question.is_some() });
    // he had the chat open a moment ago and may have put the phone down since: it is woken if so
    push::consider(app, &mut inner, &entry_id);
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
            let Some(permit) = inner.permits.get_mut(id) else { return PermissionState { id: id.into(), state: "gone".into(), ..PermissionState::default() } };
            permit.polled = Instant::now();
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
    inner.close_permit(id, Ended::AtKeyboard);
    app.save(&inner);
    app.kick.notify_one();
}

/// How a request ended (`Inner::close_permit`).
pub(super) enum Ended {
    /// He answered: yes or no, and his reason with a no.
    Answered(bool, String),
    /// The harness stopped waiting for the phone and is asking at its keyboard.
    AtKeyboard,
    /// The same agent asked something newer.
    Replaced,
    /// Nobody asks after it any more: it was answered at the keyboard, or the agent has gone.
    Gone,
}


/// What the Allow and Deny buttons send: the two answers that need no reading.
pub(super) use airband_core::radio::{ALLOW, DENY};
/// How the line of a request for leave ends while it waits, and what takes its place once it is over.
pub(super) const ASKS: &str = " Allow?";
pub(super) const MOVED: &str = " It is asking at the keyboard now.";

/// What `re` is, as far as permission goes.
pub(super) enum Leave {
    /// Not a request for leave: an ordinary message, or a question of the agent's (whatever he says to that
    /// is its answer).
    No,
    /// A request for leave the harness is still waiting on.
    Waiting,
    /// A request for leave that is over: answered, withdrawn, replaced by a newer one, or from before the
    /// tower last started. Nothing he says to it now allows or refuses anything, and nothing is passed on.
    Over,
}

/// Is what he just said a reply to a request for leave, and does that request still wait? A request whose
/// harness has stopped asking after it is closed here, so he is never told "Allowed" about one nobody will
/// collect.
pub(super) fn asked_leave(app: &App, re: Option<&str>) -> Leave {
    let mut inner = app.inner.lock().unwrap();
    let Some(entry) = re else { return Leave::No };
    if !inner.log.iter().any(|e| e.id == entry && e.leave) {
        return Leave::No;
    }
    match inner.permits.iter().find(|(_, p)| p.entry == entry && p.decided.is_none()).map(|(id, p)| (id.clone(), p.live())) {
        Some((_, true)) => Leave::Waiting,
        Some((id, false)) => {
            inner.close_permit(&id, Ended::Gone);
            app.save(&inner);
            Leave::Over
        }
        None => Leave::Over,
    }
}

/// His answer to a request, said in reply to it. `verdict` is what his words came to for a request for leave:
/// yes, no, or (None) his own words that are not a yes, which refuse it and go back as the reason.
pub(super) fn answer(app: &App, text: &str, re: Option<&str>, verdict: Option<bool>) -> Option<Reply> {
    let say = |s: &str| Some(Reply { say: s.into(), listen: false, ..Reply::default() });
    let mut inner = app.inner.lock().unwrap();
    let waiting = |p: &Permit| p.decided.is_none();
    let on_question = re.and_then(|r| inner.permits.iter().find(|(_, p)| p.entry == r && waiting(p)).map(|(id, _)| id.clone()));
    let id = match on_question {
        Some(id) => id,
        // Not said to a request: it answers none. A "yes" or an "ok" he says to someone else, or to nobody in
        // particular, must never let a command run.
        None => return None,
    };
    let question = inner.permits[&id].question;
    // to a question of the agent's, whatever he says is the answer
    let (allow, reason) = match verdict.filter(|_| !question) {
        Some(allow) => (allow, String::new()),
        None => (false, text.trim().to_string()),
    };
    let (agent_id, entry) = (inner.permits[&id].agent_id.clone(), inner.permits[&id].entry.clone());
    inner.close_permit(&id, Ended::Answered(allow, reason.clone()));
    let Some(session) = inner.find(&agent_id).cloned() else { return say("That agent has gone.") };
    log(format!("← radio: \"{text}\" → {} is {} {}", session.callsign.text(), if question { "answered" } else if allow { "allowed" } else { "refused" }, if reason.is_empty() || question { String::new() } else { "with his reason".into() }));
    let shown = if allow { ALLOW.to_string() } else if reason.is_empty() { DENY.into() } else { reason };
    inner.record(Entry::his(config::random_hex(8), &[session.clone()], shown));
    inner.touch(&entry);
    app.save(&inner);
    // his answer was taken as he gave it, and is in the chat as his own line: the words only confirm it
    say(if question { "Sent to it." } else if allow { "Allowed." } else { "Refused." }).map(|r| Reply { done: true, ..r })
}
