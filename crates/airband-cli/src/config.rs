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
    /// What cuts a long message down to a sentence or two: the one he chose, on his phone or in `airband
    /// setup`. Until he has chosen, nothing does, and he gets each message as it was written. When it does not
    /// answer he is told so; no other model is asked in its place, and nothing is ever worked out from the
    /// words themselves in place of a model.
    #[serde(default)]
    pub summarizer: Summarizer,
    /// The keys he has given, each under the id of the provider it is for. What uses a key (summaries) holds
    /// its value here in memory; the settings file writes that as `key:<name>`.
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
    /// How to start a new agent, by name. The ones for the coding tools airband knows come with it (a tool's
    /// row, `catalog/tools/`), are here whenever the settings are read, and are never written to the file: a
    /// later version's correction to one reaches every computer. The file holds only the ones he added, under
    /// names of their own. To change one that came with airband, copy it under another name and make that the
    /// `launcher`.
    #[serde(default, serialize_with = "his_own", skip_serializing_if = "none_his_own")]
    pub launchers: BTreeMap<String, Launcher>,
    /// The launcher he has made the usual one, when he has: a new agent is started with it unless he asks for
    /// another tool. Empty until he sets it; the coding tool he uses most (which his phone says) decides then.
    #[serde(default, skip_serializing_if = "String::is_empty")]
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

/// What reads is a provider in use: a row of the catalogue, a key, and what is set differently from the row.
/// (What listens and what speaks are the phone's, chosen and run there.)
pub use crate::providers::Use;
pub type Summarizer = Use;

pub use airband_core::tools::{Held, Launcher};

/// The launchers that come with airband: every coding tool's, in the order the tools are listed, each tool's
/// preferred one first.
fn built_in() -> impl Iterator<Item = &'static Launcher> {
    // always this build's own: a catalogue taken later never brings or changes a program to run
    airband_core::catalog::built().tools.iter().flat_map(|t| t.launchers.iter())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredKey {
    /// The provider it is for, by the name its settings use ("gemini", "azure").
    pub provider: String,
    pub secret: String,
}

