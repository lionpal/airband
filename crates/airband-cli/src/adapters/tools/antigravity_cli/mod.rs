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
}

/// The most of a transcript read from its end: its last turn is in its last part.
const TAIL: u64 = 8 << 20;

/// The steps at the end of a transcript, newest first. A line that does not read as a step is passed over.
fn last_steps(path: &Path) -> Vec<Step> {
    let Ok(size) = airband_io::files::metadata(path).map(|m| m.len()) else { return Vec::new() };
    let from = size.saturating_sub(TAIL);
    let Ok(bytes) = airband_io::files::read_part(path, from, TAIL) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    // a part that starts in the middle of a line leaves that line out
    let lines = text.lines().skip(usize::from(from > 0));
    let mut steps: Vec<Step> = lines.filter_map(|l| serde_json::from_str(l).ok()).collect();
    steps.reverse();
    steps
}

/// The last thing it said since his last prompt, from its transcript.
fn last_answer(path: &Path) -> Option<String> {
    last_steps(path).into_iter().take_while(|s| s.kind != StepType::UserInput).find_map(|s| s.answer())
}

/// Its transcript says it is waiting for him: its last step is a finished answer that calls no tool.
fn waiting(path: &Path) -> bool {
    last_steps(path).first().is_some_and(|s| s.status == Status::Done && s.tool_calls.is_empty() && s.answer().is_some())
}

// ---- what it says is open, and where it has worked ----

/// The folder each conversation was last had in, by its own record of the latest in each folder. A conversation
/// that is not the latest anywhere has no folder by that record.
fn folders() -> Vec<(String, String)> {
    let path = state().join(crate::adapters::row_says(NAME, "last_conversations"));
    let read: Option<std::collections::BTreeMap<String, String>> = airband_io::files::read_bytes(&path).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok());
    read.unwrap_or_default().into_iter().filter(|(folder, id)| !folder.is_empty() && !id.is_empty()).collect()
}

fn transcript_of(id: &str) -> PathBuf {
    state().join(crate::adapters::row_says(NAME, "brain")).join(id).join(crate::adapters::row_says(NAME, "transcript"))
}

/// The conversations open now: each holds a lock on its file in `presence/` for as long as it is open. Nothing
/// when that folder cannot be read (it is not on this computer).
fn open() -> Option<Vec<crate::adapters::Open>> {
    let dir = state().join(crate::adapters::row_says(NAME, "presence"));
    if !dir.is_dir() {
        return None;
    }
    let files = airband_io::files::entries(&dir).ok()?;
    let folders = folders();
    let mut open = Vec::new();
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
        let cwd = folders.iter().find(|(_, last)| last == id).map(|(folder, _)| folder.clone());
        open.push(crate::adapters::Open { agent: agent(id, cwd, None), idle: waiting(&transcript_of(id)) });
    }
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
/// that tool was given (`rawInput`), in its row's words for that tool, whole. A tool the row has no words for is
/// read the protocol's way, by Antigravity's own title for the call. One whose row words it does not fill (a
/// field missing) says nothing he could answer, so it is not asked.
fn acp_leave(request: &Value, agent: &Agent) -> crate::adapters::AcpLeave {
    let call = &request["toolCall"];
    let Some(tool) = call["name"].as_str() else { return crate::adapters::AcpLeave::Protocol };
    let words = &crate::adapters::hooks::tool_use(NAME, tool)["asks"];
    if words.is_null() {
        return crate::adapters::AcpLeave::Protocol;
    }
    let input = &call["rawInput"];
    crate::adapters::AcpLeave::Own(crate::adapters::hooks::fill(words, input).and_then(|what| crate::adapters::hooks::leave(agent.clone(), tool, what, input, "", None)).map(Box::new))
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
        // a tool its row has no words for is read the protocol's way
        assert!(matches!(acp_leave(&json!({ "toolCall": { "name": "read_url_content", "title": "Read example.com", "rawInput": { "Url": "https://example.com" } } }), &me), crate::adapters::AcpLeave::Protocol));
    }
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod row;
