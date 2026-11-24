//! Fetching a model onto this device. Each file is fetched in pieces ([`PIECE`]) asked for by byte range, several
//! at once, and each piece is kept as it lands, so a connection that drops costs at most the pieces under way and
//! a fetch tried again goes on from there. A file takes its name only once it is whole and its hash, taken over
//! the whole of it, is the one its row gives.
//!
//! What moves the bytes is the host's, when it has a transfer of its own that goes on while the app is in the
//! background or closed ([`Transfers`]); else this core's own connections. Either way the core decides what is
//! fetched and where it goes, and checks every piece and every file; the host only moves bytes.
use super::*;
use airband_io::files;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc;

/// How far the fetch now running has got: bytes so far (never fewer than were said before, in one fetch), and
/// bytes in all, and whether every file is whole and checked.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicBool = AtomicBool::new(false);
/// When the fetch now running last had a byte more, or began.
static MOVED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());
/// Tests that fetch, one at a time: how far a fetch has got is one for the app, so a test that reads it must be
/// the only one fetching from before it begins until it has read it.
#[cfg(test)]
pub(crate) fn one_fetching_test() -> std::sync::MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(|e| e.into_inner())
}
/// Where what the host says about its transfers goes, while a fetch waits on them.
static LISTENER: Mutex<Option<mpsc::Sender<Event>>> = Mutex::new(None);

/// How much of a file is asked for at once. A dropped connection costs at most this much of each piece under
/// way; small enough that this is little, large enough that asking costs nothing next to the bytes.
pub(crate) const PIECE: u64 = 16 << 20;
/// How many pieces this core fetches at once over its own connections. Measured against airband.dev from a
/// computer on a fast line, one connection carried 54 to 81 MB/s and two to eight carried 83 to 99 MB/s,
/// level from two on; on a phone's slower, lossier links several connections recover from a loss each on its
/// own. The host's transfer keeps its own count.
const AT_ONCE: usize = 4;

/// The host's own way to move a file's bytes, which goes on while the app is in the background or closed.
#[uniffi::export(with_foreign)]
pub trait Transfers: Send + Sync {
    /// Move bytes `from` up to `until` (not included) of `url` into a new file at `to`, asking with an HTTP range
    /// (`Range: bytes=from-(until-1)`). The file is put at `to` only once the transfer has ended, whole as it
    /// came. As it goes, tell [`Desk::transfer_moved`] how many bytes have come; at the end, tell
    /// [`Desk::transfer_ended`] with the status the server answered. `id` names it in both. Starting an `id`
    /// that has a transfer already, under way or ended and not yet told, leaves that one and starts nothing more.
    fn start(&self, id: String, url: String, from: u64, until: u64, to: String);
    /// Stop the transfer `id`, if there is one, and put nothing at its `to`. Nothing more is told of it.
    fn stop(&self, id: String);
}

/// What is heard about a piece under way.
enum Event {
    /// This many bytes of it have come.
    Moved { id: String, bytes: u64 },
    /// The host's transfer ended: with the status the server answered, or none when it got no answer.
    Ended { id: String, status: Option<u16> },
    /// This core's connection fetched it, or why not.
    Own { id: String, went: Result<(), FetchFailed> },
    /// He stopped the fetch.
    Stop,
}

fn heard(event: Event) {
    if let Some(to) = LISTENER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        to.send(event).ok();
    }
}

#[uniffi::export]
impl Desk {
    /// Bring a model's files with the host's own transfers from now on, so a fetch goes on while the app is in
    /// the background or closed. Without this, this core fetches them itself while the app runs.
    pub fn use_transfers(&self, transfers: Arc<dyn Transfers>) {
        *self.shared.chooser.transfers.lock().unwrap_or_else(|e| e.into_inner()) = Some(transfers);
    }
    /// How many bytes of a transfer under way have come.
    pub fn transfer_moved(&self, id: String, bytes: u64) {
        heard(Event::Moved { id, bytes });
    }
    /// A transfer ended, with the status the server answered, or none when it got no answer. Its file is in
    /// place when there is one.
    pub fn transfer_ended(&self, id: String, status: Option<u16>) {
        heard(Event::Ended { id, status });
    }
    /// He stopped the model being downloaded: the fetch ends now, keeping what has come, and another can start.
    /// What is here is gone on from next time.
    pub fn stop_model_fetch(&self) {
        heard(Event::Stop);
    }
}

/// How long with no byte more before a download is said to be waiting for the network.
pub(crate) const STALLED_AFTER: Duration = Duration::from_secs(10);

