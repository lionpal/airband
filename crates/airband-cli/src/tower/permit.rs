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
    /// When the tool stops waiting on it, in seconds since 1970, if the tool said.
    pub(super) until: Option<u64>,
}
impl Permit {
    /// How it ended once nobody waits on it: past the time its tool gave it, the tool settled it itself;
    /// otherwise nobody asks after it any more.
    pub(super) fn unwaited(&self) -> Ended {
        if self.until.is_some_and(|t| airband_core::clock::now_secs() >= t) { Ended::RanOut } else { Ended::Gone }
    }
    /// Is the harness still waiting on this? A decided one waits to be collected; an undecided one is over
    /// when the harness has stopped asking after it, the time its tool gave it is up, or an hour has gone by.
    pub(super) fn live(&self) -> bool {
        let in_time = self.until.is_none_or(|t| airband_core::clock::now_secs() < t);
        self.asked.elapsed() < Duration::from_secs(3600) && (self.decided.is_some() || (in_time && self.polled.elapsed() < Duration::from_secs(60)))
    }
}

/// Why a request was not put to him, as the log says it. The harness then asks at its own keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NotAsked {
    /// It came in a shape this tower does not read.
    OtherSchema,
    /// His settings say requests are not put to him (`permission_wait` is 0).
    TurnedOff,
    /// No radio is following the chat or on a call, and no phone has said where it can be woken.
    NobodyToAsk,
    /// The agent it is for could not be put on the radio.
    NoAgent,
    /// The time the tool gave it was over before it came.
    PastItsTime,
}

impl NotAsked {
    fn said(self) -> &'static str {
        match self {
            NotAsked::OtherSchema => "it came in a shape this tower does not read",
            NotAsked::TurnedOff => "putting requests to him is turned off (permission_wait is 0)",
            NotAsked::NobodyToAsk => "no radio is there and no phone can be woken",
            NotAsked::NoAgent => "the agent it is for could not be put on the radio",
            NotAsked::PastItsTime => "the time its tool gave it was over before it came",
        }
    }
}

/// A harness asks. The question goes into the feed, and the id comes back for fetching the answer.
pub(super) fn ask(app: &Arc<App>, ask: PermissionAsk) -> PermissionState {
    // A tool that settles its own prompt meanwhile writes down how: watched here, not by the hook, which the tool
    // can end with no word. Its first look is now, as the request comes: the call it is for is the one open now,
    // or the next one written.
    let watch = ask.settled_in.as_ref().and_then(|record| crate::adapters::settled(&ask.agent.harness, record)).map(|mut watch| {
        // a first look finds the call; nothing is settled before the call it is for is known
        let _first: Option<crate::adapters::Settled> = watch();
        watch
    });
    match put(app, &ask, watch) {
        Ok(state) => state,
        Err(why) => {
            log(format!("{}: asks permission; not put to him, so it is asked at the keyboard: {}", short_id(&ask.agent.id), why.said()));
            PermissionState { state: agent::Leave::Nobody, ..PermissionState::default() }
        }
    }
}

