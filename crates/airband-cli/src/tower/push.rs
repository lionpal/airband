//! Waking his phone. When a message deserves it and his phone does not already have it in front of him, the
//! tower seals a notification with that phone's pairing key and asks the relay to hand it to the phone's push
//! service. The relay and the push service pass on bytes they cannot read.
//!
//! What deserves one: a line that is news, judged to need him or to be a report, a permission card, a question
//! a harness puts to him. Never chatter, a stall, or his own words. Not to a radio that has the chat open and has
//! it: such a radio asks for what follows a line as soon as it gets it, and that request is how the tower knows.
//! A report is not sent to a radio on a call either (it is read out there); something that needs him is,
//! because a call can be held, or quiet, or in his pocket with the screen off.
//!
//! A notification lives as long as its line is news, and no longer. One line, one notification, kept by the
//! push service and the phone under the line's tag (`Keys::notice_tag`), which says nothing of the line. The
//! words are the line's as the chat shows them. The moment the line stops being news (its place in the reading
//! queue ends: heard, a newer line from its agent, his answer, the request being over, and every other reason,
//! or he has it on screen on another device) each phone woken for it is sent a sealed notification under the
//! same tag that takes it down. There is one way this happens for every line, whatever said it.
//!
//! Nothing here knows which push service a phone uses: an address is a provider's name and a token, as the
//! phone gave them. The log says when a notification was asked for and what came back, and never an address or
//! a message's words.
use super::*;
use airband_core::radio::{Notice, PushTarget, Pushed, Withdrawn, deserves_notice};
use airband_core::relay::{Direction, Keys, NotPushed, PUSH_MOST, PushAnswer, PushRequest, Route};

/// The longest the tower waits for a radio that says it is showing the chat to ask for what follows a message
/// it was just handed. That request is the fact waited on: a radio that has it asks again at once (the phone's
/// follow loop does not pause after an answer), so it comes within one round trip through the relay. A radio
/// that does not ask by then is not showing it (its app was put away with the request open): it is woken.
/// With no radio saying it shows the chat, nothing is waited for.
///
/// The bound itself is a rule of this product, not a fact: a phone whose app is put away mid-request says
/// nothing more (iOS gives it no time to), so no fact ever says "not on screen" for it. Four seconds is a
/// round trip through the relay many times over; a radio slower than that to ask is woken, which costs him a
/// notification he did not need, never one he did.
const GRACE: Duration = Duration::from_secs(4);
/// A message older than this when its short version is made is history, not news.
const STALE_MS: u64 = 15 * 60 * 1000;
/// How many addresses one pairing keeps: a pairing is one phone, and a phone's address changes now and then.
const ADDRESSES: usize = 3;
/// How many lines' notifications are remembered as shown. One stays until its line stops being news, which
/// every line does; this bounds what one that left the log meanwhile could hold.
const SHOWN_MOST: usize = 1000;

/// A line that stopped being news: the notifications it woke phones with are taken down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Ending {
    pub(super) id: String,
    pub(super) why: Withdrawn,
    /// The radio that has it done with itself (he answered from it, or has it on screen there): not sent one.
    pub(super) by: Option<String>,
}

/// Where each paired radio can be woken, by the radio's id: its pairing's relay channel; and the lines each
/// was woken for that are still news.
#[derive(Default)]
pub(super) struct Wake {
    targets: Mutex<BTreeMap<String, Vec<PushTarget>>>,
    /// The radios woken for each line still news, by the line's id. Kept across a restart, so a line that stops
    /// being news after one still has its notification taken down.
    shown: Mutex<BTreeMap<String, Woken>>,
}

/// The radios woken for one line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Woken {
    pub(super) radios: Vec<String>,
    /// When it was shown, in milliseconds since 1970: the oldest go first past [`SHOWN_MOST`].
    pub(super) at: u64,
    /// Its notification is being sent.
    #[serde(skip)]
    sending: bool,
    /// It stopped being news while that was being sent: taken down once it is.
    #[serde(skip)]
    ended: Option<Ending>,
}

impl Wake {
    /// The addresses kept, each under its radio's channel.
    pub(super) fn load(store: &Store) -> Result<Wake, String> {
        Ok(Wake { targets: Mutex::new(store.push_targets()?), shown: Mutex::new(store.notified()?) })
    }
    /// Is there any phone to wake at all?
    pub(super) fn any(&self) -> bool {
        !self.targets.lock().unwrap().is_empty()
    }
    /// Can a phone be woken now: one paired through the relay has said where it can be. Then a request for
    /// leave has someone to be put to even with no radio there, because that phone will be told of it.
    pub(super) fn can_wake(&self, cfg: &Config) -> bool {
        cfg.relay.is_some() && self.targets.lock().unwrap().keys().any(|radio| cfg.pairings.iter().any(|p| channel_of(p).as_deref() == Some(radio.as_str())))
    }
}

