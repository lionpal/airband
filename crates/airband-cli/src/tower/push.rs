//! Waking his phone. When a message deserves it and his phone does not already have it in front of him, the
//! tower seals a notification with that phone's pairing key and asks the relay to hand it to the phone's push
//! service. The relay and the push service pass on bytes they cannot read.
//!
//! What deserves one: a message judged to need him or to be a report, a permission card, a question a harness
//! puts to him. Never chatter, a stall, or his own words. One per message. Not to a radio that has the chat
//! open and has it: such a radio asks for what follows a message as soon as it gets it, and that request is
//! how the tower knows. A report is not sent to a radio on a call either (it is read out there); something
//! that needs him is, because a call can be held, or quiet, or in his pocket with the screen off.
//!
//! Nothing here knows which push service a phone uses: an address is a provider's name and a token, as the
//! phone gave them. The log says when a notification was asked for and what came back, and never an address or
//! a message's words.
use super::*;
use airband_core::radio::{Notice, PushTarget, deserves_notice};
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

/// Where each paired radio can be woken, by the radio's id: its pairing's relay channel.
#[derive(Default)]
pub(super) struct Wake {
    targets: Mutex<BTreeMap<String, Vec<PushTarget>>>,
    /// The radios woken for a line that holds an agent, by the line's id, until its request ends and they are
    /// told so. Every request ends (answered, withdrawn, replaced, or reaped once nobody waits on it), so this
    /// holds only requests still open.
    woke: Mutex<BTreeMap<String, Woken>>,
}

/// The radios woken for one request.
struct Woken {
    radios: Vec<String>,
    /// Its first notification is being sent.
    sending: bool,
    /// It ended, while that was being sent: with the radio he answered from, if he did.
    ended: Option<Option<String>>,
}