/// How far a download has got, as a card shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Progress {
    /// Bytes here so far, counting those still coming in pieces under way.
    pub(crate) got: u64,
    /// Bytes in all.
    pub(crate) of: u64,
    /// Every file is here and checked.
    pub(crate) whole: bool,
    /// Nothing more has come for [`STALLED_AFTER`].
    pub(crate) stalled: bool,
}

impl Progress {
    /// How far along, from 0 to 1: 1 only once every file is whole and checked, never on the bytes alone.
    pub(crate) fn fraction(&self) -> f64 {
        if self.whole {
            return 1.0;
        }
        match self.of {
            0 => 0.0,
            of => self.got.min(of - 1) as f64 / of as f64,
        }
    }
    /// The line under the bar. The card already shows the model's name.
    pub(crate) fn said(&self) -> String {
        if self.stalled && !self.whole {
            return "Waiting for the network".into();
        }
        let of = megabytes(self.of);
        let got = if self.whole { of } else { (self.got.min(self.of) / 1_000_000).min(of.saturating_sub(1) as u64) as u32 };
        format!("Downloading {got} of {of}\u{a0}MB")
    }
}

/// Bytes as the whole megabytes a card shows a size in.
pub(crate) fn megabytes(bytes: u64) -> u32 {
    (bytes as f64 / 1e6).round() as u32
}

/// How far the fetch now running has got. Asked a few times a second.
pub(crate) fn local_model_progress() -> Progress {
    let since = MOVED.lock().unwrap_or_else(|e| e.into_inner()).map(|t| t.elapsed());
    Progress { got: GOT.load(Ordering::SeqCst), of: OF.load(Ordering::SeqCst), whole: DONE.load(Ordering::SeqCst), stalled: since.is_some_and(|s| s >= STALLED_AFTER) }
}

/// The bytes here now: what is said goes up to it, never down.
fn moved_to(got: u64) {
    if GOT.fetch_max(got, Ordering::SeqCst) < got {
        *MOVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    }
    happened();
}

/// Something a test waits on has happened: the fetch has taken in what its transfers said, or ended. Tests wait
/// on this, never on a clock.
#[cfg(test)]
pub(crate) static HAPPENED: (Mutex<u64>, std::sync::Condvar) = (Mutex::new(0), std::sync::Condvar::new());
fn happened() {
    #[cfg(test)]
    {
        *HAPPENED.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        HAPPENED.1.notify_all();
    }
}

/// One range of one file, asked for on its own.
#[derive(Clone)]
struct Piece {
    /// Which of the model's files.
    file: usize,
    from: u64,
    until: u64,
    url: String,
    /// Where it is kept until it is joined to the file.
    to: PathBuf,
    id: String,
}

impl Piece {
    fn len(&self) -> u64 {
        self.until - self.from
    }
}

/// A file of the model as it is being put together: what of it is joined already, and the pieces still to be.
struct Building {
    path: PathBuf,
    size: u64,
    sha: String,
    /// Bytes from the start that are joined in `<name>.part`.
    joined: u64,
    whole: bool,
}

fn part_of(path: &Path) -> PathBuf {
    files::beside(path, ".part")
}
/// What a piece is called beside its file: by where it starts.
const PIECE_TAIL: &str = ".from-";
fn piece_at(path: &Path, from: u64) -> PathBuf {
    files::beside(path, &format!("{PIECE_TAIL}{from}"))
}
fn len_of(path: &Path) -> Option<u64> {
    files::metadata(path).ok().filter(|f| f.is_file()).map(|f| f.len())
}
/// The name of a file, as text.
fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The ranges still wanted of a file of `size` bytes of which `joined` are here: up to the next boundary of
/// [`PIECE`], then a piece at a time. The boundaries are the same every time, so a piece fetched by an earlier
/// try is found by its name.
fn ranges(joined: u64, size: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut at = joined;
    while at < size {
        let end = ((at / PIECE + 1) * PIECE).min(size);
        out.push((at, end));
        at = end;
    }
    out
}

/// Why a fetch did not end with the model here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchFailed {
    /// The connection stopped part way.
    Stopped,
    /// He stopped it.
    Cancelled,
    /// The download could not be reached.
    Unreached,
    /// This device would not take the bytes.
    NotWritten,
    /// This device ran out of room.
    NoRoom,
    /// The server sent the whole file where a part was asked for.
    NotResumed,
    /// What came down is not the model: its hash is not the row's.
    NotTheModel,
    /// The server answered with this, and not the file.
    Status(u16),
}

