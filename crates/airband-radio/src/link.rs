//! Reaching the tower: straight to it, or sealed through a relay. Everything above this module makes
//! `airband.radio/2` requests and does not know which way they travel. A new transport is a new [`Route`].
use super::*;

mod errors;
mod trail;
pub(crate) use errors::*;
pub(crate) use trail::*;

/// The tower holds `GET /next` open for 25 seconds; anything past this means the connection is dead.
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(50);

/// The way to the tower: straight there, or sealed through a relay. Nothing above this knows which.
pub(crate) enum Route {
    Direct { url: String, auth: String, radio: String },
    Relay { relay: String, keys: Keys },
    /// Sealed like the relay's, with no relay: only ever straight to the tower's own addresses.
    Near { keys: Keys },
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
    trail: Mutex<std::collections::VecDeque<Line>>,
    trail_file: Option<std::path::PathBuf>,
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
fn went(steps: &[Step]) -> Way {
    steps.last().map_or(Way::Relay, |s| s.way)
}

impl Link {
    /// `radio` is this radio's own name for itself, the same every launch: the tower keeps each radio's place
    /// in a call by it. Through a relay the tower knows a radio by its pairing and this is not sent.
    /// `trail_file` is where the trail is kept between launches; nothing keeps it in memory only.
    pub(crate) fn new(pairing: Pairing, radio: &str, trail_file: Option<std::path::PathBuf>) -> Result<Self, String> {
        let agent = crate::net::agent(POLL_TIMEOUT);
        let trail = trail_file.as_deref().and_then(airband_core::kept::load).unwrap_or_default();
        let mut at = Vec::new();
        let route = match pairing {
            Pairing::Near { at: given, secret } => {
                at = given;
                Route::Near { keys: Keys::derive(&secret)? }
            }
            Pairing::Direct { url, token } => Route::Direct { url, auth: format!("Bearer {token}"), radio: radio.to_string() },
            Pairing::Relay { relay, secret } => Route::Relay { relay, keys: Keys::derive(&secret)? },
        };
        Ok(Link { route, agent, near: crate::net::agent_quick(POLL_TIMEOUT, Duration::from_millis(700)), direct: Mutex::new(Direct { addrs: at, ..Direct::default() }), trail: Mutex::new(trail), trail_file, presence: Mutex::default(), kept_at_relay: Default::default(), lost: Mutex::default(), awaited: Mutex::default(), told: std::sync::Condvar::new() })
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
        matches!(self.route, Route::Direct { .. } | Route::Near { .. }) || self.direct.lock().unwrap().good.is_some()
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
    fn call(&self, method: &str, path: &str, body: Value) -> Call {
        Call { t: now_ms(), id: random_hex(12), method: method.into(), path: path.into(), body, schema: airband_core::radio::SCHEMA.into(), presence: self.presence() }
    }

    /// Add a line to the trail, and to its file.
    fn keep(&self, line: Line) {
        let mut trail = self.trail.lock().unwrap();
        trail.push_back(line);
        while trail.len() > TRAIL {
            trail.pop_front();
        }
        if let Some(file) = &self.trail_file {
            let _ = airband_core::kept::save(file, &*trail);
        }
    }
    /// Something worth a line in the trail that is not a step. Never anything he or an agent said.
    pub(crate) fn remark(&self, what: String) {
        self.keep(Line::Remark { at: now_ms(), what });
    }

    /// The trail, oldest first, a line for each step.
    pub(crate) fn trail(&self) -> Vec<String> {
        self.trail.lock().unwrap().iter().map(Line::told).collect()
    }
    /// How the last try each way went: for showing him whether this computer can be reached, and how.
    pub(crate) fn last_by(&self, way: Way) -> Option<Step> {
        self.trail.lock().unwrap().iter().rev().find_map(|l| match l {
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
        self.tell("/presence", &self.presence());
    }

    /// Tell the tower something nobody here waits to hear the answer to. It is designed to be harmless when
    /// lost; one that did not get there is counted, so the diagnostics say how often that happens.
    pub(crate) fn tell<T: serde::Serialize>(&self, path: &str, body: &T) {
        if self.post(path, body).is_err() {
            *self.lost.lock().unwrap().entry(path.split('?').next().unwrap_or_default().trim_start_matches('/').to_string()).or_default() += 1;
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

    /// Make a request straight to the tower, sealed exactly as it would be for the relay. `None` when there is
    /// nothing to try: the tower has given no address, or none of them answered a moment ago (he is away from
    /// its network), which is remembered for a while so the relay is not kept waiting behind a dead address
    /// each time.
    fn straight(&self, keys: &Keys, method: &str, path: &str, body: &Option<Value>, ordered: bool, steps: &mut Vec<Step>) -> Option<Result<(u16, Value), LinkError>> {
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
            let call = self.call(method, path, body.clone().unwrap_or(Value::Null));
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

    /// One `airband.radio/2` request. `path` is what follows `/v1/radio`. An answer from the tower comes back
    /// with its status, whatever that is; an error is a request the tower never answered.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), LinkError> {
        self.request_as(method, path, body, false, &mut Vec::new())
    }

    /// `ordered`: this is something he said, which must reach the tower after anything of his the relay is still
    /// holding for it. The relay turns it back (`Behind`) while there is, and it is left with the relay too.
    /// `steps` is given each way it was tried and how that went.
    fn request_as(&self, method: &str, path: &str, body: Option<Value>, ordered: bool, steps: &mut Vec<Step>) -> Result<(u16, Value), LinkError> {
        let path = format!("{}{}", airband_core::radio::PATH.trim_end_matches('/'), path);
        let what = format!("{method} {path}");
        match &self.route {
            Route::Direct { url, auth, radio } => {
                let began = std::time::Instant::now();
                let presence = serde_json::to_string(&self.presence()).unwrap();
                let res = if method == "GET" {
                    self.agent.get(format!("{url}{path}")).header("authorization", auth).header(airband_core::radio::RADIO_HEADER, radio).header(airband_core::radio::PRESENCE_HEADER, &presence).call()
                } else {
                    self.agent.post(format!("{url}{path}")).header("authorization", auth).header(airband_core::radio::RADIO_HEADER, radio).header(airband_core::radio::PRESENCE_HEADER, &presence).send_json(body.unwrap_or_else(|| serde_json::json!({})))
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
            Route::Near { keys } => self.straight(keys, method, &path, &body, ordered, steps).unwrap_or(Err(LinkError::Unreachable(Way::Near))),
            Route::Relay { relay, keys } => {
                // Straight to the tower whenever it is on this network: no trip to the relay and back, and it
                // works when the relay does not. Only while the relay still keeps earlier words of his does
                // something he says go by the relay first, so it keeps its place behind them.
                if !(ordered && self.kept_at_relay.load(std::sync::atomic::Ordering::SeqCst))
                    && let Some(Ok(answer)) = self.straight(keys, method, &path, &body, ordered, steps) {
                        return Ok(answer);
                    }
                let began = std::time::Instant::now();
                let call = self.call(method, &path, body.clone().unwrap_or(Value::Null));
                let sealed = keys.seal_call(&call);
                // the token shows the relay this is the channel's own radio: it takes nothing on the channel from anyone else
                let mut req = self.agent.post(format!("{relay}/v1/channels/{}/call", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream");
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
                    && let Some(Ok(answer)) = self.straight(keys, method, &path, &body, ordered, steps) {
                        return Ok(answer);
                    }
                Err(failed)
            }
        }
    }

    /// What the tower said no with, as an error: its status and its own explanation.
    fn no(status: u16, v: &Value) -> LinkError {
        LinkError::Tower { status, error: v["error"].as_str().unwrap_or_default().to_string() }
    }

    /// The next transmission, or None when the tower had nothing for 25 seconds. `have` is the id of the last
    /// one this radio got, so a tower that gave it another since, in an answer that never arrived, gives it
    /// that one again. What comes back is words: this radio makes the sound.
    pub(crate) fn next(&self, joined: bool, have: &str) -> Result<Option<Transmission>, LinkError> {
        let mut query = vec![format!("have={have}")];
        if joined {
            query.push("joined=1".into());
        }
        let mut steps = Vec::new();
        match self.request_as("GET", &format!("/next?{}", query.join("&")), None, false, &mut steps)? {
            (200, v) => serde_json::from_value(v).map(Some).map_err(|_| LinkError::Unreadable(went(&steps))),
            (204, _) => Ok(None),
            (status, v) => Err(Self::no(status, &v)),
        }
    }
    /// Leave something he said with the relay, for a tower that is away: sealed like any request, under the
    /// utterance's own id so leaving it twice keeps it once. The tower takes it, in order, when it is back.
    /// Only through a relay; a tower reached directly is simply tried again.
    fn hold(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<(), LinkError> {
        let Route::Relay { relay, keys } = &self.route else { return Err(LinkError::Unreachable(Way::Paired)) };
        let id = &u.id;
        let began = std::time::Instant::now();
        let sealed = keys.seal_call(&self.call("POST", "/v1/radio/utterance", serde_json::to_value(u).unwrap()));
        let kept = match self.agent.post(format!("{relay}/v1/channels/{}/queue", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream").header(airband_core::relay::ID_HEADER, id).send(&sealed[..]) {
            Err(e) => Err(unsent(&e, Way::Kept)),
            Ok(res) if res.status().as_u16() == 202 => Ok(202),
            Ok(res) => Err(LinkError::HoldRefused(res.status().as_u16())),
        };
        self.note(steps, true, began, "POST /v1/radio/utterance", Way::Kept, kept.clone());
        kept.map(|_| ())
    }

    /// Something he said. Where it stands with the tower, or what stopped it reaching it.
    /// The outbox says whether the relay still keeps something he said.
    pub(crate) fn keeps_at_relay(&self, kept: bool) {
        self.kept_at_relay.store(kept, std::sync::atomic::Ordering::SeqCst);
    }

    fn say_ordered(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<airband_core::radio::Outcome, LinkError> {
        match self.request_as("POST", "/utterance", Some(serde_json::to_value(u).unwrap()), true, steps)? {
            (200, v) => serde_json::from_value(v).map_err(|_| LinkError::Unreadable(went(steps))),
            (status, v) => Err(Self::no(status, &v)),
        }
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

    pub(crate) fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<Value, LinkError> {
        match self.request("POST", path, Some(serde_json::to_value(body).unwrap()))? {
            (200, v) => Ok(v),
            (status, v) => Err(Self::no(status, &v)),
        }
    }
    /// One part of a file he is attaching. Returns where the next part starts.
    /// Returns how far the tower has the file whole from its start, and whether it takes parts in any order.
    pub(crate) fn attachment_part(&self, part: &airband_core::radio::AttachmentPart) -> Result<u64, LinkError> {
        let v = self.post("/attachment", part)?;
        v["next"].as_u64().ok_or_else(|| Self::no(200, &v))
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
        let later = [Relay(429), HoldRefused(507), HoldRefused(429), Unreadable(Way::Relay), Tower { status: 500, error: String::new() }];
        let never = [Relay(413), HoldRefused(400), Tower { status: 408, error: "the clocks of your phone and your computer are more than two minutes apart".into() }, Tower { status: 400, error: String::new() }];
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
        assert!(Tower { status: 400, error: String::new() }.refused() && !Away.refused());
    }

    fn said(id: &str, text: &str) -> Utterance {
        Utterance { text: text.into(), id: id.into(), ..Default::default() }
    }

    /// A link to a relay nobody is at, with a tower address nobody is at either.
    fn nowhere() -> Link {
        let secret: String = "ab".repeat(32);
        let link = Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret }, "test", None).unwrap();
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

    #[test]
    fn the_trail_is_still_there_after_the_app_was_closed() {
        let file = std::env::temp_dir().join(format!("airband-trail-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let secret: String = "cd".repeat(32);
        let open = || Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret: secret.clone() }, "test", Some(file.clone())).unwrap();
        let link = open();
        let _ = link.utterance_once(&said("0123456789ab", "ship it"));
        link.remark("something worth a line".into());
        link.tell("/heard", &Heard { id: "m1".into() });
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
        assert!(link.request("GET", "/feed?since=0", None).is_err());
        let trail = link.trail();
        // straight first, then the relay; what follows a `?` is not kept
        assert!(trail.len() == 2 && trail[0].contains("GET /v1/radio/feed by own network") && trail[1].contains("GET /v1/radio/feed by relay"), "{trail:?}");
    }

    #[test]
    fn what_he_says_goes_straight_first_unless_the_relay_still_keeps_earlier_words() {
        let ways = |link: &Link| -> Vec<Way> {
            let mut steps = Vec::new();
            let _ = link.request_as("POST", "/utterance", None, true, &mut steps);
            steps.iter().map(|s| s.way).collect()
        };
        // nothing kept: his own network first, the relay when the computer is not on it
        let link = nowhere();
        assert_eq!(ways(&link), [Way::Near, Way::Relay]);
        // the relay keeps earlier words of his: by the relay first, so these stay behind them
        link.keeps_at_relay(true);
        assert_eq!(ways(&link)[0], Way::Relay);
        // a pairing with no relay never goes near one
        let near = Link::new(Pairing::Near { at: vec!["http://127.0.0.1:9".into()], secret: "ab".repeat(32) }, "test", None).unwrap();
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
        let link = Link::new(Pairing::parse(std::env::var("AIRBAND_LINK").unwrap().trim()).unwrap(), "probe", None).unwrap();
        for (what, ordered) in [("as a question (by the relay or straight)", false), ("as something he said (by the relay first)", true)] {
            let began = std::time::Instant::now();
            let mut steps = Vec::new();
            let got = link.request_as("GET", "/feed?since=0&limit=1", None, ordered, &mut steps);
            println!("{what}: {} in {:.1} s", match &got { Ok((status, body)) => format!("{} answered {status}", body["station"].as_str().unwrap_or("?")), Err(e) => e.told() }, began.elapsed().as_secs_f32());
            steps.iter().for_each(|s| println!("    {}", s.line()));
        }
    }
}
