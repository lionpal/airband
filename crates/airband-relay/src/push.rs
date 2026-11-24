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
#![expect(clippy::disallowed_macros, reason = "Apple's own token and notification shapes")]
use airband_core::relay::{PushRequest, PushService};
use airband_core::encode::base64_url;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What became of one notification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Sent,
    /// The address is no longer good: the app was removed, or the address was never this service's.
    Gone,
    /// The service said no for another reason.
    Refused(Said),
    /// It never got to the service.
    Failed(Unsent),
}

/// What a push service said when it refused one: its status, and the reason it gave in its own answer, if any.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Said {
    pub(crate) status: u16,
    pub(crate) reason: Option<String>,
}
impl std::fmt::Display for Said {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.reason {
            Some(reason) => write!(f, "the push service said no ({reason}, status {})", self.status),
            None => write!(f, "the push service said no (status {})", self.status),
        }
    }
}

/// Why a notification never got to the service.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unsent {
    /// The relay's own key could not sign the pass the service asks for.
    CannotSign,
    TimedOut,
    Unreachable,
}
impl Unsent {
    /// What the tower is told.
    pub(crate) fn told(&self) -> airband_core::relay::NotPushed {
        use airband_core::relay::NotPushed;
        match self {
            Unsent::CannotSign => NotPushed::CannotSign,
            Unsent::TimedOut => NotPushed::TimedOut,
            Unsent::Unreachable => NotPushed::Unreachable,
        }
    }
}
impl std::fmt::Display for Unsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unsent::CannotSign => "the push key could not sign",
            Unsent::TimedOut => "the push service did not answer in time",
            Unsent::Unreachable => "the push service could not be reached",
        })
    }
}

/// Every push service this relay can use.
#[derive(Default)]
pub(crate) struct Push {
    apns: Option<Apns>,
}

use super::var;

impl Push {
    pub(crate) fn from_env() -> Push {
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
    pub(crate) fn with_apns(apns: Apns) -> Push {
        Push { apns: Some(apns) }
    }
    pub(crate) fn describe(&self) -> &'static str {
        if self.apns.is_some() { "can wake iPhones (Apple's push service)" } else { "has no push service: phones are not woken" }
    }
    /// Hand a sealed notification to the service an address belongs to. Nothing when this relay has no such service.
    pub(crate) async fn send(&self, ask: &PushRequest) -> Option<Outcome> {
        match ask.provider {
            PushService::Apns => Some(self.apns.as_ref()?.send(ask).await),
        }
    }
}

/// What Apple is handed for a notification, and until when it keeps it for a phone that is off. What shows if
/// the phone cannot open it (the app's extension did not run) is the same few words every time.
fn apple_body(ask: &PushRequest, now: u64) -> (String, u64) {
    let body = if ask.quiet {
        "An earlier message is out of date"
    } else if ask.question {
        "An agent needs you"
    } else {
        "New message"
    };
    let mut aps = serde_json::json!({ "alert": { "title": "airband", "body": body }, "mutable-content": 1 });
    if ask.quiet {
        // shown, if the phone cannot open it, without lighting the screen or a sound
        aps["interruption-level"] = "passive".into();
    } else {
        aps["sound"] = "default".into();
    }
    // kept by Apple for a phone that is off: a question for a day, anything else for an hour, and never past
    // the time the tower says it is worth something
    let kept = now + if ask.question { 86_400 } else { 3600 };
    let until = ask.until.map_or(kept, |u| u.min(kept));
    (serde_json::json!({ "aps": aps, "sealed": ask.sealed }).to_string(), until)
}

/// The header that has Apple keep only the newest notification under one id.
fn collapse(ask: &PushRequest) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(id) = ask.replaces.as_deref().and_then(|r| reqwest::header::HeaderValue::from_str(r).ok()) {
        headers.insert("apns-collapse-id", id);
    }
    headers
}