fn put(app: &Arc<App>, ask: &PermissionAsk, watch: Option<crate::adapters::Watch>) -> Result<PermissionState, NotAsked> {
    if ask.schema != agent::SCHEMA {
        return Err(NotAsked::OtherSchema);
    }
    if app.cfg().permission_wait == 0 {
        return Err(NotAsked::TurnedOff);
    }
    if wait_for(app.cfg().permission_wait, ask.until) == 0 {
        return Err(NotAsked::PastItsTime);
    }
    let mut inner = app.change();
    // no radio is following the chat or on a call, and no phone can be woken to be told: there is nobody to ask
    if !inner.following() && !inner.connected() && !app.push.can_wake(&app.cfg()) {
        return Err(NotAsked::NobodyToAsk);
    }
    if inner.check_in(&ask.agent).is_none() {
        return Err(NotAsked::NoAgent);
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
    // The tool's nudge that it asks leave at its keyboard can come before its request does (they come by two
    // hooks, in no set order). When the agent's latest line is that nudge, the card takes its place: one line,
    // the request itself.
    let nudge = ask.question.is_none().then(|| inner.log.iter().rev().find(|e| e.agent.as_deref() == Some(agent_id.as_str()))).flatten().filter(|e| e.note == Some(airband_core::radio::Note::LeaveAtKeyboard)).map(|e| e.id.clone());
    if let Some(line) = &nudge {
        log(format!("{}: its request takes the place of its tool's nudge that it asks at the keyboard", short_id(&agent_id)));
        inner.log.retain(|e| &e.id != line);
        // the card is a notification of its own, not the nudge's
        inner.pushed.remove(line);
    }
    let (id, entry_id) = (config::random_hex(8), nudge.unwrap_or_else(|| config::random_hex(8)));
    let what = ask.what.trim();
    // a question of the agent's is put as the question it is, with its own answers; anything else as "may I"
    let (line, options, detail, more) = match &ask.question {
        Some(q) => {
            log(format!("{}: asks him a question", short_id(&session.agent.id)));
            let choices: Vec<String> = q.choices.iter().filter(|c| c.chars().count() <= 28).take(4).cloned().collect();
            let all = if q.choices.is_empty() { q.text.clone() } else { format!("{}\n\n{}", q.text, q.choices.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")) };
            // a long question is shown by its opening; all of it is one tap away, as for any message
            let line = airband_core::prose::clip(&q.text, 220);
            let more = !q.choices.is_empty() || q.text.chars().count() > 220;
            (line, if choices.len() == q.choices.len() { choices } else { Vec::new() }, all, more)
        }
        None => {
            log(format!("{}: asks permission ({} characters)", short_id(&session.agent.id), what.chars().count()));
            let line = Request::waiting(what).line().unwrap_or_default();
            let more = !ask.detail.trim().is_empty();
            let detail = if more { ask.detail.clone() } else { line.clone() };
            (line, vec![ALLOW.into(), DENY.into()], detail, more)
        }
    };
    // on a call it is read out like any other question
    inner.record(Entry { short: Some(line), more, options, permit: Some(id.clone()), leave: ask.question.is_none().then(|| Request::waiting(what)), aloud: Aloud::Waiting, ..Entry::said(entry_id.clone(), &session, Some(Kind::NeedsYou), detail) });
    inner.permits.insert(id.clone(), Permit { agent_id, entry: entry_id.clone(), decided: None, asked: Instant::now(), polled: Instant::now(), question: ask.question.is_some(), until: ask.until });
    // he had the chat open a moment ago and may have put the phone down since: it is woken if so
    push::consider(app, &mut inner, &entry_id);
    inner.tell();
    if let Some(watch) = watch {
        watch_settled(app.clone(), id.clone(), entry_id.clone(), watch);
    }
    Ok(PermissionState { id, state: agent::Leave::Pending, message: String::new(), wait_seconds: wait_for(app.cfg().permission_wait, ask.until) })
}

/// How often a tool's record of a call is looked at while its request is open.
const RECORD_LOOKED_AT: Duration = Duration::from_secs(1);

/// Watch the tool's record of the call request `id` (its line `entry`) is for, until the request is over. Settled
/// at the tool while it is on his phone, it is taken back saying how; settled after it moved to the keyboard
/// (his time to answer from the phone ran out first), its line says how it ended there. Answered from the phone,
/// or replaced, or closed any other way, there is nothing more to watch.
fn watch_settled(app: Arc<App>, id: String, entry: String, mut watch: crate::adapters::Watch) {
    use crate::adapters::Settled;
    let (request, line) = (id.clone(), entry.clone());
    let open = move |app: &App| {
        let inner = app.read();
        let on_phone = inner.permits.get(&request).is_some_and(|p| p.decided.is_none());
        let at_keyboard = || inner.log.iter().any(|e| e.id == line && e.leave.as_ref().is_some_and(|r| r.ended == Some(RequestEnded::AtKeyboard)));
        (on_phone, on_phone || at_keyboard())
    };
    std::thread::spawn(move || {
        loop {
            if !open(&app).1 {
                return;
            }
            let Some(how) = watch() else {
                std::thread::sleep(RECORD_LOOKED_AT);
                continue;
            };
            let (why, ended) = match how {
                Settled::There => (airband_core::radio::wire::Withdrawn::SettledThere, RequestEnded::SettledThere),
                Settled::RefusedByTool => (airband_core::radio::wire::Withdrawn::RefusedByTool, RequestEnded::RefusedByTool),
            };
            match open(&app) {
                (true, _) => withdraw(&app, &id, why),
                (false, true) => {
                    let mut inner = app.change();
                    log(format!("{}: settled at its tool after it moved to the keyboard: {}", short_id(&id), if how == Settled::RefusedByTool { "its tool refused it" } else { "at the keyboard" }));
                    inner.close_line(&entry, ended);
                    inner.tell();
                }
                (false, false) => {}
            }
            return;
        }
    });
}

/// How long the harness is to wait on him now: his setting, and never past the tool's own time for it.
fn wait_for(setting: u64, until: Option<u64>) -> u64 {
    until.map_or(setting, |t| setting.min(t.saturating_sub(airband_core::clock::now_secs())))
}

/// Where a request stands. Held open for up to `wait` seconds while there is no answer.
pub(super) async fn state(app: &Arc<App>, id: &str, wait: u64) -> PermissionState {
    let until = tokio::time::Instant::now() + Duration::from_secs(wait.min(airband_core::radio::wire::FeedQuery::WAIT_MOST as u64));
    // the harness is asking now, and again when this answers: that is what says it still waits on him
    let polled = |app: &App| {
        if let Some(permit) = app.change().permits.get_mut(id) {
            permit.polled = Instant::now();
        }
    };
    polled(app);
    let mut changes = app.watch_changes();
    loop {
        let (decided, tool_until) = match app.read().permits.get(id) {
            None => return PermissionState { id: id.into(), state: agent::Leave::Gone, ..PermissionState::default() },
            Some(permit) => (permit.decided.clone(), permit.until),
        };
        if let Some((allow, message)) = decided {
            // handed over: the harness has its answer
            app.change().permits.remove(id);
            return PermissionState { id: id.into(), state: if allow { agent::Leave::Allow } else { agent::Leave::Deny }, message, wait_seconds: 0 };
        }
        // woken when the state changes, and only then looked at again
        if tokio::time::timeout_at(until, changes.changed()).await.is_err() {
            polled(app);
            return PermissionState { id: id.into(), state: agent::Leave::Pending, message: String::new(), wait_seconds: wait_for(app.cfg().permission_wait, tool_until) };
        }
    }
}

/// The harness has stopped waiting: it asks in its own way, or the request was settled there. The question in
/// the feed is no longer one he can answer from here, and says which.
pub(super) fn withdraw(app: &App, id: &str, why: airband_core::radio::wire::Withdrawn) {
    use airband_core::radio::wire::Withdrawn;
    let mut inner = app.change();
    if let Some(p) = inner.permits.get(id) {
        let agent = p.agent_id.clone();
        log(format!("{}: its request for leave is taken back: {}", short_id(&agent), match why {
            Withdrawn::Unanswered => "no answer from the phone in time; it asks at the keyboard",
            Withdrawn::SettledThere => "it was settled at the keyboard",
            Withdrawn::RanOut => "the time its tool gave it ran out",
            Withdrawn::RefusedByTool => "its tool refused it itself",
        }));
    }
    inner.close_permit(id, match why {
        Withdrawn::Unanswered => Ended::AtKeyboard,
        Withdrawn::SettledThere => Ended::SettledThere,
        Withdrawn::RanOut => Ended::RanOut,
        Withdrawn::RefusedByTool => Ended::RefusedByTool,
    });
    inner.tell();
}

/// How a request ended (`Inner::close_permit`).
pub(super) enum Ended {
    /// He answered: yes or no, his reason with a no, and the radio he answered from.
    Answered(bool, String, String),
    /// The harness stopped waiting for the phone and is asking at its keyboard.
    AtKeyboard,
    /// The tool says it was settled at its keyboard: answered there, or refused by the tool itself.
    SettledThere,
    /// The time the tool gave it ran out: the tool settled it itself.
    RanOut,
    /// The tool refused it itself, by its own rules, with nobody's answer.
    RefusedByTool,
    /// The same agent asked something newer.
    Replaced,
    /// Nobody asks after it any more: it was answered at the keyboard, or the agent has gone.
    Gone,
}

/// What the Allow and Deny buttons send: the two answers that need no reading.
pub(super) use airband_core::radio::{ALLOW, DENY};
/// How the line of a request for leave ends while it waits, and what takes its place once it is over.
const ASKS: &str = " Allow?";
const MOVED: &str = " It's asking at the keyboard now.";
const SETTLED: &str = " Settled at the keyboard.";
const RAN_OUT: &str = " Its time ran out.";
const TOOL_REFUSED: &str = " Its tool refused it.";

/// A line that is, or was, a request for leave: what it asked to do, and how it ended once it is over. The
/// line's words are made from this, so its ending is never read back out of its text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Request {
    /// What it asked leave to do. Nothing for a line kept before this was written down.
    pub(super) what: Option<String>,
    /// How the request ended without his answer; nothing while it waits, or once he answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ended: Option<RequestEnded>,
}

/// How a request for leave ended, when it was not by his answer from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum RequestEnded {
    /// The agent asks at its own keyboard now.
    AtKeyboard,
    /// It was settled at the tool's own keyboard.
    SettledThere,
    /// The time the tool gave it ran out, and the tool settled it itself.
    RanOut,
    /// The tool refused it itself, by its own rules, with nobody's answer.
    RefusedByTool,
    /// He allowed it, from a radio.
    Allowed,
    /// He refused it, from a radio.
    Refused,
}

impl Request {
    /// A request still waiting on him.
    pub(super) fn waiting(what: &str) -> Request {
        Request { what: Some(what.to_string()), ended: None }
    }
    /// The line he sees for it, when what it asked is known.
    pub(super) fn line(&self) -> Option<String> {
        let ending = match self.ended {
            None => ASKS,
            Some(RequestEnded::AtKeyboard) => MOVED,
            Some(RequestEnded::SettledThere) => SETTLED,
            Some(RequestEnded::RanOut) => RAN_OUT,
            Some(RequestEnded::RefusedByTool) => TOOL_REFUSED,
            Some(RequestEnded::Allowed) => " Allowed.",
            Some(RequestEnded::Refused) => " Refused.",
        };
        self.what.as_ref().map(|what| format!("Wants to {what}.{ending}"))
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
            let ended = inner.permits[&id].unwaited();
            inner.close_permit(&id, ended);
            Leave::Over
        }
        None => Leave::Over,
    }
}

