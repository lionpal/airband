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

/// A file kept again and again (the saved chat, the team colours, what he has read): he is told when it
/// stops being kept, once, and again only after it has been kept since.
#[derive(Default)]
pub(crate) struct Keeping(std::sync::atomic::AtomicBool);
impl Keeping {
    /// Keep `value` at `path`. The words to tell him when this is the first save to fail: why, then `lost`,
    /// what that costs him.
    pub(crate) fn save<T: Serialize + ?Sized>(&self, path: &Path, value: &T, lost: &str) -> Option<String> {
        use std::sync::atomic::Ordering;
        match save(path, value) {
            Ok(()) => {
                self.0.store(false, Ordering::SeqCst);
                None
            }
            Err(why) => (!self.0.swap(true, Ordering::SeqCst)).then(|| format!("{why} {lost}")),
        }
    }
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
