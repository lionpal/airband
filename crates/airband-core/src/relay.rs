//! `airband.relay/1`: a radio reaching its tower through a relay on the internet.
//!
//! The tower sits behind a home router, so it cannot be called; it calls out. Both sides connect outward to a
//! relay, which passes sealed messages between them and can read none of them:
//!
//!   radio  ──POST call──▶  relay  ◀──GET pull──  tower        (the radio's request, sealed)
//!   radio  ◀──response───  relay  ◀──POST reply─  tower        (the tower's answer, sealed)
//!
//! Pairing gives the radio and the tower one 32-byte secret, which never reaches the relay. Everything else is
//! derived from it: the channel's name on the relay, the token that lets only the tower pull from that
//! channel, and one encryption key per direction. Each message is sealed with ChaCha20-Poly1305 (RFC 8439) under
//! a random 96-bit nonce, and keys are derived with HKDF-SHA256 (RFC 5869): published standards only, nothing
//! of our own. A random 96-bit nonce is safe for the few thousand messages a pairing ever carries; a pairing that
//! might carry billions should count its nonces instead. Inside each request are the time and a random id; the tower refuses a request that is old or
//! that it has already seen, and the radio refuses an answer whose id is not its request's.
//!
//! What this does not give: forward secrecy. Someone who recorded the relay's traffic and later got the
//! pairing secret could read the old traffic. Re-pairing makes a new secret.
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use crate::encode::{hex, random_hex, unhex};
use sha2::{Digest, Sha256};

mod push;
mod routes;
pub use push::*;
pub use routes::{Route, SEALED};

pub const SCHEMA: &str = "airband.relay/1";
/// A request older or newer than this, by the tower's clock, is refused.
pub const FRESH_MS: u64 = 120_000;
/// The largest sealed message a relay passes, either way: a radio's request, a message kept for a tower, and
/// a tower's answer. The relay refuses anything larger, and a tower never sends a larger answer
/// ([`Keys::seal_answer_for_relay`]).
pub const MAX_MESSAGE: usize = 256 * 1024;
/// What sealing adds to a message: its nonce and its tag.
const SEALING: usize = 12 + 16;

/// The hash of the radio's own key, on a tower's opening of a channel: which radio the channel is for.
pub const RADIO_HASH_HEADER: &str = "x-airband-radio-hash";
/// The relay's admission key, when the relay asks for one.
pub const ADMISSION_HEADER: &str = "x-airband-admission";
/// On what the relay hands a tower's pull: the id it gave that request, which the tower answers to in the
/// address of its reply (`/reply/{id}`).
pub const REQUEST_HEADER: &str = "x-airband-request";
/// On a radio's message for the relay to keep (`/queue`): the radio's own id for it, so one sent twice is kept
/// once.
pub const ID_HEADER: &str = "x-airband-id";
/// On a radio's request: keep it in order behind the ones the relay is keeping for the tower.
pub const ORDERED_HEADER: &str = "x-airband-ordered";

/// On every pull: the version of the pull the tower speaks ([`PULL_VERSION`]). The relay takes a pull in any
/// version of [`PULL_VERSIONS`] and refuses any other (426, with [`PullRefused::OtherVersion`] saying which side
/// is the older).
pub const PULL_HEADER: &str = "x-airband-pull";
/// The version a tower speaks: pulls that say what they got ([`GOT_HEADER`]), which tower is pulling
/// ([`TOWER_HEADER`]) and which kept message it is still answering for ([`ANSWERING_HEADER`]).
pub const PULL_VERSION: &str = "3";
/// Every version of the pull a relay takes. 2 says what it got and nothing more: a kept message it was handed
/// waits for its answer however long that takes, and two towers on one pairing are not told apart.
pub const PULL_VERSIONS: std::ops::RangeInclusive<u32> = 2..=3;
/// On a pull: the id of the request the tower's previous pull was handed and the tower read whole. A tower pulls
/// one at a time, so a request handed over that the next pull does not name never reached it (the connection died
/// on the way, or it could not be read), and the relay hands it over again. Nothing else tells the relay a request
/// arrived. Said until a pull is taken ([`Pulling::taken`]): one that fails before the relay looks at it has told
/// it nothing.
pub const GOT_HEADER: &str = "x-airband-got";
/// On a pull (3): which tower is pulling, a random id it makes each time it starts. Two towers with one pairing
/// (its settings copied to another computer) would each take the other's pull's place for ever; the relay refuses
/// the one that is not pulling now (409, [`PullRefused::AnotherTower`]).
pub const TOWER_HEADER: &str = "x-airband-tower";
/// On a pull (3): the kept message ([`KEPT_HEADER`]) the tower is still answering for. The relay hands nothing
/// else kept until that answer comes ([`KEPT_HEADER`] on the reply). A pull that does not name it says the tower
/// no longer is (the answer did not get there, or the work on it ended without one): the relay, which still
/// keeps it, hands it over again.
pub const ANSWERING_HEADER: &str = "x-airband-answering";

