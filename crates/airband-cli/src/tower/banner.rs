//! The banner on his phone's lock screen while the app is asleep: how many agents are waiting for him, how many
//! are working, how many reports he has not seen. The phone puts nothing on it by itself then, so the tower asks
//! the relay to have the phone's push service change it.
//!
//! A push service delivers a banner's contents unsealed. So the tower sends three numbers
//! ([`airband_core::relay::Counts`]) and nothing else: never a name, a project, a question or an agent's id.
//! What an agent asked reaches him sealed, in a notification (`push`), or in the app.
//!
//! A banner is up while someone is waiting for him. The tower puts one up when the first agent starts waiting
//! and he is not there, changes its numbers as they change, and takes it down when nobody is waiting. While
//! the app is in front or he is on a call the app looks after what he sees and the tower sends nothing. With no
//! relay there is no banner: the relay is what holds the push service's key.
//!
//! The log says when a change was asked for and what came back, never an address or the numbers.
use super::*;
use airband_core::radio::{BannerAddress, BannerTarget};
use airband_core::relay::{BannerEvent, BannerRequest, Counts, Keys, PushAnswer};
use std::collections::VecDeque;

/// How often the tower looks at whether the numbers have changed.
const LOOK: Duration = Duration::from_secs(5);
/// The least time between two changes to one phone's banner: numbers that move several times in a moment
/// arrive once, as they ended up.
const GAP: Duration = Duration::from_secs(20);
/// How many changes one phone is sent in an hour. Apple allows an app only so many and does not say how many;
/// past this the banner keeps the numbers it has until the hour moves on. Taking it down always goes.
const AN_HOUR_MOST: usize = 40;
/// A banner was asked to be put up and the phone has not yet said where to change it: how long before the
/// tower takes it that none went up and asks again.
const START_AGAIN: Duration = Duration::from_secs(600);

/// Where one radio's banner can be changed. Empty text is "no such address".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Addresses {
    provider: String,
    /// Puts a banner up.
    start: String,
    /// Changes the banner that is up.
    update: String,
}

/// What one radio's banner was last sent, and when.
#[derive(Default)]
pub(super) struct Told {
    last: Option<Counts>,
    at: Option<Instant>,
    /// When each change of the last hour was sent.
    lately: VecDeque<Instant>,
    /// A banner was asked to be put up then, and the phone has not said where to change it.
    started: Option<Instant>,
}

pub(super) struct Banners {
    addresses: Mutex<BTreeMap<String, Addresses>>,
    told: Mutex<HashMap<String, Told>>,
    /// The change count when this tower started. What was reported before it is not counted as unread for a
    /// radio the tower has not heard from since.
    since: u64,
}

impl Banners {
    pub(super) fn load(store: &Store, since: u64) -> Banners {
        Banners { addresses: Mutex::new(store.banner_addresses()), told: Mutex::default(), since }
    }
    /// Is there any phone with a banner to keep current?
    pub(super) fn any(&self) -> bool {
        !self.addresses.lock().unwrap().is_empty()
    }
}

/// A radio says where its banner can be put up or changed, or (with no token) that it no longer can.
pub(super) fn register(app: &App, radio: &str, target: BannerTarget) {
    let ok = |t: &str, most: usize| !t.is_empty() && t.len() <= most && t.bytes().all(|b| b.is_ascii_alphanumeric());
    if !ok(&target.provider, 24) || !(target.token.is_empty() || ok(&target.token, 512)) {
        return;
    }
    let mut all = app.banner.addresses.lock().unwrap();
    let mine = all.entry(radio.to_string()).or_default();
    let before = mine.clone();
    if mine.provider != target.provider {
        *mine = Addresses { provider: target.provider.clone(), ..Addresses::default() };
    }
    match target.which {
        BannerAddress::Start => mine.start = target.token,
        BannerAddress::Update => mine.update = target.token,
    }
    if *mine == before {
        return;
    }
    if target.which == BannerAddress::Update {
        // the banner asked for is up, or the one that was up has gone: either way nothing is being waited for
        app.banner.told.lock().unwrap().entry(radio.to_string()).or_default().started = None;
    }
    if mine.start.is_empty() && mine.update.is_empty() {
        all.remove(radio);
    }
    app.store.save_banner_addresses(&all);
}

