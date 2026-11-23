//! Fetching against a server on this machine that serves byte ranges as a CDN does, and can be made to drop a
//! connection part way, ignore ranges, or send the wrong bytes.
use super::*;
use airband_core::providers::Job;
use std::net::{TcpListener, TcpStream};

/// How the test server behaves.
#[derive(Clone, Copy, Default)]
struct Ways {
    /// Answer a range with the whole file, as a server that does not do ranges.
    no_ranges: bool,
    /// Close the first answer that asks for more than this many bytes after sending this many.
    drop_after: Option<usize>,
    /// Send other bytes than the file's.
    wrong: bool,
}

struct Server {
    base: String,
    /// The ranges asked for, by path: (from, until), until not included.
    asked: Arc<Mutex<Vec<(String, u64, u64)>>>,
}

fn serve(files: Vec<(String, Vec<u8>)>, ways: Ways) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/m", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let files = Arc::new(files);
    let dropped = Arc::new(AtomicBool::new(false));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (files, log, dropped) = (files.clone(), log.clone(), dropped.clone());
            std::thread::spawn(move || answer(stream, &files, &log, &dropped, ways));
        }
    });
    Server { base, asked }
}

fn answer(mut stream: TcpStream, files: &[(String, Vec<u8>)], log: &Mutex<Vec<(String, u64, u64)>>, dropped: &AtomicBool, ways: Ways) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let path = head.split(' ').nth(1).unwrap_or_default().trim_start_matches("/m/").to_string();
    let Some((_, body)) = files.iter().find(|(name, _)| *name == path) else {
        stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").ok();
        return;
    };
    let body: Vec<u8> = if ways.wrong { body.iter().map(|b| b ^ 0xff).collect() } else { body.clone() };
    let range = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string)).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()? + 1))
    });
    let (from, until) = range.unwrap_or((0, body.len() as u64));
    log.lock().unwrap().push((path, from, until));
    let (status, part) = match range {
        Some((a, b)) if !ways.no_ranges => (format!("206 Partial Content\r\ncontent-range: bytes {a}-{}/{}", b - 1, body.len()), &body[a as usize..b as usize]),
        _ => ("200 OK".to_string(), &body[..]),
    };
    stream.write_all(format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", part.len()).as_bytes()).ok();
    match ways.drop_after {
        Some(n) if part.len() > n && !dropped.swap(true, Ordering::SeqCst) => {
            stream.write_all(&part[..n]).ok();
            stream.flush().ok();
            // cut off part way
        }
        _ => {
            stream.write_all(part).ok();
        }
    }
}

/// Bytes that differ from piece to piece, so a piece put in the wrong place is caught.
fn bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u64).wrapping_mul(2654435761).to_le_bytes()[1] ^ seed).collect()
}

fn sha(b: &[u8]) -> String {
    airband_core::encode::hex(&<sha2::Sha256 as sha2::Digest>::digest(b))
}

/// A model with a file of a piece and a half (three pieces' worth of ranges at most) and a small one.
fn model(base: &str, files: &[(String, Vec<u8>)]) -> Model {
    Model { kind: "tiny".into(), name: "Tiny".into(), job: Job::Voice, engine: "ptts".into(), version: "1".into(), base: base.into(), files: files.iter().map(|(n, b)| (n.clone(), b.len() as u64, sha(b))).collect(), voices: Vec::new(), usual_voice: String::new(), credit: String::new(), with: Default::default() }
}

fn files() -> Vec<(String, Vec<u8>)> {
    vec![("weights/big.bin".into(), bytes((PIECE + PIECE / 2) as usize, 7)), ("small.json".into(), bytes(600, 9))]
}

/// Every byte kept under a folder.
fn on_disk(dir: &Path) -> u64 {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| if e.path().is_dir() { on_disk(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()
}

fn folder() -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("airband-fetch-{}", random_hex(4)));
    let at = dir.display().to_string();
    (dir, at)
}

fn here_whole(dir: &Path, files: &[(String, Vec<u8>)]) -> bool {
    files.iter().all(|(n, b)| std::fs::read(dir.join("models/tiny/1").join(n)).ok().as_deref() == Some(&b[..])) && dir.join("models/tiny/1/.whole").exists()
}