impl FetchFailed {
    /// In words for him.
    pub(crate) fn said(self) -> String {
        match self {
            FetchFailed::Stopped => "The download stopped part way. Try again.".into(),
            FetchFailed::Cancelled => "The download was stopped. What came is kept for next time.".into(),
            FetchFailed::Unreached => "Could not reach the download. Check the connection and try again.".into(),
            FetchFailed::NotWritten => "Could not write the model to this device.".into(),
            FetchFailed::NoRoom => "This device ran out of room for the model.".into(),
            FetchFailed::NotResumed => "The download could not go on from where it stopped. Try again.".into(),
            FetchFailed::NotTheModel => "What came down was not the model. Check the connection and try again.".into(),
            FetchFailed::Status(404 | 410) => "The download is not there.".into(),
            FetchFailed::Status(429) => "The download was being asked for too often. Try again in a while.".into(),
            FetchFailed::Status(status @ 500..=599) => format!("The place the download comes from ran into trouble ({status}). Try again later."),
            FetchFailed::Status(status) => format!("The download could not be fetched ({status})."),
        }
    }
}

impl From<FetchFailed> for RadioError {
    fn from(why: FetchFailed) -> RadioError {
        crate::choices::told(why.said())
    }
}

/// Fetch a model, with the host's transfers when there are some, else over this core's own connections.
/// Blocks until it is all here, or he stops it ([`Desk::stop_model_fetch`]): call it off the main thread. What
/// is here already, whole files and pieces, is kept, so one that was cut short goes on from there.
pub(crate) fn fetch_local_model(dir: String, m: &Model, host: Option<Arc<dyn Transfers>>) -> Result<(), FetchFailed> {
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = version_folder(&dir, m);
    OF.store(m.files.iter().map(|f| f.1).sum(), Ordering::SeqCst);
    GOT.store(0, Ordering::SeqCst);
    DONE.store(false, Ordering::SeqCst);
    *MOVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    let mut files: Vec<Building> = Vec::new();
    let mut wanted: Vec<Piece> = Vec::new();
    // pieces already here and whole, by file and where each starts
    let mut ready: BTreeMap<(usize, u64), Piece> = BTreeMap::new();
    for (i, (to, size, sha)) in m.files.iter().enumerate() {
        let path = folder.join(to);
        let whole = len_of(&path) == Some(*size);
        let mut joined = 0;
        if !whole {
            files::make_folder(path.parent().unwrap_or(&folder)).map_err(|_| FetchFailed::NotWritten)?;
            joined = len_of(&part_of(&path)).unwrap_or(0);
            if joined > *size {
                files::remove(&part_of(&path)).ok();
                joined = 0;
            }
            let plan = ranges(joined, *size);
            // a piece left by an earlier try that is not one of these is of no use
            let stem = format!("{}{PIECE_TAIL}", name_of(&path));
            for e in files::entries(path.parent().unwrap_or(&folder)).unwrap_or_default() {
                if name_of(&e).strip_prefix(&stem).is_some_and(|from| !plan.iter().any(|(a, _)| a.to_string() == from)) {
                    files::remove(&e).ok();
                }
            }
            for (from, until) in plan {
                let p = Piece { file: i, from, until, url: format!("{}/{to}", m.base), to: piece_at(&path, from), id: format!("{}/{}/{to}@{from}", m.kind, m.version), };
                match len_of(&p.to) {
                    Some(n) if n == p.len() => {
                        ready.insert((i, from), p);
                    }
                    // what this core fetched before goes on from where it stopped; the host's transfers put
                    // only whole pieces in place, so part of one is nothing to go on from
                    Some(n) if n < p.len() && host.is_none() => wanted.push(p),
                    Some(_) => {
                        files::remove(&p.to).ok();
                        wanted.push(p);
                    }
                    None => wanted.push(p),
                }
            }
        }
        files.push(Building { path, size: *size, sha: sha.clone(), joined, whole });
    }
    let total = |files: &[Building], ready: &BTreeMap<(usize, u64), Piece>, moving: &HashMap<String, u64>| -> u64 {
        files.iter().map(|f| if f.whole { f.size } else { f.joined }).sum::<u64>() + ready.values().map(Piece::len).sum::<u64>() + moving.values().sum::<u64>()
    };
    let (tx, rx) = mpsc::channel::<Event>();
    let stop = AtomicBool::new(false);
    // told from here on: a stop he asks for, and what the host says of its transfers
    *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
    let outcome = std::thread::scope(|scope| {
        match &host {
            Some(host) => {
                // every piece at once: the system goes on with all of them while the app is not running. One
                // whose transfer ended whole before this was listening is here already: taken, not asked again
                wanted.retain(|p| {
                    let here = len_of(&p.to) == Some(p.len());
                    if here {
                        ready.insert((p.file, p.from), p.clone());
                    } else {
                        host.start(p.id.clone(), p.url.clone(), p.from, p.until, p.to.display().to_string());
                    }
                    !here
                });
            }
            None => {
                let queue = Arc::new(Mutex::new(wanted.iter().cloned().collect::<VecDeque<Piece>>()));
                let client = crate::net::Client::download(Duration::from_secs(15), Duration::from_secs(30));
                for _ in 0..AT_ONCE.min(wanted.len()) {
                    let (queue, client, tx, stop) = (queue.clone(), client.clone(), tx.clone(), &stop);
                    let sizes: Vec<u64> = m.files.iter().map(|f| f.1).collect();
                    scope.spawn(move || {
                        while !stop.load(Ordering::SeqCst) {
                            let Some(p) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { break };
                            let went = fetch_own(&client, &p, sizes[p.file], &tx, stop);
                            tx.send(Event::Own { id: p.id.clone(), went }).ok();
                        }
                    });
                }
            }
        }
        drop(tx);
        let mut waiting: HashMap<String, Piece> = wanted.iter().map(|p| (p.id.clone(), p.clone())).collect();
        let mut moving: HashMap<String, u64> = HashMap::new();
        let went = loop {
            if let Err(why) = join(&mut files, &mut ready) {
                break Err(why);
            }
            moved_to(total(&files, &ready, &moving));
            if files.iter().all(|f| f.whole) {
                break Ok(());
            }
            let Ok(event) = rx.recv() else { break Err(FetchFailed::Stopped) };
            let (id, ended) = match event {
                Event::Stop => break Err(FetchFailed::Cancelled),
                Event::Moved { id, bytes } => {
                    if waiting.contains_key(&id) {
                        moving.insert(id, bytes);
                    }
                    continue;
                }
                Event::Ended { id, status } => {
                    let ended = waiting.get(&id).map(|p| host_said(status, p, files[p.file].size));
                    (id, ended)
                }
                Event::Own { id, went } => (id, Some(went)),
            };
            let Some(ended) = ended else { continue };
            let Some(p) = waiting.remove(&id) else { continue };
            moving.remove(&id);
            // whatever moved it, a piece is taken only when it is all there
            match ended.and_then(|()| if len_of(&p.to) == Some(p.len()) { Ok(()) } else { Err(FetchFailed::Stopped) }) {
                Ok(()) => {
                    ready.insert((p.file, p.from), p);
                }
                Err(why) => {
                    if host.is_some() {
                        files::remove(&p.to).ok();
                    }
                    break Err(why);
                }
            }
        };
        stop.store(true, Ordering::SeqCst);
        *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // the host's transfers still under way are stopped: nothing comes of them after this
        if let (Err(_), Some(host)) = (&went, &host) {
            waiting.keys().for_each(|id| host.stop(id.clone()));
        }
        went
    });
    outcome?;
    // This version is whole: say so, and only now let go of the one that was here before it, which was in use
    // until this moment, and of any piece left over from fetching this one.
    files::write_atomic(&folder.join(WHOLE), "").map_err(|_| FetchFailed::NotWritten)?;
    DONE.store(true, Ordering::SeqCst);
    happened();
    for other in files::entries(&home(&dir, m)).unwrap_or_default().into_iter().filter(|p| *p != folder) {
        if other.is_dir() { files::remove_folder(&other) } else { files::remove(&other) }.ok();
    }
    sweep_pieces(&folder);
    Ok(())
}

