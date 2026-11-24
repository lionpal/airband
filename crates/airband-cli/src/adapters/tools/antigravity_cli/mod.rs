//! Antigravity CLI (`agy`), Google's coding agent, in a terminal he opened himself or held by the tower.
//!
//! An agent the tower starts is held through the Agent Client Protocol (`agy acp`, its row's launcher): its
//! models and how hard it thinks are listed by the session itself, and his words go in as his own turns. Its
//! requests for leave name the tool and give what the tool was given (`rawInput`) in the tool's own fields,
//! so they are read here (`acp_leave`): a command is put to him as the command itself.
//!
//! A session he opened himself reports through Antigravity's hooks, one named entry in its shared hooks file
//! (`config/hooks.json` under its folder). They say when a session starts, when a turn begins (the first call
//! to the model after his prompt) and when it stops, each with the conversation's id, where it works and the
//! model it runs on. What it said is the last answer in its own transcript, whose path the hook gives.
//!
//! What it does not have, and so what such a session does not do here:
//! - no hook that asks leave: its own prompt is not one a hook can answer (`PreToolUse` runs for every tool,
//!   before Antigravity has decided whether to ask), so its requests for leave stay at its own keyboard;
//! - no way into a session that is open: his words reach only an agent the tower holds;
//! - no hook when a session ends: one has ended when the lock it holds while open (`presence/`) is let go.
#![cfg_attr(test, allow(clippy::disallowed_macros, reason = "fixtures in the tool's own shapes"))]
use crate::adapters::hooks::On;
use airband_core::agent::{Agent, AgentEvent, Event};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NAME: &str = "antigravity-cli";
/// The name its entry in the hooks file goes under.
const ENTRY: &str = "airband";

/// Its folder (`home`), and its own files in it (`state`), as its row has them.
fn home() -> PathBuf {
    crate::adapters::tool_home(NAME)
}
fn state() -> PathBuf {
    home().join(crate::adapters::row_says(NAME, "state"))
}

// ---- its hooks ----

/// The events of its hooks that are read here, by the names it gives them (`hookEventName`).
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
enum HookEvent {
    SessionStart,
    PreInvocation,
    Stop,
    #[serde(other)]
    Other,
}

/// Why its loop stopped, as a `Stop` hook is told (`terminationReason`).
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Stopped {
    /// It answered and called no tool: the turn is over.
    NoToolCall,
    Error,
    UserCanceled,
    /// It stopped for a reason of its own (a limit, a step that ends it, a hook): what it last said stands.
    #[serde(other)]
    Other,
}

/// What a hook is given on its standard input, the fields read here (`docs/hooks.md` of its own skills).
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Hook {
    conversation_id: String,
    hook_event_name: HookEvent,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    transcript_path: Option<PathBuf>,
    /// Which call to the model this is in the turn: the first is 0.
    #[serde(default)]
    invocation_num: Option<u64>,
    #[serde(default)]
    termination_reason: Option<Stopped>,
    #[serde(default)]
    error: Option<String>,
}

fn agent(id: &str, cwd: Option<String>, model: Option<String>) -> Agent {
    let mut meta = serde_json::Map::new();
    if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
        meta.insert("model".into(), Value::String(model));
    }
    // no way into one of its open sessions: his words reach only an agent the tower holds
    Agent { cwd: cwd.filter(|c| !c.is_empty()), name: crate::adapters::env("AIRBAND_NAME"), meta, ..crate::adapters::agent_with(id, NAME) }
}

fn events(hook: &Value) -> Vec<AgentEvent> {
    let Ok(hook) = Hook::deserialize(hook) else { return Vec::new() };
    if hook.conversation_id.trim().is_empty() {
        return Vec::new();
    }
    let said = match hook.hook_event_name {
        HookEvent::SessionStart => vec![Event::Checkin],
        // a turn begins with its first call to the model; the later ones are the same turn going on
        HookEvent::PreInvocation if hook.invocation_num == Some(0) => vec![Event::Working],
        HookEvent::Stop => vec![ended(&hook)],
        _ => Vec::new(),
    };
    crate::adapters::hooks::from_agent(Some(agent(&hook.conversation_id, hook.cwd.clone(), hook.model_name.clone())), said)
}

