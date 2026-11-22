//! Adapters: each turns one harness's own notifications into `airband.agent/1` events and posts them to the tower
//! on this machine. An adapter never holds its harness up: it gives up on the tower after a second and a half,
//! and keeps what the tower did not answer for when it is back (`tower_client`).
//!
//! To put another harness on the radio:
//! - with no code, call `airband report` from whatever the harness can run (see `report.rs`);
//! - or add a module here that implements [`Harness`] and a line in [`HARNESSES`]. A tool says only what it
//!   has: one that reports through hooks says which (`hooks.rs` puts them in its file), one that is a server
//!   to watch says how (`watch`), and either says how his words get into a session of its own (`deliver`),
//!   which of its sessions are open now (`open`) and which folders it has worked in (`worked`).
//!   Its row in `catalog/tools/` says what it is called and how the tower starts one. `claude_code.rs` is the
//!   model for hooks, `opencode.rs` for a server.
pub(crate) mod claude_accounts;
pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod hooks;
pub(crate) mod opencode;
pub(crate) mod report;
mod accounts;
mod tower_client;
pub(crate) use accounts::*;
pub(crate) use tower_client::*;

use crate::config;
use airband_core::agent::{Agent, AgentEvent, PermissionAsk, PermissionState};
use airband_core::radio::{Account, Accounts};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

/// One coding tool the tower can hear from, by the id its row in the catalogue has. Everything but the name
/// has a default that means "this tool does not have that".
pub(crate) trait Harness: Sync {
    /// What `airband hook <name>` is called with, and what `Agent::harness` says.
    fn name(&self) -> &'static str;
    /// A notification of the harness's, as JSON, to the events it means (none, when it means nothing to the tower).
    fn events(&self, _notification: &Value) -> Vec<AgentEvent> {
        Vec::new()
    }
    /// For a tool that reports through hooks: its hooks file and the events wanted in it. Nothing when the tool
    /// is not on this computer, or is wired up some other way.
    fn hooks(&self) -> Option<hooks::File> {
        None
    }
    /// What to tell him once its hooks are in place; `changed` when the file was written.
    fn installed(&self, file: &hooks::File, _changed: bool) -> String {
        format!("{} now reports to airband (set in {}).", called(self.name()).0, file.path.display())
    }
    /// For a harness that can let something else answer its permission prompts: its notification as the request
    /// to put to him, or nothing when the notification is not one.
    fn permission(&self, _notification: &Value) -> Option<PermissionAsk> {
        None
    }
    /// His answer to that request, as the harness wants it printed on standard output.
    fn permission_answer(&self, ask: &PermissionAsk, allow: bool, reason: &str) -> String {
        hooks::decision(allow, reason, ask.question.is_some())
    }
    /// For a harness whose sessions can be moved between logins while they run.
    fn accounts(&self) -> Option<&'static AccountBook> {
        None
    }
    /// A look at one of its agents, when the harness keeps this somewhere the tower can read cheaply. Asked
    /// every couple of seconds while a radio is looking, so it must be quick.
    fn glance(&self, _agent: &Agent) -> Glance {
        Glance::default()
    }
    /// The files and commands of an agent's latest steps, newest first, as the harness wrote them. For working
    /// out which project it is working on; asked now and then, not on every look.
    fn touched(&self, _agent: &Agent) -> Vec<String> {
        Vec::new()
    }
    /// What he has said to an agent at its own keyboard since `from`, a place in the harness's record that an
    /// earlier call returned (0 the first time). Returns his words, oldest first, and the place to ask from next
    /// time. What he typed himself comes back with `his` set; what the tower passed on comes back without, when
    /// the agent has taken it in.
    fn told(&self, _agent: &Agent, from: u64) -> (Vec<Told>, u64) {
        (Vec::new(), from)
    }
    /// The steps an agent has taken since a moment (milliseconds since 1970), oldest first, each as the few
    /// words the harness has for it: what it set out to do, not how it went.
    fn steps(&self, _agent: &Agent, _since: u64) -> Vec<String> {
        Vec::new()
    }
    /// For a tool with no hooks that the tower watches instead: watch it for as long as the tower runs,
    /// telling the tower what is seen. Run on a thread of its own; returns at once for a tool that is not
    /// watched.
    fn watch(&self, _tower: Tower) {}
    /// Put his words into a session of this tool's that is already open, as a turn of his own
    /// (`Deliver::Harness`).
    fn deliver(&self, _session: &str, _words: &str) -> Result<(), String> {
        Err(format!("{} sessions cannot be reached this way.", self.name()))
    }
    /// Put one of this tool's sessions away, where the tool has such a thing: it is then no longer one of the
    /// open ones (`open`), and what it did is kept. Nothing for a tool that has no way to.
    fn put_away(&self, _session: &str) -> Option<Result<(), String>> {
        None
    }
    /// Whether this tool has a way to put a session away.
    fn puts_away(&self) -> bool {
        false
    }
    /// The sessions of this tool's that are open on this computer right now, by the tool's own record of
    /// them, each as an agent the tower can put on the radio and reach. Nothing when the tool cannot say
    /// (it is not on this computer, it is not running, it keeps no such record): then the tower learns of
    /// its sessions only as they report. Asked every few seconds, so it must be quick.
    fn open(&self) -> Option<Vec<Open>> {
        None
    }
    /// The folders this tool has had sessions in, by its own record, each with when the last one was.
    fn worked(&self) -> Vec<Worked> {
        Vec::new()
    }
}

