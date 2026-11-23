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

impl crate::kept::Rejoin for Saved {
    /// What he said while the file could not be read is still to go, after what was waiting before; the chat
    /// has every line either holds, each in its newer form; the place in the feed is the one this launch
    /// reached once it has had a feed; this radio's name is the one kept before, and the computer's the one
    /// it said last.
    fn rejoin(&mut self, since: Saved) {
        for o in since.outbox {
            if !self.outbox.iter().any(|kept| kept.id == o.id) {
                self.outbox.push(o);
            }
        }
        for m in since.history {
            match self.history.iter_mut().find(|h| h.id == m.id) {
                Some(have) => *have = m,
                None => self.history.push(m),
            }
        }
        self.history.sort_by_key(|m| m.seq);
        let extra = self.history.len().saturating_sub(HISTORY_KEEP);
        self.history.drain(..extra);
        if since.after > 0 || since.rev > 0 {
            (self.after, self.rev) = (since.after, since.rev);
        }
        if self.radio.is_empty() {
            self.radio = since.radio;
        }
        if !since.computer.is_empty() {
            self.computer = since.computer;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kept::Rejoin;

    #[test]
    fn what_he_said_while_the_file_could_not_be_read_still_goes_once_it_reads() {
        // (audit 8, M4)
        let said = |id: &str| Outgoing::new(id.into(), id.into(), None, None, None, Vec::new(), Vec::new());
        let line = |id: &str, seq: u64, text: &str| -> airband_core::radio::Message { serde_json::from_value(serde_json::json!({ "seq": seq, "id": id, "at": seq, "from": "a", "group": "g", "text": text })).unwrap() };
        let mut found = Saved { outbox: vec![said("before")], history: vec![line("m1", 1, "old"), line("m2", 2, "kept")], after: 2, rev: 4, radio: "radio-kept".into(), computer: "studio".into() };
        let since = Saved { outbox: vec![said("meanwhile"), said("before")], history: vec![line("m1", 1, "new"), line("m3", 3, "since")], after: 3, rev: 6, radio: "radio-new".into(), computer: String::new() };
        found.rejoin(since);
        assert_eq!(found.outbox.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["before", "meanwhile"]);
        assert_eq!(found.history.iter().map(|m| (m.id.as_str(), m.text.as_str())).collect::<Vec<_>>(), [("m1", "new"), ("m2", "kept"), ("m3", "since")]);
        assert_eq!((found.after, found.rev, found.radio.as_str(), found.computer.as_str()), (3, 6, "radio-kept", "studio"));
        // with no feed had since, the place the file kept stands
        let mut found = Saved { after: 9, rev: 9, ..Saved::default() };
        found.rejoin(Saved::default());
        assert_eq!((found.after, found.rev), (9, 9));
    }
}
