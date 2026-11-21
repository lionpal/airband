//! Waking his phone. When a message deserves it and his phone does not already have it in front of him, the
//! tower seals a notification with that phone's pairing key and asks the relay to hand it to the phone's push
//! service. The relay and the push service pass on bytes they cannot read.
//!
//! What deserves one: a message judged to need him or to be a report, a permission card, a question a harness
//! puts to him. Never chatter, a stall, or his own words. One per message. Not to a radio that is on a call
//! (it is read out there), and not to one that has the chat open: such a radio asks for what follows a message
//! within a moment of getting it, which is how the tower knows.
//!
//! Nothing here knows which push service a phone uses: an address is a provider's name and a token, as the
//! phone gave them. The log says when a notification was asked for and what came back, and never an address or
//! a message's words.
use super::*;
use airband_core::radio::{Notice, PushTarget, deserves_notice};
use airband_core::relay::{Direction, Keys, PUSH_MOST, PushAnswer, PushRequest};

/// How long after a message is ready the tower waits before deciding: long enough for a radio that is showing
/// the chat to have asked for what follows.
const GRACE: Duration = Duration::from_secs(4);
/// A message older than this when its short version is made is history, not news.
const STALE_MS: u64 = 15 * 60 * 1000;
/// How many addresses one pairing keeps: a pairing is one phone, and a phone's address changes now and then.
const ADDRESSES: usize = 3;

/// Where each paired radio can be woken, by the radio's name.
#[derive(Default)]
pub(super) struct Wake {
    targets: Mutex<BTreeMap<String, Vec<PushTarget>>>,
}

impl Wake {
    pub(super) fn load(store: &Store) -> Wake {
        Wake { targets: Mutex::new(store.push_targets()) }
    }
    /// Is there any phone to wake at all?
    pub(super) fn any(&self) -> bool {
        !self.targets.lock().unwrap().is_empty()
    }
}

/// A radio says where it can be woken, or (with no token) that it no longer can.
pub(super) fn register(app: &App, radio: &str, target: PushTarget) {
    let ok = |t: &str, most: usize| !t.is_empty() && t.len() <= most && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b));
    if !ok(&target.provider, 24) || !(target.token.is_empty() || ok(&target.token, 256)) {
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
        log(format!("push: \"{radio}\" {}", if mine.is_empty() { "can no longer be woken" } else { "can be woken with a notification" }));
        if mine.is_empty() {
            targets.remove(radio);
        }
        app.store.save_push_targets(&targets);
    }
}

/// Does this line deserve a notification at all?
pub(super) fn deserves(e: &Entry, now_ms: u64) -> bool {
    e.from != "you" && e.agent.is_some() && e.short.is_some() && deserves_notice(e.kind) && now_ms.saturating_sub(e.at) < STALE_MS
}

/// Does this radio already have the line (as of change count `rev`) in front of him?
pub(super) fn there(call: Option<&Call>, rev: u64) -> bool {
    call.is_some_and(|c| c.connected() || c.has >= rev)
}

/// The notification for a line, as the phone will open it.
pub(super) fn notice(inner: &Inner, e: &Entry, station: &str) -> Notice {
    let m = inner.message(e);
    let short = |t: &str, most: usize| if t.chars().count() > most { format!("{}…", t.chars().take(most - 1).collect::<String>().trim_end()) } else { t.to_string() };
    let text = short(&m.text, 400);
    // A request for leave that does not fit is shown by its opening and offers no answer from the lock screen:
    // he opens it, reads all of what would run, and answers there.
    let cut = m.permission && text != m.text;
    Notice { id: m.id, colour: inner.colours.get(&m.group).copied().unwrap_or(0), from: m.from, group: m.group, station: station.into(), text, kind: m.kind, options: if cut { Vec::new() } else { m.options.iter().take(4).map(|o| short(o, 40)).collect() }, permission: m.permission && !cut }
}

