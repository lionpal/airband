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
//!   AIRBAND_RELAY_ADMISSION  if set, a tower must present this to open a channel; without it, anyone may
//!
//! Who may do what. A tower does everything with its channel's pull token, and opening a new channel also
//! takes the admission key. A radio does everything with its channel's radio token. Both tokens are made from
//! the pairing secret, and the relay holds only their hashes. A channel's name alone gets nothing, with one
//! exception for phones from before radios sent their token on `call` and `queue`: until a channel has once
//! seen its radio's token on one of those, they are taken without it. From that moment, for good, they are not.
//!
//!   PUT  /v1/channels/{c}             tower: open the channel (Bearer pull token; x-airband-radio-hash)
//!   GET  /v1/channels/{c}/pull        tower: the next sealed request, held up to 25 s (204 when none)
//!   POST /v1/channels/{c}/reply/{id}  tower: the sealed answer to request id
//!   POST /v1/channels/{c}/push        tower: a sealed notification for a push service to wake the phone with (push.rs)
//!   POST /v1/channels/{c}/call        radio: a sealed request; the response is the sealed answer (Bearer radio token)
//!   POST /v1/channels/{c}/queue       radio: a sealed message to keep for a tower that is away (x-airband-id names it)
//!   GET  /v1/channels/{c}/ears        radio: a pass to the relay's speech service
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
use airband_core::relay::{HELD, HELD_FOR_MS, HELD_MOST, MAX_MESSAGE, PUSH_MOST, PushAnswer, PushRequest, random_hex, token_hash};

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
    /// The hash of the token the paired radio proves itself with, as the tower gave it (when it opened the
    /// channel, and with each pull).
    radio_hash: Option<String>,
    /// A radio has shown that token on `call` or `queue`: nothing is taken on this channel without it again.
    proven: bool,
    /// How many notifications it has sent lately, and since when.
    pushed: Option<(Instant, u32)>,
}

impl Channel {
    #[cfg(test)]
    fn new(pull_hash: String) -> Channel {
        Channel::from(store::Saved { pull_hash, ..Default::default() })
    }
    fn from(saved: store::Saved) -> Channel {
        Channel { pull_hash: saved.pull_hash, queue: VecDeque::new(), waiting: HashMap::new(), last_pull: None, wake: Arc::new(Notify::new()), has_held: true, holding: None, radio_hash: saved.radio_hash, proven: saved.proven, pushed: None }
    }
}

/// A kept message the tower was handed and never answered for is handed over again after this long.
const HAND_AGAIN: Duration = Duration::from_secs(60);

mod dynamo;
mod push;
mod store;

/// How many keys (channels or internet addresses) one limit remembers at once.
const KEYS_KEPT: usize = 20_000;

/// At most so many requests per key in each stretch of time. A key is a channel or an internet address.
struct Limiter {
    most: u32,
    window: Duration,
    by: HashMap<String, (Instant, u32)>,
    swept: Option<Instant>,
}

impl Limiter {
    fn new(most: u32, seconds: u64) -> Limiter {
        Limiter { most, window: Duration::from_secs(seconds), by: HashMap::new(), swept: None }
    }
    /// May this one be taken? Counts it if so.
    fn allow(&mut self, key: &str, now: Instant) -> bool {
        // forget stretches that are over: now and then, and at once when there is no room left
        if self.by.len() >= KEYS_KEPT || self.swept.is_none_or(|at| now.duration_since(at) >= self.window) {
            let window = self.window;
            self.by.retain(|_, (began, _)| now.duration_since(*began) < window);
            self.swept = Some(now);
        }
        let full = self.by.len() >= KEYS_KEPT;
        match self.by.get_mut(key) {
            Some((began, count)) if now.duration_since(*began) < self.window => {
                if *count >= self.most {
                    return false;
                }
                *count += 1;
            }
            Some(slot) => *slot = (now, 1),
            // a key not seen before, with the table full: refuse rather than forget what others have used
            None if full => return false,
            None => {
                self.by.insert(key.to_string(), (now, 1));
            }
        }
        true
    }
}