/// Which side of a pull is the older, when the two speak no version of it in common.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Tower,
    Relay,
}

/// Why a relay would not take a pull: the body of its 426 or 409.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "refused", rename_all = "snake_case")]
pub enum PullRefused {
    /// 426: the tower and the relay speak no version of the pull in common; `older` is the one to update.
    OtherVersion { older: Side },
    /// 409: another tower with this pairing is pulling. Two computers are serving one pairing; one of them has
    /// to stop, or be paired again on its own.
    AnotherTower,
    /// 409: a pull from another tower with this pairing was held open, and the relay has let it go to see
    /// whether that tower is still there: one that restarted under a new id, or whose connection a proxy keeps
    /// after it went, is not. Ask again at once, saying nothing to him: if the other tower pulls again first, the
    /// answer is [`PullRefused::AnotherTower`]; if not, this pull is taken.
    AskAgain,
    /// A reason this build does not know.
    #[serde(other)]
    Unknown,
}

impl PullRefused {
    /// The status the relay answers with.
    pub fn status(self) -> u16 {
        match self {
            PullRefused::OtherVersion { .. } => 426,
            PullRefused::AnotherTower | PullRefused::AskAgain | PullRefused::Unknown => 409,
        }
    }
    /// The version a pull says it speaks ([`PULL_HEADER`]), if a relay takes it; else why not, with the side to
    /// update. A pull that says no version, or one that is not a number, is from a tower older than any taken.
    pub fn version(said: Option<&str>) -> Result<u32, PullRefused> {
        let v = said.and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
        if PULL_VERSIONS.contains(&v) {
            Ok(v)
        } else {
            Err(PullRefused::OtherVersion { older: if v < *PULL_VERSIONS.start() { Side::Tower } else { Side::Relay } })
        }
    }
}

/// Why the relay did not pass a radio's request on (`/call`, [`Route::Call`]) or keep it (`/queue`,
/// [`Route::Queue`]): the body of its refusal, written by the relay and read by the radio, each by this type.
/// Its status is [`CallRefused::status`], the one place the relay's statuses for these are agreed. A refusal
/// with no such body is the relay's own about the request itself (its token, its size, how many came), told by
/// its status alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "refused", rename_all = "snake_case")]
pub enum CallRefused {
    /// 409 on `/call` for an ordered request ([`ORDERED_HEADER`]): earlier messages are still on their way to
    /// the tower. Leave it with the relay (`/queue`) to go behind them.
    Behind,
    /// 503 on `/call`: the tower is not connected to the relay.
    Away,
    /// 504 on `/call`: the tower was handed it and did not answer in time.
    TowerSilent,
    /// 410: the pairing was ended. Only a new pairing reaches that computer again.
    Ended,
    /// A reason this build does not know.
    #[serde(other)]
    Unknown,
}

impl CallRefused {
    /// The status the relay answers with.
    pub fn status(self) -> u16 {
        match self {
            CallRefused::Behind | CallRefused::Unknown => 409,
            CallRefused::Away => 503,
            CallRefused::TowerSilent => 504,
            CallRefused::Ended => 410,
        }
    }
    /// What the relay's answer on `route` says: passed on (a call answered, a message kept), or refused, with
    /// its reason when its body gives one (`None` when it does not: see [`CallRefused`]).
    pub fn of(route: Route, status: u16, body: &[u8]) -> Result<(), Option<CallRefused>> {
        match (route, status) {
            (Route::Call, 200) | (Route::Queue, 202) => Ok(()),
            _ => Err(serde_json::from_slice(body).ok()),
        }
    }
}

