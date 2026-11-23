//! Reaching the tower: straight to it, or sealed through a relay. Everything above this module makes
//! requests of the radio protocol, each one an [`Endpoint`] of core's, and does not know which way they
//! travel. A new transport is a new [`Route`].
#![expect(clippy::disallowed_methods, reason = "the phone's own connection to its computers")]
use super::*;

mod errors;
mod trail;
pub(crate) use errors::*;
pub(crate) use trail::*;

/// The tower holds `GET /next` open for 25 seconds; anything past this means the connection is dead.
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(50);

use airband_core::radio::wire::{self, Endpoint, Nothing, Refusal, at};

/// The way to the tower: straight there, or sealed through a relay. Nothing above this knows which.
pub(crate) enum Route {
    Direct { url: String, auth: String, radio: String },
    Relay { relay: String, keys: Keys },
    /// Sealed like the relay's, with no relay: only ever straight to the tower's own addresses.
    Near { keys: Keys },
    /// Through something that stands in for the network ([`Transport`]): a scripted tower in tests.
    Through(Arc<dyn Transport>),
}

/// What carries one request of the radio protocol to a tower and its answer back, in place of the network: the
/// method, the whole path, and the body as JSON; the tower's status and what it answered. A stand-in tower
/// implements it, so what the desk does with typed answers is tested without a computer.
pub(crate) trait Transport: Send + Sync {
    fn exchange(&self, method: Method, path: &str, body: Option<Value>) -> Result<(u16, Value), LinkError>;
}

/// Which way a request was tried.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Way {
    /// Through the relay.
    Relay,
    /// Straight to his computer, at its own address on the network the radio is on.
    Near,
    /// Left with the relay for a computer that is away.
    Kept,
    /// To a tower this radio is paired with directly, with no relay.
    Paired,
}

/// What he is told when what he said was left with the relay for a computer that is away.
pub(crate) const AWAY: &str = "Your computer is away. It gets this when it is back.";

/// What came of one try at sending something he said.
pub(crate) enum Sent {
    /// His computer has it and is working out what to do with it. What comes of it arrives in the feed.
    Taken,
    /// His computer had it already, and this is what came of it.
    Answered(Reply),
    /// The relay is keeping it for his computer: because the computer is `away`, or because the relay is still
    /// handing it earlier words of his and these wait their turn behind them.
    Left { away: bool },
}

/// The tower's own addresses, for reaching it without the relay when the radio is on its network.
#[derive(Default)]
struct Direct {
    /// What the tower last said it can be reached at.
    addrs: Vec<String>,
    /// The one that last worked.
    good: Option<String>,
    /// None of them answered: not tried again before this.
    rest_until: Option<std::time::Instant>,
}

pub(crate) struct Link {
    route: Route,
    agent: ureq::Agent,
    /// For the tower's own addresses: quick to give up when it is not there.
    near: ureq::Agent,
    direct: Mutex<Direct>,
    /// Every try that failed, every try at sending something he said, and the first that worked after one
    /// that did not: the last few hundred, for him to copy when something is wrong. Kept in a file of the
    /// app's own on this device, so it is still there after the app was closed, and sent nowhere.
    pub(crate) trail: Arc<Trail>,
    /// Where this radio stands, which every request says: on a call or not, showing the chat or not.
    presence: Mutex<airband_core::radio::Presence>,
    /// The relay still keeps something he said for his computer. While it does, what he says next goes by the
    /// relay, behind it; otherwise everything goes straight to his computer whenever it is on this network.
    kept_at_relay: std::sync::atomic::AtomicBool,
    /// What the tower was told that nobody waits to hear the answer to (he heard this, he saw that), and
    /// that did not get there: how many of each, since the app started.
    lost: Mutex<std::collections::BTreeMap<String, u32>>,
    /// Things he said aloud whose outcome someone here is waiting for, by id, and the outcome once the feed
    /// has told it.
    awaited: Mutex<std::collections::HashMap<String, Option<Reply>>>,
    told: std::sync::Condvar,
}

/// What a failed send came to: nothing answered, or it took too long.
fn unsent(e: &ureq::Error, way: Way) -> LinkError {
    match e {
        ureq::Error::Timeout(_) => LinkError::TimedOut(way),
        _ => LinkError::Unreachable(way),
    }
}

pub(crate) use airband_core::clock::now_ms;

