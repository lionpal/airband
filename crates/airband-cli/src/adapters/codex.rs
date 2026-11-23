//! Codex, in a terminal he opened himself or reached through its own server.
//!
//! Codex has hooks in the same shape as Claude Code's: a command run on an event, the event as JSON on its
//! standard input, and for a permission request the decision as JSON on its standard output. So this adapter
//! is short. Two things differ, and both in Codex's favour:
//!
//! - His words reach a session as a turn of his own. `codex queue` puts a message into a session that is
//!   already open, and the session takes it as if he had typed it. Nothing has to be taken over first.
//! - Its model comes with every event, so there is no transcript to read for it.
//!
//! - Codex runs one server on the computer that its sessions go through (a socket file in its folder). It
//!   says which sessions are open now, so one that was open before the tower started is found with nothing
//!   typed in it, and `codex queue` reaches it.
//! - What a session says comes two ways. Its hooks say it at once, but Codex reads those only when a session
//!   starts and only once he has trusted them. So the tower also watches the server, which says what every
//!   open session's newest turn came to, hooks or none. Both name the turn by Codex's own id for it, and the
//!   tower takes what a turn said once.
//!
//! One thing he has to do himself, once: Codex runs a hook only after he has looked at it and trusted it
//! (`/hooks` inside Codex), and again whenever it changes. `airband install` writes the hooks and says so.
#![expect(clippy::disallowed_macros, reason = "speaks the tool's own app-server protocol")]
use super::env;
use super::hooks::{On, on, text};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk};
use serde_json::Value;
use std::path::PathBuf;

const NAME: &str = "codex";

fn agent_of(hook: &Value) -> Option<Agent> {
    let id = text(hook, "session_id")?;
    let mut meta = serde_json::Map::new();
    if let Some(model) = text(hook, "model") {
        meta.insert("model".into(), Value::String(model));
    }
    Some(Agent {
        // What he says is queued for the session with the tool's own command (`deliver` below). The session takes
        // it as a turn of his. One the tower holds is reached through its holder (`super::as_held`).
        deliver: Some(Deliver::Harness { name: NAME.into(), session: id.clone() }),
        id,
        harness: NAME.into(),
        cwd: text(hook, "cwd"),
        name: env("AIRBAND_NAME"),
        title: None,
        pid: None,
        meta,
    })
}

pub(crate) fn events(hook: &Value) -> Vec<AgentEvent> {
    let events = super::hooks::session_events(hook).unwrap_or_else(|| match hook["hook_event_name"].as_str() {
        Some(at @ ("SubagentStart" | "SubagentStop")) => vec![Event::Subagent { done: at == "SubagentStop", what: text(hook, "agent_type").unwrap_or_default() }],
        _ => Vec::new(),
    });
    super::hooks::from_agent(agent_of(hook), events)
}

/// What it runs on, which every event of its says. What it is doing is not known without reading its record.
pub(crate) fn glance(agent: &Agent) -> super::Glance {
    super::Glance { doing: None, model: agent.meta.get("model").and_then(Value::as_str).map(String::from), effort: None }
}

/// A request for leave, as he is asked it: whole, never cut, because what he reads is what will run.
pub(crate) fn permission(hook: &Value) -> Option<PermissionAsk> {
    let (tool, input) = super::hooks::permission_hook(hook)?;
    // the row's words for one of Codex's own tools; any other says what it is doing itself
    let what = super::hooks::asking(NAME, tool, input, text(input, "description"));
    // one too long to put in front of him whole is not asked on the phone: Codex asks at its keyboard
    super::hooks::leave(agent_of(hook)?, tool, what, input, super::hooks::AT_KEYBOARD, None)
}

