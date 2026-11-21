//! Reaching the tower: straight to it, or sealed through a relay. Everything above this module makes
//! `airband.radio/1` requests and does not know which way they travel. A new transport is a new [`Route`].
use super::*;

/// The tower holds `GET /next` open for 25 seconds; anything past this means the connection is dead.
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(50);

/// The way to the tower: straight there, or sealed through a relay. Nothing above this knows which.
pub(crate) enum Route {
    Direct { url: String, auth: String, radio: String },
    Relay { relay: String, keys: Keys },
}

pub(crate) enum LinkError {
    /// Nothing came back: no network, no relay, or the tower did not answer in time.
    Offline,
    /// The relay is there and the tower is not: his computer is asleep, off or disconnected. What he says can be
    /// left with the relay for it.
    Away,
    /// The tower is back and still being handed what was left for it: what he says now joins the end of that.
    Behind,
    /// The tower answered and said no.
    Refused,
}

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
    /// Why the last thing he said did not get through, in a few words, for telling him if it never does.
    trouble: Mutex<String>,
}

impl Link {
    /// Why the last thing he said did not get through, if it is known.
    pub(crate) fn trouble(&self) -> String {
        self.trouble.lock().unwrap().clone()
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
        let route = match pairing {
            Pairing::Direct { url, token } => Route::Direct { url, auth: format!("Bearer {token}"), radio: radio.to_string() },
            Pairing::Relay { relay, secret } => Route::Relay { relay, keys: Keys::derive(&secret)? },
        };
        Ok(Link { route, agent, near: crate::net::agent_quick(POLL_TIMEOUT, Duration::from_millis(700)), direct: Mutex::new(Direct::default()), trouble: Mutex::new(String::new()), armed: Mutex::new(Vec::new()) })
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
        matches!(self.route, Route::Direct { .. }) || self.direct.lock().unwrap().good.is_some()
    }

    /// The tower has given addresses that have not been tried yet (or not lately): the next request finds out
    /// whether it is there.
    pub(crate) fn untried(&self) -> bool {
        let d = self.direct.lock().unwrap();
        d.good.is_none() && !d.addrs.is_empty() && d.rest_until.is_none_or(|t| std::time::Instant::now() >= t)
    }

    /// Make a request straight to the tower, sealed exactly as it would be for the relay. Nothing when the tower
    /// is not at any of its addresses (he is away from its network), which is remembered for a while so the
    /// relay is not kept waiting behind a dead address each time.
    fn straight(&self, keys: &Keys, method: &str, path: &str, body: &Option<Value>) -> Option<(u16, Value)> {
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
        for addr in tries {
            let call = Call { t: now_ms(), id: random_hex(12), method: method.into(), path: path.into(), body: body.clone().unwrap_or(Value::Null), schema: airband_core::radio::SCHEMA.into() };
            let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
            let answer = self.near.post(format!("{addr}/v1/sealed/{}", keys.channel)).header("content-type", "application/octet-stream").send(&sealed[..]).ok().filter(|res| res.status().as_u16() == 200).and_then(|mut res| res.body_mut().read_to_vec().ok());
            // only the tower can make an answer this key opens: anything else at that address is not it
            let answer: Option<Answer> = answer.and_then(|bytes| keys.open(Direction::ToRadio, &bytes).ok()).and_then(|p| serde_json::from_slice(&p).ok());
            if let Some(answer) = answer.filter(|a| a.id == call.id) {
                self.direct.lock().unwrap().good = Some(addr);
                return Some((answer.status, answer.body));
            }
        }
        let mut d = self.direct.lock().unwrap();
        d.good = None;
        d.rest_until = Some(std::time::Instant::now() + Duration::from_secs(40));
        None
    }