/// The numbers for one radio's banner.
pub(super) fn counts(inner: &Inner, radio: &str, since: u64) -> Counts {
    let open = inner.open_ids();
    let in_state = |state: &str| inner.sessions.iter().filter(|s| inner.state_of(s, &open) == state).count() as u32;
    // what this radio has had on screen, or everything from before the tower started if it has not been heard from
    let has = inner.calls.get(radio).map_or(0, |c| c.has).max(since);
    let unread = inner.log.iter().filter(|e| e.agent.is_some() && e.kind == Some(Kind::Report) && e.short.is_some() && e.rev > has).count() as u32;
    Counts { waiting: in_state("asking"), working: in_state("working"), unread }
}

/// Is this radio looking after what he sees by itself now: is the app in front, or is he on a call?
fn there(call: Option<&Call>) -> bool {
    call.is_some_and(|c| c.presence.on_call() || c.presence.showing())
}

/// Whether to change one radio's banner now, and how: put one up, change it, or take it down, and whether it
/// is worth lighting the screen for. Records the change as sent.
pub(super) fn decide(told: &mut Told, start: bool, update: bool, counts: Counts, there: bool, now: Instant) -> Option<(BannerEvent, bool)> {
    if there {
        // The app takes its banner down when he opens it, and says so. With none up, he has these numbers in
        // front of him and they are not news when he leaves. Until the app has said so, the tower leaves what
        // it last sent as it is: if the banner is still up after all, it is brought up to date when he goes.
        if !update {
            told.last = Some(counts);
        }
        return None;
    }
    if told.last == Some(counts) || told.at.is_some_and(|t| now.duration_since(t) < GAP) {
        return None;
    }
    let event = match (counts.waiting, update, start) {
        (0, true, _) => BannerEvent::End,
        // nobody is waiting and nothing is up
        (0, false, _) => {
            told.last = Some(counts);
            return None;
        }
        (_, true, _) => BannerEvent::Update,
        (_, false, true) if told.started.is_none_or(|t| now.duration_since(t) >= START_AGAIN) => BannerEvent::Start,
        _ => return None,
    };
    while told.lately.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600)) {
        told.lately.pop_front();
    }
    if event != BannerEvent::End && told.lately.len() >= AN_HOUR_MOST {
        return None;
    }
    let urgent = counts.waiting > told.last.map_or(0, |c| c.waiting);
    told.lately.push_back(now);
    (told.last, told.at) = (Some(counts), Some(now));
    if event == BannerEvent::Start {
        told.started = Some(now);
    }
    Some((event, urgent))
}

/// Keep every paired phone's banner current.
pub(super) async fn banner_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(LOOK).await;
        if !app.banner.any() {
            continue;
        }
        let due: Vec<(String, Addresses, BannerEvent, Counts, bool)> = {
            let inner = app.inner.lock().unwrap();
            let addresses = app.banner.addresses.lock().unwrap();
            let mut told = app.banner.told.lock().unwrap();
            let now = Instant::now();
            addresses
                .iter()
                .filter_map(|(radio, at)| {
                    let numbers = counts(&inner, radio, app.banner.since);
                    let (event, urgent) = decide(told.entry(radio.clone()).or_default(), !at.start.is_empty(), !at.update.is_empty(), numbers, there(inner.calls.get(radio)), now)?;
                    Some((radio.clone(), at.clone(), event, numbers, urgent))
                })
                .collect()
        };
        for (radio, at, event, numbers, urgent) in due {
            let app = app.clone();
            let _ = tokio::task::spawn_blocking(move || send(&app, &radio, &at, event, numbers, urgent)).await;
        }
    }
}