/// A radio says where it can be woken, or (with no token) that it no longer can.
pub(super) fn register(app: &App, radio: &str, target: PushTarget) {
    if !(target.token.is_empty() || airband_core::relay::push_address(&target.token)) {
        return;
    }
    let mut targets = app.push.targets.lock().unwrap();
    let mine = targets.entry(radio.to_string()).or_default();
    let before = mine.clone();
    if target.token.is_empty() {
        mine.retain(|t| t.provider != target.provider);
    } else if !mine.contains(&target) {
        mine.push(target);
        let extra = mine.len().saturating_sub(ADDRESSES);
        mine.drain(..extra);
    }
    if *mine != before {
        log(format!("push: {} {}", radio_tag(radio), if mine.is_empty() { "can no longer be woken" } else { "can be woken with a notification" }));
        if mine.is_empty() {
            targets.remove(radio);
        }
        app.store.save_push_targets(&targets);
    }
}

/// Does this line deserve a notification at all? Only while it is news: still to be heard. One that a newer
/// line, his answer or anything else took out of the reading queue before its short version was made is not.
pub(super) fn deserves(e: &Entry, now_ms: u64) -> bool {
    !e.his && e.agent.is_some() && e.short.is_some() && e.aloud.unheard() && deserves_notice(e.kind) && now_ms.saturating_sub(e.at) < STALE_MS
}

/// Why a radio is not woken for a line that deserves it, as the log says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NotWoken {
    /// It asked for what follows the line while showing the chat.
    HasIt,
    /// It is on a call, which reads the report out.
    OnCall,
}

/// Whether to wake this radio for the line (as of change count `rev`), and why not. It has the line in front of
/// him when it asked for what follows it while showing the chat; a report is also had on a call, which reads it
/// out. Something that needs him is not taken as had because a call is on.
pub(super) fn wake(call: Option<&Call>, rev: u64, needs_him: bool) -> Result<(), NotWoken> {
    match call {
        Some(c) if c.has >= rev => Err(NotWoken::HasIt),
        Some(c) if !needs_him && c.connected() => Err(NotWoken::OnCall),
        _ => Ok(()),
    }
}

/// Is this radio saying it shows the chat without yet having asked for what follows the line? Then the tower
/// waits for it to ask (see [`GRACE`]).
fn may_ask(call: Option<&Call>, rev: u64) -> bool {
    call.is_some_and(|c| c.presence.showing_chat() && c.has < rev)
}

/// The notification for a line, as the phone will open it.
pub(super) fn notice(inner: &Inner, e: &Entry, station: &str) -> Notice {
    let m = inner.message(e);
    const MOST: usize = 400;
    let text = airband_core::prose::clip(&m.text, MOST);
    // A request for leave that does not fit is still a request for leave, shown by its opening: it offers no
    // Allow from the lock screen. He opens it, reads all of what would run, and answers there.
    let cut = m.permission && m.text.chars().count() > MOST;
    // an agent waits on his answer to it: a request for leave or a question through its tool, not yet over
    let held = e.permit.as_ref().and_then(|id| inner.permits.get(id)).filter(|p| p.decided.is_none());
    Notice { id: m.id, from: m.from, agent: m.agent, badge: m.badge, group: m.group, team: m.team, station: station.into(), text, kind: m.kind, options: if m.permission { Vec::new() } else { m.options }, permission: m.permission, cut, holding: held.is_some(), rev: m.rev, until: held.and_then(|p| p.until) }
}