/// His answer to a request, said in reply to it. `verdict` is what his words came to for a request for leave:
/// yes, no, or (None) his own words that are not a yes, which refuse it and go back as the reason. `said` is the
/// id the radio sent his answer under: his line in the chat takes it.
pub(super) fn answer(app: &App, radio: &str, text: &str, said: &str, re: Option<&str>, verdict: Option<bool>) -> Option<Reply> {
    let say = |s: &str| Some(reply(s, false));
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
    inner.close_permit(&id, Ended::Answered(allow, reason.clone(), radio.to_string()));
    let Some(session) = inner.find(&agent_id).cloned() else { return say("That agent has gone.") };
    log(format!("← radio: {} characters → {} is {} {}", text.chars().count(), short_id(&session.agent.id), if question { "answered" } else if allow { "allowed" } else { "refused" }, if reason.is_empty() || question { String::new() } else { "with his reason".into() }));
    let shown = if allow { ALLOW.to_string() } else if reason.is_empty() { DENY.into() } else { reason };
    // under the id the radio sent it with, as any line of his: the radio shows it as on its way until the chat
    // has that id, and would otherwise show his answer twice
    let line = inner.his_line_id(said);
    inner.record(Entry::his(line, std::slice::from_ref(&session), shown));
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
        let ended_before = self.ended.len();
        let (agent_id, entry) = match &ended {
            // his answer waits here for the harness to collect it
            permit::Ended::Answered(allow, reason, _) => {
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
            // answered: a request for leave offers nothing more, and says what he said
            permit::Ended::Answered(allow, ..) => self.end_line(&entry, Dropped::Answered, |e| {
                if let Some(request) = e.leave.as_mut() {
                    e.options.clear();
                    request.ended = Some(if allow { permit::RequestEnded::Allowed } else { permit::RequestEnded::Refused });
                    if let Some(line) = request.line() {
                        e.short = Some(line);
                    }
                }
            }),
            permit::Ended::SettledThere => self.close_line(&entry, permit::RequestEnded::SettledThere),
            permit::Ended::RanOut => self.close_line(&entry, permit::RequestEnded::RanOut),
            permit::Ended::RefusedByTool => self.close_line(&entry, permit::RequestEnded::RefusedByTool),
            _ => self.close_line(&entry, permit::RequestEnded::AtKeyboard),
        }
        // answered from a radio: that radio has it done with already, and only the others have its
        // notification taken down
        if let permit::Ended::Answered(_, _, radio) = &ended {
            super::push::done_on(self, ended_before, radio);
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
            self.close_line(&id, permit::RequestEnded::AtKeyboard);
        }
    }
    /// The line of a request that is over.
    pub(super) fn close_line(&mut self, entry: &str, how: permit::RequestEnded) {
        self.end_line(entry, Dropped::PermissionEnded, |e| {
            e.options.clear();
            if let Some(request) = e.leave.as_mut() {
                request.ended = Some(how);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// With no radio heard from lately, a request for leave is put to him only when a phone can be woken.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_phone_that_can_be_woken_is_someone_to_ask_with_no_radio_there() {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-wake", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 30 }));
        let ask = || PermissionAsk { schema: agent::SCHEMA.into(), agent: airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "test") }, what: "run `ls`".into(), detail: String::new(), question: None, until: None, settled_in: None };
        super::super::logged::take();
        assert_eq!(super::ask(&app, ask()).state, agent::Leave::Nobody, "nobody there and nothing to wake");
        // and the log says so, by the agent and why: the keyboard asks, and it is never silent
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: asks permission; not put to him, so it is asked at the keyboard: no radio is there and no phone can be woken")));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let asked = super::ask(&app, ask());
        assert_eq!(asked.state, agent::Leave::Pending, "the phone is told of it");
        let entry = app.read().log.iter().find(|e| e.permit.as_deref() == Some(asked.id.as_str())).map(|e| e.id.clone()).unwrap();
        // the phone is to be woken for it: the tower has taken it on (sent once the short wait for a radio is over)
        assert!(app.read().pushed.contains(&entry), "a notification is asked for");
        // the tool's own nudge that it asks at its keyboard tells him nothing the card does not: no second card
        let lines = app.read().log.len();
        let nudge = |leave: bool| -> airband_core::agent::AgentEvent {
            serde_json::from_value(serde_json::json!({ "schema": agent::SCHEMA, "agent": ask().agent, "event": { "type": "notice", "text": "Claude needs your permission", "leave": leave } })).unwrap()
        };
        super::super::intake::on_event(&app, nudge(true)).unwrap();
        assert_eq!(app.read().log.len(), lines, "no card for the nudge");
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("its tool asks at the keyboard for what is on his phone already; not told again")));
        // a notice of another kind is still shown
        super::super::intake::on_event(&app, nudge(false)).unwrap();
        assert_eq!(app.read().log.len(), lines + 1);
        // settled at the tool's own keyboard: the card says so at once, and an answer to it now does nothing
        let again = super::ask(&app, ask());
        let card = app.read().log.iter().find(|e| e.permit.as_deref() == Some(again.id.as_str())).map(|e| e.id.clone()).unwrap();
        super::withdraw(&app, &again.id, airband_core::radio::wire::Withdrawn::SettledThere);
        let line = app.read().log.iter().find(|e| e.id == card).cloned().unwrap();
        assert_eq!((line.short.as_deref(), line.options.is_empty()), (Some("Wants to run `ls`. Settled at the keyboard."), true));
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: its request for leave is taken back: it was settled at the keyboard")));
        assert!(matches!(super::asked_leave(&app, Some(&card)), Leave::Over));
        // the phone woken for each request that ended without his answer from it is to be told: the one replaced
        // by a newer request, and the one settled at the keyboard
        let ended = |id: &str| app.read().ended.iter().any(|e| e.id == id && e.by.is_none() && e.why == airband_core::radio::Withdrawn::NotRead { reason: Dropped::PermissionEnded });
        assert!(ended(&entry) && ended(&card), "{:?}", app.read().ended);
        let lines = app.read().log.len();
        // once nothing is waiting on his phone, the nudge is how he learns the keyboard wants him
        super::super::intake::on_event(&app, nudge(true)).unwrap();
        assert_eq!(app.read().log.len(), lines + 1);
    }

    /// His Allow is one line in the chat, under the id his radio sent it with: the radio shows it as on its way
    /// until the chat has that id, so a line under any other id would show his answer twice.
    #[tokio::test(flavor = "multi_thread")]
    async fn his_answer_to_a_request_is_his_line_under_the_id_his_radio_sent_it_with() {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-line-id", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 30 }));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let ask = || PermissionAsk { schema: agent::SCHEMA.into(), agent: airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "test") }, what: "run `ls`".into(), detail: String::new(), question: None, until: None, settled_in: None };
        let card_of = |id: &str| app.read().log.iter().find(|e| e.permit.as_deref() == Some(id)).map(|e| e.id.clone()).unwrap();
        let his = || app.read().log.iter().filter(|e| e.his).map(|e| (e.id.clone(), e.full.clone())).collect::<Vec<_>>();
        let card = card_of(&super::ask(&app, ask()).id);
        let reply = super::answer(&app, "radio", ALLOW, "d0af0c1603f65c0760ed2abc", Some(&card), Some(true)).expect("it answers the request");
        assert!(reply.done);
        assert_eq!(his(), [("d0af0c1603f65c0760ed2abc".to_string(), ALLOW.to_string())]);
        // an id already in the chat is never taken twice: the line gets one of its own
        let card = card_of(&super::ask(&app, ask()).id);
        super::answer(&app, "radio", DENY, "d0af0c1603f65c0760ed2abc", Some(&card), Some(false)).expect("it answers the request");
        let lines = his();
        assert_eq!(lines.len(), 2);
        assert_ne!(lines[1].0, lines[0].0);
    }

    /// A tool that says how long it waits on a request: the harness waits no longer than that, one past it is
    /// not put to him, and one whose time ran out says so and takes no answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_waits_no_longer_than_its_tool_gives_it() {
        // a phone that can be woken: someone to ask
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-until", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 30 }));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let now = airband_core::clock::now_secs();
        let ask = |until: Option<u64>| PermissionAsk { schema: agent::SCHEMA.into(), agent: airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "test") }, what: "run `make release`".into(), detail: String::new(), question: None, until, settled_in: None };
        assert_eq!(super::ask(&app, ask(None)).wait_seconds, 30, "a tool that gives no time: his setting");
        let asked = super::ask(&app, ask(Some(now + 10)));
        assert!((9..=10).contains(&asked.wait_seconds), "{}", asked.wait_seconds);
        assert_eq!(super::state(&app, &asked.id, 0).await.wait_seconds.min(10), super::state(&app, &asked.id, 0).await.wait_seconds, "still bounded when asked again");
        // past its time before it came
        super::super::logged::take();
        assert_eq!(super::ask(&app, ask(Some(now.saturating_sub(1)))).state, agent::Leave::Nobody);
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: asks permission; not put to him, so it is asked at the keyboard: the time its tool gave it was over before it came")));
        // its time ran out: the harness says so, and the card says how it ended
        let brief = super::ask(&app, ask(Some(now + 10)));
        let card = app.read().log.iter().find(|e| e.permit.as_deref() == Some(brief.id.as_str())).map(|e| e.id.clone()).unwrap();
        super::withdraw(&app, &brief.id, airband_core::radio::wire::Withdrawn::RanOut);
        let line = app.read().log.iter().find(|e| e.id == card).cloned().unwrap();
        assert_eq!((line.short.as_deref(), line.options.is_empty()), (Some("Wants to run `make release`. Its time ran out."), true));
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: its request for leave is taken back: the time its tool gave it ran out")));
        assert!(matches!(super::asked_leave(&app, Some(&card)), Leave::Over));
        // one whose harness has not said yet, once the time is up, is over too: no answer to it counts
        let late = super::ask(&app, ask(Some(now + 10)));
        let card = app.read().log.iter().find(|e| e.permit.as_deref() == Some(late.id.as_str())).map(|e| e.id.clone()).unwrap();
        app.change().permits.get_mut(&late.id).unwrap().until = Some(now.saturating_sub(1));
        assert!(matches!(super::asked_leave(&app, Some(&card)), Leave::Over));
        assert_eq!(app.read().log.iter().find(|e| e.id == card).and_then(|e| e.short.clone()).as_deref(), Some("Wants to run `make release`. Its time ran out."));
    }

    /// A request its tool refused itself, by its own rules (Claude Code blocking it once its countdown is up), is
    /// over as that: the card says its tool refused it, a phone woken for it is told, and an answer from the
    /// phone afterwards does nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_its_tool_refused_itself_is_over_as_that() {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-stopped", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 180 }));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let asked = super::ask(&app, PermissionAsk { schema: agent::SCHEMA.into(), agent: airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "test") }, what: "run `rm -rf build`".into(), detail: String::new(), question: None, until: None, settled_in: None });
        let card = app.read().log.iter().find(|e| e.permit.as_deref() == Some(asked.id.as_str())).map(|e| e.id.clone()).unwrap();
        super::super::logged::take();
        super::withdraw(&app, &asked.id, airband_core::radio::wire::Withdrawn::RefusedByTool);
        let line = app.read().log.iter().find(|e| e.id == card).cloned().unwrap();
        assert_eq!((line.short.as_deref(), line.options.is_empty()), (Some("Wants to run `rm -rf build`. Its tool refused it."), true));
        assert_eq!(line.leave.as_ref().and_then(|r| r.ended), Some(RequestEnded::RefusedByTool));
        assert!(app.read().ended.iter().any(|e| e.id == card && e.by.is_none()), "the phones woken for it have its notification taken down");
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: its request for leave is taken back: its tool refused it itself")));
        assert!(matches!(super::asked_leave(&app, Some(&card)), Leave::Over));
        assert_eq!(super::state(&app, &asked.id, 0).await.state, agent::Leave::Gone, "and the harness, asking after it, is told it is gone");
    }

    /// The tower watches the tool's own record of the call, so a request settled at the tool is closed on the
    /// phone however the hook that asked ended (Claude Code ends it with no word when it blocks the request): at
    /// once while it is on his phone, and on the card still after it moved to the keyboard.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_settled_at_its_tool_is_seen_there_whatever_became_of_the_hook() {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-record", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 180 }));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let dir = std::env::temp_dir().join(format!("airband-record-{}", config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let input = serde_json::json!({ "command": "rm -rf build" });
        let used = |id: &str| serde_json::json!({ "message": { "content": [{ "type": "tool_use", "id": id, "name": "Bash", "input": input }] } }).to_string() + "\n";
        let blocked = |id: &str| serde_json::json!({ "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "is_error": true }] }, "permissionDecision": { "decision": "reject", "source": "config" } }).to_string() + "\n";
        let asking = |record: &std::path::Path| {
            let agent = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "claude-code") };
            let settled_in = Some(airband_core::agent::CallRecord { record: record.display().to_string(), tool: "Bash".into(), input: input.clone() });
            super::ask(&app, PermissionAsk { schema: agent::SCHEMA.into(), agent, what: "run `rm -rf build`".into(), detail: String::new(), question: None, until: None, settled_in })
        };
        let line_of = |id: &str| app.read().log.iter().find(|e| e.permit.as_deref() == Some(id)).map(|e| e.id.clone()).unwrap();
        let short = |line: &str| app.read().log.iter().find(|e| e.id == line).and_then(|e| e.short.clone()).unwrap_or_default();
        let becomes = |line: &str, want: &str| (0..50).any(|_| short(line) == want || { std::thread::sleep(Duration::from_millis(100)); false });
        // refused by the tool while on his phone, the hook that asked long gone
        let first = dir.join("first.jsonl");
        std::fs::write(&first, used("t1")).unwrap();
        let asked = asking(&first);
        let line = line_of(&asked.id);
        std::fs::write(&first, used("t1") + &blocked("t1")).unwrap();
        assert!(becomes(&line, "Wants to run `rm -rf build`. Its tool refused it."), "{}", short(&line));
        assert!(matches!(super::asked_leave(&app, Some(&line)), Leave::Over));
        // his time to answer from the phone ran out first: the card says it is at the keyboard, then how it ended
        let second = dir.join("second.jsonl");
        std::fs::write(&second, used("t2")).unwrap();
        let asked = asking(&second);
        let line = line_of(&asked.id);
        super::withdraw(&app, &asked.id, airband_core::radio::wire::Withdrawn::Unanswered);
        assert_eq!(short(&line), "Wants to run `rm -rf build`. It's asking at the keyboard now.");
        std::fs::write(&second, used("t2") + &blocked("t2")).unwrap();
        assert!(becomes(&line, "Wants to run `rm -rf build`. Its tool refused it."), "{}", short(&line));
        std::fs::remove_dir_all(dir).ok();
    }

    /// The tool's nudge and its request come by two hooks in no set order: either way he gets one card. A
    /// question of the agent's waiting does not hide a nudge about something else.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_nudge_and_its_request_make_one_card_in_whichever_order_they_come() {
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = push::channel_of(&phone).unwrap();
        let app = super::super::http::tests::app_with("permit-nudge", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone], "permission_wait": 30 }));
        push::register(&app, &channel, airband_core::radio::PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        let agent = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a1", "test") };
        let ask = |question: Option<airband_core::agent::Asked>| PermissionAsk { schema: agent::SCHEMA.into(), agent: agent.clone(), what: "run `ls`".into(), detail: String::new(), question, until: None, settled_in: None };
        let nudge = || -> airband_core::agent::AgentEvent {
            serde_json::from_value(serde_json::json!({ "schema": agent::SCHEMA, "agent": agent, "event": { "type": "notice", "text": "Claude needs your permission", "leave": true } })).unwrap()
        };
        let from_agent = |app: &App| app.read().log.iter().filter(|e| e.agent.as_deref() == Some("a1")).map(|e| (e.id.clone(), e.permit.is_some())).collect::<Vec<_>>();
        // the nudge first: the card takes its line
        super::super::intake::on_event(&app, nudge()).unwrap();
        let before = from_agent(&app);
        assert_eq!(before.len(), 1);
        super::super::logged::take();
        let asked = super::ask(&app, ask(None));
        let after = from_agent(&app);
        assert_eq!(after, [(before[0].0.clone(), true)], "one line, and it is the card");
        assert!(super::super::logged::take().iter().any(|l| l.ends_with("a1: its request takes the place of its tool's nudge that it asks at the keyboard")));
        assert!(app.read().pushed.contains(&after[0].0), "the card is sent as a notification of its own");
        super::withdraw(&app, &asked.id, airband_core::radio::wire::Withdrawn::SettledThere);
        // a question of the agent's waiting: a nudge about leave is still shown
        super::ask(&app, ask(Some(airband_core::agent::Asked { text: "Which one?".into(), choices: Vec::new() })));
        let lines = app.read().log.len();
        super::super::intake::on_event(&app, nudge()).unwrap();
        assert_eq!(app.read().log.len(), lines + 1);
    }
}