/// The way an answer came: the last way its request was tried, as its steps were written down. Every way that
/// brings an answer writes its step before handing the answer back, so an answer always has one.
/// How a request is made of the tower.
pub(crate) use airband_core::radio::wire::Method;

impl Link {
    /// `radio` is this radio's own name for itself, the same every launch: the tower keeps each radio's place
    /// in a call by it. Through a relay the tower knows a radio by its pairing and this is not sent.
    /// `trail` is the computer's record of how requests went ([`Trail`]), the same for each of its links.
    pub(crate) fn new(pairing: Pairing, radio: &str, trail: Arc<Trail>) -> Result<Self, String> {
        let agent = crate::net::agent(POLL_TIMEOUT);
        let mut at = Vec::new();
        let route = match pairing {
            Pairing::Near { at: given, secret } => {
                at = given;
                Route::Near { keys: Keys::derive(&secret)? }
            }
            Pairing::Direct { url, token } => Route::Direct { url, auth: format!("Bearer {token}"), radio: radio.to_string() },
            Pairing::Relay { relay, secret } => Route::Relay { relay, keys: Keys::derive(&secret)? },
        };
        Ok(Link { route, agent, near: crate::net::agent_quick(POLL_TIMEOUT, Duration::from_millis(700)), direct: Mutex::new(Direct { addrs: at, ..Direct::default() }), trail, presence: Mutex::default(), kept_at_relay: Default::default(), lost: Mutex::default(), awaited: Mutex::default(), told: std::sync::Condvar::new() })
    }

    /// Reach the tower through `transport` and nothing else.
    pub(crate) fn through(mut self, transport: Arc<dyn Transport>) -> Self {
        self.route = Route::Through(transport);
        self
    }

    /// The tower said where it can be reached without the relay. Anything new is tried at the next request.
    pub(crate) fn set_direct(&self, addrs: Vec<String>) {
        let mut d = self.direct.lock().unwrap();
        if d.addrs != addrs {
            *d = Direct { addrs, good: None, rest_until: None };
        }
    }

    /// Is the tower being reached straight, on its own network, and not through the relay?
    pub(crate) fn is_direct(&self) -> bool {
        matches!(self.route, Route::Direct { .. } | Route::Near { .. } | Route::Through(_)) || self.direct.lock().unwrap().good.is_some()
    }

    /// The tower has given addresses that have not been tried yet (or not lately): the next request finds out
    /// whether it is there.
    pub(crate) fn untried(&self) -> bool {
        let d = self.direct.lock().unwrap();
        d.good.is_none() && !d.addrs.is_empty() && d.rest_until.is_none_or(|t| std::time::Instant::now() >= t)
    }

    /// Write a step down: in `steps` for whoever is following this one request, and in the link's own trail
    /// when it failed, when it carried something he said, or when it is the first to work after a failure.
    fn note(&self, steps: &mut Vec<Step>, ordered: bool, began: std::time::Instant, what: &str, way: Way, outcome: Result<u16, LinkError>) {
        let step = Step { at: now_ms(), what: what.split('?').next().unwrap_or_default().to_string(), way, outcome, ms: began.elapsed().as_millis() as u64 };
        let recovered = step.outcome.is_ok() && self.last_by(way).is_some_and(|s| s.outcome.is_err());
        if step.outcome.is_err() || ordered || recovered {
            self.keep(Line::Step(step.clone()));
        }
        steps.push(step);
    }

    /// A request as it is sealed for the tower: made now, under an id of its own, saying where this radio
    /// stands.
    fn sealed_call(&self, method: Method, path: &str, body: Value) -> Call {
        Call { t: now_ms(), id: random_hex(12), method, path: path.into(), body, schema: airband_core::radio::SCHEMA.into(), presence: self.presence() }
    }

    /// Add a line to the trail, and to its file.
    fn keep(&self, line: Line) {
        let mut trail = self.trail.lines.lock().unwrap();
        // a file that could not be read when the app opened: once it can be, its lines come first
        if let Some(Some(found)) = self.trail.keeper.read_again::<std::collections::VecDeque<Line>>() {
            let since = std::mem::replace(&mut *trail, found);
            trail.extend(since);
        }
        trail.push_back(line);
        while trail.len() > TRAIL {
            trail.pop_front();
        }
        // the record is for him to read: one that could not be kept says so in itself, the first time in a row
        if let Some(why) = self.trail.keeper.save(&*trail).news() {
            trail.push_back(Line::Remark { at: now_ms(), what: format!("{} The lines after this are lost when the app closes.", crate::kept::not_kept(why)) });
        }
    }
    /// Something worth a line in the trail that is not a step. Never anything he or an agent said.
    pub(crate) fn remark(&self, what: String) {
        self.keep(Line::Remark { at: now_ms(), what });
    }

