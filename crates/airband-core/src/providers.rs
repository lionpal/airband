//! Every outside service airband can use, one row each. The rows themselves are data: the files under
//! `catalog/providers/`, read by [`crate::catalog`]. This is what a row is made of and how one is used.
//!
//! A provider does up to three jobs: it reads (summaries and routing), it speaks, it listens. For each job a row
//! says which interface the service is reached through and what it has to be told. Where a common interface
//! exists, that is the one used, so most services are a row and no code:
//!
//! - reading: OpenAI's Chat Completions (`POST {url}/chat/completions`), which nearly every model service and
//!   every local model server accepts; or a command, for a model reached through a program on this computer.
//! - speaking: OpenAI's speech endpoint (`POST {url}/audio/speech`), which several services and most local
//!   voice servers accept. Services with something that endpoint lacks (word timings) keep an interface of
//!   their own.
//! - listening: for a finished recording, OpenAI's transcription endpoint (`POST {url}/audio/transcriptions`),
//!   which several services and most local speech servers accept. For speech while it is spoken there is no
//!   common interface, so each such service is a dialect the phone speaks (`airband-radio/src/ears.rs`),
//!   named here.
//!
//! Reading is done on his computer, and `airband setup` and `airband model` are driven by this table, as is
//! the call itself (`tower/judge.rs`). Speaking and listening are done on the phone and nowhere else, with a
//! key of his kept there: the phone reads the same table for what it can reach (`airband-radio/src/voices.rs`,
//! `ears.rs`, `hears.rs`). Nothing else names a provider. A setting in use is a [`Use`]: the row's id, a key, and whatever was set differently
//! from the row.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Job {
    Summary,
    Voice,
    Ears,
}

/// How a service is reached: the name of an interface, and what that interface has to be told about this
/// service that he is never asked (`with`). The interfaces themselves are code, in a table for each job:
/// reading in the tower (`tower/judge.rs`), speaking and listening in the phone core (`airband-radio`'s
/// `voices.rs` and `ears.rs`). Nothing here lists them: a program that meets a name it has no code for
/// leaves that offer out.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct By {
    pub interface: String,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub with: serde_json::Map<String, Value>,
}

impl By {
    /// Something the interface is told, as text. Empty when the row does not say.
    pub fn text(&self, name: &str) -> &str {
        self.with.get(name).and_then(Value::as_str).unwrap_or_default()
    }
    pub fn flag(&self, name: &str) -> bool {
        self.with.get(name).and_then(Value::as_bool).unwrap_or(false)
    }
    /// How this service is handed a key: the header it goes in and what stands before it there. Nothing when
    /// the row does not say; an interface that sends a key then has no way to, and says so.
    pub fn auth(&self) -> Option<Auth> {
        self.with.get("auth").and_then(|a| serde_json::from_value(a.clone()).ok())
    }
    /// The header a key goes in for this service and its value there, for sending with a request. Nothing
    /// for a service that is sent no key. Err when there is a key to send and the row does not say how:
    /// the caller says so in its own words.
    pub fn key_header(&self, key: &str) -> Result<Option<(String, String)>, Untold> {
        if key.is_empty() {
            return Ok(None);
        }
        self.auth().map(|a| Some((a.header.clone(), a.value(key)))).ok_or(Untold)
    }
    pub fn list(&self, name: &str) -> Vec<String> {
        self.with.get(name).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
    }
}

/// A row does not say how its service takes a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Untold;

/// How a key is presented to a service: `{ "header": "authorization", "prefix": "Bearer " }`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Auth {
    pub header: String,
    #[serde(default)]
    pub prefix: String,
}

impl Auth {
    /// The header's value for this key.
    pub fn value(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }
}

/// An interface a program has the code for: its name in a row's `by`, what else it can do that a row may
/// say it needs (`Offer::needs`), and what a row has to tell it (`By::with`).
pub struct Interface {
    pub name: &'static str,
    pub can: &'static [&'static str],
    pub told: &'static [&'static str],
}

impl Interface {
    /// Can this interface, as this build has it, do what the offer asks of it?
    pub fn fits(&self, offer: &Offer) -> bool {
        self.name == offer.by.interface && offer.needs.iter().all(|n| self.can.contains(&n.as_str())) && self.told.iter().all(|t| offer.by.with.contains_key(*t))
    }
}

/// Something a job has to be told beyond the key: the setting's name, the question, and the usual answer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Ask {
    pub field: String,
    pub question: String,
    #[serde(default)]
    pub usual: String,
    /// This setting is where the service is reached, so where a key of his is sent. A catalogue taken later
    /// cannot change the usual of one of these for a provider this build knows.
    #[serde(default, skip_serializing_if = "no")]
    pub reach: bool,
}

