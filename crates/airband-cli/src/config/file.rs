//! The settings file, `~/.airband/config.json`: where it is, read, held while it is changed, and written. The
//! one way to it; the settings themselves are in `config.rs`.
use super::*;

pub(crate) use airband_io::files::write_private;

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
pub(super) fn kept(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
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
pub(super) fn parse(text: &str, path: &Path) -> Result<Config, String> {
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

/// The file's text for these settings, and the keys to keep once it is written. The keys are never in it: they
/// are kept encrypted (`vault`) and the text has a mark in each of their places, from the first time the file
/// is written. Until the file is written the keys it has now are kept too, beside the new ones: a write that
/// fails leaves no mark in it pointing at a key that is gone.
pub(super) fn render(cfg: &Config, path: &Path) -> Result<(String, Option<serde_json::Map<String, serde_json::Value>>), String> {
    cfg.keys_read().map_err(|why| format!("the settings cannot be changed from here: {why}"))?;
    // a file that is there and cannot be read is never taken as one without keys: that would write over it
    let existing = airband_io::files::read_text(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?.unwrap_or_default();
    let mut doc = serde_json::to_value(cfg).unwrap();
    name_keys(&mut doc);
    let before = if crate::vault::sealed(&existing) { kept(path)? } else { Default::default() };
    let now = crate::vault::seal(&mut doc, &before);
    let mut both = before.clone();
    both.extend(now.clone());
    if both != before {
        crate::vault::put(&both)?;
    }
    Ok((format!("{}\n", serde_json::to_string_pretty(&doc).unwrap()), (now != both).then_some(now)))
}

/// Once the file is written: only the keys it marks are kept.
pub(super) fn keep_only(keys: Option<serde_json::Map<String, serde_json::Value>>) -> Result<(), String> {
    keys.map_or(Ok(()), |keys| crate::vault::put(&keys))
}

/// Held while the settings are read, changed and written, by every airband process: a change made from
/// another one in between is never written over.
pub(crate) fn hold() -> Result<crate::lock::Held, String> {
    hold_at(&file())
}
/// Hold the settings file at `path` while it is read, changed and written.
pub(crate) fn hold_at(path: &Path) -> Result<crate::lock::Held, String> {
    crate::lock::take(&path.with_extension("lock"))
}

/// Change the settings as the file has them now: held, read, changed by `f` and written in one go, so a change
/// another airband process made since this one read them is kept. A file that cannot be read, or settings
/// whose keys cannot be read, are never written: nothing changes and he is told why.
pub(crate) fn change<R>(f: impl FnOnce(&mut Config) -> Result<R, String>) -> Result<R, String> {
    change_at(&file(), f)
}
pub(super) fn change_at<R>(path: &Path, f: impl FnOnce(&mut Config) -> Result<R, String>) -> Result<R, String> {
    let held = hold_at(path)?;
    let mut cfg = match existing(path)? {
        Some((_, cfg)) => cfg,
        None => parse("{}", path)?,
    };
    cfg.keys_read().map_err(|why| format!("the settings cannot be changed from here: {why}"))?;
    let made = f(&mut cfg)?;
    save_at(&cfg, path, &held)?;
    Ok(made)
}

/// Write the settings to the file at `path`, which the caller holds. The keys never go in the file: they are
/// kept encrypted (`vault`), from the first save.
pub(crate) fn save_at(cfg: &Config, path: &Path, _: &crate::lock::Held) -> Result<(), String> {
    let (text, keys) = render(cfg, path)?;
    write_private(path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    keep_only(keys)
}

/// What is in the settings file: `None` when there is no file yet. A file that is there and cannot be read or
/// parsed is an error, never "no file": it holds the token, the pairings and the keys, and is never replaced.
pub(super) fn existing(path: &std::path::Path) -> Result<Option<(String, Config)>, String> {
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
    let (text, keys) = match render(&cfg, &path) {
        Ok(rendered) => rendered,
        Err(_) if cfg.keyless.is_some() => return cfg,
        Err(e) => {
            eprintln!("airband: {e}");
            std::process::exit(2);
        }
    };
    if existing.as_deref() != Some(text.as_str()) {
        write_private(&path, &text).expect("cannot write ~/.airband/config.json");
        // the keys it no longer marks are let go; one that cannot be is harmless, and kept until the next save
        keep_only(keys).ok();
    }
    cfg
}
