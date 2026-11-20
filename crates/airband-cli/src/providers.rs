//! Every outside service the tower can use, one row each.
//!
//! A provider does up to three jobs: it reads (summaries and routing), it speaks, it listens. For each job a row
//! says which interface the service is reached through and what it has to be told. Where a common interface
//! exists, that is the one used, so most services are a row and no code:
//!
//! - reading: OpenAI's Chat Completions (`POST {url}/chat/completions`), which nearly every model service and
//!   every local model server accepts; or a command, for a model reached through a program on this computer.
//! - speaking: OpenAI's speech endpoint (`POST {url}/audio/speech`), which several services and most local
//!   voice servers accept. Services with something that endpoint lacks (word timings, a pass the relay can
//!   lend) keep an interface of their own.
//! - listening: there is no common interface for speech while it is spoken, so each is a dialect the phone
//!   speaks (`airband-radio/src/ears.rs`), named here.
//!
//! `airband setup`, `airband model`, `airband voice` and `airband ears` are all driven by this table, and so
//! are the calls themselves (`tower/judge.rs`, `tower/speech.rs`, `tower/ears.rs`). Nothing else in the tower
//! names a provider. A setting in use is a [`Use`]: the row's id, a key, and whatever was set differently
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
    Chat { cold: bool },
    /// A program on this computer: the prompt on standard input, the answer on standard output. The arguments
    /// here are the usual ones; `argv` in the settings replaces them.
    Command(&'static [&'static str]),
    /// OpenAI's speech endpoint, asked for raw 16-bit PCM.
    Speech,
    /// Google's own speech route.
    GeminiVoice,
    /// Microsoft's own speech route, by key, or by a short-lived pass in place of the key.
    AzureVoice { pass: bool },
    /// ElevenLabs' own route, which says when each word is spoken.
    ElevenlabsVoice,
    /// A listening dialect the phone speaks, by its name there.
    Listener(&'static str),
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
    pub asks: &'static [Ask],
}

pub struct Provider {
    /// Its name in the settings (`"kind"`), on the command line, and the usual name for its key.
    pub id: &'static str,
    pub name: &'static str,
    pub key: bool,
    /// Where its keys are made.
    pub keys_at: &'static str,
    pub offers: &'static [Offer],
    /// Offered when he chooses. Not so for what the tower sets for itself.
    pub offered: bool,
}

const fn ask(field: &'static str, question: &'static str, usual: &'static str) -> Ask {
    Ask { field, question, usual }
}

/// Claude Haiku with a one-line system prompt and no tools. Measured on 2026-11-18: about 650 tokens a message.
/// The same call with Claude Code's own system prompt and tool list was about 17,400.
pub const CLAUDE: &[&str] = &["claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--system-prompt", "You are a dispatcher. Output only what is asked for.", "--tools", ""];