/// Ask the relay to have one radio's banner changed.
fn send(app: &App, radio: &str, at: &Addresses, event: BannerEvent, counts: Counts, urgent: bool) {
    let cfg = config::read().unwrap_or_else(|| app.cfg.clone());
    // only a radio paired through a relay has a banner the tower can change: the relay holds the push service's key
    let (Some(relay), Some(pairing)) = (cfg.relay.clone(), cfg.pairings.iter().find(|p| p.name == radio)) else { return };
    let Ok(keys) = Keys::derive(&pairing.secret) else { return };
    let to = if event == BannerEvent::Start { at.start.clone() } else { at.update.clone() };
    let ask = BannerRequest { provider: at.provider.clone(), to, event, counts, urgent };
    let res = airband_radio::net::agent(Duration::from_secs(20)).post(format!("{}/v1/channels/{}/banner", relay.trim_end_matches('/'), keys.channel)).header("authorization", format!("Bearer {}", keys.pull_token)).send_json(&ask);
    // the address used is no longer good, or the banner it changed has been taken down: forget it
    let forget = || {
        let mut all = app.banner.addresses.lock().unwrap();
        if let Some(mine) = all.get_mut(radio) {
            if event == BannerEvent::Start { mine.start.clear() } else { mine.update.clear() }
            if mine.start.is_empty() && mine.update.is_empty() {
                all.remove(radio);
            }
        }
        app.store.save_banner_addresses(&all);
    };
    // not sent: the numbers are still news, and a banner that did not go up is not being waited for
    let again = || {
        if let Some(told) = app.banner.told.lock().unwrap().get_mut(radio) {
            (told.last, told.started) = (None, None);
        }
    };
    let outcome = match res {
        Err(e) => {
            again();
            format!("the relay could not be reached ({e})")
        }
        Ok(mut res) => {
            let status = res.status().as_u16();
            let answer: PushAnswer = res.body_mut().read_json().unwrap_or_default();
            match status {
                200 => {
                    if event == BannerEvent::End {
                        forget();
                    }
                    "done".into()
                }
                410 => {
                    forget();
                    again();
                    format!("that address is no longer good ({}); forgotten", answer.reason)
                }
                404 | 405 => "the relay does not change banners (it is older than this tower, or has no such channel)".into(),
                429 => "the relay says too many were sent lately".into(),
                503 => "the relay has no push service for that phone".into(),
                _ => {
                    again();
                    format!("not done ({status}: {})", if answer.reason.is_empty() { "no reason given" } else { &answer.reason })
                }
            }
        }
    };
    let what = match event {
        BannerEvent::Start => "put a banner up on",
        BannerEvent::Update => "change the banner on",
        BannerEvent::End => "take the banner down on",
    };
    log(format!("banner: asked the relay to {what} \"{radio}\": {outcome}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(waiting: u32, working: u32, unread: u32) -> Counts {
        Counts { waiting, working, unread }
    }

    #[test]
    fn numbers_that_move_in_a_moment_arrive_once_as_they_ended_up() {
        let (mut told, t0) = (Told::default(), Instant::now());
        let s = Duration::from_secs;
        // the first agent starts waiting: a banner goes up, and lights the screen
        assert_eq!(decide(&mut told, true, false, n(1, 2, 0), false, t0), Some((BannerEvent::Start, true)));
        // the phone says where to change it
        told.started = None;
        // three changes in the next few seconds: nothing is sent until the gap has passed, then the last one
        assert_eq!(decide(&mut told, true, true, n(1, 1, 0), false, t0 + s(5)), None);
        assert_eq!(decide(&mut told, true, true, n(1, 1, 1), false, t0 + s(10)), None);
        assert_eq!(decide(&mut told, true, true, n(1, 0, 2), false, t0 + s(15)), None);
        assert_eq!(decide(&mut told, true, true, n(1, 0, 2), false, t0 + GAP), Some((BannerEvent::Update, false)));
        // the same numbers again are not sent again
        assert_eq!(decide(&mut told, true, true, n(1, 0, 2), false, t0 + GAP * 3), None);
        // one more waiting is worth lighting the screen for; one fewer is not
        assert_eq!(decide(&mut told, true, true, n(2, 0, 2), false, t0 + GAP * 4), Some((BannerEvent::Update, true)));
        assert_eq!(decide(&mut told, true, true, n(1, 0, 2), false, t0 + GAP * 5), Some((BannerEvent::Update, false)));
    }

    #[test]
    fn it_comes_down_when_nobody_is_waiting_and_is_not_put_up_twice() {
        let (mut told, t0) = (Told::default(), Instant::now());
        assert_eq!(decide(&mut told, true, false, n(1, 0, 0), false, t0), Some((BannerEvent::Start, true)));
        // the phone has not yet said where to change it: no second banner
        assert_eq!(decide(&mut told, true, false, n(2, 0, 0), false, t0 + GAP), None);
        // unless none ever went up
        assert_eq!(decide(&mut told, true, false, n(2, 0, 0), false, t0 + START_AGAIN), Some((BannerEvent::Start, true)));
        told.started = None;
        // he answered: it comes down, whatever else is still going on
        assert_eq!(decide(&mut told, true, true, n(0, 3, 1), false, t0 + START_AGAIN + GAP), Some((BannerEvent::End, false)));
        // with none up and nobody waiting, working agents and unread reports put nothing up
        assert_eq!(decide(&mut told, true, false, n(0, 4, 2), false, t0 + START_AGAIN + GAP * 2), None);
        // a phone that gave no address is sent nothing
        assert_eq!(decide(&mut Told::default(), false, false, n(3, 0, 0), false, t0), None);
    }

    #[test]
    fn a_phone_he_is_looking_at_is_sent_nothing_and_what_he_saw_is_not_news() {
        let (mut told, t0) = (Told::default(), Instant::now());
        assert_eq!(decide(&mut told, true, false, n(1, 0, 0), true, t0), None);
        // he puts the phone down with the same agent still waiting: he knows
        assert_eq!(decide(&mut told, true, false, n(1, 0, 0), false, t0 + GAP), None);
        // then another starts waiting
        assert_eq!(decide(&mut told, true, false, n(2, 0, 0), false, t0 + GAP * 2), Some((BannerEvent::Start, true)));        // a banner the app has not said it took down is left alone while he is there, and caught up after
        told.started = None;
        assert_eq!(decide(&mut told, true, true, n(1, 0, 0), true, t0 + GAP * 3), None);
        assert_eq!(decide(&mut told, true, true, n(1, 0, 0), false, t0 + GAP * 4), Some((BannerEvent::Update, false)));
    }

    #[test]
    fn only_so_many_in_an_hour_but_taking_it_down_always_goes() {
        let (mut told, t0) = (Told::default(), Instant::now());
        // a change at every gap for an hour is more than the most
        let tries = (3600 / GAP.as_secs()) as u32;
        assert!(tries as usize > AN_HOUR_MOST);
        let sent = (0..tries).filter(|i| decide(&mut told, true, true, n(1, *i, 0), false, t0 + GAP * *i).is_some()).count();
        assert_eq!(sent, AN_HOUR_MOST);
        assert_eq!(decide(&mut told, true, true, n(0, 0, 0), false, t0 + GAP * tries), Some((BannerEvent::End, false)));
        // and when the hour has moved on, changes go again
        assert_eq!(decide(&mut told, true, true, n(1, 0, 0), false, t0 + Duration::from_secs(2 * 3600)), Some((BannerEvent::Update, true)));
    }

    #[test]
    fn the_numbers_are_who_is_waiting_who_is_working_and_what_he_has_not_seen() {
        let mut inner = Inner::default();
        for id in ["a", "b", "c"] {
            inner.check_in(&Agent { id: id.into(), harness: "test".into(), cwd: Some("/x/shop".into()), name: None, title: None, pid: None, deliver: None, meta: Default::default() });
        }
        let (a, b) = (inner.sessions[0].clone(), inner.sessions[1].clone());
        inner.sessions[0].waiting = true;
        inner.sessions[1].waiting = true;
        inner.record(Entry { short: Some("Ship it?".into()), ..Entry::said("q".into(), &a, Some(Kind::NeedsYou), "long".into()) });
        inner.record(Entry { short: Some("Shipped.".into()), ..Entry::said("r".into(), &b, Some(Kind::Report), "long".into()) });
        assert_eq!(counts(&inner, "phone", 0), n(1, 1, 1));
        // he has had the report on screen
        inner.call("phone").has = inner.log.last().unwrap().rev;
        assert_eq!(counts(&inner, "phone", 0), n(1, 1, 0));
        // what was reported before the tower started is not unread for a phone it has not heard from
        assert_eq!(counts(&inner, "another", inner.rev), n(1, 1, 0));
    }
}
