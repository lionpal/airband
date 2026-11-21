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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Job {
    Summary,
    Voice,
    Ears,
}

/// The interface a service is reached through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum By {
    /// OpenAI Chat Completions. `cold` sends `temperature: 0`, which most services take and some refuse.
    /// `shaped`: the service can be held to a JSON schema (`response_format`), so what comes back always
    /// parses; a call that knows the shape it wants says so. Small models need this most.
    Chat { cold: bool, shaped: bool },
    /// A model the tower fetches and runs itself on this computer, with llama.cpp's server, and speaks to by
    /// the same chat interface held to shape. Named as that server takes it: a Hugging Face repo and a quant.
    OnThisComputer(&'static str),
    /// A program on this computer: the prompt on standard input, the answer on standard output. The arguments
    /// here are the usual ones; `argv` in the settings replaces them.
    Command(&'static [&'static str]),
    /// OpenAI's speech endpoint, asked for raw 16-bit PCM.
    Speech,
    /// Google's own speech route.
    GeminiVoice,
    /// Microsoft's own speech route.
    AzureVoice,
    /// ElevenLabs' own route, which says when each word is spoken.
    ElevenlabsVoice,
    /// A listening dialect the phone speaks while he talks, by its name there. The service gives a
    /// short-lived pass for a key, so a key on the computer never has to reach the phone.
    Listener(&'static str),
    /// OpenAI's transcription endpoint (`POST {url}/audio/transcriptions`): a finished recording in, words out.
    Transcribe,
}

/// Something a job has to be told beyond the key: the setting's name, the question, and the usual answer.
pub struct Ask {
    pub field: &'static str,
    pub question: &'static str,
    pub usual: &'static str,
}

pub struct Offer {
    pub job: Job,
    pub by: By,
    /// What this is called where he chooses it: the maker and the model ("Google Gemini 3.5 Flash-Lite").
    pub label: &'static str,
    /// One line under that, for something that needs no key: where it runs and what it costs him.
    pub about: &'static str,
    /// One of the few shown before the rest where he chooses. The others sit behind "More services".
    pub preferred: bool,
    /// The coding tool whose sign-in on his computer this uses (`claude-code`, `codex`, `opencode`), when it
    /// uses one and so needs no key. It is offered to someone who uses that tool. Empty otherwise.
    pub login: &'static str,
    /// What it costs him, in a few words, as its maker published it on 2026-11-20 and worked out for this
    /// use (a summary is about 650 tokens in and 80 out; speech is about 900 characters a minute). Empty
    /// when it costs nothing, or when the maker's price could not be confirmed.
    pub price: &'static str,
    /// For a voice: a few of its voices to choose from, the usual one first. The one chosen is the setting
    /// `voice`. Empty when there is only the setting.
    pub voices: &'static [Voice],
    pub asks: &'static [Ask],
}

/// One of the voices a voice service speaks in.
pub struct Voice {
    /// What the service calls it: the value of the setting `voice`.
    pub id: &'static str,
    pub name: &'static str,
    /// How it sounds, in the maker's own words, and its accent. Empty when the maker says nothing.
    pub about: &'static str,
    /// A clip of it that its maker publishes, to hear before choosing. Empty when there is none.
    pub sample: &'static str,
}

pub struct Provider {
    /// Its name in the settings (`"kind"`), on the command line, and the usual name for its key.
    pub id: &'static str,
    pub name: &'static str,
    pub key: bool,
    /// What its key is called, for asking for it ("Gemini API key").
    pub key_name: &'static str,
    /// Where its keys are made.
    pub keys_at: &'static str,
    pub offers: &'static [Offer],
    /// Offered when he chooses. Not so for what the tower sets for itself.
    pub offered: bool,
}

impl Offer {
    /// It runs on his own computer: nothing leaves it, and it costs him nothing.
    pub fn local(&self) -> bool {
        matches!(self.by, By::OnThisComputer(_))
    }
}

/// The live listening dialects this build speaks (`airband-radio/src/ears.rs` has one for each). A row that
/// names another is left out when the catalogue is read: it is for a newer build.
pub const DIALECTS: &[&str] = &["azure", "gemini", "elevenlabs"];

/// What a pass says when the phone is to send its recording to the tower, which holds the key.
pub const BY_TOWER: &str = "tower";
/// What it says when nothing listens.
pub const NO_EARS: &str = "none";

impl Use {
    /// Every setting of a job as it stands, set or usual: what a phone is told beside its pass.
    pub fn settings(&self, job: Job) -> BTreeMap<String, String> {
        self.offer(job).map(|(_, o)| o.asks.iter().map(|a| (a.field.to_string(), self.get(job, a.field))).collect()).unwrap_or_default()
    }
}

pub fn provider(id: &str) -> Option<&'static Provider> {
    crate::catalog::current().providers.iter().find(|p| p.id == id)
}

/// The providers he can choose for a job.
/// The offers for a job in the order to show them where he chooses: what runs on his own computer first,
/// then what needs no key (neither costs him anything more), then the preferred, then the rest. Within each,
/// by name.
pub fn in_order(job: Job) -> Vec<(&'static Provider, &'static Offer)> {
    let mut all: Vec<(&Provider, &Offer)> = offering(job).filter_map(|p| p.offers.iter().find(|o| o.job == job).map(|o| (p, o))).collect();
    all.sort_by_key(|(p, o)| (!o.local(), p.key, !o.preferred, o.label));
    all
}

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
            None => self.offer(job).and_then(|(_, o)| o.asks.iter().find(|a| a.field == field)).map(|a| a.usual.to_string()).unwrap_or_default(),
        }
    }
    /// One line for him: the provider and what it is set to. Never the key.
    pub fn describe(&self, job: Job) -> String {
        match self.offer(job) {
            None if self.is_none() => "none".into(),
            None => format!("\"{}\", which this version of airband does not have", self.kind),
            Some((p, o)) => {
                let told: Vec<String> = o.asks.iter().map(|a| format!("{} {}", a.question.to_lowercase(), self.get(job, a.field))).collect();
                if told.is_empty() { p.name.to_string() } else { format!("{} ({})", p.name, told.join(", ")) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_well_formed() {
        let rows = crate::catalog::current().providers;
        for p in rows {
            assert!(!p.offers.is_empty(), "{} does nothing", p.id);
            assert_eq!(rows.iter().filter(|o| o.id == p.id).count(), 1, "{} is listed twice", p.id);
            for o in p.offers {
                assert_eq!(p.offers.iter().filter(|x| x.job == o.job).count(), 1, "{} does one job two ways", p.id);
                // each kind of job is done through an interface for that job
                let fits = match o.by {
                    By::Chat { .. } | By::Command(_) | By::OnThisComputer(_) => o.job == Job::Summary,
                    By::Speech | By::GeminiVoice | By::AzureVoice | By::ElevenlabsVoice => o.job == Job::Voice,
                    By::Listener(_) | By::Transcribe => o.job == Job::Ears,
                };
                assert!(fits, "{}: {:?} is not a way to do {:?}", p.id, o.by, o.job);
            }
        }
    }

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
