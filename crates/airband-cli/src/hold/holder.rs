//! The holder itself: the program it keeps, the socket it listens on, and the two drivers (see `hold.rs`).
use super::{Answer, By, NotStarted, State, Tell, ToHolder};
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{ChildStdin, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Why the holder could not read a line the tower told it, said by where and of what kind, never by the
/// parser's own words: those quote what was in the line, which can be his words, and this reason is logged.
fn unreadable(e: &serde_json::Error) -> String {
    format!("its holder could not read what it was told ({:?}, at column {})", e.classify(), e.column())
}

impl Answer {
    fn line(&self) -> String {
        serde_json::to_string(self).expect("a holder's answer writes as JSON")
    }
}

pub(crate) enum Msg {
    Tell(Tell),
    /// A line the program printed (only passed on to a driver that reads them).
    Out(String),
    /// The program has ended.
    Gone,
    /// A file in a folder the driver watches has changed: where the agent's tool keeps a record the driver
    /// reads (`acp.rs`).
    Changed,
}

/// What a driver is given.
pub(crate) struct Held {
    pub(crate) id: String,
    pub(crate) harness: String,
    pub(crate) cwd: String,
    pub(crate) rx: mpsc::Receiver<Msg>,
    /// Where something the driver itself watches says it has changed ([`Msg::Changed`]).
    pub(crate) tx: mpsc::Sender<Msg>,
    pub(crate) to_program: Arc<Mutex<Option<ChildStdin>>>,
    pub(crate) state: Arc<Mutex<State>>,
    /// The last of what the program printed as an error.
    pub(crate) errors: Arc<Mutex<String>>,
}

impl Held {
    pub(crate) fn write(&self, line: &str) {
        if let Some(to) = self.to_program.lock().unwrap().as_mut() {
            writeln!(to, "{line}").and_then(|()| to.flush()).ok();
        }
    }
    pub(crate) fn close_input(&self) {
        *self.to_program.lock().unwrap() = None;
    }
    pub(crate) fn ready(&self, session: Option<String>, offers: airband_core::tools::Offers) {
        *self.state.lock().unwrap() = State::Ready { session, offers };
    }
    /// Why it failed, with what the program itself had to say.
    pub(crate) fn why(&self, what: &str) -> String {
        let said = self.errors.lock().unwrap().trim().to_string();
        if said.is_empty() { what.to_string() } else { format!("{what}: {said}") }
    }
}

/// The driver for a program that reads one message a line and reports by itself.
fn lines(held: &Held, line: &str, stop: Option<&str>) -> Result<(), String> {
    // A program that cannot run ends at once. One still there after a moment is taken to be reading its
    // input: a rule, not a fact, since a program driven this way gives no sign of its own that it is ready
    // (one that speaks the Agent Client Protocol does, and is waited for: `acp`).
    match held.rx.recv_timeout(Duration::from_millis(400)) {
        Ok(Msg::Gone) => return Err(held.why("it ended as soon as it started")),
        Ok(Msg::Tell(first)) => {
            held.ready(None, Default::default());
            told(held, first, line, stop);
        }
        _ => held.ready(None, Default::default()),
    }
    loop {
        match held.rx.recv() {
            Ok(Msg::Tell(tell)) => told(held, tell, line, stop),
            Ok(Msg::Out(_) | Msg::Changed) => {}
            Ok(Msg::Gone) | Err(_) => return Ok(()),
        }
    }
}
/// The row's line for his words, his words and the receipt each put in once, as JSON strings: words of his that
/// read like a place (`{receipt}`) stay his words.
fn said_line(line: &str, text: &str, receipt: &str) -> String {
    airband_core::template::filled(line, &[("text", &Value::String(text.to_string()).to_string()), ("receipt", &Value::String(receipt.to_string()).to_string())])
}

fn told(held: &Held, tell: Tell, line: &str, stop: Option<&str>) {
    match tell {
        Tell::Say(text, receipt) => held.write(&said_line(line, &text, &receipt)),
        Tell::Stop => {
            if let Some(stop) = stop {
                held.write(&airband_core::template::filled(stop, &[("id", &crate::config::random_hex(8))]));
            }
        }
        Tell::End => held.close_input(),
    }
}

impl By {
    fn read(word: &str) -> Option<By> {
        [By::Lines, By::Acp].into_iter().find(|by| by.word() == word)
    }
}

/// Run a coding tool's program held for the tower (`airband hold --sock … --by … -- program args`), driving it
/// as `--by` says until it ends.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let crate::Words { flags: named, rest: stray, after: argv, .. } = crate::words("hold", args, &["--sock", "--cwd", "--id", "--harness", "--by", "--line", "--stop", "--session", "--mode", "--model", "--effort"], &[])?;
    if let Some(word) = stray.first() {
        return Err(format!("airband hold does not take \"{word}\""));
    }
    let sock = named.get("sock").cloned().ok_or("airband hold needs --sock")?;
    let (program, rest) = argv.split_first().ok_or("airband hold needs a program after --")?;
    // which of the two drivers: the tower always says, and nothing is assumed when it does not
    let by = named.get("by").map(String::as_str).ok_or("airband hold needs --by")?;
    let by = By::read(by).ok_or_else(|| format!("this version of airband does not drive a program by \"{by}\""))?;
    // a terminal closing, or the tower that started this going away, must not take the agent with it
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    // where it works, and who it is: the tower always says, and nothing is made up when it does not
    let cwd = named.get("cwd").cloned().ok_or("airband hold needs --cwd")?;
    let (id, harness) = (named.get("id").cloned().ok_or("airband hold needs --id")?, named.get("harness").cloned().ok_or("airband hold needs --harness")?);
    // all it needs is known before the program starts: one started and then given up on would be left running
    let line = match by {
        By::Lines => Some(named.get("line").cloned().ok_or("airband hold --by lines needs --line")?),
        By::Acp => None,
    };
    // the socket is opened only once every argument has been read: one opened for a holder that then gives up
    // would sit there with nobody answering, and no state to say why
    let state = Arc::new(Mutex::new(State::Starting));
    let (tx, rx) = mpsc::channel::<Msg>();
    listen(&sock, state.clone(), tx.clone())?;
    let mut command = airband_io::child::command(program);
    command.args(rest).current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // A program whose holder reports for it stays quiet itself, hooks and all: two reports of the same thing
    // would be read to him twice.
    if by == By::Acp {
        command.env("AIRBAND_INTERNAL", "1");
    }
    let errors = Arc::new(Mutex::new(String::new()));
    let outcome = match command.spawn() {
        Err(e) => Err(NotStarted::Failed(format!("{program} did not start: {e}"))),
        Ok(mut child) => {
            pass_output(&mut child, tx.clone(), by == By::Acp);
            keep_errors(&mut child, errors.clone());
            let held = Held { id, harness, cwd, rx, tx, to_program: Arc::new(Mutex::new(child.stdin.take())), state: state.clone(), errors };
            // a line to write is what a program driven by lines has, and only that
            let done = match &line {
                Some(line) => lines(&held, line, named.get("stop").map(String::as_str)).map_err(NotStarted::Failed),
                None => crate::acp::run(&held, named.get("session").cloned(), named.get("mode").map(String::as_str), crate::acp::Asked { model: named.get("model").cloned(), effort: named.get("effort").cloned() }),
            };
            held.close_input();
            end(&mut child);
            done
        }
    };
    if let Err(why) = &outcome {
        // it never became an agent: say why to whoever asks, for long enough that the tower has asked
        *state.lock().unwrap() = match why {
            NotStarted::NoSuch(no) => State::NoSuch(no.clone()),
            other => State::Failed { why: other.to_string() },
        };
        std::thread::sleep(Duration::from_secs(15));
    }
    airband_io::files::remove(std::path::Path::new(&sock)).ok();
    end_group();
    outcome.map_err(|e| e.to_string())
}