/// What a tower says on its pulls, kept from one pull to the next: the tower's side of the pull protocol in one
/// place. Shared by the pull loop and the work answering a kept message.
#[derive(Debug, Clone)]
pub struct Pulling {
    tower: String,
    got: Option<String>,
    answering: Option<Handing>,
    /// How many kept messages have been handed to this tower: each handing is told apart by its number.
    handings: u64,
}

/// One handing of a kept message to this tower ([`Pulling::handed`]). The work on it ends that handing only: a
/// message handed again meanwhile (the channel was opened again, the relay handed it over a second time) is
/// still answered for until the work on that handing ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handing {
    id: String,
    n: u64,
}

impl Default for Pulling {
    fn default() -> Pulling {
        Pulling::new()
    }
}

impl Pulling {
    /// A tower that has just started: a new id, nothing got, nothing being answered.
    pub fn new() -> Pulling {
        Pulling::of_tower(random_hex(8))
    }
    /// A tower that has just started under the id it keeps (made once on that computer and kept): a pull it made
    /// before a restart, or under another spelling of the relay's address, is then its own, never another
    /// computer's.
    pub fn of_tower(tower: impl Into<String>) -> Pulling {
        Pulling { tower: tower.into(), got: None, answering: None, handings: 0 }
    }
    /// The headers for the next pull: its version and tower, what the last pull got, and the kept message being
    /// answered for.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut said = vec![(PULL_HEADER, PULL_VERSION.to_string()), (TOWER_HEADER, self.tower.clone())];
        said.extend(self.got.clone().map(|id| (GOT_HEADER, id)));
        said.extend(self.answering.as_ref().map(|h| (ANSWERING_HEADER, h.id.clone())));
        said
    }
    /// The relay took the pull (it handed a request over, or had nothing): it has heard what the pull said it
    /// got. Not after a pull it refused or that never reached it, which told it nothing.
    pub fn taken(&mut self) {
        self.got = None;
    }
    /// A pull was handed this request and the tower read it whole (never before: one cut off on the way is not
    /// got, and is handed over again). `kept`: one the relay kept ([`KEPT_HEADER`]), which the tower is now
    /// answering for until [`Pulling::answered`] with the handing this returns (nothing for one not kept).
    pub fn handed(&mut self, id: &str, kept: bool) -> Option<Handing> {
        self.got = Some(id.to_string());
        if !kept {
            return None;
        }
        self.handings += 1;
        let handing = Handing { id: id.to_string(), n: self.handings };
        self.answering = Some(handing.clone());
        Some(handing)
    }
    /// The work on one handing of a kept message ended, its answer sent or not. Called however it ended, a panic
    /// included: the next pull stops naming it, and if the relay did not hear the answer it hands the message
    /// over again. The work on an earlier handing of the same message ending changes nothing. Whether it was the
    /// handing the pulls named, now let go.
    pub fn answered(&mut self, handing: &Handing) -> bool {
        let named = self.answering.as_ref() == Some(handing);
        if named {
            self.answering = None;
        }
        named
    }
    /// The tower opened its channel again: what the last pull got is not said again. A kept message whose work
    /// still runs is still answered for (the opening says so, and the relay keeps it in this tower's hands).
    pub fn reopened(&mut self) {
        self.got = None;
    }
}

/// On what the relay hands a tower, and on the tower's reply to it: this is one the relay had been keeping while
/// the tower was away. Each side goes by this and never by how the request's id reads.
pub const KEPT_HEADER: &str = "x-airband-kept";
/// How long the relay keeps one, and so how old one may be when the tower takes it.
pub const HELD_FOR_MS: u64 = 7 * 24 * 3600 * 1000;
/// How many a channel may have waiting.
pub const HELD_MOST: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    ToTower,
    ToRadio,
    /// A notification for the phone, which the relay hands to a push service still sealed. It has a key of
    /// its own, so it cannot be passed off as an answer, nor an answer as a notification.
    Push,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Direction::ToTower => "to tower",
            Direction::ToRadio => "to radio",
            Direction::Push => "push",
        }
    }
}

/// Everything derived from one pairing secret.
#[derive(Clone)]
pub struct Keys {
    /// The channel's name on the relay. Not secret: knowing it lets you send the tower noise it will discard.
    pub channel: String,
    /// Proves to the relay that this is the tower for the channel. The relay stores only its hash.
    pub pull_token: String,
    /// Proves to the relay that this is the radio paired on the channel, for the little the relay does for a
    /// radio by itself. The tower tells the relay its hash; the relay never sees the token until a radio shows it.
    pub radio_token: String,
    to_tower: [u8; 32],
    to_radio: [u8; 32],
    push: [u8; 32],
}

