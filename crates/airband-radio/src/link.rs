//! Reaching the tower: straight to it, or sealed through a relay. Everything above this module makes
//! `airband.radio/1` requests and does not know which way they travel. A new transport is a new [`Route`].
use super::*;

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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// What went wrong with one request. Each is a different thing to tell him and a different thing to do next;
/// none is folded into another.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum LinkError {
    /// Nothing answered that way: no network, or nothing at that address.
    Unreachable(Way),
    /// It was reached and gave no answer in the time allowed.
    TimedOut(Way),
    /// The relay is there and says his computer is not connected to it: asleep, off or offline. What he says
    /// can be left with the relay for it.
    Away,
    /// His computer is back and still being handed what was left for it: what he says now joins the end of that.
    Behind,
    /// The relay handed the request to his computer and no answer came back in the time the relay waits.
    TowerSilent,
    /// The relay answered, and not with his computer's answer.
    Relay(u16),
    /// The relay would not keep what he said.
    HoldRefused(u16),
    /// An answer came that this pairing's key does not open, or that was to another request.
    Unreadable(Way),
    /// His computer has what he said and is still working out the answer. Nothing is wrong.
    Working,
    /// His computer answered, and the answer was no: the status and its own explanation, when it gave one.
    Tower { status: u16, error: String },
}

/// What to do after a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Retry {
    /// It may well work in a moment.
    Soon,
    /// It will not work for a while: something is full, or was asked too often.
    Later,
    /// It will not work however often it is tried: he has to do something, or it cannot be sent.
    Never,
}

impl LinkError {
    pub(crate) fn retry(&self) -> Retry {
        match self {
            LinkError::Unreachable(_) | LinkError::TimedOut(_) | LinkError::Away | LinkError::Behind | LinkError::TowerSilent | LinkError::Working => Retry::Soon,
            // more than the relay carries, or not something it keeps: the same again gets the same answer
            LinkError::Relay(413) | LinkError::HoldRefused(400 | 413) => Retry::Never,
            // The relay does not know this phone for that computer. Just after pairing that is so for a few
            // seconds, until the computer has opened its side; the phone cannot tell that from a pairing that
            // was taken away, so it keeps trying and says what the relay said.
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => Retry::Soon,
            LinkError::Relay(500..=599) | LinkError::HoldRefused(500..=506 | 508..=599) => Retry::Soon,
            LinkError::Relay(_) | LinkError::HoldRefused(_) | LinkError::Unreadable(_) => Retry::Later,
            // the computer is in trouble of its own, which may pass; anything else it refuses, it refuses for good
            LinkError::Tower { status: 500..=599, .. } => Retry::Later,
            LinkError::Tower { .. } => Retry::Never,
        }
    }
    /// His computer answered and said no.
    pub(crate) fn refused(&self) -> bool {
        matches!(self, LinkError::Tower { .. })
    }
    /// What happened, in words for him. No capital and no full stop: it is joined to others.
    pub(crate) fn told(&self) -> String {
        match self {
            LinkError::Unreachable(Way::Relay | Way::Kept) => "there was no connection to the relay".into(),
            LinkError::Unreachable(Way::Near) => "your computer did not answer on this network".into(),
            LinkError::Unreachable(Way::Paired) => "there was no connection to your computer".into(),
            LinkError::TimedOut(Way::Relay | Way::Kept) => "the relay did not answer in time".into(),
            LinkError::TimedOut(Way::Near | Way::Paired) => "your computer did not answer in time".into(),
            LinkError::Away => "the relay says your computer is not connected".into(),
            LinkError::Behind => "earlier messages are still on their way to your computer".into(),
            LinkError::TowerSilent => "your computer took it from the relay and has not answered yet".into(),
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => "the relay does not know this phone for that computer".into(),
            LinkError::Relay(413) | LinkError::HoldRefused(413) => "it is more than the relay carries in one message".into(),
            LinkError::Relay(429) | LinkError::HoldRefused(429) => "the relay was being asked too often".into(),
            LinkError::Relay(status) => format!("the relay answered {status}"),
            LinkError::HoldRefused(507) => "the relay is keeping as many messages as it can for your computer".into(),
            LinkError::HoldRefused(status) => format!("the relay could not keep it ({status})"),
            LinkError::Unreadable(Way::Near | Way::Paired) => "your computer sent an answer that could not be read".into(),
            LinkError::Unreadable(_) => "an answer came through the relay that could not be read".into(),
            LinkError::Working => "your computer is still working on it".into(),
            LinkError::Tower { error, .. } if !error.is_empty() => format!("your computer refused it: {error}"),
            LinkError::Tower { status, .. } => format!("your computer refused it ({status})"),
        }
    }
}