/// The socket the tower talks to it through, open to this user only: asked how it stands, it says; told
/// something for the agent, it passes it on.
fn listen(sock: &str, state: Arc<Mutex<State>>, tx: mpsc::Sender<Msg>) -> Result<(), String> {
    airband_io::files::remove(std::path::Path::new(sock)).ok();
    let listener = UnixListener::bind(sock).map_err(|e| format!("cannot open {sock}: {e}"))?;
    {
        // only he may talk to the agent: a socket others could reach is not opened at all
        airband_io::files::set_mode(std::path::Path::new(sock), 0o600).map_err(|e| format!("cannot make {sock} private: {e}"))?;
    }
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut said = String::new();
            if conn.read_to_string(&mut said).is_err() {
                continue;
            }
            for line in said.lines() {
                let tell = match serde_json::from_str::<ToHolder>(line) {
                    Ok(ToHolder::Status) => {
                        writeln!(conn, "{}", serde_json::to_string(&*state.lock().unwrap()).unwrap()).ok();
                        continue;
                    }
                    Ok(ToHolder::Say { text, receipt }) => Tell::Say(text, receipt),
                    Ok(ToHolder::Stop) => Tell::Stop,
                    Ok(ToHolder::End) => Tell::End,
                    Err(e) => {
                        writeln!(conn, "{}", Answer::Refused { why: unreadable(&e) }.line()).ok();
                        continue;
                    }
                };
                // its program has gone and nothing reads this any more: not taken
                let answer = match tx.send(Msg::Tell(tell)) {
                    Ok(()) => Answer::Taken,
                    Err(_) => Answer::Refused { why: "it has ended".into() },
                };
                writeln!(conn, "{}", answer.line()).ok();
            }
        }
    });
    Ok(())
}

