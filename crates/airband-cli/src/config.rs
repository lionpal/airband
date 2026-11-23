//! ~/.airband: the config (port, token, who to call him, which model summarises), state and log.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The settings. Their `Debug` shows what is plain and leaves every token, key and pairing secret out.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    /// Why these settings came without their keys, when they did: the keychain could not be read where they
    /// were read. Never in the file. What needs a key says so and stops, and these settings are never saved.
    #[serde(skip)]
    pub(crate) keyless: Option<String>,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
    /// The tower's own token: everything on its port, including reporting as an agent. Stays on this machine.
    #[serde(default = "new_token")]
    pub(crate) token: String,
    /// What a radio paired directly is given instead: good for `/v1/radio/` and nothing else, so a pairing link
    /// cannot be used to report as an agent (an agent can name a command for the tower to run).
    #[serde(default = "new_token")]
    pub(crate) radio_token: String,
    /// What sessions are told his name is.
    #[serde(default = "default_name")]
    pub(crate) name: String,
    /// What this computer is called on a phone paired with several. Left out, its short host name is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) station: Option<String>,
    /// Let a paired radio on the same network reach this tower straight, without the relay: faster, and it
    /// works when the relay does not. The tower then listens on this computer's local network addresses, for
    /// sealed requests only. `airband lan off` turns it off.
    #[serde(default = "yes")]
    pub(crate) lan: bool,
    /// What cuts a long message down to a sentence or two: the one he chose, on his phone or in `airband
    /// setup`. Until he has chosen, nothing does, and he gets each message as it was written. When it does not
    /// answer he is told so; no other model is asked in its place, and nothing is ever worked out from the
    /// words themselves in place of a model.
    #[serde(default)]
    pub(crate) summarizer: Summarizer,
    /// The keys he has given, each under the id of the provider it is for. What uses a key (summaries) holds
    /// its value here in memory; the settings file writes that as `key:<name>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) keys: BTreeMap<String, StoredKey>,
    /// The relay radios reach this tower through: airband's own unless he names another (`airband relay <url>`)
    /// or turns it off (`airband relay off`, written as null). With none, a radio reaches this computer at its
    /// own addresses and nowhere else.
    #[serde(default = "default_relay")]
    pub(crate) relay: Option<String>,
    /// The key the relay asks for before it opens a channel, if it asks for one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) relay_admission: Option<String>,
    /// How to start a new agent, by name. The ones for the coding tools airband knows come with it (a tool's
    /// row, `catalog/tools/`), are here whenever the settings are read, and are never written to the file: a
    /// later version's correction to one reaches every computer. The file holds only the ones he added, under
    /// names of their own. To change one that came with airband, copy it under another name and make that the
    /// `launcher`.
    #[serde(default, serialize_with = "his_own", skip_serializing_if = "none_his_own")]
    pub(crate) launchers: BTreeMap<String, Launcher>,
    /// The launcher he has made the usual one, when he has: a new agent is started with it unless he asks for
    /// another tool. Empty until he sets it; the coding tool he uses most (which his phone says) decides then.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) launcher: String,
    /// Projects a new agent may be launched in, by spoken name, beyond the ones the tower has seen agents work
    /// in. Name to folder.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) projects: BTreeMap<String, String>,
    /// How long, in seconds, a harness waits for him to allow or refuse something from the phone before it asks at
    /// the keyboard instead. 0: never ask the phone. Nothing is ever allowed by waiting: no answer is no answer.
    #[serde(default = "default_permission_wait")]
    pub(crate) permission_wait: u64,
    /// His own quick messages, offered after the standard ones: things he finds himself telling agents again and
    /// again. `airband quick add "Make no mistakes"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) quick: Vec<airband_core::radio::Quick>,
    /// One per paired radio. Removing an entry cuts that radio off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pairings: Vec<PairedRadio>,
}

/// What reads is a provider in use: a row of the catalogue, a key, and what is set differently from the row.
/// (What listens and what speaks are the phone's, chosen and run there.)
mod here;
pub(crate) use here::*;
pub(crate) use crate::providers::Use;
pub(crate) type Summarizer = Use;