/// Apple's push service, with a signing key (token-based authentication, HTTP/2).
pub(crate) struct Apns {
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
    pub(crate) fn new(key: &str, key_id: &str, team: &str, topic: &str, host: &str) -> Result<Apns, String> {
        // however the lines were kept (an environment variable may hold them joined, or with "\n" written out)
        let text = key.replace("\\n", "\n");
        let der = rustls_pemfile::pkcs8_private_keys(&mut text.as_bytes()).next().and_then(Result::ok).ok_or("it is not a .p8 key file's text")?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, der.secret_pkcs8_der(), &SystemRandom::new()).map_err(|_| "it is not a P-256 signing key")?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).pool_idle_timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
        Ok(Apns { key, key_id: key_id.into(), team: team.into(), topic: topic.into(), host: host.trim_end_matches('/').into(), http, pass: Mutex::new(None) })
    }

    /// The pass Apple asks for with every notification: a JWT signed with the key (ES256).
    fn pass(&self) -> Result<String, Unsent> {
        let mut pass = self.pass.lock().unwrap();
        if let Some((made, when)) = pass.as_ref()
            && when.elapsed() < PASS_FOR {
                return Ok(made.clone());
            }
        let now = airband_core::clock::now_secs();
        let part = |v: serde_json::Value| base64_url(v.to_string().as_bytes());
        let signed = format!("{}.{}", part(serde_json::json!({ "alg": "ES256", "kid": self.key_id })), part(serde_json::json!({ "iss": self.team, "iat": now })));
        let signature = self.key.sign(&SystemRandom::new(), signed.as_bytes()).map_err(|_| Unsent::CannotSign)?;
        let made = format!("{signed}.{}", base64_url(signature.as_ref()));
        *pass = Some((made.clone(), Instant::now()));
        Ok(made)
    }

    async fn send(&self, ask: &PushRequest) -> Outcome {
        let to = &ask.to;
        let (body, until) = apple_body(ask, airband_core::clock::now_secs());
        let mut tried_again = false;
        loop {
            let pass = match self.pass() {
                Ok(pass) => pass,
                Err(e) => return Outcome::Failed(e),
            };
            let sent = self
                .http
                .post(format!("{}/3/device/{to}", self.host))
                .header("authorization", format!("bearer {pass}"))
                .header("apns-topic", &self.topic)
                .header("apns-push-type", "alert")
                .header("apns-priority", "10")
                .header("apns-expiration", until.to_string())
                .header("content-type", "application/json")
                .headers(collapse(ask))
                .body(body.clone())
                .send()
                .await;
            let res = match sent {
                Ok(res) => res,
                Err(e) => return Outcome::Failed(if e.is_timeout() { Unsent::TimedOut } else { Unsent::Unreachable }),
            };
            let status = res.status().as_u16();
            if status == 200 {
                return Outcome::Sent;
            }
            // Apple's answer names its reason in a field of its own, from a list it publishes
            let reason = res.json::<serde_json::Value>().await.ok().and_then(|v| v["reason"].as_str().map(String::from));
            return match (status, reason.as_deref()) {
                // the pass went stale between two notifications: make another and try once more
                (403, Some("ExpiredProviderToken")) if !tried_again => {
                    *self.pass.lock().unwrap() = None;
                    tried_again = true;
                    continue;
                }
                (410, _) | (400, Some("BadDeviceToken" | "DeviceTokenNotForTopic")) => Outcome::Gone,
                _ => Outcome::Refused(Said { status, reason }),
            };
        }
    }
}

/// How many notifications one channel may send, and in how long. A tower sends one for a question or a report,
/// to each phone paired with it; anything faster than this is not a person's agents.
pub(crate) const PUSHES: u32 = 60;
pub(crate) const PUSHES_IN: Duration = Duration::from_secs(600);

