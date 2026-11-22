//! airband-relay: the meeting point for towers and radios that cannot reach each other directly.
//!
//! It keeps, per channel, the hash of the tower's pull token, the hash of its radio's token and the requests
//! in flight. It holds no keys and sees only sealed bytes (`airband.relay/1`, crates/airband-core/src/relay.rs).
//! Put it behind TLS, on a proxy or load balancer that adds the caller's address to `x-forwarded-for`: the
//! limits per address go by the last entry there, and without one every caller counts as the same address.
//!
//!   PORT                     port to listen on (default 7474)
//!   AIRBAND_RELAY_DATA       folder for channels.json (default ./relay-data), when no database is set
//!   DB_HOST and friends      keep the channel list in Postgres instead (store.rs)
//!   AIRBAND_RELAY_DYNAMO     keep it in Amazon DynamoDB instead: the tables' prefix (store.rs), with AWS_REGION
//!   AIRBAND_RELAY_ADMISSION  if set, a tower must present this to open a channel; without it, anyone may,
//!                            within limits (so many new channels from one address in a day, and in all in an hour)
//!
//! Who may do what. A tower does everything with its channel's pull token, and opening a new channel also
//! takes the admission key. A radio does everything with its channel's radio token. Both tokens are made from
//! the pairing secret, and the relay holds only their hashes. A channel's name alone gets nothing.
//!
//!   PUT  /v1/channels/{c}             tower: open the channel (Bearer pull token; x-airband-radio-hash)
//!   GET  /v1/channels/{c}/pull        tower: the next sealed request, held up to 25 s (204 when none)
//!   POST /v1/channels/{c}/reply/{id}  tower: the sealed answer to request id
//!   POST /v1/channels/{c}/push        tower: a sealed notification for a push service to wake the phone with (push.rs)
//!   POST /v1/channels/{c}/call        radio: a sealed request; the response is the sealed answer (Bearer radio token)
//!   POST /v1/channels/{c}/queue       radio: a sealed message to keep for a tower that is away (x-airband-id names it)
//!   POST /v1/waitlist                 the homepage's sign-up: {"email": "…"} (airband.dev only, by CORS)
//!   GET  /healthz
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, oneshot};
use airband_core::relay::{HELD_FOR_MS, HELD_MOST, MAX_MESSAGE, PUSH_MOST, PushAnswer, PushRequest, random_hex, token_hash};

/// How long a tower's pull is held open.
const HOLD: Duration = Duration::from_secs(25);
/// A tower that has not pulled for this long is offline: radios are told at once instead of waiting.
const AIRBAND_GONE: Duration = Duration::from_secs(45);
/// How long a radio's call waits for the tower's answer. Longer than the 25 s a tower holds `next` open.
const ANSWER_WAIT: Duration = Duration::from_secs(40);
/// Requests in flight per channel; more are refused.
const MAX_IN_FLIGHT: usize = 16;

struct Channel {
    pull_hash: String,
    queue: VecDeque<(String, Bytes)>,
    waiting: HashMap<String, oneshot::Sender<Bytes>>,
    last_pull: Option<Instant>,
    wake: Arc<Notify>,
    /// There may be messages kept for this tower from while it was away. Assumed after a restart, until looked.
    has_held: bool,
    /// The kept message the tower has been handed and has not yet said it took, and when.
    holding: Option<(String, Instant)>,
    /// The hash of the token the paired radio proves itself with, as the tower gave it when it opened the
    /// channel.
    radio_hash: String,
    /// How many notifications it has sent lately, and since when.
    pushed: Option<(Instant, u32)>,
}

impl Channel {
    #[cfg(test)]
    fn new(pull_hash: String, radio_hash: String) -> Channel {
        Channel::from(store::Saved { pull_hash, radio_hash })
    }
    fn from(saved: store::Saved) -> Channel {
        Channel { pull_hash: saved.pull_hash, queue: VecDeque::new(), waiting: HashMap::new(), last_pull: None, wake: Arc::new(Notify::new()), has_held: true, holding: None, radio_hash: saved.radio_hash, pushed: None }
    }
}

/// A kept message the tower was handed and never answered for is handed over again after this long.
const HAND_AGAIN: Duration = Duration::from_secs(60);

mod dynamo;
mod push;
mod store;
mod limits;
mod waitlist;
use limits::*;
use waitlist::*;

/// How many keys (channels or internet addresses) one limit remembers at once.
const KEYS_KEPT: usize = 20_000;

struct Relay {
    /// The push services this relay can wake phones through.
    push: push::Push,
    channels: Mutex<HashMap<String, Channel>>,
    store: store::Store,
    admission: Option<String>,
    /// How many sign-ups the waiting list has taken lately.
    signups: Mutex<Signups>,
    /// At most two sign-ups are being written to the database at any moment.
    writing: tokio::sync::Semaphore,
    /// How much each channel and each internet address has asked for lately.
    limits: Mutex<Limits>,
}