pub(crate) use airband_core::tools::{Held, Launcher};

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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredKey {
    /// The provider it is for, by the name its settings use ("gemini", "azure").
    pub(crate) provider: String,
    pub(crate) secret: String,
}

impl Config {
    /// The launcher for a new agent, or to carry one on. With no tool asked for, the one he set as usual, and
    /// nothing when he has set none. For a tool: the usual one when that is the tool it starts; otherwise the
    /// one that tool's own row lists first; and for a tool airband has no row for, the one launcher of his own
    /// that starts it. Where he has written several for such a tool and made none the usual, none is picked.
    pub(crate) fn launcher_for(&self, harness: Option<&str>) -> Option<(&String, &Launcher)> {
        let usual = self.launchers.get_key_value(&self.launcher);
        let Some(h) = harness else { return usual };
        usual.filter(|(_, l)| l.harness == h).or_else(|| built_in().find(|l| l.harness == h).and_then(|l| self.launchers.get_key_value(&l.name))).or_else(|| {
            let mut own = self.launchers.iter().filter(|(_, l)| l.harness == h);
            own.next().filter(|_| own.next().is_none())
        })
    }
    /// The key he gave for a provider. The one rule: a provider has one key here, found by the provider it is
    /// for (a key kept by an older version under another name is still found by that).
    pub(crate) fn key_of(&self, provider: &str) -> Option<String> {
        self.keys.values().find(|k| k.provider == provider).map(|k| k.secret.clone())
    }
    /// Keep a provider's key, under the provider's own id. A new one takes the place of the one before, wherever
    /// that one was in use.
    pub(crate) fn keep_key(&mut self, provider: &str, secret: String) {
        let was = self.key_of(provider);
        if self.summarizer.kind == provider && Some(&self.summarizer.key) == was.as_ref() {
            self.summarizer.key = secret.clone();
        }
        self.keys.retain(|_, k| k.provider != provider);
        self.keys.insert(provider.to_string(), StoredKey { provider: provider.to_string(), secret });
    }
    /// The harnesses a new agent can be started with, the usual one first.
    pub(crate) fn tools(&self) -> Vec<String> {
        let mut out: Vec<String> = self.launchers.get(&self.launcher).map(|l| l.harness.clone()).into_iter().collect();
        self.launchers.values().for_each(|l| if !out.contains(&l.harness) { out.push(l.harness.clone()) });
        out.retain(|h| !h.is_empty());
        out
    }
}

/// The places in the settings that use a key.
pub(crate) const USERS: [&str; 1] = ["summarizer"];
/// What a use of a kept key is written as, before the key's name.
pub(crate) const BY_NAME: &str = "key:";

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

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct PairedRadio {
    pub(crate) name: String,
    /// 32 random bytes as hex, shared with that radio and nobody else, the relay included.
    pub(crate) secret: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("port", &self.port).field("name", &self.name).field("station", &self.station).field("relay", &self.relay).field("pairings", &self.pairings).finish_non_exhaustive()
    }
}
impl std::fmt::Debug for StoredKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredKey").field("provider", &self.provider).finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PairedRadio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedRadio").field("name", &self.name).finish_non_exhaustive()
    }
}

/// airband's own relay. It is open to anyone, sees only sealed messages, and limits how much each may ask.
pub(crate) const RELAY: &str = "https://relay.airband.dev";
fn default_relay() -> Option<String> {
    Some(RELAY.into())
}
/// The port the tower listens on unless the settings say another.
pub(crate) const DEFAULT_PORT: u16 = 7373;
fn default_port() -> u16 {
    DEFAULT_PORT
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
pub(crate) use airband_core::encode::random_hex;
fn default_permission_wait() -> u64 {
    180
}
fn new_token() -> String {
    random_hex(24)
}

/// His home folder. A test never gets his real one: it gets a folder of its own under the system's temporary
/// one, or the one it put in place (`tests_home::set`).
pub(crate) fn home() -> PathBuf {
    #[cfg(test)]
    return tests_home::get();
    #[cfg(not(test))]
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).expect("no home directory (HOME or USERPROFILE)")
}
pub(crate) fn dir() -> PathBuf {
    home().join(".airband")
}