    /// The trail, oldest first, a line for each step.
    pub(crate) fn trail(&self) -> Vec<String> {
        self.trail.lines.lock().unwrap().iter().map(Line::told).collect()
    }
    /// How the last try each way went: for showing him whether this computer can be reached, and how.
    pub(crate) fn last_by(&self, way: Way) -> Option<Step> {
        self.trail.lines.lock().unwrap().iter().rev().find_map(|l| match l {
            Line::Step(s) if s.way == way => Some(s.clone()),
            _ => None,
        })
    }

    /// Where this radio stands now. True when that is news.
    pub(crate) fn set_presence(&self, change: impl FnOnce(&mut airband_core::radio::Presence)) -> bool {
        let mut presence = self.presence.lock().unwrap();
        let was = *presence;
        change(&mut presence);
        *presence != was
    }
    fn presence(&self) -> airband_core::radio::Presence {
        *self.presence.lock().unwrap()
    }
    /// Say where this radio stands, with nothing else to ask.
    pub(crate) fn say_presence(&self) {
        self.tell::<at::Presence>(&self.presence());
    }

    /// Tell the tower something nobody here waits to hear the answer to. It is designed to be harmless when
    /// lost; one that did not get there is counted by its route, so the diagnostics say how often that happens.
    pub(crate) fn tell<E: Endpoint<Query = Nothing>>(&self, body: &E::Body) {
        if self.call::<E>(None, &Nothing {}, body).is_err() {
            *self.lost.lock().unwrap().entry(E::PATH.rsplit('/').next().unwrap_or_default().to_string()).or_default() += 1;
        }
    }
    /// How many of each kind of thing told that way did not get there, since the app started.
    pub(crate) fn lost(&self) -> Vec<(String, u32)> {
        self.lost.lock().unwrap().iter().map(|(what, n)| (what.clone(), *n)).collect()
    }

    /// The feed told what came of something he said. Whoever is waiting for it here is given it.
    pub(crate) fn answered(&self, id: &str, reply: &Reply) {
        if let Some(slot) = self.awaited.lock().unwrap().get_mut(id) {
            *slot = Some(reply.clone());
            self.told.notify_all();
        }
    }
    /// Wait up to `at_most` for the feed to tell what came of an utterance someone here expects.
    fn await_answer(&self, id: &str, at_most: Duration) -> Option<Reply> {
        let awaited = self.awaited.lock().unwrap();
        let (mut awaited, _) = self.told.wait_timeout_while(awaited, at_most, |a| a.get(id).is_some_and(Option::is_none)).unwrap();
        awaited.get_mut(id).and_then(Option::take)
    }
    pub(crate) fn by_relay(&self) -> bool {
        matches!(self.route, Route::Relay { .. })
    }
    /// The way the last step of a request went; with none taken, the way this pairing reaches the tower.
    fn went(&self, steps: &[Step]) -> Way {
        steps.last().map_or(if self.by_relay() { Way::Relay } else { Way::Paired }, |s| s.way)
    }