/// A line's short version has just been made, or a card was put to him: wake whichever phones should know.
/// Decided after a moment, once: by then a radio that is showing the chat has asked for what follows the line.
pub(super) fn consider(app: &Arc<App>, inner: &mut Inner, id: &str) {
    if !app.push.any() {
        return;
    }
    let Some(e) = inner.log.iter().find(|e| e.id == id).filter(|e| deserves(e, now())) else { return };
    let (rev, card) = (e.rev, e.permit.is_some());
    if !inner.pushed.insert(id.to_string()) {
        return;
    }
    // only what is recent can be new to this set
    if inner.pushed.len() > 4000 {
        inner.pushed.clear();
    }
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        tokio::time::sleep(GRACE).await;
        let station = config::station(&app.cfg);
        let (note, to): (Notice, Vec<(String, Vec<PushTarget>)>) = {
            let inner = app.inner.lock().unwrap();
            let Some(e) = inner.log.iter().find(|e| e.id == id).filter(|e| deserves(e, now())) else { return };
            // answered meanwhile, from another radio or at the keyboard: there is nothing left to tell him
            if (card && e.permit.is_none()) || (e.kind == Some(Kind::NeedsYou) && !inner.open_ids().contains(&id)) {
                return;
            }
            let targets = app.push.targets.lock().unwrap();
            (notice(&inner, e, &station), targets.iter().filter(|(radio, _)| !there(inner.calls.get(*radio), rev)).map(|(radio, t)| (radio.clone(), t.clone())).collect())
        };
        for (radio, targets) in to {
            let (app, note) = (app.clone(), note.clone());
            let _ = tokio::task::spawn_blocking(move || send(&app, &radio, &targets, note)).await;
        }
    });
}

