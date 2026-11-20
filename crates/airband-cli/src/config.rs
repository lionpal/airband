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
    /// Let a paired radio on the same network reach this tower straight, without the relay: faster, and it
    /// works when the relay does not. The tower then listens on this computer's local network addresses, for
    /// sealed requests only. `airband lan off` turns it off.
    #[serde(default = "yes")]
    pub lan: bool,
    /// What cuts a long message down to a spoken sentence or two. Any model, through any command.
    #[serde(default = "default_summarizer")]
    pub summarizer: Summarizer,
    /// A second model, asked when the first does not answer (it is logged out, over its limit, or down). Nothing
    /// is ever worked out from the words themselves in place of a model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarizer_fallback: Option<Summarizer>,
    /// The keys he has given, each under a name with the provider it is for. What uses a key (summaries,
    /// voice, listening) holds its value here in memory; the settings file writes that as `key:<name>`, so a
    /// key is stored once however many things use it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, StoredKey>,
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

/// What reads, what speaks and what listens are each a provider in use: a row of `providers::PROVIDERS`, a
/// key, and what is set differently from the row.
pub use crate::providers::Use;
pub type Summarizer = Use;
pub type Voice = Use;
pub type Ears = Use;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Launcher {
    /// The harness it starts, by its adapter's name ("claude-code", "codex"): how the tower knows which
    /// launcher carries on an agent of that kind, and what he means when he asks for a tool by name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub harness: String,
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
    /// When set, the tower does not hand the agent off: it keeps the process and its input, and writes his
    /// words there as his own turns. `argv` then starts a program that reads one message a line on standard
    /// input for as long as it runs; `{task}` is not used, the task is the first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

/// How a held agent is started and spoken to. Each list is arguments, with `{id}` the session's id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Held {
    /// Arguments that start a new session under an id the tower chose.
    pub fresh: Vec<String>,
    /// Arguments that carry on the session with that id.
    pub resume: Vec<String>,
    /// One line of input for one message of his, with `{text}` where his words go as a JSON string.
    pub line: String,
    /// One line of input that stops what it is doing, with `{id}` a new id for the request. Without one, an
    /// agent started this way cannot be stopped from a radio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
}

