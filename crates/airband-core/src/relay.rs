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
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "airband.relay/1";
/// A request older or newer than this, by the tower's clock, is refused.
pub const FRESH_MS: u64 = 120_000;
/// The largest sealed message a relay passes.
pub const MAX_MESSAGE: usize = 256 * 1024;

/// A request the relay kept while the tower was away is handed over under an id that starts with this, so the
/// tower knows it is old on purpose.
pub const HELD: &str = "held-";
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

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the system has no random source");
    hex(&buf)
}
/// A new pairing secret: 32 random bytes, as hex.
pub fn new_secret() -> String {
    random_hex(32)
}
/// What the relay stores for a pull token.
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
        if sealed.len() < 12 + 16 {
            return Err("too short to be a sealed message".into());
        }
        let (nonce, body) = sealed.split_at(12);
        self.cipher(dir).decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: &self.bound(dir) }).map_err(|_| "not sealed with this pairing's key".to_string())
    }
}

/// What the radio seals and sends: one `airband.radio/1` request.
#[derive(Debug, Serialize, Deserialize)]
pub struct Call {
    /// When it was made, in milliseconds since 1970.
    pub t: u64,
    /// Random; the tower answers each id once.
    pub id: String,
    pub method: String,
    /// Under `/v1/radio/`.
    pub path: String,
    #[serde(default)]
    pub body: serde_json::Value,
    /// The radio protocol the request is in (`airband.radio/1`). A tower that does not speak it says so, in
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

/// `POST /v1/channels/<channel>/push`, from the channel's tower: have a push service wake the phone. The relay
/// passes `sealed` on as it is and can read none of it; the phone opens it with the pairing's key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    /// Which push service the address belongs to ("apns").
    pub provider: String,
    /// The phone's address at that service, as the phone gave it to the tower.
    pub to: String,
    /// A notification sealed for [`Direction::Push`], base64.
    pub sealed: String,
    /// It is a question to him. All the relay and the push service learn about what is inside.
    #[serde(default)]
    pub question: bool,
}

/// The relay's answer to a [`PushRequest`]. The HTTP status says the same in a number: 200 sent, 410 the address
/// is no longer good, 429 too many, 503 this relay has no such push service, 502 the service refused or failed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushAnswer {
    pub sent: bool,
    /// The phone no longer takes notifications at that address: the tower forgets it.
    #[serde(default)]
    pub gone: bool,
    /// Why not, in the push service's own word for it or the relay's.
    #[serde(default)]
    pub reason: String,
}

/// The largest sealed notification, in bytes before base64: what fits in one push with room for the rest.
pub const PUSH_MOST: usize = 2400;

/// What a banner on the phone's lock screen says while the app is asleep: three numbers and nothing else.
///
/// A push service delivers a banner's contents as it is given them, not sealed, so this is everything a relay
/// and a push service ever see of what his agents are doing. It must stay numbers: never a name, a project, a
/// sentence or an agent's id. A test below fails if a field that can hold text is added.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// How many agents are waiting for him.
    pub waiting: u32,
    /// How many are working.
    pub working: u32,
    /// How many reports he has not had in front of him.
    pub unread: u32,
}

/// What a tower asks to be done with a phone's banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerEvent {
    /// Put one up (the phone has none from this computer).
    Start,
    /// Change the numbers on the one that is up.
    Update,
    /// Take it down.
    End,
}

/// `POST /v1/channels/<channel>/banner`, from the channel's tower: have a push service change the banner on the
/// phone. Answered with a [`PushAnswer`], as a notification is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BannerRequest {
    /// Which push service the address belongs to ("apns").
    pub provider: String,
    /// The banner's address at that service, as the phone gave it to the tower: for [`BannerEvent::Start`] the
    /// address that puts one up, otherwise the address of the one that is up.
    pub to: String,
    pub event: BannerEvent,
    pub counts: Counts,
    /// More agents are waiting than before: worth lighting the screen for. Anything else arrives quietly.
    #[serde(default)]
    pub urgent: bool,
}

/// The number a banner carries to say which of his computers it is for: the phone may be paired with several,
/// and each keeps its own banner. Taken from the channel's name, which the relay already knows and which says
/// nothing about him. The phone works out the same number for each pairing.
pub fn banner_slot(channel: &str) -> u32 {
    channel.get(..8).and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0)
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
    fn same_words_never_seal_the_same_way() {
        let k = Keys::derive(&new_secret()).unwrap();
        assert_ne!(k.seal(Direction::ToTower, b"yes"), k.seal(Direction::ToTower, b"yes"));
    }
    /// A banner's numbers go through a push service unsealed. Nothing that can hold words may be among them.
    #[test]
    fn what_a_banner_carries_is_numbers_and_can_hold_no_text() {
        // a type with a String, a Vec or a map in it is not Copy: adding one stops this compiling
        fn only_plain_values<T: Copy>() {}
        only_plain_values::<Counts>();
        only_plain_values::<BannerEvent>();
        // and as it is written out, every value is a number
        let written = serde_json::to_value(Counts { waiting: u32::MAX, working: u32::MAX, unread: u32::MAX }).unwrap();
        let fields = written.as_object().unwrap();
        let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, ["unread", "waiting", "working"]);
        assert!(fields.values().all(|v| v.is_u64()), "a banner's contents must be numbers: {written}");
        // the whole request: the only text in it is the service's name, the address and the event's own word
        let ask = serde_json::to_value(BannerRequest { provider: "apns".into(), to: "abc".into(), event: BannerEvent::Update, counts: Counts::default(), urgent: true }).unwrap();
        let mut text: Vec<&str> = ask.as_object().unwrap().iter().filter(|(_, v)| v.is_string()).map(|(k, _)| k.as_str()).collect();
        text.sort();
        assert_eq!(text, ["event", "provider", "to"]);
        assert_eq!(ask["event"], "update");
    }

    #[test]
    fn a_banner_slot_is_the_same_number_on_both_sides() {
        let k = Keys::derive(&new_secret()).unwrap();
        assert_eq!(banner_slot(&k.channel), u32::from_str_radix(&k.channel[..8], 16).unwrap());
        assert_eq!(banner_slot("zz"), 0);
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