/// May this channel send another now? Counts it if so.
pub(crate) fn allow(slot: &mut Option<(Instant, u32)>, now: Instant) -> bool {
    super::limits::count(slot, PUSHES, PUSHES_IN, now)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, KeyPair, UnparsedPublicKey};

    /// A signing key made for a test, as a .p8 file's text, and its public half.
    pub(crate) fn test_key() -> (String, Vec<u8>) {
        let made = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new()).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, made.as_ref(), &SystemRandom::new()).unwrap();
        let text = airband_core::encode::base64(made.as_ref());
        // as the file has it: marker lines, and the base64 broken into lines
        let lines: Vec<&str> = text.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
        (format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n")), pair.public_key().as_ref().to_vec())
    }

    /// Is this the pass Apple would accept: the right fields, signed by the key?
    pub(crate) fn pass_is_good(pass: &str, public: &[u8], key_id: &str, team: &str) -> bool {
        let parts: Vec<&str> = pass.split('.').collect();
        let [head, claims, signature] = parts.as_slice() else { return false };
        let read = |part: &str| airband_core::encode::unbase64_url(part).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).unwrap_or_default();
        let (head_json, claims_json) = (read(head), read(claims));
        let Some(signature) = airband_core::encode::unbase64_url(signature) else { return false };
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

    /// A notification that takes an earlier one down takes its place quietly, under the tag the tower gave; one about something with a deadline is not kept past it.
    #[test]
    fn what_apple_is_handed_follows_the_typed_request() {
        let ask = PushRequest { provider: PushService::Apns, to: "ab".into(), sealed: "c2VhbGVk".into(), question: true, replaces: None, quiet: false, until: None };
        let (body, until) = apple_body(&ask, 1000);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!((v["aps"]["sound"].as_str(), v["aps"]["alert"]["body"].as_str(), until), (Some("default"), Some("An agent needs you"), 1000 + 86_400));
        assert!(collapse(&ask).is_empty());
        let over = PushRequest { replaces: Some("c0ffee12".into()), quiet: true, until: Some(1100), ..ask };
        let (body, until) = apple_body(&over, 1000);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["aps"].get("sound").is_none(), "{v}");
        assert_eq!(v["aps"]["interruption-level"].as_str(), Some("passive"));
        assert_eq!((v["aps"]["alert"]["body"].as_str(), v["sealed"].as_str(), until), (Some("An earlier message is out of date"), Some("c2VhbGVk"), 1100));
        assert_eq!(collapse(&over).get("apns-collapse-id").unwrap(), "c0ffee12");
        // a replacing id that is not a short plain one is not passed on at all
        assert!(!PushRequest { replaces: Some("not plain!".into()), ..over.clone() }.well_formed());
        assert!(!PushRequest { replaces: Some("a".repeat(65)), ..over }.well_formed());
    }

    #[test]
    fn a_channel_sends_so_many_and_then_waits() {
        let (mut slot, t0) = (None, Instant::now());
        assert!((0..PUSHES).all(|_| allow(&mut slot, t0)) && !allow(&mut slot, t0 + Duration::from_secs(1)));
        assert!(allow(&mut slot, t0 + PUSHES_IN));
    }

    /// Against Apple itself, for a key in a file: an address nobody has must come back as not good, which
    /// shows the key, the pass and the HTTP/2 connection all work.
    ///   APNS_KEY_PATH=… APNS_KEY_ID=… APNS_TEAM_ID=… APNS_TOPIC=… cargo test -p airband-relay apple_itself -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn apple_itself_refuses_an_address_nobody_has() {
        let key = std::fs::read_to_string(std::env::var("APNS_KEY_PATH").expect("APNS_KEY_PATH")).unwrap();
        let apns = Apns::new(&key, &std::env::var("APNS_KEY_ID").unwrap(), &std::env::var("APNS_TEAM_ID").unwrap(), &std::env::var("APNS_TOPIC").unwrap(), "https://api.push.apple.com").unwrap();
        let ask = PushRequest { provider: PushService::Apns, to: "ab".repeat(32), sealed: "c2VhbGVk".into(), question: false, replaces: None, quiet: false, until: None };
        let outcome = apns.send(&ask).await;
        println!("Apple answered: {outcome:?}");
        assert_eq!(outcome, Outcome::Gone);
    }
}
