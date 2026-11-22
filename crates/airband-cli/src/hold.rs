//! `airband hold`: keeps one agent's program running, apart from the tower, and drives it.
//!
//! The tower starts one of these for each agent it runs itself, and then only ever talks to it through a
//! socket. So the agent does not end when the tower does: a tower that is updated or restarted finds the socket
//! again and carries on, and whatever the agent was in the middle of goes on undisturbed.
//!
//!   airband hold --sock <path> --cwd <folder> --id <agent id> --harness <tool> --by <lines|acp> [--model <name>] [--effort <name>] […] -- <program> [arguments...]
//!
//! What the tower says to it is one JSON line a connection, the same whatever the program is ([`Tell`]): his
//! words for the agent, stop what you are doing, finish, and how are you. How that reaches the program is the
//! driver's business (`--by`, the two kinds a launcher's row can name, `airband_core::tools::Held`):
//!
//! - `lines`: the program reads one message a line on its standard input (`--line` is what a line looks like,
//!   `--stop` the line that stops it) and reports to the tower by itself.
//! - `acp`: the program speaks the Agent Client Protocol; `acp.rs` speaks it back and reports for the agent.
//!   The agent says what models and efforts it can be set to when its session opens. `--model` and `--effort`
//!   name the ones he asked for, and an agent that has no such one never starts.
//!
//! To "how are you" it answers where it has got to ([`State`]): still starting, ready, or failed and why, with
//! the last of what the program printed as an error. The tower tells him an agent is there only once it is.
#![cfg(unix)]
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::os::unix::net::UnixListener;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;


/// What the tower says to a holder.
pub(crate) enum Tell {
    /// His words, as the agent's next turn, and the id this delivery of them goes under.
    Say(String, String),
    /// Leave off what you are doing.
    Stop,
    /// Finish: the program's input is closed, and it ends.
    End,
}

impl Tell {
    pub(crate) fn line(&self) -> String {
        match self {
            Tell::Say(text, receipt) => json!({ "say": text, "receipt": receipt }),
            Tell::Stop => json!({ "stop": true }),
            Tell::End => json!({ "end": true }),
        }
        .to_string()
    }
    fn read(line: &Value) -> Option<Tell> {
        match (line["say"].as_str(), line.get("stop"), line.get("end")) {
            (Some(text), _, _) => Some(Tell::Say(text.to_string(), line["receipt"].as_str().unwrap_or_default().to_string())),
            (None, Some(_), _) => Some(Tell::Stop),
            (None, None, Some(_)) => Some(Tell::End),
            _ => None,
        }
    }
}
/// The line that asks a holder where it has got to. The answer is a [`State`].
pub(crate) const STATUS: &str = r#"{"status":true}"#;

/// Where a holder has got to with its program.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub(crate) enum State {
    Starting,
    /// The agent is there and takes his words. `session` is what the agent's own tool calls its session, when
    /// that is not the id the tower gave it: what carries it on later.
    /// `offers` is what the agent itself said a session of its can be set to, when it says.
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        #[serde(default)]
        offers: airband_core::tools::Offers,
    },
    /// It did not start, and never will: why, in the program's own words where it had any.
    Failed { why: String },
}

pub(crate) enum Msg {
    Tell(Tell),
    /// A line the program printed (only passed on to a driver that reads them).
    Out(String),
    /// The program has ended.
    Gone,
}

/// What a driver is given.
pub(crate) struct Held {
    pub(crate) id: String,
    pub(crate) harness: String,
    pub(crate) cwd: String,
    pub(crate) rx: mpsc::Receiver<Msg>,
    pub(crate) to_program: Arc<Mutex<Option<ChildStdin>>>,
    pub(crate) state: Arc<Mutex<State>>,
    /// The last of what the program printed as an error.
    pub(crate) errors: Arc<Mutex<String>>,
}

impl Held {
    pub(crate) fn write(&self, line: &str) {
        if let Some(to) = self.to_program.lock().unwrap().as_mut() {
            let _ = writeln!(to, "{line}").and_then(|()| to.flush());
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
            Ok(Msg::Out(_)) => {}
            Ok(Msg::Gone) | Err(_) => return Ok(()),
        }
    }
}
fn told(held: &Held, tell: Tell, line: &str, stop: Option<&str>) {
    match tell {
        Tell::Say(text, receipt) => held.write(&line.replace("{text}", &Value::String(text).to_string()).replace("{receipt}", &Value::String(receipt).to_string())),
        Tell::Stop => {
            if let Some(stop) = stop {
                held.write(&stop.replace("{id}", &crate::config::random_hex(8)));
            }
        }
        Tell::End => held.close_input(),
    }
}

