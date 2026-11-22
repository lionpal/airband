//! The tower's settings while it runs: the one way to them. They are read from the file again whenever it has
//! changed (`airband pair`, `airband keys` and `airband setup` write it from other processes), and changed
//! here by writing the file first and only then using the new settings, so the tower never uses settings it
//! could not keep.
//!
//! Settings read without their keys (the keychain could not be read this time) are never put in place of ones
//! that have them: the tower goes on with the last it had, and says why it could not read them again.
use super::*;
use crate::providers::Trouble;
use std::sync::RwLock;
use std::path::PathBuf;
use std::time::SystemTime;

pub(crate) struct Settings {
    now: RwLock<Now>,
    /// Why what reads could not, the last time it was asked: said to him on his phone, and gone when it next
    /// answers. Nothing else is asked in its place.
    failed: Mutex<Option<Trouble>>,
}

struct Now {
    cfg: Arc<Config>,
    /// The file's stamp when `cfg` was read or written: a file with another one is read again.
    stamp: Option<SystemTime>,
    /// Why the file could not be read again, the last time it was tried: nothing while it could.
    unread: Option<String>,
    /// The file these settings are read from: none for settings made for a test, which are never read again.
    file: Option<PathBuf>,
}

impl Settings {
    /// The settings the tower started with, read with their keys.
    pub(crate) fn new(cfg: Config) -> Settings {
        Settings { now: RwLock::new(Now { cfg: Arc::new(cfg), stamp: config::stamp_of(&config::file()), unread: None, file: Some(config::file()) }), failed: Mutex::new(None) }
    }

    /// The settings now: the file's, when it has changed since they were last read and could be read with its
    /// keys; the last ones the tower had otherwise.
    pub(crate) fn current(&self) -> Arc<Config> {
        let Some(file) = self.now.read().unwrap().file.clone() else { return self.now.read().unwrap().cfg.clone() };
        let stamp = config::stamp_of(&file);
        {
            let now = self.now.read().unwrap();
            if now.stamp == stamp {
                return now.cfg.clone();
            }
        }
        let mut now = self.now.write().unwrap();
        if now.stamp != stamp {
            match config::read_from(&file) {
                Some(cfg) if cfg.keyless.is_none() => {
                    now.cfg = Arc::new(cfg);
                    now.unread = None;
                }
                Some(cfg) => {
                    let why = cfg.keyless.unwrap_or_default();
                    if now.unread.as_deref() != Some(why.as_str()) {
                        log(format!("the settings changed, and their keys could not be read: going on with the ones before ({why})"));
                    }
                    now.unread = Some(why);
                }
                None => {
                    if now.unread.is_none() {
                        log("the settings changed, and the file could not be read: going on with the ones before");
                    }
                    now.unread = Some("the settings file could not be read".into());
                }
            }
            // tried for this version of the file: it is tried again when it changes again
            now.stamp = stamp;
        }
        now.cfg.clone()
    }

    /// Why the settings could not be read again, the last time the file changed. Nothing while they could.
    pub(crate) fn unread(&self) -> Option<String> {
        self.current();
        self.now.read().unwrap().unread.clone()
    }

    /// Change the settings: the change is made to the settings now, written, and used from then on only once it
    /// is written. When it cannot be written, nothing changes.
    pub(crate) fn change<R>(&self, f: impl FnOnce(&mut Config) -> R) -> Result<R, String> {
        let mut cfg = (*self.current()).clone();
        if let Some(why) = self.unread() {
            return Err(format!("This computer's settings cannot be read right now: {why}"));
        }
        let made = f(&mut cfg);
        let file = self.now.read().unwrap().file.clone();
        if file.is_some() {
            config::save(&cfg)?;
        }
        let mut now = self.now.write().unwrap();
        now.cfg = Arc::new(cfg);
        now.stamp = file.as_deref().and_then(config::stamp_of);
        Ok(made)
    }

    /// What reads for the tower now.
    pub(crate) fn reader(&self) -> Summarizer {
        self.current().summarizer.clone()
    }

    /// Why what reads could not, the last time it was asked.
    pub(crate) fn failed(&self) -> Option<Trouble> {
        *self.failed.lock().unwrap()
    }

    /// What became of the last time what reads was asked: a failure is kept until it next answers.
    pub(crate) fn asked(&self, failed: Option<Trouble>) {
        *self.failed.lock().unwrap() = failed;
    }

    /// Settings as given, never read from the file again or written to it: for a command that asks one
    /// question, and for tests.
    pub(crate) fn held(cfg: Config) -> Settings {
        Settings { now: RwLock::new(Now { cfg: Arc::new(cfg), stamp: None, unread: None, file: None }), failed: Mutex::new(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings read from a file of a test's own, never written.
    fn reading(cfg: Config, file: &std::path::Path) -> Settings {
        Settings { now: RwLock::new(Now { cfg: Arc::new(cfg), stamp: config::stamp_of(file), unread: None, file: Some(file.to_path_buf()) }), failed: Mutex::new(None) }
    }

    fn write(file: &std::path::Path, text: &str, age: u64) {
        std::fs::write(file, text).unwrap();
        // a stamp of its own for each version, however fast they are written
        let at = SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::File::options().write(true).open(file).unwrap().set_modified(at).unwrap();
    }

    #[test]
    fn a_pairing_added_while_the_tower_runs_is_in_the_settings_the_relay_link_and_push_read() {
        let dir = std::env::temp_dir().join(format!("airband-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");
        let start = r#"{"token":"own","radio_token":"radio","pairings":[]}"#;
        write(&file, start, 60);
        let settings = reading(config::read_from(&file).unwrap(), &file);
        assert!(settings.current().pairings.is_empty());
        // `airband pair`, from another process
        write(&file, r#"{"token":"own","radio_token":"radio","pairings":[{"name":"phone","secret":"00"}]}"#, 30);
        assert_eq!(settings.current().pairings.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["phone"]);
        // a file that cannot be read is said, and the settings before go on being used
        write(&file, "{ not settings", 10);
        assert_eq!(settings.current().pairings.len(), 1);
        assert!(settings.unread().is_some());
        // and one that can be read again is used again
        write(&file, start, 0);
        assert!(settings.current().pairings.is_empty() && settings.unread().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_of_what_reads_is_kept_until_it_answers() {
        let settings = Settings::held(serde_json::from_str(r#"{"token":"own","radio_token":"radio"}"#).unwrap());
        settings.asked(Some(Trouble::Unreachable));
        assert_eq!(settings.failed(), Some(Trouble::Unreachable));
        settings.asked(None);
        assert_eq!(settings.failed(), None);
    }
}