fn no(b: &bool) -> bool {
    !*b
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Offer {
    /// Its name among its provider's offers, and what a setting names beside the provider (`Use::offer`). A
    /// provider can do one job more than one way (two voice models, say): each is an offer of its own, and
    /// one key serves them all.
    pub id: String,
    pub job: Job,
    pub by: By,
    /// What this is called where he chooses it: the maker and the model ("Google Gemini 3.5 Flash-Lite").
    pub label: String,
    /// One line under that, for something that needs no key: where it runs and what it costs him.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// One of the few shown before the rest where he chooses. The others sit behind "More services".
    #[serde(default, skip_serializing_if = "no")]
    pub preferred: bool,
    /// It runs on his own computer: nothing leaves it, and it costs him nothing.
    #[serde(default, skip_serializing_if = "no")]
    pub local: bool,
    /// The coding tool whose sign-in on his computer this uses (`claude-code`, `codex`, `opencode`), when it
    /// uses one and so needs no key. It is offered to someone who uses that tool. Empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub login: String,
    /// What it costs him, in a few words, as its maker published it on the day `priced` and worked out for
    /// this use (a summary is about 650 tokens in and 80 out; speech is about 900 characters a minute). Empty
    /// when it costs nothing, or when the maker's price could not be confirmed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub price: String,
    /// The day that price was read from its maker (`2026-11-20`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub priced: String,
    /// For a voice: a few of its voices to choose from, the usual one first. The one chosen is the setting
    /// `voice`. Empty when there is only the setting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voices: Vec<Voice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<Ask>,
    /// What the interface must be able to do, beyond being there, for this offer to work (`brief`: the
    /// command takes a brief file). A build whose interface cannot leaves the offer out, where otherwise it
    /// would call the service the old way.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
}

/// One of the voices a voice service speaks in.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Voice {
    /// What the service calls it: the value of the setting `voice`.
    pub id: String,
    pub name: String,
    /// How it sounds, in the maker's own words, and its accent. Empty when the maker says nothing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// A clip of it that its maker publishes, to hear before choosing. Empty when there is none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sample: String,
}

/// How a key is tested when he gives it, before it is kept: one request to the service that costs nothing
/// and that a wrong key fails. Part of the provider's row, beside what its key is called.
///
/// `url` may name settings of the offer he is choosing in braces (`{url}/models`, `https://{region}.…`): they
/// are filled with what he set, or the row's usual. What the answer means is said by its status alone, in the
/// lists here. Nothing the service writes in its answer is read.
/// The ways a key check is asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    /// Sent with an empty body.
    Post,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Check {
    /// How it is asked. A row naming a method this build does not know does not read.
    pub method: Method,
    pub url: String,
    /// How the key is presented for this request.
    pub auth: Auth,
    /// Anything else the service must be sent with it (a version header).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The statuses that mean the key is good.
    pub good: Vec<u16>,
    /// Those that mean it is not a key the service knows.
    pub wrong_key: Vec<u16>,
    /// Those that mean the service knows the key and will not serve it: no credit, not allowed, too many
    /// requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<u16>,
    /// This request is one the service charges for: it has none that is free and needs the key. He is told
    /// one was made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paid: bool,
}

/// Why a service did not do what it was asked: the same whether a key of his was being tested or a model was
/// being asked to read. What he is told about each is written here and nowhere else.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Trouble {
    /// It could not be reached.
    Unreachable,
    /// It does not know the key, or will not take it (401, 403).
    WrongKey,
    /// It knows the key and will not serve it: out of credit, over a limit, not allowed (402, 429).
    Refused,
    /// It does not know the model, or is not at the address it was asked at (404).
    NotFound,
    /// It answered with a status nothing here accounts for.
    Unexpected { status: u16 },
    /// It answered, with nothing that can be used.
    NoAnswer,
    /// It is a program on his computer, and did not start or stopped with an error.
    Program,
    /// It took longer than it is given.
    TooSlow,
    /// Nothing is chosen, or what is chosen is not something this version can use.
    NotSet,
}