    /// Make a request straight to the tower, sealed exactly as it would be for the relay. `None` when there is
    /// nothing to try: the tower has given no address, or none of them answered a moment ago (he is away from
    /// its network), which is remembered for a while so the relay is not kept waiting behind a dead address
    /// each time.
    fn straight(&self, keys: &Keys, method: Method, path: &str, body: &Option<Value>, ordered: bool, steps: &mut Vec<Step>) -> Option<Result<(u16, Value), LinkError>> {
        let tries: Vec<String> = {
            let d = self.direct.lock().unwrap();
            if d.addrs.is_empty() || d.rest_until.is_some_and(|t| std::time::Instant::now() < t) {
                return None;
            }
            match &d.good {
                Some(good) => vec![good.clone()],
                None => d.addrs.clone(),
            }
        };
        let what = format!("{method} {path}");
        let mut last = LinkError::Unreachable(Way::Near);
        for addr in tries {
            let began = std::time::Instant::now();
            let call = self.sealed_call(method, path, body.clone().unwrap_or(Value::Null));
            let sealed = keys.seal_call(&call);
            let got = match self.near.post(format!("{addr}/v1/sealed/{}", keys.channel)).header("content-type", "application/octet-stream").send(&sealed[..]) {
                Err(e) => Err(unsent(&e, Way::Near)),
                // only the tower can make an answer this key opens: anything else at that address is not it
                Ok(mut res) => match (res.status().as_u16() == 200).then(|| res.body_mut().read_to_vec().ok()).flatten().and_then(|bytes| keys.open_answer(&bytes, &call.id)) {
                    Some(answer) => Ok((answer.status, answer.body)),
                    None => Err(LinkError::Unreadable(Way::Near)),
                },
            };
            self.note(steps, ordered, began, &what, Way::Near, got.as_ref().map(|(status, _)| *status).map_err(Clone::clone));
            match got {
                Ok(answer) => {
                    self.direct.lock().unwrap().good = Some(addr);
                    return Some(Ok(answer));
                }
                Err(e) => last = e,
            }
        }
        let mut d = self.direct.lock().unwrap();
        d.good = None;
        // with a relay to fall back on, a dead address is left alone for a while; with none it is the only way
        d.rest_until = matches!(self.route, Route::Relay { .. }).then(|| std::time::Instant::now() + Duration::from_secs(40));
        Some(Err(last))
    }

    /// One request of the radio protocol ([`airband_core::radio::SCHEMA`]), by its endpoint: `id` fills the one
    /// part of the path that names something, when the endpoint has one. A 200 is read as the endpoint's
    /// answer; anything else the tower answers is its refusal ([`LinkError::Refused`]) or its own trouble.
    pub(crate) fn call<E: Endpoint>(&self, id: Option<&str>, query: &E::Query, body: &E::Body) -> Result<E::Answer, LinkError> {
        let mut steps = Vec::new();
        let body = (E::METHOD == Method::Post).then(|| serde_json::to_value(body).expect("a request type writes as JSON"));
        let got = self.request_as(E::METHOD, &wire::address::<E>(id, query), body, false, &mut steps)?;
        self.answer(got, &steps)
    }

    /// An answer from the tower, read: the endpoint's own on a 200, the refusal it carries otherwise.
    fn answer<A: serde::de::DeserializeOwned>(&self, (status, v): (u16, Value), steps: &[Step]) -> Result<A, LinkError> {
        match status {
            200 => serde_json::from_value(v).map_err(|_| LinkError::Unreadable(self.went(steps))),
            _ => Err(LinkError::refusal(status, v)),
        }
    }