/// What a stopped turn came to: its error in its own words, nothing for one he stopped, else its last answer.
fn ended(hook: &Hook) -> Event {
    match hook.termination_reason {
        Some(Stopped::Error) => Event::Stopped { why: hook.error.clone().filter(|e| !e.trim().is_empty()) },
        Some(Stopped::UserCanceled) => Event::Stopped { why: None },
        Some(Stopped::NoToolCall | Stopped::Other) | None => match hook.transcript_path.as_deref().and_then(last_answer) {
            Some(text) => Event::Said { text, kind: None, turn: None },
            None => Event::Stopped { why: None },
        },
    }
}

// ---- its transcript ----

/// Who wrote a step of its transcript, and what kind of step it is, as its transcript names them.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Source {
    Model,
    UserExplicit,
    #[serde(other)]
    Other,
}
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum StepType {
    UserInput,
    PlannerResponse,
    #[serde(other)]
    Other,
}
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Status {
    Done,
    #[serde(other)]
    Other,
}

/// One step of its transcript, the fields read here.
#[derive(Deserialize, Debug)]
struct Step {
    source: Source,
    #[serde(rename = "type")]
    kind: StepType,
    status: Status,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<Value>,
}

impl Step {
    /// What the model said in this step, when it is an answer of its with words in it.
    fn answer(&self) -> Option<String> {
        (self.source == Source::Model && self.kind == StepType::PlannerResponse).then_some(())?;
        self.content.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from)
    }
    /// A finished answer that calls no tool: it is waiting for him.
    fn waits(&self) -> bool {
        self.status == Status::Done && self.tool_calls.is_empty() && self.answer().is_some()
    }
}

/// The most of a transcript read from its end the first time it is looked at: its last turn is in its last part.
const TAIL: u64 = 8 << 20;

/// The steps in a transcript from `from` up to `to`, in order, with where the last whole line of them ends. Only
/// lines that end in a newline are read: one still being written is left for the next look. A part that starts
/// in the middle of a line leaves that line out, and a line that does not read as a step is passed over.
fn steps_between(path: &Path, from: u64, to: u64) -> Option<(Vec<Step>, u64)> {
    // from one byte before, so a part that starts right after a newline drops only that newline
    let at = from.saturating_sub(1);
    let bytes = airband_io::files::read_part(path, at, to.saturating_sub(at)).ok()?;
    let skip = match from {
        0 => 0,
        _ => match bytes.iter().position(|b| *b == b'\n') {
            Some(i) => i + 1,
            None => return Some((Vec::new(), from)),
        },
    };
    let body = &bytes[skip..];
    let whole = body.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let steps = body[..whole].split(|b| *b == b'\n').filter_map(|l| serde_json::from_slice(l).ok()).collect();
    Some((steps, at + (skip + whole) as u64))
}

/// The steps at the end of a transcript, newest first.
fn last_steps(path: &Path) -> Vec<Step> {
    let Ok(size) = airband_io::files::metadata(path).map(|m| m.len()) else { return Vec::new() };
    let mut steps = steps_between(path, size.saturating_sub(TAIL), size).map(|(s, _)| s).unwrap_or_default();
    steps.reverse();
    steps
}

/// The last thing it said since his last prompt, from its transcript.
fn last_answer(path: &Path) -> Option<String> {
    last_steps(path).into_iter().take_while(|s| s.kind != StepType::UserInput).find_map(|s| s.answer())
}

/// How far an open conversation's transcript has been read, and whether the last whole step in it waits for him.
struct Read {
    to: u64,
    waits: bool,
}

/// What has been read of each open conversation's transcript, by its path, so each look reads only what is new.
static READ: std::sync::Mutex<std::collections::BTreeMap<PathBuf, Read>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Its transcript says it is waiting for him: its last step is a finished answer that calls no tool. Only what
/// was written since the last look is read; a transcript that is shorter than what was read is read again.
fn waiting(path: &Path) -> bool {
    let mut read = READ.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(size) = airband_io::files::metadata(path).map(|m| m.len()) else {
        read.remove(path);
        return false;
    };
    let (from, waits) = match read.get(path).filter(|r| r.to <= size) {
        Some(r) => (r.to, r.waits),
        None => (size.saturating_sub(TAIL), false),
    };
    let Some((steps, to)) = steps_between(path, from, size) else { return false };
    let waits = steps.last().map_or(waits, Step::waits);
    read.insert(path.to_path_buf(), Read { to, waits });
    waits
}

