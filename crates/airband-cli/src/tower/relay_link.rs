//! The tower's side of `airband.relay/1`: for each paired radio, hold a channel open on the relay, pull that
//! radio's sealed requests, run them exactly as if they had arrived on the local port, and seal the answers.
//!
//! The tower only ever calls out, so it works behind any router with no port opened. The relay sees sealed
//! bytes; a request that does not open with a paired radio's key, is not fresh, or was already answered is
//! dropped here.
use airband_core::radio::wire::Refusal;
use crate::config::PairedRadio;
use super::{App, PRESENCE_HEADER, VIA_HEADER, log, now_ms};
use axum::extract::State;
use airband_core::relay::{ADMISSION_HEADER, Pulling, PullRefused, RADIO_HASH_HEADER, REQUEST_HEADER, Side};
use axum::Router;
use axum::body::Body;
use airband_io::http::Ask;
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

/// What the relay's HTTP status says, read in one place for every call the tower and `airband unpair` make
/// to it. What each means for a route is said where that route is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelaySaid {
    /// 200, 201: done, or (pulling) a request handed over.
    Done,
    /// 204: done with nothing to say, or (pulling) nothing waiting.
    Nothing,
    /// 401: an admission key is needed.
    NeedsAdmission,
    /// 403: the channel belongs to another computer.
    NotYours,
    /// 404: the relay does not know the channel.
    Unknown,
    /// 409: (pulling) the relay will not take this pull; its body says why ([`PullRefused`]).
    Refused,
    /// 410: closed for good (opening), or the radio stopped waiting (answering).
    Closed,
    /// 426: (pulling) the relay and this tower speak no version of the pull in common; its body says which is
    /// the older ([`PullRefused::OtherVersion`]).
    OtherVersion,
    /// Anything else, kept as the number for the log.
    Other(u16),
}

impl RelaySaid {
    pub(crate) fn of(status: u16) -> RelaySaid {
        match status {
            200 | 201 => RelaySaid::Done,
            204 => RelaySaid::Nothing,
            401 => RelaySaid::NeedsAdmission,
            403 => RelaySaid::NotYours,
            404 => RelaySaid::Unknown,
            409 => RelaySaid::Refused,
            410 => RelaySaid::Closed,
            426 => RelaySaid::OtherVersion,
            other => RelaySaid::Other(other),
        }
    }
}

/// Tell the relay a channel is finished with (`airband unpair`), so it does not keep it on record for ever.
pub(crate) fn close_channel(relay: &str, keys: &Keys) -> Result<RelaySaid, String> {
    airband_io::http::Client::new(Duration::from_secs(8)).send(Ask::delete(keys.to(relay, Route::Channel)).header("authorization", keys.as_tower())).map(|r| RelaySaid::of(r.status().as_u16())).map_err(|e| e.to_string())
}

/// Why a link's serving thread ended. One the relay keeps closed, or a pairing undone, is not started again
/// while the same pairing is wanted: only a new pairing (a new secret, so a new channel) opens one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// Told to stop: the pairing or the relay address changed.
    Stopped,
    /// The relay keeps this channel closed for good.
    Closed,
    /// The settings no longer hold this pairing.
    Unpaired,
}

/// Every pairing being served, by its channel: a request that comes straight to the tower is answered by the
/// same code, with the same key, as one pulled from the relay. One for each tower, on its [`App`].
#[derive(Default)]
pub(crate) struct Served(Mutex<HashMap<String, Serve>>);

/// The one route a radio uses to reach the tower without the relay: the sealed request it would have given the
/// relay, answered with the sealed answer. Whoever sends it needs the pairing key to make a request the tower
/// will open, and to read what comes back; nothing else is served here.
pub(crate) fn sealed_router(app: Arc<App>) -> Router {
    Router::new().route(airband_core::relay::SEALED, axum::routing::post(sealed)).with_state(app)
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
        let mut links = Links::default();
        loop {
            links.watch(&app, &router, &handle);
            std::thread::sleep(Duration::from_secs(10));
        }
    });
}

/// A link: its relay and its pairing.
type Link = (Option<String>, PairedRadio);