#[test]
fn a_model_comes_down_in_ranged_pieces_and_every_file_is_checked_whole() {
    let files = files();
    let server = serve(files.clone(), Ways::default());
    let (dir, at) = folder();
    fetch_local_model(at.clone(), &model(&server.base, &files), None).unwrap();
    assert!(here_whole(&dir, &files));
    // the big file in two pieces on the boundary, the small one whole; nothing asked twice
    let mut asked = server.asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, vec![("small.json".to_string(), 0, 600), ("weights/big.bin".into(), 0, PIECE), ("weights/big.bin".into(), PIECE, PIECE + PIECE / 2)]);
    // no piece or part is left behind
    assert!(std::fs::read_dir(dir.join("models/tiny/1/weights")).unwrap().count() == 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_fetch_cut_short_goes_on_from_the_bytes_it_has() {
    let files = files();
    // the first answer is cut after a megabyte
    let server = serve(files.clone(), Ways { drop_after: Some(1 << 20), ..Ways::default() });
    let (dir, at) = folder();
    let m = model(&server.base, &files);
    let first = fetch_local_model(at.clone(), &m, None);
    assert!(matches!(&first, Err(RadioError::Told { reason }) if reason == STOPPED), "{first:?}");
    assert!(!here_whole(&dir, &files));
    let before = server.asked.lock().unwrap().len();
    let kept = on_disk(&dir.join("models/tiny/1"));
    assert!(kept >= 1 << 20, "what came before the cut is kept");
    fetch_local_model(at.clone(), &m, None).unwrap();
    assert!(here_whole(&dir, &files));
    // tried again, only what was not here is asked for: each piece from where it stopped
    let again: u64 = server.asked.lock().unwrap()[before..].iter().map(|(_, from, until)| until - from).sum();
    let all: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
    assert_eq!(again, all - kept);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn what_was_joined_before_is_kept_and_only_the_rest_is_asked_for() {
    let files = files();
    let server = serve(files.clone(), Ways::default());
    let (dir, at) = folder();
    // the first piece of the big file is joined already, and the small file is whole
    let big = dir.join("models/tiny/1/weights/big.bin");
    std::fs::create_dir_all(big.parent().unwrap()).unwrap();
    std::fs::write(part_of(&big), &files[0].1[..PIECE as usize]).unwrap();
    std::fs::write(dir.join("models/tiny/1/small.json"), &files[1].1).unwrap();
    fetch_local_model(at, &model(&server.base, &files), None).unwrap();
    assert!(here_whole(&dir, &files));
    assert_eq!(server.asked.lock().unwrap().clone(), vec![("weights/big.bin".to_string(), PIECE, PIECE + PIECE / 2)]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn bytes_that_are_not_the_model_never_take_its_name() {
    let files = files();
    let server = serve(files.clone(), Ways { wrong: true, ..Ways::default() });
    let (dir, at) = folder();
    let went = fetch_local_model(at, &model(&server.base, &files), None);
    assert!(matches!(&went, Err(RadioError::Told { reason }) if reason.starts_with("What came down was not the model")), "{went:?}");
    let v = dir.join("models/tiny/1");
    assert!(files.iter().all(|(n, _)| !v.join(n).exists() && !part_of(&v.join(n)).exists()) && !v.join(WHOLE).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_server_that_ignores_ranges_is_not_taken_for_a_piece() {
    let files = files();
    let server = serve(files.clone(), Ways { no_ranges: true, ..Ways::default() });
    let (dir, at) = folder();
    let went = fetch_local_model(at, &model(&server.base, &files), None);
    assert!(matches!(&went, Err(RadioError::Told { reason }) if reason == NOT_RESUMED), "{went:?}");
    assert!(!dir.join("models/tiny/1/weights/big.bin").exists());
    std::fs::remove_dir_all(dir).ok();
}

/// A host whose transfers are threads that copy the bytes asked for out of memory, as a system transfer would
/// bring them, and say so as the host does.
struct Moves {
    files: Vec<(String, Vec<u8>)>,
    started: Mutex<Vec<(String, u64, u64)>>,
    /// Put this many bytes too few in the piece that starts here.
    short_at: Option<u64>,
}

impl Transfers for Moves {
    fn start(&self, id: String, url: String, from: u64, until: u64, to: String) {
        let name = url.split_once("/m/").map(|(_, n)| n.to_string()).unwrap();
        self.started.lock().unwrap().push((name.clone(), from, until));
        let body = self.files.iter().find(|(n, _)| *n == name).unwrap().1[from as usize..until as usize].to_vec();
        let short = self.short_at == Some(from);
        std::thread::spawn(move || {
            heard(Event::Moved { id: id.clone(), bytes: body.len() as u64 / 2 });
            let body = if short { &body[..body.len() - 1] } else { &body[..] };
            std::fs::write(&to, body).unwrap();
            heard(Event::Ended { id, status: Some(206) });
        });
    }
}

#[test]
fn with_the_hosts_transfers_every_piece_is_handed_over_at_once_and_checked_here() {
    let files = files();
    let (dir, at) = folder();
    let m = model("https://example.invalid/m", &files);
    let host = Arc::new(Moves { files: files.clone(), started: Mutex::new(Vec::new()), short_at: None });
    fetch_local_model(at.clone(), &m, Some(host.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    let mut started = host.started.lock().unwrap().clone();
    started.sort();
    assert_eq!(started, vec![("small.json".to_string(), 0, 600), ("weights/big.bin".into(), 0, PIECE), ("weights/big.bin".into(), PIECE, PIECE + PIECE / 2)]);
    std::fs::remove_dir_all(dir).ok();

    // a piece that came short is not joined, and is asked for again next time
    let (dir, at) = folder();
    let host = Arc::new(Moves { files: files.clone(), started: Mutex::new(Vec::new()), short_at: Some(PIECE) });
    assert!(fetch_local_model(at.clone(), &m, Some(host.clone())).is_err());
    assert!(!dir.join("models/tiny/1/weights/big.bin").exists());
    let whole = Arc::new(Moves { files: files.clone(), started: Mutex::new(Vec::new()), short_at: None });
    fetch_local_model(at, &m, Some(whole.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    assert!(whole.started.lock().unwrap().iter().any(|(_, from, _)| *from == PIECE));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_download_that_fails_is_told_by_what_it_answered() {
    assert_eq!(not_fetched(404), "The download is not there.");
    assert!(not_fetched(503).contains("trouble (503)"));
    assert!(not_fetched(429).contains("too often"));
    assert!(not_fetched(403).contains("(403)"));
}

#[test]
fn pieces_meet_on_the_same_boundaries_whatever_is_here() {
    assert_eq!(ranges(0, 10), vec![(0, 10)]);
    assert_eq!(ranges(0, 2 * PIECE + 1), vec![(0, PIECE), (PIECE, 2 * PIECE), (2 * PIECE, 2 * PIECE + 1)]);
    assert_eq!(ranges(5, PIECE + 1), vec![(5, PIECE), (PIECE, PIECE + 1)]);
    assert!(ranges(7, 7).is_empty());
}