/// The line for a radio's request that failed: see [`Relay::note_failed`].
/// How busy a channel was when a request on it failed: (requests in flight, messages kept for its tower, its
/// tower pulling).
type Load = (usize, bool, bool);

fn failed_line(route: &str, channel: &str, status: u16, took: Duration, bytes: usize, (in_flight, has_held, pulling): Load) -> String {
    // six characters tell one channel's lines from another's in the log and are not enough to use the channel
    let short: String = channel.chars().filter(char::is_ascii_hexdigit).take(6).collect();
    format!("airband-relay: {route} on {short} answered {status} after {} ms ({bytes} bytes sent; {in_flight} in flight; kept messages: {}; tower pulling: {})", took.as_millis(), if has_held { "yes" } else { "no" }, if pulling { "yes" } else { "no" })
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
fn bearer(headers: &HeaderMap) -> &str {
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("")
}
fn valid_channel(c: &str) -> bool {
    c.len() == 32 && c.bytes().all(|b| b.is_ascii_hexdigit())
}
fn say(code: StatusCode, why: &str) -> Response {
    (code, why.to_string()).into_response()
}
/// A token's hash as a tower sends it: 64 hex characters.
fn radio_hash_given(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-airband-radio-hash").and_then(|v| v.to_str().ok()).filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}
/// What a radio's request gets when it may not use the channel. The same whether the token is missing or wrong
/// or there is no such channel, so asking tells a stranger nothing about which channels exist.
fn not_the_radio() -> Response {
    say(StatusCode::UNAUTHORIZED, "not this channel's radio")
}

impl Relay {
    fn new(push: push::Push, channels: HashMap<String, Channel>, store: store::Store, admission: Option<String>) -> Relay {
        Relay { push, channels: Mutex::new(channels), store, admission, signups: Mutex::new(Signups::default()), writing: tokio::sync::Semaphore::new(2), limits: Mutex::new(Limits::new()) }
    }

    /// Is this request from the channel's radio, and within what a radio may ask? Every route a radio uses
    /// starts here.
    ///
    /// The radio shows a token made from the pairing secret (`Authorization: Bearer`), whose hash the channel's
    /// tower gave when it opened the channel. Without that token nothing is taken.
    async fn admit(&self, asks: Asks, channel: &str, headers: &HeaderMap) -> Result<(), Box<Response>> {
        let now = Instant::now();
        if !self.limits.lock().unwrap().address(asks, &caller(headers), now) {
            return Err(Box::new(say(StatusCode::TOO_MANY_REQUESTS, "too many requests from this address; try again in a minute")));
        }
        let shown = token_hash(bearer(headers));
        if !self.channels.lock().unwrap().get(channel).is_some_and(|ch| same(&shown, &ch.radio_hash)) {
            return Err(Box::new(not_the_radio()));
        }
        // counted only now, so a stranger who knows a channel's name cannot use up its radio's share
        if !self.limits.lock().unwrap().channel(asks, channel, now) {
            return Err(Box::new(say(StatusCode::TOO_MANY_REQUESTS, "too many requests on this channel; try again in a minute")));
        }
        Ok(())
    }

    /// A radio's request did not come to what it asked for: one line saying so. It has what the relay itself
    /// handled and nothing else: the route, the status, how long it took, how much was sent, a shortened name
    /// for the channel, how many requests were in flight on it, whether messages are kept for its tower, and
    /// whether its tower has been pulling. Never what was sent (it is sealed, and the relay has no key to it),
    /// and never a token or an internet address.
    fn note_failed(&self, route: &str, channel: &str, status: StatusCode, began: Instant, bytes: usize, wanted: StatusCode) {
        if status == wanted {
            return;
        }
        let (in_flight, has_held, pulling) = self.channels.lock().unwrap().get(channel).map(|ch| (ch.waiting.len(), ch.has_held, ch.last_pull.is_some_and(|t| t.elapsed() <= AIRBAND_GONE))).unwrap_or_default();
        eprintln!("{}", failed_line(route, channel, status.as_u16(), began.elapsed(), bytes, (in_flight, has_held, pulling)));
    }

    /// The channel, if this request carries its tower's pull token.
    fn as_tower<'a>(&self, channels: &'a mut HashMap<String, Channel>, channel: &str, headers: &HeaderMap) -> Result<&'a mut Channel, Box<Response>> {
        let ch = channels.get_mut(channel).ok_or_else(|| Box::new(say(StatusCode::NOT_FOUND, "no such channel: open it first")))?;
        if same(&token_hash(bearer(headers)), &ch.pull_hash) { Ok(ch) } else { Err(Box::new(say(StatusCode::FORBIDDEN, "not this channel's tower"))) }
    }
}