    /// `path` is the whole path, as the tower serves it. `ordered`: this is something he said, which must reach the tower after anything of his the relay is still
    /// holding for it. The relay turns it back (`Behind`) while there is, and it is left with the relay too.
    /// `steps` is given each way it was tried and how that went.
    fn request_as(&self, method: Method, path: &str, body: Option<Value>, ordered: bool, steps: &mut Vec<Step>) -> Result<(u16, Value), LinkError> {
        let what = format!("{method} {path}");
        match &self.route {
            Route::Direct { url, auth, radio } => {
                let began = std::time::Instant::now();
                let presence = serde_json::to_string(&self.presence()).unwrap();
                let headers = [("authorization", auth.as_str()), (airband_core::radio::RADIO_HEADER, radio.as_str()), (airband_core::radio::PRESENCE_HEADER, &presence), (airband_core::radio::SCHEMA_HEADER, airband_core::radio::SCHEMA)];
                let address = format!("{url}{path}");
                let res = match method {
                    Method::Get => headers.iter().fold(self.agent.get(&address), |r, (k, v)| r.header(*k, *v)).call(),
                    Method::Post => headers.iter().fold(self.agent.post(&address), |r, (k, v)| r.header(*k, *v)).send_json(body.unwrap_or_else(|| serde_json::Value::Object(Default::default()))),
                    Method::Delete => headers.iter().fold(self.agent.delete(&address), |r, (k, v)| r.header(*k, *v)).call(),
                };
                let got = match res {
                    Err(e) => Err(unsent(&e, Way::Paired)),
                    Ok(mut res) => {
                        let status = res.status().as_u16();
                        match res.body_mut().read_json::<Value>() {
                            Ok(v) => Ok((status, v)),
                            // a refusal may come with nothing to read; an answer must be readable
                            Err(_) if status != 200 => Ok((status, Value::Null)),
                            Err(_) => Err(LinkError::Unreadable(Way::Paired)),
                        }
                    }
                };
                self.note(steps, ordered, began, &what, Way::Paired, got.as_ref().map(|(status, _)| *status).map_err(Clone::clone));
                got
            }
            Route::Near { keys } => self.straight(keys, method, path, &body, ordered, steps).unwrap_or(Err(LinkError::Unreachable(Way::Near))),
            Route::Through(transport) => {
                let began = std::time::Instant::now();
                let got = transport.exchange(method, path, body);
                self.note(steps, ordered, began, &what, Way::Paired, got.as_ref().map(|(status, _)| *status).map_err(Clone::clone));
                got
            }
            Route::Relay { relay, keys } => {
                // Straight to the tower whenever it is on this network: no trip to the relay and back, and it
                // works when the relay does not. Only while the relay still keeps earlier words of his does
                // something he says go by the relay first, so it keeps its place behind them.
                if !(ordered && self.kept_at_relay.load(std::sync::atomic::Ordering::SeqCst))
                    && let Some(Ok(answer)) = self.straight(keys, method, path, &body, ordered, steps) {
                        return Ok(answer);
                    }
                let began = std::time::Instant::now();
                let call = self.sealed_call(method, path, body.clone().unwrap_or(Value::Null));
                let sealed = keys.seal_call(&call);
                // the token shows the relay this is the channel's own radio: it takes nothing on the channel from anyone else
                let mut req = self.agent.post(keys.at(relay, "/call")).header("authorization", keys.as_radio()).header("content-type", "application/octet-stream");
                if ordered {
                    req = req.header(airband_core::relay::ORDERED_HEADER, "1");
                }
                let got = match req.send(&sealed[..]) {
                    Err(e) => Err(unsent(&e, Way::Relay)),
                    // anything but 200 is the relay speaking (tower offline, timed out, unknown channel), not the tower
                    Ok(mut res) => match res.status().as_u16() {
                        // an answer to some other request is not an answer to this one
                        200 => match res.body_mut().read_to_vec().ok().and_then(|bytes| keys.open_answer(&bytes, &call.id)) {
                            Some(answer) => Ok((answer.status, answer.body)),
                            None => Err(LinkError::Unreadable(Way::Relay)),
                        },
                        409 => Err(LinkError::Behind),
                        503 => Err(LinkError::Away),
                        504 => Err(LinkError::TowerSilent),
                        status => Err(LinkError::Relay(status)),
                    },
                };
                self.note(steps, ordered, began, &what, Way::Relay, got.as_ref().map(|(status, _)| *status).map_err(Clone::clone));
                let failed = match got {
                    Ok(answer) => return Ok(answer),
                    Err(e) => e,
                };
                // Whatever the relay made of it, a tower on this network can be handed what he said straight:
                // the relay taking his computer for away, or not taking the request at all, must not keep his
                // words on the phone while the computer is in the same room. Two exceptions. While the relay
                // is still handing over earlier words of his, these wait their turn behind them. And when the
                // tower already has the request and is slow to answer, asking it again another way only makes
                // two of them wait.
                if ordered && self.kept_at_relay.load(std::sync::atomic::Ordering::SeqCst) && !matches!(failed, LinkError::Behind | LinkError::TowerSilent)
                    && let Some(Ok(answer)) = self.straight(keys, method, path, &body, ordered, steps) {
                        return Ok(answer);
                    }
                Err(failed)
            }
        }
    }

