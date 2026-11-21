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
        if changed { format!("{name} reports to airband once you allow it: open {name} and type /hooks. (Set in {}.)", file.path.display()) } else { format!("{name} reports to airband (set in {}).", file.path.display()) }
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
    fn a_codex_permission_request_is_put_to_him_whole() {
        let ask = permission(&json!({ "session_id": "t1", "cwd": "/x", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "npm run deploy" } })).unwrap();
        assert_eq!(ask.what, "run `npm run deploy`");
        assert!(super::super::Harness::permission_answer(&Codex, &ask, true, "").contains("\"behavior\":\"allow\""));
        assert!(super::super::Harness::permission_answer(&Codex, &ask, false, "not now").contains("not now"));
        assert!(permission(&json!({ "session_id": "t1", "hook_event_name": "Stop" })).is_none());
    }
}