/// Forget what was read of transcripts whose conversations are no longer open.
fn forget_all_but(open: &[PathBuf]) {
    READ.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|path, _| open.contains(path));
}

// ---- what it says is open, and where it has worked ----

/// The folder each conversation was last had in, by its own record of the latest in each folder. A conversation
/// that is not the latest anywhere has no folder by that record.
fn folders() -> Vec<(String, String)> {
    let path = state().join(crate::adapters::row_says(NAME, "last_conversations"));
    let read: Option<std::collections::BTreeMap<String, String>> = airband_io::files::read_bytes(&path).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok());
    read.unwrap_or_default().into_iter().filter(|(folder, id)| !folder.is_empty() && !id.is_empty()).collect()
}

/// The folder a conversation works in, by its own record in Antigravity's database of conversations (the
/// `conversation_summaries` table, its `workspace_uris`: a JSON list of `file://` addresses). A conversation it
/// records no folder for, or more than one, has no one folder by that record. Opened read-only: Antigravity
/// may be writing it.
fn own_folders(path: &Path, ids: &[&str]) -> std::collections::BTreeMap<String, String> {
    use rusqlite::{Connection, OpenFlags};
    let mut found = std::collections::BTreeMap::new();
    if ids.is_empty() || !path.is_file() {
        return found;
    }
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return found };
    let Ok(mut q) = db.prepare("SELECT workspace_uris FROM conversation_summaries WHERE conversation_id = ?1") else { return found };
    for id in ids {
        let Ok(uris) = q.query_row([id], |r| r.get::<_, String>(0)) else { continue };
        let Ok(uris) = serde_json::from_str::<Vec<String>>(&uris) else { continue };
        if let [one] = uris.as_slice()
            && let Some(folder) = file_path(one)
        {
            found.insert((*id).to_string(), folder);
        }
    }
    found
}

/// The path a `file://` address names (RFC 8089, on this computer: no host, or `localhost`), its escapes undone.
/// Nothing for any other address, or one whose path is not text.
fn file_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    path.starts_with('/').then_some(())?;
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().filter(|p| p.len() > 1)
}

fn transcript_of(id: &str) -> PathBuf {
    state().join(crate::adapters::row_says(NAME, "brain")).join(id).join(crate::adapters::row_says(NAME, "transcript"))
}

/// The kind of lock its row says a conversation holds on its file in `presence/`: `system` is the system's own
/// whole-file lock, the kind `airband_io::files::locked` asks (flock on Unix, LockFileEx on Windows). Antigravity
/// 1.5.15 takes it with its bundled flock library's `TryLock`, an exclusive flock(2), and that library opens the
/// file sharing it for reading on Windows, so another process can open it to ask.
const SYSTEM_LOCK: &str = "system";

/// The conversations open now: each holds a lock on its file in `presence/` for as long as it is open. Nothing
/// when that folder cannot be read (it is not on this computer), or when its row says it locks some other way
/// than the one asked here.
fn open() -> Option<Vec<crate::adapters::Open>> {
    let dir = state().join(crate::adapters::row_says(NAME, "presence"));
    if crate::adapters::row_says(NAME, "presence_lock") != SYSTEM_LOCK || !dir.is_dir() {
        return None;
    }
    let files = airband_io::files::entries(&dir).ok()?;
    let folders = folders();
    let ids: Vec<String> = files.iter().filter(|f| f.extension().is_some_and(|e| e == "lock")).filter_map(|f| f.file_stem().and_then(|s| s.to_str()).map(String::from)).collect();
    let own = own_folders(&state().join(crate::adapters::row_says(NAME, "summaries")), &ids.iter().map(String::as_str).collect::<Vec<_>>());
    let mut open = Vec::new();
    let mut transcripts = Vec::new();
    for file in files {
        // a lock file is the conversation's id and `.lock`, read as the path's own parts, never by its spelling
        let (Some(id), Some(ext)) = (file.file_stem().and_then(|s| s.to_str()), file.extension()) else { continue };
        if ext != "lock" {
            continue;
        }
        match airband_io::files::locked(&file) {
            Ok(Some(true)) => {}
            Ok(_) => continue,
            // one that cannot be asked is not one that has closed: nothing is said of any this look
            Err(_) => return None,
        }
        // where it works, by its own record; one not recorded there yet (a conversation that has not been
        // summed up) by the record of the latest conversation in each folder, which names it when it is that
        let cwd = own.get(id).cloned().or_else(|| folders.iter().find(|(_, last)| last == id).map(|(folder, _)| folder.clone()));
        let transcript = transcript_of(id);
        open.push(crate::adapters::Open { agent: agent(id, cwd, None), idle: waiting(&transcript) });
        transcripts.push(transcript);
    }
    forget_all_but(&transcripts);
    Some(open)
}