impl Config {
    /// The launcher for a new agent, or to carry one on. With no tool asked for, the one he set as usual, and
    /// nothing when he has set none. For a tool: the usual one when that is the tool it starts; otherwise the
    /// one that tool's own row lists first; and for a tool airband has no row for, the one launcher of his own
    /// that starts it. Where he has written several for such a tool and made none the usual, none is picked.
    pub fn launcher_for(&self, harness: Option<&str>) -> Option<(&String, &Launcher)> {
        let usual = self.launchers.get_key_value(&self.launcher);
        let Some(h) = harness else { return usual };
        usual.filter(|(_, l)| l.harness == h).or_else(|| built_in().find(|l| l.harness == h).and_then(|l| self.launchers.get_key_value(&l.name))).or_else(|| {
            let mut own = self.launchers.iter().filter(|(_, l)| l.harness == h);
            own.next().filter(|_| own.next().is_none())
        })
    }
    /// The key he gave for a provider. The one rule: a provider has one key here, found by the provider it is
    /// for (a key kept by an older version under another name is still found by that).
    pub fn key_of(&self, provider: &str) -> Option<String> {
        self.keys.values().find(|k| k.provider == provider).map(|k| k.secret.clone())
    }
    /// Keep a provider's key, under the provider's own id. A new one takes the place of the one before, wherever
    /// that one was in use.
    pub fn keep_key(&mut self, provider: &str, secret: String) {
        let was = self.key_of(provider);
        if self.summarizer.kind == provider && Some(&self.summarizer.key) == was.as_ref() {
            self.summarizer.key = secret.clone();
        }
        self.keys.retain(|_, k| k.provider != provider);
        self.keys.insert(provider.to_string(), StoredKey { provider: provider.to_string(), secret });
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
const USERS: [&str; 1] = ["summarizer"];
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

/// airband's own relay. It is open to anyone, sees only sealed messages, and limits how much each may ask.
pub const RELAY: &str = "https://relay.airband.dev";
fn default_relay() -> Option<String> {
    Some(RELAY.into())
}
fn default_port() -> u16 {
    7373
}
/// What his agents are told his name is, until he gives another in `airband setup`: the name this computer's
/// own account has for him.
fn default_name() -> String {
    // the account's full name when it has one, and its login name when it has none
    account().map(|a| if a.full.is_empty() { a.login } else { a.full }).unwrap_or_default()
}
/// This computer's account that airband runs as, as the system has it (not as the environment says, which a
/// test or a tool may have changed).
pub(crate) struct Account {
    pub(crate) login: String,
    pub(crate) full: String,
    pub(crate) home: String,
}
#[cfg(unix)]
pub(crate) fn account() -> Option<Account> {
    let text = |p: *const libc::c_char| if p.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned() };
    let pw = unsafe { libc::getpwuid(libc::getuid()) };
    if pw.is_null() {
        return None;
    }
    let (login, gecos, home) = unsafe { (text((*pw).pw_name), text((*pw).pw_gecos), text((*pw).pw_dir)) };
    Some(Account { login, full: gecos.split(',').next().unwrap_or_default().trim().to_string(), home })
}
#[cfg(not(unix))]
pub(crate) fn account() -> Option<Account> {
    None
}
pub use airband_core::encode::random_hex;
fn default_permission_wait() -> u64 {
    180
}
fn new_token() -> String {
    random_hex(24)
}

/// The tower on this computer, asked with its own token. Every status comes back as an answer, for the caller
/// to read; only a tower that cannot be reached is an error. The one way the command line and the hooks reach it.
pub(crate) struct Tower {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

impl Tower {
    pub(crate) fn here(cfg: &Config, timeout: std::time::Duration) -> Tower {
        Tower { agent: airband_core::net::agent(timeout), base: format!("http://127.0.0.1:{}", cfg.port), auth: format!("Bearer {}", cfg.token) }
    }
    pub(crate) fn get(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent.get(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
    pub(crate) fn post(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.agent.post(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
    pub(crate) fn delete(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent.delete(format!("{}{path}", self.base)).header("authorization", &self.auth)
    }
}

/// A request's answer, when the tower took it (200).
pub(crate) fn taken(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Option<ureq::http::Response<ureq::Body>> {
    res.ok().filter(|r| r.status().as_u16() == 200)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).expect("no home directory (HOME or USERPROFILE)")
}
pub fn dir() -> PathBuf {
    home().join(".airband")
}

pub use airband_core::kept::write_private;

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
    if let (Some(stamp), Some((at, map))) = (stamp, last.as_ref())
        && *at == stamp {
            return Ok(map.clone());
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

/// The file's text for these settings. The keys are never in it: they are kept encrypted (`vault`) and the
/// text has a mark in each of their places, from the first time the file is written.
fn render(cfg: &Config) -> Result<String, String> {
    if let Some(why) = keyless() {
        return Err(format!("the settings cannot be changed from here: {why}"));
    }
    let existing = fs::read_to_string(dir().join("config.json")).unwrap_or_default();
    let mut doc = serde_json::to_value(cfg).unwrap();
    name_keys(&mut doc);
    let before = if crate::vault::sealed(&existing) { kept()? } else { Default::default() };
    let now = crate::vault::seal(&mut doc, &before);
    if now != before {
        crate::vault::put(&now)?;
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()))
}

/// Write the settings. The keys never go in the file: they are kept encrypted (`vault`), from the first save.
pub fn save(cfg: &Config) -> Result<(), String> {
    let text = render(cfg)?;
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
    let text = match render(&cfg) {
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

/// This machine's addresses in the shared address space, 100.64.0.0/10 (RFC 6598): the range a private
/// network laid over the internet gives its machines. The tower listens on these besides the machine itself.
/// Which product made the network is not known here and not asked.
pub fn overlay_addrs() -> Vec<std::net::Ipv4Addr> {
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
    // Asked of the system once: this is called for every answer to a radio. A Mac has a name he gave it
    // ("Charles's Mac mini"), which is what he knows it by; elsewhere the host's own name, up to its first dot.
    // The name is used as it is given: nothing is cut out of it.
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let said = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|name| !name.is_empty());
    let short = HOST
        .get_or_init(|| {
            let given = if cfg!(target_os = "macos") { said("scutil", &["--get", "ComputerName"]) } else { None };
            given.or_else(|| said("hostname", &[]).map(|host| host.split('.').next().unwrap_or_default().to_string())).unwrap_or_default()
        })
        .clone();
    if short.is_empty() { "computer".into() } else { short }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_chosen_until_he_chooses_it() {
        let cfg = parse("{}").unwrap();
        // no model writes summaries and no launcher is the usual one
        assert!(cfg.summarizer.is_none() && cfg.launcher.is_empty());
        assert!(cfg.launcher_for(None).is_none());
        // a tool he names is started the way its own row lists first, and a tool there is no row for is not started
        let first = airband_core::catalog::built().tools.first().unwrap();
        assert_eq!(cfg.launcher_for(Some(&first.id)).map(|(name, _)| name.clone()), first.launchers.first().map(|l| l.name.clone()));
        assert!(cfg.launcher_for(Some("no-such-tool")).is_none());
        // one of his own is used only by being made the usual one
        let mut cfg = cfg;
        cfg.launchers.insert("mine".into(), Launcher { name: "mine".into(), harness: first.id.clone(), ..Launcher::default() });
        assert_eq!(cfg.launcher_for(Some(&first.id)).map(|(name, _)| name.clone()), first.launchers.first().map(|l| l.name.clone()));
        // a tool airband has no row for is started by the one launcher he wrote for it, and by none of two
        cfg.launchers.insert("other".into(), Launcher { name: "other".into(), harness: "his-own-tool".into(), ..Launcher::default() });
        assert_eq!(cfg.launcher_for(Some("his-own-tool")).map(|(name, _)| name.as_str()), Some("other"));
        cfg.launchers.insert("other-two".into(), Launcher { name: "other-two".into(), harness: "his-own-tool".into(), ..Launcher::default() });
        assert!(cfg.launcher_for(Some("his-own-tool")).is_none());
        cfg.launcher = "mine".into();
        assert_eq!(cfg.launcher_for(Some(&first.id)).map(|(name, _)| name.as_str()), Some("mine"));
        assert_eq!(cfg.launcher_for(None).map(|(name, _)| name.as_str()), Some("mine"));
    }

    #[test]
    fn a_provider_has_one_key_and_a_new_one_takes_its_place() {
        let mut cfg = parse("{}").unwrap();
        assert_eq!(cfg.key_of("maker"), None);
        cfg.keep_key("maker", "first".into());
        cfg.summarizer = Use { kind: "maker".into(), offer: "small".into(), key: "first".into(), set: Default::default() };
        cfg.keep_key("maker", "second".into());
        assert_eq!((cfg.key_of("maker").as_deref(), cfg.keys.len(), cfg.summarizer.key.as_str()), (Some("second"), 1, "second"));
        // another provider's key is its own
        cfg.keep_key("other", "third".into());
        assert_eq!((cfg.key_of("maker").as_deref(), cfg.key_of("other").as_deref()), (Some("second"), Some("third")));
    }
}