fn default_launcher() -> String {
    "claude-held".into()
}
/// The launcher every tower used before it could hold an agent itself.
const LAUNCHER_HANDED_OFF: &str = "claude-code";
/// Claude Code in a detached tmux session: an ordinary interactive session he can attach to at his desk
/// (`tmux attach -t <session>`), which reports in through the same hooks as any other.
fn default_launchers() -> BTreeMap<String, Launcher> {
    let list = |words: &[&str]| words.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let claude = Launcher {
        harness: "claude-code".into(),
        argv: list(&["tmux", "new-session", "-d", "-s", "{session}", "-c", "{cwd}", "claude", "{options}", "{task}"]),
        models: ["fable", "opus", "sonnet", "haiku"].iter().map(|m| (m.to_string(), m.to_string())).collect(),
        efforts: list(&["low", "medium", "high", "xhigh", "max"]),
        with_model: list(&["--model", "{model}"]),
        with_effort: list(&["--effort", "{effort}"]),
        held: None,
    };
    // Claude Code run by the tower itself, the way its Agent SDK runs it: one process that takes his messages a
    // line at a time. What he says is his own turn there, and it reports through the same hooks as any other.
    let held = Launcher {
        argv: list(&["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "{options}"]),
        held: Some(Held { fresh: list(&["--session-id", "{id}"]), resume: list(&["--resume", "{id}"]), line: r#"{"type":"user","message":{"role":"user","content":{text}}}"#.into(), stop: Some(r#"{"type":"control_request","request_id":"{id}","request":{"subtype":"interrupt"}}"#.into()) }),
        ..claude.clone()
    };
    BTreeMap::from([(LAUNCHER_HANDED_OFF.to_string(), claude), (default_launcher(), held)])
}
/// The launcher every tower had before a model could be asked for. One still set to exactly this is brought up
/// to date; one he has changed is his and is left alone.
const LAUNCHER_BEFORE_MODELS: &[&str] = &["tmux", "new-session", "-d", "-s", "{session}", "-c", "{cwd}", "claude", "{task}"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredKey {
    /// The provider it is for, by the name its settings use ("gemini", "azure").
    pub provider: String,
    pub secret: String,
}

impl Config {
    /// The launcher for a new agent, or to carry one on. With no harness asked for, the one set as usual.
    /// For a harness, the usual one if it starts that, else one that runs it itself, else any that starts it.
    pub fn launcher_for(&self, harness: Option<&str>) -> Option<(&String, &Launcher)> {
        let usual = self.launchers.get_key_value(&self.launcher);
        match harness {
            None => usual,
            Some(h) => usual.filter(|(_, l)| l.harness == h).or_else(|| self.launchers.iter().find(|(_, l)| l.harness == h && l.held.is_some())).or_else(|| self.launchers.iter().find(|(_, l)| l.harness == h)),
        }
    }
    /// The harnesses a new agent can be started with, the usual one first.
    pub fn tools(&self) -> Vec<String> {
        let mut out: Vec<String> = self.launchers.get(&self.launcher).map(|l| l.harness.clone()).into_iter().collect();
        self.launchers.values().for_each(|l| if !out.contains(&l.harness) { out.push(l.harness.clone()) });
        out.retain(|h| !h.is_empty());
        out
    }
}

/// The places in the settings that use a key.
const USERS: [&str; 4] = ["summarizer", "summarizer_fallback", "voice", "ears"];
const BY_NAME: &str = "key:";

/// In the settings as they are written: each use of a kept key becomes the key's name.
fn name_keys(doc: &mut serde_json::Value) {
    let kept: Vec<(String, String)> = doc["keys"].as_object().map(|m| m.iter().filter_map(|(n, k)| Some((n.clone(), k["secret"].as_str()?.to_string()))).collect()).unwrap_or_default();
    for user in USERS {
        let Some(key) = doc.get_mut(user).and_then(|u| u.get_mut("key")) else { continue };
        if let Some((name, _)) = kept.iter().find(|(_, secret)| !secret.is_empty() && key.as_str() == Some(secret.as_str())) {
            *key = serde_json::Value::String(format!("{BY_NAME}{name}"));
        }
    }
}

/// And as they are read: each name becomes the key again. Err names one that is not kept.
fn fill_keys(doc: &mut serde_json::Value) -> Result<(), String> {
    for user in USERS {
        let Some(name) = doc.get(user).and_then(|u| u["key"].as_str()).and_then(|k| k.strip_prefix(BY_NAME)).map(str::to_string) else { continue };
        let secret = doc["keys"][&name]["secret"].as_str().ok_or_else(|| format!("the settings use a key called \"{name}\" that is not kept; run airband setup"))?.to_string();
        doc[user]["key"] = serde_json::Value::String(secret);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PairedRadio {
    pub name: String,
    /// 32 random bytes as hex, shared with that radio and nobody else, the relay included.
    pub secret: String,
}

/// What reads when nothing else was chosen: Claude Code on this computer, as a command.
fn default_summarizer() -> Summarizer {
    Use::of("claude")
}
/// The command that read before a measurement showed what its tool list cost. Settings still holding it, or
/// the lean one written out in full, are moved to the row that has the lean one.
const HEAVY: [&str; 8] = ["claude", "-p", "--model", "haiku", "--no-session-persistence", "--strict-mcp-config", "--disallowedTools", "Bash,Edit,Write,Read,Agent,WebFetch,WebSearch"];

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

/// Write a file only its owner can read (it holds the token, or sessions' inbox tokens). It is made that way
/// from the start, under another name, and takes its own name once it is whole: nobody else can read it at
/// any moment, and a write that is cut short leaves the old one as it was.
pub fn write_private(path: &PathBuf, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    let part = path.with_extension(format!("part-{}", random_hex(4)));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&part).and_then(|mut f| f.write_all(bytes.as_ref()).and_then(|_| f.sync_all())).and_then(|_| fs::rename(&part, path));
    if written.is_err() {
        let _ = fs::remove_file(&part);
    }
    written
}

/// The config as it is on disk now, or None if there is none yet.
pub fn read() -> Option<Config> {
    parse(&fs::read_to_string(dir().join("config.json")).ok()?).ok()
}

/// What the keychain holds for the tower, read once for each version of the file: every program that changes
/// the keys also writes the file, so a file that has not changed means keys that have not.
fn kept() -> Result<serde_json::Map<String, serde_json::Value>, String> {
    static LAST: std::sync::Mutex<Option<(std::time::SystemTime, serde_json::Map<String, serde_json::Value>)>> = std::sync::Mutex::new(None);
    let stamp = fs::metadata(dir().join("config.json")).and_then(|m| m.modified()).ok();
    let mut last = LAST.lock().unwrap();
    if let (Some(stamp), Some((at, map))) = (stamp, last.as_ref()) {
        if *at == stamp {
            return Ok(map.clone());
        }
    }
    let map = crate::vault::get()?;
    if let Some(stamp) = stamp {
        *last = Some((stamp, map.clone()));
    }
    Ok(map)
}

/// Why this run has the settings without their keys, when it does: the keychain could not be read here.
static KEYLESS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Set when the keys could not be had: what needs one must say so and stop, not go on with a mark for a key.
pub fn keyless() -> Option<String> {
    KEYLESS.lock().unwrap().clone()
}

/// The settings from the file's text, with any secret that is kept in the keychain put back in its place.
/// Where the keychain cannot be read (a shell over SSH) the settings still come back, with marks where the
/// keys are and [`keyless`] saying why: a command that needs no key goes on working.
fn parse(text: &str) -> Result<Config, String> {
    let mut doc: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if crate::vault::sealed(text) {
        match kept() {
            Ok(kept) => crate::vault::open(&mut doc, &kept)?,
            Err(e) => *KEYLESS.lock().unwrap() = Some(e),
        }
    }
    if keyless().is_none() {
        fill_keys(&mut doc)?;
    }
    serde_json::from_value(doc).map_err(|e| e.to_string())
}

/// The file's text for these settings. When the keys are kept in the keychain (`lock`, or the file as it is
/// now already has them there) they go there and the text has a mark in their places.
fn render(cfg: &Config, lock: bool) -> Result<String, String> {
    if let Some(why) = keyless() {
        return Err(format!("the settings cannot be changed from here: {why}"));
    }
    let existing = fs::read_to_string(dir().join("config.json")).unwrap_or_default();
    let mut doc = serde_json::to_value(cfg).unwrap();
    name_keys(&mut doc);
    if !lock && !crate::vault::sealed(&existing) {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()));
    }
    let before = if crate::vault::sealed(&existing) { kept()? } else { Default::default() };
    let now = crate::vault::seal(&mut doc, &before);
    if now != before {
        crate::vault::put(&now)?;
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()))
}

/// Write the settings. The keys never go in the file: they are kept encrypted (`vault`), from the first save.
pub fn save(cfg: &Config) -> Result<(), String> {
    let text = render(cfg, true)?;
    write_private(&dir().join("config.json"), &text).map_err(|e| format!("cannot write ~/.airband/config.json: {e}"))
}

/// Are the keys in the keychain?
pub fn locked() -> bool {
    fs::read_to_string(dir().join("config.json")).is_ok_and(|t| crate::vault::sealed(&t))
}

/// Move the keys into the keychain. The file is only rewritten once the keychain has them and gives them back.
pub fn lock() -> Result<(), String> {
    let cfg = read().ok_or("there is no configuration to lock yet")?;
    let text = render(&cfg, true)?;
    let back = crate::vault::get()?;
    let mut doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    crate::vault::open(&mut doc, &back)?;
    fill_keys(&mut doc)?;
    if serde_json::to_value(&cfg).unwrap() != doc {
        return Err("the keychain did not give back what was put in it; nothing was changed".into());
    }
    write_private(&dir().join("config.json"), &text).map_err(|e| format!("cannot write ~/.airband/config.json: {e}"))
}

/// Read the config, creating it (with a fresh token) on first use.
pub fn load() -> Config {
    let path = dir().join("config.json");
    fs::create_dir_all(dir()).expect("cannot create ~/.airband");
    let existing = fs::read_to_string(&path).ok();
    let mut cfg: Config = match existing.as_deref().map(parse) {
        Some(Ok(cfg)) => cfg,
        None => serde_json::from_str("{}").unwrap(),
        // A config that cannot be read is never replaced: it holds the token, the pairings and the keys. A typo,
        // or a setting written by a newer build, must cost nothing but this message.
        Some(Err(e)) => {
            eprintln!("{} cannot be read: {e}.\nNothing was changed. Fix that line, or move the file aside to start again with a new one (which unpairs every radio).", path.display());
            std::process::exit(2);
        }
    };
    let argv_is = |u: &Use, usual: &[&str]| u.kind == "command" && u.set.get("argv").and_then(|a| a.as_array()).is_some_and(|a| a.iter().map(|v| v.as_str().unwrap_or_default()).eq(usual.iter().copied()));
    if argv_is(&cfg.summarizer, &HEAVY) || argv_is(&cfg.summarizer, match crate::providers::provider("claude").and_then(|p| p.offers.first()).map(|o| o.by) { Some(crate::providers::By::Command(usual)) => usual, _ => &[] }) {
        cfg.summarizer = default_summarizer();
    }
    if cfg.launchers.get(&default_launcher()).is_some_and(|l| l.models.is_empty() && l.argv.iter().map(String::as_str).eq(LAUNCHER_BEFORE_MODELS.iter().copied())) {
        cfg.launchers.extend(default_launchers());
    }
    // a tower set up before it could hold agents itself gets that launcher, and uses it if it was on the old default
    if !cfg.launchers.contains_key(&default_launcher()) {
        if let Some(held) = default_launchers().remove(&default_launcher()) {
            cfg.launchers.insert(default_launcher(), held);
        }
        if cfg.launcher == LAUNCHER_HANDED_OFF {
            cfg.launcher = default_launcher();
        }
    }
    // the two that came with the tower, from before a launcher said what it starts
    for name in [LAUNCHER_HANDED_OFF.to_string(), default_launcher()] {
        if let Some(l) = cfg.launchers.get_mut(&name).filter(|l| l.harness.is_empty()) {
            l.harness = "claude-code".into();
        }
    }
    // the held launcher as first shipped could not stop its agent: one still exactly that gets the line that does
    if let (Some(mine), Some(now)) = (cfg.launchers.get_mut(&default_launcher()).and_then(|l| l.held.as_mut()), default_launchers().remove(&default_launcher()).and_then(|l| l.held)) {
        if mine.stop.is_none() && mine.line == now.line {
            mine.stop = now.stop;
        }
    }
    // a key given before keys had names is kept under its provider's name, so it can be used again and shown
    // (by name only) in `airband keys`
    if keyless().is_none() {
        for used in [serde_json::to_value(&cfg.summarizer), serde_json::to_value(&cfg.voice), serde_json::to_value(&cfg.ears)].into_iter().flatten() {
            let (Some(kind), Some(secret)) = (used["kind"].as_str(), used["key"].as_str().filter(|k| !k.is_empty())) else { continue };
            if !cfg.keys.values().any(|k| k.secret == secret) {
                let name = (1..).map(|n| if n == 1 { kind.to_string() } else { format!("{kind}-{n}") }).find(|n| !cfg.keys.contains_key(n)).unwrap();
                cfg.keys.insert(name, StoredKey { provider: kind.to_string(), secret: secret.to_string() });
            }
        }
    }
    // without the keys nothing is rewritten: the file is left exactly as it is
    let text = match render(&cfg, false) {
        Ok(text) => text,
        Err(_) if keyless().is_some() => return cfg,
        Err(e) => {
            eprintln!("airband: {e}");
            std::process::exit(2);
        }
    };
    if existing.as_deref() != Some(text.as_str()) {
        write_private(&path, &text).expect("cannot write ~/.airband/config.json");
    }
    cfg
}

/// This machine's Tailscale addresses (100.64.0.0/10): the only way in from outside the machine.
fn yes() -> bool {
    true
}

/// This computer's addresses on its local network: the private ranges, and nothing a stranger could route to.
pub fn lan_addrs() -> Vec<std::net::Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(a) if a.is_private() => Some(a),
            _ => None,
        })
        .collect()
}

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
    // asked of the system once: this is called for every answer to a radio
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let host = HOST.get_or_init(|| std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default());
    let short = host.split('.').next().unwrap_or("").to_lowercase();
    // "someones-mac-mini" says more than it needs to: keep what kind of machine it is
    let short = short.rsplit("s-").next().unwrap_or(&short).to_string();
    if short.is_empty() { "computer".into() } else { short }
}