/// The links the watcher keeps: each that is up, with the way to stop it, and those that ended for good, never
/// opened again while that pairing stands.
#[derive(Default)]
struct Links {
    running: HashMap<Link, (Arc<AtomicBool>, std::thread::JoinHandle<Ended>)>,
    finished: std::collections::HashSet<Link>,
}

impl Links {
    /// One look at the settings: start what is wanted, stop what is not.
    fn watch(&mut self, app: &Arc<App>, router: &Router, handle: &Handle) {
        let Links { running, finished } = self;
        // the settings now, with their keys: ones read without them never take the place of these
        let cfg = app.cfg();
        // every pairing is served straight, at the tower's own addresses; through the relay too when there is one
        let wanted: Vec<Link> = cfg.pairings.iter().map(|p| (cfg.relay.clone(), p.clone())).collect();
        finished.retain(|key| wanted.contains(key));
        let ended: Vec<Link> = running.iter().filter(|(key, (_, serving))| wanted.contains(key) && serving.is_finished()).map(|(key, _)| key.clone()).collect();
        for key in ended {
            let Some((_, serving)) = running.remove(&key) else { continue };
            // A pairing whose serving thread has ended while it is still wanted is started again below (with
            // nobody pulling, the relay would call the computer away while it is on), unless it ended for good:
            // the relay keeps the channel closed, or the pairing is gone.
            match serving.join() {
                // ended for good: its straight route goes with it, so a phone no longer paired, or whose
                // channel the relay keeps closed, is not answered on the local network either
                Ok(Ended::Closed | Ended::Unpaired) => {
                    if let Some(channel) = super::push::channel_of(&key.1) {
                        app.served.0.lock().unwrap().remove(&channel);
                    }
                    finished.insert(key);
                }
                Ok(Ended::Stopped) | Err(_) => log(format!("the link serving \"{}\" stopped; starting it again", key.1.name)),
            }
        }
        running.retain(|key, (stop, _)| {
            let keep = wanted.contains(key);
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
            if running.contains_key(&key) || finished.contains(&key) {
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
            let serve = Serve { app: app.clone(), relay: key.0.clone(), name: key.1.name.clone(), keys, router: router.clone(), handle: handle.clone(), stop: stop.clone(), seen: Arc::new(Mutex::new(HashMap::new())), replies: airband_io::http::Client::new(Duration::from_secs(15)), pulling: Default::default() };
            app.served.0.lock().unwrap().insert(serve.keys.channel.clone(), serve.clone());
            running.insert(key, (stop, std::thread::spawn(move || serve.run())));
        }
    }
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
    replies: airband_io::http::Client,
    /// What the next pull says (`airband_core::relay::Pulling`): what the last one got, and the kept message
    /// being answered for. An answer to a kept message that does not reach the relay is not sent again from
    /// here: the work on it ends, the next pull stops naming it, and the relay, which still keeps it, hands it
    /// over again.
    pulling: Arc<Mutex<Pulling>>,
}

/// The work on one request handed over by a pull, for as long as it lasts: however it ends, its answer sent or
/// not, a panic included, the pulls stop saying the tower is answering for it.
struct Answering {
    pulling: Arc<Mutex<Pulling>>,
    id: String,
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.pulling.lock().unwrap_or_else(|e| e.into_inner()).answered(&self.id);
    }
}

/// What he is told when the relay will not take a pull, said once for each reason.
fn refused_pull(relay: &str, why: PullRefused) -> String {
    match why {
        PullRefused::OtherVersion { older: Side::Tower } => format!("{relay} needs a newer airband than this computer has: update airband here"),
        PullRefused::OtherVersion { older: Side::Relay } => format!("this computer's airband is newer than {relay}: update the relay"),
        PullRefused::AnotherTower => format!("another computer is serving this pairing through {relay}; stop airband there, or pair this computer again on its own"),
        PullRefused::Unknown => format!("{relay} refused this computer's pull for a reason this airband does not know: update airband here"),
    }
}

impl Serve {
    /// Whether the settings now still hold this pairing, read again from the file when it has changed.
    fn still_paired(&self) -> bool {
        self.app.cfg().pairings.iter().any(|p| super::push::channel_of(p).as_ref() == Some(&self.keys.channel))
    }

