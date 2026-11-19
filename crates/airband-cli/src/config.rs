//! ~/.airband: the config (port, token, who to call him, which model summarises), state and log.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    /// The tower's own token: everything on its port, including reporting as an agent. Stays on this machine.
    #[serde(default = "new_token")]
    pub token: String,
    /// What a radio paired directly is given instead: good for `/v1/radio/` and nothing else, so a pairing link
    /// cannot be used to report as an agent (an agent can name a command for the tower to run).
    #[serde(default = "new_token")]
    pub radio_token: String,
    /// What sessions are told his name is.
    #[serde(default = "default_name")]
    pub name: String,
    /// What this computer is called on a phone paired with several. Left out, its short host name is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// What cuts a long message down to a spoken sentence or two. Any model, through any command.
    #[serde(default)]
    pub summarizer: Summarizer,
    /// The relay radios reach this tower through ("https://relay.example"). Without one, a radio must be able
    /// to reach this machine itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    /// The key the relay asks for before it opens a channel, if it asks for one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_admission: Option<String>,
    /// Outbound: what speaks to him on a call. Set separately from `ears`.
    #[serde(default)]
    pub voice: Voice,
    /// Inbound: what turns his speech into text on a call. Set separately from `voice`.
    #[serde(default)]
    pub ears: Ears,
    /// How to start a new agent, by name. Each is a command; `{cwd}`, `{task}`, `{project}` and `{session}` (a
    /// fresh name for a terminal session) are filled in. Add one per agent tool.
    #[serde(default = "default_launchers")]
    pub launchers: BTreeMap<String, Launcher>,
    /// Which launcher `launch` uses.
    #[serde(default = "default_launcher")]
    pub launcher: String,
    /// Projects a new agent may be launched in, by spoken name, beyond the ones the tower has seen agents work
    /// in. Name to folder.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, String>,
    /// How long, in seconds, a harness waits for him to allow or refuse something from the phone before it asks at
    /// the keyboard instead. 0: never ask the phone. Nothing is ever allowed by waiting: no answer is no answer.
    #[serde(default = "default_permission_wait")]
    pub permission_wait: u64,
    /// His own quick messages, offered after the standard ones: things he finds himself telling agents again and
    /// again. `airband quick add "Make no mistakes"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<airband_core::radio::Quick>,
    /// One per paired radio. Removing an entry cuts that radio off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairings: Vec<PairedRadio>,
}

