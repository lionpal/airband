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
    /// What the computer last said it is called: known at launch, before it is reached again.
    #[serde(default)]
    pub(crate) computer: String,
}

pub(crate) struct Store {
    pub(crate) saved: Mutex<Saved>,
    /// Who is told when what he said could not be kept, once until it can be again.
    host: Option<Arc<dyn RadioHost>>,
    keeper: airband_io::kept::Keeper,
}

impl Store {
    pub(crate) fn open(data_dir: &str, host: Option<Arc<dyn RadioHost>>) -> Store {
        let (saved, keeper, damaged) = crate::kept::open(crate::kept::file(data_dir, "radio.json"), "This computer's unsent messages and chat");
        if let (Some(told), Some(host)) = (damaged, &host) {
            host.told(told);
        }
        Store { saved: Mutex::new(saved), host, keeper }
    }
    /// Where fetched pictures are kept: beside the saved chat, or in the system's temporary folder when the
    /// host keeps nothing.
    pub(crate) fn pictures(&self) -> std::path::PathBuf {
        self.keeper.path().and_then(|f| f.parent().map(|d| d.join("pictures"))).unwrap_or_else(|| std::env::temp_dir().join("airband-pictures"))
    }
    /// Drop pictures fetched long ago; one that is wanted again is fetched again.
    pub(crate) fn prune_pictures(&self) {
        airband_io::files::prune_older(&self.pictures(), PICTURES_KEEP);
    }
    pub(crate) fn change<T>(&self, f: impl FnOnce(&mut Saved) -> T) -> T {
        let mut saved = self.saved.lock().unwrap();
        crate::kept::again(&self.keeper, &mut *saved);
        let out = f(&mut saved);
        if let Some(told) = crate::kept::told(self.keeper.save(&*saved), "What you send and the chat are lost if the app closes.")
            && let Some(host) = &self.host
        {
            host.told(told);
        }
        out
    }
}