/// His words into that session as a turn of his own, by Codex's own queue.
fn deliver(session: &str, words: &str) -> Result<(), String> {
    let program = super::row_says(NAME, "program");
    let argv = super::tool_row(NAME).map(|t| t.command("deliver", &[("program", &program), ("session", session), ("words", words)])).unwrap_or_default();
    let (program, args) = argv.split_first().ok_or("its row gives no way to hand it words")?;
    // what it writes on failing is its own, and can hold his words: never read to him or logged
    let done = std::process::Command::new(program).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map_err(|e| format!("{} could not be started: {e}", super::called(NAME).0))?;
    if done.success() { Ok(()) } else { Err(didnt_take(&super::called(NAME).0, program)) }
}

/// What he is told when the tool's own command would not take his words: that it failed, and where to look.
fn didnt_take(tool: &str, program: &str) -> String {
    format!("{tool} did not take it: {program} stopped with an error. Running it in a terminal shows why.")
}

// ---- what Codex itself says is open, and where it has worked ----

/// A conversation with Codex's own server on this computer: one request and its answer at a time.
#[cfg(unix)]
struct Server {
    socket: tungstenite::WebSocket<std::os::unix::net::UnixStream>,
    asked: u64,
}

#[cfg(unix)]
impl Server {
    /// Nothing when the server is not running: it is Codex that starts it, never the tower.
    fn connect() -> Option<Server> {
        let stream = std::os::unix::net::UnixStream::connect(control_socket()).ok()?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok()?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(3))).ok()?;
        let (socket, _) = tungstenite::client("ws://localhost/", stream).ok()?;
        let mut server = Server { socket, asked: 0 };
        server.ask("initialize", serde_json::json!({ "clientInfo": { "name": "airband", "title": "airband", "version": env!("CARGO_PKG_VERSION") } }))?;
        server.socket.send(tungstenite::Message::text(serde_json::json!({ "jsonrpc": "2.0", "method": "initialized" }).to_string())).ok()?;
        Some(server)
    }
    /// The answer to one request. Nothing when the server refused it or did not answer in time.
    fn ask(&mut self, method: &str, params: Value) -> Option<Value> {
        self.asked += 1;
        self.socket.send(tungstenite::Message::text(serde_json::json!({ "jsonrpc": "2.0", "id": self.asked, "method": method, "params": params }).to_string())).ok()?;
        // what it says meanwhile of its own accord is passed over; it cannot go on for ever
        for _ in 0..400 {
            let tungstenite::Message::Text(text) = self.socket.read().ok()? else { continue };
            let said: Value = serde_json::from_str(text.as_str()).ok()?;
            if said["id"].as_u64() == Some(self.asked) {
                return said.get("result").cloned();
            }
        }
        None
    }
}

