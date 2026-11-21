//! The tower's side of `airband.relay/1`: for each paired radio, hold a channel open on the relay, pull that
//! radio's sealed requests, run them exactly as if they had arrived on the local port, and seal the answers.
//!
//! The tower only ever calls out, so it works behind any router with no port opened. The relay sees sealed
//! bytes; a request that does not open with a paired radio's key, is not fresh, or was already answered is
//! dropped here.
use crate::config::{self, PairedRadio};
use super::{App, PRESENCE_HEADER, RADIO_HEADER, VIA_HEADER, log, now_ms};
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
use airband_core::relay::{Answer, Call, FRESH_MS, Keys, fresh};


/// What a radio whose clock is wrong is told.
const WRONG_CLOCK: &str = "the clocks of your phone and your computer are more than two minutes apart";

/// Every pairing being served, by its channel: a request that comes straight to the tower is answered by the
/// same code, with the same key, as one pulled from the relay.
static SERVED: std::sync::OnceLock<Mutex<HashMap<String, Serve>>> = std::sync::OnceLock::new();
fn served() -> &'static Mutex<HashMap<String, Serve>> {
    SERVED.get_or_init(Default::default)
}

/// The one route a radio uses to reach the tower without the relay: the sealed request it would have given the
/// relay, answered with the sealed answer. Whoever sends it needs the pairing key to make a request the tower
/// will open, and to read what comes back; nothing else is served here.
pub fn sealed_router() -> Router {
    Router::new().route("/v1/sealed/{channel}", axum::routing::post(sealed))
}

async fn sealed(axum::extract::Path(channel): axum::extract::Path<String>, body: axum::body::Bytes) -> axum::response::Response {
    use axum::response::IntoResponse;
    let serve = served().lock().unwrap().get(&channel).cloned();
    let Some(serve) = serve else { return axum::http::StatusCode::NOT_FOUND.into_response() };
    match serve.answer_here(&body).await {
        Some(sealed) => sealed.into_response(),
        None => axum::http::StatusCode::BAD_REQUEST.into_response(),
    }
}

