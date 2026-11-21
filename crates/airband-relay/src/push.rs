//! Waking a phone: handing a tower's sealed notification to a push service. The relay holds the service's key
//! and can read nothing of what it passes on; what the phone's owner sees before his phone has opened it is the
//! same few words for every notification.
//!
//! One push service so far, Apple's. Another is a struct beside [`Apns`] with a `send` of its own and an arm
//! in [`Push::send`]; nothing outside this file knows which services there are.
//!
//!   APNS_KEY       Apple's .p8 signing key, as the file's text
//!   APNS_KEY_ID    the key's id
//!   APNS_TEAM_ID   the developer team it belongs to
//!   APNS_TOPIC     the app's bundle id
//!   APNS_HOST      where to send (default https://api.push.apple.com; a development build needs the sandbox)
use airband_core::relay::{BannerEvent, Counts};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What became of one notification.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// The address is no longer good: the app was removed, or the address was never this service's.
    Gone(String),
    /// The service said no for another reason, in its own word for it.
    Refused(String),
    /// The service could not be reached, or did not answer.
    Failed(String),
}

/// Every push service this relay can use.
#[derive(Default)]
pub struct Push {
    apns: Option<Apns>,
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Push {
    pub fn from_env() -> Push {
        let apns = match (var("APNS_KEY"), var("APNS_KEY_ID"), var("APNS_TEAM_ID"), var("APNS_TOPIC")) {
            (Some(key), Some(key_id), Some(team), Some(topic)) => match Apns::new(&key, &key_id, &team, &topic, &var("APNS_HOST").unwrap_or_else(|| "https://api.push.apple.com".into())) {
                Ok(apns) => Some(apns),
                Err(e) => {
                    eprintln!("airband-relay: Apple's push key cannot be used: {e}");
                    None
                }
            },
            _ => None,
        };
        Push { apns }
    }
    #[cfg(test)]
    pub fn with_apns(apns: Apns) -> Push {
        Push { apns: Some(apns) }
    }
    pub fn describe(&self) -> &'static str {
        if self.apns.is_some() { "can wake iPhones (Apple's push service)" } else { "has no push service: phones are not woken" }
    }
    /// Hand a sealed notification to the service an address belongs to. Nothing when this relay has no such service.
    pub async fn send(&self, provider: &str, to: &str, sealed: &str, question: bool) -> Option<Outcome> {
        match provider {
            "apns" => Some(self.apns.as_ref()?.send(to, sealed, question).await),
            _ => None,
        }
    }
    /// Change a phone's banner through the service its address belongs to. Nothing when this relay has no such service.
    pub async fn banner(&self, provider: &str, to: &str, event: BannerEvent, counts: Counts, slot: u32, urgent: bool) -> Option<Outcome> {
        match provider {
            "apns" => Some(self.apns.as_ref()?.banner(to, event, counts, slot, urgent).await),
            _ => None,
        }
    }
}

/// Apple's push service, with a signing key (token-based authentication, HTTP/2).
pub struct Apns {
    key: EcdsaKeyPair,
    key_id: String,
    team: String,
    topic: String,
    host: String,
    http: reqwest::Client,
    /// The signed pass in use and when it was made. Apple wants the same one used for a while: it refuses one
    /// older than an hour, and a relay that makes a new one more often than every twenty minutes.
    pass: Mutex<Option<(String, Instant)>>,
}

const PASS_FOR: Duration = Duration::from_secs(40 * 60);