/// The folders it has had conversations in, by its record of the latest in each, with when that one was last
/// written to.
fn worked() -> Vec<crate::adapters::Worked> {
    let conversations = state().join(crate::adapters::row_says(NAME, "conversations"));
    let found = folders().into_iter().filter_map(|(folder, id)| {
        // a conversation's file is its id and `.db`, as it names them
        let at = crate::adapters::written(&conversations.join(&id).with_extension("db"))?;
        Some(crate::adapters::Worked { folder, at })
    });
    crate::adapters::newest(found)
}

// ---- an agent the tower holds: its requests for leave ----

/// A request for leave of `agy acp`'s, as he is asked it: its tool by its own name (`toolCall.name`) with what
/// that tool was given (`rawInput`), whole. A tool its row has words for is put in those words; one whose words
/// it does not fill (a field missing) says nothing he could answer, so it is not asked. A tool the row has no
/// words for is put as itself, by its name with exactly what it was given: never by Antigravity's title for the
/// call, which does not say what will run.
fn acp_leave(request: &Value, agent: &Agent) -> crate::adapters::AcpLeave {
    let call = &request["toolCall"];
    let input = &call["rawInput"];
    let Some(tool) = call["name"].as_str().filter(|t| !t.is_empty()) else { return crate::adapters::AcpLeave::Own(None) };
    let words = &crate::adapters::hooks::tool_use(NAME, tool)["asks"];
    let what = match words {
        Value::Null => Some(format!("{tool} {input}")),
        words => crate::adapters::hooks::fill(words, input),
    };
    crate::adapters::AcpLeave::Own(what.and_then(|what| crate::adapters::hooks::leave(agent.clone(), tool, what, input, "", None)).map(Box::new))
}

/// What it runs on, as its hooks say.
fn glance(agent: &Agent) -> crate::adapters::Glance {
    crate::adapters::Glance { doing: None, model: agent.meta.get("model").and_then(Value::as_str).map(String::from), effort: None }
}

// ---- setting it up to report ----

const HOOKS: &[On] = &[crate::adapters::hooks::on("SessionStart", 5), crate::adapters::hooks::on("PreInvocation", 5), crate::adapters::hooks::on("Stop", 5)];

pub(crate) struct AntigravityCli;