/// The three things a radio asks of the relay.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Asks {
    Call,
    Queue,
    Ears,
}

/// How much is taken in a minute: on one channel, and from one internet address whatever channels it names.
/// A phone sending a large file makes many calls quickly, six at a time, so calls are allowed generously; how
/// many may wait at once (`MAX_IN_FLIGHT`) is what bounds them. Messages to keep are written to the database,
/// and passes cost a request to the speech service, so those are few. An address gets twice a channel's share:
/// a household's phones come from one address.
const CALLS_A_MINUTE: u32 = 3000;
const KEPT_A_MINUTE: u32 = 60;
const PASSES_A_MINUTE: u32 = 30;
/// Notifications asked for from one address in ten minutes, across its channels (`push::PUSHES` is one channel's).
const PUSHES_AN_ADDRESS: u32 = push::PUSHES * 4;

struct Limits {
    /// For each of [`Asks`]: by channel, then by address.
    radio: [(Limiter, Limiter); 3],
    push_by_address: Limiter,
}

impl Limits {
    fn new() -> Limits {
        let pair = |most: u32| (Limiter::new(most, 60), Limiter::new(most * 2, 60));
        Limits { radio: [pair(CALLS_A_MINUTE), pair(KEPT_A_MINUTE), pair(PASSES_A_MINUTE)], push_by_address: Limiter::new(PUSHES_AN_ADDRESS, 600) }
    }
    fn channel(&mut self, asks: Asks, channel: &str, now: Instant) -> bool {
        self.radio[asks as usize].0.allow(channel, now)
    }
    fn address(&mut self, asks: Asks, address: &str, now: Instant) -> bool {
        self.radio[asks as usize].1.allow(address, now)
    }
}