pub const PROVIDERS: &[Provider] = &[
    Provider { id: "claude", name: "Claude on this computer, with your Claude login", key: false, keys_at: "", offered: true, offers: &[Offer { job: Job::Summary, by: By::Command(CLAUDE), asks: &[] }] },
    Provider {
        id: "anthropic",
        name: "Anthropic, with an API key",
        key: true,
        keys_at: "console.anthropic.com/settings/keys",
        offered: true,
        offers: &[Offer { job: Job::Summary, by: By::Chat { cold: false }, asks: &[ask("url", "Address", "https://api.anthropic.com/v1"), ask("model", "Model", "claude-haiku-5-5")] }],
    },
    Provider {
        id: "gemini",
        name: "Google Gemini",
        key: true,
        keys_at: "aistudio.google.com/apikey",
        offered: true,
        offers: &[
            // the lite model is cheaper and misread two of the end-to-end checks; this one passes them all
            Offer { job: Job::Summary, by: By::Chat { cold: true }, asks: &[ask("url", "Address", "https://generativelanguage.googleapis.com/v1beta/openai"), ask("model", "Model", "gemini-flash-latest")] },
            Offer { job: Job::Voice, by: By::GeminiVoice, asks: &[ask("model", "Model", "gemini-3.8-flash-lite-tts"), ask("voice", "Voice", "Leda")] },
        ],
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        key: true,
        keys_at: "platform.openai.com/api-keys",
        offered: true,
        offers: &[
            Offer { job: Job::Summary, by: By::Chat { cold: true }, asks: &[ask("url", "Address", "https://api.openai.com/v1"), ask("model", "Model", "gpt-4o-mini")] },
            Offer { job: Job::Voice, by: By::Speech, asks: &[ask("url", "Address", "https://api.openai.com/v1"), ask("model", "Model", "gpt-4o-mini-tts"), ask("voice", "Voice", "coral"), ask("rate", "Samples a second", "24000")] },
        ],
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        key: true,
        keys_at: "openrouter.ai/keys",
        offered: true,
        offers: &[Offer { job: Job::Summary, by: By::Chat { cold: true }, asks: &[ask("url", "Address", "https://openrouter.ai/api/v1"), ask("model", "Model", "google/gemini-flash-latest")] }],
    },
    Provider {
        id: "groq",
        name: "Groq",
        key: true,
        keys_at: "console.groq.com/keys",
        offered: true,
        offers: &[Offer { job: Job::Summary, by: By::Chat { cold: true }, asks: &[ask("url", "Address", "https://api.groq.com/openai/v1"), ask("model", "Model", "llama-3.3-70b-versatile")] }],
    },
    Provider {
        id: "local",
        name: "A model server on this computer or your network (Ollama, LM Studio, llama.cpp, a voice server)",
        key: false,
        keys_at: "",
        offered: true,
        offers: &[
            Offer { job: Job::Summary, by: By::Chat { cold: true }, asks: &[ask("url", "Address", "http://localhost:11434/v1"), ask("model", "Model", "")] },
            Offer { job: Job::Voice, by: By::Speech, asks: &[ask("url", "Address", "http://localhost:8880/v1"), ask("model", "Model", "kokoro"), ask("voice", "Voice", "af_heart"), ask("rate", "Samples a second", "24000")] },
        ],
    },
    Provider {
        id: "elevenlabs",
        name: "ElevenLabs",
        key: true,
        keys_at: "elevenlabs.io/app/settings/api-keys",
        offered: true,
        // the voice is Jessica: young, bright and warm
        offers: &[Offer { job: Job::Voice, by: By::ElevenlabsVoice, asks: &[ask("voice", "Voice id", "cgSgspJ2msm6clMCkdW9"), ask("model", "Model", "eleven_flash_v2_5")] }],
    },
    Provider {
        id: "azure",
        name: "Microsoft Azure Speech",
        key: true,
        keys_at: "portal.azure.com, a Speech resource, Keys and Endpoint",
        offered: true,
        offers: &[
            Offer { job: Job::Voice, by: By::AzureVoice { pass: false }, asks: &[ask("region", "Region", "eastus"), ask("voice", "Voice", AZURE_VOICE)] },
            Offer { job: Job::Ears, by: By::Listener("azure"), asks: &[ask("region", "Region", "eastus"), ask("language", "Language", "en-US")] },
        ],
    },
    // a program of his own that reads: `argv` says what to run
    Provider { id: "command", name: "A command of your own", key: false, keys_at: "", offered: false, offers: &[Offer { job: Job::Summary, by: By::Command(&[]), asks: &[] }] },
    // what the tower uses when nothing is set here and the relay lends its pass: never written in the settings
    Provider { id: LENT, name: "The relay's speech service", key: false, keys_at: "", offered: false, offers: &[Offer { job: Job::Voice, by: By::AzureVoice { pass: true }, asks: &[ask("region", "Region", ""), ask("voice", "Voice", AZURE_VOICE)] }] },
];

const AZURE_VOICE: &str = "en-US-AvaMultilingualNeural";
/// The row for the relay's lent pass, and the listening dialect that pass is for.
pub const LENT: &str = "relay-pass";
pub const LENT_LISTENER: &str = "azure";

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// The providers he can choose for a job.
pub fn offering(job: Job) -> impl Iterator<Item = &'static Provider> {
    PROVIDERS.iter().filter(move |p| p.offered && p.offers.iter().any(|o| o.job == job))
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
        for p in PROVIDERS {
            assert!(!p.offers.is_empty(), "{} does nothing", p.id);
            assert_eq!(PROVIDERS.iter().filter(|o| o.id == p.id).count(), 1, "{} is listed twice", p.id);
            for o in p.offers {
                assert_eq!(p.offers.iter().filter(|x| x.job == o.job).count(), 1, "{} does one job two ways", p.id);
                // each kind of job is done through an interface for that job
                let fits = match o.by {
                    By::Chat { .. } | By::Command(_) => o.job == Job::Summary,
                    By::Speech | By::GeminiVoice | By::AzureVoice { .. } | By::ElevenlabsVoice => o.job == Job::Voice,
                    By::Listener(_) => o.job == Job::Ears,
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