/// One try at one request, one way: what the trail is made of. Nothing of what was said is in it, only the
/// kind of request, the way it went and how it ended. It never leaves the phone.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) at: u64,
    /// The method and the route, without anything after a `?`.
    pub(crate) what: String,
    pub(crate) way: Way,
    /// The status that came back, or what went wrong.
    pub(crate) outcome: Result<u16, LinkError>,
    pub(crate) ms: u64,
}

impl Step {
    /// One line of the diagnostics he can copy.
    pub(crate) fn line(&self) -> String {
        let s = self.at / 1000;
        let way = match self.way {
            Way::Relay => "relay",
            Way::Near => "own network",
            Way::Kept => "kept at relay",
            Way::Paired => "direct",
        };
        let outcome = match &self.outcome {
            Ok(status) => format!("answered {status}"),
            Err(e) => e.told(),
        };
        format!("{:02}:{:02}:{:02}Z {} by {way}: {outcome} ({} ms)", s / 3600 % 24, s / 60 % 60, s % 60, self.what, self.ms)
    }
}

/// How many steps the link remembers, for the diagnostics.
const TRAIL: usize = 300;

/// What he is told when what he said was left with the relay for a computer that is away.
pub(crate) const AWAY: &str = "Your computer is away. It gets this when it is back.";

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
    /// Files he has attached and that have reached the tower: the next thing he says takes them with it.
    armed: Mutex<Vec<String>>,
    /// Every try that failed, every try at sending something he said, and the first that worked after one
    /// that did not: the last few hundred, in memory, for him to copy when something is wrong.
    trail: Mutex<std::collections::VecDeque<Step>>,
}

