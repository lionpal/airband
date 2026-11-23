//! ~/.airband: the config (port, token, who to call him, which model summarises), state and log.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    #[serde(default = "airband_core::yes")]
    pub(crate) lan: bool,
    /// What cuts a long message down to a sentence or two: the one he chose, on his phone or in `airband
    /// setup`. Until he has chosen, nothing does, and he gets each message as it was written. When it does not
    /// answer he is told so; no other model is asked in its place, and nothing is ever worked out from the
    /// words themselves in place of a model.
    #[serde(default)]
    pub(crate) summarizer: Summarizer,
    /// How the model that writes his summaries is to write them, in his own words, set on his phone; nothing
    /// for the usual text (`airband_core::summary`). Only that part of the question is his. Never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary_brief: Option<String>,
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
    /// His usual launcher, when he has one: a new agent is started with it when it can start what he asked for
    /// (`launch::starter`). Set by his answer the first time he is asked which should start one, or by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) launcher: Option<airband_core::ids::LauncherId>,
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
    /// The provider it is for, by the name its row in `catalog/providers/` gives it.
    pub(crate) provider: String,
    pub(crate) secret: String,
}

impl Config {
    /// Whether the keys were read with these settings: the one refusal, for whatever needs them.
    pub(crate) fn keys_read(&self) -> Result<(), String> {
        match &self.keyless {
            Some(why) => Err(format!("airband's keys are in the keychain and could not be read here ({why})")),
            None => Ok(()),
        }
    }
    /// The key he gave for a provider. The one rule: a provider has one key here, kept under the provider's id.
    pub(crate) fn key_of(&self, provider: &str) -> Option<String> {
        self.keys.get(provider).map(|k| k.secret.clone())
    }
    /// Keep a provider's key, under the provider's own id. A new one takes the place of the one before, wherever
    /// that one was in use.
    pub(crate) fn keep_key(&mut self, provider: &str, secret: String) {
        let was = self.key_of(provider);
        if self.summarizer.kind == provider && Some(&self.summarizer.key) == was.as_ref() {
            self.summarizer.key = secret.clone();
        }
        self.keys.insert(provider.to_string(), StoredKey { provider: provider.to_string(), secret });
    }
    /// The harnesses a new agent can be started with, each once. Which starts one is `launch::starter`'s alone to
    /// say, and never by where it stands here.
    pub(crate) fn tools(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.launchers.values().for_each(|l| if !out.contains(&l.harness) { out.push(l.harness.clone()) });
        out.retain(|h| !h.is_empty());
        out
    }
}

mod keys;
pub(crate) use keys::*;

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

mod file;
pub(crate) use file::*;

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
        let shown = format!("{cfg:?} {:?}", StoredKey { provider: "maker".into(), secret: "SECRET-FOUR".into() });
        assert!(shown.contains("phone") && shown.contains("maker"));
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
    #[cfg(unix)]
    fn a_save_whose_file_cannot_be_written_leaves_every_key_its_file_marks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-cfg-keys-{}", random_hex(4)));
        std::fs::create_dir_all(dir.join("vault")).unwrap();
        crate::vault::tests_folder::set(dir.join("vault"));
        let path = dir.join("config.json");
        let key = |provider: &str| StoredKey { provider: provider.into(), secret: format!("sk-{provider}-0123456789") };
        let mut cfg = parse("{}", &path).unwrap();
        cfg.keys.insert("one".into(), key("one"));
        cfg.keys.insert("two".into(), key("two"));
        save_at(&cfg, &path, &hold_at(&path).unwrap()).unwrap();
        let first = crate::vault::get().unwrap().len();
        // "two" is dropped, and the file cannot be written: the file still marks it, so it is still kept
        cfg.keys.remove("two");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let failed = save_at(&cfg, &path, &hold_at(&path).unwrap());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failed.is_err());
        let kept = parse(&std::fs::read_to_string(&path).unwrap(), &path).expect("every key its file marks is there");
        assert_eq!(kept.keys.len(), 2);
        assert_eq!(crate::vault::get().unwrap().len(), first);
        // written: only what it marks is kept
        save_at(&cfg, &path, &hold_at(&path).unwrap()).unwrap();
        assert_eq!(crate::vault::get().unwrap().len(), first - 1);
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
        assert!(cfg.summarizer.is_none() && cfg.launcher.is_none());
        // his usual, once he has one, is read and written as its id
        let cfg = parse(r#"{"launcher":"mine"}"#, &file()).unwrap();
        assert_eq!(cfg.launcher, Some(airband_core::ids::LauncherId::new("mine")));
        assert_eq!(serde_json::to_value(&cfg).unwrap()["launcher"], "mine");
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