    /// One `airband.radio/1` request. `path` is what follows `/v1/radio`.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), LinkError> {
        self.request_as(method, path, body, false)
    }

    /// `ordered`: this is something he said, which must reach the tower after anything of his the relay is still
    /// holding for it. The relay turns it back (`Behind`) while there is, and it is left with the relay too.
    fn request_as(&self, method: &str, path: &str, body: Option<Value>, ordered: bool) -> Result<(u16, Value), LinkError> {
        let path = format!("/v1/radio{path}");
        match &self.route {
            Route::Direct { url, auth, radio } => {
                let res = if method == "GET" {
                    self.agent.get(format!("{url}{path}")).header("authorization", auth).header("x-airband-radio", radio).call()
                } else {
                    self.agent.post(format!("{url}{path}")).header("authorization", auth).header("x-airband-radio", radio).send_json(body.unwrap_or_else(|| serde_json::json!({})))
                };
                let mut res = res.map_err(|_| LinkError::Offline)?;
                let status = res.status().as_u16();
                Ok((status, if status == 200 { res.body_mut().read_json().map_err(|_| LinkError::Offline)? } else { Value::Null }))
            }
            Route::Relay { relay, keys } => {
                // Straight to the tower when it is on this network: no trip to the relay and back, and it works
                // when the relay does not. Something he said goes by the relay first all the same, so it keeps
                // its place behind anything of his the relay is still holding; straight only if the relay is gone.
                if !ordered {
                    if let Some(answer) = self.straight(keys, method, &path, &body) {
                        return Ok(answer);
                    }
                }
                let call = Call { t: now_ms(), id: random_hex(12), method: method.into(), path: path.clone(), body: body.clone().unwrap_or(Value::Null), schema: airband_core::radio::SCHEMA.into() };
                let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
                // the token shows the relay this is the channel's own radio: it takes nothing on the channel from anyone else
                let mut req = self.agent.post(format!("{relay}/v1/channels/{}/call", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream");
                if ordered {
                    req = req.header("x-airband-ordered", "1");
                }
                let mut res = match req.send(&sealed[..]) {
                    Ok(res) => res,
                    Err(_) => {
                        if ordered {
                            *self.trouble.lock().unwrap() = "the relay could not be reached, and neither could the computer on this network".into();
                        }
                        return if ordered { self.straight(keys, method, &path, &body).ok_or(LinkError::Offline) } else { Err(LinkError::Offline) };
                    }
                };
                // anything but 200 is the relay speaking (tower offline, timed out, unknown channel), not the tower
                // Whatever the relay makes of it, a tower that is on this network can be handed what he said
                // straight: the relay taking his computer for away, or not taking the request at all, must not
                // keep his words on the phone while the computer is in the same room.
                match res.status().as_u16() {
                    200 => {}
                    409 => return Err(LinkError::Behind),
                    status => {
                        if ordered {
                            if let Some(answer) = self.straight(keys, method, &path, &body) {
                                return Ok(answer);
                            }
                        }
                        if ordered {
                            *self.trouble.lock().unwrap() = match status {
                                504 => "the computer took what you said from the relay and did not answer".into(),
                                429 => "the relay was being asked too often".into(),
                                503 => "the relay says the computer is not connected".into(),
                                other => format!("the relay answered {other}"),
                            };
                        }
                        return Err(if status == 503 { LinkError::Away } else { LinkError::Offline });
                    }
                }
                let bytes = res.body_mut().read_to_vec().map_err(|_| LinkError::Offline)?;
                let answer: Answer = keys.open(Direction::ToRadio, &bytes).ok().and_then(|p| serde_json::from_slice(&p).ok()).ok_or(LinkError::Offline)?;
                // an answer to some other request is not an answer to this one
                if answer.id != call.id {
                    return Err(LinkError::Offline);
                }
                Ok((answer.status, answer.body))
            }
        }
    }

    /// The next transmission, or None when the tower had nothing for 25 seconds.
    pub(crate) fn next(&self) -> Result<Option<Transmission>, LinkError> {
        // a radio that makes its own speech says so, and the tower makes none for it
        match self.request("GET", &format!("/next{}", crate::mouth::asking()), None)? {
            (200, v) => serde_json::from_value(v).map(Some).map_err(|_| LinkError::Refused),
            (204, _) => Ok(None),
            _ => Err(LinkError::Refused),
        }
    }
    /// Leave something he said with the relay, for a tower that is away: sealed like any request, under the
    /// utterance's own id so leaving it twice keeps it once. The tower takes it, in order, when it is back.
    /// Only through a relay; a tower reached directly is simply tried again.
    pub(crate) fn hold(&self, u: &Utterance) -> Result<(), LinkError> {
        let (Route::Relay { relay, keys }, Some(id)) = (&self.route, u.id.as_deref()) else { return Err(LinkError::Offline) };
        let call = Call { t: now_ms(), id: random_hex(12), method: "POST".into(), path: "/v1/radio/utterance".into(), body: serde_json::to_value(u).unwrap(), schema: airband_core::radio::SCHEMA.into() };
        let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
        let res = self.agent.post(format!("{relay}/v1/channels/{}/queue", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).header("content-type", "application/octet-stream").header("x-airband-id", id).send(&sealed[..]).map_err(|_| LinkError::Offline)?;
        if res.status().as_u16() == 202 { Ok(()) } else { Err(LinkError::Offline) }
    }

    /// A pass for the listening service straight from the relay, for when his computer cannot be reached to
    /// ask. Only through a relay that lends one.
    pub(crate) fn relay_ears(&self) -> Option<airband_core::radio::EarsGrant> {
        let Route::Relay { relay, keys } = &self.route else { return None };
        let mut res = self.agent.get(format!("{relay}/v1/channels/{}/ears", keys.channel)).header("authorization", format!("Bearer {}", keys.radio_token)).call().ok()?;
        if res.status().as_u16() != 200 {
            return None;
        }
        res.body_mut().read_json().ok()
    }

    fn post_ordered<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<Value, LinkError> {
        match self.request_as("POST", path, Some(serde_json::to_value(body).unwrap()), true)? {
            (200, v) => Ok(v),
            _ => Err(LinkError::Refused),
        }
    }

    pub(crate) fn post<T: serde::Serialize>(&self, path: &str, body: &T) -> Result<Value, LinkError> {
        match self.request("POST", path, Some(serde_json::to_value(body).unwrap()))? {
            (200, v) => Ok(v),
            _ => Err(LinkError::Refused),
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
        Ok((v["next"].as_u64().ok_or(LinkError::Refused)?, v["any"].as_bool().unwrap_or(false)))
    }

    pub(crate) fn utterance_once(&self, id: &str, text: &str, re: Option<String>, scope: Option<String>, label: Option<String>, attachments: Vec<String>, others: &str) -> Result<Reply, LinkError> {
        let u = Utterance { text: text.to_string(), id: Some(id.to_string()), re, scope, label, attachments, others: serde_json::from_str(others).unwrap_or_default() };
        match self.post_ordered("/utterance", &u) {
            Ok(v) => serde_json::from_value(v).map_err(|_| LinkError::Refused),
            // his computer is away, or still catching up: the relay keeps it for it. If that fails too, the
            // caller is told which it was and tries again later.
            Err(away @ (LinkError::Away | LinkError::Behind)) => match self.hold(&u) {
                Ok(()) => Ok(Reply { say: if matches!(away, LinkError::Away) { AWAY.into() } else { String::new() }, listen: false, ..Reply::default() }),
                Err(_) => Err(away),
            },
            Err(e) => Err(e),
        }
    }

    pub(crate) fn utterance(&self, text: &str, re: Option<String>, scope: Option<String>) -> Result<Reply, LinkError> {
        // what he has attached goes with what he says; if this fails for good they are his to send again
        let attachments = self.take_armed();
        let u = Utterance { text: text.to_string(), id: Some(random_hex(12)), re, scope, label: None, attachments: attachments.clone(), others: Vec::new() };
        let mut last = LinkError::Offline;
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(2));
            }
            match self.post_ordered("/utterance", &u) {
                Ok(v) => return serde_json::from_value(v).map_err(|_| LinkError::Refused),
                Err(LinkError::Refused) => return Err(LinkError::Refused),
                // his computer is away: no use trying again in two seconds. The relay keeps it for it.
                Err(away @ (LinkError::Away | LinkError::Behind)) => match self.hold(&u) {
                    Ok(()) => return Ok(Reply { say: if matches!(away, LinkError::Away) { AWAY.into() } else { String::new() }, listen: false, ..Reply::default() }),
                    Err(e) => last = e,
                },
                Err(e) => last = e,
            }
        }
        // it never got there: what was attached is still his to send
        attachments.into_iter().for_each(|id| self.arm(id));
        Err(last)
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
            let got = link.request_as("GET", "/feed?since=0&limit=1&away=1", None, ordered);
            println!("{what}: {} in {:.1} s", match &got { Ok((status, body)) => format!("{} answered {status}", body["station"].as_str().unwrap_or("?")), Err(LinkError::Offline) => "offline".into(), Err(LinkError::Away) => "away".into(), Err(LinkError::Behind) => "behind".into(), Err(LinkError::Refused) => "refused".into() }, began.elapsed().as_secs_f32());
        }
    }
}
