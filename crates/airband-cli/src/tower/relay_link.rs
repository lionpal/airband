//! The tower's side of `airband.relay/1`: for each paired radio, hold a channel open on the relay, pull that
//! radio's sealed requests, run them exactly as if they had arrived on the local port, and seal the answers.
//!
//! The tower only ever calls out, so it works behind any router with no port opened. The relay sees sealed
//! bytes; a request that does not open with a paired radio's key, is not fresh, or was already answered is
//! dropped here.
use crate::config::{self, PairedRadio};
use super::{App, RADIO_HEADER, log, now as now_ms};
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tower::ServiceExt;
use airband_core::relay::{Answer, Call, Direction, FRESH_MS, Keys, fresh};


/// Watch the config: a pairing added by `airband pair` starts being served, one removed by `airband unpair`
/// stops, and a changed relay address moves everything, all without restarting the tower.
pub fn start(app: Arc<App>, router: Router, handle: Handle) {
    std::thread::spawn(move || {
        let mut running: HashMap<(String, PairedRadio), Arc<AtomicBool>> = HashMap::new();
        loop {
            let cfg = config::read().unwrap_or_else(|| app.cfg.clone());
            let wanted: Vec<(String, PairedRadio)> = cfg.relay.iter().flat_map(|r| cfg.pairings.iter().map(move |p| (r.clone(), p.clone()))).collect();
            running.retain(|key, stop| {
                let keep = wanted.contains(key);
                if !keep {
                    stop.store(true, Ordering::SeqCst);
                    log(format!("\"{}\" is no longer served", key.1.name));
                }
                keep
            });
            for key in wanted {
                if running.contains_key(&key) {
                    continue;
                }
                let keys = match Keys::derive(&key.1.secret) {
                    Ok(k) => k,
                    Err(e) => {
                        log(format!("pairing \"{}\" is unusable: {e}", key.1.name));
                        continue;
                    }
                };
                let stop = Arc::new(AtomicBool::new(false));
                running.insert(key.clone(), stop.clone());
                let serve = Serve { relay: key.0, name: key.1.name, keys, admission: cfg.relay_admission.clone(), token: app.cfg.radio_token.clone(), router: router.clone(), handle: handle.clone(), stop, seen: Arc::new(Mutex::new(HashMap::new())), replies: airband_radio::net::agent(Duration::from_secs(15)) };
                std::thread::spawn(move || serve.run());
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

#[derive(Clone)]
struct Serve {
    relay: String,
    name: String,
    keys: Keys,
    admission: Option<String>,
    /// The radio token, for handing requests to the tower's local routes: a paired radio gets `/v1/radio/` only.
    token: String,
    router: Router,
    handle: Handle,
    stop: Arc<AtomicBool>,
    /// Ids of calls already answered, with when: a recorded request played back is dropped.
    seen: Arc<Mutex<HashMap<String, u64>>>,
    /// The HTTP agent answers go back through.
    replies: ureq::Agent,
}

impl Serve {
    fn run(self) {
        let agent = airband_radio::net::agent(Duration::from_secs(40));
        let auth = format!("Bearer {}", self.keys.pull_token);
        let base = format!("{}/v1/channels/{}", self.relay, self.keys.channel);
        let (mut open, mut wait, mut told) = (false, 1, false);
        let nap = |wait: &mut u64| {
            std::thread::sleep(Duration::from_secs(*wait));
            *wait = (*wait * 2).min(15);
        };
        while !self.stop.load(Ordering::SeqCst) {
            if !open {
                let mut req = agent.put(&base).header("authorization", &auth);
                if let Some(a) = &self.admission {
                    req = req.header("x-airband-admission", a);
                }
                match req.send_empty().map(|r| r.status().as_u16()) {
                    Ok(201 | 204) => {
                        log(format!("serving \"{}\" through {}", self.name, self.relay));
                        (open, wait, told) = (true, 1, false);
                    }
                    other => {
                        if !told {
                            log(match other {
                                Ok(401) => format!("{} needs an admission key: airband relay <url> --admission <key>", self.relay),
                                Ok(403) => format!("{} says the channel for \"{}\" belongs to another tower; pair again", self.relay, self.name),
                                Ok(code) => format!("{} refused to open a channel ({code})", self.relay),
                                Err(e) => format!("cannot reach {}: {e}; retrying", self.relay),
                            });
                            told = true;
                        }
                        nap(&mut wait);
                    }
                }
                continue;
            }
            match agent.get(format!("{base}/pull")).header("authorization", &auth).call() {
                Ok(mut res) => match res.status().as_u16() {
                    200 => {
                        wait = 1;
                        let id = res.headers().get("x-airband-request").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                        if let Ok(sealed) = res.body_mut().read_to_vec() {
                            let me = self.clone();
                            // not awaited: `next` is held for 25 s, and other requests must get through meanwhile
                            self.handle.spawn(async move { me.answer(id, sealed).await });
                        }
                    }
                    204 => wait = 1,
                    // the relay restarted without its data, or the channel changed hands: open it again
                    403 | 404 => open = false,
                    _ => nap(&mut wait),
                },
                Err(e) => {
                    if !told {
                        log(format!("lost {}: {e}; retrying", self.relay));
                        told = true;
                    }
                    open = false;
                    nap(&mut wait);
                }
            }
        }
    }

    async fn answer(self, request_id: String, sealed: Vec<u8>) {
        let Some(call) = self.keys.open(Direction::ToTower, &sealed).ok().and_then(|p| serde_json::from_slice::<Call>(&p).ok()) else {
            log(format!("dropped a request on \"{}\"'s channel that its key does not open", self.name));
            // one the relay was keeping has to be taken off its hands all the same, or it would be handed over
            // for ever and hold up everything kept behind it
            if request_id.starts_with(airband_core::relay::HELD) {
                let (url, auth, agent) = (format!("{}/v1/channels/{}/reply/{request_id}", self.relay, self.keys.channel), format!("Bearer {}", self.keys.pull_token), self.replies.clone());
                let _ = tokio::task::spawn_blocking(move || agent.post(url).header("authorization", auth).send_empty().map(|_| ())).await;
            }
            return;
        };
        let now = now_ms();
        // A message the relay kept while this tower was away is old on purpose, and may be handed over twice.
        // Only something he said is taken that way, and only with an id: the tower takes each id once, for good,
        // so neither its age nor a second copy can make it happen again.
        let kept = request_id.starts_with(airband_core::relay::HELD);
        let (status, body) = if kept {
            let said = call.method == "POST" && call.path == "/v1/radio/utterance" && call.body["id"].as_str().is_some_and(|id| !id.is_empty());
            if said && now.saturating_sub(call.t) <= airband_core::relay::HELD_FOR_MS {
                log(format!("\"{}\" said this while the tower was away", self.name));
                self.dispatch(&call).await
            } else {
                (400, serde_json::json!({ "error": "only something he said, sent in the last week, is taken late" }))
            }
        } else if !fresh(call.t, now) {
            // answered, not dropped, so a phone with a wrong clock is told why nothing works
            (408, serde_json::json!({ "error": "this request's time is more than two minutes from the tower's clock" }))
        } else {
            {
                let mut seen = self.seen.lock().unwrap();
                seen.retain(|_, t| t.abs_diff(now) <= FRESH_MS * 2);
                if seen.insert(call.id.clone(), now).is_some() {
                    return log(format!("dropped a replayed request on \"{}\"'s channel", self.name));
                }
            }
            self.dispatch(&call).await
        };
        let sealed = self.keys.seal(Direction::ToRadio, &serde_json::to_vec(&Answer { id: call.id, status, body }).unwrap());
        let url = format!("{}/v1/channels/{}/reply/{request_id}", self.relay, self.keys.channel);
        let auth = format!("Bearer {}", self.keys.pull_token);
        // one agent for every answer on this channel: its connection to the relay is kept, not made again each time
        let agent = self.replies.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = agent.post(url).header("authorization", auth).header("content-type", "application/octet-stream").send(&sealed[..]);
        })
        .await;
    }

    /// Run a radio's request through the same routes the local port serves. Only the radio's routes: a paired
    /// phone cannot post agent events or read the status page.
    async fn dispatch(&self, call: &Call) -> (u16, Value) {
        if !call.schema.is_empty() && call.schema != airband_core::radio::SCHEMA {
            return (400, serde_json::json!({ "error": format!("this tower speaks {}, not \"{}\"", airband_core::radio::SCHEMA, call.schema) }));
        }
        if !call.path.starts_with("/v1/radio/") || !matches!(call.method.as_str(), "GET" | "POST") {
            return (403, serde_json::json!({ "error": "a radio may only use /v1/radio/" }));
        }
        let body = if call.body.is_null() { Body::empty() } else { Body::from(serde_json::to_vec(&call.body).unwrap()) };
        let Ok(req) = Request::builder().method(call.method.as_str()).uri(&call.path).header("authorization", format!("Bearer {}", self.token)).header(RADIO_HEADER, &self.name).header("content-type", "application/json").body(body) else {
            return (400, Value::Null);
        };
        let Ok(res) = self.router.clone().oneshot(req).await;
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap_or_default();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}
