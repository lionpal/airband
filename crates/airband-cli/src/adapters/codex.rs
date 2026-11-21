//! OpenAI's Codex, in a terminal he opened himself.
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
//!   typed in it, and `codex queue` reaches it. What a session says still comes by its hooks, which Codex
//!   reads when a session starts: one opened before the hooks were there is on the radio and takes his
//!   words, and is heard from once it has been started again.
//!
//! One thing he has to do himself, once: Codex runs a hook only after he has looked at it and trusted it
//! (`/hooks` inside Codex), and again whenever it changes. `airband install` writes the hooks and says so.
use super::env;
use super::hooks::{On, on};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk, SCHEMA};
use serde_json::Value;
use std::path::PathBuf;

const NAME: &str = "codex";

fn agent_of(hook: &Value) -> Option<Agent> {
    let text = |k: &str| hook[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    let id = text("session_id")?;
    let mut meta = serde_json::Map::new();
    if let Some(model) = text("model") {
        meta.insert("model".into(), Value::String(model));
    }
    Some(Agent {
        // What he says is queued for the session with Codex's own command (`deliver` below). The session takes
        // it as a turn of his.
        deliver: if env("AIRBAND_HELD").is_some() { Some(Deliver::Held) } else { Some(Deliver::Harness { name: NAME.into(), session: id.clone() }) },
        id,
        harness: NAME.into(),
        cwd: text("cwd"),
        name: env("AIRBAND_NAME"),
        title: None,
        pid: None,
        meta,
    })
}

pub fn events(hook: &Value) -> Vec<AgentEvent> {
    let text = |k: &str| hook[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    let events = match hook["hook_event_name"].as_str().unwrap_or("") {
        "SessionStart" => vec![Event::Checkin],
        "UserPromptSubmit" => vec![Event::Working],
        "Stop" => text("last_assistant_message").map(|text| Event::Said { text, kind: None }).into_iter().collect(),
        "SubagentStart" => vec![Event::Subagent { done: false, what: text("agent_type").unwrap_or_default() }],
        "SubagentStop" => vec![Event::Subagent { done: true, what: text("agent_type").unwrap_or_default() }],
        "SessionEnd" => vec![Event::Signoff],
        _ => Vec::new(),
    };
    let Some(agent) = agent_of(hook) else { return Vec::new() };
    events.into_iter().map(|event| AgentEvent { schema: SCHEMA.into(), agent: agent.clone(), event }).collect()
}

/// What it runs on, which every event of its says. What it is doing is not known without reading its record.
pub fn glance(agent: &Agent) -> super::Glance {
    super::Glance { doing: None, model: agent.meta.get("model").and_then(Value::as_str).map(String::from), effort: None }
}

/// A request for leave, as he is asked it: whole, never cut, because what he reads is what will run.
pub fn permission(hook: &Value) -> Option<PermissionAsk> {
    if hook["hook_event_name"].as_str() != Some("PermissionRequest") {
        return None;
    }
    let tool = hook["tool_name"].as_str().unwrap_or("a tool");
    let input = &hook["tool_input"];
    let field = |k: &str| input[k].as_str().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let what = match tool {
        "Bash" | "shell" | "exec_command" => field("command").or_else(|| input["command"].as_array().map(|parts| parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))).map(|c| format!("run `{c}`")),
        "apply_patch" => Some(field("description").unwrap_or_else(|| "change files".into())),
        _ => field("description"),
    }
    .unwrap_or_else(|| format!("use {tool}"));
    // one too long to put in front of him whole is not asked on the phone: Codex asks at its keyboard
    super::hooks::leave(agent_of(hook)?, tool, what, input, " The rest is at the keyboard.", None)
}

/// His words into that session as a turn of his own, by Codex's own queue.
fn deliver(session: &str, words: &str) -> Result<(), String> {
    let done = std::process::Command::new(super::row_says(NAME, "program")).args(["queue", "--thread", session, "--resume", "--message", words]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).output().map_err(|e| format!("Codex could not be started: {e}"))?;
    if done.status.success() { Ok(()) } else { Err(format!("{} did not take it: {}", super::called(NAME).0, String::from_utf8_lossy(&done.stderr).trim().chars().take(200).collect::<String>())) }
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
        let stream = std::os::unix::net::UnixStream::connect(home().join("app-server-control").join("app-server-control.sock")).ok()?;
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

/// The sessions open now: the threads Codex's server has loaded, each read for where it is.
#[cfg(unix)]
fn open() -> Option<Vec<super::Open>> {
    let mut server = Server::connect()?;
    let (mut open, mut cursor) = (Vec::new(), Value::Null);
    for _ in 0..20 {
        let page = server.ask("thread/loaded/list", serde_json::json!({ "cursor": cursor }))?;
        for id in page["data"].as_array()?.iter().filter_map(Value::as_str) {
            // one that cannot be read just now is not one that has gone: then nothing is said of any of them
            open.extend(open_of(&server.ask("thread/read", serde_json::json!({ "threadId": id }))?["thread"]));
        }
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Some(open)
}
#[cfg(not(unix))]
fn open() -> Option<Vec<super::Open>> {
    None
}

/// One of Codex's threads, as it describes it, as a session on the radio. A subagent's thread is its
/// parent's business, and one that keeps no record is a one-off run: neither is his to talk to. The name
/// a thread has is one Codex made up from its first words, so it is not taken for a name he gave.
fn open_of(thread: &Value) -> Option<super::Open> {
    let text = |k: &str| thread[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    if !thread["parentThreadId"].is_null() || thread["ephemeral"] == true {
        return None;
    }
    let id = text("id")?;
    let mut meta = serde_json::Map::new();
    if let Some(model) = text("model") {
        meta.insert("model".into(), Value::String(model));
    }
    Some(super::Open {
        agent: Agent { deliver: Some(Deliver::Harness { name: NAME.into(), session: id.clone() }), id, harness: NAME.into(), cwd: text("cwd"), name: None, title: None, pid: None, meta },
    })
}

/// The folders Codex has had sessions in: each session's file starts with a line that says where it was.
/// The newest few hundred files are asked.
fn worked_in(codex: &std::path::Path) -> Vec<super::Worked> {
    use std::io::BufRead;
    // sessions/<year>/<month>/<day>/<one file per session>
    let inside = |dirs: Vec<PathBuf>| -> Vec<PathBuf> { dirs.iter().filter_map(|d| std::fs::read_dir(d).ok()).flat_map(|d| d.flatten().map(|e| e.path())).collect() };
    let mut files: Vec<PathBuf> = inside(inside(inside(inside(vec![codex.join("sessions")])))).into_iter().filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect();
    files.sort();
    let found = files.iter().rev().take(300).filter_map(|file| {
        let mut first = String::new();
        std::io::BufReader::new(std::fs::File::open(file).ok()?).read_line(&mut first).ok()?;
        let line: Value = serde_json::from_str(&first).ok()?;
        let folder = line["payload"]["cwd"].as_str().filter(|c| !c.is_empty() && line["type"] == "session_meta")?.to_string();
        Some(super::Worked { folder, at: super::written(file) })
    });
    super::newest(found)
}

// ---- setting Codex up to report: its hooks, in its own hooks file ----

const HOOKS: &[On] = &[on("SessionStart", 5), on("UserPromptSubmit", 5), on("Stop", 5), on("SubagentStart", 5), on("SubagentStop", 5), on("SessionEnd", 2), on("PermissionRequest", 900)];

fn home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".codex"))
}

pub struct Codex;

impl super::Harness for Codex {
    fn name(&self) -> &'static str {
        NAME
    }
    fn events(&self, hook: &Value) -> Vec<AgentEvent> {
        events(hook)
    }
    /// Only when Codex is on this computer.
    fn hooks(&self) -> Option<super::hooks::File> {
        home().is_dir().then(|| super::hooks::File { name: NAME, path: home().join("hooks.json"), events: HOOKS })
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
    fn worked(&self) -> Vec<super::Worked> {
        worked_in(&home())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_codex_session_checks_in_speaks_and_is_reached_as_his_own_turn() {
        let hook = json!({ "session_id": "t1", "cwd": "/x/shop", "hook_event_name": "Stop", "model": "gpt-6-luna", "last_assistant_message": " Done. " });
        let e = events(&hook).remove(0);
        assert_eq!((e.agent.id.as_str(), e.agent.harness.as_str()), ("t1", "codex"));
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None });
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
}
