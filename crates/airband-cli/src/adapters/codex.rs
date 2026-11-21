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
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::fs;
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
        // What he says is handed to this program (`airband deliver codex <session>`), which queues it for the
        // session with Codex's own command. The session takes it as a turn of his.
        deliver: if env("AIRBAND_HELD").is_some() { Some(Deliver::Held) } else { Some(Deliver::Command { argv: vec![std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "airband".into()), "deliver".into(), NAME.into(), id.clone()] }) },
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
    // too long to put in front of him whole on a phone: it is not asked there, and Codex asks at its keyboard
    if what.chars().count() > super::claude_code::SHOWN_WHOLE {
        return None;
    }
    let whole = serde_json::to_string_pretty(input).unwrap_or_default();
    let detail = match whole.chars().count() {
        n if n > 6000 => format!("{}\n\n(The first 6,000 of {n} characters. The rest is at the keyboard.)", whole.chars().take(6000).collect::<String>()),
        _ => whole,
    };
    Some(PermissionAsk { schema: SCHEMA.into(), agent: agent_of(hook)?, what, detail: format!("{tool}\n{detail}"), question: None })
}

/// His answer, as Codex wants it printed: the same shape Claude Code takes.
pub fn permission_answer(_ask: &PermissionAsk, allow: bool, reason: &str) -> String {
    let decision = if allow {
        json!({ "behavior": "allow" })
    } else {
        json!({ "behavior": "deny", "message": if reason.is_empty() { "He said no from his phone.".to_string() } else { format!("He said no from his phone: {reason}") } })
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

/// `airband deliver codex <session>`: his words, on standard input, into that session as a turn of his own.
pub fn deliver(session: &str) -> Result<(), String> {
    let mut words = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut words).map_err(|e| e.to_string())?;
    let done = std::process::Command::new("codex").args(["queue", "--thread", session, "--resume", "--message", words.trim()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).output().map_err(|e| format!("Codex could not be started: {e}"))?;
    if done.status.success() { Ok(()) } else { Err(format!("Codex did not take it: {}", String::from_utf8_lossy(&done.stderr).trim().chars().take(200).collect::<String>())) }
}

// ---- setting Codex up to report: its hooks, in its own hooks file ----

const HOOK_EVENTS: [(&str, u32); 7] = [("SessionStart", 5), ("UserPromptSubmit", 5), ("Stop", 5), ("SubagentStart", 5), ("SubagentStop", 5), ("SessionEnd", 2), ("PermissionRequest", 900)];

fn home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".codex"))
}
fn hooks_path() -> PathBuf {
    home().join("hooks.json")
}
fn read() -> Result<Value, String> {
    match fs::read_to_string(hooks_path()) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{} is not valid JSON; fix it first ({e})", hooks_path().display())),
        Err(_) => Ok(json!({})),
    }
}
fn write(v: &Value) -> Result<(), String> {
    let (path, tmp) = (hooks_path(), hooks_path().with_extension("json.airband-tmp"));
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(v).unwrap())).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
/// A hook of ours: the `airband` binary, wherever it is installed, run as `hook codex`.
fn is_ours(hook: &Value) -> bool {
    hook["command"].as_str().and_then(|c| c.trim_end().strip_suffix(" hook codex")).is_some_and(|program| program.trim().trim_matches('"').rsplit(['/', '\\']).next() == Some("airband"))
}
/// Take our hooks out, leaving everyone else's exactly as they were.
fn strip(file: &mut Value) {
    let Some(hooks) = file.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut() {
        if let Some(list) = groups.as_array_mut() {
            for group in list.iter_mut() {
                if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    inner.retain(|h| !is_ours(h));
                }
            }
            list.retain(|g| g["hooks"].as_array().is_none_or(|h| !h.is_empty()));
        }
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|l| !l.is_empty()));
}

/// Write the hooks, when Codex is on this computer. Codex will not run them until he has trusted them.
pub fn install(exe: &std::path::Path) -> Result<String, String> {
    if !home().is_dir() {
        return Ok("Codex is not on this computer: nothing to set up for it.".into());
    }
    let mut file = read()?;
    let before = file.clone();
    strip(&mut file);
    if !file.is_object() {
        return Err(format!("{} is not a JSON object", hooks_path().display()));
    }
    let command = format!("\"{}\" hook codex", exe.display());
    let hooks = file.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({}));
    for (event, timeout) in HOOK_EVENTS {
        let list = hooks.as_object_mut().ok_or("\"hooks\" in Codex's hooks file is not an object")?.entry(event).or_insert_with(|| json!([]));
        list.as_array_mut().ok_or("a hooks entry in Codex's hooks file is not a list")?.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": timeout }] }));
    }
    // unchanged: Codex's trust in them still stands, and the file is left exactly as it is
    if file == before {
        return Ok(format!("Codex's hooks are in place ({}).", hooks_path().display()));
    }
    write(&file)?;
    Ok(format!("Hooks added to {}. Codex runs them once you have trusted them: open Codex and type /hooks.", hooks_path().display()))
}

pub fn uninstall() -> Result<(), String> {
    if !hooks_path().is_file() {
        return Ok(());
    }
    let mut file = read()?;
    strip(&mut file);
    write(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codex_session_checks_in_speaks_and_is_reached_as_his_own_turn() {
        let hook = json!({ "session_id": "t1", "cwd": "/x/shop", "hook_event_name": "Stop", "model": "gpt-6-luna", "last_assistant_message": " Done. " });
        let e = events(&hook).remove(0);
        assert_eq!((e.agent.id.as_str(), e.agent.harness.as_str()), ("t1", "codex"));
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None });
        assert_eq!(glance(&e.agent).model.as_deref(), Some("gpt-6-luna"));
        // his words go to Codex's own queue for that session, whole, as one argument
        match e.agent.deliver {
            Some(Deliver::Command { argv }) => assert_eq!(&argv[1..], ["deliver", "codex", "t1"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(events(&json!({ "session_id": "t1", "hook_event_name": "SessionEnd" }))[0].event, Event::Signoff);
    }

    #[test]
    fn a_codex_permission_request_is_put_to_him_whole() {
        let ask = permission(&json!({ "session_id": "t1", "cwd": "/x", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "npm run deploy" } })).unwrap();
        assert_eq!(ask.what, "run `npm run deploy`");
        assert!(permission_answer(&ask, true, "").contains("\"behavior\":\"allow\""));
        assert!(permission_answer(&ask, false, "not now").contains("not now"));
        assert!(permission(&json!({ "session_id": "t1", "hook_event_name": "Stop" })).is_none());
    }
}