async fn open(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let (true, false, Some(radio)) = (valid_channel(&channel), bearer(&headers).is_empty(), radio_hash_given(&headers)) else {
        return say(StatusCode::BAD_REQUEST, "a channel is 32 hex characters and needs a pull token and its radio's token hash");
    };
    let hash = token_hash(bearer(&headers));
    // opening again is how a tower checks it is still known after a relay restart
    let known = |channels: &HashMap<String, Channel>| channels.get(&channel).map(|existing| if same(&existing.pull_hash, &hash) { StatusCode::NO_CONTENT.into_response() } else { say(StatusCode::FORBIDDEN, "that channel belongs to another tower") });
    let answer = known(&relay.channels.lock().unwrap());
    if let Some(answer) = answer {
        return answer;
    }
    if let Some(want) = &relay.admission {
        let given = headers.get("x-airband-admission").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !same(given, want) {
            return say(StatusCode::UNAUTHORIZED, "this relay needs an admission key to open a channel");
        }
    }
    // a new channel, which anyone may ask for: so many from one address, and so many in all
    {
        let (mut limits, now) = (relay.limits.lock().unwrap(), Instant::now());
        if !limits.opened_by_address.allow(&caller(&headers), now) {
            return say(StatusCode::TOO_MANY_REQUESTS, "too many new pairings from this address today; try again tomorrow");
        }
        if !limits.opened.allow("all", now) {
            return say(StatusCode::TOO_MANY_REQUESTS, "the relay is taking no new pairings for a while; try again in an hour");
        }
    }
    // durable first: a channel the relay would forget on restart is worse than a slow answer
    if let Err(e) = relay.store.add(&channel, &hash, radio).await {
        eprintln!("airband-relay: could not save a new channel: {e}");
        return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not save the channel; try again");
    }
    let mut channels = relay.channels.lock().unwrap();
    // two towers raced for the same name: the first one in keeps it
    if let Some(answer) = known(&channels) {
        return answer;
    }
    channels.insert(channel, Channel::from(store::Saved { pull_hash: hash, radio_hash: radio.to_string() }));
    StatusCode::CREATED.into_response()
}

async fn pull(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let until = Instant::now() + HOLD;
    loop {
        let wake = {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return *r,
            };
            ch.last_pull = Some(Instant::now());
            // a radio that gave up leaves its request behind; skip those
            while let Some((id, sealed)) = ch.queue.pop_front() {
                if ch.waiting.contains_key(&id) {
                    return ([("x-airband-request", id)], sealed).into_response();
                }
            }
            // then what was kept while it was away: one at a time, the oldest first, the next only once this one is taken
            let due = ch.has_held && ch.holding.as_ref().is_none_or(|(_, at)| at.elapsed() > HAND_AGAIN);
            (ch.wake.clone(), due)
        };
        let (wake, due) = wake;
        if due {
            match relay.store.held_next(&channel).await {
                Ok(Some((id, sealed))) => {
                    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                        ch.holding = Some((id.clone(), Instant::now()));
                    }
                    return ([("x-airband-request", id.to_string()), (airband_core::relay::KEPT_HEADER, "1".to_string())], sealed).into_response();
                }
                Ok(None) => {
                    if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                        ch.has_held = false;
                    }
                }
                Err(e) => eprintln!("airband-relay: cannot read what is kept for a channel: {e}"),
            }
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return StatusCode::NO_CONTENT.into_response();
        }
        // wake at least every few seconds so last_pull stays current while the tower waits
        let _ = tokio::time::timeout(left.min(Duration::from_secs(5)), wake.notified()).await;
    }
}

/// A tower closes its channel: the radio it was paired with has been cut off. Without this every channel ever
/// opened stayed on record.
async fn close(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    {
        let mut channels = relay.channels.lock().unwrap();
        if let Err(r) = relay.as_tower(&mut channels, &channel, &headers) {
            return *r;
        }
        channels.remove(&channel);
    }
    match relay.store.remove(&channel).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("cannot forget channel: {e}");
            say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not forget the channel; try again")
        }
    }
}

async fn reply(State(relay): State<Arc<Relay>>, Path((channel, id)): Path<(String, String)>, headers: HeaderMap, sealed: Bytes) -> Response {
    // the tower took a message that was kept for it: there is no radio waiting on the answer, only the next one to send
    // said so by the tower in a header of its own, never read from how the id is written
    if headers.contains_key(airband_core::relay::KEPT_HEADER) {
        let kept = id.as_str();
        {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return *r,
            };
            if ch.holding.as_ref().is_some_and(|(held, _)| held == kept) {
                ch.holding = None;
            }
            ch.wake.notify_one();
        }
        return match relay.store.held_done(&channel, kept).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => say(StatusCode::SERVICE_UNAVAILABLE, &e),
        };
    }
    let mut channels = relay.channels.lock().unwrap();
    let ch = match relay.as_tower(&mut channels, &channel, &headers) {
        Ok(ch) => ch,
        Err(r) => return *r,
    };
    match ch.waiting.remove(&id) {
        Some(radio) => {
            let _ = radio.send(sealed);
            StatusCode::NO_CONTENT.into_response()
        }
        None => say(StatusCode::GONE, "the radio stopped waiting for that answer"),
    }
}

