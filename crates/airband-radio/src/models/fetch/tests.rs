//! Fetching against a server on this machine that serves byte ranges as a CDN does, and can be made to drop a
//! connection part way, ignore ranges, or send the wrong bytes.
use super::*;
use airband_core::providers::Job;
use std::net::{TcpListener, TcpStream};

/// Wait until `fact` holds, woken each time the fetch or a stand-in host does something (`HAPPENED`). The minute
/// is only a bound on a test that has gone wrong, so it fails instead of hanging.
fn when<T>(what: &str, fact: impl Fn() -> Option<T>) -> T {
    let end = std::time::Instant::now() + Duration::from_secs(60);
    let mut seen = HAPPENED.0.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(it) = fact() {
            return it;
        }
        let left = end.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "it never came to pass: {what}");
        seen = HAPPENED.1.wait_timeout(seen, left).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// A stand-in host did something a test may be waiting on.
fn host_did() {
    *HAPPENED.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    HAPPENED.1.notify_all();
}

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
    let _one = one_fetching_test();
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
    let _one = one_fetching_test();
    let files = files();
    // the first answer is cut after a megabyte
    let server = serve(files.clone(), Ways { drop_after: Some(1 << 20), ..Ways::default() });
    let (dir, at) = folder();
    let m = model(&server.base, &files);
    let first = fetch_local_model(at.clone(), &m, None);
    assert_eq!(first, Err(FetchFailed::Stopped));
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
    let _one = one_fetching_test();
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
    let _one = one_fetching_test();
    let files = files();
    let server = serve(files.clone(), Ways { wrong: true, ..Ways::default() });
    let (dir, at) = folder();
    let went = fetch_local_model(at, &model(&server.base, &files), None);
    assert_eq!(went, Err(FetchFailed::NotTheModel));
    let v = dir.join("models/tiny/1");
    assert!(files.iter().all(|(n, _)| !v.join(n).exists() && !part_of(&v.join(n)).exists()) && !v.join(WHOLE).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_server_that_ignores_ranges_is_not_taken_for_a_piece() {
    let _one = one_fetching_test();
    let files = files();
    let server = serve(files.clone(), Ways { no_ranges: true, ..Ways::default() });
    let (dir, at) = folder();
    let went = fetch_local_model(at, &model(&server.base, &files), None);
    assert_eq!(went, Err(FetchFailed::NotResumed));
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
    /// Never end the piece that starts here: a transfer the system put off.
    stuck_at: Option<u64>,
    /// The transfers stopped, by id.
    stopped: Mutex<Vec<String>>,
    /// How many transfers have told the fetch they ended: told in order with what he says after.
    ended: Arc<Mutex<usize>>,
}

impl Moves {
    fn of(files: &[(String, Vec<u8>)]) -> Moves {
        Moves { files: files.to_vec(), started: Mutex::new(Vec::new()), short_at: None, stuck_at: None, stopped: Mutex::new(Vec::new()), ended: Arc::default() }
    }
}

impl Transfers for Moves {
    fn start(&self, id: String, url: String, from: u64, until: u64, to: String) {
        let name = url.split_once("/m/").map(|(_, n)| n.to_string()).unwrap();
        self.started.lock().unwrap().push((name.clone(), from, until));
        host_did();
        let body = self.files.iter().find(|(n, _)| *n == name).unwrap().1[from as usize..until as usize].to_vec();
        let short = self.short_at == Some(from);
        if self.stuck_at == Some(from) {
            return;
        }
        let ended = self.ended.clone();
        std::thread::spawn(move || {
            heard(Event::Moved { id: id.clone(), bytes: body.len() as u64 / 2 });
            let body = if short { &body[..body.len() - 1] } else { &body[..] };
            std::fs::write(&to, body).unwrap();
            heard(Event::Ended { id, status: Some(206) });
            *ended.lock().unwrap() += 1;
            host_did();
        });
    }
    fn stop(&self, id: String) {
        self.stopped.lock().unwrap().push(id);
    }
}

#[test]
fn with_the_hosts_transfers_every_piece_is_handed_over_at_once_and_checked_here() {
    let _one = one_fetching_test();
    let files = files();
    let (dir, at) = folder();
    let m = model("https://example.invalid/m", &files);
    let host = Arc::new(Moves::of(&files));
    fetch_local_model(at.clone(), &m, Some(host.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    let mut started = host.started.lock().unwrap().clone();
    started.sort();
    assert_eq!(started, vec![("small.json".to_string(), 0, 600), ("weights/big.bin".into(), 0, PIECE), ("weights/big.bin".into(), PIECE, PIECE + PIECE / 2)]);
    std::fs::remove_dir_all(dir).ok();

    // a piece that came short is not joined, and is asked for again next time
    let (dir, at) = folder();
    let host = Arc::new(Moves { short_at: Some(PIECE), ..Moves::of(&files) });
    assert!(fetch_local_model(at.clone(), &m, Some(host.clone())).is_err());
    assert!(!dir.join("models/tiny/1/weights/big.bin").exists());
    let whole = Arc::new(Moves::of(&files));
    fetch_local_model(at, &m, Some(whole.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    assert!(whole.started.lock().unwrap().iter().any(|(_, from, _)| *from == PIECE));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_download_that_fails_is_told_by_what_it_answered() {
    let _one = one_fetching_test();
    let files = files();
    let (dir, at) = folder();
    // a server that has nothing there
    let server = serve(Vec::new(), Ways::default());
    assert_eq!(fetch_local_model(at, &model(&server.base, &files), None), Err(FetchFailed::Status(404)));
    // each answer is told in words of its own
    let all = [FetchFailed::Stopped, FetchFailed::Cancelled, FetchFailed::Unreached, FetchFailed::NotWritten, FetchFailed::NoRoom, FetchFailed::NotResumed, FetchFailed::NotTheModel, FetchFailed::Status(404), FetchFailed::Status(429), FetchFailed::Status(503), FetchFailed::Status(403)];
    let said: std::collections::HashSet<String> = all.iter().map(|f| f.said()).collect();
    assert_eq!(said.len(), all.len());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_fetch_he_stops_ends_and_lets_the_next_one_start() {
    let _one = one_fetching_test();
    let files = files();
    let (dir, at) = folder();
    let m = model("https://example.invalid/m", &files);
    // the system puts off the second piece of the big file, for good
    let host = Arc::new(Moves { stuck_at: Some(PIECE), ..Moves::of(&files) });
    let fetching = {
        let (at, m, host) = (at.clone(), m.clone(), host.clone());
        std::thread::spawn(move || fetch_local_model(at, &m, Some(host)))
    };
    // once the stuck one is all that is left, he stops it
    // (every other piece has told the fetch it ended, before he does)
    when("every piece has started and every one but the stuck one has ended", || (host.started.lock().unwrap().len() == 3 && *host.ended.lock().unwrap() == 2).then_some(()));
    // what `Desk::stop_model_fetch` tells
    heard(Event::Stop);
    assert_eq!(fetching.join().unwrap(), Err(FetchFailed::Cancelled));
    // its transfer is stopped, and another fetch can start: it goes on from what came
    assert_eq!(host.stopped.lock().unwrap().len(), 1);
    let whole = Arc::new(Moves::of(&files));
    fetch_local_model(at, &m, Some(whole.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    assert_eq!(whole.started.lock().unwrap().iter().map(|(_, from, _)| *from).collect::<Vec<_>>(), [PIECE]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_piece_that_came_before_the_fetch_was_listening_is_taken_and_none_is_left_behind() {
    let _one = one_fetching_test();
    let files = files();
    let (dir, at) = folder();
    let m = model("https://example.invalid/m", &files);
    // a transfer from before ended whole, with nobody listening, and another left a piece of no use
    let big = dir.join("models/tiny/1/weights/big.bin");
    std::fs::create_dir_all(big.parent().unwrap()).unwrap();
    std::fs::write(piece_at(&big, 0), &files[0].1[..PIECE as usize]).unwrap();
    let host = Arc::new(Moves::of(&files));
    fetch_local_model(at, &m, Some(host.clone())).unwrap();
    assert!(here_whole(&dir, &files));
    // it was not asked for again
    assert!(host.started.lock().unwrap().iter().all(|(n, from, _)| !(n == "weights/big.bin" && *from == 0)));
    // and once the version is whole, a piece a late transfer put there goes too
    std::fs::write(piece_at(&big, PIECE), b"late").unwrap();
    sweep_pieces(&dir.join("models/tiny/1"));
    assert_eq!(std::fs::read_dir(big.parent().unwrap()).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pieces_meet_on_the_same_boundaries_whatever_is_here() {
    assert_eq!(ranges(0, 10), vec![(0, 10)]);
    assert_eq!(ranges(0, 2 * PIECE + 1), vec![(0, PIECE), (PIECE, 2 * PIECE), (2 * PIECE, 2 * PIECE + 1)]);
    assert_eq!(ranges(5, PIECE + 1), vec![(5, PIECE), (PIECE, PIECE + 1)]);
    assert!(ranges(7, 7).is_empty());
}

/// Transfers the test moves itself, a step at a time: each piece waits to be told how far it has got and when
/// it ends.
#[derive(Default)]
struct Gated {
    started: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl Transfers for Gated {
    fn start(&self, id: String, url: String, from: u64, until: u64, to: String) {
        let name = url.split_once("/m/").map(|(_, n)| n.to_string()).unwrap();
        let body = files().into_iter().find(|(n, _)| *n == name).unwrap().1[from as usize..until as usize].to_vec();
        self.started.lock().unwrap().push((id, to, body));
        host_did();
    }
    fn stop(&self, _: String) {}
}

#[test]
fn progress_counts_bytes_under_way_only_goes_up_and_is_whole_only_when_the_files_are() {
    let _one = one_fetching_test();
    let files = files();
    let (dir, at) = folder();
    let m = model("https://example.invalid/m", &files);
    let host = Arc::new(Gated::default());
    let fetching = {
        let (at, m, host) = (at.clone(), m.clone(), host.clone());
        std::thread::spawn(move || fetch_local_model(at, &m, Some(host)))
    };
    let until = |what: &str, fact: &dyn Fn(&Progress) -> bool| when(what, || Some(local_model_progress()).filter(|p| fact(p)));
    let pieces = when("every piece has started", || Some(host.started.lock().unwrap().clone()).filter(|p| p.len() == 3));
    assert_eq!(pieces.len(), 3);
    let of: u64 = files.iter().map(|f| f.1.len() as u64).sum();
    let mut seen = until("it has begun", &|p| p.of == of);
    assert_eq!((seen.got, seen.fraction(), seen.said()), (0, 0.0, format!("Downloading 0 of {}\u{a0}MB", megabytes(of))));
    // half of a piece under way is counted as it comes, before the piece is whole
    let (id, _, body) = &pieces[0];
    heard(Event::Moved { id: id.clone(), bytes: body.len() as u64 / 2 });
    let now = until("bytes under way are counted", &|p| p.got == body.len() as u64 / 2);
    assert!(now.fraction() > 0.0 && now.fraction() < 1.0);
    seen = now;
    // every byte here, the last still to be joined and checked: never shown as done
    for (id, to, body) in &pieces {
        heard(Event::Moved { id: id.clone(), bytes: body.len() as u64 });
        let now = local_model_progress();
        assert!(now.got >= seen.got, "it never goes back: {seen:?} then {now:?}");
        seen = now;
        if id != &pieces[2].0 {
            std::fs::write(to, body).unwrap();
            heard(Event::Ended { id: id.clone(), status: Some(206) });
        }
    }
    let all = until("every byte is counted", &|p| p.got == of);
    assert!(all.fraction() < 1.0 && !all.whole, "{all:?}");
    assert_ne!(all.said(), format!("Downloading {0} of {0}\u{a0}MB", megabytes(of)));
    let (id, to, body) = &pieces[2];
    std::fs::write(to, body).unwrap();
    heard(Event::Ended { id: id.clone(), status: Some(206) });
    fetching.join().unwrap().unwrap();
    let done = local_model_progress();
    assert!(done.whole && here_whole(&dir, &files));
    assert_eq!((done.fraction(), done.said()), (1.0, format!("Downloading {0} of {0}\u{a0}MB", megabytes(of))));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_download_says_it_waits_for_the_network_only_when_the_phone_has_none() {
    let p = Progress { got: 23_400_000, of: 66_000_000, whole: false, offline: false, kind: None };
    assert_eq!(p.said(), "Downloading 23 of 66\u{a0}MB");
    assert_eq!(Progress { offline: true, ..p.clone() }.said(), "Waiting for the network");
    // one that is whole is never waiting
    assert_eq!(Progress { offline: true, whole: true, got: 66_000_000, ..p }.said(), "Downloading 66 of 66\u{a0}MB");
}
