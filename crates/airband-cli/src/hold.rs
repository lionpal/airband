//! `airband hold`: keeps one agent's program running, apart from the tower, and drives it.
//!
//! The tower starts one of these for each agent it runs itself, and then only ever talks to it through a
//! socket. So the agent does not end when the tower does: a tower that is updated or restarted finds the socket
//! again and carries on, and whatever the agent was in the middle of goes on undisturbed.
//!
//!   airband hold --sock <path> --cwd <folder> --id <agent id> --harness <tool> --by <lines|acp> […] -- <program> [arguments...]
//!
//! What the tower says to it is one JSON line a connection, the same whatever the program is ([`Tell`]): his
//! words for the agent, stop what you are doing, finish, and how are you. How that reaches the program is the
//! driver's business (`--by`, the two kinds a launcher's row can name, `airband_core::tools::Held`):
//!
//! - `lines`: the program reads one message a line on its standard input (`--line` is what a line looks like,
//!   `--stop` the line that stops it) and reports to the tower by itself.
//! - `acp`: the program speaks the Agent Client Protocol; `acp.rs` speaks it back and reports for the agent.
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

/// What closes the program's input in a holder started by a version before this one, which took raw bytes.
pub const END_BEFORE: &[u8] = b"\0end\n";

/// What the tower says to a holder.
pub enum Tell {
    /// His words, as the agent's next turn.
    Say(String),
    /// Leave off what you are doing.
    Stop,
    /// Finish: the program's input is closed, and it ends.
    End,
}

impl Tell {
    pub fn line(&self) -> String {
        match self {
            Tell::Say(text) => json!({ "say": text }),
            Tell::Stop => json!({ "stop": true }),
            Tell::End => json!({ "end": true }),
        }
        .to_string()
    }
    fn read(line: &Value) -> Option<Tell> {
        match (line["say"].as_str(), line.get("stop"), line.get("end")) {
            (Some(text), _, _) => Some(Tell::Say(text.to_string())),
            (None, Some(_), _) => Some(Tell::Stop),
            (None, None, Some(_)) => Some(Tell::End),
            _ => None,
        }
    }
}
/// The line that asks a holder where it has got to. The answer is a [`State`].
pub const STATUS: &str = r#"{"status":true}"#;

/// Where a holder has got to with its program.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum State {
    Starting,
    /// The agent is there and takes his words. `session` is what the agent's own tool calls its session, when
    /// that is not the id the tower gave it: what carries it on later.
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
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
    pub id: String,
    pub harness: String,
    pub cwd: String,
    pub rx: mpsc::Receiver<Msg>,
    pub to_program: Arc<Mutex<Option<ChildStdin>>>,
    pub state: Arc<Mutex<State>>,
    /// The last of what the program printed as an error.
    pub errors: Arc<Mutex<String>>,
}

impl Held {
    pub fn write(&self, line: &str) {
        if let Some(to) = self.to_program.lock().unwrap().as_mut() {
            let _ = writeln!(to, "{line}").and_then(|()| to.flush());
        }
    }
    pub fn close_input(&self) {
        *self.to_program.lock().unwrap() = None;
    }
    pub fn ready(&self, session: Option<String>) {
        *self.state.lock().unwrap() = State::Ready { session };
    }
    /// Why it failed, with what the program itself had to say.
    pub fn why(&self, what: &str) -> String {
        let said = self.errors.lock().unwrap().trim().to_string();
        if said.is_empty() { what.to_string() } else { format!("{what}: {said}") }
    }
}

/// The driver for a program that reads one message a line and reports by itself.
fn lines(held: &Held, line: &str, stop: Option<&str>) -> Result<(), String> {
    // a program that cannot run ends at once; one still there after a moment is taking input
    match held.rx.recv_timeout(Duration::from_millis(400)) {
        Ok(Msg::Gone) => return Err(held.why("it ended as soon as it started")),
        Ok(Msg::Tell(first)) => {
            held.ready(None);
            told(held, first, line, stop);
        }
        _ => held.ready(None),
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
        Tell::Say(text) => held.write(&line.replace("{text}", &Value::String(text).to_string())),
        Tell::Stop => {
            if let Some(stop) = stop {
                held.write(&stop.replace("{id}", &crate::config::random_hex(8)));
            }
        }
        Tell::End => held.close_input(),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let mut named: std::collections::BTreeMap<&str, String> = Default::default();
    let mut argv: Vec<String> = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => argv = it.by_ref().cloned().collect(),
            flag @ ("--sock" | "--cwd" | "--id" | "--harness" | "--by" | "--line" | "--stop" | "--session") => {
                named.insert(&flag[2..], it.next().cloned().ok_or_else(|| format!("airband hold: {flag} needs a value"))?);
            }
            other => return Err(format!("airband hold does not take \"{other}\"")),
        }
    }
    let sock = named.get("sock").cloned().ok_or("airband hold needs --sock")?;
    let (program, rest) = argv.split_first().ok_or("airband hold needs a program after --")?;
    let by = named.get("by").map(String::as_str).unwrap_or("lines");
    // a terminal closing, or the tower that started this going away, must not take the agent with it
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).map_err(|e| format!("cannot open {sock}: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));
    }
    let state = Arc::new(Mutex::new(State::Starting));
    let (tx, rx) = mpsc::channel::<Msg>();
    {
        let (state, tx) = (state.clone(), tx.clone());
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
    }
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
            if let Some(out) = child.stdout.take() {
                let (tx, wanted) = (tx.clone(), by == "acp");
                std::thread::spawn(move || {
                    for line in std::io::BufReader::new(out).split(b'\n').map_while(Result::ok) {
                        if wanted {
                            let _ = tx.send(Msg::Out(String::from_utf8_lossy(&line).into_owned()));
                        }
                    }
                    let _ = tx.send(Msg::Gone);
                });
            }
            if let Some(err) = child.stderr.take() {
                let errors = errors.clone();
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
            let held = Held { id: named.get("id").cloned().unwrap_or_default(), harness: named.get("harness").cloned().unwrap_or_default(), cwd, rx, to_program: Arc::new(Mutex::new(child.stdin.take())), state: state.clone(), errors };
            let done = match by {
                "lines" => lines(&held, named.get("line").ok_or("airband hold --by lines needs --line")?, named.get("stop").map(String::as_str)),
                "acp" => crate::acp::run(&held, named.get("session").cloned()),
                other => Err(format!("this version of airband does not drive a program by \"{other}\"")),
            };
            held.close_input();
            // asked to finish, it is given a moment to; then it is ended
            for _ in 0..30 {
                if child.try_wait().is_ok_and(|ended| ended.is_some()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
            done
        }
    };
    if let Err(why) = &outcome {
        // it never became an agent: say why to whoever asks, for long enough that the tower has asked
        *state.lock().unwrap() = State::Failed { why: why.clone() };
        std::thread::sleep(Duration::from_secs(15));
    }
    let _ = std::fs::remove_file(&sock);
    outcome
}
