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
pub use routes::Route;

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

/// On every pull: the version of the pull the tower speaks ([`PULL_VERSION`]). The relay refuses a pull without
/// it (426), since an older tower never says what it got.
pub const PULL_HEADER: &str = "x-airband-pull";
/// Pulls that say what they got ([`GOT_HEADER`]).
pub const PULL_VERSION: &str = "2";
/// On a pull: the id of the request the tower's previous pull was handed and the tower read. A tower pulls one at a
/// time, so a request handed over that the next pull does not name never reached it (the connection died on the
/// way), and the relay hands it over again. Nothing else tells the relay a request arrived.
pub const GOT_HEADER: &str = "x-airband-got";

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
    /// What a tower shows the relay it is this channel's tower with (`Authorization`).
    pub fn as_tower(&self) -> String {
        format!("Bearer {}", self.pull_token)
    }
    /// What a radio shows the relay it is this channel's radio with (`Authorization`).
    pub fn as_radio(&self) -> String {
        format!("Bearer {}", self.radio_token)
    }
    /// A request sealed for the tower.
    pub fn seal_call(&self, call: &Call) -> Vec<u8> {
        self.seal(Direction::ToTower, &serde_json::to_vec(call).unwrap_or_default())
    }
    /// A request as a radio sealed it. Nothing for bytes this pairing's key does not open or that are not one.
    pub fn open_call(&self, sealed: &[u8]) -> Option<Call> {
        serde_json::from_slice(&self.open(Direction::ToTower, sealed).ok()?).ok()
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
#[derive(Debug, Serialize, Deserialize)]
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
    /// derived, so two versions cannot read each other at all.)
    pub schema: String,
    /// Where the radio stands as it makes this request: on a call or not, showing the chat or not.
    #[serde(default)]
    pub presence: crate::radio::Presence,
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
    fn freshness_window() {
        assert!(fresh(1_000_000, 1_000_000 + FRESH_MS));
        assert!(fresh(1_000_000 + 5, 1_000_000));
        assert!(!fresh(1_000_000, 1_000_001 + FRESH_MS));
    }
}