impl Trouble {
    /// What a status says went wrong. A service's own row says which of its statuses mean a wrong key and
    /// which a refusal (`check`); for anything the row does not list, and for a service with no such row, the
    /// status is read as HTTP itself defines it.
    pub fn of_status(status: u16, check: Option<&Check>) -> Trouble {
        if let Some(check) = check {
            if check.wrong_key.contains(&status) {
                return Trouble::WrongKey;
            }
            if check.refused.contains(&status) {
                return Trouble::Refused;
            }
        }
        match status {
            401 | 403 => Trouble::WrongKey,
            402 | 429 => Trouble::Refused,
            404 => Trouble::NotFound,
            _ => Trouble::Unexpected { status },
        }
    }
    /// What happened, for him. `service` is what the service is called where he chose it.
    pub fn said(self, service: &str) -> String {
        let service = if service.is_empty() { "The service" } else { service };
        match self {
            Trouble::Unreachable => format!("{service} could not be reached."),
            Trouble::WrongKey => format!("{service} did not accept the key."),
            Trouble::Refused => format!("{service} knows the key and would not serve it. The account may be out of credit, or not allowed to use this."),
            Trouble::NotFound => format!("{service} does not have that model at that address."),
            Trouble::Unexpected { status } => format!("{service} gave an answer airband did not expect (status {status})."),
            Trouble::NoAnswer => format!("{service} answered with nothing that could be used."),
            Trouble::Program => format!("{service} did not run on your computer."),
            Trouble::TooSlow => format!("{service} took too long to answer."),
            Trouble::NotSet => "Nothing that this version of airband can use is chosen.".to_string(),
        }
    }
    /// The same when it was a key of his being tested: what happened, and what to do about the key.
    pub fn said_of_key(self, service: &str) -> String {
        let then = match self {
            Trouble::WrongKey => "Check it and paste it again.",
            Trouble::Unreachable => "Check your connection and try again.",
            _ => "The key was not kept.",
        };
        format!("{} {then}", self.said(service))
    }
}

/// What testing a key came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCheck {
    /// The service took it. `paid`: the request was one it charges for.
    Good { paid: bool },
    /// The service does not know this key.
    WrongKey,
    /// The service knows the key and will not serve it.
    Refused,
    /// The service could not be reached, so nothing is known about the key.
    Unreachable,
    /// The service answered with a status the row does not account for.
    Unexpected { status: u16 },
}

impl KeyCheck {
    /// What went wrong, when the key did not pass.
    pub fn trouble(self) -> Option<Trouble> {
        match self {
            KeyCheck::Good { .. } => None,
            KeyCheck::WrongKey => Some(Trouble::WrongKey),
            KeyCheck::Refused => Some(Trouble::Refused),
            KeyCheck::Unreachable => Some(Trouble::Unreachable),
            KeyCheck::Unexpected { status } => Some(Trouble::Unexpected { status }),
        }
    }
}

/// An address from a row, with the settings it names in braces filled in (`https://{region}.example/v1`,
/// `…/models/{model}:generate`, `{url}/models`). This is the one place a setting becomes part of an address
/// a key is sent to, so what may go in is narrow:
///
/// - A setting inside an address is letters, digits, `.`, `-`, `_` or `:`: nothing that could end a host
///   name or a path and lead a key somewhere else.
/// - A setting the address starts with is the whole address of a service (`{url}/models`): an `http://` or
///   `https://` address with a host and a path and nothing else, no login, question mark or fragment.
///
/// Err says which setting is not that, in words for him.
pub fn fill(template: &str, setting: &dyn Fn(&str) -> String) -> Result<String, String> {
    let part = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':');
    let mut out = String::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        let (field, after) = after.split_once('}').ok_or("An address in the catalogue is not well formed.")?;
        let value = setting(field).trim().to_string();
        if out.is_empty() && before.is_empty() {
            let value = value.trim_end_matches('/');
            // a whole web address with no login, question or fragment, made only of characters an address part may have
            let whole = crate::links::web(value).filter(|u| u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none());
            let plain = value.bytes().all(|b| part(b) || matches!(b, b'/' | b'[' | b']'));
            if whole.is_none() || !plain {
                return Err(format!("\"{value}\" is not something the setting {field} can be. It is the address of a service, like the usual one."));
            }
            out.push_str(value);
        } else {
            if value.is_empty() || !value.bytes().all(part) {
                return Err(format!("\"{value}\" is not something the setting {field} can be. It is letters and digits, like the usual one."));
            }
            out.push_str(before);
            out.push_str(&value);
        }
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}

impl Check {
    /// The address to ask, with the settings filled in ([`fill`]). Nothing when it names one that is not set,
    /// or one whose value cannot go in an address.
    pub fn address(&self, settings: &BTreeMap<String, String>) -> Option<String> {
        fill(&self.url, &|field| settings.get(field).cloned().unwrap_or_default()).ok()
    }

