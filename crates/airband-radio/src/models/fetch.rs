//! Fetching a model onto this device. Each file is fetched in pieces ([`PIECE`]) asked for by byte range, several
//! at once, and each piece is kept as it lands, so a connection that drops costs at most the pieces under way and
//! a fetch tried again goes on from there. A file takes its name only once it is whole and its hash, taken over
//! the whole of it, is the one its row gives.
//!
//! What moves the bytes is the host's, when it has a transfer of its own that goes on while the app is in the
//! background or closed ([`Transfers`]); else this core's own connections. Either way the core decides what is
//! fetched and where it goes, and checks every piece and every file; the host only moves bytes.
#![expect(clippy::disallowed_methods, reason = "writes a model to this device a piece at a time as it downloads: airband_io writes files whole")]
use super::*;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc;

/// How far the fetch now running has got: bytes so far, and bytes in all.
static GOT: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
/// One fetch at a time.
static FETCHING: Mutex<()> = Mutex::new(());
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
    /// that is already under way leaves that one going and starts nothing more.
    fn start(&self, id: String, url: String, from: u64, until: u64, to: String);
}

/// What is heard about a piece under way.
enum Event {
    /// This many bytes of it have come.
    Moved { id: String, bytes: u64 },
    /// The host's transfer ended: with the status the server answered, or none when it got no answer.
    Ended { id: String, status: Option<u16> },
    /// This core's connection fetched it, or why not, in words for him.
    Own { id: String, went: Result<(), String> },
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
}