    /// The next transmission, or None when the tower had nothing for 25 seconds. `have` is the id of the last
    /// one this radio got, so a tower that gave it another since, in an answer that never arrived, gives it
    /// that one again. What comes back is words: this radio makes the sound.
    pub(crate) fn next(&self, joined: bool, have: &str) -> Result<Option<Transmission>, LinkError> {
        self.call::<at::Next>(None, &wire::NextQuery { joined, have: (!have.is_empty()).then(|| have.to_string()) }, &Nothing {})
    }
    /// Leave something he said with the relay, for a tower that is away: sealed like any request, under the
    /// utterance's own id so leaving it twice keeps it once. The tower takes it, in order, when it is back.
    /// Only through a relay; a tower reached directly is simply tried again.
    fn hold(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<(), LinkError> {
        let Route::Relay { relay, keys } = &self.route else { return Err(LinkError::Unreachable(Way::Paired)) };
        let id = &u.id;
        let began = std::time::Instant::now();
        let path = at::Utterance::PATH;
        let sealed = keys.seal_call(&self.sealed_call(Method::Post, path, serde_json::to_value(u).unwrap()));
        let kept = match self.agent.post(keys.at(relay, "/queue")).header("authorization", keys.as_radio()).header("content-type", "application/octet-stream").header(airband_core::relay::ID_HEADER, id).send(&sealed[..]) {
            Err(e) => Err(unsent(&e, Way::Kept)),
            Ok(res) if res.status().as_u16() == 202 => Ok(202),
            Ok(res) => Err(LinkError::HoldRefused(res.status().as_u16())),
        };
        self.note(steps, true, began, &format!("{} {path}", Method::Post), Way::Kept, kept.clone());
        kept.map(|_| ())
    }

    /// Something he said. Where it stands with the tower, or what stopped it reaching it.
    /// The outbox says whether the relay still keeps something he said.
    pub(crate) fn keeps_at_relay(&self, kept: bool) {
        self.kept_at_relay.store(kept, std::sync::atomic::Ordering::SeqCst);
    }

    fn say_ordered(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<airband_core::radio::Outcome, LinkError> {
        let got = self.request_as(at::Utterance::METHOD, at::Utterance::PATH, Some(serde_json::to_value(u).unwrap()), true, steps)?;
        self.answer(got, steps)
    }
    /// The same, and when his computer is away or still catching up, the relay keeps it for it. If the relay
    /// cannot, the caller is told why that failed, and tries again later.
    fn say_or_leave(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<Sent, LinkError> {
        use airband_core::radio::Outcome;
        match self.say_ordered(u, steps) {
            Ok(Outcome::Taken) => Ok(Sent::Taken),
            Ok(Outcome::Answered { reply }) => Ok(Sent::Answered(reply)),
            // the tower would not act on it, and said why
            Ok(Outcome::Rejected { why }) => Err(LinkError::Rejected(why)),
            Err(away @ (LinkError::Away | LinkError::Behind)) => {
                self.hold(u, steps)?;
                Ok(Sent::Left { away: away == LinkError::Away })
            }
            Err(e) => Err(e),
        }
    }

    /// One part of a file he is attaching. Returns how far the tower has the file whole from its start.
    pub(crate) fn attachment_part(&self, part: &airband_core::radio::AttachmentPart) -> Result<u64, LinkError> {
        Ok(self.call::<at::Attachment>(None, &Nothing {}, part)?.next)
    }

    /// One try at sending something he typed, with each way it was tried and how that went.
    pub(crate) fn utterance_once(&self, u: &Utterance) -> (Result<Sent, LinkError>, Vec<Step>) {
        let mut steps = Vec::new();
        (self.say_or_leave(u, &mut steps), steps)
    }

    /// Something he said aloud, tried a few times over a few seconds while he waits for the answer. The tower
    /// takes it at once; its answer comes by the feed, or to this asking again, and is waited for as long as a
    /// person on a call would. Nothing, when the tower has it and has not said what came of it in that time:
    /// it is in the chat when it does.
    pub(crate) fn say_and_wait(&self, u: &Utterance) -> Result<Option<Reply>, LinkError> {
        self.awaited.lock().unwrap().insert(u.id.clone(), None);
        let got = self.keep_trying(u);
        self.awaited.lock().unwrap().remove(&u.id);
        got
    }
    fn keep_trying(&self, u: &Utterance) -> Result<Option<Reply>, LinkError> {
        /// How long his answer is waited for before the microphone is given back to him.
        const WAIT: Duration = Duration::from_secs(90);
        /// The longest the tower goes unasked meanwhile, for a radio that is not following the feed. It is
        /// asked sooner at first: most answers take a second or two.
        const ASK_AGAIN: Duration = Duration::from_secs(5);
        let began = std::time::Instant::now();
        let (mut failures, mut taken, mut ask_in) = (0, false, Duration::from_secs(1));
        while began.elapsed() < WAIT {
            match self.say_or_leave(u, &mut Vec::new()) {
                Ok(Sent::Answered(reply)) => return Ok(Some(reply)),
                Ok(Sent::Left { away }) => return Ok(Some(Reply { say: if away { AWAY.into() } else { String::new() }, ..Reply::default() })),
                Ok(Sent::Taken) => {
                    taken = true;
                    if let Some(reply) = self.await_answer(&u.id, ask_in) {
                        return Ok(Some(reply));
                    }
                    ask_in = (ask_in * 2).min(ASK_AGAIN);
                }
                // once the tower has it, a request that fails is only a question that went unanswered
                Err(_) if taken => thread::sleep(ASK_AGAIN),
                Err(e) if e.retry() == Retry::Never || failures == 2 => return Err(e),
                Err(_) => {
                    failures += 1;
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cause_says_what_to_do_next_and_what_to_tell_him() {
        use LinkError::*;
        let soon = [Unreachable(Way::Relay), Unreachable(Way::Near), TimedOut(Way::Relay), Away, Behind, TowerSilent, Relay(502), HoldRefused(503), Relay(401), HoldRefused(403)];
        let later = [Relay(429), HoldRefused(507), HoldRefused(429), Unreadable(Way::Relay), Trouble(500)];
        let never = [Relay(413), HoldRefused(400), Refused(Refusal::told("the clocks of your phone and your computer are more than two minutes apart")), Trouble(400), Refused(Refusal::OtherVersion { speaks: "airband.radio/9".into() }), Refused(Refusal::Gone)];
        for (wanted, causes) in [(Retry::Soon, &soon[..]), (Retry::Later, &later[..]), (Retry::Never, &never[..])] {
            for cause in causes {
                assert_eq!(cause.retry(), wanted, "{cause:?}");
            }
        }
        // each is told in its own words, and none as another is
        let all: Vec<&LinkError> = soon.iter().chain(&later).chain(&never).collect();
        let told: std::collections::HashSet<String> = all.iter().map(|c| c.told()).collect();
        assert!(told.len() >= all.len() - 3, "{told:?}");
        assert!(all.iter().all(|c| !c.told().is_empty() && !c.told().ends_with('.')));
        // the tower's own explanation reaches him
        assert!(never[2].told().contains("more than two minutes apart"));
        // a no is never tried again; his computer's trouble and a lost connection are
        assert!([Trouble(400), Refused(Refusal::Gone)].iter().all(|e| e.retry() == Retry::Never) && [Trouble(503), Away].iter().all(|e| e.retry() != Retry::Never));
    }

    fn said(id: &str, text: &str) -> Utterance {
        Utterance { text: text.into(), id: id.into(), ..Default::default() }
    }

    /// A link to a relay nobody is at, with a tower address nobody is at either.
    fn nowhere() -> Link {
        let secret: String = "ab".repeat(32);
        let link = Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret }, "test", Trail::open(None)).unwrap();
        link.set_direct(vec!["http://127.0.0.1:9".into()]);
        link
    }

    #[test]
    fn something_he_said_is_tried_straight_then_by_the_relay_and_each_try_is_written_down() {
        let link = nowhere();
        let (got, steps) = link.utterance_once(&said("0123456789ab", "ship it"));
        assert_eq!(got.err(), Some(LinkError::Unreachable(Way::Relay)));
        assert_eq!(steps.iter().map(|s| s.way).collect::<Vec<_>>(), [Way::Near, Way::Relay]);
        assert!(steps.iter().all(|s| s.outcome.is_err() && s.what == "POST /v1/radio/utterance"));
        // the link's own trail has them too, a line each, with nothing of what he said
        // (a port nobody listens on refuses at once on some systems and is silent on others: either is his
        // computer not answering)
        let trail = link.trail();
        assert_eq!(trail.len(), 2);
        assert!(trail[0].contains("by own network: your computer did not answer") && trail[1].contains("by relay: there was no connection to the relay"), "{trail:?}");
        assert!(!trail.iter().any(|l| l.contains("ship it")));
        // the address that did not answer is left alone for a while: the next try goes by the relay only
        let (_, again) = link.utterance_once(&said("0123456789ab", "ship it"));
        assert_eq!(again.iter().map(|s| s.way).collect::<Vec<_>>(), [Way::Relay]);
        assert_eq!(link.last_by(Way::Near).map(|s| s.outcome.is_err()), Some(true));
    }

    #[cfg(unix)]
    #[test]
    fn a_trail_that_could_not_be_read_or_kept_says_so_in_itself_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-trail-bad-{}", airband_core::encode::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("trail.json");
        std::fs::write(&file, b"[ half").unwrap();
        // the folder cannot be written: the old file cannot be moved aside, and nothing can be kept
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let link = Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret: "cd".repeat(32) }, "test", Trail::open(Some(file.clone()))).unwrap();
        link.remark("one".into());
        link.remark("two".into());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let trail = link.trail();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(trail[0].contains("could not be read"), "{trail:?}");
        assert_eq!(trail.iter().filter(|l| l.contains("lost when the app closes")).count(), 1, "{trail:?}");
    }

    #[test]
    fn the_trail_is_still_there_after_the_app_was_closed() {
        let file = std::env::temp_dir().join(format!("airband-trail-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let secret: String = "cd".repeat(32);
        let open = || Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret: secret.clone() }, "test", Trail::open(Some(file.clone()))).unwrap();
        let link = open();
        let _ = link.utterance_once(&said("0123456789ab", "ship it"));
        link.remark("something worth a line".into());
        link.tell::<at::Heard>(&Heard { id: "m1".into() });
        assert_eq!(link.lost(), [("heard".to_string(), 1)]);
        let had = link.trail();
        assert!(had.len() == 3 && had[1].ends_with("something worth a line"), "{had:?}");
        drop(link);
        // opened again: the same lines, and the file holds nothing he said and nothing of the pairing
        assert_eq!(open().trail(), had);
        let kept = std::fs::read_to_string(&file).unwrap();
        let keys = Keys::derive(&secret).unwrap();
        assert!(!kept.contains("ship it") && !kept.contains(&secret) && !kept.contains(&keys.channel) && !kept.contains(&keys.radio_token) && !kept.contains("127.0.0.1"), "{kept}");
        // and it never grows past its bound
        let link = open();
        (0..TRAIL + 20).for_each(|n| link.remark(format!("line {n}")));
        assert_eq!(open().trail().len(), TRAIL);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_question_is_asked_straight_first_and_only_its_failures_are_kept() {
        let link = nowhere();
        assert!(link.call::<at::Feed>(None, &wire::FeedQuery { since: Some(0), ..Default::default() }, &Nothing {}).is_err());
        let trail = link.trail();
        // straight first, then the relay; what follows a `?` is not kept
        assert!(trail.len() == 2 && trail[0].contains("GET /v1/radio/feed by own network") && trail[1].contains("GET /v1/radio/feed by relay"), "{trail:?}");
    }

    #[test]
    fn what_he_says_goes_straight_first_unless_the_relay_still_keeps_earlier_words() {
        let ways = |link: &Link| -> Vec<Way> {
            let mut steps = Vec::new();
            let _ = link.request_as(Method::Post, at::Utterance::PATH, None, true, &mut steps);
            steps.iter().map(|s| s.way).collect()
        };
        // nothing kept: his own network first, the relay when the computer is not on it
        let link = nowhere();
        assert_eq!(ways(&link), [Way::Near, Way::Relay]);
        // the relay keeps earlier words of his: by the relay first, so these stay behind them
        link.keeps_at_relay(true);
        assert_eq!(ways(&link)[0], Way::Relay);
        // a pairing with no relay never goes near one
        let near = Link::new(Pairing::Near { at: vec!["http://127.0.0.1:9".into()], secret: "ab".repeat(32) }, "test", Trail::open(None)).unwrap();
        assert_eq!(ways(&near), [Way::Near]);
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    /// Against a real pairing, the way a phone asks: `AIRBAND_LINK=… cargo test -p airband-radio -- --ignored
    /// --nocapture asks_a_real_tower`. Says how each kind of request went, and nothing of the pairing.
    #[test]
    #[ignore]
    fn asks_a_real_tower() {
        let link = Link::new(Pairing::parse(std::env::var("AIRBAND_LINK").unwrap().trim()).unwrap(), "probe", Trail::open(None)).unwrap();
        for (what, ordered) in [("as a question (by the relay or straight)", false), ("as something he said (by the relay first)", true)] {
            let began = std::time::Instant::now();
            let mut steps = Vec::new();
            let got = link.request_as(Method::Get, "/feed?since=0&limit=1", None, ordered, &mut steps);
            println!("{what}: {} in {:.1} s", match &got { Ok((status, body)) => format!("{} answered {status}", body["station"].as_str().unwrap_or("?")), Err(e) => e.told() }, began.elapsed().as_secs_f32());
            steps.iter().for_each(|s| println!("    {}", s.line()));
        }
    }
}
