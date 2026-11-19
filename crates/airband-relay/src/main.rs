//! airband-relay: the meeting point for towers and radios that cannot reach each other directly.
//!
//! It keeps, per channel, the hash of the tower's pull token and the requests in flight. It holds no keys and
//! sees only sealed bytes (`airband.relay/1`, crates/airband-core/src/relay.rs). Put it behind TLS.
//!
//!   PORT                     port to listen on (default 7474)
//!   AIRBAND_RELAY_DATA       folder for channels.json (default ./relay-data), when no database is set
//!   DB_HOST and friends      keep the channel list in Postgres instead (store.rs)
//!   AIRBAND_RELAY_ADMISSION  if set, a tower must present this to open a channel; without it, anyone may
//!
//!   PUT  /v1/channels/{c}             tower: open the channel (Bearer pull token)
//!   GET  /v1/channels/{c}/pull        tower: the next sealed request, held up to 25 s (204 when none)
//!   POST /v1/channels/{c}/reply/{id}  tower: the sealed answer to request id
//!   POST /v1/channels/{c}/call        radio: a sealed request; the response is the sealed answer
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
use airband_core::relay::{MAX_MESSAGE, random_hex, token_hash};

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
}

mod store;

struct Relay {
    channels: Mutex<HashMap<String, Channel>>,
    store: store::Store,
    admission: Option<String>,
    /// How many sign-ups the waiting list has taken lately.
    signups: Mutex<Signups>,
    /// At most two sign-ups are being written to the database at any moment.
    writing: tokio::sync::Semaphore,
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

impl Relay {
    /// The channel, if this request carries its tower's pull token.
    fn as_tower<'a>(&self, channels: &'a mut HashMap<String, Channel>, channel: &str, headers: &HeaderMap) -> Result<&'a mut Channel, Response> {
        let ch = channels.get_mut(channel).ok_or_else(|| say(StatusCode::NOT_FOUND, "no such channel: open it first"))?;
        if same(&token_hash(bearer(headers)), &ch.pull_hash) { Ok(ch) } else { Err(say(StatusCode::FORBIDDEN, "not this channel's tower")) }
    }
}

async fn open(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    if !valid_channel(&channel) || bearer(&headers).is_empty() {
        return say(StatusCode::BAD_REQUEST, "a channel is 32 hex characters and needs a pull token");
    }
    let hash = token_hash(bearer(&headers));
    // opening again is how a tower checks it is still known after a relay restart
    let known = |channels: &HashMap<String, Channel>| channels.get(&channel).map(|existing| if same(&existing.pull_hash, &hash) { StatusCode::NO_CONTENT.into_response() } else { say(StatusCode::FORBIDDEN, "that channel belongs to another tower") });
    if let Some(answer) = known(&relay.channels.lock().unwrap()) {
        return answer;
    }
    if let Some(want) = &relay.admission {
        let given = headers.get("x-airband-admission").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !same(given, want) {
            return say(StatusCode::UNAUTHORIZED, "this relay needs an admission key to open a channel");
        }
    }
    // durable first: a channel the relay would forget on restart is worse than a slow answer
    if let Err(e) = relay.store.add(&channel, &hash).await {
        eprintln!("airband-relay: could not save a new channel: {e}");
        return say(StatusCode::SERVICE_UNAVAILABLE, "the relay could not save the channel; try again");
    }
    let mut channels = relay.channels.lock().unwrap();
    // two towers raced for the same name: the first one in keeps it
    if let Some(answer) = known(&channels) {
        return answer;
    }
    channels.insert(channel, Channel { pull_hash: hash, queue: VecDeque::new(), waiting: HashMap::new(), last_pull: None, wake: Arc::new(Notify::new()) });
    StatusCode::CREATED.into_response()
}

async fn pull(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let until = Instant::now() + HOLD;
    loop {
        let wake = {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return r,
            };
            ch.last_pull = Some(Instant::now());
            // a radio that gave up leaves its request behind; skip those
            while let Some((id, sealed)) = ch.queue.pop_front() {
                if ch.waiting.contains_key(&id) {
                    return ([("x-airband-request", id)], sealed).into_response();
                }
            }
            ch.wake.clone()
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return StatusCode::NO_CONTENT.into_response();
        }
        // wake at least every few seconds so last_pull stays current while the tower waits
        let _ = tokio::time::timeout(left.min(Duration::from_secs(5)), wake.notified()).await;
    }
}

