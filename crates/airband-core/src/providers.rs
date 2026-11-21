//! Every outside service the tower can use, one row each. The rows themselves are data: the files under
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
//! `airband setup`, `airband model`, `airband voice` and `airband ears` are all driven by this table, and so
//! are the calls themselves (`tower/judge.rs`, `airband-radio/src/voices.rs`, `tower/ears.rs`). The phone reads
//! the same table for the voices it can reach with a key of his own. Nothing else names a provider. A setting in use is a [`Use`]: the row's id, a key, and whatever was set differently
//! from the row.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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
    pub fn list(&self, name: &str) -> Vec<String> {
        self.with.get(name).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
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
    pub offers: Vec<Offer>,
    /// Offered when he chooses. Not so for what the tower sets for itself.
    pub offered: bool,
}

impl Offer {
    /// It runs on his own computer: nothing leaves it, and it costs him nothing.
    pub fn local(&self) -> bool {
        self.local
    }
}

/// What a pass says when the phone is to send its recording to the tower, which holds the key.
pub const BY_TOWER: &str = "tower";
/// What it says when nothing listens.
pub const NO_EARS: &str = "none";
/// Words that mean "no provider" or a way of listening where a provider's id would stand (`Use::kind`,
/// `EarsGrant::kind`). No provider may take one of these as its id.
pub const NOT_AN_ID: &[&str] = &[BY_TOWER, NO_EARS, "device", "own-key", "on-device", "kind", "key"];

impl Use {
    /// Every setting of a job as it stands, set or usual: what a phone is told beside its pass.
    pub fn settings(&self, job: Job) -> BTreeMap<String, String> {
        self.offer(job).map(|(_, o)| o.asks.iter().map(|a| (a.field.to_string(), self.get(job, &a.field))).collect()).unwrap_or_default()
    }
}

pub fn provider(id: &str) -> Option<&'static Provider> {
    crate::catalog::current().providers.iter().find(|p| p.id == id)
}

/// The offers for a job in the order to show them where he chooses: what runs on his own computer first,
/// then what needs no key (neither costs him anything more), then the preferred, then the rest. Within each,
/// by name.
pub fn in_order(job: Job) -> Vec<(&'static Provider, &'static Offer)> {
    let mut all: Vec<(&Provider, &Offer)> = offering(job).filter_map(|p| p.offers.iter().find(|o| o.job == job).map(|o| (p, o))).collect();
    all.sort_by_key(|(p, o)| (!o.local(), p.key, !o.preferred, o.label.as_str()));
    all
}

/// The providers he can choose for a job. Every one the catalogue has: a program lists the ones whose
/// interface it has the code for.
pub fn offering(job: Job) -> impl Iterator<Item = &'static Provider> {
    crate::catalog::current().providers.iter().filter(move |p| p.offered && p.offers.iter().any(|o| o.job == job))
}

/// A provider in use for one job: which row, with what key, and whatever is set differently from the row.
/// Written in the settings as `{"kind": …, "key": …, …}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(flatten)]
    pub set: BTreeMap<String, Value>,
}

impl Default for Use {
    fn default() -> Self {
        Use::of(NONE)
    }
}

const NONE: &str = "none";

impl Use {
    pub fn of(kind: &str) -> Use {
        Use { kind: kind.into(), key: String::new(), set: BTreeMap::new() }
    }
    /// Nothing is set. `device` is what "nothing" was called when a phone's own voice was still an answer.
    pub fn is_none(&self) -> bool {
        matches!(self.kind.as_str(), NONE | "device" | "")
    }
    /// The row for this job, and how the job is done. Nothing when the settings name a provider this build
    /// does not have, or one that does not do this job.
    pub fn offer(&self, job: Job) -> Option<(&'static Provider, &'static Offer)> {
        let p = provider(&self.kind)?;
        p.offers.iter().find(|o| o.job == job).map(|o| (p, o))
    }
    /// A setting for this job: what was set, else what the row usually uses.
    pub fn get(&self, job: Job, field: &str) -> String {
        match self.set.get(field) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => self.offer(job).and_then(|(_, o)| o.asks.iter().find(|a| a.field == field)).map(|a| a.usual.clone()).unwrap_or_default(),
        }
    }
    /// One line for him: the provider and what it is set to. Never the key.
    pub fn describe(&self, job: Job) -> String {
        match self.offer(job) {
            None if self.is_none() => "none".into(),
            None => format!("\"{}\", which this version of airband does not have", self.kind),
            Some((p, o)) => {
                let told: Vec<String> = o.asks.iter().map(|a| format!("{} {}", a.question.to_lowercase(), self.get(job, &a.field))).collect();
                if told.is_empty() { p.name.clone() } else { format!("{} ({})", p.name, told.join(", ")) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_written_before_the_table_still_read() {
        let v: Use = serde_json::from_str(r#"{"kind":"elevenlabs","key":"SECRETKEY","voice":"abc","model":"eleven_x"}"#).unwrap();
        assert_eq!((v.get(Job::Voice, "voice"), v.get(Job::Voice, "model"), v.key.as_str()), ("abc".into(), "eleven_x".into(), "SECRETKEY"));
        // what was not set is the row's usual
        let e: Use = serde_json::from_str(r#"{"kind":"azure","key":"k","region":"japaneast"}"#).unwrap();
        assert_eq!((e.get(Job::Ears, "region"), e.get(Job::Ears, "language")), ("japaneast".into(), "en-US".into()));
        // nothing set, under either of its names
        assert!(serde_json::from_str::<Use>(r#"{"kind":"device"}"#).unwrap().is_none());
        assert!(serde_json::from_str::<Use>(r#"{"kind":"none"}"#).unwrap().is_none());
        // a command of his own keeps its arguments, and round-trips as it was written
        let c: Use = serde_json::from_str(r#"{"kind":"command","argv":["mine","-x"]}"#).unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"kind":"command","argv":["mine","-x"]}"#);
        // the key is never part of what is said about it
        assert!(!v.describe(Job::Voice).contains("SECRETKEY"));
        assert_eq!(e.describe(Job::Ears), "Microsoft Azure Speech (region japaneast, language en-US)");
    }
}