/// Run a coding tool's program held for the tower (`airband hold --sock … --by … -- program args`), driving it
/// as `--by` says until it ends.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let (named, argv) = flags(args)?;
    let sock = named.get("sock").cloned().ok_or("airband hold needs --sock")?;
    let (program, rest) = argv.split_first().ok_or("airband hold needs a program after --")?;
    // which of the two drivers: the tower always says, and nothing is assumed when it does not
    let by = named.get("by").map(String::as_str).ok_or("airband hold needs --by")?;
    // a terminal closing, or the tower that started this going away, must not take the agent with it
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let state = Arc::new(Mutex::new(State::Starting));
    let (tx, rx) = mpsc::channel::<Msg>();
    listen(&sock, state.clone(), tx.clone())?;
    let cwd = named.get("cwd").cloned().unwrap_or_else(|| std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default());
    let mut command = Command::new(program);
    command.args(rest).current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // A program whose holder reports for it stays quiet itself, hooks and all: two reports of the same thing
    // would be read to him twice.
    if by == "acp" {
        command.env("AIRBAND_INTERNAL", "1");
    }
    let errors = Arc::new(Mutex::new(String::new()));
    let outcome = match command.spawn() {
        Err(e) => Err(format!("{program} did not start: {e}")),
        Ok(mut child) => {
            pass_output(&mut child, tx, by == "acp");
            keep_errors(&mut child, errors.clone());
            let held = Held { id: named.get("id").cloned().unwrap_or_default(), harness: named.get("harness").cloned().unwrap_or_default(), cwd, rx, to_program: Arc::new(Mutex::new(child.stdin.take())), state: state.clone(), errors };
            let done = match by {
                "lines" => lines(&held, named.get("line").ok_or("airband hold --by lines needs --line")?, named.get("stop").map(String::as_str)),
                "acp" => crate::acp::run(&held, named.get("session").cloned(), named.get("mode").map(String::as_str), crate::acp::Asked { model: named.get("model").cloned(), effort: named.get("effort").cloned() }),
                other => Err(format!("this version of airband does not drive a program by \"{other}\"")),
            };
            held.close_input();
            end(&mut child);
            done
        }
    };
    if let Err(why) = &outcome {
        // it never became an agent: say why to whoever asks, for long enough that the tower has asked
        *state.lock().unwrap() = State::Failed { why: why.clone() };
        std::thread::sleep(Duration::from_secs(15));
    }
    let _ = std::fs::remove_file(&sock);
    end_group();
    outcome
}

/// The flags it was given, by name, and the program and its arguments after `--`.
fn flags(args: &[String]) -> Result<(std::collections::BTreeMap<&str, String>, Vec<String>), String> {
    let mut it = args.iter();
    let mut named: std::collections::BTreeMap<&str, String> = Default::default();
    let mut argv: Vec<String> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => argv = it.by_ref().cloned().collect(),
            flag @ ("--sock" | "--cwd" | "--id" | "--harness" | "--by" | "--line" | "--stop" | "--session" | "--mode" | "--model" | "--effort") => {
                named.insert(&flag[2..], it.next().cloned().ok_or_else(|| format!("airband hold: {flag} needs a value"))?);
            }
            other => return Err(format!("airband hold does not take \"{other}\"")),
        }
    }
    Ok((named, argv))
}

/// The socket the tower talks to it through, open to this user only: asked how it stands, it says; told
/// something for the agent, it passes it on.
fn listen(sock: &str, state: Arc<Mutex<State>>, tx: mpsc::Sender<Msg>) -> Result<(), String> {
    let _ = std::fs::remove_file(sock);
    let listener = UnixListener::bind(sock).map_err(|e| format!("cannot open {sock}: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600));
    }
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut said = String::new();
            if conn.read_to_string(&mut said).is_err() {
                continue;
            }
            for line in said.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
                if line.get("status").is_some() {
                    let _ = writeln!(conn, "{}", serde_json::to_string(&*state.lock().unwrap()).unwrap());
                } else if let Some(tell) = Tell::read(&line) {
                    let _ = tx.send(Msg::Tell(tell));
                }
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
                    let _ = tx.send(Msg::Out(String::from_utf8_lossy(&line).into_owned()));
                }
            }
            let _ = tx.send(Msg::Gone);
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
    let _ = child.kill();
    let _ = child.wait();
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