/// Keep a sealed message for a tower that is away. The radio names it (`x-airband-id`), so sending it twice keeps
/// it once; the tower is handed what is kept in the order it came, as soon as it is back.
async fn hold(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, sealed: Bytes) -> Response {
    let (began, bytes) = (Instant::now(), sealed.len());
    let answer = keep(&relay, &channel, &headers, sealed).await;
    relay.note_failed("queue", &channel, answer.status(), began, bytes, StatusCode::ACCEPTED);
    answer
}
async fn keep(relay: &Arc<Relay>, channel: &str, headers: &HeaderMap, sealed: Bytes) -> Response {
    let id = headers.get("x-airband-id").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !(8..=40).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) || sealed.is_empty() {
        return say(StatusCode::BAD_REQUEST, "a kept message needs an id and a body");
    }
    if let Err(no) = relay.admit(Asks::Queue, channel, headers).await {
        return *no;
    }
    match relay.store.hold(channel, id, &sealed, HELD_MOST).await {
        Ok(true) => {
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(channel) {
                ch.has_held = true;
                ch.wake.notify_one();
            }
            StatusCode::ACCEPTED.into_response()
        }
        Ok(false) => say(StatusCode::INSUFFICIENT_STORAGE, "too many messages are already waiting for that tower"),
        Err(e) => {
            eprintln!("airband-relay: cannot keep a message: {e}");
            say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not keep it")
        }
    }
}

async fn call(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, sealed: Bytes) -> Response {
    let (began, bytes) = (Instant::now(), sealed.len());
    let answer = carry(&relay, &channel, &headers, sealed).await;
    relay.note_failed("call", &channel, answer.status(), began, bytes, StatusCode::OK);
    answer
}
async fn carry(relay: &Arc<Relay>, channel: &str, headers: &HeaderMap, sealed: Bytes) -> Response {
    if let Err(no) = relay.admit(Asks::Call, channel, headers).await {
        return *no;
    }
    let id = random_hex(12);
    let answer = {
        let mut channels = relay.channels.lock().unwrap();
        let Some(ch) = channels.get_mut(channel) else { return not_the_radio() };
        if ch.last_pull.is_none_or(|t| t.elapsed() > AIRBAND_GONE) {
            return say(StatusCode::SERVICE_UNAVAILABLE, "the tower is not connected to the relay");
        }
        // what he says must arrive in the order he said it: while older words are still being handed over, new
        // ones are turned back to join the end of that line
        if ch.has_held && headers.contains_key("x-airband-ordered") {
            return say(StatusCode::CONFLICT, "earlier messages are still on their way to the tower");
        }
        if ch.waiting.len() >= MAX_IN_FLIGHT {
            return say(StatusCode::TOO_MANY_REQUESTS, "too many requests in flight on this channel");
        }
        let (tx, rx) = oneshot::channel();
        ch.waiting.insert(id.clone(), tx);
        ch.queue.push_back((id.clone(), sealed));
        ch.wake.notify_one();
        rx
    };
    match tokio::time::timeout(ANSWER_WAIT, answer).await {
        Ok(Ok(sealed)) => sealed.into_response(),
        _ => {
            // nothing of a request nobody waits for stays in memory
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(channel) {
                ch.waiting.remove(&id);
                ch.queue.retain(|(queued, _)| queued != &id);
            }
            say(StatusCode::GATEWAY_TIMEOUT, "the tower did not answer")
        }
    }
}

/// A tower asks for its phone to be woken: a sealed notification, handed to the push service the phone's
/// address belongs to. Only the channel's own tower may ask, each channel only so often, and nothing of what
/// is passed on can be read here. Neither the address nor the notification is logged or kept.
async fn wake_phone(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let answer = |code: StatusCode, sent: bool, gone: bool, reason: &str| (code, axum::Json(PushAnswer { sent, gone, reason: reason.into() })).into_response();
    if !relay.limits.lock().unwrap().push_by_address.allow(&caller(&headers), Instant::now()) {
        return answer(StatusCode::TOO_MANY_REQUESTS, false, false, "too many notifications from this address; try again later");
    }
    {
        let mut channels = relay.channels.lock().unwrap();
        let ch = match relay.as_tower(&mut channels, &channel, &headers) {
            Ok(ch) => ch,
            Err(r) => return *r,
        };
        if !push::allow(&mut ch.pushed, Instant::now()) {
            return answer(StatusCode::TOO_MANY_REQUESTS, false, false, "too many notifications from this channel; try again later");
        }
    }
    let Ok(ask) = serde_json::from_slice::<PushRequest>(&body) else { return answer(StatusCode::BAD_REQUEST, false, false, "not a push request") };
    // an address is a short run of letters and digits; a sealed notification is base64 of a size one push can carry
    let plain = |t: &str, extra: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || extra.as_bytes().contains(&b));
    if !plain(&ask.to, "-_:") || ask.to.len() > 256 || !plain(&ask.sealed, "+/=") || ask.sealed.len() > PUSH_MOST * 4 / 3 + 4 {
        return answer(StatusCode::BAD_REQUEST, false, false, "the address or the sealed notification is not the right shape");
    }
    match relay.push.send(ask.provider, &ask.to, &ask.sealed, ask.question).await {
        None => answer(StatusCode::SERVICE_UNAVAILABLE, false, false, "this relay has no such push service"),
        Some(push::Outcome::Sent) => answer(StatusCode::OK, true, false, ""),
        Some(push::Outcome::Gone(why)) => answer(StatusCode::GONE, false, true, &why),
        Some(push::Outcome::Refused(why)) => {
            eprintln!("airband-relay: the push service refused a notification: {why}");
            answer(StatusCode::BAD_GATEWAY, false, false, &why)
        }
        Some(push::Outcome::Failed(why)) => {
            eprintln!("airband-relay: a notification could not be sent: {why}");
            answer(StatusCode::BAD_GATEWAY, false, false, &why)
        }
    }
}

