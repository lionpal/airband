//! Permission, asked from the phone. A harness that would stop and ask at the keyboard can ask the tower first:
//! the request goes into the feed as a question with Allow and Deny, and his answer goes back to the harness.
//!
//! What this must never do is decide. The tower passes on his answer and nothing else: there is no default, no
//! "whatever you think best", and nothing a model says counts as a yes. The line he reads is built from what the
//! harness sent, word for word, not rewritten, because it is what will run on his computer. If no radio is
//! there to be asked, the harness is told so at once and asks in its own way.
use super::*;
use airband_core::agent::{PermissionAsk, PermissionState};

/// One request for permission.
pub(super) struct Permit {
    pub(super) agent_id: String,
    /// The log line that puts it to him.
    pub(super) entry: String,
    /// His answer, once he has given it: allowed or not, and his reason if he refused with one.
    pub(super) decided: Option<(bool, String)>,
    pub(super) asked: Instant,
    /// When the harness last asked where this stands. It asks every few seconds while it waits; one that has
    /// stopped asking has stopped waiting.
    pub(super) polled: Instant,
    /// It is a question of the agent's, not a request for leave: whatever he says is the answer.
    pub(super) question: bool,
}
impl Permit {
    /// Is the harness still waiting on this? A decided one waits to be collected; an undecided one is over
    /// when the harness has stopped asking after it, or an hour has gone by.
    pub(super) fn live(&self) -> bool {
        self.asked.elapsed() < Duration::from_secs(3600) && (self.decided.is_some() || self.polled.elapsed() < Duration::from_secs(60))
    }
}