/// A new pairing secret: 32 random bytes, as hex.
pub fn new_secret() -> String {
    random_hex(32)
}
/// What the relay stores for a token, the tower's pull token and the radio's token alike.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

impl Keys {
    pub fn derive(secret_hex: &str) -> Result<Keys, String> {
        let secret = unhex(secret_hex.trim()).filter(|s| s.len() == 32).ok_or("a pairing secret is 64 hex characters")?;
        let hk = Hkdf::<Sha256>::new(Some(SCHEMA.as_bytes()), &secret);
        let take = |label: &str| {
            let mut out = [0u8; 32];
            hk.expand(label.as_bytes(), &mut out).expect("32 bytes is a valid length");
            out
        };
        Ok(Keys { channel: hex(&take("channel")[..16]), pull_token: hex(&take("pull")), radio_token: hex(&take("radio")), to_tower: take(Direction::ToTower.label()), to_radio: take(Direction::ToRadio.label()), push: take(Direction::Push.label()) })
    }

    fn cipher(&self, dir: Direction) -> ChaCha20Poly1305 {
        let key = match dir {
            Direction::ToTower => &self.to_tower,
            Direction::ToRadio => &self.to_radio,
            Direction::Push => &self.push,
        };
        ChaCha20Poly1305::new(key.into())
    }
    /// Bound into every message, so one cannot be replayed on another channel or turned around.
    fn bound(&self, dir: Direction) -> Vec<u8> {
        format!("{SCHEMA} {} {}", self.channel, dir.label()).into_bytes()
    }

    /// nonce (12 bytes) followed by the ciphertext.
    pub fn seal(&self, dir: Direction, plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).expect("the system has no random source");
        let sealed = self.cipher(dir).encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: &self.bound(dir) }).expect("sealing cannot fail");
        [nonce.as_slice(), &sealed].concat()
    }

    pub fn open(&self, dir: Direction, sealed: &[u8]) -> Result<Vec<u8>, String> {
        if sealed.len() < SEALING {
            return Err("too short to be a sealed message".into());
        }
        let (nonce, body) = sealed.split_at(12);
        self.cipher(dir).decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: &self.bound(dir) }).map_err(|_| "not sealed with this pairing's key".to_string())
    }

    /// This channel's address at a relay for one of its routes: the one place the relay's addresses are put
    /// together, for the tower and the radio alike.
    pub fn to(&self, relay: &str, route: Route) -> String {
        format!("{}{}", relay.trim_end_matches('/'), route.on(&self.channel))
    }
    /// Where a tower on his own network takes this channel's sealed requests straight, with no relay between
    /// (`base` is the address its feed lists): [`routes::SEALED`] for this channel.
    pub fn straight(&self, base: &str) -> String {
        format!("{}{}", base.trim_end_matches('/'), crate::template::filled(routes::SEALED, &[("channel", &self.channel)]))
    }
    /// What a tower shows the relay it is this channel's tower with (`Authorization`).
    pub fn as_tower(&self) -> String {
        crate::token::as_bearer(&self.pull_token)
    }
    /// What a radio shows the relay it is this channel's radio with (`Authorization`).
    pub fn as_radio(&self) -> String {
        crate::token::as_bearer(&self.radio_token)
    }
    /// A request sealed for the tower.
    pub fn seal_call(&self, call: &Call) -> Vec<u8> {
        self.seal(Direction::ToTower, &serde_json::to_vec(call).unwrap_or_default())
    }
    /// A request as a radio sealed it. Nothing for bytes this pairing's key does not open or that are not one.
    pub fn open_call(&self, sealed: &[u8]) -> Option<Call> {
        self.open_request(sealed)?.ok()
    }
    /// A request as a radio sealed it, or, for one this pairing's key opens and this build cannot read (a method
    /// it does not know), its id: it is from a radio that speaks another version, and is answered so under that
    /// id rather than dropped. Nothing for bytes the key does not open or that carry no id.
    pub fn open_request(&self, sealed: &[u8]) -> Option<Result<Call, Unread>> {
        let plain = self.open(Direction::ToTower, sealed).ok()?;
        match serde_json::from_slice::<Call>(&plain) {
            Ok(call) => Some(Ok(call)),
            Err(_) => serde_json::from_slice::<Unread>(&plain).ok().map(Err),
        }
    }
    /// An answer sealed for the radio.
    pub fn seal_answer(&self, answer: &Answer) -> Vec<u8> {
        self.seal(Direction::ToRadio, &serde_json::to_vec(answer).unwrap_or_default())
    }
    /// An answer sealed for the radio, to go through a relay: no larger than [`MAX_MESSAGE`]. One that would be
    /// is not cut short or left for the relay to turn away (the radio would wait and hear nothing); the radio is
    /// answered with the typed too-big refusal, under the same id.
    pub fn seal_answer_for_relay(&self, answer: &Answer) -> Vec<u8> {
        let plain = serde_json::to_vec(answer).unwrap_or_default();
        if plain.len() + SEALING <= MAX_MESSAGE {
            return self.seal(Direction::ToRadio, &plain);
        }
        let no = crate::radio::wire::Refusal::TooBig;
        let body = serde_json::to_value(&no).unwrap_or_default();
        self.seal_answer(&Answer { id: answer.id.clone(), status: no.status(), body })
    }
    /// The tower's answer to one request, by the request's id. Nothing for bytes this pairing's key does
    /// not open, and nothing for an answer to some other request: only the tower can make one, and only
    /// the one asked for is this one's.
    pub fn open_answer(&self, sealed: &[u8], id: &str) -> Option<Answer> {
        serde_json::from_slice::<Answer>(&self.open(Direction::ToRadio, sealed).ok()?).ok().filter(|a| a.id == id)
    }
}