async fn relay_from_env() -> Arc<Relay> {
    let store = store::Store::from_env().unwrap_or_else(|e| {
        eprintln!("airband-relay: {e}");
        std::process::exit(2);
    });
    // the database may be waking up or briefly unreachable at start: wait for it rather than start empty
    let mut saved = store.load().await;
    for attempt in 1..=10 {
        if saved.is_ok() {
            break;
        }
        eprintln!("airband-relay: cannot read the channel list from {} (try {attempt} of 10): {}", store.describe(), saved.as_ref().unwrap_err());
        tokio::time::sleep(Duration::from_secs(6)).await;
        saved = store.load().await;
    }
    let saved = saved.unwrap_or_else(|e| {
        eprintln!("airband-relay: giving up: {e}");
        std::process::exit(2);
    });
    println!("airband-relay: channels are kept in {}", store.describe());
    if let Err(e) = store.held_purge(HELD_FOR_MS / 86_400_000).await {
        eprintln!("airband-relay: could not clear old kept messages: {e}");
    }
    let channels = saved.into_iter().map(|(c, saved)| (c, Channel::from(saved))).collect::<HashMap<_, _>>();
    let admission = std::env::var("AIRBAND_RELAY_ADMISSION").ok().filter(|a| !a.is_empty());
    println!("airband-relay: {} channels known; opening a channel {}", channels.len(), if admission.is_some() { "needs the admission key" } else { "is open to anyone" });
    let push = push::Push::from_env();
    println!("airband-relay: {}", push.describe());
    Arc::new(Relay::new(push, channels, store, admission))
}