/// Lines that stopped being news: each phone woken for one is sent what takes its notification down, all but a
/// radio that has it done with already. Runs for the tower's life.
pub(super) async fn withdraw_ended(app: Arc<App>) {
    let mut changes = app.watch_changes();
    loop {
        let ended = std::mem::take(&mut app.change().ended);
        for ending in ended {
            let now = {
                let mut shown = app.push.shown.lock().unwrap();
                let now = match shown.get_mut(&ending.id) {
                    None => None,
                    // its notification is still on its way: taken down once that is sent (`consider`)
                    Some(w) if w.sending => {
                        w.ended = Some(ending);
                        None
                    }
                    Some(_) => shown.remove(&ending.id).map(|w| (w, ending)),
                };
                if now.is_some() {
                    app.store.save_notified(&shown);
                }
                now
            };
            if let Some((w, ending)) = now {
                take_down(&app, w, ending).await;
            }
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}

/// Send what takes a line's notification down to the radios woken for it, but the one that has it done with.
async fn take_down(app: &Arc<App>, w: Woken, ending: Ending) {
    let to: Vec<(String, Vec<PushTarget>)> = {
        let targets = app.push.targets.lock().unwrap();
        w.radios.into_iter().filter(|r| ending.by.as_ref() != Some(r)).filter_map(|r| targets.get(&r).map(|t| (r, t.clone()))).collect()
    };
    let pushed = Pushed::Withdraw { id: ending.id, why: ending.why };
    for (radio, targets) in to {
        let (app, pushed) = (app.clone(), pushed.clone());
        tokio::task::spawn_blocking(move || send(&app, &radio, &targets, pushed)).await.ok();
    }
}

/// The lines that stopped being news since the `since`-th ending did so by what he did on this radio (he
/// answered there): that radio has them done with, and is sent nothing to take them down.
pub(super) fn done_on(inner: &mut Inner, since: usize, radio: &str) {
    inner.ended.iter_mut().skip(since).for_each(|e| e.by = Some(radio.to_string()));
}

/// This radio has the chat on screen up to its change count: the lines it was woken for that it has now in
/// their current form are done with there (its app takes their notifications down itself), and every other
/// phone woken for them has theirs taken down.
pub(super) fn read_on(app: &App, inner: &mut Inner, radio: &str) {
    let Some(has) = inner.calls.get(radio).map(|c| c.has) else { return };
    let mut shown = app.push.shown.lock().unwrap();
    let read: Vec<String> = shown.iter().filter(|(id, w)| w.ended.is_none() && w.radios.iter().any(|r| r == radio) && inner.log.iter().any(|e| e.id == **id && e.rev <= has)).map(|(id, _)| id.clone()).collect();
    if read.is_empty() {
        return;
    }
    for id in read {
        let Some(w) = shown.get_mut(&id) else { continue };
        w.radios.retain(|r| r != radio);
        if w.radios.is_empty() && !w.sending {
            shown.remove(&id);
        } else {
            inner.ended.push(Ending { id, why: Withdrawn::Read, by: Some(radio.to_string()) });
        }
    }
    app.store.save_notified(&shown);
}

/// A line's short version has just been made, or a card was put to him: wake whichever phones should know.
/// Decided once: at once when no radio says it is showing the chat, else once each that does has asked for what
/// follows the line or [`GRACE`] has gone by.
pub(super) fn consider(app: &Arc<App>, inner: &mut Inner, id: &str) {
    if !app.push.any() {
        return;
    }
    let Some(e) = inner.log.iter().find(|e| e.id == id).filter(|e| deserves(e, now_ms())) else { return };
    let rev = e.rev;
    if !inner.pushed.insert(id.to_string()) {
        return;
    }
    // only what is recent can be new to this set
    if inner.pushed.len() > 4000 {
        inner.pushed.clear();
    }
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        let until = tokio::time::Instant::now() + GRACE;
        let mut changes = app.watch_changes();
        loop {
            let waiting = {
                let inner = app.read();
                let targets = app.push.targets.lock().unwrap();
                targets.keys().any(|radio| may_ask(inner.calls.get(radio), rev))
            };
            if !waiting || tokio::time::timeout_at(until, changes.changed()).await.is_err() {
                break;
            }
        }
        let station = config::station(&app.cfg());
        let (note, to): (Notice, Vec<(String, Vec<PushTarget>)>) = {
            let inner = app.read();
            let Some(e) = inner.log.iter().find(|e| e.id == id).filter(|e| deserves(e, now_ms())) else {
                return log(format!("push: line {id} stopped being news before it was sent; no notification"));
            };
            let needs_him = e.kind == Some(Kind::NeedsYou);
            let targets = app.push.targets.lock().unwrap();
            let mut to = Vec::new();
            for (radio, t) in targets.iter() {
                match wake(inner.calls.get(radio), rev, needs_him) {
                    Ok(()) => to.push((radio.clone(), t.clone())),
                    Err(NotWoken::HasIt) => log(format!("push: {} has line {id} on screen; not woken", radio_tag(radio))),
                    Err(NotWoken::OnCall) => log(format!("push: {} is on a call, which reads line {id} out; not woken", radio_tag(radio))),
                }
            }
            let note = notice(&inner, e, &station);
            // written down before it is sent, under the lock its line stops being news under: an ending that
            // comes while it is on its way finds it, and waits for it to be sent
            if !to.is_empty() {
                shown(&app, &id, to.iter().map(|(r, _)| r.clone()).collect(), e.at);
            }
            (note, to)
        };
        if to.is_empty() {
            return;
        }
        let pushed = Pushed::Show { notice: note };
        for (radio, targets) in to {
            let (app, pushed) = (app.clone(), pushed.clone());
            tokio::task::spawn_blocking(move || send(&app, &radio, &targets, pushed)).await.ok();
        }
        // sent: kept as shown, and taken down now if its line stopped being news meanwhile
        if let Some((w, ending)) = sent(&app, &id) {
            take_down(&app, w, ending).await;
        }
    });
}

/// A line's notification is on its way to these radios.
fn shown(app: &App, id: &str, radios: Vec<String>, at: u64) {
    let mut shown = app.push.shown.lock().unwrap();
    shown.insert(id.to_string(), Woken { radios, at, sending: true, ended: None });
    let extra = shown.len().saturating_sub(SHOWN_MOST);
    if extra > 0 {
        let mut by_age: Vec<(u64, String)> = shown.iter().map(|(id, w)| (w.at, id.clone())).collect();
        by_age.sort();
        by_age.into_iter().take(extra).for_each(|(_, id)| {
            shown.remove(&id);
        });
    }
}

/// A line's notification has been sent: it is kept as shown, or, if the line stopped being news meanwhile,
/// handed back with how, to be taken down.
fn sent(app: &App, id: &str) -> Option<(Woken, Ending)> {
    let mut shown = app.push.shown.lock().unwrap();
    let ended = match shown.get_mut(id) {
        Some(w) if w.ended.is_some() => shown.remove(id).and_then(|w| w.ended.clone().map(|e| (w, e))),
        Some(w) => {
            w.sending = false;
            None
        }
        None => None,
    };
    app.store.save_notified(&shown);
    ended
}