/// How far the fetch now running has got, in hundredths. For a progress bar, asked a few times a second.
pub(crate) fn local_model_progress() -> u32 {
    let of = OF.load(Ordering::SeqCst);
    (GOT.load(Ordering::SeqCst).min(of) * 100).checked_div(of).unwrap_or(0) as u32
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

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
fn part_of(path: &Path) -> PathBuf {
    with_suffix(path, ".part")
}
fn piece_at(path: &Path, from: u64) -> PathBuf {
    with_suffix(path, &format!(".from-{from}"))
}
fn len_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(|f| f.is_file()).map(|f| f.len())
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

/// Why a fetch stopped, in words for him.
const STOPPED: &str = "The download stopped part way. Try again.";
const UNREACHED: &str = "Could not reach the download. Check the connection and try again.";
const NOT_WRITTEN: &str = "Could not write the model to this device.";
const NOT_RESUMED: &str = "The download could not go on from where it stopped. Try again.";

/// Fetch a model, with the host's transfers when there are some, else over this core's own connections.
/// Blocks until it is all here: call it off the main thread. What is here already, whole files and pieces, is
/// kept, so one that was cut short goes on from there.
pub(crate) fn fetch_local_model(dir: String, m: &Model, host: Option<Arc<dyn Transfers>>) -> Result<(), RadioError> {
    let told = crate::choices::told;
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = version_folder(&dir, m);
    OF.store(m.files.iter().map(|f| f.1).sum(), Ordering::SeqCst);
    GOT.store(0, Ordering::SeqCst);
    let mut files: Vec<Building> = Vec::new();
    let mut wanted: Vec<Piece> = Vec::new();
    // pieces already here and whole, by file and where each starts
    let mut ready: BTreeMap<(usize, u64), Piece> = BTreeMap::new();
    for (i, (to, size, sha)) in m.files.iter().enumerate() {
        let path = folder.join(to);
        let whole = len_of(&path) == Some(*size);
        let mut joined = 0;
        if !whole {
            std::fs::create_dir_all(path.parent().unwrap_or(&folder)).map_err(|_| told("Could not make room for the model on this device."))?;
            joined = len_of(&part_of(&path)).unwrap_or(0);
            if joined > *size {
                std::fs::remove_file(part_of(&path)).ok();
                joined = 0;
            }
            let plan = ranges(joined, *size);
            // a piece left by an earlier try that is not one of these is of no use
            let stem = format!("{}.from-", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            for e in std::fs::read_dir(path.parent().unwrap_or(&folder)).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.strip_prefix(&stem).is_some_and(|from| !plan.iter().any(|(a, _)| a.to_string() == from)) {
                    std::fs::remove_file(e.path()).ok();
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
                        std::fs::remove_file(&p.to).ok();
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
    let outcome = std::thread::scope(|scope| {
        match &host {
            Some(host) => {
                *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
                // every piece at once: the system goes on with all of them while the app is not running
                for p in &wanted {
                    host.start(p.id.clone(), p.url.clone(), p.from, p.until, p.to.display().to_string());
                }
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
            GOT.store(total(&files, &ready, &moving), Ordering::SeqCst);
            if files.iter().all(|f| f.whole) {
                break Ok(());
            }
            let Ok(event) = rx.recv() else { break Err(STOPPED.to_string()) };
            let (id, ended) = match event {
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
            match ended.and_then(|()| if len_of(&p.to) == Some(p.len()) { Ok(()) } else { Err(STOPPED.to_string()) }) {
                Ok(()) => {
                    ready.insert((p.file, p.from), p);
                }
                Err(why) => {
                    if host.is_some() {
                        std::fs::remove_file(&p.to).ok();
                    }
                    break Err(why);
                }
            }
        };
        stop.store(true, Ordering::SeqCst);
        *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        went
    });
    outcome.map_err(crate::choices::told)?;
    // This version is whole: say so, and only now let go of the one that was here before it, which was in use
    // until this moment.
    std::fs::write(folder.join(WHOLE), "").map_err(|_| told(NOT_WRITTEN))?;
    for other in std::fs::read_dir(home(&dir, m)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| *p != folder) {
        if other.is_dir() { std::fs::remove_dir_all(&other) } else { std::fs::remove_file(&other) }.ok();
    }
    Ok(())
}

/// What the host's word on a transfer that ended means for its piece of a file of `size` bytes: the server sent
/// the range asked for, or the whole file when that was what was asked. Its length is checked after.
fn host_said(status: Option<u16>, p: &Piece, size: u64) -> Result<(), String> {
    match status {
        None => Err(STOPPED.into()),
        Some(206) => Ok(()),
        Some(200) if p.from == 0 && p.until == size => Ok(()),
        Some(200) => Err(NOT_RESUMED.into()),
        Some(other) => Err(not_fetched(other)),
    }
}

/// Join to each file the pieces that come next in it, in order; a file whose bytes are all joined is checked
/// against its hash, over the whole of it, and only then takes its name.
fn join(files: &mut [Building], ready: &mut BTreeMap<(usize, u64), Piece>) -> Result<(), String> {
    for (i, f) in files.iter_mut().enumerate().filter(|(_, f)| !f.whole) {
        let part = part_of(&f.path);
        while let Some(p) = ready.remove(&(i, f.joined)) {
            let mut from = std::fs::File::open(&p.to).map_err(|_| NOT_WRITTEN.to_string())?;
            let mut into = std::fs::OpenOptions::new().create(true).append(true).open(&part).map_err(|_| NOT_WRITTEN.to_string())?;
            let copied = std::io::copy(&mut from, &mut into).map_err(|_| "This device ran out of room for the model.".to_string())?;
            into.sync_all().ok();
            if copied != p.len() {
                return Err(NOT_WRITTEN.into());
            }
            f.joined += copied;
            std::fs::remove_file(&p.to).ok();
        }
        if f.joined == f.size {
            // a sign-in page, a connection that closed early, a file changed at the source: none of them is
            // the model
            if hash_of(&part).as_deref() != Some(f.sha.as_str()) {
                std::fs::remove_file(&part).ok();
                f.joined = 0;
                return Err("What came down was not the model. Check the connection and try again.".into());
            }
            std::fs::rename(&part, &f.path).map_err(|_| NOT_WRITTEN.to_string())?;
            f.whole = true;
        }
    }
    Ok(())
}

/// The SHA-256 of a file, in hex.
fn hash_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hash, &buf[..n]);
    }
    Some(airband_core::encode::hex(&sha2::Digest::finalize(hash)))
}

/// Fetch one piece over this core's own connection, going on from what of it is here already. Ok when the
/// server sent what was asked for to the end.
fn fetch_own(client: &crate::net::Client, p: &Piece, size: u64, tx: &mpsc::Sender<Event>, stop: &AtomicBool) -> Result<(), String> {
    let have = len_of(&p.to).unwrap_or(0);
    if have >= p.len() {
        return Ok(());
    }
    let start = p.from + have;
    let mut res = client.send(crate::net::Ask::get(&p.url).header("range", format!("bytes={start}-{}", p.until - 1))).map_err(|_| UNREACHED.to_string())?;
    match res.status().as_u16() {
        206 => {}
        // a server that sends the whole file for a range is taken at its word only when the whole file was asked for
        200 if start == 0 && p.until == size => {}
        200 => return Err(NOT_RESUMED.into()),
        other => return Err(not_fetched(other)),
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&p.to).map_err(|_| NOT_WRITTEN.to_string())?;
    let mut body = res.body_mut().as_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = have;
    while got < p.len() {
        if stop.load(Ordering::SeqCst) {
            return Err(STOPPED.into());
        }
        let n = body.read(&mut buf).map_err(|_| STOPPED.to_string())?;
        if n == 0 {
            break;
        }
        // never more than was asked for, whatever came
        let n = n.min((p.len() - got) as usize);
        file.write_all(&buf[..n]).map_err(|_| "This device ran out of room for the model.".to_string())?;
        got += n as u64;
        tx.send(Event::Moved { id: p.id.clone(), bytes: got }).ok();
    }
    if got == p.len() { Ok(()) } else { Err(STOPPED.into()) }
}

/// What he is told when the download answered with something other than the file, by what it answered.
fn not_fetched(status: u16) -> String {
    match status {
        404 | 410 => "The download is not there.".into(),
        429 => "The download was being asked for too often. Try again in a while.".into(),
        500..=599 => format!("The place the download comes from ran into trouble ({status}). Try again later."),
        _ => format!("The download could not be fetched ({status})."),
    }
}

#[cfg(test)]
mod tests;