/// A harness asks. The question goes into the feed, and the id comes back for fetching the answer.
pub(super) fn ask(app: &Arc<App>, ask: PermissionAsk) -> PermissionState {
    let nobody = PermissionState { state: agent::Leave::Nobody, ..PermissionState::default() };
    if ask.schema != agent::SCHEMA || app.cfg().permission_wait == 0 {
        return nobody;
    }
    let mut inner = app.change();
    // no radio is following the chat or on a call: there is nobody to ask
    if !inner.following() && !inner.connected() {
        return nobody;
    }
    if inner.check_in(&ask.agent).is_none() {
        return nobody;
    }
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
    let (line, options, detail, more) = match &ask.question {
        Some(q) => {
            log(format!("{}: asks him a question", session.callsign.text()));
            let choices: Vec<String> = q.choices.iter().filter(|c| c.chars().count() <= 28).take(4).cloned().collect();
            let all = if q.choices.is_empty() { q.text.clone() } else { format!("{}\n\n{}", q.text, q.choices.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")) };
            // a long question is shown by its opening; all of it is one tap away, as for any message
            let line = airband_core::prose::clip(&q.text, 220);
            let more = !q.choices.is_empty() || q.text.chars().count() > 220;
            (line, if choices.len() == q.choices.len() { choices } else { Vec::new() }, all, more)
        }
        None => {
            log(format!("{}: asks permission to {what}", session.callsign.text()));
            let line = Request { what: Some(what.to_string()), at_keyboard: false }.line().unwrap_or_default();
            let more = !ask.detail.trim().is_empty();
            let detail = if more { ask.detail.clone() } else { line.clone() };
            (line, vec![ALLOW.into(), DENY.into()], detail, more)
        }
    };
    // on a call it is read out like any other question
    inner.record(Entry { short: Some(line), more, options, permit: Some(id.clone()), leave: ask.question.is_none().then(|| Request { what: Some(what.to_string()), at_keyboard: false }), aloud: Aloud::Waiting, ..Entry::said(entry_id.clone(), &session, Some(Kind::NeedsYou), detail) });
    inner.permits.insert(id.clone(), Permit { agent_id, entry: entry_id.clone(), decided: None, asked: Instant::now(), polled: Instant::now(), question: ask.question.is_some() });
    // he had the chat open a moment ago and may have put the phone down since: it is woken if so
    push::consider(app, &mut inner, &entry_id);
    inner.tell();
    PermissionState { id, state: agent::Leave::Pending, message: String::new(), wait_seconds: app.cfg().permission_wait }
}

/// Where a request stands. Held open for up to `wait` seconds while there is no answer.
pub(super) async fn state(app: &Arc<App>, id: &str, wait: u64) -> PermissionState {
    let until = Instant::now() + Duration::from_secs(wait.min(airband_core::radio::wire::FeedQuery::WAIT_MOST as u64));
    loop {
        {
            let mut inner = app.change();
            let Some(permit) = inner.permits.get_mut(id) else { return PermissionState { id: id.into(), state: agent::Leave::Gone, ..PermissionState::default() } };
            permit.polled = Instant::now();
            if let Some((allow, message)) = permit.decided.clone() {
                // handed over: the harness has its answer
                inner.permits.remove(id);
                return PermissionState { id: id.into(), state: if allow { agent::Leave::Allow } else { agent::Leave::Deny }, message, wait_seconds: 0 };
            }
        }
        if Instant::now() >= until {
            return PermissionState { id: id.into(), state: agent::Leave::Pending, message: String::new(), wait_seconds: app.cfg().permission_wait };
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The harness has stopped waiting and is asking in its own way. The question in the feed is no longer one he
/// can answer from here.
pub(super) fn withdraw(app: &App, id: &str) {
    let mut inner = app.change();
    inner.close_permit(id, Ended::AtKeyboard);
    inner.tell();
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
const ASKS: &str = " Allow?";
const MOVED: &str = " It is asking at the keyboard now.";

/// A line that is, or was, a request for leave: what it asked to do, and whether it has gone back to the
/// keyboard. The line's words are made from this, so its ending is never read back out of its text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Request {
    /// What it asked leave to do. Nothing for a line kept before this was written down.
    pub(super) what: Option<String>,
    /// The request is over and the agent asks at its own keyboard now.
    pub(super) at_keyboard: bool,
}
impl Request {
    /// The line he sees for it, when what it asked is known.
    pub(super) fn line(&self) -> Option<String> {
        self.what.as_ref().map(|what| format!("Wants to {what}.{}", if self.at_keyboard { MOVED } else { ASKS }))
    }
}

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
    let mut inner = app.change();
    let Some(entry) = re else { return Leave::No };
    if !inner.log.iter().any(|e| e.id == entry && e.leave.is_some()) {
        return Leave::No;
    }
    match inner.permits.iter().find(|(_, p)| p.entry == entry && p.decided.is_none()).map(|(id, p)| (id.clone(), p.live())) {
        Some((_, true)) => Leave::Waiting,
        Some((id, false)) => {
            inner.close_permit(&id, Ended::Gone);
            Leave::Over
        }
        None => Leave::Over,
    }
}

/// His answer to a request, said in reply to it. `verdict` is what his words came to for a request for leave:
/// yes, no, or (None) his own words that are not a yes, which refuse it and go back as the reason.
pub(super) fn answer(app: &App, text: &str, re: Option<&str>, verdict: Option<bool>) -> Option<Reply> {
    let say = |s: &str| Some(Reply { say: s.into(), listen: false, ..Reply::default() });
    let mut inner = app.change();
    let waiting = |p: &Permit| p.decided.is_none();
    let on_question = re.and_then(|r| inner.permits.iter().find(|(_, p)| p.entry == r && waiting(p)).map(|(id, _)| id.clone()));
    let id = on_question?;
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
    inner.record(Entry::his(config::random_hex(8), std::slice::from_ref(&session), shown));
    inner.touch(&entry);
    // his answer was taken as he gave it, and is in the chat as his own line: the words only confirm it
    say(if question { "Sent to it." } else if allow { "Allowed." } else { "Refused." }).map(|r| Reply { done: true, ..r })
}

impl Inner {
    /// A request ends: he answered it, the harness stopped waiting and asks at its keyboard, it asked
    /// something newer, or it is gone. This is the one way any of those happens. The line stops offering an
    /// answer (and, unanswered, says where the question went), it leaves the call queue, and the agent is
    /// waiting on him from now only if it is asking at its keyboard.
    pub(super) fn close_permit(&mut self, id: &str, ended: permit::Ended) {
        let (agent_id, entry) = match &ended {
            // his answer waits here for the harness to collect it
            permit::Ended::Answered(allow, reason) => {
                let Some(permit) = self.permits.get_mut(id) else { return };
                permit.decided = Some((*allow, reason.clone()));
                (permit.agent_id.clone(), permit.entry.clone())
            }
            _ => {
                let Some(permit) = self.permits.remove(id) else { return };
                (permit.agent_id, permit.entry)
            }
        };
        match ended {
            // answered: a request for leave offers nothing more
            permit::Ended::Answered(..) => self.end_line(&entry, Dropped::Answered, |e| {
                if e.leave.is_some() {
                    e.options.clear();
                }
            }),
            _ => self.close_line(&entry),
        }
        if let Some(s) = self.session(&agent_id) {
            s.waiting = matches!(ended, permit::Ended::AtKeyboard | permit::Ended::Replaced);
        }
    }
    /// At start: no harness is waiting on anything asked before the tower stopped, so every request for leave
    /// still offering an answer is over.
    pub(super) fn close_requests_from_before(&mut self) {
        let open: Vec<String> = self.log.iter().filter(|e| e.leave.is_some() && !e.options.is_empty()).map(|e| e.id.clone()).collect();
        for id in open {
            self.close_line(&id);
        }
    }
    /// The line of a request that is over.
    pub(super) fn close_line(&mut self, entry: &str) {
        self.end_line(entry, Dropped::PermissionEnded, |e| {
            e.options.clear();
            if let Some(request) = e.leave.as_mut() {
                request.at_keyboard = true;
                if let Some(line) = request.line() {
                    e.short = Some(line);
                }
            }
        });
    }
    /// A line that was waiting on a request stops waiting: it is changed as `change` says, it leaves the
    /// call queue for the reason given if he had not heard it yet, and every radio sees it again.
    fn end_line(&mut self, entry: &str, why: Dropped, change: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.log.iter_mut().find(|e| e.id == entry) {
            e.permit = None;
            change(e);
        }
        if self.log.iter().any(|e| e.id == entry && e.aloud.unheard()) {
            self.set_aloud(entry, Aloud::dropped(why));
        }
        self.touch(entry);
    }
}