/// A session its tool says is open: who it is, where, and how his words reach it.
#[derive(Debug, Clone)]
pub(crate) struct Open {
    pub(crate) agent: Agent,
    /// Its tool says it is doing nothing right now. One the tower first learns of this way starts out
    /// waiting for him, not working; what it does after that comes with its own reports.
    pub(crate) idle: bool,
}

/// A folder a tool has had a session in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Worked {
    pub(crate) folder: String,
    /// When its latest session there was last written to, in milliseconds since 1970.
    pub(crate) at: u64,
}

/// Asks the tower which sessions of a harness it has that their tool has listed.
pub(crate) type Listed = Box<dyn Fn(&str) -> Vec<String> + Send>;

/// What a watcher is given of the tower it runs in.
pub(crate) struct Tower {
    /// Hand the tower an event. False when it was not taken: it should be offered again.
    pub(crate) event: Box<dyn Fn(AgentEvent) -> bool + Send>,
    /// Is this session of the tool's one the tower runs itself? Those are reported by their own driver.
    pub(crate) holds: Box<dyn Fn(&str) -> bool + Send>,
    /// A session its tool lists as open. The tower puts it on the radio if it is not there, and marks it as
    /// one its tool lists. False when it was not taken: it should be offered again.
    pub(crate) found: Box<dyn Fn(Agent, bool) -> bool + Send>,
    /// The sessions of a harness on the tower's roster that its tool has listed as open, now or before the
    /// tower last stopped. One of these the tool no longer lists has gone.
    pub(crate) listed: Listed,
    /// Has this session reported by its tool's hooks? One that has asks for leave by its hook.
    pub(crate) hooked: Box<dyn Fn(&str) -> bool + Send>,
}

/// Something he said to an agent directly, at its keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Told {
    /// The harness's own id for it: the same words are never taken twice.
    pub(crate) id: String,
    /// When he said it, in milliseconds since 1970.
    pub(crate) at: u64,
    /// His words, as he typed them. Empty when he gave it only a picture.
    pub(crate) text: String,
    /// Pictures he gave it with those words, as the files' own bytes. At most four.
    pub(crate) images: Vec<Vec<u8>>,
    /// False for something the tower passed on to the agent, seen here at the moment the agent took it in:
    /// that is how the tower knows a message of his has been read.
    pub(crate) his: bool,
}