/// Every thread Codex's server has loaded, each as the server describes it. Nothing when any of them could
/// not be read just now: one that cannot be read is not one that has gone.
#[cfg(unix)]
fn threads(server: &mut Server) -> Option<Vec<Value>> {
    let (mut threads, mut cursor) = (Vec::new(), Value::Null);
    for _ in 0..20 {
        let page = server.ask("thread/loaded/list", serde_json::json!({ "cursor": cursor }))?;
        for id in page["data"].as_array()?.iter().filter_map(Value::as_str) {
            threads.push(server.ask("thread/read", serde_json::json!({ "threadId": id }))?["thread"].clone());
        }
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Some(threads)
}

/// The sessions open now: the threads Codex's server has loaded, each read for where it is.
#[cfg(unix)]
fn open() -> Option<Vec<super::Open>> {
    Some(threads(&mut Server::connect()?)?.iter().filter_map(open_of).collect())
}
#[cfg(not(unix))]
fn open() -> Option<Vec<super::Open>> {
    None
}

/// One of Codex's threads, as it describes it, as a session on the radio. A subagent's thread is its
/// parent's business, and one that keeps no record is a one-off run: neither is his to talk to. The name
/// a thread has is one Codex made up from its first words, so it is not taken for a name he gave.
fn open_of(thread: &Value) -> Option<super::Open> {
    if !thread["parentThreadId"].is_null() || thread["ephemeral"] == true {
        return None;
    }
    let id = text(thread, "id")?;
    let mut meta = serde_json::Map::new();
    if let Some(model) = text(thread, "model") {
        meta.insert("model".into(), Value::String(model));
    }
    Some(super::Open {
        agent: Agent { deliver: Some(Deliver::Harness { name: NAME.into(), session: id.clone() }), id, harness: NAME.into(), cwd: text(thread, "cwd"), name: None, title: None, pid: None, meta },
        idle: thread["status"]["type"] == "idle",
    })
}

/// What a thread's newest turn has come to, as the server gives the turn: it is being worked on, or it is
/// over and this is what came of it. What it said is the last thing Codex wrote in the turn, the same words
/// its hook is given, and carries the turn's id.
fn turn_event(turn: &Value) -> Option<Event> {
    let id = turn["id"].as_str()?;
    Some(match turn["status"].as_str()? {
        "inProgress" => Event::Working,
        "completed" => {
            let said = turn["items"].as_array().into_iter().flatten().rev().filter(|item| item["type"] == "agentMessage").find_map(|item| item["text"].as_str().map(str::trim).filter(|t| !t.is_empty()));
            match said {
                Some(text) => Event::Said { text: text.to_string(), kind: None, turn: Some(id.to_string()) },
                None => Event::Stopped { why: None },
            }
        }
        "failed" => Event::Stopped { why: turn["error"]["message"].as_str().map(String::from) },
        "interrupted" => Event::Stopped { why: None },
        _ => return None,
    })
}

/// Is this thread stopped at a request for leave, as the server describes it?
fn waits_for_leave(thread: &Value) -> bool {
    thread["status"]["activeFlags"].as_array().is_some_and(|flags| flags.iter().any(|f| f == "waitingOnApproval"))
}

/// The requests for approval Codex's server sends, by their method: to run a command (the one that can be put
/// to him from here), and to change files.
const APPROVALS: [&str; 2] = ["item/commandExecution/requestApproval", "item/fileChange/requestApproval"];

/// A request of Codex's server to run a command, as he is asked it: the command whole, where it runs, and the
/// reason Codex gave. Any other kind of request (a change to files, which the server describes only by a
/// reference) is not put to him from here: it stays at Codex's keyboard.
fn leave_of(agent: Agent, request: &Value) -> Option<PermissionAsk> {
    if request["method"] != APPROVALS[0] {
        return None;
    }
    let params = &request["params"];
    let command = params["command"].as_str().filter(|c| !c.trim().is_empty())?;
    let mut input = serde_json::Map::new();
    input.insert("command".into(), command.into());
    for key in ["cwd", "reason"] {
        if let Some(said) = params[key].as_str().filter(|s| !s.is_empty()) {
            input.insert(key.into(), said.into());
        }
    }
    super::hooks::leave(agent, "shell", super::hooks::to_run(NAME, command), &Value::Object(input), super::hooks::AT_KEYBOARD, None)
}

/// Put a session's pending request for leave to him and give Codex his answer. The server sends a request to
/// every client that follows the thread and takes the first answer, so the keyboard can still answer it
/// meanwhile: then the server says it is settled and the question is taken back from the phone. Nothing is
/// answered unless he answered. Runs for as long as he takes.
#[cfg(unix)]
fn put_to_him(agent: Agent, asking: std::sync::Arc<std::sync::Mutex<super::Asking>>) {
    use std::cell::{Cell, RefCell};
    let thread = agent.id.clone();
    let mut key = thread.clone();
    let outcome = (|| {
        let mut server = Server::connect()?;
        // following the thread is what makes the server send its pending request here too
        server.socket.send(tungstenite::Message::text(serde_json::json!({ "jsonrpc": "2.0", "id": 1_000_000, "method": "thread/resume", "params": { "threadId": thread, "excludeTurns": true } }).to_string())).ok()?;
        let mut request = None;
        for _ in 0..200 {
            let tungstenite::Message::Text(text) = server.socket.read().ok()? else { continue };
            let said: Value = serde_json::from_str(text.as_str()).ok()?;
            if said["id"] == 1_000_000 && said.get("error").is_some() {
                return None;
            }
            // one of this thread's requests that was not answered or left already
            if said["params"]["threadId"] == thread.as_str() && !said["id"].is_null() && said["method"].as_str().is_some_and(|m| APPROVALS.contains(&m)) && !asking.lock().unwrap().done(&request_key(&said, &thread)) {
                request = Some(said);
                break;
            }
        }
        let request = request?;
        key = request_key(&request, &thread);
        let Some(ask) = leave_of(agent.clone(), &request) else { return Some(super::Answer::Unanswered) };
        server.socket.get_ref().set_read_timeout(Some(std::time::Duration::from_millis(200))).ok();
        let (server, settled) = (RefCell::new(server), Cell::new(false));
        // read what the server has said since: it says so when the request was answered somewhere else
        let still = || {
            loop {
                match server.borrow_mut().socket.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        let said: Value = serde_json::from_str(text.as_str()).unwrap_or_default();
                        if said["method"] == "serverRequest/resolved" && said["params"]["requestId"] == request["id"] {
                            settled.set(true);
                        }
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                    Err(_) => {
                        settled.set(true);
                        break;
                    }
                }
            }
            !settled.get()
        };
        let answer = super::ask_him_while(&ask, Some(&still));
        if let super::Answer::Said(allow, _) = &answer
            && !settled.get()
        {
            // his answer counts once the server has it; one that could not be sent is put to him again
            let mut server = server.borrow_mut();
            let reply = serde_json::json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "decision": if *allow { "accept" } else { "decline" } } }).to_string();
            server.socket.send(tungstenite::Message::text(reply)).ok()?;
            server.socket.flush().ok()?;
        }
        Some(answer)
    })();
    // nobody there to ask, the server could not be followed, or his answer could not be given: tried again in a
    // while; answered, here or at the keyboard, or left: that request is not asked again, and the next one the
    // session waits on is put to him at once
    asking.lock().unwrap().over(&thread, &key, outcome.as_ref());
}