struct Relay {
    /// The push services this relay can wake phones through.
    push: push::Push,
    channels: Mutex<HashMap<String, Channel>>,
    store: store::Store,
    admission: Option<String>,
    /// A speech service's key and region, when this relay lends its own to towers that have none
    /// (`AZURE_SPEECH_KEY`, `AZURE_SPEECH_REGION`), and the pass last made with it.
    speech: Option<(String, String)>,
    speech_pass: Mutex<Option<(String, Instant)>>,
    /// How many sign-ups the waiting list has taken lately.
    signups: Mutex<Signups>,
    /// At most two sign-ups are being written to the database at any moment.
    writing: tokio::sync::Semaphore,
    /// How much each channel and each internet address has asked for lately.
    limits: Mutex<Limits>,
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
fn bearer(headers: &HeaderMap) -> &str {
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("")
}
/// A tower with no speech service of its own asks for a pass to this relay's: a token from Microsoft that works
/// for ten minutes, for listening and for speaking. The key itself never leaves here. Only a tower that holds
/// the admission key is given one, and a relay open to anyone lends nothing. One pass is made every eight
/// minutes at most, however many towers ask.
async fn speech_pass(State(relay): State<Arc<Relay>>, headers: HeaderMap) -> Response {
    let Some(want) = relay.admission.as_ref().filter(|_| relay.speech.is_some()) else { return say(StatusCode::NOT_FOUND, "this relay has no speech service to lend") };
    if !same(headers.get("x-airband-admission").and_then(|v| v.to_str().ok()).unwrap_or(""), want) {
        return say(StatusCode::UNAUTHORIZED, "a speech pass needs the admission key");
    }
    lend(&relay).await
}

/// The same pass, for a radio of a channel this relay knows: so he can still speak to his agents when his
/// computer cannot be reached to ask. What he says is then kept here, sealed, until it is back. Knowing a
/// channel's name is what a paired radio has; the pass is the one every tower shares, so asking costs nothing.
async fn ears_pass(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    if relay.speech.is_none() || relay.admission.is_none() {
        return say(StatusCode::NOT_FOUND, "this relay has no speech service to lend");
    }
    // only the radio paired on this channel: it shows a token made from the pairing secret, whose hash the
    // channel's tower has given. A channel's name alone gets nothing, and neither does a channel whose tower
    // has not said who its radio is.
    match relay.admit(Asks::Ears, &channel, &headers).await {
        Ok(()) => lend(&relay).await,
        Err(no) => no,
    }
}

async fn lend(relay: &Arc<Relay>) -> Response {
    let Some((key, region)) = relay.speech.clone() else { return say(StatusCode::NOT_FOUND, "this relay has no speech service to lend") };
    let fresh = relay.speech_pass.lock().unwrap().clone().filter(|(_, when)| when.elapsed() < Duration::from_secs(480));
    let token = match fresh {
        Some((token, _)) => token,
        None => {
            let url = format!("https://{region}.api.cognitive.microsoft.com/sts/v1.0/issueToken");
            let made = tokio::task::spawn_blocking(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).http_status_as_error(false).build().into();
                let mut res = agent.post(&url).header("Ocp-Apim-Subscription-Key", &key).header("Content-Length", "0").send_empty().ok()?;
                (res.status().as_u16() == 200).then(|| res.body_mut().read_to_string().ok()).flatten()
            })
            .await
            .ok()
            .flatten();
            let Some(token) = made else {
                eprintln!("airband-relay: the speech service gave no pass");
                return say(StatusCode::BAD_GATEWAY, "the speech service gave no pass");
            };
            *relay.speech_pass.lock().unwrap() = Some((token.clone(), Instant::now()));
            token
        }
    };
    axum::Json(serde_json::json!({ "kind": "azure", "token": token, "set": { "region": region, "language": "en-US" } })).into_response()
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
    fn new(push: push::Push, channels: HashMap<String, Channel>, store: store::Store, admission: Option<String>, speech: Option<(String, String)>) -> Relay {
        Relay { push, channels: Mutex::new(channels), store, admission, speech, speech_pass: Mutex::new(None), signups: Mutex::new(Signups::default()), writing: tokio::sync::Semaphore::new(2), limits: Mutex::new(Limits::new()) }
    }

    /// Is this request from the channel's radio, and within what a radio may ask? Every route a radio uses
    /// starts here.
    ///
    /// The radio shows a token made from the pairing secret (`Authorization: Bearer`), whose hash the channel's
    /// tower has given. A wrong token is always refused. No token is refused for a pass, and for `call` and
    /// `queue` once the channel is proven: once a radio has shown the right token on one of those two. Until
    /// then they are taken as they were before radios sent a token, so a phone on an older build keeps working.
    /// A pass does not prove a channel, because older phones already show the token for a pass and for nothing else.
    async fn admit(&self, asks: Asks, channel: &str, headers: &HeaderMap) -> Result<(), Response> {
        let now = Instant::now();
        if !self.limits.lock().unwrap().address(asks, &caller(headers), now) {
            return Err(say(StatusCode::TOO_MANY_REQUESTS, "too many requests from this address; try again in a minute"));
        }
        let token = bearer(headers);
        let shown = token_hash(token);
        let first = {
            let channels = self.channels.lock().unwrap();
            let Some(ch) = channels.get(channel) else { return Err(not_the_radio()) };
            match &ch.radio_hash {
                Some(hash) if !token.is_empty() => {
                    if !same(&shown, hash) {
                        return Err(not_the_radio());
                    }
                    !ch.proven && asks != Asks::Ears
                }
                _ if asks == Asks::Ears || ch.proven => return Err(not_the_radio()),
                _ => false,
            }
        };
        // counted only now, so a stranger who knows a proven channel's name cannot use up its radio's share
        if !self.limits.lock().unwrap().channel(asks, channel, now) {
            return Err(say(StatusCode::TOO_MANY_REQUESTS, "too many requests on this channel; try again in a minute"));
        }
        if first {
            // durable before it counts: after a restart the channel must still refuse what it refuses now
            match self.store.prove(channel).await {
                Ok(()) => {
                    if let Some(ch) = self.channels.lock().unwrap().get_mut(channel) {
                        ch.proven = true;
                    }
                }
                Err(e) => eprintln!("airband-relay: could not save that a channel's radio has shown its token: {e}"),
            }
        }
        Ok(())
    }

    /// The channel's tower has said what its radio's token hashes to: remember it, in memory and for good.
    async fn note_radio(&self, channel: &str, given: Option<&str>) {
        let Some(given) = given else { return };
        {
            let mut channels = self.channels.lock().unwrap();
            let Some(ch) = channels.get_mut(channel) else { return };
            if ch.radio_hash.as_deref() == Some(given) {
                return;
            }
            ch.radio_hash = Some(given.to_string());
        }
        if let Err(e) = self.store.set_radio(channel, given).await {
            eprintln!("airband-relay: could not save a channel's radio token hash: {e}");
        }
    }

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
    let radio = radio_hash_given(&headers);
    // opening again is how a tower checks it is still known after a relay restart
    let known = |channels: &HashMap<String, Channel>| channels.get(&channel).map(|existing| if same(&existing.pull_hash, &hash) { StatusCode::NO_CONTENT.into_response() } else { say(StatusCode::FORBIDDEN, "that channel belongs to another tower") });
    let answer = known(&relay.channels.lock().unwrap());
    if let Some(answer) = answer {
        if answer.status() == StatusCode::NO_CONTENT {
            relay.note_radio(&channel, radio).await;
        }
        return answer;
    }
    if let Some(want) = &relay.admission {
        let given = headers.get("x-airband-admission").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !same(given, want) {
            return say(StatusCode::UNAUTHORIZED, "this relay needs an admission key to open a channel");
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
    channels.insert(channel, Channel::from(store::Saved { pull_hash: hash, radio_hash: radio.map(str::to_string), proven: false }));
    StatusCode::CREATED.into_response()
}

async fn pull(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap) -> Response {
    let until = Instant::now() + HOLD;
    // the tower says who its radio is with every pull, for a relay that did not have it from when the channel was opened
    if relay.channels.lock().unwrap().get(&channel).is_some_and(|ch| same(&token_hash(bearer(&headers)), &ch.pull_hash)) {
        relay.note_radio(&channel, radio_hash_given(&headers)).await;
    }
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
                    return ([("x-airband-request", format!("{HELD}{id}"))], sealed).into_response();
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
    // the tower took a message that was kept for it: there is no radio waiting on the answer, only the next one to send
    if let Some(kept) = id.strip_prefix(HELD) {
        {
            let mut channels = relay.channels.lock().unwrap();
            let ch = match relay.as_tower(&mut channels, &channel, &headers) {
                Ok(ch) => ch,
                Err(r) => return r,
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

/// Keep a sealed message for a tower that is away. The radio names it (`x-airband-id`), so sending it twice keeps
/// it once; the tower is handed what is kept in the order it came, as soon as it is back.
async fn hold(State(relay): State<Arc<Relay>>, Path(channel): Path<String>, headers: HeaderMap, sealed: Bytes) -> Response {
    let id = headers.get("x-airband-id").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !(8..=40).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) || sealed.is_empty() {
        return say(StatusCode::BAD_REQUEST, "a kept message needs an id and a body");
    }
    if let Err(no) = relay.admit(Asks::Queue, &channel, &headers).await {
        return no;
    }
    match relay.store.hold(&channel, id, &sealed, HELD_MOST).await {
        Ok(true) => {
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
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
    if let Err(no) = relay.admit(Asks::Call, &channel, &headers).await {
        return no;
    }
    let id = random_hex(12);
    let answer = {
        let mut channels = relay.channels.lock().unwrap();
        let Some(ch) = channels.get_mut(&channel) else { return not_the_radio() };
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
            if let Some(ch) = relay.channels.lock().unwrap().get_mut(&channel) {
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
            Err(r) => return r,
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
    match relay.push.send(&ask.provider, &ask.to, &ask.sealed, ask.question).await {
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
    let speech = std::env::var("AZURE_SPEECH_KEY").ok().filter(|k| !k.is_empty()).zip(std::env::var("AZURE_SPEECH_REGION").ok().filter(|r| !r.is_empty()));
    println!("airband-relay: {}", if speech.is_some() { "lends its speech service to towers that have none" } else { "has no speech service to lend" });
    let push = push::Push::from_env();
    println!("airband-relay: {}", push.describe());
    Arc::new(Relay::new(push, channels, store, admission, speech))
}

fn routes(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/v1/channels/{channel}", put(open).delete(close))
        .route("/v1/channels/{channel}/pull", get(pull))
        .route("/v1/channels/{channel}/reply/{id}", post(reply))
        .route("/v1/channels/{channel}/call", post(call))
        .route("/v1/channels/{channel}/queue", post(hold))
        .route("/v1/channels/{channel}/ears", get(ears_pass))
        .route("/v1/channels/{channel}/push", post(wake_phone))
        .route("/v1/speech/pass", post(speech_pass))
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
        (Arc::new(Relay::new(push::Push::default(), HashMap::new(), store::Store::Folder(folder.clone()), admission.map(str::to_string), None)), folder)
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
    /// A radio leaves a message, showing `shown` as its token (nothing when empty, as an older phone does).
    async fn leave(relay: &Arc<Relay>, channel: &str, shown: &str, id: &str) -> (u16, String) {
        let auth = token(shown);
        let mut headers = with(&[("x-airband-id", id)]);
        if !shown.is_empty() {
            headers.insert(auth.0, auth.1.parse().unwrap());
        }
        text(hold(State(relay.clone()), Path(channel.to_string()), headers, Bytes::from_static(b"sealed")).await).await
    }

    #[tokio::test]
    async fn nobody_sends_to_a_channel_that_is_not_theirs() {
        let (relay, folder) = relay_in_a_folder(None);
        let (mine, other) = (Keys::derive(&airband_core::relay::new_secret()).unwrap(), Keys::derive(&airband_core::relay::new_secret()).unwrap());
        assert_eq!(open_as(&relay, &mine, "").await, 201);
        assert_eq!(open_as(&relay, &other, "").await, 201);
        // the radio's token hash is kept with the channel from the moment it is opened
        assert_eq!(relay.store.load().await.unwrap()[&mine.channel].radio_hash, Some(token_hash(&mine.radio_token)));

        // a channel that has never seen its radio's token takes a request without one, as an older phone sends it
        assert_eq!(leave(&relay, &mine.channel, "", "aaaaaaaa01").await.0, 202);
        // but never one with a wrong token, and that proves nothing
        assert_eq!(leave(&relay, &mine.channel, "not-the-token", "aaaaaaaa02").await.0, 401);
        assert_eq!(leave(&relay, &mine.channel, &other.radio_token, "aaaaaaaa02").await.0, 401);
        assert_eq!(leave(&relay, &mine.channel, "", "aaaaaaaa03").await.0, 202);

        // the right token once, and the channel is proven: in memory and where it survives a restart
        assert_eq!(leave(&relay, &mine.channel, &mine.radio_token, "aaaaaaaa04").await.0, 202);
        assert!(relay.channels.lock().unwrap()[&mine.channel].proven);
        assert!(relay.store.load().await.unwrap()[&mine.channel].proven);

        // from then on: nothing without the token, with a wrong one, or with another channel's
        let refused = leave(&relay, &mine.channel, "", "aaaaaaaa05").await;
        assert_eq!(refused.0, 401);
        assert_eq!(leave(&relay, &mine.channel, "not-the-token", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &other.radio_token, "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &mine.channel, &mine.radio_token, "aaaaaaaa05").await.0, 202);
        // a channel that does not exist answers exactly as one that refuses
        assert_eq!(leave(&relay, &"0".repeat(32), "", "aaaaaaaa05").await, refused);
        assert_eq!(leave(&relay, &"0".repeat(32), &mine.radio_token, "aaaaaaaa05").await, refused);
        // the other channel has proven nothing and is as it was
        assert_eq!(leave(&relay, &other.channel, "", "aaaaaaaa06").await.0, 202);

        // a call is under the same rule: refused without the token, let through to "no tower" with it
        let ask = |headers: HeaderMap| call(State(relay.clone()), Path(mine.channel.clone()), headers, Bytes::from_static(b"sealed"));
        let auth = token(&mine.radio_token);
        assert_eq!(text(ask(HeaderMap::new()).await).await, refused);
        assert_eq!(text(ask(with(&[("authorization", "Bearer nope")])).await).await, refused);
        assert_eq!(ask(with(&[(auth.0, &auth.1)])).await.status().as_u16(), 503);
        assert_eq!(call(State(relay.clone()), Path("0".repeat(32)), HeaderMap::new(), Bytes::from_static(b"sealed")).await.status().as_u16(), 401);

        // a relay started again on the same folder remembers all of it
        let saved = store::Store::Folder(folder.clone()).load().await.unwrap();
        let again = Arc::new(Relay::new(push::Push::default(), saved.into_iter().map(|(c, s)| (c, Channel::from(s))).collect(), store::Store::Folder(folder.clone()), None, None));
        assert_eq!(leave(&again, &mine.channel, "", "aaaaaaaa07").await, refused);
        assert_eq!(leave(&again, &mine.channel, &mine.radio_token, "aaaaaaaa07").await.0, 202);
        assert_eq!(leave(&again, &other.channel, "", "aaaaaaaa08").await.0, 202);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_pass_is_only_for_the_radio_and_asking_for_one_proves_nothing() {
        let (relay, folder) = relay_in_a_folder(Some("key"));
        let relay = Arc::new(Relay { speech: Some(("k".into(), "nowhere".into())), speech_pass: Mutex::new(Some(("a-pass".into(), Instant::now()))), ..Arc::into_inner(relay).unwrap() });
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        assert_eq!(open_as(&relay, &keys, "key").await, 201);
        let ask = |headers: HeaderMap| ears_pass(State(relay.clone()), Path(keys.channel.clone()), headers);
        assert_eq!(ask(HeaderMap::new()).await.status().as_u16(), 401);
        assert_eq!(ask(with(&[("authorization", "Bearer nope")])).await.status().as_u16(), 401);
        let auth = token(&keys.radio_token);
        let (status, body) = text(ask(with(&[(auth.0, &auth.1)])).await).await;
        assert!(status == 200 && body.contains("a-pass"), "{status} {body}");
        // an older phone shows its token for a pass and for nothing else: its calls must go on being taken
        assert!(!relay.channels.lock().unwrap()[&keys.channel].proven);
        assert_eq!(leave(&relay, &keys.channel, "", "bbbbbbbb01").await.0, 202);
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
        let as_other = || with(&[(theirs.0, &theirs.1), ("x-airband-admission", "key")]);
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
        // another tower cannot say who this channel's radio is
        let mut lying = as_other();
        lying.insert("x-airband-radio-hash", token_hash(&other.radio_token).parse().unwrap());
        assert_eq!(pull(State(relay.clone()), channel(), lying).await.status().as_u16(), 403);
        assert_eq!(relay.channels.lock().unwrap()[&mine.channel].radio_hash, Some(token_hash(&mine.radio_token)));
        // its own tower closes it, and it is gone
        let own = token(&mine.pull_token);
        assert_eq!(close(State(relay.clone()), channel(), with(&[(own.0, &own.1)])).await.status().as_u16(), 204);
        assert!(relay.store.load().await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[tokio::test]
    async fn a_tower_that_only_says_who_its_radio_is_when_it_pulls_is_remembered_too() {
        let (relay, folder) = relay_in_a_folder(None);
        let keys = Keys::derive(&airband_core::relay::new_secret()).unwrap();
        // an older tower: opens with its pull token alone
        let own = token(&keys.pull_token);
        assert_eq!(open(State(relay.clone()), Path(keys.channel.clone()), with(&[(own.0, &own.1)])).await.status().as_u16(), 201);
        assert_eq!(relay.store.load().await.unwrap()[&keys.channel].radio_hash, None);
        // something is waiting, so the pull returns at once
        assert_eq!(leave(&relay, &keys.channel, "", "cccccccc01").await.0, 202);
        let hash = token_hash(&keys.radio_token);
        let res = pull(State(relay.clone()), Path(keys.channel.clone()), with(&[(own.0, &own.1), ("x-airband-radio-hash", &hash)])).await;
        assert_eq!(res.headers().get("x-airband-request").unwrap(), "held-cccccccc01");
        assert_eq!(relay.store.load().await.unwrap()[&keys.channel].radio_hash, Some(hash));
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

    #[tokio::test]
    async fn a_channel_list_from_before_radios_had_tokens_is_still_read() {
        let folder = std::env::temp_dir().join(format!("airband-relay-test-{}", random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("channels.json"), r#"{"00000000000000000000000000000001":"abc"}"#).unwrap();
        let store = store::Store::Folder(folder.clone());
        assert_eq!(store.load().await.unwrap()["00000000000000000000000000000001"], store::Saved { pull_hash: "abc".into(), radio_hash: None, proven: false });
        store.set_radio("00000000000000000000000000000001", "def").await.unwrap();
        store.prove("00000000000000000000000000000001").await.unwrap();
        assert_eq!(store.load().await.unwrap()["00000000000000000000000000000001"], store::Saved { pull_hash: "abc".into(), radio_hash: Some("def".into()), proven: true });
        let _ = std::fs::remove_dir_all(folder);
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
        let relay = Arc::new(Relay::new(push::Push::with_apns(apns), HashMap::from([(keys.channel.clone(), Channel::new(token_hash(&keys.pull_token)))]), store::Store::Folder(folder), None, None));
        let ask = |token: &str, provider: &str, to: &str| {
            let (relay, channel) = (relay.clone(), keys.channel.clone());
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
            let body = serde_json::to_vec(&PushRequest { provider: provider.into(), to: to.into(), sealed: "c2VhbGVk".into(), question: true }).unwrap();
            async move {
                let res = wake_phone(State(relay), Path(channel), headers, Bytes::from(body)).await;
                let status = res.status().as_u16();
                let answer: PushAnswer = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), 1 << 16).await.unwrap()).unwrap_or_default();
                (status, answer.sent, answer.gone, answer.reason)
            }
        };
        // not the channel's tower: nothing is sent
        assert_eq!(ask("someone-else", "apns", "aa11").await.0, 403);
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        assert_eq!(ask(&keys.pull_token, "apns", "aa11").await, (200, true, false, String::new()));
        assert_eq!(ask(&keys.pull_token, "apns", "bb22").await, (410, false, true, "Unregistered".into()));
        assert_eq!(ask(&keys.pull_token, "apns", "cc33").await, (410, false, true, "BadDeviceToken".into()));
        assert_eq!(seen.load(Ordering::SeqCst), 3);
        // a service this relay does not have, and an address that is not one
        assert_eq!(ask(&keys.pull_token, "carrier-pigeon", "aa11").await.0, 503);
        assert_eq!(ask(&keys.pull_token, "apns", "aa11/../x").await.0, 400);
        // and only so many from one channel
        relay.channels.lock().unwrap().get_mut(&keys.channel).unwrap().pushed = Some((Instant::now(), push::PUSHES));
        assert_eq!(ask(&keys.pull_token, "apns", "aa11").await.0, 429);
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