/// What the radio seals and sends: one request in the radio protocol ([`crate::radio::SCHEMA`]).
#[derive(Serialize, Deserialize)]
pub struct Call {
    /// When it was made, in milliseconds since 1970.
    pub t: u64,
    /// Random; the tower answers each id once.
    pub id: String,
    pub method: crate::radio::wire::Method,
    /// Under `/v1/radio/`.
    pub path: String,
    #[serde(default)]
    pub body: serde_json::Value,
    /// The radio protocol the request is in ([`crate::radio::SCHEMA`]). A tower that does not speak it says so, in
    /// place of guessing. (The relay protocol's own version needs no field: it is part of how the keys are
    /// derived, so two versions cannot read each other at all.) A request that says none reads as saying the
    /// empty one, which is another version: it is answered so, not dropped as unreadable.
    #[serde(default)]
    pub schema: String,
    /// Where the radio stands as it makes this request: on a call or not, showing the chat or not.
    #[serde(default)]
    pub presence: crate::radio::Presence,
}

/// A request this pairing's key opened that this build cannot read as a [`Call`]: only its id, to answer it
/// [`crate::radio::wire::Refusal::OtherVersion`] under ([`Keys::open_request`]).
#[derive(Debug, Deserialize)]
pub struct Unread {
    pub id: String,
}

/// His words are in the body: its debug form says how long it is, never what it says.
impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call").field("t", &self.t).field("id", &self.id).field("method", &self.method).field("path", &self.path).field("body", &crate::redact::Withheld(&self.body.to_string())).field("schema", &self.schema).field("presence", &self.presence).finish()
    }
}

/// What the tower seals and sends back.
#[derive(Debug, Serialize, Deserialize)]
pub struct Answer {
    /// The id of the call this answers.
    pub id: String,
    pub status: u16,
    #[serde(default)]
    pub body: serde_json::Value,
}