/// What one request of a thread's is known by from one connection to the next: the item it asks about. The
/// server's own id for it is only this connection's. A request with no item is known by its thread.
fn request_key(request: &Value, thread: &str) -> String {
    request["params"]["itemId"].as_str().unwrap_or(thread).to_string()
}

/// Watch Codex's server for as long as the tower runs: what each open session's newest turn has come to.
/// Which sessions there are is not this watcher's to say (`open`, which every tool has). A session whose
/// hooks report says the same turns by them; the tower takes each once. Blocks: run it on a thread of its
/// own.
#[cfg(unix)]
fn watch(tower: super::Tower) {
    // each thread seen: its newest turn as far as the tower has been told, and whether that turn was over.
    // Something the tower did not take is not marked, and is told again on the next look.
    let mut heard: std::collections::HashMap<String, Option<(String, bool)>> = Default::default();
    let asking: std::sync::Arc<std::sync::Mutex<super::Asking>> = Default::default();
    let asking_for_threads = asking.clone();
    // the first look that reached the server: what it finds was said before the tower started
    let mut first = true;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(if control_socket().exists() { 2 } else { 20 }));
        let Some(mut server) = Server::connect() else { continue };
        let Some(threads) = threads(&mut server) else { continue };
        // not the ones the tower runs itself: their driver reports for them
        let found: Vec<(Agent, bool)> = threads.iter().filter_map(|t| Some((open_of(t)?.agent, waits_for_leave(t)))).filter(|(a, _)| !(tower.holds)(&a.id)).collect();
        // A request for leave of a session whose hooks do not report (they were not trusted, or it was open
        // before they were written) is put to him from here. One whose hooks report asks by its hook, and is
        // not asked a second time this way.
        {
            let mut asking = asking.lock().unwrap();
            asking.keep_only(|id| found.iter().any(|(a, waits)| a.id == id && *waits));
            for (agent, _) in found.iter().filter(|(a, waits)| *waits && !(tower.hooked)(&a.id)) {
                if !asking.take(&agent.id) {
                    continue;
                }
                let (agent, asking) = (agent.clone(), asking_for_threads.clone());
                std::thread::spawn(move || put_to_him(agent, asking));
            }
        }
        let here: Vec<Agent> = found.into_iter().map(|(a, _)| a).collect();
        for agent in &here {
            let Some(page) = server.ask("thread/turns/list", serde_json::json!({ "threadId": agent.id, "limit": 1, "itemsView": "full" })) else { continue };
            let turn = &page["data"][0];
            let now = turn["id"].as_str().zip(turn["status"].as_str()).map(|(id, status)| (id.to_string(), status != "inProgress"));
            let Some(was) = heard.get(&agent.id).cloned() else {
                // A turn that was already over when the thread was first seen has been said, or was said
                // before the tower started: nothing of it is said again. One under way is working.
                let told = match (first_seen(first, &now), turn_event(turn)) {
                    (FirstSeen::Working, _) => (tower.event)(AgentEvent::new(agent.clone(), Event::Working)).then(|| now.clone()),
                    (FirstSeen::Over, Some(event)) => (tower.event)(AgentEvent::new(agent.clone(), event)).then(|| now.clone()),
                    _ => Some(now.clone()),
                };
                if let Some(told) = told {
                    heard.insert(agent.id.clone(), told);
                }
                continue;
            };
            if was == now {
                continue;
            }
            let Some(event) = turn_event(turn) else { continue };
            if (tower.event)(AgentEvent::new(agent.clone(), event)) {
                heard.insert(agent.id.clone(), now);
            }
        }
        heard.retain(|id, _| here.iter().any(|a| a.id == *id));
        first = false;
    }
}