    /// What a status means.
    pub fn read(&self, status: u16) -> KeyCheck {
        if self.good.contains(&status) {
            KeyCheck::Good { paid: self.paid }
        } else if self.wrong_key.contains(&status) {
            KeyCheck::WrongKey
        } else if self.refused.contains(&status) {
            KeyCheck::Refused
        } else {
            KeyCheck::Unexpected { status }
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Provider {
    /// Its name in the settings (`"kind"`), on the command line, and the usual name for its key.
    pub id: String,
    pub name: String,
    pub key: bool,
    /// What its key is called, for asking for it ("Gemini API key").
    pub key_name: String,
    /// Where its keys are made.
    pub keys_at: String,
    /// How its key is tested when he gives it. Every provider that takes a key has one.
    pub check: Option<Check>,
    pub offers: Vec<Arc<Offer>>,
    /// Offered when he chooses. Not so for what the tower sets for itself.
    pub offered: bool,
}

impl Offer {
    /// It runs on his own computer: nothing leaves it, and it costs him nothing.
    pub fn local(&self) -> bool {
        self.local
    }
}

/// Words that stand where a provider's id or a setting's name would (a choice on the phone, the fields of a
/// `Use` as written down). No provider, offer or setting may take one of these as its name.
pub const NOT_AN_ID: &[&str] = &["none", "kind", "offer", "key"];

impl Use {
    /// Every setting of a job as it stands, set or usual: what a phone is told beside its pass.
    pub fn settings(&self, job: Job) -> BTreeMap<String, String> {
        self.offer(job).map(|(_, o)| o.asks.iter().map(|a| (a.field.to_string(), self.get(job, &a.field))).collect()).unwrap_or_default()
    }
}

pub fn provider(id: &str) -> Option<Arc<Provider>> {
    crate::catalog::current().providers.iter().find(|p| p.id == id).cloned()
}

/// The offers for a job in the order to show them where he chooses: what runs on his own computer first,
/// then what needs no key (neither costs him anything more), then the preferred, then the rest. Within each,
/// by name.
pub fn in_order(job: Job) -> Vec<(Arc<Provider>, Arc<Offer>)> {
    let mut all: Vec<(Arc<Provider>, Arc<Offer>)> = offering(job).into_iter().flat_map(|p| p.offers.iter().filter(|o| o.job == job).map(|o| (p.clone(), o.clone())).collect::<Vec<_>>()).collect();
    all.sort_by(|(p, o), (q, r)| (!o.local(), p.key, !o.preferred, o.label.as_str()).cmp(&(!r.local(), q.key, !r.preferred, r.label.as_str())));
    all
}

/// The providers he can choose for a job. Every one the catalogue has: a program lists the ones whose
/// interface it has the code for.
pub fn offering(job: Job) -> Vec<Arc<Provider>> {
    crate::catalog::current().providers.iter().filter(move |p| p.offered && p.offers.iter().any(|o| o.job == job)).cloned().collect()
}

/// A provider in use for one job: which row and which of its offers, with what key, and whatever is set
/// differently from the offer. Written in the settings as `{"kind": …, "offer": …, "key": …, …}`. The key is
/// the provider's: the same one whichever of its offers is used.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Use {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(flatten)]
    pub set: BTreeMap<String, Value>,
}



impl Use {
    /// This offer of this provider, with no key and everything as usual.
    pub fn of(kind: &str, offer: &str) -> Use {
        Use { kind: kind.into(), offer: offer.into(), key: String::new(), set: BTreeMap::new() }
    }
    /// It names no provider the catalogue has: nothing is set, or what was set is a row this build lacks.
    pub fn is_none(&self) -> bool {
        provider(&self.kind).is_none()
    }
    /// The row for this job, and how the job is done. Nothing when the settings name a provider or an offer
    /// this build does not have, or one that does not do this job.
    pub fn offer(&self, job: Job) -> Option<(Arc<Provider>, Arc<Offer>)> {
        let p = provider(&self.kind)?;
        let o = p.offers.iter().find(|o| o.job == job && o.id == self.offer).cloned()?;
        Some((p, o))
    }
    /// A setting for this job: what was set, else what the row usually uses.
    pub fn get(&self, job: Job, field: &str) -> String {
        match self.set.get(field) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => self.offer(job).and_then(|(_, o)| o.asks.iter().find(|a| a.field == field).map(|a| a.usual.clone())).unwrap_or_default(),
        }
    }
    /// One line for him: the provider and what it is set to. Never the key.
    pub fn describe(&self, job: Job) -> String {
        match self.offer(job) {
            None if self.kind.is_empty() => "none".into(),
            None => format!("\"{} {}\", which this version of airband does not have", self.kind, self.offer),
            Some((_, o)) => {
                let told: Vec<String> = o.asks.iter().map(|a| format!("{} {}", a.question.to_lowercase(), self.get(job, &a.field))).collect();
                if told.is_empty() { o.label.clone() } else { format!("{} ({})", o.label, told.join(", ")) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_goes_into_an_address_only_if_it_cannot_lead_a_key_elsewhere() {
        let set = |pairs: &'static [(&'static str, &'static str)]| move |field: &str| pairs.iter().find(|(k, _)| *k == field).map(|(_, v)| v.to_string()).unwrap_or_default();
        assert_eq!(fill("https://{region}.example/v1", &set(&[("region", "eastus")])).unwrap(), "https://eastus.example/v1");
        assert_eq!(fill("https://x.example/models/{model}:generate", &set(&[("model", "flash-2.5")])).unwrap(), "https://x.example/models/flash-2.5:generate");
        // a setting that would end the host and start another, or is not set at all
        for bad in ["evil.example/", "a@b", "a b", "a?b", "a#b", ""] {
            assert!(fill("https://{region}.example/v1", &move |_| bad.to_string()).is_err(), "{bad}");
        }
        // a setting the address starts with is a whole address, and nothing but one
        assert_eq!(fill("{url}/models", &set(&[("url", "https://api.example/v1/")])).unwrap(), "https://api.example/v1/models");
        assert_eq!(fill("{url}/models", &set(&[("url", "http://127.0.0.1:8080")])).unwrap(), "http://127.0.0.1:8080/models");
        for bad in ["api.example/v1", "ftp://api.example", "https://", "https://user@api.example", "https://api.example/v1?x=", "https://api.example/#", "https://a b", ""] {
            assert!(fill("{url}/models", &move |_| bad.to_string()).is_err(), "{bad}");
        }
        assert!(fill("https://{region.example", &set(&[])).is_err());
        // the key check asks only an address that passes
        let check = |url: &str| serde_json::from_value::<Check>(serde_json::json!({ "method": "GET", "url": url, "auth": { "header": "authorization", "scheme": "Bearer" }, "good": [200], "wrong_key": [401] })).unwrap();
        assert_eq!(check("{url}/models").address(&BTreeMap::from([("url".to_string(), "https://api.example/v1".to_string())])).as_deref(), Some("https://api.example/v1/models"));
        assert_eq!(check("{url}/models").address(&BTreeMap::from([("url".to_string(), "https://api.example@evil.example".to_string())])), None);
        assert_eq!(check("{url}/models").address(&BTreeMap::new()), None);
    }

    #[test]
    fn a_setting_names_an_offer_and_reads_what_was_set_or_the_usual() {
        let v: Use = serde_json::from_str(r#"{"kind":"elevenlabs","offer":"flash","key":"SECRETKEY","voice":"abc","model":"eleven_x"}"#).unwrap();
        assert_eq!((v.get(Job::Voice, "voice"), v.get(Job::Voice, "model"), v.key.as_str()), ("abc".into(), "eleven_x".into(), "SECRETKEY"));
        // what was not set is the offer's usual
        let e: Use = serde_json::from_str(r#"{"kind":"azure","offer":"speech-to-text","key":"k","region":"japaneast"}"#).unwrap();
        assert_eq!((e.get(Job::Ears, "region"), e.get(Job::Ears, "language")), ("japaneast".into(), "en-US".into()));
        // nothing set
        assert!(Use::default().is_none() && serde_json::from_str::<Use>(r#"{"kind":"no-such-row"}"#).unwrap().is_none());
        // a command of his own keeps its arguments, and round-trips as it was written
        let c: Use = serde_json::from_str(r#"{"kind":"command","offer":"own","argv":["mine","-x"]}"#).unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"kind":"command","offer":"own","argv":["mine","-x"]}"#);
        // the key is never part of what is said about it
        assert!(!v.describe(Job::Voice).contains("SECRETKEY"));
        assert_eq!(e.describe(Job::Ears), "Microsoft Azure Speech (region japaneast, language en-US)");
        // an offer is named, never guessed: a provider with no offer named does nothing, nor does an offer
        // asked to do another job
        assert!(Use::of("azure", "").offer(Job::Ears).is_none() && e.offer(Job::Voice).is_none());
    }
}
