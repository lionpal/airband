//! The files the phone keeps, read and written one way. A file that is there and cannot be read is told apart
//! from one that is not there yet: core's `kept::load` sets it aside, never written over as if it had been
//! empty, and this says so in the device's log. A save that fails says why, for him to be told.
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;

/// What `path` holds, or the default for a file that is not there. One that cannot be read has been moved
/// aside by core's `kept::load`, and the default is used in its place.
pub(crate) fn load_or_set_aside<T: DeserializeOwned + Default>(path: &Path) -> T {
    airband_core::kept::load(path).unwrap_or_else(|why| {
        eprintln!("airband: {why}");
        None
    })
    .unwrap_or_default()
}

/// Keep `value` at `path`, whole or not at all. Err says, in words for him, that it was not kept.
pub(crate) fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    airband_core::kept::save(path, value).map_err(|e| format!("This phone could not keep it: {e}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_file_is_set_aside_and_not_written_over() {
        let dir = std::env::temp_dir().join(format!("airband-kept-{}", airband_core::encode::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("radio.json");
        std::fs::write(&file, b"{ half written").unwrap();
        let got: Vec<String> = load_or_set_aside(&file);
        assert!(got.is_empty() && !file.exists());
        let aside = std::fs::read_dir(&dir).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("radio.json.unreadable")).unwrap();
        assert_eq!(std::fs::read(aside.path()).unwrap(), b"{ half written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