/// The pages allowed to post to the waiting list from a browser.
const SITES: [&str; 2] = ["https://airband.dev", "https://www.airband.dev"];

/// How many sign-ups are taken: from one address on the internet in an hour, from everyone in a minute, and from
/// everyone in a day. The list is for people typing their own address into one page; anything faster is a
/// script, and the daily figure is what bounds how much a determined one can ever write to the database.
const PER_ADDRESS_AN_HOUR: u32 = 5;
const A_MINUTE: u32 = 20;
const A_DAY: u32 = 2000;
/// How many internet addresses are remembered at once for the hourly count.
const ADDRESSES_KEPT: usize = 20_000;

#[derive(Default)]
struct Signups {
    minute: Option<(Instant, u32)>,
    day: Option<(Instant, u32)>,
    by_address: HashMap<String, (Instant, u32)>,
}

impl Signups {
    /// May this one be taken? Counts it if so.
    fn allow(&mut self, address: &str, now: Instant) -> bool {
        let fresh = |slot: &Option<(Instant, u32)>, window: u64| slot.filter(|(began, _)| now.duration_since(*began) < Duration::from_secs(window)).unwrap_or((now, 0));
        let (minute, day) = (fresh(&self.minute, 60), fresh(&self.day, 86_400));
        self.by_address.retain(|_, (began, _)| now.duration_since(*began) < Duration::from_secs(3600));
        let mine = self.by_address.get(address).copied().unwrap_or((now, 0));
        // an address not seen before, with the table full: refuse rather than forget who has already had their five
        let room = self.by_address.contains_key(address) || self.by_address.len() < ADDRESSES_KEPT;
        if minute.1 >= A_MINUTE || day.1 >= A_DAY || mine.1 >= PER_ADDRESS_AN_HOUR || !room {
            return false;
        }
        self.minute = Some((minute.0, minute.1 + 1));
        self.day = Some((day.0, day.1 + 1));
        self.by_address.insert(address.to_string(), (mine.0, mine.1 + 1));
        true
    }
}

/// The headers that let the homepage, and only it, call this from a browser.
fn from_site(headers: &HeaderMap) -> [(&'static str, String); 4] {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).filter(|o| SITES.contains(o)).unwrap_or(SITES[0]);
    [("access-control-allow-origin", origin.to_string()), ("access-control-allow-methods", "POST, OPTIONS".into()), ("access-control-allow-headers", "content-type".into()), ("vary", "origin".into())]
}

/// Is this plausibly an email address? Whether it is a real one only a message to it can tell. Only the plain
/// characters nearly every address is made of are let through: the list will be read in spreadsheets and mail
/// tools one day, and nothing that could mean something to one of those belongs in it.
fn email_shaped(e: &str) -> bool {
    let Some((name, domain)) = e.split_once('@') else { return false };
    let plain = |part: &str, extra: &str| part.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    e.len() <= 254 && !name.is_empty() && plain(name, "._%+-") && plain(domain, ".-") && domain.contains('.') && !domain.starts_with(['.', '-']) && !domain.ends_with(['.', '-']) && !domain.contains("..")
}

/// Who is asking, as the load balancer saw them. It adds the caller's address to the end of this header, so the
/// last entry is the one a caller cannot make up; anything before it is whatever they chose to send.
fn caller(headers: &HeaderMap) -> String {
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| "direct".into())
}

async fn waitlist_preflight(headers: HeaderMap) -> Response {
    (StatusCode::NO_CONTENT, from_site(&headers)).into_response()
}