/// Seal one notification for one radio and ask the relay to pass it to each of that radio's addresses, under
/// its line's tag.
fn send(app: &App, radio: &str, targets: &[PushTarget], pushed: Pushed) {
    let cfg = app.cfg();
    // only a radio paired through a relay can be woken: the relay is what holds the push service's key
    let (Some(relay), Some(pairing)) = (cfg.relay.clone(), cfg.pairings.iter().find(|p| channel_of(p).as_deref() == Some(radio))) else { return };
    let Ok(keys) = Keys::derive(&pairing.secret) else { return };
    let what = match &pushed {
        Pushed::Withdraw { id, .. } => format!("line {id} (no longer news)"),
        Pushed::Show { notice: n } => format!("line {} ({})", n.id, if n.permission { "permission" } else if n.kind == Some(Kind::NeedsYou) { "question" } else { "report" }),
    };
    let Some(asks) = requests(&keys, pushed, targets) else { return log(format!("push: {what} is too long to send to {}", radio_tag(radio))) };
    let client = airband_io::http::Client::new(Duration::from_secs(20));
    for (target, ask) in targets.iter().zip(asks) {
        let res = client.send(airband_io::http::Ask::post(keys.to(&relay, Route::Push)).header("authorization", keys.as_tower()).json(&ask));
        let outcome = match res {
            Err(e) => format!("the relay could not be reached ({e})"),
            Ok(mut res) => {
                let status = res.status().as_u16();
                // what the relay says, typed
                match res.body_mut().read_json::<PushAnswer>() {
                    Ok(answer) if answer.sent => "sent".into(),
                    Ok(PushAnswer { why: Some(NotPushed::Gone), .. }) => {
                        let mut all = app.push.targets.lock().unwrap();
                        if let Some(mine) = all.get_mut(radio) {
                            mine.retain(|t| t != target);
                            if mine.is_empty() {
                                all.remove(radio);
                            }
                        }
                        app.store.save_push_targets(&all);
                        format!("{}; forgotten", NotPushed::Gone)
                    }
                    Ok(PushAnswer { why: Some(why), .. }) => format!("not sent: {why}"),
                    // no answer of the push route's shape: what the status says, and nothing more is claimed
                    Ok(_) => format!("the relay answered {status} with no word on whether it was sent"),
                    Err(_) => format!("the relay answered {:?} with no push answer", crate::tower::RelaySaid::of(status)),
                }
            }
        };
        log(format!("push: asked the relay to wake {} for {what}: {outcome}", radio_tag(radio)));
    }
}

/// What the relay is asked to pass on to each of these addresses: the notification sealed with the pairing's key,
/// kept under its line's tag. Nothing for one that will not fit in a push however much of its words are cut.
fn requests(keys: &Keys, mut pushed: Pushed, targets: &[PushTarget]) -> Option<Vec<PushRequest>> {
    let sealed = loop {
        let sealed = keys.seal(Direction::Push, &serde_json::to_vec(&pushed).unwrap());
        if sealed.len() <= PUSH_MOST {
            break sealed;
        }
        // too long for one push: he gets the opening, and the rest when he opens it
        let Pushed::Show { notice: note } = &mut pushed else { return None };
        let keep = note.text.chars().count() * 2 / 3;
        if keep < 20 {
            return None;
        }
        note.text = airband_core::prose::clip(&note.text, keep);
        // cut: never an Allow under part of a command, and still a request for leave
        note.cut |= note.permission;
    };
    let sealed = airband_core::encode::base64(&sealed);
    // every notification about a line is kept under its tag: what takes one down takes its place
    let replaces = Some(keys.notice_tag(pushed.line()));
    let (question, quiet, until) = match &pushed {
        Pushed::Show { notice } => (notice.kind == Some(Kind::NeedsYou), false, notice.until),
        Pushed::Withdraw { .. } => (false, true, None),
    };
    Some(targets.iter().map(|t| PushRequest { provider: t.provider, to: t.token.clone(), sealed: sealed.clone(), question, replaces: replaces.clone(), quiet, until }).collect())
}

/// What a radio is called where he reads about it (the log, `airband status`): the name its pairing was given,
/// found by the channel the radio is known by; a radio that came with the tower's own token is known by its name.
pub(super) fn radio_name(cfg: &Config, radio: &str) -> String {
    cfg.pairings.iter().find(|p| channel_of(p).as_deref() == Some(radio)).map_or_else(|| radio.to_string(), |p| p.name.clone())
}