impl Wake {
    /// The addresses kept, each under its radio's channel.
    pub(super) fn load(store: &Store) -> Result<Wake, String> {
        Ok(Wake { targets: Mutex::new(store.push_targets()?), woke: Mutex::default() })
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

/// Does this line deserve a notification at all?
pub(super) fn deserves(e: &Entry, now_ms: u64) -> bool {
    !e.his && e.agent.is_some() && e.short.is_some() && deserves_notice(e.kind) && now_ms.saturating_sub(e.at) < STALE_MS
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
    Notice { id: m.id, from: m.from, agent: m.agent, badge: m.badge, group: m.group, team: m.team, station: station.into(), text, kind: m.kind, options: if m.permission { Vec::new() } else { m.options.iter().take(4).map(|o| airband_core::prose::clip(o, 40)).collect() }, permission: m.permission, cut, holding: held.is_some(), over: false, until: held.and_then(|p| p.until) }
}

/// Requests that ended: each phone woken for one gets a notification in its place, quiet and with nothing to
/// answer, that says how it ended. Not the radio he answered it from. Runs for the tower's life.
pub(super) async fn close_ended(app: Arc<App>) {
    let mut changes = app.watch_changes();
    loop {
        let ended = std::mem::take(&mut app.change().ended);
        for (id, by) in ended {
            let over = {
                let mut woke = app.push.woke.lock().unwrap();
                match woke.get_mut(&id) {
                    None => None,
                    // its first notification is still on its way: it is told once that is sent (`consider`)
                    Some(w) if w.sending => {
                        w.ended = Some(by);
                        None
                    }
                    Some(_) => woke.remove(&id).map(|w| Woken { ended: Some(by), ..w }),
                }
            };
            if let Some(w) = over {
                tell_over(&app, &id, w).await;
            }
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}

/// Send the notification that says a line's request is over to the radios woken for it.
async fn tell_over(app: &Arc<App>, id: &str, w: Woken) {
    let station = config::station(&app.cfg());
    let note = {
        let inner = app.read();
        let Some(e) = inner.log.iter().find(|e| e.id == id) else { return };
        Notice { over: true, holding: false, options: Vec::new(), permission: false, cut: false, ..notice(&inner, e, &station) }
    };
    let answered_from = w.ended.flatten();
    let to: Vec<(String, Vec<PushTarget>)> = {
        let targets = app.push.targets.lock().unwrap();
        w.radios.into_iter().filter(|r| answered_from.as_ref() != Some(r)).filter_map(|r| targets.get(&r).map(|t| (r, t.clone()))).collect()
    };
    for (radio, targets) in to {
        let (app, note) = (app.clone(), note.clone());
        tokio::task::spawn_blocking(move || send(&app, &radio, &targets, note)).await.ok();
    }
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
            let Some(e) = inner.log.iter().find(|e| e.id == id).filter(|e| deserves(e, now_ms())) else { return };
            let needs_him = e.kind == Some(Kind::NeedsYou);
            // answered meanwhile, from another radio or at the keyboard: there is nothing left to tell him. A
            // card stands while its harness waits on it, whatever the agent has said since.
            let over = match &e.permit {
                Some(permit) => inner.permits.get(permit).is_none_or(|p| p.decided.is_some()),
                None => needs_him && !inner.open_ids().contains(&id),
            };
            if over {
                return log(format!("push: line {id} was answered before it was sent; no notification"));
            }
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
            // written down before it is sent, under the lock its request ends under: an end that comes while it
            // is on its way finds it, and waits for it to be sent
            if note.holding && !to.is_empty() {
                app.push.woke.lock().unwrap().insert(id.clone(), Woken { radios: to.iter().map(|(r, _)| r.clone()).collect(), sending: true, ended: None });
            }
            (note, to)
        };
        let holding = note.holding;
        for (radio, targets) in to {
            let (app, note) = (app.clone(), note.clone());
            tokio::task::spawn_blocking(move || send(&app, &radio, &targets, note)).await.ok();
        }
        if !holding {
            return;
        }
        // sent: a request that ended meanwhile is told as over now, after the notification it takes the place of
        let ended = {
            let mut woke = app.push.woke.lock().unwrap();
            match woke.get_mut(&id) {
                Some(w) if w.ended.is_some() => woke.remove(&id),
                Some(w) => {
                    w.sending = false;
                    None
                }
                None => None,
            }
        };
        if let Some(w) = ended {
            tell_over(&app, &id, w).await;
        }
    });
}

/// Seal one notification for one radio and ask the relay to pass it to each of that radio's addresses.
fn send(app: &App, radio: &str, targets: &[PushTarget], mut note: Notice) {
    let cfg = app.cfg();
    // only a radio paired through a relay can be woken: the relay is what holds the push service's key
    let (Some(relay), Some(pairing)) = (cfg.relay.clone(), cfg.pairings.iter().find(|p| channel_of(p).as_deref() == Some(radio))) else { return };
    let Ok(keys) = Keys::derive(&pairing.secret) else { return };
    let what = format!("line {} ({})", note.id, if note.over { "its request is over" } else if note.permission { "permission" } else if note.kind == Some(Kind::NeedsYou) { "question" } else { "report" });
    let sealed = loop {
        let sealed = keys.seal(Direction::Push, &serde_json::to_vec(&note).unwrap());
        if sealed.len() <= PUSH_MOST {
            break sealed;
        }
        // too long for one push: he gets the opening, and the rest when he opens it
        let keep = note.text.chars().count() * 2 / 3;
        if keep < 20 {
            return log(format!("push: {what} is too long to send to {}", radio_tag(radio)));
        }
        note.text = airband_core::prose::clip(&note.text, keep);
        // cut: never an Allow under part of a command, and still a request for leave
        note.cut |= note.permission;
    };
    let sealed = airband_core::encode::base64(&sealed);
    let client = airband_io::http::Client::new(Duration::from_secs(20));
    for target in targets {
        // one that holds an agent, and the one that says it is over, are kept under the line's id: the second
        // takes the place of the first
        let replaces = (note.holding || note.over).then(|| note.id.clone());
        let ask = PushRequest { provider: target.provider, to: target.token.clone(), sealed: sealed.clone(), question: note.kind == Some(Kind::NeedsYou) && !note.over, replaces, quiet: note.over, until: note.until };
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
        inner.record(Entry { short: short.map(String::from), ..Entry::said(id.into(), &s, kind, "the whole message".into()) });
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
        assert!(n.holding && !n.over && n.until.is_none(), "{n:?}");
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
}
