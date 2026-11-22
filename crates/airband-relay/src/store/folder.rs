//! The relay's records kept in a folder on its own disk: `channels.json`, `closed.json` (the channels their
//! towers closed), `waitlist.txt`, and under `held/` one folder per channel with its kept messages and their list. For a relay run on a machine with a disk.
use super::*;
use std::path::Path;

/// One change to a folder's channel list or waiting list at a time: two towers opening at once must not each read
/// the list and write it back without the other's channel.
static LISTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// One change to a channel's list of kept messages at a time: two requests for one channel must not each read it
/// and write it.
static HELD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// channels.json: each channel's [`Saved`]. A file that is there but cannot be read is an error, never "no
/// channels": taken for empty, the next change would write that back and lose every pairing.
fn read_channels(dir: &Path) -> Result<HashMap<String, Saved>, String> {
    airband_io::kept::read(&dir.join("channels.json")).map(Option::unwrap_or_default)
}
fn write_channels(dir: &Path, all: &HashMap<String, Saved>) -> Result<(), String> {
    let file = dir.join("channels.json");
    airband_io::kept::save(&file, all).map_err(|e| format!("cannot save {}: {e}", file.display()))
}
/// closed.json: every channel its tower closed. Read the same way: unreadable is an error, never "none closed".
fn read_closed(dir: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    airband_io::kept::read(&dir.join("closed.json")).map(Option::unwrap_or_default)
}

/// What a channel's folder keeps, in the order it came: each by the number it is kept under and the id the
/// radio gave it. Written whole (`kept`), so what is kept is read from here and never worked out from the names
/// of the files, and a half-written file in the folder counts for nothing.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct HeldList {
    pub(super) items: Vec<HeldItem>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct HeldItem {
    pub(super) seq: u64,
    pub(super) id: String,
    /// When it was kept, in seconds since 1970: what a tower never came back for is dropped after a while.
    pub(super) at: u64,
}

pub(super) fn held_dir(dir: &Path, channel: &str) -> PathBuf {
    dir.join("held").join(channel)
}
/// A channel's list. One that cannot be read is moved aside and said, and the channel starts again with
/// nothing kept, so it is never refused for ever; only one that cannot even be moved aside is an error, left
/// as it is.
pub(super) fn held_list(held: &Path) -> Result<HeldList, String> {
    let (list, _, loaded) = airband_io::kept::Keeper::open::<HeldList>(Some(held.join("list.json")));
    let channel = held.file_name().map(|c| c.to_string_lossy().to_string()).unwrap_or_default();
    match loaded {
        airband_io::kept::Loaded::LeftInPlace { why } => Err(why),
        airband_io::kept::Loaded::SetAside { why } => {
            eprintln!("airband-relay: the list of kept messages on a channel could not be read; it starts again: {}", crate::state::masked(&why, &channel));
            // the bodies it named can never be handed over now: they go, and the next ones start at 1 again
            for body in airband_io::files::entries(held).map_err(|e| e.to_string())? {
                if body.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.parse::<u64>().is_ok()) {
                    airband_io::files::remove(&body).ok();
                }
            }
            Ok(list)
        }
        airband_io::kept::Loaded::Fresh | airband_io::kept::Loaded::Read => Ok(list),
    }
}
impl HeldItem {
    /// Still within the time a kept message waits for its tower.
    fn fresh(&self, now: u64) -> bool {
        in_time(expires(self.at), now)
    }
}

/// Take out of a channel's list every message `out` picks, keep the list, then remove their bodies. Nothing,
/// and nothing written, when it picks none.
fn drop_from(held: &Path, list: &mut HeldList, out: impl Fn(&HeldItem) -> bool) -> Result<(), String> {
    let gone: Vec<u64> = list.items.iter().filter(|h| out(h)).map(|h| h.seq).collect();
    if gone.is_empty() {
        return Ok(());
    }
    list.items.retain(|h| !gone.contains(&h.seq));
    held_keep(held, list)?;
    gone.iter().for_each(|seq| remove_body(held, *seq));
    Ok(())
}

/// Remove a kept message's body once the list no longer names it. One that cannot be removed is said: it
/// counts for nothing, as the list is what is read, but it takes room on the disk.
fn remove_body(held: &Path, seq: u64) {
    if let Err(e) = airband_io::files::remove(&held.join(seq.to_string())) {
        eprintln!("airband-relay: a kept message's body could not be removed: {e}");
    }
}
pub(super) fn held_keep(held: &Path, list: &HeldList) -> Result<(), String> {
    airband_io::kept::save(&held.join("list.json"), list).map_err(|e| e.to_string())
}

