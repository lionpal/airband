//! The tower's side of `airband.relay/1`: for each paired radio, hold a channel open on the relay, pull that
//! radio's sealed requests, run them exactly as if they had arrived on the local port, and seal the answers.
//!
//! The tower only ever calls out, so it works behind any router with no port opened. The relay sees sealed
//! bytes; a request that does not open with a paired radio's key, is not fresh, or was already answered is
//! dropped here.
#![expect(clippy::disallowed_methods, reason = "the tower's own connection to the relay")]
use airband_core::radio::wire::Refusal;
use crate::config::PairedRadio;
use super::{App, PRESENCE_HEADER, VIA_HEADER, log, now_ms};
use axum::extract::State;
use airband_core::relay::{ADMISSION_HEADER, GOT_HEADER, PULL_HEADER, PULL_VERSION, RADIO_HASH_HEADER, REQUEST_HEADER};
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
use airband_core::relay::{Answer, Call, FRESH_MS, Keys, Route, fresh};


/// Why a request too far from this computer's clock is refused, said from the limit itself.
fn wrong_clock() -> String {
    format!("the clocks of your phone and your computer are more than {} minutes apart", FRESH_MS / 60_000)
}

/// Every pairing being served, by its channel: a request that comes straight to the tower is answered by the
/// same code, with the same key, as one pulled from the relay. One for each tower, on its [`App`].
#[derive(Default)]
pub(crate) struct Served(Mutex<HashMap<String, Serve>>);

/// The one route a radio uses to reach the tower without the relay: the sealed request it would have given the
/// relay, answered with the sealed answer. Whoever sends it needs the pairing key to make a request the tower
/// will open, and to read what comes back; nothing else is served here.
pub(crate) fn sealed_router(app: Arc<App>) -> Router {
    Router::new().route("/v1/sealed/{channel}", axum::routing::post(sealed)).with_state(app)
}

async fn sealed(State(app): State<Arc<App>>, axum::extract::Path(channel): axum::extract::Path<String>, body: axum::body::Bytes) -> axum::response::Response {
    use axum::response::IntoResponse;
    let serve = app.served.0.lock().unwrap().get(&channel).cloned();
    let Some(serve) = serve else { return axum::http::StatusCode::NOT_FOUND.into_response() };
    match serve.answer_here(&body).await {
        Some(sealed) => sealed.into_response(),
        None => axum::http::StatusCode::BAD_REQUEST.into_response(),
    }
}