fn routes(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/v1/channels/{channel}", put(open).delete(close))
        .route("/v1/channels/{channel}/pull", get(pull))
        .route("/v1/channels/{channel}/reply/{id}", post(reply))
        .route("/v1/channels/{channel}/call", post(call))
        .route("/v1/channels/{channel}/queue", post(hold))
        .route("/v1/channels/{channel}/push", post(wake_phone))
        .route("/v1/waitlist", post(waitlist).options(waitlist_preflight))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE))
        .with_state(relay)
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7474);
    let app = routes(relay_from_env().await);
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.expect("cannot listen");
    println!("airband-relay on port {port}");
    axum::serve(listener, app).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await.expect("the relay stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::relay::Keys;

    /// A relay on a folder of its own, with nothing configured, and that folder.
    fn relay_in_a_folder(admission: Option<&str>) -> (Arc<Relay>, std::path::PathBuf) {
        let folder = std::env::temp_dir().join(format!("airband-relay-test-{}", random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::Store::Folder(folder.clone()), admission.map(str::to_string))), folder)
    }
    fn with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
        }
        headers
    }
    fn token(t: &str) -> (&'static str, String) {
        ("authorization", format!("Bearer {t}"))
    }
    async fn text(res: Response) -> (u16, String) {
        let status = res.status().as_u16();
        (status, String::from_utf8_lossy(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).to_string())
    }
    /// A tower opens its channel, saying who its radio is.
    async fn open_as(relay: &Arc<Relay>, keys: &Keys, admission: &str) -> u16 {
        let (auth, hash) = (token(&keys.pull_token), token_hash(&keys.radio_token));
        open(State(relay.clone()), Path(keys.channel.clone()), with(&[(auth.0, &auth.1), ("x-airband-radio-hash", &hash), ("x-airband-admission", admission)])).await.status().as_u16()
    }
    /// A radio leaves a message, showing `shown` as its token (none when empty).
    async fn leave(relay: &Arc<Relay>, channel: &str, shown: &str, id: &str) -> (u16, String) {
        let auth = token(shown);
        let mut headers = with(&[("x-airband-id", id)]);
        if !shown.is_empty() {
            headers.insert(auth.0, auth.1.parse().unwrap());
        }
        text(hold(State(relay.clone()), Path(channel.to_string()), headers, Bytes::from_static(b"sealed")).await).await
    }

    #[test]
    fn a_failed_request_is_one_line_with_nothing_of_the_channel_but_its_first_six_characters() {
        let channel = "0123456789abcdef0123456789abcdef";
        let line = failed_line("call", channel, 504, Duration::from_millis(40_012), 412, (3, true, true));
        assert_eq!(line, "airband-relay: call on 012345 answered 504 after 40012 ms (412 bytes sent; 3 in flight; kept messages: yes; tower pulling: yes)");
        assert!(!line.contains(&channel[..7]));
        // a name that is not a channel's at all says nothing of itself either
        assert!(failed_line("queue", "../../etc/passwd", 400, Duration::ZERO, 0, (0, false, false)).contains("queue on ecad answered 400"));
    }

    #[tokio::test]
    async fn nobody_sends_to_a_channel_that_is_not_theirs() {
        let (relay, folder) = relay_in_a_folder(None);
        let (mine, other) = (Keys::derive(&airband_core::relay::new_secret()).unwrap(), Keys::derive(&airband_core::relay::new_secret()).unwrap());
        assert_eq!(open_as(&relay, &mine, "").await, 201);
        assert_eq!(open_as(&relay, &other, "").await, 201);
        // the radio's token hash is kept with the channel from the moment it is opened
        assert_eq!(relay.store.load().await.unwrap()[&mine.channel].radio_hash, token_hash(&mine.radio_token));

        // nothing without the token, with a wrong one, or with another channel's
        let refused = leave(&relay, &mine.channel, "", "aaaaaaaa05").await;
        assert_eq!(refused.0, 401);
        assert_eq!(leave(&relay, &mine.channel, "not-the-token", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &other.radio_token, "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &mine.radio_token, "aaaaaaaa05").await.0, 202);
        // a channel that does not exist answers exactly as one that refuses
        assert_eq!(leave(&relay, &"0".repeat(32), "", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &"0".repeat(32), &mine.radio_token, "aaaaaaaa05").await, refused);
        // the other channel takes its own radio's token and not this one's
        assert_eq!(leave(&relay, &other.channel, &mine.radio_token, "aaaaaaaa06").await, refused);
        assert_eq!(leave(&relay, &other.channel, &other.radio_token, "aaaaaaaa06").await.0, 202);

        // a call is under the same rule: refused without the token, let through to "no tower" with it
        let ask = |headers: HeaderMap| call(State(relay.clone()), Path(mine.channel.clone()), headers, Bytes::from_static(b"sealed"));
        let auth = token(&mine.radio_token);
        assert_eq!(text(ask(HeaderMap::new()).await).await, refused);
        assert_eq!(text(ask(with(&[("authorization", "Bearer nope")])).await).await, refused);
        assert_eq!(ask(with(&[(auth.0, &auth.1)])).await.status().as_u16(), 503);
        assert_eq!(call(State(relay.clone()), Path("0".repeat(32)), HeaderMap::new(), Bytes::from_static(b"sealed")).await.status().as_u16(), 401);

        // a relay started again on the same folder remembers all of it
        let saved = store::Store::Folder(folder.clone()).load().await.unwrap();
        let again = Arc::new(Relay::new(push::Push::default(), saved.into_iter().map(|(c, s)| (c, Channel::from(s))).collect(), store::Store::Folder(folder.clone()), None));
        assert_eq!(leave(&again, &mine.channel, "", "aaaaaaaa07").await, refused);
        assert_eq!(leave(&again, &mine.channel, &mine.radio_token, "aaaaaaaa07").await.0, 202);
        assert_eq!(leave(&again, &other.channel, &other.radio_token, "aaaaaaaa08").await.0, 202);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn an_open_relay_takes_so_many_new_channels_from_one_address() {
        let (relay, folder) = relay_in_a_folder(None);
        let new = || Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let first = new();
        assert_eq!(open_as(&relay, &first, "").await, 201);
        for _ in 1..OPENED_AN_ADDRESS_A_DAY {
            assert_eq!(open_as(&relay, &new(), "").await, 201);
        }
        assert_eq!(open_as(&relay, &new(), "").await, 429);
        // a channel it already has is not a new one: its tower can still look in
        assert_eq!(open_as(&relay, &first, "").await, 204);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_tower_needs_its_channels_pull_token_for_everything_and_the_admission_key_to_open_one() {
        let (relay, folder) = relay_in_a_folder(Some("key"));
        let (mine, other) = (Keys::derive(&airband_core::relay::new_secret()).unwrap(), Keys::derive(&airband_core::relay::new_secret()).unwrap());
        // a new channel: not without the admission key
        assert_eq!(open_as(&relay, &mine, "wrong").await, 401);
        assert_eq!(open_as(&relay, &mine, "key").await, 201);
        // opening it again needs no key, only the same token; another tower's token is turned away
        assert_eq!(open_as(&relay, &mine, "").await, 204);
        let theirs = token(&other.pull_token);
        let their_radio = token_hash(&other.radio_token);
        let as_other = || with(&[(theirs.0, &theirs.1), ("x-airband-admission", "key"), ("x-airband-radio-hash", &their_radio)]);
        let channel = || Path(mine.channel.clone());
        assert_eq!(open(State(relay.clone()), channel(), as_other()).await.status().as_u16(), 403);
        // and so is everything else a tower does, with another channel's token or with none
        for headers in [as_other(), HeaderMap::new()] {
            assert_eq!(pull(State(relay.clone()), channel(), headers.clone()).await.status().as_u16(), 403);
            assert_eq!(reply(State(relay.clone()), Path((mine.channel.clone(), "abc".into())), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(reply(State(relay.clone()), Path((mine.channel.clone(), "held-abc".into())), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(wake_phone(State(relay.clone()), channel(), headers.clone(), Bytes::new()).await.status().as_u16(), 403);
            assert_eq!(close(State(relay.clone()), channel(), headers).await.status().as_u16(), 403);
        }
        // who this channel's radio is was said when it was opened, and nobody changes it
        assert_eq!(relay.channels.lock().unwrap()[&mine.channel].radio_hash, token_hash(&mine.radio_token));
        // a channel is not opened without saying who its radio is
        let fresh = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let own = token(&fresh.pull_token);
        assert_eq!(open(State(relay.clone()), Path(fresh.channel.clone()), with(&[(own.0, &own.1), ("x-airband-admission", "key")])).await.status().as_u16(), 400);
        // its own tower closes it, and it is gone
        let own = token(&mine.pull_token);
        assert_eq!(close(State(relay.clone()), channel(), with(&[(own.0, &own.1)])).await.status().as_u16(), 204);
        assert!(relay.store.load().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn only_so_much_is_taken_from_one_channel_and_from_one_address() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "").await, 201);
        let auth = token(&keys.radio_token);
        let from = |address: &str| with(&[(auth.0, &auth.1), ("x-airband-id", "dddddddd01"), ("x-forwarded-for", address)]);
        let leave = |address: &'static str| hold(State(relay.clone()), Path(keys.channel.clone()), from(address), Bytes::from_static(b"sealed"));
        // the channel's share for a minute, from two addresses, and then no more from anywhere
        for i in 0..KEPT_A_MINUTE {
            assert_eq!(leave(if i % 2 == 0 { "198.51.100.1" } else { "198.51.100.2" }).await.status().as_u16(), 202, "{i}");
        }
        assert_eq!(leave("198.51.100.3").await.status().as_u16(), 429);
        // one address asking about channels it has no token for uses up its own share and nobody else's
        let stranger = || hold(State(relay.clone()), Path("0".repeat(32)), with(&[("x-airband-id", "dddddddd01"), ("x-forwarded-for", "203.0.113.9")]), Bytes::from_static(b"sealed"));
        for _ in 0..KEPT_A_MINUTE * 2 {
            assert_eq!(stranger().await.status().as_u16(), 401);
        }
        assert_eq!(stranger().await.status().as_u16(), 429);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_limit_counts_per_key_and_starts_again_when_its_time_is_up() {
        let (mut limit, t0) = (Limiter::new(3, 60), Instant::now());
        assert!((0..3).all(|_| limit.allow("a", t0)) && !limit.allow("a", t0 + Duration::from_secs(59)));
        assert!(limit.allow("b", t0));
        assert!(limit.allow("a", t0 + Duration::from_secs(60)));
        // a full table turns new keys away and keeps counting the ones it has
        let mut limit = Limiter::new(2, 60);
        for i in 0..KEYS_KEPT {
            assert!(limit.allow(&i.to_string(), t0));
        }
        assert!(!limit.allow("one more", t0) && limit.allow("7", t0) && !limit.allow("7", t0));
        assert!(limit.allow("one more", t0 + Duration::from_secs(61)));
    }

    /// The push route, with a stand-in for Apple's server on this machine: the tower's token is asked for, the
    /// sealed notification reaches the service untouched under a pass signed by the key, and what the service
    /// says about an address comes back as the tower needs it.
    #[tokio::test]
    async fn a_tower_wakes_its_phone_through_the_push_service() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let (key, public) = push::tests::test_key();
        let seen = Arc::new(AtomicU32::new(0));
        // the stand-in: checks what a push service checks, and answers by the address
        let apple = {
            let seen = seen.clone();
            Router::new().route(
                "/3/device/{to}",
                post(move |Path(to): Path<String>, headers: HeaderMap, body: Bytes| {
                    let (public, seen) = (public.clone(), seen.clone());
                    async move {
                        let head = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                        let pass = head("authorization");
                        let sent: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                        let good = push::tests::pass_is_good(pass.strip_prefix("bearer ").unwrap_or(""), &public, "KEY123", "TEAM456") && head("apns-topic") == "dev.example.app" && head("apns-push-type") == "alert" && sent["aps"]["mutable-content"] == 1 && sent["sealed"] == "c2VhbGVk" && sent["aps"]["alert"]["body"] == "A question for you";
                        if !good {
                            return (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "reason": "InvalidProviderToken" })));
                        }
                        seen.fetch_add(1, Ordering::SeqCst);
                        match to.as_str() {
                            "aa11" => (StatusCode::OK, axum::Json(serde_json::json!({}))),
                            "bb22" => (StatusCode::GONE, axum::Json(serde_json::json!({ "reason": "Unregistered" }))),
                            _ => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "reason": "BadDeviceToken" }))),
                        }
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, apple).await });

        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        let apns = push::Apns::new(&key, "KEY123", "TEAM456", "dev.example.app", &format!("http://{at}")).unwrap();
        let folder = std::env::temp_dir().join(format!("airband-relay-test-{}", random_hex(6)));
        let relay = Arc::new(Relay::new(push::Push::with_apns(apns), HashMap::from([(keys.channel.clone(), Channel::new(token_hash(&keys.pull_token), token_hash(&keys.radio_token)))]), store::Store::Folder(folder), None));
        let ask = |token: &str, provider: airband_core::relay::PushService, to: &str| {
            let (relay, channel) = (relay.clone(), keys.channel.clone());
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
            let body = serde_json::to_vec(&PushRequest { provider, to: to.into(), sealed: "c2VhbGVk".into(), question: true }).unwrap();
            async move {
                let res = wake_phone(State(relay), Path(channel), headers, Bytes::from(body)).await;
                let status = res.status().as_u16();
                let answer: PushAnswer = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap_or_default();
                (status, answer.sent, answer.gone, answer.reason)
            }
        };
        // not the channel's tower: nothing is sent
        assert_eq!(ask("someone-else", airband_core::relay::PushService::Apns, "aa11").await.0, 403);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await, (200, true, false, String::new()));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "bb22").await, (410, false, true, "Unregistered".into()));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "cc33").await, (410, false, true, "BadDeviceToken".into()));
        assert_eq!(seen.load(Ordering::SeqCst), 3);
        // a service this relay does not have, and an address that is not one
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Unknown, "aa11").await.0, 503);
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11/../x").await.0, 400);
        // and only so many from one channel
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pushed = Some((Instant::now(), push::PUSHES));
        assert_eq!(ask(&keys.pull_token, airband_core::relay::PushService::Apns, "aa11").await.0, 429);
        assert_eq!(seen.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn what_passes_for_an_email_address() {
        for good in ["c@example.games", "a.b+c@example.co.uk"] {
            assert!(email_shaped(good), "{good}");
        }
        for bad in ["", "nobody", "@example.com", "a@b", "a@.com", "a@b.com.", "a b@example.com", "a@b@c.com", "x');drop--@example.com", "=cmd|calc@example.com", "<b>@example.com", "a@exa..mple.com"] {
            assert!(!email_shaped(bad), "{bad}");
        }
    }
    #[test]
    fn sign_ups_are_counted_per_caller_per_minute_and_per_day() {
        let (mut s, t0) = (Signups::default(), Instant::now());
        // one caller: five in an hour, and again after the hour
        assert!((0..5).all(|_| s.allow("1.1.1.1", t0)) && !s.allow("1.1.1.1", t0));
        assert!(s.allow("1.1.1.1", t0 + Duration::from_secs(3601)));
        // everyone together: twenty in a minute, whoever they are
        let (mut s, mut taken) = (Signups::default(), 0);
        for i in 0..40 {
            taken += s.allow(&format!("2.2.2.{i}"), t0) as u32;
        }
        assert_eq!(taken, A_MINUTE);
        assert!(s.allow("3.3.3.3", t0 + Duration::from_secs(61)));
        // and a day's worth, however slowly they come
        let mut s = Signups::default();
        let taken = (0..3000u64).filter(|i| s.allow(&format!("a{i}"), t0 + Duration::from_secs(i * 20))).count() as u32;
        assert_eq!(taken, A_DAY);
    }
    #[test]
    fn the_caller_is_the_address_the_load_balancer_added() {
        let mut h = HeaderMap::new();
        assert_eq!(caller(&h), "direct");
        h.insert("x-forwarded-for", "9.9.9.9, 203.0.113.7".parse().unwrap()); // the first is whatever the caller sent
        assert_eq!(caller(&h), "203.0.113.7");
    }
    #[test]
    fn only_the_homepage_is_named_as_an_allowed_origin() {
        let mut h = HeaderMap::new();
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert_eq!(from_site(&h)[0].1, "https://airband.dev");
        h.insert("origin", "https://www.airband.dev".parse().unwrap());
        assert_eq!(from_site(&h)[0].1, "https://www.airband.dev");
    }
}