impl Apns {
    /// `key` is the text of the .p8 file: PKCS#8, base64, between its two marker lines.
    pub fn new(key: &str, key_id: &str, team: &str, topic: &str, host: &str) -> Result<Apns, String> {
        // however the lines were kept (an environment variable may hold them joined, or with "\n" written out)
        let body: String = key.replace("\\n", "\n").lines().filter(|l| !l.contains("-----")).flat_map(|l| l.chars()).filter(|c| !c.is_whitespace()).collect();
        let der = STANDARD.decode(body).map_err(|_| "it is not a .p8 key file's text")?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new()).map_err(|_| "it is not a P-256 signing key")?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).pool_idle_timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
        Ok(Apns { key, key_id: key_id.into(), team: team.into(), topic: topic.into(), host: host.trim_end_matches('/').into(), http, pass: Mutex::new(None) })
    }

    /// The pass Apple asks for with every notification: a JWT signed with the key (ES256).
    fn pass(&self) -> Result<String, String> {
        let mut pass = self.pass.lock().unwrap();
        if let Some((made, when)) = pass.as_ref() {
            if when.elapsed() < PASS_FOR {
                return Ok(made.clone());
            }
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let part = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let signed = format!("{}.{}", part(serde_json::json!({ "alg": "ES256", "kid": self.key_id })), part(serde_json::json!({ "iss": self.team, "iat": now })));
        let signature = self.key.sign(&SystemRandom::new(), signed.as_bytes()).map_err(|_| "the push key could not sign")?;
        let made = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()));
        *pass = Some((made.clone(), Instant::now()));
        Ok(made)
    }

    async fn send(&self, to: &str, sealed: &str, question: bool) -> Outcome {
        // what shows if the phone cannot open it (the app's extension did not run): the same words every time
        let body = serde_json::json!({
            "aps": { "alert": { "title": "airband", "body": if question { "A question for you" } else { "New message" } }, "sound": "default", "mutable-content": 1 },
            "sealed": sealed,
        })
        .to_string();
        // kept by Apple for a phone that is off: a question for a day, anything else for an hour
        let until = now_secs() + if question { 86_400 } else { 3600 };
        self.post(to, &self.topic, "alert", "10", until, body).await
    }

    /// Change a banner (a Live Activity). Its contents are not sealed: they are the three numbers and the fixed
    /// words below, and nothing a tower sends can add to them.
    async fn banner(&self, to: &str, event: BannerEvent, counts: Counts, slot: u32, urgent: bool) -> Outcome {
        let now = now_secs();
        let body = banner_body(event, counts, slot, now).to_string();
        // not kept for a phone that is off for long: by then the numbers are old
        self.post(to, &format!("{}.push-type.liveactivity", self.topic), "liveactivity", if urgent || event == BannerEvent::Start { "10" } else { "5" }, now + 900, body).await
    }

    async fn post(&self, to: &str, topic: &str, kind: &str, priority: &str, until: u64, body: String) -> Outcome {
        for attempt in 0..2 {
            let pass = match self.pass() {
                Ok(pass) => pass,
                Err(e) => return Outcome::Failed(e),
            };
            let sent = self
                .http
                .post(format!("{}/3/device/{to}", self.host))
                .header("authorization", format!("bearer {pass}"))
                .header("apns-topic", topic)
                .header("apns-push-type", kind)
                .header("apns-priority", priority)
                .header("apns-expiration", until.to_string())
                .header("content-type", "application/json")
                .body(body.clone())
                .send()
                .await;
            let res = match sent {
                Ok(res) => res,
                Err(e) => return Outcome::Failed(if e.is_timeout() { "Apple did not answer in time".into() } else { "Apple could not be reached".into() }),
            };
            let status = res.status().as_u16();
            if status == 200 {
                return Outcome::Sent;
            }
            let reason = res.json::<serde_json::Value>().await.ok().and_then(|v| v["reason"].as_str().map(String::from)).unwrap_or_else(|| format!("status {status}"));
            return match (status, reason.as_str()) {
                // the pass went stale between two notifications: make another and try once more
                (403, "ExpiredProviderToken") if attempt == 0 => {
                    *self.pass.lock().unwrap() = None;
                    continue;
                }
                (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => Outcome::Gone(reason),
                _ => Outcome::Refused(reason),
            };
        }
        Outcome::Refused("ExpiredProviderToken".into())
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The name of the banner in the app (`StatusAttributes` in apps/apple/Live), which Apple asks for when one is put up.
const BANNER: &str = "StatusAttributes";
/// What the phone shows for a moment when a banner is put up. The same words every time.
const BANNER_SAYS: &str = "An agent needs you";

/// What Apple is sent for a banner. Apple delivers this as it is, so it is built here from numbers only: the
/// words in it are the ones written in this file.
pub fn banner_body(event: BannerEvent, counts: Counts, slot: u32, now: u64) -> serde_json::Value {
    let state = serde_json::json!({ "waiting": counts.waiting, "working": counts.working, "unread": counts.unread });
    match event {
        // shown as out of date after an hour with no news, so old numbers are not taken for new ones
        BannerEvent::Update => serde_json::json!({ "aps": { "timestamp": now, "event": "update", "content-state": state, "stale-date": now + 3600 } }),
        BannerEvent::End => serde_json::json!({ "aps": { "timestamp": now, "event": "end", "content-state": state, "dismissal-date": now } }),
        BannerEvent::Start => serde_json::json!({ "aps": {
            "timestamp": now, "event": "start", "content-state": state, "stale-date": now + 3600,
            "attributes-type": BANNER, "attributes": { "slot": slot },
            "alert": { "title": "airband", "body": BANNER_SAYS },
        } }),
    }
}

/// How many notifications one channel may send, and in how long. A tower sends one for a question or a report,
/// to each phone paired with it; anything faster than this is not a person's agents.
pub const PUSHES: u32 = 60;
pub const PUSHES_IN: Duration = Duration::from_secs(600);

/// How many times one channel may change a banner in the same time. The numbers on it change more often than
/// he is asked something, and a tower already spaces them out.
pub const BANNERS: u32 = 90;

/// May this channel send another now, of a kind it may send `most` of? Counts it if so.
pub fn allow(slot: &mut Option<(Instant, u32)>, most: u32, now: Instant) -> bool {
    let (began, sent) = slot.filter(|(began, _)| now.duration_since(*began) < PUSHES_IN).unwrap_or((now, 0));
    if sent >= most {
        return false;
    }
    *slot = Some((began, sent + 1));
    true
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, KeyPair, UnparsedPublicKey};

    /// A signing key made for a test, as a .p8 file's text, and its public half.
    pub fn test_key() -> (String, Vec<u8>) {
        let made = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new()).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, made.as_ref(), &SystemRandom::new()).unwrap();
        let text = STANDARD.encode(made.as_ref());
        // as the file has it: marker lines, and the base64 broken into lines
        let lines: Vec<&str> = text.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
        (format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n")), pair.public_key().as_ref().to_vec())
    }

    /// Is this the pass Apple would accept: the right fields, signed by the key?
    pub fn pass_is_good(pass: &str, public: &[u8], key_id: &str, team: &str) -> bool {
        let parts: Vec<&str> = pass.split('.').collect();
        let [head, claims, signature] = parts.as_slice() else { return false };
        let read = |part: &str| URL_SAFE_NO_PAD.decode(part).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).unwrap_or_default();
        let (head_json, claims_json) = (read(head), read(claims));
        let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else { return false };
        head_json["alg"] == "ES256" && head_json["kid"] == key_id && claims_json["iss"] == team && claims_json["iat"].as_u64().is_some() && UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public).verify(format!("{head}.{claims}").as_bytes(), &signature).is_ok()
    }

    #[test]
    fn the_pass_is_signed_by_the_key_and_used_again() {
        let (key, public) = test_key();
        let apns = Apns::new(&key, "KEY123", "TEAM456", "dev.example.app", "http://127.0.0.1:1").unwrap();
        let pass = apns.pass().unwrap();
        assert!(pass_is_good(&pass, &public, "KEY123", "TEAM456"));
        assert!(!pass_is_good(&pass, &test_key().1, "KEY123", "TEAM456"));
        assert_eq!(apns.pass().unwrap(), pass);
        // the key's text with its lines joined by a written-out "\n", as an environment variable may hold it
        assert!(Apns::new(&key.replace('\n', "\\n"), "K", "T", "t", "http://x").is_ok());
        assert!(Apns::new("not a key", "K", "T", "t", "http://x").is_err());
    }

    #[test]
    fn a_channel_sends_so_many_and_then_waits() {
        let (mut slot, t0) = (None, Instant::now());
        assert!((0..PUSHES).all(|_| allow(&mut slot, PUSHES, t0)) && !allow(&mut slot, PUSHES, t0 + Duration::from_secs(1)));
        assert!(allow(&mut slot, PUSHES, t0 + PUSHES_IN));
    }

    /// Apple is handed a banner unsealed. Every piece of text in what it is handed is one of the words written
    /// in this file, whatever the numbers are; a field that carried anything else would show up here.
    #[test]
    fn a_banner_is_numbers_and_the_same_few_words() {
        fn text(v: &serde_json::Value, out: &mut Vec<String>) {
            match v {
                serde_json::Value::String(s) => out.push(s.clone()),
                serde_json::Value::Array(a) => a.iter().for_each(|v| text(v, out)),
                serde_json::Value::Object(o) => o.values().for_each(|v| text(v, out)),
                _ => {}
            }
        }
        let counts = Counts { waiting: 2, working: 5, unread: 1 };
        for (event, words) in [(BannerEvent::Update, vec!["update"]), (BannerEvent::End, vec!["end"]), (BannerEvent::Start, vec!["airband", BANNER_SAYS, BANNER, "start"])] {
            let body = banner_body(event, counts, 7, 1000);
            let mut found = Vec::new();
            text(&body, &mut found);
            found.sort();
            let mut words: Vec<String> = words.into_iter().map(String::from).collect();
            words.sort();
            assert_eq!(found, words, "{body}");
            assert_eq!(body["aps"]["content-state"], serde_json::json!({ "waiting": 2, "working": 5, "unread": 1 }));
        }
        assert_eq!(banner_body(BannerEvent::Start, counts, 7, 1000)["aps"]["attributes"], serde_json::json!({ "slot": 7 }));
    }

    /// Against Apple itself, for a key in a file: an address nobody has must come back as not good, which
    /// shows the key, the pass and the HTTP/2 connection all work.
    ///   APNS_KEY_PATH=… APNS_KEY_ID=… APNS_TEAM_ID=… APNS_TOPIC=… cargo test -p airband-relay apple_itself -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn apple_itself_refuses_an_address_nobody_has() {
        let key = std::fs::read_to_string(std::env::var("APNS_KEY_PATH").expect("APNS_KEY_PATH")).unwrap();
        let apns = Apns::new(&key, &std::env::var("APNS_KEY_ID").unwrap(), &std::env::var("APNS_TEAM_ID").unwrap(), &std::env::var("APNS_TOPIC").unwrap(), "https://api.push.apple.com").unwrap();
        let outcome = apns.send(&"ab".repeat(32), "c2VhbGVk", false).await;
        println!("Apple answered: {outcome:?}");
        assert_eq!(outcome, Outcome::Gone("BadDeviceToken".into()));
    }
}