/// What speaks. `device` is the phone's own voice (Apple's on an iPhone): free, instant, works with no signal.
/// The others are made by the tower, which holds the key, and sent to the phone as audio.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Voice {
    #[default]
    Device,
    Gemini { key: String, #[serde(default = "gemini_model")] model: String, #[serde(default = "gemini_voice")] voice: String },
    Azure { key: String, region: String, #[serde(default = "azure_voice")] voice: String },
}

/// What listens. `device` is the phone's own recognition (Apple's on an iPhone). With `azure` the phone sends
/// his speech straight to Microsoft, using a ten-minute pass the tower hands it; the key stays on the tower.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Ears {
    #[default]
    Device,
    Azure { key: String, region: String, #[serde(default = "language")] language: String },
}

fn gemini_model() -> String {
    "gemini-3.8-flash-lite-tts".into()
}
fn gemini_voice() -> String {
    "Leda".into()
}
fn azure_voice() -> String {
    "en-US-AvaMultilingualNeural".into()
}
fn language() -> String {
    "en-US".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Launcher {
    /// `{cwd}`, `{task}`, `{project}` and `{session}` are filled in. An argument that is exactly `{options}` is
    /// replaced by `with_model` and `with_effort`, each only when he asked for one, and by nothing otherwise.
    pub argv: Vec<String>,
    /// The models he can ask for by name ("with fable, …"), and what the tool calls each.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// The efforts he can ask for ("with fable high, …"), as the tool takes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The arguments that choose a model, with `{model}` filled in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with_model: Vec<String>,
    /// The arguments that choose an effort, with `{effort}` filled in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with_effort: Vec<String>,
}

fn default_launcher() -> String {
    "claude-code".into()
}
/// Claude Code in a detached tmux session: an ordinary interactive session he can attach to at his desk
/// (`tmux attach -t <session>`), which reports in through the same hooks as any other.
fn default_launchers() -> BTreeMap<String, Launcher> {
    let list = |words: &[&str]| words.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let claude = Launcher {
        argv: list(&["tmux", "new-session", "-d", "-s", "{session}", "-c", "{cwd}", "claude", "{options}", "{task}"]),
        models: ["fable", "opus", "sonnet", "haiku"].iter().map(|m| (m.to_string(), m.to_string())).collect(),
        efforts: list(&["low", "medium", "high", "xhigh", "max"]),
        with_model: list(&["--model", "{model}"]),
        with_effort: list(&["--effort", "{effort}"]),
    };
    BTreeMap::from([(default_launcher(), claude)])
}
/// The launcher every tower had before a model could be asked for. One still set to exactly this is brought up
/// to date; one he has changed is his and is left alone.
const LAUNCHER_BEFORE_MODELS: &[&str] = &["tmux", "new-session", "-d", "-s", "{session}", "-c", "{cwd}", "claude", "{task}"];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PairedRadio {
    pub name: String,
    /// 32 random bytes as hex, shared with that radio and nobody else, the relay included.
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Summarizer {
    /// Run a command: the prompt goes in on standard input, the answer is read from standard output.
    /// `AIRBAND_INTERNAL=1` is set so the command's own hooks do not report back to the tower.
    Command { argv: Vec<String> },
    /// No model: read out the first two sentences as written.
    None,
}

/// Claude Haiku with a one-line system prompt and no tools. Measured on 2026-11-18: about 650 tokens a message.
/// The same call with Claude Code's own system prompt and tool list was about 17,400.
const LEAN: [&str; 10] = ["claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--system-prompt", "You are a dispatcher. Output only what is asked for.", "--tools", ""];
/// The default before that measurement. A config still holding it is moved to the lean one.
const HEAVY: [&str; 8] = ["claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--disallowedTools", "Bash,Edit,Write,Read,Agent,WebFetch,WebSearch"];

impl Default for Summarizer {
    fn default() -> Self {
        Summarizer::Command { argv: LEAN.iter().map(|s| s.to_string()).collect() }
    }
}

fn default_port() -> u16 {
    7373
}
fn default_name() -> String {
    "Charles".into()
}
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the system has no random source");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
fn default_permission_wait() -> u64 {
    180
}
fn new_token() -> String {
    random_hex(24)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).expect("no home directory (HOME or USERPROFILE)")
}
pub fn dir() -> PathBuf {
    home().join(".airband")
}

/// Write a file only its owner can read (it holds the token, or sessions' inbox tokens).
pub fn write_private(path: &PathBuf, text: &str) -> std::io::Result<()> {
    fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The config as it is on disk now, or None if there is none yet.
pub fn read() -> Option<Config> {
    serde_json::from_str(&fs::read_to_string(dir().join("config.json")).ok()?).ok()
}

pub fn save(cfg: &Config) -> Result<(), String> {
    write_private(&dir().join("config.json"), &format!("{}\n", serde_json::to_string_pretty(cfg).unwrap())).map_err(|e| format!("cannot write ~/.airband/config.json: {e}"))
}

/// Read the config, creating it (with a fresh token) on first use.
pub fn load() -> Config {
    let path = dir().join("config.json");
    fs::create_dir_all(dir()).expect("cannot create ~/.airband");
    let existing = fs::read_to_string(&path).ok();
    let mut cfg: Config = match existing.as_deref().map(serde_json::from_str::<Config>) {
        Some(Ok(cfg)) => cfg,
        None => serde_json::from_str("{}").unwrap(),
        // A config that cannot be read is never replaced: it holds the token, the pairings and the keys. A typo,
        // or a setting written by a newer build, must cost nothing but this message.
        Some(Err(e)) => {
            eprintln!("{} cannot be read: {e}.\nNothing was changed. Fix that line, or move the file aside to start again with a new one (which unpairs every radio).", path.display());
            std::process::exit(2);
        }
    };
    if matches!(&cfg.summarizer, Summarizer::Command { argv } if argv.iter().map(String::as_str).eq(HEAVY)) {
        cfg.summarizer = Summarizer::default();
    }
    if cfg.launchers.get(&default_launcher()).is_some_and(|l| l.models.is_empty() && l.argv.iter().map(String::as_str).eq(LAUNCHER_BEFORE_MODELS.iter().copied())) {
        cfg.launchers.extend(default_launchers());
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&cfg).unwrap());
    if existing.as_deref() != Some(text.as_str()) {
        write_private(&path, &text).expect("cannot write ~/.airband/config.json");
    }
    cfg
}

/// This machine's Tailscale addresses (100.64.0.0/10): the only way in from outside the machine.
pub fn tailscale_addrs() -> Vec<std::net::Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]) => Some(a),
            _ => None,
        })
        .collect()
}

/// This computer's name for a phone paired with several: the configured one, else the short host name.
pub fn station(cfg: &Config) -> String {
    if let Some(s) = cfg.station.clone().filter(|s| !s.trim().is_empty()) {
        return s;
    }
    let host = std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let short = host.split('.').next().unwrap_or("").to_lowercase();
    // "someones-mac-mini" says more than it needs to: keep what kind of machine it is
    let short = short.rsplit("s-").next().unwrap_or(&short).to_string();
    if short.is_empty() { "computer".into() } else { short }
}