    fn run(self) -> Ended {
        // with no relay there is nothing to pull: requests come straight (`answer_here`) until this is stopped
        let Some(relay) = self.relay.clone() else {
            log(format!("serving \"{}\" at this computer's own addresses, with no relay", self.name));
            while !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
            }
            return Ended::Stopped;
        };
        let agent = airband_io::http::Client::new(Duration::from_secs(40));
        let auth = self.keys.as_tower();
        let (channel, pull) = (self.keys.to(&relay, Route::Channel), self.keys.to(&relay, Route::Pull));
        let (mut open, mut wait, mut told) = (false, 1, false);
        // each reason a pull was refused, once told: he is not told the same thing every few seconds
        let mut told_refused: Vec<PullRefused> = Vec::new();
        let nap = |wait: &mut u64| {
            std::thread::sleep(Duration::from_secs(*wait));
            *wait = (*wait * 2).min(15);
        };
        while !self.stop.load(Ordering::SeqCst) {
            // a pairing undone since this started is never opened again, and nothing more it pulls is answered:
            // `airband unpair` has just closed its channel, and opening it would put it back on record for a
            // phone that was cut off
            if !self.still_paired() {
                log(format!("\"{}\" is no longer paired; its channel is not opened again", self.name));
                return Ended::Unpaired;
            }
            if !open {
                // who its radio is goes with the opening, so the relay knows from the first moment whose channel it is
                let mut req = Ask::put(&channel).header("authorization", &auth).header(RADIO_HASH_HEADER, airband_core::relay::token_hash(&self.keys.radio_token));
                // the admission key as the settings hold it now: a new one is used without a restart
                if let Some(a) = &self.app.cfg().relay_admission {
                    req = req.header(ADMISSION_HEADER, a);
                }
                match agent.send(req).map(|r| RelaySaid::of(r.status().as_u16())) {
                    Ok(RelaySaid::Done | RelaySaid::Nothing) => {
                        log(format!("serving \"{}\" through {}", self.name, relay));
                        // opened again, the relay hands again whatever it had handed: nothing is in this tower's hands
                        self.pulling.lock().unwrap().reopened();
                        (open, wait, told) = (true, 1, false);
                    }
                    // the relay keeps this channel closed for good: it was unpaired, and only a new pairing opens one
                    Ok(RelaySaid::Closed) => {
                        log(format!("{} keeps the channel for \"{}\" closed; pair again to use it", relay, self.name));
                        return Ended::Closed;
                    }
                    other => {
                        if !told {
                            log(match other {
                                Ok(RelaySaid::NeedsAdmission) => format!("{} needs an admission key: airband relay <url> --admission <key>", relay),
                                Ok(RelaySaid::NotYours) => format!("{} says the channel for \"{}\" belongs to another computer; pair again", relay, self.name),
                                Ok(said) => format!("{} refused to open a channel ({said:?})", relay),
                                Err(e) => format!("cannot reach {}: {e}; retrying", relay),
                            });
                            told = true;
                        }
                        nap(&mut wait);
                    }
                }
                continue;
            }
            let said = self.pulling.lock().unwrap().headers();
            match agent.send(Ask::get(&pull).header("authorization", &auth).headers(said)) {
                Ok(mut res) => match RelaySaid::of(res.status().as_u16()) {
                    RelaySaid::Done => {
                        // the relay has taken this pull, and so heard what it got
                        self.pulling.lock().unwrap().taken();
                        wait = 1;
                        // a request with no id could never be answered: the relay has broken its side of the contract
                        let Some(id) = res.header(REQUEST_HEADER).map(String::from) else {
                            log(format!("{relay} handed over a request for \"{}\" with no id; it cannot be answered", self.name));
                            continue;
                        };
                        // the relay says outright when it had been keeping this one; nothing is read from the id
                        let kept = res.has_header(airband_core::relay::KEPT_HEADER);
                        match res.body_mut().read_to_vec() {
                            // read whole: the next pull says it was got, and it is answered for from here
                            Ok(sealed) => {
                                self.pulling.lock().unwrap().handed(&id, kept);
                                self.answer_apart(id, kept, sealed);
                            }
                            // one cut off on the way is not named as got, so the relay hands it over again
                            Err(e) => log(format!("a request for \"{}\" was cut off on the way ({e}); the relay hands it over again", self.name)),
                        }
                    }
                    RelaySaid::Nothing => {
                        self.pulling.lock().unwrap().taken();
                        wait = 1;
                    }
                    // the relay restarted without its data, or the channel changed hands: open it again
                    said @ (RelaySaid::NotYours | RelaySaid::Unknown) => {
                        log(format!("{} no longer knows the channel for \"{}\" ({said:?}); opening it again", relay, self.name));
                        open = false;
                    }
                    // refused, saying why: which side is the older, or that another computer pulls with this pairing.
                    // A refused pull told the relay nothing, so what it got is said again on the next
                    RelaySaid::OtherVersion | RelaySaid::Refused => {
                        let why = res.body_mut().read_json::<PullRefused>().unwrap_or(PullRefused::Unknown);
                        if !told_refused.contains(&why) {
                            log(refused_pull(&relay, why));
                            told_refused.push(why);
                        }
                        nap(&mut wait);
                    }
                    said => {
                        log(format!("{} answered {said:?} when asked for \"{}\"'s requests; retrying", relay, self.name));
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
        Ended::Stopped
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

    /// Answer a request on a task of its own (`next` is held for 25 s, and other requests must get through
    /// meanwhile). However the task ends, a panic included, its guard tells the pulls it is no longer answered for.
    fn answer_apart(&self, request_id: String, kept: bool, sealed: Vec<u8>) {
        let guard = Answering { pulling: self.pulling.clone(), id: request_id.clone() };
        let me = self.clone();
        drop(self.handle.spawn(async move {
            let _answering = guard;
            me.answer(request_id, kept, sealed).await;
        }));
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
    /// request was one the relay kept while the tower was away, said back to it as such. One of those whose
    /// answer did not get there is handed over again by the relay once the work on it ends ([`Answering`]).
    /// Whether the relay has it.
    async fn send_back(&self, request_id: &str, kept: bool, sealed: Option<Vec<u8>>) -> bool {
        let Some(relay) = &self.relay else { return false };
        let url = self.keys.to(relay, Route::Reply(request_id));
        let auth = self.keys.as_tower();
        // one agent for every answer on this channel: its connection to the relay is kept, not made again each time
        let agent = self.replies.clone();
        let sent = tokio::task::spawn_blocking(move || {
            let req = Ask::post(url).header("authorization", auth);
            let req = if kept { req.header(airband_core::relay::KEPT_HEADER, "1") } else { req };
            let req = match sealed {
                Some(sealed) => req.bytes("application/octet-stream", sealed),
                None => req,
            };
            agent
                .send(req)
                .map(|r| RelaySaid::of(r.status().as_u16()))
            .map_err(|e| e.to_string())
        })
        .await;
        match sent {
            Ok(Ok(RelaySaid::Nothing | RelaySaid::Done)) => true,
            // the radio gave up waiting before the answer was ready: it asks again, and nothing is lost
            Ok(Ok(RelaySaid::Closed)) => {
                log(format!("an answer for \"{}\" came after its radio stopped waiting (request {request_id})", self.name));
                true
            }
            Ok(Ok(said)) => {
                log(format!("{} answered {said:?} to an answer for \"{}\" (request {request_id})", relay, self.name));
                false
            }
            Ok(Err(e)) => {
                log(format!("an answer for \"{}\" did not reach {} (request {request_id}): {e}", self.name, relay));
                false
            }
            Err(_) => {
                log(format!("an answer for \"{}\" was not sent: its sending failed (request {request_id})", self.name));
                false
            }
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
        let serve = Serve { app: app.clone(), relay: Some("http://127.0.0.1:9".into()), name: phone.name.clone(), keys, router: crate::tower::http::router(app.clone()), handle: tokio::runtime::Handle::current(), stop: Arc::new(AtomicBool::new(false)), seen: Default::default(), replies: airband_io::http::Client::new(Duration::from_secs(1)), pulling: Default::default() };
        assert!(serve.still_paired());
        app.settings.change(|c| c.pairings.clear()).unwrap();
        assert!(!serve.still_paired());
        let ended = tokio::task::spawn_blocking(move || serve.run());
        let ended = tokio::time::timeout(Duration::from_secs(5), ended).await.expect("it ends without opening the channel").unwrap();
        assert_eq!(ended, Ended::Unpaired);
    }

    /// A relay that keeps the channel closed answers every open with 410: the link ends for good, and the
    /// watcher never opens it again for the same pairing (the relay would count each open against the address).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_channel_the_relay_keeps_closed_is_opened_once_and_never_again() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = opens.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _read = stream.read(&mut buf);
                counted.fetch_add(1, Ordering::SeqCst);
                stream.write_all(b"HTTP/1.1 410 Gone\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").ok();
            }
        });
        let app = crate::tower::http::tests::app("closed-link");
        let phone = crate::config::PairedRadio { name: "phone".into(), secret: "22".repeat(32) };
        app.settings.change(|c| {
            c.pairings.push(phone.clone());
            c.relay = Some(relay.clone());
        })
        .unwrap();
        let keys = Keys::derive(&phone.secret).unwrap();
        let serve = Serve { app: app.clone(), relay: Some(relay), name: phone.name.clone(), keys, router: crate::tower::http::router(app.clone()), handle: tokio::runtime::Handle::current(), stop: Arc::new(AtomicBool::new(false)), seen: Default::default(), replies: airband_io::http::Client::new(Duration::from_secs(1)), pulling: Default::default() };
        let ended = tokio::time::timeout(Duration::from_secs(5), tokio::task::spawn_blocking(move || serve.run())).await.unwrap().unwrap();
        assert_eq!(ended, Ended::Closed);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        // the watcher, with the same pairing wanted, opens it once of its own and never again
        let (router, handle) = (crate::tower::http::router(app.clone()), tokio::runtime::Handle::current());
        let mut links = Links::default();
        links.watch(&app, &router, &handle);
        while links.running.values().any(|(_, t)| !t.is_finished()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        links.watch(&app, &router, &handle);
        links.watch(&app, &router, &handle);
        assert!(links.running.is_empty());
        assert_eq!(opens.load(Ordering::SeqCst), 2, "opened once by the watcher, and never again while the same pairing stands");
        // and its straight route is gone with it: a phone the relay keeps closed is not answered on the network
        assert!(app.served.0.lock().unwrap().is_empty());
    }

    /// A relay that answers each request with what `answer` says for it (its request line and headers, and how
    /// many came before it), and keeps every request it was sent.
    fn fake_relay(answer: impl Fn(&str, usize) -> Vec<u8> + Send + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        let asked: Arc<Mutex<Vec<String>>> = Default::default();
        let kept = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = vec![0u8; 65536];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let before = { let mut k = kept.lock().unwrap(); k.push(head.clone()); k.len() - 1 };
                stream.write_all(&answer(&head, before)).ok();
            }
        });
        (relay, asked)
    }