/// What a look at an agent can tell. Anything the harness cannot say is left out.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Glance {
    /// What it is doing right now, in a few words.
    pub(crate) doing: Option<String>,
    /// The model it runs on, as a person would say it: "Opus 4.7".
    pub(crate) model: Option<String>,
    /// How hard it is set to think: "high".
    pub(crate) effort: Option<String>,
}

/// How one harness's accounts are listed, switched and kept. Every function answers in words for him.
pub(crate) struct AccountBook {
    /// Every account with how much of it is used, and anything to say about the list as a whole.
    pub(crate) list: fn() -> (Vec<Account>, Option<String>),
    /// Make this account the one in use. The swap is done on return; the thread finishes the tidying.
    pub(crate) switch: fn(&str) -> Result<std::thread::JoinHandle<()>, String>,
    /// Keep the login in use now. Returns what it is called.
    pub(crate) save: fn() -> Result<String, String>,
    /// Sign in to another account, at the computer, and keep it.
    pub(crate) add: fn() -> Result<String, String>,
    pub(crate) forget: fn(&str) -> Result<String, String>,
}

/// What a coding tool's row tells the code that speaks to it (`Tool::with`): the program's name, an address.
/// Always from the rows this build came with. Empty when the row does not say.
pub(crate) fn row_says(tool: &str, what: &str) -> String {
    tool_row(tool).map(|t| t.told(what).to_string()).unwrap_or_default()
}

/// A coding tool's row as built into this program: what the code that speaks to it does is never a fetched
/// copy's to change. The one lookup of a tool's row by its id for that; only names shown to him come from the
/// copy in use (`called`).
pub(crate) fn tool_row(tool: &str) -> Option<&'static airband_core::catalog::Tool> {
    airband_core::catalog::built().tools.iter().find(|t| t.id == tool)
}

/// What a coding tool and its maker are called, as its row has them.
pub(crate) fn called(tool: &str) -> (String, String) {
    airband_core::catalog::current().tools.iter().find(|t| t.id == tool).map(|t| (t.name.clone(), t.maker.clone())).unwrap_or_default()
}

/// Every coding tool with an adapter, by the id its row has. A new one is a module and a line here.
static HARNESSES: &[&dyn Harness] = &[&claude_code::ClaudeCode, &codex::Codex, &opencode::OpenCode];

fn harness(name: &str) -> Option<&'static dyn Harness> {
    HARNESSES.iter().copied().find(|h| h.name() == name)
}

/// `airband install`: set up every harness that can be. One line to say for each.
pub(crate) fn install(exe: &std::path::Path) -> Result<Vec<String>, String> {
    HARNESSES.iter().filter_map(|h| h.hooks().map(|file| (h, file))).map(|(h, file)| file.install(exe).map(|changed| h.installed(&file, changed))).collect()
}
pub(crate) fn uninstall() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    HARNESSES.iter().filter_map(|h| h.hooks()).try_for_each(|file| file.uninstall(&exe))
}

/// What this agent is doing right now and what it runs on, as far as its harness's adapter can tell.
pub(crate) fn glance(agent: &Agent) -> Glance {
    harness(&agent.harness).map(|h| h.glance(agent)).unwrap_or_default()
}

/// The files and commands of this agent's latest steps, if its harness's adapter can tell.
pub(crate) fn touched(agent: &Agent) -> Vec<String> {
    harness(&agent.harness).map(|h| h.touched(agent)).unwrap_or_default()
}

/// The steps this agent has taken since `since`, if its harness's adapter can tell.
pub(crate) fn steps(agent: &Agent, since: u64) -> Vec<String> {
    harness(&agent.harness).map(|h| h.steps(agent, since)).unwrap_or_default()
}