#[cfg(test)]
pub(crate) mod tests_home {
    use std::path::PathBuf;
    thread_local!(static HOME: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) });
    /// Give this test thread a home of its own.
    pub(crate) fn set(home: PathBuf) {
        HOME.with(|h| *h.borrow_mut() = Some(home));
    }
    pub(crate) fn get() -> PathBuf {
        HOME.with(|h| h.borrow().clone()).unwrap_or_else(|| std::env::temp_dir().join(format!("airband-test-home-{}", std::process::id())))
    }
}

pub(crate) use airband_io::files::write_private;

/// The config as it is on disk now, or None if there is none yet.
/// How a hook or a report reaches the tower here: its port and its own token, read from the file alone. The keys
/// are not opened for this (every hook would otherwise read the keychain to send one event).
pub(crate) struct Reach {
    pub(crate) port: u16,
    pub(crate) token: String,
}

/// The tower's port and token as the settings file has them. None when there is no file or it holds no token.
pub(crate) fn reach() -> Result<Option<Reach>, String> {
    #[derive(Deserialize)]
    struct Plain {
        #[serde(default = "default_port")]
        port: u16,
        #[serde(default)]
        token: String,
    }
    let path = file();
    let Some(text) = airband_io::files::read_text(&path).map_err(|e| format!("{} cannot be read: {e}", path.display()))? else { return Ok(None) };
    let plain: Plain = serde_json::from_str(&text).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
    Ok((!plain.token.is_empty()).then_some(Reach { port: plain.port, token: plain.token }))
}

/// Where the settings are kept.
pub(crate) fn file() -> PathBuf {
    dir().join("config.json")
}

/// The settings in this file: None when there is none, and why when it cannot be read.
pub(crate) fn read_from(path: &Path) -> Result<Option<Config>, String> {
    match airband_io::files::read_text(path).map_err(|e| e.to_string())? {
        None => Ok(None),
        Some(text) => parse(&text, path).map(Some),
    }
}