impl crate::adapters::Harness for AntigravityCli {
    fn name(&self) -> &'static str {
        NAME
    }
    fn events(&self, hook: &Value) -> Vec<AgentEvent> {
        events(hook)
    }
    /// Only once it has been run on this computer: its own folder is there.
    fn hooks(&self) -> Option<crate::adapters::hooks::File> {
        state().is_dir().then(|| crate::adapters::hooks::File { name: NAME, path: home().join(crate::adapters::row_says(NAME, "hooks_file")), events: HOOKS, shape: crate::adapters::hooks::Shape::Named(ENTRY) })
    }
    /// Its hooks read what is printed as JSON, and take an empty object as nothing to say.
    fn quiet(&self) -> Option<&'static str> {
        Some("{}")
    }
    fn acp_leave(&self, request: &Value, agent: &Agent) -> crate::adapters::AcpLeave {
        acp_leave(request, agent)
    }
    fn glance(&self, agent: &Agent) -> crate::adapters::Glance {
        glance(agent)
    }
    fn open(&self) -> Option<crate::adapters::Listing> {
        open().map(crate::adapters::Listing::whole)
    }
    fn worked(&self) -> Vec<crate::adapters::Worked> {
        worked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook(event: &str, more: Value) -> Value {
        let mut h = json!({ "conversationId": "c1", "cwd": "/x/shop", "hookEventName": event, "modelName": "gemini-4-neon-low", "workspacePaths": ["/x/shop"], "lastUserInput": "his words" });
        h.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        h
    }

    #[test]
    fn a_session_checks_in_works_from_its_first_call_to_the_model_and_says_its_last_answer() {
        let e = events(&hook("SessionStart", json!({})));
        assert_eq!((e[0].agent.id.as_str(), e[0].agent.harness.as_str(), e[0].agent.cwd.as_deref(), &e[0].event), ("c1", NAME, Some("/x/shop"), &Event::Checkin));
        assert_eq!(glance(&e[0].agent).model.as_deref(), Some("gemini-4-neon-low"));
        // nothing reaches one of its own open sessions from here
        assert_eq!(e[0].agent.deliver, None);
        assert_eq!(events(&hook("PreInvocation", json!({ "invocationNum": 0 })))[0].event, Event::Working);
        assert!(events(&hook("PreInvocation", json!({ "invocationNum": 1 }))).is_empty());
        let dir = std::env::temp_dir().join(format!("airband-agy-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("transcript_full.jsonl");
        std::fs::write(&t, include_str!("fixtures/transcript.jsonl")).unwrap();
        let stop = |reason: &str, error: &str| events(&hook("Stop", json!({ "terminationReason": reason, "error": error, "transcriptPath": t, "fullyIdle": true })))[0].event.clone();
        assert_eq!(stop("NO_TOOL_CALL", ""), Event::Said { text: "finished".into(), kind: None, turn: None });
        assert_eq!(stop("ERROR", "The model could not be reached."), Event::Stopped { why: Some("The model could not be reached.".into()) });
        assert_eq!(stop("USER_CANCELED", ""), Event::Stopped { why: None });
        assert_eq!(stop("MAX_INVOCATIONS", ""), Event::Said { text: "finished".into(), kind: None, turn: None });
        assert!(waiting(&t));
        // his next prompt is in, and nothing has been answered since: it has nothing to say for that turn
        std::fs::write(&t, format!("{}{}\n", include_str!("fixtures/transcript.jsonl"), json!({ "step_index": 9, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "content": "more" }))).unwrap();
        assert_eq!(stop("NO_TOOL_CALL", ""), Event::Stopped { why: None });
        assert!(!waiting(&t));
        // an event it has that is not read here, and one that names no conversation, say nothing
        assert!(events(&hook("PostToolUse", json!({}))).is_empty());
        assert!(events(&json!({ "hookEventName": "SessionStart" })).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_open_conversation_is_read_only_where_it_is_new_and_a_line_still_being_written_waits() {
        let dir = std::env::temp_dir().join(format!("airband-agy-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("transcript_full.jsonl");
        let answer = json!({ "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": "done" }).to_string();
        let asked = json!({ "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "content": "more" }).to_string();
        std::fs::write(&t, format!("{answer}\n")).unwrap();
        assert!(waiting(&t));
        let at = READ.lock().unwrap()[&t].to;
        // his next prompt half written: not read yet, and what was read stands
        let (head, tail) = asked.split_at(10);
        std::fs::write(&t, format!("{answer}\n{head}")).unwrap();
        assert!(waiting(&t));
        assert_eq!(READ.lock().unwrap()[&t].to, at);
        // the rest of it: read from where the last look stopped
        std::fs::write(&t, format!("{answer}\n{head}{tail}\n")).unwrap();
        assert!(!waiting(&t));
        // forgotten once the conversation is no longer open
        forget_all_but(&[]);
        assert!(!READ.lock().unwrap().contains_key(&t));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_conversation_works_in_the_one_folder_its_own_record_names() {
        let dir = std::env::temp_dir().join(format!("airband-agy-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conversation_summaries.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE conversation_summaries (conversation_id text, workspace_uris text NOT NULL, PRIMARY KEY (conversation_id))").unwrap();
        for (id, uris) in [("c1", r#"["file:///x/my%20shop"]"#), ("c2", r#"["file:///x/a","file:///x/b"]"#), ("c3", "[]"), ("c4", r#"["https://example.com/x"]"#)] {
            db.execute("INSERT INTO conversation_summaries VALUES (?1, ?2)", [id, uris]).unwrap();
        }
        drop(db);
        let found = own_folders(&path, &["c1", "c2", "c3", "c4", "c5"]);
        // one folder: that one; several, none, another kind of address, or no record: no one folder
        assert_eq!(found.into_iter().collect::<Vec<_>>(), [("c1".to_string(), "/x/my shop".to_string())]);
        assert_eq!(file_path("file://localhost/x/shop").as_deref(), Some("/x/shop"));
        assert_eq!(file_path("file:///"), None);
        assert!(own_folders(&dir.join("none.db"), &["c1"]).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_held_agents_request_to_run_a_command_is_put_as_the_command_itself() {
        let me = Agent { deliver: Some(airband_core::agent::Deliver::Held), ..crate::adapters::agent_with("a1", NAME) };
        // as `agy acp` 1.5.15 asks it
        let request = json!({ "sessionId": "s", "toolCall": { "toolCallId": "call_1", "name": "run_command", "kind": "execute", "title": "echo airband-probe > made.txt", "rawInput": { "CommandLine": "echo airband-probe > made.txt", "WaitMsBeforeAsync": 5000, "toolSummary": "Write airband-probe to made.txt" } }, "options": [{ "optionId": "allow", "kind": "allow_once" }, { "optionId": "deny", "kind": "reject_once" }] });
        let crate::adapters::AcpLeave::Own(Some(ask)) = acp_leave(&request, &me) else { panic!("not read as a command") };
        assert_eq!(ask.what, "run `echo airband-probe > made.txt`");
        assert!(ask.detail.starts_with("run_command\n") && ask.detail.contains("Write airband-probe to made.txt"));
        let write = json!({ "toolCall": { "name": "write_file", "kind": "edit", "title": "File note.txt", "rawInput": { "Content": "hello", "Overwrite": true, "Path": "/x/shop/note.txt" } } });
        let crate::adapters::AcpLeave::Own(Some(ask)) = acp_leave(&write, &me) else { panic!("not read as a file") };
        assert_eq!(ask.what, "write the file /x/shop/note.txt");
        // the row's words for it, with what they need missing: not asked, so not allowed
        assert!(matches!(acp_leave(&json!({ "toolCall": { "name": "run_command", "rawInput": {} } }), &me), crate::adapters::AcpLeave::Own(None)));
        // a file it writes and a page it reads, in the row's words for them (field names as `agy` 1.5.15 has them)
        let crate::adapters::AcpLeave::Own(Some(ask)) = acp_leave(&json!({ "toolCall": { "name": "write_to_file", "title": "File note.txt", "rawInput": { "TargetFile": "/x/shop/note.txt", "CodeContent": "hello" } } }), &me) else { panic!("not read as a file") };
        assert_eq!(ask.what, "write the file /x/shop/note.txt");
        let crate::adapters::AcpLeave::Own(Some(ask)) = acp_leave(&json!({ "toolCall": { "name": "read_url_content", "title": "Read example.com", "rawInput": { "Url": "https://example.com/a" } } }), &me) else { panic!("not read as a page") };
        assert_eq!(ask.what, "fetch https://example.com/a");
        // a tool its row has no words for is put as itself, with exactly what it was given, never by its title
        let crate::adapters::AcpLeave::Own(Some(ask)) = acp_leave(&json!({ "toolCall": { "name": "send_command_input", "title": "Input", "rawInput": { "Input": "y\n" } } }), &me) else { panic!("not put as itself") };
        assert_eq!(ask.what, r#"send_command_input {"Input":"y\n"}"#);
        // one that does not name its tool is not asked: no name is made up for it
        assert!(matches!(acp_leave(&json!({ "toolCall": { "title": "File note.txt", "rawInput": {} } }), &me), crate::adapters::AcpLeave::Own(None)));
    }
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod row;
