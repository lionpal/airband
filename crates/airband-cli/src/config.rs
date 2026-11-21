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
    /// The relay radios reach this tower through: airband's own unless he names another (`airband relay <url>`)
    /// or turns it off (`airband relay off`, written as null). With none, a radio reaches this computer at its
    /// own addresses and nowhere else.
    #[serde(default = "default_relay")]
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
    /// How to start a new agent, by name. The ones for the coding tools airband knows come with it (a tool's
    /// row, `catalog/tools/`), are here whenever the settings are read, and are never written to the file: a
    /// later version's correction to one reaches every computer. The file holds only the ones he added, under
    /// names of their own. To change one that came with airband, copy it under another name and make that the
    /// `launcher`.
    #[serde(default, serialize_with = "his_own", skip_serializing_if = "none_his_own")]
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

/// What reads, what speaks and what listens are each a provider in use: a row of the catalogue, a
/// key, and what is set differently from the row.
pub use crate::providers::Use;
pub type Summarizer = Use;
pub type Voice = Use;
pub type Ears = Use;

pub use airband_core::tools::{Held, Launcher};

/// The launchers that come with airband: every coding tool's, in the order the tools are listed, each tool's
/// preferred one first.
fn built_in() -> impl Iterator<Item = &'static Launcher> {
    airband_core::catalog::current().tools.iter().flat_map(|t| t.launchers.iter())
}
/// The settings as read from the file, made whole: his own launchers, and the ones that come with airband.
fn with_built_in(own: BTreeMap<String, Launcher>) -> BTreeMap<String, Launcher> {
    let mut all: BTreeMap<String, Launcher> = own.into_iter().map(|(name, l)| (name.clone(), Launcher { name, ..l })).collect();
    all.extend(built_in().map(|l| (l.name.clone(), l.clone())));
    all
}
fn none_his_own(all: &BTreeMap<String, Launcher>) -> bool {
    all.keys().all(|name| built_in().any(|b| b.name == *name))
}
fn his_own<S: serde::Serializer>(all: &BTreeMap<String, Launcher>, to: S) -> Result<S::Ok, S::Error> {
    // the name is the key it is written under
    to.collect_map(all.iter().filter(|(name, _)| !built_in().any(|b| b.name == **name)).map(|(name, l)| (name, Launcher { name: String::new(), ..l.clone() })))
}

/// The launcher a new agent is started with until he sets another: the first that the tower holds itself.
fn default_launcher() -> String {
    built_in().find(|l| l.held.is_some()).or_else(|| built_in().next()).map(|l| l.name.clone()).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredKey {
    /// The provider it is for, by the name its settings use ("gemini", "azure").
    pub provider: String,
    pub secret: String,
}

impl Config {
    /// The launcher for a new agent, or to carry one on. With no tool asked for, the one set as usual. For a
    /// tool: the usual one if it starts that tool; else the first of his own that does; else the first in
    /// that tool's own row. Nothing when no launcher starts that tool.
    pub fn launcher_for(&self, harness: Option<&str>) -> Option<(&String, &Launcher)> {
        let usual = self.launchers.get_key_value(&self.launcher);
        let Some(h) = harness else { return usual };
        let named = |name: &String| self.launchers.get_key_value(name);
        usual
            .filter(|(_, l)| l.harness == h)
            .or_else(|| self.launchers.iter().find(|(name, l)| l.harness == h && !built_in().any(|b| b.name == **name)))
            .or_else(|| built_in().find(|l| l.harness == h).and_then(|l| named(&l.name)))
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
/// airband's own relay. It is open to anyone, sees only sealed messages, and limits how much each may ask.
pub const RELAY: &str = "https://relay.airband.dev";
fn default_relay() -> Option<String> {
    Some(RELAY.into())
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
    // a launcher in the file under a name airband uses is not read at all: that name is airband's own
    if let Some(his) = doc.get_mut("launchers").and_then(|l| l.as_object_mut()) {
        his.retain(|name, _| !built_in().any(|b| b.name == *name));
    }
    let mut cfg: Config = serde_json::from_value(doc).map_err(|e| e.to_string())?;
    cfg.launchers = with_built_in(std::mem::take(&mut cfg.launchers));
    Ok(cfg)
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
    let cfg: Config = match existing.as_deref().map(parse) {
        Some(Ok(cfg)) => cfg,
        None => parse("{}").unwrap(),
        // A config that cannot be read is never replaced: it holds the token, the pairings and the keys. A typo,
        // or a setting written by a newer build, must cost nothing but this message.
        Some(Err(e)) => {
            eprintln!("{} cannot be read: {e}.\nNothing was changed. Fix that line, or move the file aside to start again with a new one (which unpairs every radio).", path.display());
            std::process::exit(2);
        }
    };
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
