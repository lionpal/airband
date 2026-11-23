//! The files the phone keeps, through `airband_io::kept`: what happens to one that is damaged is decided there,
//! once (set aside, or left as it is with every save refused). This says so in the device's log, and puts a save
//! that failed in words for him.
use airband_io::kept::{Keeper, Loaded, Saved};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// The file `name` in the host's folder `dir`: none when the host keeps nothing (a test).
pub(crate) fn file(dir: &str, name: &str) -> Option<PathBuf> {
    (!dir.is_empty()).then(|| Path::new(dir).join(name))
}

/// What `path` holds, or the default, and where to keep it. One that could not be read is told in the log, and
/// in words for him naming `what` it held: the caller tells him once.
pub(crate) fn open<T: DeserializeOwned + Default>(path: Option<PathBuf>, what: &str) -> (T, Keeper, Option<String>) {
    let (value, keeper, loaded) = Keeper::open(path);
    if let Some(trouble) = loaded.trouble() {
        eprintln!("airband: {trouble}");
    }
    let told = damaged(&loaded, what);
    (value, keeper, told)
}

/// In words for him, what came of a file that could not be read. Nothing when it read, or was not there yet.
fn damaged(loaded: &Loaded, what: &str) -> Option<String> {
    match loaded {
        Loaded::Fresh | Loaded::Read => None,
        Loaded::SetAside { .. } => Some(format!("{what} could not be read on this phone, so it starts again. The old copy is kept beside it.")),
        Loaded::LeftInPlace { .. } => Some(format!("{what} could not be read on this phone and is left as it was. Nothing new is kept until it can be.")),
    }
}

/// A file that could not be read when it was opened, read again now that it may be: what it holds replaces
/// `value` (the default when it has gone), so the change about to be made is made to it and nothing kept is
/// written over. Called before each change of a bare [`Keeper`]'s value.
pub(crate) fn again<T: DeserializeOwned + Default>(keeper: &Keeper, value: &mut T) {
    if let Some(found) = keeper.read_again::<T>() {
        *value = found.unwrap_or_default();
    }
}

/// The words for him when a save failed: why, in words for him.
pub(crate) fn not_kept(why: &str) -> String {
    format!("This phone could not keep it: {why}.")
}

/// For a file kept again and again (the saved chat, the team colours, what he has read): the words to tell him
/// when this save is the first in a row to fail, with `lost`, what that costs him.
pub(crate) fn told(saved: Saved, lost: &str) -> Option<String> {
    saved.news().map(|why| format!("{} {lost}", not_kept(why)))
}

/// For a save he asked for: Err in words for him whenever it failed.
pub(crate) fn said(saved: Saved) -> Result<(), String> {
    saved.into_result().map_err(|why| not_kept(&why))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_that_cannot_be_moved_aside_is_never_saved_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-kept-{}", airband_core::encode::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("radio.json");
        std::fs::write(&file, b"{ half written").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let (got, keeper, told): (Vec<String>, _, _) = open(Some(file.clone()), "Test");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let saved = said(keeper.save(&vec!["new".to_string()]));
        let left = std::fs::read(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_empty());
        assert!(told.is_some_and(|t| t.contains("left as it was")), "he is not told");
        assert!(saved.is_err(), "it was saved over");
        assert_eq!(left, b"{ half written");
    }

    #[cfg(unix)]
    #[test]
    fn the_phone_stores_take_a_file_that_could_not_be_read_once_it_can_be() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-kept-{}", airband_core::encode::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let chat = dir.join("radio.json");
        let colours = dir.join("colours.json");
        std::fs::write(&chat, br#"{"outbox":[],"history":[],"after":7,"radio":"kept"}"#).unwrap();
        std::fs::write(&colours, br#"{"shop":3}"#).unwrap();
        // unreadable as the app opens: nothing is taken for empty
        for f in [&chat, &colours] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let store = crate::store::Store::open(dir.to_str().unwrap(), None);
        let mut paint = crate::desk::colours::Colours::open(dir.to_str().unwrap(), 6);
        assert_eq!(store.saved.lock().unwrap().radio, "");
        for f in [&chat, &colours] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        // readable again: the next change is made to what it holds, and it is saved
        store.change(|s| s.rev = 9);
        let _ = paint.see(["garden".to_string()]);
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&chat).unwrap()).unwrap();
        let painted = (paint.of("shop"), paint.of("garden").is_some());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((saved["radio"].as_str(), saved["after"].as_u64(), saved["rev"].as_u64()), (Some("kept"), Some(7), Some(9)));
        assert_eq!(painted, (Some(3), true));
    }
}