/// What the program writes, line by line, for a driver that reads it; and that it has ended.
fn pass_output(child: &mut std::process::Child, tx: mpsc::Sender<Msg>, wanted: bool) {
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).split(b'\n').map_while(Result::ok) {
                if wanted {
                    tx.send(Msg::Out(String::from_utf8_lossy(&line).into_owned())).ok();
                }
            }
            tx.send(Msg::Gone).ok();
        });
    }
}

/// The last of what the program writes as errors, which is what says why it failed.
fn keep_errors(child: &mut std::process::Child, errors: Arc<Mutex<String>>) {
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                let mut kept = errors.lock().unwrap();
                kept.push_str(&line);
                kept.push('\n');
                // the last of it is what says why
                let over = kept.chars().count().saturating_sub(600);
                if over > 0 {
                    *kept = kept.chars().skip(over).collect();
                }
            }
        });
    }
}

/// Asked to finish, it is given a moment to; then it is ended.
fn end(child: &mut std::process::Child) {
    for _ in 0..150 {
        if child.try_wait().is_ok_and(|ended| ended.is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().ok();
    child.wait().ok();
}

/// Whatever the program started and left behind (a package runner's child, a server of the tool's own) goes
/// with it. The tower starts each holder as the leader of a group of its own; one that is not, is not
/// the keeper of the group it is in, and ends nothing but its own program.
fn end_group() {
    unsafe {
        if libc::getpgrp() == libc::getpid() {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::kill(0, libc::SIGTERM);
        }
    }
}

#[cfg(test)]
mod by_tests {
    use super::*;

    #[test]
    fn a_holder_missing_an_argument_opens_no_socket() {
        let sock = std::env::temp_dir().join(format!("airband-hold-{}.sock", crate::config::random_hex(4)));
        let args: Vec<String> = ["--sock", sock.to_str().unwrap(), "--by", "lines", "--cwd", "/tmp", "--id", "a", "--harness", "t", "--", "true"].iter().map(|s| s.to_string()).collect();
        assert!(run(&args).unwrap_err().contains("--line"));
        assert!(!sock.exists(), "no socket is left behind");
    }

    #[test]
    fn the_holder_kind_is_read_back_as_the_row_named_it() {
        let lines = airband_core::tools::Held::Lines { fresh: Vec::new(), resume: Vec::new(), line: "{text}".into(), stop: None };
        assert_eq!(By::read(By::of(&lines).word()), Some(By::Lines));
        assert_eq!(By::read("lines "), None);
    }

    #[test]
    fn a_line_the_holder_cannot_read_is_refused_without_quoting_it() {
        let e = serde_json::from_str::<ToHolder>(r#"{"line":"buy the blue umbrella"}"#).map(|_| ()).unwrap_err();
        assert!(e.to_string().contains("umbrella"), "the parser's own words quote the line");
        let why = unreadable(&e);
        assert!(!why.contains("umbrella") && why.contains("could not read"), "{why}");
    }
}

#[cfg(test)]
mod said_line_tests {
    #[test]
    fn his_words_go_into_the_line_once_and_are_never_read_again_as_a_place() {
        assert_eq!(super::said_line(r#"{"say":{text},"r":{receipt}}"#, "keep {receipt} as is", "r1"), r#"{"say":"keep {receipt} as is","r":"r1"}"#);
    }
}
