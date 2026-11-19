//! What the radio keeps on the device across restarts: his unsent messages, the recent chat, and its place in
//! the feed, in one small JSON file in a folder the host app provides; and beside it the pictures it has fetched.
use super::*;

/// A typed message that has not got through after this long is dropped and he is told. An instruction that
/// arrives hours late, after things have moved on, is worse than one that never arrives.
pub(crate) const OUTBOX_EXPIRY_MS: u64 = 30 * 60 * 1000;
/// How many chat messages are kept on the device, to show at once when the app opens with no connection.
pub(crate) const HISTORY_KEEP: usize = 300;

/// A fetched picture is kept on the device this long.
const PICTURES_KEEP: Duration = Duration::from_secs(14 * 24 * 3600);

/// What survives the app being closed: unsent messages and the recent chat.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Saved {
    pub(crate) outbox: Vec<Outgoing>,
    pub(crate) history: Vec<airband_core::radio::Message>,
    pub(crate) after: u64,
    /// The tower's change count as of the last feed: what to ask for changes since.
    #[serde(default)]
    pub(crate) rev: u64,
    /// This radio's name for itself, made once.
    #[serde(default)]
    pub(crate) radio: String,
}

pub(crate) struct Store {
    file: Option<std::path::PathBuf>,
    pub(crate) saved: Mutex<Saved>,
}

impl Store {
    pub(crate) fn open(data_dir: &str) -> Store {
        let file = (!data_dir.is_empty()).then(|| std::path::Path::new(data_dir).join("radio.json"));
        let saved = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Store { file, saved: Mutex::new(saved) }
    }
    /// Where fetched pictures are kept: beside the saved chat, or in the system's temporary folder when the
    /// host keeps nothing.
    pub(crate) fn pictures(&self) -> std::path::PathBuf {
        self.file.as_ref().and_then(|f| f.parent().map(|d| d.join("pictures"))).unwrap_or_else(|| std::env::temp_dir().join("airband-pictures"))
    }
    /// Drop pictures fetched long ago; one that is wanted again is fetched again.
    pub(crate) fn prune_pictures(&self) {
        let Ok(dir) = std::fs::read_dir(self.pictures()) else { return };
        for entry in dir.flatten() {
            if entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > PICTURES_KEEP) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    pub(crate) fn change<T>(&self, f: impl FnOnce(&mut Saved) -> T) -> T {
        let mut saved = self.saved.lock().unwrap();
        let out = f(&mut saved);
        if let Some(file) = &self.file {
            let tmp = file.with_extension("json.tmp");
            if std::fs::create_dir_all(file.parent().unwrap()).and_then(|_| std::fs::write(&tmp, serde_json::to_vec(&*saved).unwrap())).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
        out
    }
}