/// Someone on the homepage asks to be told when the app is ready.
///
/// What this guards against. Writing to the database from a script: the three limits above, counted before the
/// database is touched, and at most two writes at once, so the server it shares with other applications is never
/// flooded with connections. Other websites posting on a visitor's behalf: only JSON is taken, which a browser
/// will not send across sites without asking first, and only the homepage is told yes. Form-filling robots: a
/// field no person can see, which they fill in. Finding out who is on the list: the answer is the same whether
/// or not the address was already there. Injection: the address is only ever passed to the database as a value,
/// is never logged and is never shown on a page.
/// What it does not do: prove the address belongs to whoever typed it. Nothing is sent to it yet; when something
/// is, the first message must ask for confirmation.
async fn waitlist(State(relay): State<Arc<Relay>>, headers: HeaderMap, body: Bytes) -> Response {
    let cors = from_site(&headers);
    let answer = |code: StatusCode, why: &str| (code, cors.clone(), axum::Json(serde_json::json!({ "ok": code == StatusCode::OK, "message": why }))).into_response();
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !SITES.contains(&origin) {
        return answer(StatusCode::FORBIDDEN, "Sign up on airband.dev.");
    }
    if !headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("application/json")) {
        return answer(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.");
    }
    if body.len() > 1024 {
        return answer(StatusCode::PAYLOAD_TOO_LARGE, "That is too long to be an email address.");
    }
    let Ok(sent) = serde_json::from_slice::<serde_json::Value>(&body) else { return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.") };
    let email = sent["email"].as_str().map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if !email_shaped(&email) {
        return answer(StatusCode::BAD_REQUEST, "That does not look like an email address.");
    }
    // the field nobody can see: a person leaves it empty. Say yes and keep nothing, so the robot learns nothing.
    if sent["company"].as_str().is_some_and(|c| !c.is_empty()) {
        return answer(StatusCode::OK, "You are on the list.");
    }
    if !relay.signups.lock().unwrap().allow(&caller(&headers), Instant::now()) {
        return answer(StatusCode::TOO_MANY_REQUESTS, "Too many sign-ups right now. Try again later.");
    }
    let Ok(_turn) = relay.writing.try_acquire() else { return answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.") };
    match tokio::time::timeout(Duration::from_secs(8), relay.store.join_waitlist(&email)).await {
        Ok(Ok(())) => answer(StatusCode::OK, "You are on the list."),
        failed => {
            eprintln!("airband-relay: could not save a waitlist address: {}", failed.map(|r| r.unwrap_err()).unwrap_or_else(|_| "the database did not answer in time".into()));
            answer(StatusCode::SERVICE_UNAVAILABLE, "Could not save that just now. Try again in a minute.")
        }
    }
}

/// A tower closes its channel: the radio it was paired with has been cut off. Without this every channel ever
/// opened stayed on record.
async fn close(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    {
        let mut channels = relay.channels.lock().unwrap();
        if let Err(r) = relay.as_tower(&mut channels, &channel, &headers) {
            return r;
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
    let mut channels = relay.channels.lock().unwrap();
    let ch = match relay.as_tower(&mut channels, &channel, &headers) {
        Ok(ch) => ch,
        Err(r) => return r,
    };
    match ch.waiting.remove(&id) {
        Some(radio) => {
            let _ = radio.send(sealed);
            StatusCode::NO_CONTENT.into_response()
        }
        None => say(StatusCode::GONE, "the radio stopped waiting for that answer"),
    }
}

async fn call(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, sealed: Bytes) -> Response {
    let id = random_hex(12);
    let answer = {
        let mut channels = relay.channels.lock().unwrap();
        let Some(ch) = channels.get_mut(&channel) else { return say(StatusCode::NOT_FOUND, "no such channel") };
        if ch.last_pull.is_none_or(|t| t.elapsed() > AIRBAND_GONE) {
            return say(StatusCode::SERVICE_UNAVAILABLE, "the tower is not connected to the relay");
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
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
                ch.waiting.remove(&id);
            }
            say(StatusCode::GATEWAY_TIMEOUT, "the tower did not answer")
        }
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7474);
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
    let channels = saved.into_iter().map(|(c, pull_hash)| (c, Channel { pull_hash, queue: VecDeque::new(), waiting: HashMap::new(), last_pull: None, wake: Arc::new(Notify::new()) })).collect::<HashMap<_, _>>();
    let admission = std::env::var("AIRBAND_RELAY_ADMISSION").ok().filter(|a| !a.is_empty());
    println!("airband-relay on port {port}: {} channels known; opening a channel {}", channels.len(), if admission.is_some() { "needs the admission key" } else { "is open to anyone" });
    let relay = Arc::new(Relay { channels: Mutex::new(channels), store, admission, signups: Mutex::new(Signups::default()), writing: tokio::sync::Semaphore::new(2) });
    let app = Router::new()
        .route("/v1/channels/{channel}", put(open).delete(close))
        .route("/v1/channels/{channel}/pull", get(pull))
        .route("/v1/channels/{channel}/reply/{id}", post(reply))
        .route("/v1/channels/{channel}/call", post(call))
        .route("/v1/waitlist", post(waitlist).options(waitlist_preflight))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE))
        .with_state(relay);
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.expect("cannot listen");
    axum::serve(listener, app).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await.expect("the relay stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

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