/// What he has typed to this agent at its keyboard since `from`, if its harness's adapter can tell.
pub(crate) fn told(agent: &Agent, from: u64) -> (Vec<Told>, u64) {
    harness(&agent.harness).map(|h| h.told(agent, from)).unwrap_or((Vec::new(), from))
}

/// Start watching every tool that is watched and not hooked, each on a thread of its own. `tower` makes
/// what each is given.
pub(crate) fn watch_all(tower: impl Fn() -> Tower) {
    for h in HARNESSES {
        let tower = tower();
        std::thread::spawn(move || h.watch(tower));
    }
}

/// Keep the tower's roster in step with what every tool says is open, for as long as the tower runs: a
/// session its tool lists that the tower does not have is put on the radio, and one the tool listed before
/// and lists no longer is signed off. One way for every tool; a tool that cannot say what is open is left
/// to its own reports. Blocks: run it on a thread of its own.
pub(crate) fn find_all(tower: Tower) {
    // what was last handed to the tower of each session, so that only news is handed over
    let mut told: std::collections::HashMap<String, Value> = Default::default();
    loop {
        for h in HARNESSES.iter().copied() {
            look(h, &tower, &mut told);
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// One look at one tool's open sessions.
fn look(h: &dyn Harness, tower: &Tower, told: &mut std::collections::HashMap<String, Value>) {
    let Some(open) = h.open() else { return };
    // the ones the tower runs itself are reported by their own driver
    let open: Vec<Open> = open.into_iter().filter(|o| !(tower.holds)(&o.agent.id)).collect();
    let listed = (tower.listed)(h.name());
    for o in &open {
        let now = serde_json::to_value(&o.agent).unwrap_or_default();
        if (!listed.contains(&o.agent.id) || told.get(&o.agent.id) != Some(&now)) && (tower.found)(o.agent.clone(), o.idle) {
            told.insert(o.agent.id.clone(), now);
        }
    }
    for id in listed.iter().filter(|id| !open.iter().any(|o| o.agent.id == **id)) {
        let gone = Agent { id: id.clone(), harness: h.name().into(), cwd: None, name: None, title: None, pid: None, deliver: None, meta: Default::default() };
        if (tower.event)(AgentEvent { schema: airband_core::agent::SCHEMA.into(), agent: gone, event: airband_core::agent::Event::Signoff, via: None }) {
            told.remove(id);
        }
    }
}

/// Every folder a coding tool here has had a session in, with the tools that did and when the last one
/// was, newest first. Folders that are gone are left out. Reads each tool's records: not for a hot path.
pub(crate) fn worked_all() -> Vec<(String, Vec<String>, u64)> {
    let mut all: Vec<(String, Vec<String>, u64)> = Vec::new();
    for h in HARNESSES {
        for w in h.worked() {
            match all.iter_mut().find(|(folder, _, _)| *folder == w.folder) {
                Some((_, tools, at)) => {
                    if !tools.iter().any(|t| t == h.name()) {
                        tools.push(h.name().to_string());
                    }
                    *at = (*at).max(w.at);
                }
                None => all.push((w.folder, vec![h.name().to_string()], w.at)),
            }
        }
    }
    // Not the ones that are gone, nor the ones in a temporary folder: nothing lasting is there. Which folders
    // are temporary is the system's to say: the one it gives this user (`temp_dir`), and `/tmp`, which POSIX
    // sets aside for that. Each is compared as the disk has it, links followed.
    let passing: Vec<std::path::PathBuf> = [std::env::temp_dir(), "/tmp".into()].into_iter().filter_map(|temp| temp.canonicalize().ok()).collect();
    all.retain(|(folder, _, _)| std::path::Path::new(folder).canonicalize().is_ok_and(|real| real.is_dir() && !passing.iter().any(|temp| real.starts_with(temp))));
    all.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    all
}

/// The newest record of each folder: many sessions in one folder are one folder.
pub(crate) fn newest(found: impl IntoIterator<Item = Worked>) -> Vec<Worked> {
    let mut by: std::collections::HashMap<String, u64> = Default::default();
    for w in found {
        let at = by.entry(w.folder).or_insert(0);
        *at = (*at).max(w.at);
    }
    by.into_iter().map(|(folder, at)| Worked { folder, at }).collect()
}

/// When a file was last written, in milliseconds since 1970.
pub(crate) fn written(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// His words into a session of a tool's own, by that tool's way in. Blocks.
pub(crate) fn deliver(name: &str, session: &str, words: &str) -> Result<(), String> {
    harness(name).ok_or_else(|| format!("there is no adapter for \"{name}\""))?.deliver(session, words)
}

/// Put a session of a tool's own away, by that tool's way. Blocks.
pub(crate) fn put_away(name: &str, session: &str) -> Result<(), String> {
    harness(name).and_then(|h| h.put_away(session)).unwrap_or_else(|| Err("Only an agent airband runs can be ended from here.".into()))
}
/// Whether a tool's sessions can be put away from here.
pub(crate) fn puts_away(name: &str) -> bool {
    harness(name).is_some_and(|h| h.puts_away())
}

pub(crate) fn names() -> Vec<&'static str> {
    HARNESSES.iter().map(|h| h.name()).collect()
}

pub(crate) fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// The folder a coding tool was told to keep its own files in instead of its usual one, by the variable its row
/// names (`home_env`).
pub(crate) fn moved_home(tool: &str) -> Option<std::path::PathBuf> {
    env(&row_says(tool, "home_env")).map(std::path::PathBuf::from)
}
/// Where a coding tool keeps its own files: the moved folder, or its usual one under his home (`home`), as its
/// row has them. The one rule for every tool.
pub(crate) fn tool_home(tool: &str) -> std::path::PathBuf {
    home_given(tool, moved_home(tool))
}
fn home_given(tool: &str, moved: Option<std::path::PathBuf>) -> std::path::PathBuf {
    moved.unwrap_or_else(|| crate::config::home().join(row_says(tool, "home")))
}

/// `airband hook <harness>`: read the harness's notification from standard input, translate it, post it.
pub(crate) fn hook(name: Option<&str>) -> Result<(), String> {
    // said aloud, because this is a mistake in someone's setup, not a failure to hide from a running agent
    let unknown = || format!("there is an adapter for: {}. Any other harness can use `airband report`.", names().join(", "));
    let name = name.ok_or_else(|| format!("usage: airband hook <harness>; {}", unknown()))?;
    let adapter = harness(name).ok_or_else(|| format!("no adapter for \"{name}\"; {}", unknown()))?;
    // the tower's own model calls, and an agent whose holder reports for it, must not report to the tower
    if env("AIRBAND_INTERNAL").is_some() {
        return Ok(());
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let notification: Value = serde_json::from_str(&input).unwrap_or_default();
    // the harness is about to ask for permission at the keyboard: ask him on the radio first
    if let Some(request) = adapter.permission(&notification) {
        if let Some((allow, reason)) = ask_him(&request) {
            println!("{}", adapter.permission_answer(&request, allow, &reason));
        }
        // no answer in time, or nobody there: print nothing, and the harness asks in its own way
        return Ok(());
    }
    for mut event in adapter.events(&notification) {
        // An agent the tower runs whose program was not started under the id the tower gave it is known by
        // that id all the same, whatever its harness calls the session: its holder says so (`hold.rs`). True
        // of any harness, so it is said here and not in each adapter.
        if let Some(id) = env("AIRBAND_HELD_ID") {
            event.agent.id = id;
            event.agent.deliver = Some(airband_core::agent::Deliver::Held);
        }
        post(&event);
    }
    Ok(())
}

#[cfg(test)]
mod finding {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A tool that lists whatever the test says is open.
    struct Lists(Mutex<Option<Vec<&'static str>>>);
    impl Harness for Lists {
        fn name(&self) -> &'static str {
            "lists"
        }
        fn open(&self) -> Option<Vec<Open>> {
            let open = self.0.lock().unwrap().clone()?;
            Some(open.into_iter().map(|id| Open { agent: Agent { id: id.into(), harness: "lists".into(), cwd: Some("/x/shop".into()), name: None, title: None, pid: None, deliver: None, meta: Default::default() }, idle: true }).collect())
        }
    }

    #[test]
    fn what_a_tool_lists_as_open_goes_on_the_radio_and_what_it_lists_no_longer_is_signed_off() {
        // the tower, as far as a look can tell: what it was handed, and which sessions it has marked as listed
        type Seen = Arc<Mutex<Vec<String>>>;
        let (found, events, listed): (Seen, Seen, Seen) = Default::default();
        let tower = {
            let (found, events, listed, marks) = (found.clone(), events.clone(), listed.clone(), listed.clone());
            Tower {
                event: Box::new(move |e| {
                    events.lock().unwrap().push(format!("{:?} {}", e.event, e.agent.id));
                    true
                }),
                holds: Box::new(|session| session == "held"),
                hooked: Box::new(|_| false),
                found: Box::new(move |agent, _idle| {
                    found.lock().unwrap().push(agent.id.clone());
                    marks.lock().unwrap().push(agent.id);
                    true
                }),
                listed: Box::new(move |_| listed.lock().unwrap().clone()),
            }
        };
        let tool = Lists(Mutex::new(Some(vec!["a", "b", "held"])));
        let mut told = Default::default();
        look(&tool, &tower, &mut told);
        // the two that are his, not the one the tower runs itself
        assert_eq!(*found.lock().unwrap(), ["a", "b"]);
        // nothing new: nothing is handed over again
        look(&tool, &tower, &mut told);
        assert_eq!(found.lock().unwrap().len(), 2);
        assert!(events.lock().unwrap().is_empty());
        // one the tower has from before it last stopped, which the tool no longer lists, and one that closed
        listed.lock().unwrap().push("from-before".into());
        *tool.0.lock().unwrap() = Some(vec!["a"]);
        look(&tool, &tower, &mut told);
        let mut gone = events.lock().unwrap().clone();
        gone.sort();
        assert_eq!(gone, ["Signoff b", "Signoff from-before"]);
        // a tool that cannot say what is open just now has nothing signed off on its silence
        events.lock().unwrap().clear();
        *tool.0.lock().unwrap() = None;
        look(&tool, &tower, &mut told);
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn many_sessions_in_one_folder_are_one_folder_by_its_latest() {
        let mut once = newest([Worked { folder: "/x/shop".into(), at: 5 }, Worked { folder: "/x/garden".into(), at: 7 }, Worked { folder: "/x/shop".into(), at: 9 }]);
        once.sort_by(|a, b| a.folder.cmp(&b.folder));
        assert_eq!(once, [Worked { folder: "/x/garden".into(), at: 7 }, Worked { folder: "/x/shop".into(), at: 9 }]);
    }
}

#[cfg(test)]
mod rows {
    use super::*;

    #[test]
    fn every_coding_tool_in_the_catalogue_has_its_adapter_and_every_adapter_its_row() {
        let rows: Vec<&str> = airband_core::catalog::built().tools.iter().map(|t| t.id.as_str()).collect();
        let mut adapters: Vec<&str> = HARNESSES.iter().map(|h| h.name()).collect();
        adapters.sort();
        let mut sorted = rows.clone();
        sorted.sort();
        assert_eq!(adapters, sorted);
        // each is called something, by its row and not by this code
        assert!(rows.iter().all(|id| !called(id).0.is_empty()));
        // and a row that says nothing of a thing says nothing: no name is made up for it
        assert_eq!(row_says(rows[0], "no such thing"), "");
    }
}
