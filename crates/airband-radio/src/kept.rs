//! The files the phone keeps, through `airband_io::kept`: what happens to one that is damaged is decided there,
//! once (set aside, or left as it is with every save refused). This says so in the device's log, and puts a save
//! that failed in words for him.
use airband_io::kept::{Keeper, Saved};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// The file `name` in the host's folder `dir`: none when the host keeps nothing (a test).
pub(crate) fn file(dir: &str, name: &str) -> Option<PathBuf> {
    (!dir.is_empty()).then(|| Path::new(dir).join(name))
}

/// What `path` holds, or the default, and where to keep it. One that could not be read is told in the log.
pub(crate) fn open<T: DeserializeOwned + Default>(path: Option<PathBuf>) -> (T, Keeper) {
    let (value, keeper, loaded) = Keeper::open(path);
    if let Some(trouble) = loaded.trouble() {
        eprintln!("airband: {trouble}");
    }
    (value, keeper)
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
        let (got, keeper): (Vec<String>, _) = open(Some(file.clone()));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let saved = said(keeper.save(&vec!["new".to_string()]));
        let left = std::fs::read(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_empty());
        assert!(saved.is_err(), "it was saved over");
        assert_eq!(left, b"{ half written");
    }
}
