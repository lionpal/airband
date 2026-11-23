//! Every outside service airband can use, one row each. The rows themselves are data: the files under
//! `catalog/providers/`, read by [`crate::catalog`]. This is what a row is made of and how one is used.
//!
//! A provider does up to three jobs: it reads (summaries and routing), it speaks, it listens. For each job a row
//! says which interface the service is reached through and what it has to be told. Where a common interface
//! exists, that is the one used, so most services are a row and no code:
//!
//! - reading: the common chat-completions interface (`POST {url}/chat/completions`), which nearly every model service and
//!   every local model server accepts; or a command, for a model reached through a program on this computer.
//! - speaking: the common speech endpoint (`POST {url}/audio/speech`), which several services and most local
//!   voice servers accept. Services with something that endpoint lacks (word timings) keep an interface of
//!   their own.
//! - listening: for a finished recording, the common transcription endpoint (`POST {url}/audio/transcriptions`),
//!   which several services and most local speech servers accept. For speech while it is spoken there is no
//!   common interface, so each such service is a dialect the phone speaks (`airband-radio/src/ears.rs`),
//!   named here.
//!
//! Reading is done on his computer, and `airband setup` and `airband model` are driven by this table, as is
//! the call itself (the readers' table in `tower/judge/models.rs`). Speaking and listening are done on the phone and nowhere else, with a
//! key of his kept there: the phone reads the same table for what it can reach (`airband-radio/src/voices.rs`,
//! `ears.rs`, `hears.rs`). Nothing else names a provider. A setting in use is a [`Use`]: the row's id, a key, and whatever was set differently
//! from the row.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

mod check;
mod setting;
mod trouble;
pub use check::*;
pub use setting::Use;
pub use trouble::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Job {
    Summary,
    Voice,
    Ears,
}

/// How a service is reached: the name of an interface, and what that interface has to be told about this
/// service that he is never asked (`with`). The interfaces themselves are code, in a table for each job:
/// reading in the tower (`tower/judge/models.rs`), speaking and listening in the phone core (`airband-radio`'s
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
        crate::catalog::row_text(self.with.get(name))
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reach: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Offer {
    /// Its name among its provider's offers, and what a setting names beside the provider (`Use::offer`). A
    /// provider can do one job more than one way (two voice models, say): each is an offer of its own, and
    /// one key serves them all.
    pub id: String,
    pub job: Job,
    pub by: By,
    /// What this is called where he chooses it: the maker and the model, as its row names them.
    pub label: String,
    /// One line under that, for something that needs no key: where it runs and what it costs him.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// One of the few shown before the rest where he chooses. The others sit behind "More services".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
    /// It runs on his own computer: nothing leaves it, and it costs him nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
    /// The coding tool whose sign-in on his computer this uses (its row's id in `catalog/tools/`), when it
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

#[derive(Clone, PartialEq, Debug)]
pub struct Provider {
    /// Its name in the settings (`"kind"`), on the command line, and the usual name for its key.
    pub id: String,
    pub name: String,
    pub key: bool,
    /// What its key is called, for asking for it, as its row names it.
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
    /// The usual answer its row gives for one of its settings.
    pub fn usual(&self, field: &str) -> Option<String> {
        self.asks.iter().find(|a| a.field == field).map(|a| a.usual.clone())
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
    all.sort_by(|(p, o), (q, r)| (!o.local, p.key, !o.preferred, o.label.as_str()).cmp(&(!r.local, q.key, !r.preferred, r.label.as_str())));
    all
}

/// The providers he can choose for a job. Every one the catalogue has: a program lists the ones whose
/// interface it has the code for.
pub fn offering(job: Job) -> Vec<Arc<Provider>> {
    crate::catalog::current().providers.iter().filter(move |p| p.offered && p.offers.iter().any(|o| o.job == job)).cloned().collect()
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
        assert_eq!(check("{url}/models").address(&BTreeMap::from([("url".to_string(), "https://api.example/v1".to_string())])).as_deref(), Ok("https://api.example/v1/models"));
        assert!(check("{url}/models").address(&BTreeMap::from([("url".to_string(), "https://api.example@evil.example".to_string())])).is_err());
        // one not set says which, and is never taken for a service that could not be reached
        let unset = check("{url}/models").address(&BTreeMap::new()).unwrap_err();
        assert!(matches!(&unset, KeyCheck::BadSetting { why } if why.contains("url")), "{unset:?}");
        assert_ne!(unset.trouble(), Some(Trouble::Unreachable));
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