fn load(dir: &Path) -> Result<HashMap<String, Saved>, String> {
    // one closed whose record could not then be forgotten is closed all the same
    let closed = read_closed(dir)?;
    Ok(read_channels(dir)?.into_iter().filter(|(c, _)| !closed.contains(c)).collect())
}
fn join_waitlist(dir: &Path, email: &str) -> Result<(), String> {
    let _one = LISTS.lock().unwrap();
    let file = dir.join("waitlist.txt");
    // not there yet is an empty list; there and unreadable is an error, never written over as empty
    let have = airband_io::files::read_text(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?.unwrap_or_default();
    if have.lines().any(|l| l == email) {
        return Ok(());
    }
    airband_io::files::write_private(&file, format!("{have}{email}\n")).map_err(|e| format!("cannot save {}: {e}", file.display()))
}
fn remove(dir: &Path, channel: &str) -> Result<(), String> {
    let _one = LISTS.lock().unwrap();
    // closed on record first: a tower still pulling is refused from here on, even if the rest fails
    let mut closed = read_closed(dir)?;
    if closed.insert(channel.to_string()) {
        let file = dir.join("closed.json");
        airband_io::kept::save(&file, &closed).map_err(|e| format!("cannot save {}: {e}", file.display()))?;
    }
    let mut all = read_channels(dir)?;
    if all.remove(channel).is_some() {
        write_channels(dir, &all)?;
    }
    // what was kept for it goes too, under the lock every keeping takes, which then finds it closed
    let _held = HELD.lock().unwrap();
    airband_io::files::remove_folder(&held_dir(dir, channel)).map_err(|e| format!("the channel is closed, but what was kept for it could not be removed: {e}"))
}
fn add(dir: &Path, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<Added, String> {
    let _one = LISTS.lock().unwrap();
    if read_closed(dir)?.contains(channel) {
        return Ok(Added::Closed);
    }
    let mut all = read_channels(dir)?;
    if let Some(on_record) = all.get(channel) {
        return Ok(Added::Open(on_record.clone()));
    }
    let saved = Saved { pull_hash: pull_hash.into(), radio_hash: radio_hash.into() };
    all.insert(channel.into(), saved.clone());
    write_channels(dir, &all).map(|()| Added::Open(saved))
}
fn hold(dir: &Path, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<Held, String> {
    let _one = HELD.lock().unwrap();
    if read_closed(dir)?.contains(channel) {
        return Ok(Held::Closed);
    }
    let held = held_dir(dir, channel);
    airband_io::files::make_folder(&held).map_err(|e| e.to_string())?;
    let mut list = held_list(&held)?;
    let now = airband_core::clock::now_secs();
    if list.items.iter().any(|h| h.id == id && h.fresh(now)) {
        return Ok(Held::Kept);
    }
    // the same id, kept before and past its time: it goes, and this one is kept as new, as every store does
    drop_from(&held, &mut list, |h| h.id == id)?;
    if list.items.iter().filter(|h| h.fresh(now)).count() >= most {
        return Ok(Held::Full);
    }
    // in the order they came: a number that only goes up
    let seq = list.items.iter().map(|h| h.seq).max().map_or(1, |n| n + 1);
    airband_io::files::write_atomic(&held.join(seq.to_string()), sealed).map_err(|e| e.to_string())?;
    list.items.push(HeldItem { seq, id: id.to_string(), at: now });
    held_keep(&held, &list).map(|_| Held::Kept)
}
fn held_next(dir: &Path, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    let _one = HELD.lock().unwrap();
    let held = held_dir(dir, channel);
    let mut list = held_list(&held)?;
    list.items.sort_by_key(|h| h.seq);
    let now = airband_core::clock::now_secs();
    // what is past its time is passed over, as every store does; an entry whose message is not there
    // (its file was lost) is dropped from the list and said, so the channel does not stop at it for ever
    let mut lost = Vec::new();
    let mut found = None;
    for item in list.items.iter().filter(|h| h.fresh(now)) {
        match airband_io::files::read_bytes(&held.join(item.seq.to_string())) {
            Ok(Some(sealed)) => {
                found = Some((item.id.clone(), sealed));
                break;
            }
            Ok(None) => lost.push(item.seq),
            Err(e) => return Err(e.to_string()),
        }
    }
    if !lost.is_empty() {
        eprintln!("airband-relay: {} kept message(s) on a channel had no body and were dropped from its list", lost.len());
        // the one found is still handed over: a list that cannot be written now is tidied next time
        if let Err(e) = drop_from(&held, &mut list, |h| lost.contains(&h.seq)) {
            eprintln!("airband-relay: the list of kept messages on a channel could not be tidied: {}", crate::state::masked(&e, channel));
        }
    }
    Ok(found)
}
fn held_done(dir: &Path, channel: &str, id: &str) -> Result<(), String> {
    let _one = HELD.lock().unwrap();
    let held = held_dir(dir, channel);
    let mut list = held_list(&held)?;
    drop_from(&held, &mut list, |h| h.id == id)
}
fn held_purge(dir: &Path) -> Result<(), String> {
    let _one = HELD.lock().unwrap();
    // a held folder that is not there has nothing in it; one that cannot be read is said. One channel that
    // cannot be cleared does not stop the rest from being cleared.
    let channels = airband_io::files::entries(&dir.join("held")).map_err(|e| format!("cannot read the kept messages' folder: {e}"))?;
    let now = airband_core::clock::now_secs();
    let mut failed = 0;
    for held in channels {
        if held_list(&held).and_then(|mut list| drop_from(&held, &mut list, |h| !h.fresh(now))).is_err() {
            failed += 1;
        }
    }
    if failed == 0 { Ok(()) } else { Err(format!("{failed} channel(s) could not be cleared of old kept messages")) }
}

/// The folder store: the jobs above, for a relay with a disk.
pub(crate) struct Folder {
    pub(super) dir: PathBuf,
}

#[async_trait::async_trait]
impl ChannelStore for Folder {
    fn describe(&self) -> String {
        format!("the folder {}", self.dir.display())
    }
    async fn load(&self) -> Result<HashMap<String, Saved>, String> {
        load(&self.dir)
    }
    async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        join_waitlist(&self.dir, email)
    }
    async fn remove(&self, channel: &str) -> Result<(), String> {
        remove(&self.dir, channel)
    }
    async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<Added, String> {
        add(&self.dir, channel, pull_hash, radio_hash)
    }
    async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<Held, String> {
        hold(&self.dir, channel, id, sealed, most)
    }
    async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        held_next(&self.dir, channel)
    }
    async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        held_done(&self.dir, channel, id)
    }
    async fn held_purge(&self) -> Result<(), String> {
        held_purge(&self.dir)
    }
}