/// What is said of a thread the first time it is seen, from its newest turn: whether it is over.
#[derive(Debug, PartialEq, Eq)]
enum FirstSeen {
    /// Under way: it is working.
    Working,
    /// Over since the tower's first look (it began and ended between two looks): what it came to is said.
    Over,
    /// Nothing: there is no turn, or it was over on the first look, said before the tower started.
    Nothing,
}

fn first_seen(first: bool, newest: &Option<(String, bool)>) -> FirstSeen {
    match newest {
        Some((_, false)) => FirstSeen::Working,
        Some((_, true)) if !first => FirstSeen::Over,
        _ => FirstSeen::Nothing,
    }
}
#[cfg(not(unix))]
fn watch(_tower: super::Tower) {}

/// The folders Codex has had sessions in: each session's file starts with a line that says where it was.
/// The newest few hundred files are asked.
fn worked_in(codex: &std::path::Path) -> Vec<super::Worked> {
    // sessions/<year>/<month>/<day>/<one file per session>
    let inside = |dirs: Vec<PathBuf>| -> Vec<PathBuf> { dirs.iter().filter_map(|d| airband_io::files::entries(d).ok()).flatten().collect() };
    // newest first by when each was last written, read as a time, never by how its path sorts as text
    let mut files: Vec<(u64, PathBuf)> = inside(inside(inside(inside(vec![codex.join(super::row_says(NAME, "sessions"))])))).into_iter().filter(|p| p.extension().is_some_and(|e| e == "jsonl")).filter_map(|p| Some((super::written(&p)?, p))).collect();
    files.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let found = files.iter().take(300).filter_map(|(at, file)| {
        let first = airband_io::files::first_lines(file, 1).ok()?.into_iter().next()?;
        let line: Value = serde_json::from_str(&first).ok()?;
        let folder = line["payload"]["cwd"].as_str().filter(|c| !c.is_empty() && line["type"] == "session_meta")?.to_string();
        Some(super::Worked { folder, at: *at })
    });
    super::newest(found)
}

