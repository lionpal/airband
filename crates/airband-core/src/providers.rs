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
    /// Microsoft's own speech route, by key, or by a short-lived pass in place of the key.
    AzureVoice { pass: bool },
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
    pub asks: &'static [Ask],
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

const fn ask(field: &'static str, question: &'static str, usual: &'static str) -> Ask {
    Ask { field, question, usual }
}
const fn offer(job: Job, by: By, label: &'static str, about: &'static str, asks: &'static [Ask]) -> Offer {
    Offer { job, by, label, about, asks }
}

/// Claude Haiku with a one-line system prompt and no tools. Measured on 2026-11-18: about 650 tokens a message.
/// The same call with Claude Code's own system prompt and tool list was about 17,400.
pub const CLAUDE: &[&str] = &["claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--system-prompt", "You are a dispatcher. Output only what is asked for.", "--tools", ""];

pub const PROVIDERS: &[Provider] = &[
    Provider { id: "claude", name: "Claude on this computer, with your Claude login", key: false, key_name: "", keys_at: "", offered: true, offers: &[offer(Job::Summary, By::Command(CLAUDE), "Claude Haiku 5.5", "Uses the Claude login on your computer", &[])] },
    Provider {
        id: "anthropic",
        name: "Anthropic, with an API key",
        key: true,
        key_name: "Anthropic API key",
        keys_at: "console.anthropic.com/settings/keys",
        offered: true,
        offers: &[offer(Job::Summary, By::Chat { cold: false, shaped: false }, "Anthropic Claude Haiku 5.5", "", &[ask("url", "Address", "https://api.anthropic.com/v1"), ask("model", "Model", "claude-haiku-5-5")])],
    },
    Provider {
        id: "gemini",
        name: "Google Gemini",
        key: true,
        key_name: "Gemini API key",
        keys_at: "aistudio.google.com/apikey",
        offered: true,
        offers: &[
            offer(Job::Summary, By::Chat { cold: true, shaped: true }, "Google Gemini 3.5 Flash-Lite", "", &[ask("url", "Address", "https://generativelanguage.googleapis.com/v1beta/openai"), ask("model", "Model", "gemini-3.5-flash-lite")]),
            offer(Job::Voice, By::GeminiVoice, "Google Gemini 3.8 Flash TTS", "", &[ask("model", "Model", "gemini-3.8-flash-lite-tts"), ask("voice", "Voice", "Leda")]),
            // its Live API, with the model that only transcribes: the words come back as he speaks
            offer(Job::Ears, By::Listener("gemini"), "Google Gemini 3.5 Transcribe", "", &[ask("model", "Model", "gemini-3.5-transcribe-live"), ask("language", "Language", "en-US")]),
        ],
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        key: true,
        key_name: "OpenAI API key",
        keys_at: "platform.openai.com/api-keys",
        offered: true,
        offers: &[
            // a model that reasons takes no temperature
            offer(Job::Summary, By::Chat { cold: false, shaped: true }, "OpenAI GPT-6 Luna", "", &[ask("url", "Address", "https://api.openai.com/v1"), ask("model", "Model", "gpt-6-luna")]),
            offer(Job::Voice, By::Speech, "OpenAI GPT-4o mini TTS", "", &[ask("url", "Address", "https://api.openai.com/v1"), ask("model", "Model", "gpt-4o-mini-tts"), ask("voice", "Voice", "coral"), ask("rate", "Samples a second", "24000")]),
            offer(Job::Ears, By::Transcribe, "OpenAI GPT Transcribe", "", &[ask("url", "Address", "https://api.openai.com/v1"), ask("model", "Model", "gpt-transcribe")]),
        ],
    },
    // A model the tower fetches and runs itself, on this computer: nothing a message says leaves it. Measured
    // against Haiku on the summary job in docs/experiments/2026-11-20-local-summary-models.md.
    Provider {
        id: "qwen",
        name: "Alibaba Qwen 3.5 4B, on this computer",
        key: false,
        key_name: "",
        keys_at: "",
        offered: true,
        offers: &[offer(Job::Summary, By::OnThisComputer("unsloth/Qwen3.5-4B-GGUF:Q4_K_M"), "Alibaba Qwen 3.5 4B", "Runs on your computer. 2.7 GB download, about 4 GB of memory", &[])],
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        key: true,
        key_name: "OpenRouter API key",
        keys_at: "openrouter.ai/keys",
        offered: true,
        offers: &[offer(Job::Summary, By::Chat { cold: true, shaped: false }, "OpenRouter", "", &[ask("url", "Address", "https://openrouter.ai/api/v1"), ask("model", "Model", "google/gemini-3.5-flash-lite")])],
    },
    Provider {
        id: "groq",
        name: "Groq",
        key: true,
        key_name: "Groq API key",
        keys_at: "console.groq.com/keys",
        offered: true,
        offers: &[
            offer(Job::Summary, By::Chat { cold: true, shaped: false }, "Groq GPT-OSS 120B", "", &[ask("url", "Address", "https://api.groq.com/openai/v1"), ask("model", "Model", "openai/gpt-oss-120b")]),
            offer(Job::Ears, By::Transcribe, "Groq Whisper Large v3 Turbo", "", &[ask("url", "Address", "https://api.groq.com/openai/v1"), ask("model", "Model", "whisper-large-v3-turbo")]),
        ],
    },
    Provider {
        id: "local",
        name: "A model server on this computer or your network (Ollama, LM Studio, llama.cpp, a voice server)",
        key: false,
        key_name: "",
        keys_at: "",
        offered: true,
        offers: &[
            offer(Job::Summary, By::Chat { cold: true, shaped: true }, "Your own model server", "Ollama, LM Studio or llama.cpp, at an address you give", &[ask("url", "Address", "http://localhost:11434/v1"), ask("model", "Model", "")]),
            offer(Job::Voice, By::Speech, "Your own voice server", "At an address you give", &[ask("url", "Address", "http://localhost:8880/v1"), ask("model", "Model", "kokoro"), ask("voice", "Voice", "af_heart"), ask("rate", "Samples a second", "24000")]),
            offer(Job::Ears, By::Transcribe, "Your own speech server", "At an address you give", &[ask("url", "Address", "http://localhost:8000/v1"), ask("model", "Model", "whisper-1")]),
        ],
    },
    Provider {
        id: "elevenlabs",
        name: "ElevenLabs",
        key: true,
        key_name: "ElevenLabs API key",
        keys_at: "elevenlabs.io/app/settings/api-keys",
        offered: true,
        // the voice is Jessica: young, bright and warm
        offers: &[offer(Job::Voice, By::ElevenlabsVoice, "ElevenLabs Flash 2.5", "", &[ask("voice", "Voice id", "cgSgspJ2msm6clMCkdW9"), ask("model", "Model", "eleven_flash_v2_5")])],
    },
    Provider {
        id: "azure",
        name: "Microsoft Azure Speech",
        key: true,
        key_name: "Azure key",
        keys_at: "portal.azure.com, a Speech resource, Keys and Endpoint",
        offered: true,
        offers: &[
            offer(Job::Voice, By::AzureVoice { pass: false }, "Microsoft Azure Speech", "", &[ask("region", "Region", "eastus"), ask("voice", "Voice", AZURE_VOICE)]),
            offer(Job::Ears, By::Listener("azure"), "Microsoft Azure Speech", "", &[ask("region", "Region", "eastus"), ask("language", "Language", "en-US")]),
        ],
    },
    // a program of his own that reads: `argv` says what to run
    Provider { id: "command", name: "A command of your own", key: false, key_name: "", keys_at: "", offered: false, offers: &[offer(Job::Summary, By::Command(&[]), "A command of your own", "", &[])] },
    // what the tower uses when nothing is set here and the relay lends its pass: never written in the settings
    Provider { id: LENT, name: "The relay's speech service", key: false, key_name: "", keys_at: "", offered: false, offers: &[offer(Job::Voice, By::AzureVoice { pass: true }, "The relay's speech service", "", &[ask("region", "Region", ""), ask("voice", "Voice", AZURE_VOICE)])] },
];

const AZURE_VOICE: &str = "en-US-AvaMultilingualNeural";
/// The row for the relay's lent pass, and the listening dialect that pass is for.
pub const LENT: &str = "relay-pass";
pub const LENT_LISTENER: &str = "azure";
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
                    By::Chat { .. } | By::Command(_) | By::OnThisComputer(_) => o.job == Job::Summary,
                    By::Speech | By::GeminiVoice | By::AzureVoice { .. } | By::ElevenlabsVoice => o.job == Job::Voice,
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