/// Watch the config: a pairing added by `airband pair` starts being served, one removed by `airband unpair`
/// stops, and a changed relay address moves everything, all without restarting the tower.
pub(crate) fn start(app: Arc<App>, router: Router, handle: Handle) {
    std::thread::spawn(move || {
        // each link that is up, by its relay and its radio, with the way to stop it
        type Running = HashMap<(Option<String>, PairedRadio), (Arc<AtomicBool>, std::thread::JoinHandle<()>)>;
        let mut running: Running = HashMap::new();
        loop {
            // the settings now, with their keys: ones read without them never take the place of these
            let cfg = app.cfg();
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
                    if let Some(channel) = super::push::channel_of(&key.1) {
                        app.served.0.lock().unwrap().remove(&channel);
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
                let serve = Serve { app: app.clone(), relay: key.0.clone(), name: key.1.name.clone(), keys, router: router.clone(), handle: handle.clone(), stop: stop.clone(), seen: Arc::new(Mutex::new(HashMap::new())), replies: airband_io::http::agent(Duration::from_secs(15)) };
                app.served.0.lock().unwrap().insert(serve.keys.channel.clone(), serve.clone());
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
    router: Router,
    handle: Handle,
    stop: Arc<AtomicBool>,
    /// Ids of calls already answered, with when: a recorded request played back is dropped.
    seen: Arc<Mutex<HashMap<String, u64>>>,
    /// The HTTP agent answers go back through.
    replies: ureq::Agent,
}

impl Serve {
    /// Whether the settings now still hold this pairing, read again from the file when it has changed.
    fn still_paired(&self) -> bool {
        self.app.cfg().pairings.iter().any(|p| super::push::channel_of(p).as_ref() == Some(&self.keys.channel))
    }

    fn run(self) {
        // with no relay there is nothing to pull: requests come straight (`answer_here`) until this is stopped
        let Some(relay) = self.relay.clone() else {
            log(format!("serving \"{}\" at this computer's own addresses, with no relay", self.name));
            while !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
            }
            return;
        };
        let agent = airband_io::http::agent(Duration::from_secs(40));
        let auth = self.keys.as_tower();
        let (channel, pull) = (self.keys.to(&relay, Route::Channel), self.keys.to(&relay, Route::Pull));
        let (mut open, mut wait, mut told) = (false, 1, false);
        // the request the last pull handed over and this tower read: the next pull names it, so the relay knows it
        // arrived (`GOT_HEADER`); one it is not told of it hands over again
        let mut got: Option<String> = None;
        let nap = |wait: &mut u64| {
            std::thread::sleep(Duration::from_secs(*wait));
            *wait = (*wait * 2).min(15);
        };
        while !self.stop.load(Ordering::SeqCst) {
            if !open {
                // a pairing undone since this started is never opened again: `airband unpair` has just closed
                // its channel, and opening it would put it back on record for a phone that was cut off
                if !self.still_paired() {
                    log(format!("\"{}\" is no longer paired; its channel is not opened again", self.name));
                    return;
                }
                // who its radio is goes with the opening, so the relay knows from the first moment whose channel it is
                let mut req = agent.put(&channel).header("authorization", &auth).header(RADIO_HASH_HEADER, airband_core::relay::token_hash(&self.keys.radio_token));
                // the admission key as the settings hold it now: a new one is used without a restart
                if let Some(a) = &self.app.cfg().relay_admission {
                    req = req.header(ADMISSION_HEADER, a);
                }
                match req.send_empty().map(|r| r.status().as_u16()) {
                    Ok(201 | 204) => {
                        log(format!("serving \"{}\" through {}", self.name, relay));
                        (open, wait, told, got) = (true, 1, false, None);
                    }
                    // the relay keeps this channel closed for good: it was unpaired, and only a new pairing opens one
                    Ok(410) => {
                        log(format!("{} keeps the channel for \"{}\" closed; pair again to use it", relay, self.name));
                        return;
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
            let mut ask = agent.get(&pull).header("authorization", &auth).header(PULL_HEADER, PULL_VERSION);
            if let Some(id) = got.take() {
                ask = ask.header(GOT_HEADER, id);
            }
            match ask.call() {
                Ok(mut res) => match res.status().as_u16() {
                    200 => {
                        wait = 1;
                        // a request with no id could never be answered: the relay has broken its side of the contract
                        let Some(id) = res.headers().get(REQUEST_HEADER).and_then(|v| v.to_str().ok()).map(String::from) else {
                            log(format!("{relay} handed over a request for \"{}\" with no id; it cannot be answered", self.name));
                            continue;
                        };
                        // the relay says outright when it had been keeping this one; nothing is read from the id
                        let kept = res.headers().contains_key(airband_core::relay::KEPT_HEADER);
                        // read or not, it is answered for from here: the next pull says it was got
                        got = Some(id.clone());
                        let me = self.clone();
                        match res.body_mut().read_to_vec() {
                            // not awaited: `next` is held for 25 s, and other requests must get through meanwhile
                            Ok(sealed) => drop(self.handle.spawn(async move { me.answer(id, kept, sealed).await })),
                            // one that cannot be read is answered with nothing, once: its radio stops waiting, and
                            // one the relay was keeping is taken off its hands instead of handed over for ever
                            Err(e) => {
                                log(format!("a request for \"{}\" could not be read ({e}); it is let go", self.name));
                                drop(self.handle.spawn(async move { me.send_back(&id, kept, None).await }));
                            }
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
            self.refused(&call, "his own network", Refusal::told(wrong_clock()))
        } else {
            if !self.first_time(&call.id, now) {
                return None;
            }
            self.dispatch(&call, "his own network", true).await
        };
        Some(self.keys.seal_answer(&Answer { id: call.id, status, body }))
    }

    async fn answer(self, request_id: String, kept: bool, sealed: Vec<u8>) {
        let Some(call) = self.keys.open_call(&sealed) else {
            log(format!("dropped a request on \"{}\"'s channel that its key does not open", self.name));
            // one the relay was keeping has to be taken off its hands all the same, or it would be handed over
            // for ever and hold up everything kept behind it
            if kept {
                self.send_back(&request_id, kept, None).await;
            }
            return;
        };
        let now = now_ms();
        // A message the relay kept while this tower was away is old on purpose, and may be handed over twice.
        // Only something he said is taken that way, and only with an id: the tower takes each id once, for good,
        // so neither its age nor a second copy can make it happen again.
        let (status, body) = if kept {
            // something he said, read as what it is: its id, as the tower takes it
            let said: Option<String> = (call.method == airband_core::radio::wire::Method::Post && call.path == <airband_core::radio::wire::at::Utterance as airband_core::radio::wire::Endpoint>::PATH).then(|| serde_json::from_value::<airband_core::radio::Utterance>(call.body.clone()).ok()).flatten().map(|u| u.id).filter(|id| !id.is_empty());
            if said.is_some() && now.saturating_sub(call.t) <= airband_core::relay::HELD_FOR_MS {
                log(format!("\"{}\" said this while the tower was away", self.name));
                self.dispatch(&call, "the relay, kept while the tower was away", false).await
            } else {
                // Nobody is waiting on this answer. Something he said that came too late is told to his
                // radio, in its feed, under the id it was said with: he was told it would get here.
                if let Some(id) = &said {
                    self.app.change().settle(&self.keys.channel, id, airband_core::radio::Outcome::Rejected { why: airband_core::radio::Rejected::TooOld });
                }
                self.refused(&call, "the relay, kept while the tower was away", Refusal::BadAsk { reason: "only something he said, sent in the last week, is taken late".into() })
            }
        } else if !fresh(call.t, now) {
            // answered, not dropped, so a phone with a wrong clock is told why nothing works
            self.refused(&call, "the relay", Refusal::told(wrong_clock()))
        } else {
            if !self.first_time(&call.id, now) {
                // its first copy is being answered; an answer to this one would reach the radio in its place
                return log(format!("dropped a replayed request on \"{}\"'s channel", self.name));
            }
            self.dispatch(&call, "the relay", true).await
        };
        let sealed = self.keys.seal_answer_for_relay(&Answer { id: call.id, status, body });
        self.send_back(&request_id, kept, Some(sealed)).await;
    }

    /// Give the relay the answer to a request (or, with none, take a kept message off its hands). An answer
    /// that does not get there is written down: the radio that asked is still waiting for it. `kept`: the
    /// request was one the relay kept while the tower was away, said back to it as such.
    async fn send_back(&self, request_id: &str, kept: bool, sealed: Option<Vec<u8>>) {
        let Some(relay) = &self.relay else { return };
        let url = self.keys.to(relay, Route::Reply(request_id));
        let auth = self.keys.as_tower();
        // one agent for every answer on this channel: its connection to the relay is kept, not made again each time
        let agent = self.replies.clone();
        let sent = tokio::task::spawn_blocking(move || {
            let req = agent.post(url).header("authorization", auth);
            let req = if kept { req.header(airband_core::relay::KEPT_HEADER, "1") } else { req };
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
    fn refused(&self, call: &Call, via: &str, no: Refusal) -> (u16, Value) {
        log(format!("refused a request from \"{}\" by {via} ({} {}, {}): {}", self.name, call.method, call.path.split('?').next().unwrap_or_default(), no.status(), no.reason().unwrap_or("no reason given")));
        (no.status(), serde_json::to_value(&no).unwrap_or_default())
    }

    /// Run a radio's request through the same routes the local port serves. Only the radio's routes: a paired
    /// phone cannot post agent events or read the status page.
    /// `now`: the request was made just now, so where its radio said it stands is where it stands. One the relay
    /// kept says where the radio stood when he said it, which is not news.
    async fn dispatch(&self, call: &Call, via: &str, now: bool) -> (u16, Value) {
        if !call.path.starts_with(airband_core::radio::PATH) || !matches!(call.method, airband_core::radio::wire::Method::Get | airband_core::radio::wire::Method::Post) {
            return self.refused(call, via, Refusal::NotAllowed);
        }
        let body = if call.body.is_null() { Body::empty() } else { Body::from(serde_json::to_vec(&call.body).unwrap()) };
        let mut req = Request::builder().method(call.method.as_str()).uri(&call.path).header("authorization", format!("Bearer {}", self.app.cfg().radio_token)).header(VIA_HEADER, via).header(airband_core::radio::SCHEMA_HEADER, &call.schema).extension(super::http::RadioId(self.keys.channel.clone())).header("content-type", "application/json");
        if now {
            req = req.header(PRESENCE_HEADER, serde_json::to_string(&call.presence).unwrap());
        }
        let Ok(req) = req.body(body) else {
            return self.refused(call, via, Refusal::BadAsk { reason: "that request cannot be made into one this computer can run".into() });
        };
        let Ok(res) = self.router.clone().oneshot(req).await;
        let status = res.status().as_u16();
        // the whole answer: one too big for the relay is told so when it is sealed for the relay
        // (`seal_answer_for_relay`), and one given straight has no such bound
        let Ok(bytes) = axum::body::to_bytes(res.into_body(), usize::MAX).await else {
            return self.refused(call, via, Refusal::NotNow { reason: "airband could not read its own answer.".into() });
        };
        match serde_json::from_slice(&bytes) {
            Ok(body) => (status, body),
            Err(_) if bytes.is_empty() => (status, Value::Null),
            // its own fault, and asking again gets the same: not something to wait out
            Err(e) => self.refused(call, via, Refusal::told(format!("airband made an answer that is not JSON ({e})."))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pairing_undone_is_never_opened_on_the_relay_again() {
        let app = crate::tower::http::tests::app("unpaired-link");
        let phone = crate::config::PairedRadio { name: "phone".into(), secret: "11".repeat(32) };
        app.settings.change(|c| c.pairings.push(phone.clone())).unwrap();
        let keys = Keys::derive(&phone.secret).unwrap();
        // a relay nothing listens on: were it asked, the link would wait on it, not end
        let serve = Serve { app: app.clone(), relay: Some("http://127.0.0.1:9".into()), name: phone.name.clone(), keys, router: crate::tower::http::router(app.clone()), handle: tokio::runtime::Handle::current(), stop: Arc::new(AtomicBool::new(false)), seen: Default::default(), replies: airband_io::http::agent(Duration::from_secs(1)) };
        assert!(serve.still_paired());
        app.settings.change(|c| c.pairings.clear()).unwrap();
        assert!(!serve.still_paired());
        let ended = tokio::task::spawn_blocking(move || serve.run());
        tokio::time::timeout(Duration::from_secs(5), ended).await.expect("it ends without opening the channel").unwrap();
    }
}