// ---- setting Codex up to report: its hooks, in its own hooks file ----

const HOOKS: &[On] = &[on("SessionStart", 5), on("UserPromptSubmit", 5), on("Stop", 5), on("SubagentStart", 5), on("SubagentStop", 5), on("SessionEnd", 2), on("PermissionRequest", 900)];

/// The socket Codex's app server listens on, under its home, as its row has it.
fn control_socket() -> PathBuf {
    home().join(super::row_says(NAME, "control_socket"))
}
fn home() -> PathBuf {
    super::tool_home(NAME)
}

pub(crate) struct Codex;

impl super::Harness for Codex {
    fn name(&self) -> &'static str {
        NAME
    }
    fn events(&self, hook: &Value) -> Vec<AgentEvent> {
        events(hook)
    }
    /// Only when Codex is on this computer.
    fn hooks(&self) -> Option<super::hooks::File> {
        home().is_dir().then(|| super::hooks::File { name: NAME, path: home().join(super::row_says(NAME, "hooks_file")), events: HOOKS })
    }
    /// Codex runs a hook only once he has trusted it, and asks again whenever the file changes.
    fn installed(&self, file: &super::hooks::File, changed: bool) -> String {
        let name = super::called(NAME).0;
        if changed { format!("{name} reports to airband once you allow it: open a new {name} session and trust airband\u{2019}s hooks when it asks. (Set in {}.)", file.path.display()) } else { format!("{name} reports to airband (set in {}).", file.path.display()) }
    }
    fn permission(&self, hook: &Value) -> Option<PermissionAsk> {
        permission(hook)
    }
    fn glance(&self, agent: &Agent) -> super::Glance {
        glance(agent)
    }
    fn deliver(&self, session: &str, words: &str) -> Result<(), String> {
        deliver(session, words)
    }
    fn open(&self) -> Option<Vec<super::Open>> {
        open()
    }
    fn watch(&self, tower: super::Tower) {
        watch(tower)
    }
    fn worked(&self) -> Vec<super::Worked> {
        worked_in(&home())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_that_began_and_ended_between_two_looks_has_what_it_came_to_said() {
        let over = Some(("t1".to_string(), true));
        assert_eq!(first_seen(true, &over), FirstSeen::Nothing, "said before the tower started");
        assert_eq!(first_seen(false, &over), FirstSeen::Over);
        assert_eq!(first_seen(false, &Some(("t1".to_string(), false))), FirstSeen::Working);
        assert_eq!(first_seen(false, &None), FirstSeen::Nothing);
    }

    #[test]
    fn what_he_is_told_when_the_tool_will_not_take_his_words_never_holds_what_it_wrote() {
        let said = didnt_take("the tool", "prog");
        assert!(said.contains("stopped with an error") && said.contains("terminal"));
    }
    use serde_json::json;

    #[test]
    fn a_codex_session_checks_in_speaks_and_is_reached_as_his_own_turn() {
        let hook = json!({ "session_id": "t1", "cwd": "/x/shop", "hook_event_name": "Stop", "model": "gpt-6-luna", "last_assistant_message": " Done. " });
        let e = events(&hook).remove(0);
        assert_eq!((e.agent.id.as_str(), e.agent.harness.as_str()), ("t1", "codex"));
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None, turn: None });
        assert_eq!(glance(&e.agent).model.as_deref(), Some("gpt-6-luna"));
        // his words go to Codex's own queue for that session, whole, as one argument
        match e.agent.deliver {
            Some(Deliver::Harness { name, session }) => assert_eq!((name.as_str(), session.as_str()), ("codex", "t1")),
            other => panic!("{other:?}"),
        }
        assert_eq!(events(&json!({ "session_id": "t1", "hook_event_name": "SessionEnd" }))[0].event, Event::Signoff);
    }

    #[test]
    fn a_thread_codex_has_open_is_a_session_he_can_reach_and_a_subagents_is_not() {
        // as Codex 0.171 describes one
        let thread = json!({ "id": "01a1", "parentThreadId": null, "ephemeral": false, "model": "gpt-6.2-sol", "status": { "type": "idle" }, "cwd": "/x/shop", "name": "Say hello", "source": "vscode" });
        let found = open_of(&thread).unwrap().agent;
        assert_eq!((found.id.as_str(), found.harness.as_str(), found.cwd.as_deref()), ("01a1", "codex", Some("/x/shop")));
        assert_eq!(found.deliver, Some(Deliver::Harness { name: "codex".into(), session: "01a1".into() }));
        // the name is one Codex made up, not one he gave
        assert_eq!(found.title, None);
        assert_eq!(glance(&found).model.as_deref(), Some("gpt-6.2-sol"));
        let mut child = thread.clone();
        child["parentThreadId"] = json!("01a0");
        assert!(open_of(&child).is_none());
        let mut once = thread.clone();
        once["ephemeral"] = json!(true);
        assert!(open_of(&once).is_none());
    }

    #[test]
    fn a_turn_as_codexs_server_gives_it_is_working_said_or_stopped_and_carries_its_id() {
        let turn = |status: &str, items: Value| json!({ "id": "turn-7", "status": status, "items": items, "error": null });
        assert_eq!(turn_event(&turn("inProgress", json!([]))), Some(Event::Working));
        // what it said is the last thing it wrote, not what it wrote on the way or the commands it ran
        let items = json!([{ "type": "agentMessage", "text": "Looking." }, { "type": "commandExecution", "command": "ls" }, { "type": "agentMessage", "text": " Two boxes. " }, { "type": "reasoning", "summary": [] }]);
        assert_eq!(turn_event(&turn("completed", items)), Some(Event::Said { text: "Two boxes.".into(), kind: None, turn: Some("turn-7".into()) }));
        // its hook says the same turn by the same id
        let hooked = events(&json!({ "session_id": "t1", "hook_event_name": "Stop", "turn_id": "turn-7", "last_assistant_message": "Two boxes." })).remove(0).event;
        assert_eq!(hooked, Event::Said { text: "Two boxes.".into(), kind: None, turn: Some("turn-7".into()) });
        assert_eq!(turn_event(&turn("completed", json!([]))), Some(Event::Stopped { why: None }));
        assert_eq!(turn_event(&turn("interrupted", json!([]))), Some(Event::Stopped { why: None }));
        let mut failed = turn("failed", json!([]));
        failed["error"] = json!({ "message": "The model could not be reached." });
        assert_eq!(turn_event(&failed), Some(Event::Stopped { why: Some("The model could not be reached.".into()) }));
        assert_eq!(turn_event(&turn("something new", json!([]))), None);
    }

    #[test]
    fn the_newest_sessions_are_the_ones_written_last_never_the_ones_whose_paths_sort_last() {
        let dir = std::env::temp_dir().join(format!("airband-codex-{}", crate::config::random_hex(6)));
        let meta = |cwd: &str| format!("{}\n", json!({ "type": "session_meta", "payload": { "id": "t", "cwd": cwd } }));
        let day = dir.join("sessions").join("2026").join("11").join("21");
        std::fs::create_dir_all(&day).unwrap();
        for i in 0..300 {
            std::fs::write(day.join(format!("rollout-{i}.jsonl")), meta("/x/new")).unwrap();
        }
        // its path sorts after every other, but it was written long ago
        let late = dir.join("sessions").join("2026").join("12").join("31");
        std::fs::create_dir_all(&late).unwrap();
        std::fs::write(late.join("rollout-z.jsonl"), meta("/x/old")).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 24 * 3600);
        std::fs::File::options().write(true).open(late.join("rollout-z.jsonl")).unwrap().set_modified(old).unwrap();
        let worked: Vec<String> = worked_in(&dir).into_iter().map(|w| w.folder).collect();
        assert_eq!(worked, ["/x/new"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn where_codex_has_worked_is_read_from_the_first_line_of_each_session() {
        let dir = std::env::temp_dir().join(format!("airband-codex-{}", crate::config::random_hex(6)));
        let day = dir.join("sessions").join("2026").join("11").join("21");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |cwd: &str| format!("{}\n{}\n", json!({ "type": "session_meta", "payload": { "id": "t", "cwd": cwd } }), json!({ "type": "event", "payload": { "cwd": "/not/this" } }));
        std::fs::write(day.join("rollout-a.jsonl"), meta("/x/shop")).unwrap();
        std::fs::write(day.join("rollout-b.jsonl"), meta("/x/shop")).unwrap();
        std::fs::write(day.join("rollout-c.jsonl"), meta("/x/garden")).unwrap();
        // one just opened has nothing in it yet, and something else's file is not a session
        std::fs::write(day.join("rollout-d.jsonl"), "").unwrap();
        std::fs::write(day.join("notes.txt"), meta("/x/notes")).unwrap();
        let mut worked: Vec<String> = worked_in(&dir).into_iter().inspect(|w| assert!(w.at > 0)).map(|w| w.folder).collect();
        worked.sort();
        assert_eq!(worked, ["/x/garden", "/x/shop"]);
        assert!(worked_in(&dir.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_codex_permission_request_is_put_to_him_whole() {
        let ask = permission(&json!({ "session_id": "t1", "cwd": "/x", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "npm run deploy" } })).unwrap();
        assert_eq!(ask.what, "run `npm run deploy`");
        assert!(super::super::Harness::permission_answer(&Codex, &ask, true, "").contains("\"behavior\":\"allow\""));
        assert!(super::super::Harness::permission_answer(&Codex, &ask, false, "not now").contains("not now"));
        assert!(permission(&json!({ "session_id": "t1", "hook_event_name": "Stop" })).is_none());
    }

    #[test]
    fn a_request_to_run_a_command_from_codexs_server_is_put_to_him_whole_and_no_other_kind_is() {
        let agent = open_of(&json!({ "id": "01a1", "parentThreadId": null, "ephemeral": false, "status": { "type": "active", "activeFlags": ["waitingOnApproval"] }, "cwd": "/x/shop" })).unwrap().agent;
        assert!(waits_for_leave(&json!({ "status": { "type": "active", "activeFlags": ["waitingOnApproval"] } })));
        assert!(!waits_for_leave(&json!({ "status": { "type": "active", "activeFlags": [] } })) && !waits_for_leave(&json!({ "status": { "type": "idle" } })));
        // as Codex 0.171 sends one
        let request = json!({ "method": "item/commandExecution/requestApproval", "id": 0, "params": { "kind": "command", "threadId": "01a1", "turnId": "t1", "itemId": "call_1", "reason": "It needs to write a file.", "command": "/bin/zsh -lc 'touch made.txt'", "cwd": "/x/shop" } });
        let ask = leave_of(agent.clone(), &request).unwrap();
        assert_eq!(ask.what, "run `/bin/zsh -lc 'touch made.txt'`");
        assert!(ask.detail.contains("/x/shop") && ask.detail.contains("It needs to write a file."));
        // a change to files comes as a reference to it, not as what will change: that one stays at the keyboard
        assert!(leave_of(agent.clone(), &json!({ "method": "item/fileChange/requestApproval", "id": 1, "params": { "threadId": "01a1", "itemId": "call_2" } })).is_none());
        // and a command with nothing in it is not asked
        assert!(leave_of(agent, &json!({ "method": "item/commandExecution/requestApproval", "id": 2, "params": { "threadId": "01a1", "command": " " } })).is_none());
    }
}
