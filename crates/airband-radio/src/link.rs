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
    /// Nothing came back: no network, no relay, or the tower is not connected to the relay.
    Offline,
    /// The tower answered and said no.
    Refused,
}

pub(crate) struct Link {
    route: Route,
    agent: ureq::Agent,
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
        Ok(Link { route, agent })
    }

    /// One `airband.radio/1` request. `path` is what follows `/v1/radio`.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), LinkError> {
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
                let call = Call { t: now_ms(), id: random_hex(12), method: method.into(), path, body: body.unwrap_or(Value::Null), schema: airband_core::radio::SCHEMA.into() };
                let sealed = keys.seal(Direction::ToTower, &serde_json::to_vec(&call).unwrap());
                let mut res = self
                    .agent
                    .post(format!("{relay}/v1/channels/{}/call", keys.channel))
                    .header("content-type", "application/octet-stream")
                    .send(&sealed[..])
                    .map_err(|_| LinkError::Offline)?;
                // anything but 200 is the relay speaking (tower offline, timed out, unknown channel), not the tower
                if res.status().as_u16() != 200 {
                    return Err(LinkError::Offline);
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
        match self.request("GET", "/next", None)? {
            (200, v) => serde_json::from_value(v).map(Some).map_err(|_| LinkError::Refused),
            (204, _) => Ok(None),
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
    pub(crate) fn utterance_once(&self, id: &str, text: &str, re: Option<String>, scope: Option<String>, label: Option<String>) -> Result<Reply, LinkError> {
        let u = Utterance { text: text.to_string(), id: Some(id.to_string()), re, scope, label };
        serde_json::from_value(self.post("/utterance", &u)?).map_err(|_| LinkError::Refused)
    }

    pub(crate) fn utterance(&self, text: &str, re: Option<String>, scope: Option<String>) -> Result<Reply, LinkError> {
        let u = Utterance { text: text.to_string(), id: Some(random_hex(12)), re, scope, label: None };
        let mut last = LinkError::Offline;
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(2));
            }
            match self.post("/utterance", &u) {
                Ok(v) => return serde_json::from_value(v).map_err(|_| LinkError::Refused),
                Err(LinkError::Refused) => return Err(LinkError::Refused),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}