/// Seal one notification for one radio and ask the relay to pass it to each of that radio's addresses.
fn send(app: &App, radio: &str, targets: &[PushTarget], mut note: Notice) {
    let cfg = config::read().unwrap_or_else(|| app.cfg.clone());
    // only a radio paired through a relay can be woken: the relay is what holds the push service's key
    let (Some(relay), Some(pairing)) = (cfg.relay.clone(), cfg.pairings.iter().find(|p| p.name == radio)) else { return };
    let Ok(keys) = Keys::derive(&pairing.secret) else { return };
    let what = format!("{} ({})", note.from, if note.permission { "permission" } else if note.kind == Some(Kind::NeedsYou) { "question" } else { "report" });
    let sealed = loop {
        let sealed = keys.seal(Direction::Push, &serde_json::to_vec(&note).unwrap());
        if sealed.len() <= PUSH_MOST {
            break sealed;
        }
        // too long for one push: he gets the opening, and the rest when he opens it
        let keep = note.text.chars().count() * 2 / 3;
        if keep < 20 {
            return log(format!("push: {what} is too long to send to \"{radio}\""));
        }
        note.text = format!("{}…", note.text.chars().take(keep).collect::<String>().trim_end());
        // cut: never an Allow under part of a command
        if note.permission {
            (note.permission, note.options) = (false, Vec::new());
        }
    };
    use base64::Engine;
    let sealed = base64::engine::general_purpose::STANDARD.encode(sealed);
    let agent = airband_radio::net::agent(Duration::from_secs(20));
    for target in targets {
        let ask = PushRequest { provider: target.provider.clone(), to: target.token.clone(), sealed: sealed.clone(), question: note.kind == Some(Kind::NeedsYou) };
        let res = agent.post(format!("{}/v1/channels/{}/push", relay.trim_end_matches('/'), keys.channel)).header("authorization", format!("Bearer {}", keys.pull_token)).send_json(&ask);
        let outcome = match res {
            Err(e) => format!("the relay could not be reached ({e})"),
            Ok(mut res) => {
                let status = res.status().as_u16();
                let answer: PushAnswer = res.body_mut().read_json().unwrap_or_default();
                match status {
                    200 => "sent".into(),
                    410 => {
                        let mut all = app.push.targets.lock().unwrap();
                        if let Some(mine) = all.get_mut(radio) {
                            mine.retain(|t| t != target);
                            if mine.is_empty() {
                                all.remove(radio);
                            }
                        }
                        app.store.save_push_targets(&all);
                        format!("that address is no longer good ({}); forgotten", answer.reason)
                    }
                    404 | 405 => "the relay does not send notifications (it is older than this tower, or has no such channel)".into(),
                    429 => "the relay says too many were sent lately".into(),
                    503 => "the relay has no push service for that phone".into(),
                    _ => format!("not sent ({status}: {})", if answer.reason.is_empty() { "no reason given" } else { &answer.reason }),
                }
            }
        };
        log(format!("push: asked the relay to wake \"{radio}\" for {what}: {outcome}"));
    }
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
        inner.check_in(&Agent { id: "a".into(), harness: "test".into(), cwd: Some("/x/shop".into()), name: None, title: None, pid: None, deliver: None, meta: Default::default() });
        inner
    }

    #[test]
    fn a_question_or_a_report_deserves_one_and_nothing_else_does() {
        let mut inner = tower();
        let t = now();
        assert!(deserves(&line(&mut inner, "q", Some(Kind::NeedsYou), Some("Ship it?")), t));
        assert!(deserves(&line(&mut inner, "r", Some(Kind::Report), Some("Shipped.")), t));
        assert!(!deserves(&line(&mut inner, "f", Some(Kind::Fyi), Some("Starting.")), t));
        assert!(!deserves(&line(&mut inner, "s", Some(Kind::Stalled), Some("Next I will ship.")), t));
        // not before it has been read and cut down, and not long after it was said
        assert!(!deserves(&line(&mut inner, "u", None, None), t));
        assert!(!deserves(&line(&mut inner, "o", Some(Kind::NeedsYou), Some("Ship it?")), t + STALE_MS + 1));
        // and never his own words
        let to = vec![inner.sessions[0].clone()];
        inner.record(Entry { kind: Some(Kind::NeedsYou), ..Entry::his("h".into(), &to, "ship it".into()) });
        assert!(!deserves(inner.log.last().unwrap(), t));
    }

    #[test]
    fn a_radio_that_has_it_or_is_on_a_call_is_left_alone() {
        let mut inner = tower();
        let rev = line(&mut inner, "q", Some(Kind::NeedsYou), Some("Ship it?")).rev;
        // never heard from, or heard from before the line existed: wake it
        assert!(!there(None, rev));
        inner.call("phone").has = rev - 1;
        assert!(!there(inner.calls.get("phone"), rev));
        // it has asked for what follows the line: it is on his screen
        inner.call("phone").has = rev;
        assert!(there(inner.calls.get("phone"), rev));
        // on a call it is read out
        inner.call("car").last_poll = Some(Instant::now());
        assert!(there(inner.calls.get("car"), rev));
    }

    #[test]
    fn the_notification_carries_what_the_phone_needs_to_show_and_answer_it() {
        let mut inner = tower();
        inner.colours.insert("shop".into(), 4);
        let s = inner.sessions[0].clone();
        inner.record(Entry { short: Some("Rewrite or leave?".into()), options: vec!["Rewrite".into(), "Leave".into()], ..Entry::said("q".into(), &s, Some(Kind::NeedsYou), "long".into()) });
        let n = notice(&inner, &inner.log[0], "mini");
        assert_eq!((n.id.as_str(), n.from.as_str(), n.group.as_str(), n.colour, n.station.as_str(), n.text.as_str()), ("q", "shop one", "shop", 4, "mini", "Rewrite or leave?"));
        assert_eq!((n.options, n.permission, n.kind), (vec!["Rewrite".to_string(), "Leave".to_string()], false, Some(Kind::NeedsYou)));
        // sealed, it fits in one push even when the line is as long as a line gets
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        inner.log[0].short = Some("é".repeat(2000));
        let long = notice(&inner, &inner.log[0], "mini");
        assert!(keys.seal(Direction::Push, &serde_json::to_vec(&long).unwrap()).len() <= PUSH_MOST);
    }
}