/// True when a call's time is close enough to now to be accepted.
pub fn fresh(call_ms: u64, now_ms: u64) -> bool {
    call_ms.abs_diff(now_ms) <= FRESH_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_ends_derive_the_same_and_only_the_right_key_opens() {
        let secret = new_secret();
        let (tower, radio) = (Keys::derive(&secret).unwrap(), Keys::derive(&secret).unwrap());
        assert_eq!(tower.channel, radio.channel);
        assert_eq!(tower.channel.len(), 32);
        let sealed = radio.seal(Direction::ToTower, b"leave them");
        assert_eq!(tower.open(Direction::ToTower, &sealed).unwrap(), b"leave them");
        // not as an answer, not by another pairing, not after a bit is flipped
        assert!(tower.open(Direction::ToRadio, &sealed).is_err());
        assert!(tower.open(Direction::Push, &sealed).is_err());
        // a notification opens as a notification and as nothing else
        let note = tower.seal(Direction::Push, b"shop 3 asks");
        assert_eq!(radio.open(Direction::Push, &note).unwrap(), b"shop 3 asks");
        assert!(radio.open(Direction::ToRadio, &note).is_err() && radio.open(Direction::ToTower, &note).is_err());
        assert!(Keys::derive(&new_secret()).unwrap().open(Direction::ToTower, &sealed).is_err());
        let mut bent = sealed.clone();
        *bent.last_mut().unwrap() ^= 1;
        assert!(tower.open(Direction::ToTower, &bent).is_err());
    }
    #[test]
    fn an_answer_through_a_relay_is_never_larger_than_the_relay_passes_and_one_too_large_says_so() {
        let k = Keys::derive(&new_secret()).unwrap();
        let answer = |n: usize| Answer { id: "abc".into(), status: 200, body: serde_json::Value::String("x".repeat(n)) };
        let small = k.seal_answer_for_relay(&answer(1000));
        assert_eq!(k.open_answer(&small, "abc").unwrap().body, answer(1000).body);
        let large = k.seal_answer_for_relay(&answer(MAX_MESSAGE));
        assert!(large.len() <= MAX_MESSAGE);
        let told = k.open_answer(&large, "abc").unwrap();
        assert_eq!(told.status, 413);
        assert!(matches!(serde_json::from_value(told.body).unwrap(), crate::radio::wire::Refusal::TooBig));
        // right at the edge: what fits is sent as it is
        let edge = MAX_MESSAGE - SEALING - serde_json::to_vec(&answer(0)).unwrap().len();
        let at_edge = k.seal_answer_for_relay(&answer(edge));
        assert_eq!((at_edge.len(), k.open_answer(&at_edge, "abc").unwrap().status), (MAX_MESSAGE, 200));
    }
    #[test]
    fn same_words_never_seal_the_same_way() {
        let k = Keys::derive(&new_secret()).unwrap();
        assert_ne!(k.seal(Direction::ToTower, b"yes"), k.seal(Direction::ToTower, b"yes"));
    }
    #[test]
    fn the_pull_token_is_not_the_channel_and_secrets_are_checked() {
        let k = Keys::derive(&new_secret()).unwrap();
        assert!(!k.pull_token.contains(&k.channel));
        assert!(Keys::derive("abc").is_err());
        assert!(Keys::derive(&"zz".repeat(32)).is_err());
    }
    #[test]
    fn a_pull_says_what_it_got_until_the_relay_takes_one_and_what_it_answers_until_the_answer_ends() {
        let mut p = Pulling::new();
        let said = |p: &Pulling, h: &str| p.headers().into_iter().find(|(k, _)| *k == h).map(|(_, v)| v);
        assert_eq!(said(&p, PULL_HEADER).as_deref(), Some(PULL_VERSION));
        assert!(said(&p, TOWER_HEADER).is_some_and(|t| t.len() == 16) && said(&p, GOT_HEADER).is_none());
        // two towers that start with one pairing are told apart
        assert_ne!(said(&p, TOWER_HEADER), said(&Pulling::new(), TOWER_HEADER));
        let k1 = p.handed("k1", true).unwrap();
        // a pull that fails tells the relay nothing: the next one says it again
        assert_eq!((said(&p, GOT_HEADER).as_deref(), said(&p, ANSWERING_HEADER).as_deref()), (Some("k1"), Some("k1")));
        p.taken();
        assert_eq!((said(&p, GOT_HEADER), said(&p, ANSWERING_HEADER).as_deref()), (None, Some("k1")));
        // a radio's request handed meanwhile does not end the kept one's answer
        assert_eq!(p.handed("r1", false), None);
        assert_eq!((said(&p, GOT_HEADER).as_deref(), said(&p, ANSWERING_HEADER).as_deref()), (Some("r1"), Some("k1")));
        assert!(p.answered(&k1), "the handing the pulls named is let go");
        assert_eq!(said(&p, ANSWERING_HEADER), None);
        // B-5: opened again while the work on k2 runs, it is still answered for; handed again meanwhile, the
        // first handing's work ending does not end the second's
        let k2 = p.handed("k2", true).unwrap();
        p.reopened();
        assert_eq!((said(&p, GOT_HEADER), said(&p, ANSWERING_HEADER).as_deref()), (None, Some("k2")));
        let k2_again = p.handed("k2", true).unwrap();
        assert!(!p.answered(&k2), "an earlier handing's work ending lets go of nothing");
        assert_eq!(said(&p, ANSWERING_HEADER).as_deref(), Some("k2"));
        assert!(p.answered(&k2_again));
        assert_eq!(said(&p, ANSWERING_HEADER), None);
    }
    #[test]
    fn a_pull_in_a_version_the_relay_does_not_take_says_which_side_is_older() {
        assert_eq!(PullRefused::version(Some(PULL_VERSION)), Ok(*PULL_VERSIONS.end()));
        assert_eq!(PullRefused::version(Some("2")), Ok(2));
        let older = |side| Err(PullRefused::OtherVersion { older: side });
        assert_eq!(PullRefused::version(None), older(Side::Tower));
        assert_eq!(PullRefused::version(Some("1")), older(Side::Tower));
        assert_eq!(PullRefused::version(Some("two")), older(Side::Tower));
        assert_eq!(PullRefused::version(Some("4")), older(Side::Relay));
        // on the wire, and a reason from a newer relay
        let told = serde_json::to_string(&PullRefused::OtherVersion { older: Side::Relay }).unwrap();
        assert_eq!(told, r#"{"refused":"other_version","older":"relay"}"#);
        assert_eq!(serde_json::from_str::<PullRefused>(r#"{"refused":"later"}"#).unwrap(), PullRefused::Unknown);
        assert_eq!((PullRefused::AnotherTower.status(), PullRefused::OtherVersion { older: Side::Tower }.status()), (409, 426));
    }
    #[test]
    fn a_request_with_no_schema_or_a_method_this_build_does_not_know_is_opened_to_be_answered_as_another_version() {
        // B-29: such a request failed to open and was dropped as one its key does not open
        let k = Keys::derive(&new_secret()).unwrap();
        let sealed = |json: &str| k.seal(Direction::ToTower, json.as_bytes());
        // no schema: it opens, saying the empty one, which the tower answers as another version
        let none = k.open_call(&sealed(r#"{"t":1,"id":"a","method":"POST","path":"/v1/radio/utterance"}"#)).expect("opens");
        assert_eq!(none.schema, "");
        // a method this build does not know: its id, to answer it under
        let newer = k.open_request(&sealed(&format!(r#"{{"t":1,"id":"b","method":"PATCH","path":"/v1/radio/x","schema":"{}"}}"#, crate::radio::SCHEMA))).expect("opens");
        assert!(matches!(newer, Err(Unread { id }) if id == "b"));
        let ours = k.open_request(&sealed(&format!(r#"{{"t":1,"id":"c","method":"GET","path":"/v1/radio/next","schema":"{}"}}"#, crate::radio::SCHEMA))).unwrap();
        assert!(ours.is_ok_and(|c| c.schema == crate::radio::SCHEMA));
        // bytes that are not a request at all, or another pairing's, open to nothing
        assert!(k.open_request(&sealed("[1]")).is_none());
        assert!(Keys::derive(&new_secret()).unwrap().open_request(&sealed(r#"{"id":"d"}"#)).is_none());
    }
    #[test]
    fn a_relays_refusal_of_a_radios_request_is_read_by_its_body_and_its_status_is_agreed_once() {
        for why in [CallRefused::Behind, CallRefused::Away, CallRefused::TowerSilent, CallRefused::Ended] {
            let body = serde_json::to_vec(&why).unwrap();
            assert_eq!(CallRefused::of(Route::Call, why.status(), &body), Err(Some(why)));
        }
        assert_eq!(serde_json::to_string(&CallRefused::TowerSilent).unwrap(), r#"{"refused":"tower_silent"}"#);
        assert_eq!((CallRefused::of(Route::Call, 200, b""), CallRefused::of(Route::Queue, 202, b"")), (Ok(()), Ok(())));
        // one about the request itself has no such body; one from a newer relay is a reason not known here
        assert_eq!(CallRefused::of(Route::Call, 429, b"too many requests"), Err(None));
        assert_eq!(CallRefused::of(Route::Queue, 409, br#"{"refused":"later"}"#), Err(Some(CallRefused::Unknown)));
    }
    #[test]
    fn freshness_window() {
        assert!(fresh(1_000_000, 1_000_000 + FRESH_MS));
        assert!(fresh(1_000_000 + 5, 1_000_000));
        assert!(!fresh(1_000_000, 1_000_001 + FRESH_MS));
    }
}