/// Every piece left in a folder that is whole, and the folders under it: a transfer that ended after its piece
/// was no longer wanted leaves one.
fn sweep_pieces(folder: &Path) {
    for e in files::entries(folder).unwrap_or_default() {
        if e.is_dir() {
            sweep_pieces(&e);
        } else if name_of(&e).contains(PIECE_TAIL) {
            files::remove(&e).ok();
        }
    }
}

/// What the host's word on a transfer that ended means for its piece of a file of `size` bytes: the server sent
/// the range asked for, or the whole file when that was what was asked. Its length is checked after.
fn host_said(status: Option<u16>, p: &Piece, size: u64) -> Result<(), FetchFailed> {
    match status {
        None => Err(FetchFailed::Stopped),
        Some(206) => Ok(()),
        Some(200) if p.from == 0 && p.until == size => Ok(()),
        Some(200) => Err(FetchFailed::NotResumed),
        Some(other) => Err(FetchFailed::Status(other)),
    }
}

/// Join to each file the pieces that come next in it, in order; a file whose bytes are all joined is checked
/// against its hash, over the whole of it, and only then takes its name.
fn join(files: &mut [Building], ready: &mut BTreeMap<(usize, u64), Piece>) -> Result<(), FetchFailed> {
    for (i, f) in files.iter_mut().enumerate().filter(|(_, f)| !f.whole) {
        let part = part_of(&f.path);
        while let Some(p) = ready.remove(&(i, f.joined)) {
            let copied = append(&p.to, &part)?;
            if copied != p.len() {
                return Err(FetchFailed::NotWritten);
            }
            f.joined += copied;
            files::remove(&p.to).ok();
        }
        if f.joined == f.size {
            // a sign-in page, a connection that closed early, a file changed at the source: none of them is
            // the model
            if hash_of(&part).as_deref() != Some(f.sha.as_str()) {
                files::remove(&part).ok();
                f.joined = 0;
                return Err(FetchFailed::NotTheModel);
            }
            files::move_to(&part, &f.path).map_err(|_| FetchFailed::NotWritten)?;
            f.whole = true;
        }
    }
    Ok(())
}