    fn serving(app: &Arc<App>, relay: &str, secret: &str) -> Serve {
        let phone = crate::config::PairedRadio { name: "phone".into(), secret: secret.into() };
        app.settings.change(|c| {
            c.pairings.push(phone.clone());
            c.relay = Some(relay.to_string());
        })
        .unwrap();
        Serve { app: app.clone(), relay: Some(relay.into()), name: phone.name.clone(), keys: Keys::derive(&phone.secret).unwrap(), router: crate::tower::http::router(app.clone()), handle: tokio::runtime::Handle::current(), stop: Arc::new(AtomicBool::new(false)), seen: Default::default(), replies: airband_io::http::Client::new(Duration::from_secs(2)), pulling: Default::default() }
    }

    const CLOSE: &str = "content-length: 0\r\nconnection: close\r\n\r\n";

    /// What a pull the fake relay was sent said in `header`.
    fn said(asked: &str, header: &str) -> Option<String> {
        asked.lines().find_map(|l| l.strip_prefix(&format!("{header}: ")).map(|v| v.trim().to_string()))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_answer_to_a_kept_message_that_does_not_reach_the_relay_is_handed_over_again_by_the_relay() {
        use airband_core::relay::{ANSWERING_HEADER, GOT_HEADER, KEPT_HEADER, PULL_HEADER, PULL_VERSION, TOWER_HEADER};
        // open; a kept one handed over; every reply refused (the relay's store is down); then pulls with nothing
        let (relay, asked) = fake_relay(|head, n| match n {
            _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
            _ if head.starts_with("post") => format!("HTTP/1.1 503 Busy\r\n{CLOSE}").into_bytes(),
            1 => b"HTTP/1.1 200 OK\r\nx-airband-request: k1\r\nx-airband-kept: 1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
            _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
        });
        let app = crate::tower::http::tests::app("kept-answer-lost");
        let serve = serving(&app, &relay, &"33".repeat(32));
        let stop = serve.stop.clone();
        let running = tokio::task::spawn_blocking(move || serve.run());
        // until the answer was tried, and a pull after it no longer named it (one sent while the answer was on
        // its way still does)
        let tried = |asked: &[String]| asked.iter().position(|r| r.starts_with("post") && r.contains("/reply/k1"));
        let let_go = |asked: &[String]| tried(asked).and_then(|at| asked[at + 1..].iter().position(|r| r.starts_with("get") && said(r, ANSWERING_HEADER).is_none()).map(|n| at + 1 + n));
        let waited = tokio::time::timeout(Duration::from_secs(10), async {
            while let_go(&asked.lock().unwrap()).is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "the pulls went on naming a kept message whose answer was refused");
        stop.store(true, Ordering::SeqCst);
        running.await.unwrap();
        let asked = asked.lock().unwrap();
        let pulls: Vec<&String> = asked.iter().filter(|r| r.starts_with("get")).collect();
        // every pull says its version and the one tower it is from
        assert!(pulls.iter().all(|p| said(p, PULL_HEADER).as_deref() == Some(PULL_VERSION)));
        assert_eq!(pulls.iter().filter_map(|p| said(p, TOWER_HEADER)).collect::<std::collections::HashSet<_>>().len(), 1);
        // the pull after it says it got it; the answer went back marked kept, and was refused
        assert_eq!(said(pulls[1], GOT_HEADER).as_deref(), Some("k1"));
        let at = tried(&asked).unwrap();
        assert!(asked[at].contains(KEPT_HEADER));
        // while it was being answered the pulls named it, so the relay handed nothing kept meanwhile
        assert_eq!(said(pulls[1], ANSWERING_HEADER).as_deref(), Some("k1"));
        // the work on it ended without the relay having its answer: the pulls stop naming it, and the relay, which
        // still keeps it, hands it over again. The tower sends nothing again of its own
        assert!(let_go(&asked).is_some());
        assert_eq!(asked.iter().filter(|r| r.contains("/reply/k1")).count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_work_on_a_kept_message_that_panics_still_ends_its_answering() {
        let pulling: Arc<Mutex<Pulling>> = Default::default();
        pulling.lock().unwrap().handed("k1", true);
        let answering = |p: &Arc<Mutex<Pulling>>| p.lock().unwrap().headers().into_iter().find(|(k, _)| *k == airband_core::relay::ANSWERING_HEADER).map(|(_, v)| v);
        assert_eq!(answering(&pulling).as_deref(), Some("k1"));
        let guard = Answering { pulling: pulling.clone(), id: "k1".into() };
        let died = tokio::spawn(async move {
            let _answering = guard;
            panic!("the work on it died");
        });
        assert!(died.await.is_err());
        assert_eq!(answering(&pulling), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_pull_says_why_and_told_the_relay_nothing_so_what_was_got_is_said_again() {
        use airband_core::relay::GOT_HEADER;
        let (relay, asked) = fake_relay(|head, n| match n {
            _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
            _ if head.starts_with("post") => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
            1 => b"HTTP/1.1 200 OK\r\nx-airband-request: r1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
            2 => b"HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\ncontent-length: 27\r\nconnection: close\r\n\r\n{\"refused\":\"another_tower\"}".to_vec(),
            _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
        });
        let app = crate::tower::http::tests::app("pull-refused");
        let serve = serving(&app, &relay, &"55".repeat(32));
        let stop = serve.stop.clone();
        let running = tokio::task::spawn_blocking(move || serve.run());
        while asked.lock().unwrap().iter().filter(|r| r.starts_with("get")).count() < 4 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.store(true, Ordering::SeqCst);
        running.await.unwrap();
        let asked = asked.lock().unwrap();
        let pulls: Vec<&String> = asked.iter().filter(|r| r.starts_with("get")).collect();
        assert_eq!(said(pulls[1], GOT_HEADER).as_deref(), Some("r1"));
        assert_eq!(said(pulls[2], GOT_HEADER).as_deref(), Some("r1"), "the refused pull was not taken: it is said again");
        assert_eq!(said(pulls[3], GOT_HEADER), None, "and once taken, not again");
    }

    #[test]
    fn he_is_told_which_side_to_update_or_that_another_computer_pulls() {
        use airband_core::relay::{PullRefused, Side};
        let read = |body: &str| serde_json::from_str::<PullRefused>(body).unwrap();
        assert_eq!(read(r#"{"refused":"other_version","older":"tower"}"#), PullRefused::OtherVersion { older: Side::Tower });
        assert!(refused_pull("r", read(r#"{"refused":"other_version","older":"tower"}"#)).contains("update airband here"));
        assert!(refused_pull("r", read(r#"{"refused":"other_version","older":"relay"}"#)).contains("update the relay"));
        assert!(refused_pull("r", read(r#"{"refused":"another_tower"}"#)).contains("another computer"));
        assert_eq!(read(r#"{"refused":"tomorrow"}"#), PullRefused::Unknown);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_cut_off_on_the_way_is_never_said_to_be_got_and_one_read_whole_is() {
        // open; a pull cut off mid-body; a pull read whole; then pulls with nothing
        let (relay, asked) = fake_relay(|head, n| match n {
            _ if head.starts_with("put") => format!("HTTP/1.1 200 OK\r\n{CLOSE}").into_bytes(),
            1 => b"HTTP/1.1 200 OK\r\nx-airband-request: cut\r\ncontent-length: 400\r\nconnection: close\r\n\r\nshort".to_vec(),
            2 => b"HTTP/1.1 200 OK\r\nx-airband-request: whole\r\ncontent-length: 5\r\nconnection: close\r\n\r\nwhole".to_vec(),
            _ => format!("HTTP/1.1 204 No Content\r\n{CLOSE}").into_bytes(),
        });
        let app = crate::tower::http::tests::app("got-after-read");
        let serve = serving(&app, &relay, &"44".repeat(32));
        let stop = serve.stop.clone();
        let running = tokio::task::spawn_blocking(move || serve.run());
        while asked.lock().unwrap().len() < 5 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.store(true, Ordering::SeqCst);
        running.await.unwrap();
        let asked = asked.lock().unwrap();
        let got = |n: usize| asked[n].lines().find_map(|l| l.strip_prefix(&format!("{}: ", airband_core::relay::GOT_HEADER)).map(str::to_string));
        assert_eq!(got(2), None, "the cut-off one is handed over again, not said to be got");
        assert_eq!(got(3).as_deref(), Some("whole"));
        assert_eq!(got(4), None, "said once, to the pull after it");
    }

    #[test]
    fn the_relays_statuses_are_read_in_one_place() {
        assert_eq!(RelaySaid::of(201), RelaySaid::Done);
        assert_eq!(RelaySaid::of(204), RelaySaid::Nothing);
        assert_eq!(RelaySaid::of(409), RelaySaid::Refused);
        assert_eq!(RelaySaid::of(410), RelaySaid::Closed);
        assert_eq!(RelaySaid::of(426), RelaySaid::OtherVersion);
        assert_eq!(RelaySaid::of(418), RelaySaid::Other(418));
    }
}