/// Watch the config: a pairing added by `airband pair` starts being served, one removed by `airband unpair`
/// stops, and a changed relay address moves everything, all without restarting the tower.
pub fn start(app: Arc<App>, router: Router, handle: Handle) {
    std::thread::spawn(move || {
        let mut running: HashMap<(Option<String>, PairedRadio), (Arc<AtomicBool>, std::thread::JoinHandle<()>)> = HashMap::new();
        loop {
            let cfg = config::read().unwrap_or_else(|| app.cfg.clone());
            // every pairing is served straight, at the tower's own addresses; through the relay too when there is one
            let wanted: Vec<(Option<String>, PairedRadio)> = cfg.pairings.iter().map(|p| (cfg.relay.clone(), p.clone())).collect();
            running.retain(|key, (stop, serving)| {
                let keep = wanted.contains(key);
                // A pairing whose serving thread has ended while it is still wanted is started again below:
                // with nobody pulling, the relay would call the computer away while it is on.
                if keep && serving.is_finished() {
                    log(format!("the link serving \"{}\" stopped; starting it again", key.1.name));
                    return false;
                }
                if !keep {
                    stop.store(true, Ordering::SeqCst);
                    if let Ok(keys) = Keys::derive(&key.1.secret) {
                        served().lock().unwrap().remove(&keys.channel);
                    }
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
                let serve = Serve { app: app.clone(), relay: key.0.clone(), name: key.1.name.clone(), keys, admission: cfg.relay_admission.clone(), token: app.cfg.radio_token.clone(), router: router.clone(), handle: handle.clone(), stop: stop.clone(), seen: Arc::new(Mutex::new(HashMap::new())), replies: airband_radio::net::agent(Duration::from_secs(15)) };
                served().lock().unwrap().insert(serve.keys.channel.clone(), serve.clone());
                running.insert(key, (stop, std::thread::spawn(move || serve.run())));
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

#[derive(Clone)]
struct Serve {
    app: Arc<App>,
    /// The relay this pairing is also reached through, when there is one.
    relay: Option<String>,
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
        // with no relay there is nothing to pull: requests come straight (`answer_here`) until this is stopped
        let Some(relay) = self.relay.clone() else {
            log(format!("serving \"{}\" at this computer's own addresses, with no relay", self.name));
            while !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
            }
            return;
        };
        let agent = airband_radio::net::agent(Duration::from_secs(40));
        let auth = format!("Bearer {}", self.keys.pull_token);
        let base = format!("{}/v1/channels/{}", relay, self.keys.channel);
        let (mut open, mut wait, mut told) = (false, 1, false);
        let nap = |wait: &mut u64| {
            std::thread::sleep(Duration::from_secs(*wait));
            *wait = (*wait * 2).min(15);
        };
        while !self.stop.load(Ordering::SeqCst) {
            if !open {
                // who its radio is goes with the opening, so the relay knows from the first moment whose channel it is
                let mut req = agent.put(&base).header("authorization", &auth).header("x-airband-radio-hash", airband_core::relay::token_hash(&self.keys.radio_token));
                if let Some(a) = &self.admission {
                    req = req.header("x-airband-admission", a);
                }
                match req.send_empty().map(|r| r.status().as_u16()) {
                    Ok(201 | 204) => {
                        log(format!("serving \"{}\" through {}", self.name, relay));
                        (open, wait, told) = (true, 1, false);
                    }
                    other => {
                        if !told {
                            log(match other {
                                Ok(401) => format!("{} needs an admission key: airband relay <url> --admission <key>", relay),
                                Ok(403) => format!("{} says the channel for \"{}\" belongs to another computer; pair again", relay, self.name),
                                Ok(code) => format!("{} refused to open a channel ({code})", relay),
                                Err(e) => format!("cannot reach {}: {e}; retrying", relay),
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
                    status @ (403 | 404) => {
                        log(format!("{} no longer knows the channel for \"{}\" ({status}); opening it again", relay, self.name));
                        open = false;
                    }
                    status => {
                        log(format!("{} answered {status} when asked for \"{}\"'s requests; retrying", relay, self.name));
                        nap(&mut wait);
                    }
                },
                Err(e) => {
                    if !told {
                        log(format!("lost {}: {e}; retrying", relay));
                        told = true;
                    }
                    open = false;
                    nap(&mut wait);
                }
            }
        }
    }

    /// Is this the first time a request of this id has come? Each is answered once: a copy of one, sent
    /// again by someone who saw it go by, is not. Ids are remembered for as long as a request counts as fresh.
    fn first_time(&self, id: &str, now: u64) -> bool {
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, t| t.abs_diff(now) <= FRESH_MS * 2);
        seen.insert(id.to_string(), now).is_none()
    }

    /// A request that came straight to the tower: opened, checked and run as one from the relay is, and its
    /// answer sealed. Nothing for one this pairing's key does not open, that is stale, or that was seen before.
    async fn answer_here(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        let call = self.keys.open_call(sealed)?;
        let now = now_ms();
        let (status, body) = if !fresh(call.t, now) {
            self.refused(&call, "his own network", 408, WRONG_CLOCK)
        } else {
            if !self.first_time(&call.id, now) {
                return None;
            }
            self.dispatch(&call, "his own network", true).await
        };
        Some(self.keys.seal_answer(&Answer { id: call.id, status, body }))
    }

    async fn answer(self, request_id: String, sealed: Vec<u8>) {
        let Some(call) = self.keys.open_call(&sealed) else {
            log(format!("dropped a request on \"{}\"'s channel that its key does not open", self.name));
            // one the relay was keeping has to be taken off its hands all the same, or it would be handed over
            // for ever and hold up everything kept behind it
            if request_id.starts_with(airband_core::relay::HELD) {
                self.send_back(&request_id, None).await;
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
                self.dispatch(&call, "the relay, kept while the tower was away", false).await
            } else {
                // Nobody is waiting on this answer. Something he said that came too late is told to his
                // radio, in its feed, under the id it was said with: he was told it would get here.
                if said {
                    self.app.inner.lock().unwrap().settle(&self.name, call.body["id"].as_str().unwrap_or_default(), airband_core::radio::Outcome::Rejected { why: airband_core::radio::Rejected::TooOld });
                }
                self.refused(&call, "the relay, kept while the tower was away", 400, "only something he said, sent in the last week, is taken late")
            }
        } else if !fresh(call.t, now) {
            // answered, not dropped, so a phone with a wrong clock is told why nothing works
            self.refused(&call, "the relay", 408, WRONG_CLOCK)
        } else {
            if !self.first_time(&call.id, now) {
                return log(format!("dropped a replayed request on \"{}\"'s channel", self.name));
            }
            self.dispatch(&call, "the relay", true).await
        };
        let sealed = self.keys.seal_answer(&Answer { id: call.id, status, body });
        self.send_back(&request_id, Some(sealed)).await;
    }

    /// Give the relay the answer to a request (or, with none, take a kept message off its hands). An answer
    /// that does not get there is written down: the radio that asked is still waiting for it.
    async fn send_back(&self, request_id: &str, sealed: Option<Vec<u8>>) {
        let Some(relay) = &self.relay else { return };
        let url = format!("{}/v1/channels/{}/reply/{request_id}", relay, self.keys.channel);
        let auth = format!("Bearer {}", self.keys.pull_token);
        // one agent for every answer on this channel: its connection to the relay is kept, not made again each time
        let agent = self.replies.clone();
        let sent = tokio::task::spawn_blocking(move || {
            let req = agent.post(url).header("authorization", auth);
            match sealed {
                Some(sealed) => req.header("content-type", "application/octet-stream").send(&sealed[..]),
                None => req.send_empty(),
            }
            .map(|r| r.status().as_u16())
            .map_err(|e| e.to_string())
        })
        .await;
        match sent {
            Ok(Ok(204)) => {}
            // the radio gave up waiting before the answer was ready: it asks again, and nothing is lost
            Ok(Ok(410)) => log(format!("an answer for \"{}\" came after its radio stopped waiting (request {request_id})", self.name)),
            Ok(Ok(status)) => log(format!("{} answered {status} to an answer for \"{}\" (request {request_id})", relay, self.name)),
            Ok(Err(e)) => log(format!("an answer for \"{}\" did not reach {} (request {request_id}): {e}", self.name, relay)),
            Err(_) => {}
        }
    }

    /// A request the tower will not run, and the line that says so: which pairing, how it came, and why.
    fn refused(&self, call: &Call, via: &str, status: u16, why: &str) -> (u16, Value) {
        log(format!("refused a request from \"{}\" by {via} ({} {}, {status}): {why}", self.name, call.method, call.path.split('?').next().unwrap_or_default()));
        (status, serde_json::json!({ "error": why }))
    }

    /// Run a radio's request through the same routes the local port serves. Only the radio's routes: a paired
    /// phone cannot post agent events or read the status page.
    /// `now`: the request was made just now, so where its radio said it stands is where it stands. One the relay
    /// kept says where the radio stood when he said it, which is not news.
    async fn dispatch(&self, call: &Call, via: &str, now: bool) -> (u16, Value) {
        if call.schema != airband_core::radio::SCHEMA {
            return self.refused(call, via, 400, &format!("this version of airband takes {}, not \"{}\"", airband_core::radio::SCHEMA, call.schema));
        }
        if !call.path.starts_with("/v1/radio/") || !matches!(call.method.as_str(), "GET" | "POST") {
            return self.refused(call, via, 403, "a radio may only use /v1/radio/");
        }
        let body = if call.body.is_null() { Body::empty() } else { Body::from(serde_json::to_vec(&call.body).unwrap()) };
        let mut req = Request::builder().method(call.method.as_str()).uri(&call.path).header("authorization", format!("Bearer {}", self.token)).header(RADIO_HEADER, &self.name).header(VIA_HEADER, via).header("content-type", "application/json");
        if now {
            req = req.header(PRESENCE_HEADER, serde_json::to_string(&call.presence).unwrap());
        }
        let Ok(req) = req.body(body) else {
            return (400, Value::Null);
        };
        let Ok(res) = self.router.clone().oneshot(req).await;
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap_or_default();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}