/// Add a piece to the end of the file it is part of. Returns how many bytes went.
#[expect(clippy::disallowed_methods, reason = "adds a piece of a model to the end of what has come of it: airband_io writes files whole")]
fn append(piece: &Path, to: &Path) -> Result<u64, FetchFailed> {
    let mut from = std::fs::File::open(piece).map_err(|_| FetchFailed::NotWritten)?;
    let mut into = std::fs::OpenOptions::new().create(true).append(true).open(to).map_err(|_| FetchFailed::NotWritten)?;
    let copied = std::io::copy(&mut from, &mut into).map_err(|_| FetchFailed::NoRoom)?;
    into.sync_all().ok();
    Ok(copied)
}

/// The SHA-256 of a file, in hex, read a part at a time.
fn hash_of(path: &Path) -> Option<String> {
    const AT_ONCE: u64 = 1 << 20;
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    let mut at = 0;
    loop {
        let part = files::read_part(path, at, AT_ONCE).ok()?;
        if part.is_empty() {
            break;
        }
        sha2::Digest::update(&mut hash, &part);
        at += part.len() as u64;
    }
    Some(airband_core::encode::hex(&sha2::Digest::finalize(hash)))
}

/// Fetch one piece over this core's own connection, going on from what of it is here already. Ok when the
/// server sent what was asked for to the end.
#[expect(clippy::disallowed_methods, reason = "writes a piece of a model as it downloads, a little at a time: airband_io writes files whole")]
fn fetch_own(client: &crate::net::Client, p: &Piece, size: u64, tx: &mpsc::Sender<Event>, stop: &AtomicBool) -> Result<(), FetchFailed> {
    let have = len_of(&p.to).unwrap_or(0);
    if have >= p.len() {
        return Ok(());
    }
    let start = p.from + have;
    let mut res = client.send(crate::net::Ask::get(&p.url).header("range", format!("bytes={start}-{}", p.until - 1))).map_err(|_| FetchFailed::Unreached)?;
    match res.status().as_u16() {
        206 => {}
        // a server that sends the whole file for a range is taken at its word only when the whole file was asked for
        200 if start == 0 && p.until == size => {}
        200 => return Err(FetchFailed::NotResumed),
        other => return Err(FetchFailed::Status(other)),
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&p.to).map_err(|_| FetchFailed::NotWritten)?;
    let mut body = res.body_mut().as_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = have;
    while got < p.len() {
        if stop.load(Ordering::SeqCst) {
            return Err(FetchFailed::Stopped);
        }
        let n = body.read(&mut buf).map_err(|_| FetchFailed::Stopped)?;
        if n == 0 {
            break;
        }
        // never more than was asked for, whatever came
        let n = n.min((p.len() - got) as usize);
        file.write_all(&buf[..n]).map_err(|_| FetchFailed::NoRoom)?;
        got += n as u64;
        tx.send(Event::Moved { id: p.id.clone(), bytes: got }).ok();
    }
    if got == p.len() { Ok(()) } else { Err(FetchFailed::Stopped) }
}

#[cfg(test)]
mod tests;