/// When this file was last changed, or None when there is none.
pub(crate) fn stamp_of(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// What the keychain holds for the tower, read once for each version of the file: every program that changes
/// the keys also writes the file, so a file that has not changed means keys that have not.
fn kept(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    type Last = Option<(PathBuf, std::time::SystemTime, serde_json::Map<String, serde_json::Value>)>;
    static LAST: std::sync::Mutex<Last> = std::sync::Mutex::new(None);
    let stamp = stamp_of(path);
    let mut last = LAST.lock().unwrap();
    if let (Some(stamp), Some((of, at, map))) = (stamp, last.as_ref())
        && of == path && *at == stamp {
            return Ok(map.clone());
        }
    let map = crate::vault::get()?;
    if let Some(stamp) = stamp {
        *last = Some((path.to_path_buf(), stamp, map.clone()));
    }
    Ok(map)
}

/// The settings from the file's text, with any secret that is kept in the keychain put back in its place.
/// Where the keychain cannot be read (a shell over SSH) the settings still come back, with marks where the
/// keys are and [`Config::keyless`] saying why: a command that needs no key goes on working.
fn parse(text: &str, path: &Path) -> Result<Config, String> {
    let mut doc: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let missing = match crate::vault::sealed(text).then(|| kept(path)) {
        Some(Ok(kept)) => {
            crate::vault::open(&mut doc, &kept)?;
            None
        }
        Some(Err(e)) => Some(e),
        None => None,
    };
    if missing.is_none() {
        fill_keys(&mut doc)?;
    }
    // a launcher in the file under a name airband uses is not read at all: that name is airband's own
    if let Some(his) = doc.get_mut("launchers").and_then(|l| l.as_object_mut()) {
        his.retain(|name, _| !built_in().any(|b| b.name == *name));
    }
    let mut cfg: Config = serde_json::from_value(doc).map_err(|e| e.to_string())?;
    cfg.launchers = with_built_in(std::mem::take(&mut cfg.launchers));
    cfg.keyless = missing;
    Ok(cfg)
}

/// The file's text for these settings. The keys are never in it: they are kept encrypted (`vault`) and the
/// text has a mark in each of their places, from the first time the file is written.
fn render(cfg: &Config, path: &Path) -> Result<String, String> {
    if let Some(why) = &cfg.keyless {
        return Err(format!("the settings cannot be changed from here: {why}"));
    }
    // a file that is there and cannot be read is never taken as one without keys: that would write over it
    let existing = airband_io::files::read_text(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?.unwrap_or_default();
    let mut doc = serde_json::to_value(cfg).unwrap();
    name_keys(&mut doc);
    let before = if crate::vault::sealed(&existing) { kept(path)? } else { Default::default() };
    let now = crate::vault::seal(&mut doc, &before);
    if now != before {
        crate::vault::put(&now)?;
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()))
}

/// Write the settings. The keys never go in the file: they are kept encrypted (`vault`), from the first save.
/// Held while the settings are read, changed and written, by every airband process: a change made from
/// another one in between is never written over.
pub(crate) fn hold() -> Result<crate::lock::Held, String> {
    hold_at(&file())
}
fn hold_at(path: &Path) -> Result<crate::lock::Held, String> {
    crate::lock::take(&path.with_extension("lock"))
}

/// Change the settings as the file has them now: held, read, changed by `f` and written in one go, so a change
/// another airband process made since this one read them is kept. A file that cannot be read, or settings
/// whose keys cannot be read, are never written: nothing changes and he is told why.
pub(crate) fn change<R>(f: impl FnOnce(&mut Config) -> Result<R, String>) -> Result<R, String> {
    change_at(&file(), f)
}
fn change_at<R>(path: &Path, f: impl FnOnce(&mut Config) -> Result<R, String>) -> Result<R, String> {
    let held = hold_at(path)?;
    let mut cfg = match existing(path)? {
        Some((_, cfg)) => cfg,
        None => parse("{}", path)?,
    };
    if let Some(why) = &cfg.keyless {
        return Err(format!("the settings cannot be changed from here: {why}"));
    }
    let made = f(&mut cfg)?;
    save_at(&cfg, path, &held)?;
    Ok(made)
}

/// Write the settings, for one that already holds the file.
pub(crate) fn save_held(cfg: &Config, held: &crate::lock::Held) -> Result<(), String> {
    save_at(cfg, &file(), held)
}
fn save_at(cfg: &Config, path: &Path, _: &crate::lock::Held) -> Result<(), String> {
    let text = render(cfg, path)?;
    write_private(path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// What is in the settings file: `None` when there is no file yet. A file that is there and cannot be read or
/// parsed is an error, never "no file": it holds the token, the pairings and the keys, and is never replaced.
fn existing(path: &std::path::Path) -> Result<Option<(String, Config)>, String> {
    let unread = |e: String| format!("{} cannot be read: {e}.\nNothing was changed. Fix it, or move the file aside to start again with a new one (which unpairs every radio).", path.display());
    match airband_io::files::read_text(path) {
        Ok(None) => Ok(None),
        Ok(Some(text)) => parse(&text, path).map(|cfg| Some((text, cfg))).map_err(unread),
        Err(e) => Err(unread(e.to_string())),
    }
}

/// Read the config, creating it (with a fresh token) on first use.
pub(crate) fn load() -> Config {
    let path = file();
    airband_io::files::make_folder(&dir()).expect("cannot create ~/.airband");
    // read and, when it is new or out of date, written while no other airband process is changing it
    let _held = hold().unwrap_or_else(|why| {
        eprintln!("airband: {why}");
        std::process::exit(2);
    });
    let (existing, cfg) = match existing(&path) {
        Ok(Some((text, cfg))) => (Some(text), cfg),
        Ok(None) => (None, parse("{}", &path).unwrap()),
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };
    // without the keys nothing is rewritten: the file is left exactly as it is
    let text = match render(&cfg, &path) {
        Ok(text) => text,
        Err(_) if cfg.keyless.is_some() => return cfg,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hook_reaches_the_tower_by_port_and_token_alone_and_a_file_it_cannot_read_says_why() {
        let home = std::env::temp_dir().join(format!("airband-reach-{}", random_hex(4)));
        tests_home::set(home.clone());
        assert!(reach().unwrap().is_none(), "no file: nothing to reach");
        std::fs::create_dir_all(dir()).unwrap();
        // the keys' places hold marks the vault would have to open; reaching the tower never asks it
        std::fs::write(file(), serde_json::json!({ "port": 7400, "token": "own", "radio_token": crate::vault::MARK }).to_string()).unwrap();
        let r = reach().unwrap().unwrap();
        assert_eq!((r.port, r.token.as_str()), (7400, "own"));
        std::fs::write(file(), "not json").unwrap();
        assert!(reach().is_err());
        assert!(read_from(&file()).is_err(), "why it cannot be read is said, never taken as no file");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_settings_debug_shows_no_token_key_or_pairing_secret() {
        let cfg: Config = serde_json::from_value(serde_json::json!({ "token": "TOKEN-ONE", "radio_token": "TOKEN-TWO", "pairings": [{ "name": "phone", "secret": "SECRET-THREE" }] })).unwrap();
        let shown = format!("{cfg:?} {:?}", StoredKey { provider: "gemini".into(), secret: "SECRET-FOUR".into() });
        assert!(shown.contains("phone") && shown.contains("gemini"));
        for secret in ["TOKEN-ONE", "TOKEN-TWO", "SECRET-THREE", "SECRET-FOUR"] {
            assert!(!shown.contains(secret), "{secret} is shown");
        }
    }

    #[test]
    fn a_change_is_made_to_the_settings_as_the_file_has_them_then() {
        let dir = std::env::temp_dir().join(format!("airband-change-{}", random_hex(6)));
        airband_io::files::make_folder(&dir).unwrap();
        // its keys are kept beside it, never with his
        crate::vault::tests_folder::set(dir.clone());
        let path = dir.join("config.json");
        change_at(&path, |c| {
            c.pairings.push(PairedRadio { name: "phone".into(), secret: "s1".into() });
            Ok(())
        })
        .unwrap();
        // read by one command, then a pairing made by another before the first one writes
        let stale = read_from(&path).unwrap().unwrap();
        change_at(&path, |c| {
            c.pairings.push(PairedRadio { name: "pad".into(), secret: "s2".into() });
            Ok(())
        })
        .unwrap();
        change_at(&path, |c| {
            c.lan = !stale.lan;
            Ok(())
        })
        .unwrap();
        let now = read_from(&path).unwrap().unwrap();
        assert_eq!(now.pairings.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["phone", "pad"], "the pairing made in between is kept");
        assert_eq!(now.lan, !stale.lan);
        // a change that refuses writes nothing
        let before = airband_io::files::read_text(&path).unwrap();
        assert!(change_at(&path, |_| Err::<(), _>("no".to_string())).is_err());
        assert_eq!(airband_io::files::read_text(&path).unwrap(), before);
        airband_io::files::remove_folder(&dir).ok();
    }

    #[test]
    fn a_settings_file_that_cannot_be_read_is_an_error_never_no_file() {
        let dir = std::env::temp_dir().join(format!("airband-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        assert!(existing(&path).unwrap().is_none(), "no file is no file");
        std::fs::write(&path, b"{\"token\": \"\xff\"}").unwrap();
        assert!(existing(&path).is_err(), "a byte that is not text is an error, not an empty file");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\": \"\xff\"}", "and the file is left as it was");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(existing(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keys_that_could_not_be_had_once_do_not_block_every_later_read() {
        // settings read without their keys are never saved; the next read, which has them, is its own
        let mut without = parse("{}", &file()).unwrap();
        without.keyless = Some("the keychain was locked".into());
        assert!(render(&without, &file()).is_err());
        assert_eq!(parse("{}", &file()).unwrap().keyless, None);
    }

    #[test]
    fn nothing_is_chosen_until_he_chooses_it() {
        let cfg = parse("{}", &file()).unwrap();
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
        let mut cfg = parse("{}", &file()).unwrap();
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