/// What a failed send came to: nothing answered, or it took too long.
fn unsent(e: &ureq::Error, way: Way) -> LinkError {
    match e {
        ureq::Error::Timeout(_) => LinkError::TimedOut(way),
        _ => LinkError::Unreachable(way),
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Link {
    /// `radio` is this radio's own name for itself, the same every launch: the tower keeps each radio's place
    /// in a call by it. Through a relay the tower knows a radio by its pairing and this is not sent.
    pub(crate) fn new(pairing: Pairing, radio: &str) -> Result<Self, String> {
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
        Ok(Link { route, agent, near: crate::net::agent_quick(POLL_TIMEOUT, Duration::from_millis(700)), direct: Mutex::new(Direct { addrs: at, ..Direct::default() }), trail: Mutex::new(Default::default()), armed: Mutex::new(Vec::new()) })
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
        let mut trail = self.trail.lock().unwrap();
        let recovered = step.outcome.is_ok() && trail.iter().rev().find(|s| s.way == way).is_some_and(|s| s.outcome.is_err());
        if step.outcome.is_err() || ordered || recovered {
            trail.push_back(step.clone());
            while trail.len() > TRAIL {
                trail.pop_front();
            }
        }
        steps.push(step);
    }

    /// The trail, oldest first, a line for each step.
    pub(crate) fn trail(&self) -> Vec<String> {
        self.trail.lock().unwrap().iter().map(Step::line).collect()
    }
    /// How the last try each way went: for showing him whether this computer can be reached, and how.
    pub(crate) fn last_by(&self, way: Way) -> Option<Step> {
        self.trail.lock().unwrap().iter().rev().find(|s| s.way == way).cloned()
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
            let call = Call { t: now_ms(), id: random_hex(12), method: method.into(), path: path.into(), body: body.clone().unwrap_or(Value::Null), schema: airband_core::radio::SCHEMA.into() };
            let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
            let got = match self.near.post(format!("{addr}/v1/sealed/{}", keys.channel)).header("content-type", "application/octet-stream").send(&sealed[..]) {
                Err(e) => Err(unsent(&e, Way::Near)),
                // only the tower can make an answer this key opens: anything else at that address is not it
                Ok(mut res) => match (res.status().as_u16() == 200).then(|| res.body_mut().read_to_vec().ok()).flatten().and_then(|bytes| keys.open(Direction::ToRadio, &bytes).ok()).and_then(|p| serde_json::from_slice::<Answer>(&p).ok()).filter(|a| a.id == call.id) {
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

    /// One `airband.radio/1` request. `path` is what follows `/v1/radio`. An answer from the tower comes back
    /// with its status, whatever that is; an error is a request the tower never answered.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), LinkError> {
        self.request_as(method, path, body, false, &mut Vec::new())
    }

    /// `ordered`: this is something he said, which must reach the tower after anything of his the relay is still
    /// holding for it. The relay turns it back (`Behind`) while there is, and it is left with the relay too.
    /// `steps` is given each way it was tried and how that went.
    fn request_as(&self, method: &str, path: &str, body: Option<Value>, ordered: bool, steps: &mut Vec<Step>) -> Result<(u16, Value), LinkError> {
        let path = format!("/v1/radio{path}");
        let what = format!("{method} {path}");
        match &self.route {
            Route::Direct { url, auth, radio } => {
                let began = std::time::Instant::now();
                let res = if method == "GET" {
                    self.agent.get(format!("{url}{path}")).header("authorization", auth).header("x-airband-radio", radio).call()
                } else {
                    self.agent.post(format!("{url}{path}")).header("authorization", auth).header("x-airband-radio", radio).send_json(body.unwrap_or_else(|| serde_json::json!({})))
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
                // Straight to the tower when it is on this network: no trip to the relay and back, and it works
                // when the relay does not. Something he said goes by the relay first all the same, so it keeps
                // its place behind anything of his the relay is still holding.
                if !ordered {
                    if let Some(Ok(answer)) = self.straight(keys, method, &path, &body, ordered, steps) {
                        return Ok(answer);
                    }
                }
                let began = std::time::Instant::now();
                let call = Call { t: now_ms(), id: random_hex(12), method: method.into(), path: path.clone(), body: body.clone().unwrap_or(Value::Null), schema: airband_core::radio::SCHEMA.into() };
                let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
                // the token shows the relay this is the channel's own radio: it takes nothing on the channel from anyone else
                let mut req = self.agent.post(format!("{relay}/v1/channels/{}/call", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream");
                if ordered {
                    req = req.header("x-airband-ordered", "1");
                }
                let got = match req.send(&sealed[..]) {
                    Err(e) => Err(unsent(&e, Way::Relay)),
                    // anything but 200 is the relay speaking (tower offline, timed out, unknown channel), not the tower
                    Ok(mut res) => match res.status().as_u16() {
                        200 => match res.body_mut().read_to_vec().ok().and_then(|bytes| keys.open(Direction::ToRadio, &bytes).ok()).and_then(|p| serde_json::from_slice::<Answer>(&p).ok()) {
                            // an answer to some other request is not an answer to this one
                            Some(answer) if answer.id == call.id => Ok((answer.status, answer.body)),
                            _ => Err(LinkError::Unreadable(Way::Relay)),
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
                if ordered && !matches!(failed, LinkError::Behind | LinkError::TowerSilent) {
                    if let Some(Ok(answer)) = self.straight(keys, method, &path, &body, ordered, steps) {
                        return Ok(answer);
                    }
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
    /// that one again.
    pub(crate) fn next(&self, joined: bool, have: &str) -> Result<Option<Transmission>, LinkError> {
        // a radio that makes its own speech says so, and the tower makes none for it
        let asking = crate::mouth::asking();
        let mut query: Vec<String> = asking.strip_prefix('?').into_iter().map(String::from).collect();
        query.push(format!("have={have}"));
        if joined {
            query.push("joined=1".into());
        }
        match self.request("GET", &format!("/next?{}", query.join("&")), None)? {
            (200, v) => serde_json::from_value(v).map(Some).map_err(|_| LinkError::Unreadable(Way::Relay)),
            (204, _) => Ok(None),
            (status, v) => Err(Self::no(status, &v)),
        }
    }
    /// Leave something he said with the relay, for a tower that is away: sealed like any request, under the
    /// utterance's own id so leaving it twice keeps it once. The tower takes it, in order, when it is back.
    /// Only through a relay; a tower reached directly is simply tried again.
    fn hold(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<(), LinkError> {
        let (Route::Relay { relay, keys }, Some(id)) = (&self.route, u.id.as_deref()) else { return Err(LinkError::Unreachable(Way::Paired)) };
        let began = std::time::Instant::now();
        let call = Call { t: now_ms(), id: random_hex(12), method: "POST".into(), path: "/v1/radio/utterance".into(), body: serde_json::to_value(u).unwrap(), schema: airband_core::radio::SCHEMA.into() };
        let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
        let kept = match self.agent.post(format!("{relay}/v1/channels/{}/queue", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream").header("x-airband-id", id).send(&sealed[..]) {
            Err(e) => Err(unsent(&e, Way::Kept)),
            Ok(res) if res.status().as_u16() == 202 => Ok(202),
            Ok(res) => Err(LinkError::HoldRefused(res.status().as_u16())),
        };
        self.note(steps, true, began, "POST /v1/radio/utterance", Way::Kept, kept.clone());
        kept.map(|_| ())
    }

    /// Something he said. The tower's answer, or what stopped it: `Working` when the tower has it and is still
    /// answering an earlier try (asking again with the same id gets the answer when there is one).
    fn say_ordered(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<Reply, LinkError> {
        match self.request_as("POST", "/utterance", Some(serde_json::to_value(u).unwrap()), true, steps)? {
            (200, v) => serde_json::from_value(v).map_err(|_| LinkError::Unreadable(Way::Relay)),
            (airband_core::radio::STILL_WORKING, _) => Err(LinkError::Working),
            (status, v) => Err(Self::no(status, &v)),
        }
    }
    /// The same, and when his computer is away or still catching up, the relay keeps it for it. If the relay
    /// cannot, the caller is told why that failed, and tries again later.
    fn say_or_leave(&self, u: &Utterance, steps: &mut Vec<Step>) -> Result<Reply, LinkError> {
        match self.say_ordered(u, steps) {
            Err(away @ (LinkError::Away | LinkError::Behind)) => {
                self.hold(u, steps)?;
                Ok(Reply { say: if away == LinkError::Away { AWAY.into() } else { String::new() }, listen: false, ..Reply::default() })
            }
            other => other,
        }
    }

    pub(crate) fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<Value, LinkError> {
        match self.request("POST", path, Some(serde_json::to_value(body).unwrap()))? {
            (200, v) => Ok(v),
            (status, v) => Err(Self::no(status, &v)),
        }
    }
    /// Retried, because a reply lost in a tunnel is worse than a short wait. Every try carries the same id, so
    /// the tower delivers it once however many arrive.
    /// One try. The id makes a later retry of the same message safe.
    pub(crate) fn arm(&self, id: String) {
        let mut armed = self.armed.lock().unwrap();
        if !armed.contains(&id) {
            armed.push(id);
        }
    }
    pub(crate) fn disarm(&self, id: &str) {
        self.armed.lock().unwrap().retain(|a| a != id);
    }
    pub(crate) fn armed(&self) -> Vec<String> {
        self.armed.lock().unwrap().clone()
    }
    pub(crate) fn take_armed(&self) -> Vec<String> {
        std::mem::take(&mut *self.armed.lock().unwrap())
    }

    /// One part of a file he is attaching. Returns where the next part starts.
    /// Returns how far the tower has the file whole from its start, and whether it takes parts in any order.
    pub(crate) fn attachment_part(&self, part: &airband_core::radio::AttachmentPart) -> Result<(u64, bool), LinkError> {
        let v = self.post("/attachment", part)?;
        Ok((v["next"].as_u64().ok_or_else(|| Self::no(200, &v))?, v["any"].as_bool().unwrap_or(false)))
    }

    /// One try at sending something he typed, with each way it was tried and how that went.
    pub(crate) fn utterance_once(&self, id: &str, text: &str, re: Option<String>, scope: Option<String>, label: Option<String>, attachments: Vec<String>, others: &str) -> (Result<Reply, LinkError>, Vec<Step>) {
        let u = Utterance { text: text.to_string(), id: Some(id.to_string()), re, scope, label, attachments, others: serde_json::from_str(others).unwrap_or_default() };
        let mut steps = Vec::new();
        (self.say_or_leave(&u, &mut steps), steps)
    }

    /// Something he said aloud, tried a few times over a few seconds while he waits for the answer.
    pub(crate) fn utterance(&self, text: &str, re: Option<String>, scope: Option<String>) -> Result<Reply, LinkError> {
        // what he has attached goes with what he says; if this fails for good they are his to send again
        let attachments = self.take_armed();
        let u = Utterance { text: text.to_string(), id: Some(random_hex(12)), re, scope, label: None, attachments: attachments.clone(), others: Vec::new() };
        let mut last = LinkError::Unreachable(Way::Relay);
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(2));
            }
            match self.say_or_leave(&u, &mut Vec::new()) {
                Ok(reply) => return Ok(reply),
                Err(e) if e.retry() == Retry::Never => {
                    last = e;
                    break;
                }
                Err(e) => last = e,
            }
        }
        // it never got there: what was attached is still his to send
        attachments.into_iter().for_each(|id| self.arm(id));
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cause_says_what_to_do_next_and_what_to_tell_him() {
        use LinkError::*;
        let soon = [Unreachable(Way::Relay), Unreachable(Way::Near), TimedOut(Way::Relay), Away, Behind, TowerSilent, Working, Relay(502), HoldRefused(503), Relay(401), HoldRefused(403)];
        let later = [Relay(429), HoldRefused(507), HoldRefused(429), Unreadable(Way::Relay), Tower { status: 500, error: String::new() }];
        let never = [Relay(413), HoldRefused(400), Tower { status: 408, error: "this request's time is more than two minutes from the tower's clock".into() }, Tower { status: 400, error: String::new() }];
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
        assert!(never[2].told().contains("two minutes from the tower's clock"));
        assert!(Tower { status: 400, error: String::new() }.refused() && !Away.refused());
    }

    /// A link to a relay nobody is at, with a tower address nobody is at either.
    fn nowhere() -> Link {
        let secret: String = "ab".repeat(32);
        let link = Link::new(Pairing::Relay { relay: "http://127.0.0.1:9".into(), secret }, "test").unwrap();
        link.set_direct(vec!["http://127.0.0.1:9".into()]);
        link
    }

    #[test]
    fn something_he_said_is_tried_by_the_relay_then_straight_and_each_try_is_written_down() {
        let link = nowhere();
        let (got, steps) = link.utterance_once("0123456789ab", "ship it", None, None, None, Vec::new(), "");
        assert_eq!(got.err(), Some(LinkError::Unreachable(Way::Relay)));
        assert_eq!(steps.iter().map(|s| s.way).collect::<Vec<_>>(), [Way::Relay, Way::Near]);
        assert!(steps.iter().all(|s| s.outcome.is_err() && s.what == "POST /v1/radio/utterance"));
        // the link's own trail has them too, a line each, with nothing of what he said
        // (a port nobody listens on refuses at once on some systems and is silent on others: either is his
        // computer not answering)
        let trail = link.trail();
        assert_eq!(trail.len(), 2);
        assert!(trail[0].contains("by relay: there was no connection to the relay") && trail[1].contains("by own network: your computer did not answer"), "{trail:?}");
        assert!(!trail.iter().any(|l| l.contains("ship it")));
        // the address that did not answer is left alone for a while: the next try goes by the relay only
        let (_, again) = link.utterance_once("0123456789ab", "ship it", None, None, None, Vec::new(), "");
        assert_eq!(again.iter().map(|s| s.way).collect::<Vec<_>>(), [Way::Relay]);
        assert_eq!(link.last_by(Way::Near).map(|s| s.outcome.is_err()), Some(true));
    }

    #[test]
    fn a_question_is_asked_straight_first_and_only_its_failures_are_kept() {
        let link = nowhere();
        assert!(link.request("GET", "/feed?since=0", None).is_err());
        let trail = link.trail();
        // straight first, then the relay; what follows a `?` is not kept
        assert!(trail.len() == 2 && trail[0].contains("GET /v1/radio/feed by own network") && trail[1].contains("GET /v1/radio/feed by relay"), "{trail:?}");
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
        let link = Link::new(Pairing::parse(std::env::var("AIRBAND_LINK").unwrap().trim()).unwrap(), "probe").unwrap();
        for (what, ordered) in [("as a question (by the relay or straight)", false), ("as something he said (by the relay first)", true)] {
            let began = std::time::Instant::now();
            let mut steps = Vec::new();
            let got = link.request_as("GET", "/feed?since=0&limit=1&away=1", None, ordered, &mut steps);
            println!("{what}: {} in {:.1} s", match &got { Ok((status, body)) => format!("{} answered {status}", body["station"].as_str().unwrap_or("?")), Err(e) => e.told() }, began.elapsed().as_secs_f32());
            steps.iter().for_each(|s| println!("    {}", s.line()));
        }
    }
}