/// A pairing's id as its radio is known by: the relay channel its secret gives. Nothing for one whose secret
/// gives no keys.
pub(super) fn channel_of(pairing: &config::PairedRadio) -> Option<String> {
    Keys::derive(&pairing.secret).ok().map(|k| k.channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(inner: &mut Inner, id: &str, kind: Option<Kind>, short: Option<&str>) -> Entry {
        let s = inner.sessions[0].clone();
        inner.record(Entry { short: short.map(String::from), aloud: Aloud::Waiting, ..Entry::said(id.into(), &s, kind, "the whole message".into()) });
        inner.log.iter().find(|e| e.id == id).unwrap().clone()
    }
    fn tower() -> Inner {
        let mut inner = Inner::default();
        let _ = inner.check_in(&airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") });
        inner
    }

    #[test]
    fn the_addresses_kept_are_read_back_under_each_radios_channel() {
        let path = std::env::temp_dir().join(format!("airband-push-channel-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).unwrap();
        let phone = config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() };
        let channel = channel_of(&phone).unwrap();
        let target = PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) };
        store.save_push_targets(&BTreeMap::from([(channel.clone(), vec![target.clone()])]));
        let wake = Wake::load(&store).unwrap();
        assert_eq!(wake.targets.lock().unwrap().get(&channel), Some(&vec![target]));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_question_or_a_report_deserves_one_and_nothing_else_does() {
        let mut inner = tower();
        let t = now_ms();
        assert!(deserves(&line(&mut inner, "q", Some(Kind::NeedsYou), Some("Ship it?")), t));
        assert!(deserves(&line(&mut inner, "r", Some(Kind::Report), Some("Shipped.")), t));
        assert!(!deserves(&line(&mut inner, "f", Some(Kind::Fyi), Some("Starting.")), t));
        assert!(!deserves(&line(&mut inner, "s", Some(Kind::Stalled), Some("Next I will ship.")), t));
        // not before it has been read and cut down, and not long after it was said
        assert!(!deserves(&line(&mut inner, "u", None, None), t));
        // measured from when the line was said, not from when this test read the clock
        let old = line(&mut inner, "o", Some(Kind::NeedsYou), Some("Ship it?"));
        assert!(deserves(&old, old.at + STALE_MS - 1) && !deserves(&old, old.at + STALE_MS));
        // and never his own words
        let to = vec![inner.sessions[0].clone()];
        inner.record(Entry { kind: Some(Kind::NeedsYou), ..Entry::his("h".into(), &to, "ship it".into()) });
        assert!(!deserves(inner.log.last().unwrap(), t));
    }

    #[test]
    fn a_radio_that_has_it_is_left_alone_and_a_call_only_stands_in_for_a_report() {
        let mut inner = tower();
        let rev = line(&mut inner, "q", Some(Kind::NeedsYou), Some("Ship it?")).rev;
        // never heard from, or heard from before the line existed: wake it
        assert_eq!(wake(None, rev, true), Ok(()));
        inner.call("phone").has = rev - 1;
        assert_eq!(wake(inner.calls.get("phone"), rev, true), Ok(()));
        // it has asked for what follows the line: it is on his screen
        inner.call("phone").has = rev;
        assert_eq!(wake(inner.calls.get("phone"), rev, true), Err(NotWoken::HasIt));
        assert_eq!(wake(inner.calls.get("phone"), rev, false), Err(NotWoken::HasIt));
        // on a call a report is read out; a question or a request for leave still wakes it
        inner.call("car").presence.says(airband_core::radio::Presence { on_call: true, showing_chat: false });
        assert_eq!(wake(inner.calls.get("car"), rev, false), Err(NotWoken::OnCall));
        assert_eq!(wake(inner.calls.get("car"), rev, true), Ok(()));
    }

    #[test]
    fn only_a_radio_saying_it_shows_the_chat_and_not_yet_past_the_line_is_waited_for() {
        let mut inner = tower();
        let rev = line(&mut inner, "q", Some(Kind::NeedsYou), Some("Ship it?")).rev;
        assert!(!may_ask(None, rev));
        inner.call("pocket").presence.says(airband_core::radio::Presence { on_call: false, showing_chat: false });
        assert!(!may_ask(inner.calls.get("pocket"), rev), "one that says the chat is not in front of him is woken at once");
        inner.call("phone").presence.says(airband_core::radio::Presence { on_call: false, showing_chat: true });
        assert!(may_ask(inner.calls.get("phone"), rev));
        inner.call("phone").has = rev;
        assert!(!may_ask(inner.calls.get("phone"), rev), "it asked: nothing more to wait for");
    }

    #[test]
    fn a_long_request_for_leave_is_still_one_with_no_allow_from_the_lock_screen() {
        let mut inner = tower();
        let s = inner.sessions[0].clone();
        let what = format!("cd /x && {}", "echo step; ".repeat(40));
        let request = permit::Request::waiting(&what);
        inner.permits.insert("p".into(), permit::Permit { agent_id: s.agent.id.clone(), entry: "c".into(), decided: None, asked: Instant::now(), polled: Instant::now(), question: false, until: None });
        inner.record(Entry { short: request.line(), options: vec![permit::ALLOW.into(), permit::DENY.into()], permit: Some("p".into()), leave: Some(request), ..Entry::said("c".into(), &s, Some(Kind::NeedsYou), what) });
        let n = notice(&inner, inner.log.last().unwrap(), "mini");
        assert!(n.permission && n.cut, "{n:?}");
        assert!(n.options.is_empty());
        // one that fits is whole
        inner.log.last_mut().unwrap().short = Some("Wants to run `ls`. Allow?".into());
        let n = notice(&inner, inner.log.last().unwrap(), "mini");
        assert!(n.permission && !n.cut);
        // it holds an agent while it waits, until the time its tool gives it, and not once he answered
        assert!(n.holding && n.until.is_none(), "{n:?}");
        inner.permits.get_mut("p").unwrap().until = Some(4_000_000_000);
        assert_eq!(notice(&inner, inner.log.last().unwrap(), "mini").until, Some(4_000_000_000));
        inner.permits.get_mut("p").unwrap().decided = Some((true, String::new()));
        let n = notice(&inner, inner.log.last().unwrap(), "mini");
        assert!(!n.holding && n.until.is_none(), "{n:?}");
    }

    #[test]
    fn a_report_or_a_plain_question_holds_nobody_up() {
        let mut inner = tower();
        let report = line(&mut inner, "r", Some(Kind::Report), Some("Done."));
        let question = line(&mut inner, "q", Some(Kind::NeedsYou), Some("Which one?"));
        assert!(!notice(&inner, &report, "mini").holding);
        assert!(!notice(&inner, &question, "mini").holding);
    }

    #[test]
    fn the_notification_carries_what_the_phone_needs_to_show_and_answer_it() {
        let mut inner = tower();
        let s = inner.sessions[0].clone();
        inner.record(Entry { short: Some("Rewrite or leave?".into()), options: vec!["Rewrite".into(), "Leave".into()], ..Entry::said("q".into(), &s, Some(Kind::NeedsYou), "long".into()) });
        let n = notice(&inner, &inner.log[0], "mini");
        assert_eq!((n.id.as_str(), n.from.as_str(), n.group.as_str(), n.station.as_str(), n.text.as_str()), ("q", "shop 1", "shop", "mini", "Rewrite or leave?"));
        assert_eq!(n.badge, "S1");
        assert_eq!((n.options, n.permission, n.kind), (vec!["Rewrite".to_string(), "Leave".to_string()], false, Some(Kind::NeedsYou)));
        // sealed, it fits in one push even when the line is as long as a line gets
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        inner.log[0].short = Some("é".repeat(2000));
        let long = notice(&inner, &inner.log[0], "mini");
        assert!(keys.seal(Direction::Push, &serde_json::to_vec(&long).unwrap()).len() <= PUSH_MOST);
    }

    /// A line with its place in the reading queue, as an agent's line is recorded.
    fn news(inner: &mut Inner, id: &str, kind: Kind, short: &str) {
        let s = inner.sessions[0].clone();
        inner.record(Entry { short: Some(short.into()), aloud: Aloud::Waiting, ..Entry::said(id.into(), &s, Some(kind), "the whole message".into()) });
    }
    fn endings(inner: &mut Inner) -> Vec<(String, Withdrawn)> {
        std::mem::take(&mut inner.ended).into_iter().map(|e| (e.id, e.why)).collect()
    }

    /// Every way a line stops being news ends it once, typed with why, whatever kind of line it is: what a
    /// notification it woke a phone with is taken down by. Nothing else ends one.
    #[test]
    fn every_way_a_line_stops_being_news_ends_its_notification_once() {
        use airband_core::radio::NotRead;
        let mut inner = tower();
        let agent = inner.sessions[0].agent.id.clone();
        // a newer line from its agent: the older one is superseded
        news(&mut inner, "r1", Kind::Report, "Shipped.");
        inner.supersede(&agent, false);
        assert_eq!(endings(&mut inner), [("r1".to_string(), Withdrawn::NotRead { reason: NotRead::Superseded })]);
        // he answered its agent
        news(&mut inner, "q1", Kind::NeedsYou, "Ship it?");
        inner.no_longer_waiting(&agent);
        assert_eq!(endings(&mut inner), [("q1".to_string(), Withdrawn::NotRead { reason: NotRead::Answered })]);
        // heard aloud
        news(&mut inner, "r2", Kind::Report, "Merged.");
        inner.set_aloud("r2", Aloud::Handed { radio: "car".into(), at: 1 });
        assert!(endings(&mut inner).is_empty(), "handed to a radio is still on its way to him");
        inner.set_aloud("r2", Aloud::Heard { radio: "car".into(), at: 2 });
        assert_eq!(endings(&mut inner), [("r2".to_string(), Withdrawn::Heard)]);
        // a request for leave that is over, the same way as any other line
        news(&mut inner, "c1", Kind::NeedsYou, "Wants to run `ls`. Allow?");
        inner.close_line("c1", permit::RequestEnded::RanOut);
        assert_eq!(endings(&mut inner), [("c1".to_string(), Withdrawn::NotRead { reason: NotRead::PermissionEnded })]);
        // once over, it is over: nothing ends it a second time
        inner.set_aloud("r1", Aloud::dropped(NotRead::Stale));
        inner.set_aloud("r2", Aloud::dropped(NotRead::Superseded));
        assert!(endings(&mut inner).is_empty());
        // and a line that is no longer news is never notified: one superseded before its short version was
        // made is not sent at all
        news(&mut inner, "r3", Kind::Report, "Deployed.");
        let r3 = inner.log.iter().find(|e| e.id == "r3").unwrap().clone();
        assert!(deserves(&r3, now_ms()));
        inner.supersede(&agent, false);
        assert!(!deserves(inner.log.iter().find(|e| e.id == "r3").unwrap(), now_ms()));
    }

    /// What the relay is handed: sealed with the pairing's key, under the line's tag and never its id, for what
    /// shows a line and for what takes it down alike; the second quiet, and opening to nothing but the line's id
    /// and why.
    #[test]
    fn a_notification_and_what_takes_it_down_go_under_one_tag_that_says_nothing_of_the_line() {
        let mut inner = tower();
        news(&mut inner, "line1", Kind::NeedsYou, "Rewrite or leave?");
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let target = PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) };
        let shown = Pushed::Show { notice: notice(&inner, &inner.log[0], "mini") };
        let why = Withdrawn::NotRead { reason: airband_core::radio::NotRead::Superseded };
        let [show] = requests(&keys, shown.clone(), std::slice::from_ref(&target)).unwrap().try_into().unwrap();
        let [down] = requests(&keys, Pushed::Withdraw { id: "line1".into(), why }, std::slice::from_ref(&target)).unwrap().try_into().unwrap();
        let tag = keys.notice_tag("line1");
        assert_eq!((show.replaces.as_deref(), down.replaces.as_deref()), (Some(tag.as_str()), Some(tag.as_str())));
        assert!(show.well_formed() && down.well_formed());
        assert!((show.question, show.quiet) == (true, false) && (down.question, down.quiet) == (false, true));
        let open = |r: &PushRequest| serde_json::from_slice::<Pushed>(&keys.open(Direction::Push, &airband_core::encode::unbase64(&r.sealed).unwrap()).unwrap()).unwrap();
        assert_eq!(open(&show), shown);
        assert_eq!(open(&down), Pushed::Withdraw { id: "line1".into(), why });
        // nothing in the clear names the line or says its words
        let plain = serde_json::to_string(&PushRequest { sealed: String::new(), ..down }).unwrap() + &serde_json::to_string(&PushRequest { sealed: String::new(), ..show }).unwrap();
        assert!(!plain.contains("line1") && !plain.contains("Rewrite"), "{plain}");
    }

    /// The text a phone is woken with is the line as the chat shows it, and what is read out is that line with
    /// only who said it in front.
    #[test]
    fn the_notification_the_chat_and_the_reading_say_the_same_words() {
        let mut inner = tower();
        news(&mut inner, "r1", Kind::Report, "The checker passes on all twelve units.");
        let e = inner.log[0].clone();
        let note = notice(&inner, &e, "mini");
        assert_eq!(note.text, inner.message(&e).text, "the chat's line");
        assert_eq!(note.rev, e.rev, "with the change count the chat has it at");
        assert_eq!(super::super::pump::aloud_line(&inner, &e), format!("{}. {}", airband_core::prose::capital(&inner.sender_name(&e)), note.text));
    }

    /// A question and its answers, a request for leave, and a question read again as a reminder: the
    /// notification, the chat and the reading say the same words, the reading with only who said it in front,
    /// and the answers the notification offers are the chat's.
    #[test]
    fn a_question_its_answers_and_its_reminder_say_the_chats_words() {
        use super::super::pump::{aloud_line, reminder, transmission};
        let mut inner = tower();
        let s = inner.sessions[0].clone();
        let callsign = |inner: &Inner, e: &Entry| format!("{}. ", airband_core::prose::capital(&inner.sender_name(e)));
        inner.record(Entry { short: Some("Rewrite the two easy questions, or leave them?".into()), options: vec!["Rewrite".into(), "Leave them".into()], aloud: Aloud::Waiting, ..Entry::said("q".into(), &s, Some(Kind::NeedsYou), "long".into()) });
        let q = inner.log.last().unwrap().clone();
        let (note, chat) = (notice(&inner, &q, "mini"), inner.message(&q));
        assert_eq!((&note.text, &note.options), (&chat.text, &chat.options), "the chat's words and answers");
        let read = transmission(&inner, &q, Kind::NeedsYou, 0);
        assert_eq!(read.say, callsign(&inner, &q) + &note.text, "read with who said it in front and nothing after");
        assert_eq!(read.say, aloud_line(&inner, &q));
        // heard on a call, and not answered: the reminder is the same transmission, word for word
        crate::adapters::test_env::set("AIRBAND_REMIND_SECONDS", "0");
        inner.call("car");
        inner.offered.insert(q.id.clone(), (1, Instant::now()));
        let again = reminder(&mut inner, "car").expect("a reminder is owed");
        assert_eq!(again.id, q.id);
        assert_eq!(transmission(&inner, &again, Kind::NeedsYou, 0).say, read.say);
        // a request for leave: the notification, the chat and the reading say the same, and its answers are
        // the notification's own buttons, never words read out
        let request = permit::Request::waiting("run `ls`");
        inner.permits.insert("p".into(), permit::Permit { agent_id: s.agent.id.clone(), entry: "c".into(), decided: None, asked: Instant::now(), polled: Instant::now(), question: false, until: None });
        inner.record(Entry { short: request.line(), options: vec![permit::ALLOW.into(), permit::DENY.into()], permit: Some("p".into()), leave: Some(request), aloud: Aloud::Waiting, ..Entry::said("c".into(), &s, Some(Kind::NeedsYou), "run `ls`".into()) });
        let c = inner.log.last().unwrap().clone();
        let (note, chat) = (notice(&inner, &c, "mini"), inner.message(&c));
        assert!(note.permission && note.options.is_empty());
        assert_eq!(note.text, chat.text);
        assert_eq!(transmission(&inner, &c, Kind::NeedsYou, 0).say, callsign(&inner, &c) + &note.text);
        crate::adapters::test_env::clear();
    }

    /// A line that stops being news has its notification taken down on each phone woken for it, but the one
    /// that has it done with; one that stops while its notification is still on the way is taken down once that
    /// is sent; and a line on screen on one device is taken down on the others.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_notification_is_taken_down_on_every_phone_but_the_one_done_with_it() {
        let (phone, pad) = (config::PairedRadio { name: "phone".into(), secret: airband_core::relay::new_secret() }, config::PairedRadio { name: "pad".into(), secret: airband_core::relay::new_secret() });
        let (p, d) = (channel_of(&phone).unwrap(), channel_of(&pad).unwrap());
        let app = super::super::http::tests::app_with("push-down", serde_json::json!({ "relay": "http://127.0.0.1:9", "pairings": [phone, pad] }));
        for radio in [&p, &d] {
            register(&app, radio, PushTarget { provider: airband_core::relay::PushService::Apns, token: "ab".repeat(32) });
        }
        // which of the two each line's notification was taken down on, as the log says it
        let (pt, dt) = (radio_tag(&p), radio_tag(&d));
        let taken_down = |id: &str| {
            let lines: Vec<String> = super::super::logged::take().into_iter().filter(|l| l.contains(&format!("for line {id} (no longer news)"))).collect();
            [&pt, &dt].into_iter().filter(|t| lines.iter().any(|l| l.contains(&format!("wake {t} ")))).cloned().collect::<Vec<_>>()
        };
        tokio::spawn(withdraw_ended(app.clone()));
        super::super::logged::take();
        // shown on both, then superseded: taken down on both
        shown(&app, "a", vec![p.clone(), d.clone()], 1);
        assert!(sent(&app, "a").is_none());
        app.change().ended.push(Ending { id: "a".into(), why: Withdrawn::NotRead { reason: airband_core::radio::NotRead::Superseded }, by: None });
        let wait = || async { for _ in 0..100 { if app.push.shown.lock().unwrap().is_empty() && app.read().ended.is_empty() { break } tokio::time::sleep(Duration::from_millis(20)).await } tokio::time::sleep(Duration::from_millis(100)).await };
        wait().await;
        assert_eq!(taken_down("a"), [pt.clone(), dt.clone()], "taken down on each phone woken for it");
        // answered from the phone: taken down on the pad alone
        shown(&app, "b", vec![p.clone(), d.clone()], 2);
        sent(&app, "b");
        app.change().ended.push(Ending { id: "b".into(), why: Withdrawn::NotRead { reason: airband_core::radio::NotRead::Answered }, by: Some(p.clone()) });
        wait().await;
        assert_eq!(taken_down("b"), std::slice::from_ref(&dt));
        // it stops being news while its notification is on its way: taken down once that is sent, not before
        shown(&app, "c", vec![p.clone()], 3);
        app.change().ended.push(Ending { id: "c".into(), why: Withdrawn::Heard, by: None });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(taken_down("c").is_empty(), "nothing to take down before it is there");
        let (w, ending) = sent(&app, "c").expect("handed back to be taken down once sent");
        assert_eq!((w.radios, ending.why), (vec![p.clone()], Withdrawn::Heard));
        // on screen on the phone: the pad's is taken down, and the phone's own app has its own
        let mut inner = tower();
        news(&mut inner, "d", Kind::Report, "Done.");
        let rev = inner.log[0].rev;
        shown(&app, "d", vec![p.clone(), d.clone()], 4);
        sent(&app, "d");
        inner.call(&p).has = rev;
        read_on(&app, &mut inner, &p);
        assert_eq!(inner.ended, [Ending { id: "d".into(), why: Withdrawn::Read, by: Some(p.clone()) }]);
        assert_eq!(app.push.shown.lock().unwrap().get("d").map(|w| w.radios.clone()), Some(vec![d.clone()]));
        // and on the one phone woken, on screen there: nothing to send anyone
        news(&mut inner, "e", Kind::Report, "Done again.");
        inner.ended.clear();
        shown(&app, "e", vec![p.clone()], 5);
        sent(&app, "e");
        inner.call(&p).has = inner.rev;
        read_on(&app, &mut inner, &p);
        assert!(inner.ended.iter().all(|e| e.id != "e") && !app.push.shown.lock().unwrap().contains_key("e"));
        // what is shown is kept across a restart
        assert!(app.store.notified().unwrap().contains_key("d"));
    }
}
