//! The Claude Code adapter. Claude Code runs `airband hook claude-code` from its own hooks (SessionStart, Stop,
//! Notification, UserPromptSubmit, SessionEnd, and PostToolUse for the tools that touch pictures) with the
//! hook's JSON on standard input.
use super::env;
use airband_core::agent::is_picture;
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// The name he gave this session, if he gave one: with `/rename`, or as the name of its remote control
/// (`/remote-control shop 2`). A title Claude Code made up by itself is not used: those go stale.
fn session_name(hook: &Value) -> Option<String> {
    let home = std::path::PathBuf::from(env("HOME")?);
    // Claude Code keeps one small file per running session, by process
    let renamed = env("CLAUDE_PID")
        .and_then(|pid| std::fs::read_to_string(home.join(".claude/sessions").join(format!("{pid}.json"))).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|s| s["nameSource"].as_str() == Some("user") && s["sessionId"].as_str() == hook["session_id"].as_str())
        .and_then(|s| s["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from));
    renamed.or_else(|| remote_control_name(hook["transcript_path"].as_str()?, hook["session_id"].as_str()?))
}

/// The name last given to `/remote-control` in this session. It is only in the transcript, which can be tens of
/// megabytes, so the place reached and the name found are kept beside the tower's files and each call reads
/// only what has been written since.
fn remote_control_name(transcript: &str, session: &str) -> Option<String> {
    use std::io::{BufRead, Seek, SeekFrom};
    if !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let note = crate::config::dir().join("claude-code").join(format!("{session}.json"));
    let kept: Value = std::fs::read_to_string(&note).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let (mut at, mut name) = (kept["at"].as_u64().unwrap_or(0), kept["name"].as_str().map(String::from));
    let mut file = std::fs::File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    if len < at {
        at = 0; // the transcript was replaced
    }
    if len > at {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line).unwrap_or(0);
            // a line still being written is left for next time
            if n == 0 || line.last() != Some(&b'\n') {
                break;
            }
            at += n as u64;
            if let Some(found) = remote_control_line(&line) {
                name = Some(found);
            }
        }
        let _ = std::fs::create_dir_all(note.parent()?);
        let _ = std::fs::write(&note, serde_json::json!({ "at": at, "name": name }).to_string());
    }
    name
}

/// The name in one transcript line, if that line is him running `/remote-control <name>`. A line that only
/// quotes such a command (a tool's output, a pasted log) is not one.
fn remote_control_line(line: &[u8]) -> Option<String> {
    const MARK: &[u8] = b"/remote-control</command-name>";
    if !line.windows(MARK.len()).any(|w| w == MARK) {
        return None;
    }
    let entry: Value = serde_json::from_slice(line).ok()?;
    if entry["type"].as_str() != Some("system") || entry["subtype"].as_str() != Some("local_command") {
        return None;
    }
    let content = entry["content"].as_str()?;
    let args = content.split_once("<command-args>")?.1.split_once("</command-args>")?.0.trim();
    (!args.is_empty() && args.len() <= 60).then(|| args.to_string())
}

/// Who a notification is about: the session, where it works, how to reach it, and the name he gave it.
fn agent_of(hook: &Value) -> Option<Agent> {
    let text = |k: &str| hook[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    Some(Agent {
        id: text("session_id")?,
        harness: "claude-code".into(),
        cwd: text("cwd"),
        name: env("AIRBAND_NAME"),
        title: session_name(hook),
        pid: env("CLAUDE_PID").and_then(|p| p.parse().ok()),
        deliver: env("CLAUDE_CODE_MESSAGING_SOCKET").map(|s| Deliver::ClaudeCodeInbox { socket: s.trim_start_matches("uds:").to_string(), token: env("CLAUDE_CODE_MESSAGING_TOKEN") }),
        // where its transcript is, so the tower can ask what it is doing without a hook on every step
        meta: text("transcript_path").map(|t| serde_json::Map::from_iter([("transcript".to_string(), Value::String(t))])).unwrap_or_default(),
    })
}

/// What a session is doing right now: its own words for its latest step, from the end of its transcript.
/// A command or a subagent comes with a description Claude wrote for it ("Run the payments tests"); for the
/// rest, a few plain words. Reads at most the last 96 KB of the file, so it is quick however long the session.
pub fn activity(agent: &Agent) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(agent.meta.get("transcript")?.as_str()?).ok()?;
    let size = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(96 * 1024))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    step_in(&String::from_utf8_lossy(&tail))
}

/// The latest step named in a piece of a transcript, newest line first.
fn step_in(tail: &str) -> Option<String> {
    tail.lines().rev().filter(|l| l.contains("\"tool_use\"")).find_map(|line| {
        let entry: Value = serde_json::from_str(line).ok()?;
        entry["message"]["content"].as_array()?.iter().rev().filter(|c| c["type"] == "tool_use").find_map(|c| step(c["name"].as_str()?, &c["input"]))
    })
}

fn step(tool: &str, input: &Value) -> Option<String> {
    let text = |k: &str| input[k].as_str().map(str::trim).filter(|t| !t.is_empty());
    let file = |k: &str| text(k).map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string());
    let said = match tool {
        // bookkeeping of its own, not a step in the work: keep looking further back
        "TodoWrite" | "ToolSearch" | "TaskCreate" | "TaskUpdate" | "TaskList" | "ExitPlanMode" => return None,
        _ if text("description").is_some() => text("description")?.to_string(),
        "Read" => format!("Reading {}", file("file_path")?),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", file("file_path").or_else(|| file("notebook_path"))?),
        "Grep" => format!("Searching for {}", text("pattern")?),
        "Glob" => "Looking for files".into(),
        "WebSearch" => format!("Searching the web for {}", text("query")?),
        "WebFetch" => format!("Reading {}", text("url")?.split('/').nth(2).unwrap_or("a web page")),
        "Bash" => text("command")?.lines().next()?.to_string(),
        // a tool from a server: its own name, as words ("take screenshot")
        other => other.rsplit("__").next().unwrap_or(other).replace(['_', '-'], " "),
    };
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if said.chars().count() > 64 { format!("{}…", said.chars().take(63).collect::<String>().trim_end()) } else { said }).filter(|s| !s.is_empty())
}

/// Claude Code is about to show a permission prompt (its `PermissionRequest` hook): the request to put to him.
/// What he reads is the thing itself, not a description of it: the command, the file, the address.
pub fn permission(hook: &Value) -> Option<PermissionAsk> {
    if hook["hook_event_name"].as_str() != Some("PermissionRequest") {
        return None;
    }
    // scripts calling `claude -p` have nobody waiting on them
    if env("CLAUDE_CODE_ENTRYPOINT").as_deref() == Some("sdk-cli") && env("AIRBAND_INCLUDE_HEADLESS").is_none() {
        return None;
    }
    let tool = hook["tool_name"].as_str().unwrap_or("a tool");
    let input = &hook["tool_input"];
    let field = |k: &str| input[k].as_str().map(|v| v.trim().chars().take(240).collect::<String>()).filter(|v| !v.is_empty());
    let what = match tool {
        "Bash" => field("command").map(|c| format!("run `{c}`")),
        "Write" => field("file_path").map(|f| format!("write the file {f}")),
        "Edit" | "MultiEdit" | "NotebookEdit" => field("file_path").or(field("notebook_path")).map(|f| format!("edit the file {f}")),
        "Read" => field("file_path").map(|f| format!("read the file {f}")),
        "WebFetch" => field("url").map(|u| format!("fetch {u}")),
        "WebSearch" => field("query").map(|q| format!("search the web for \"{q}\"")),
        _ => None,
    }
    .unwrap_or_else(|| format!("use {tool}"));
    let whole = serde_json::to_string_pretty(input).unwrap_or_default();
    Some(PermissionAsk { schema: SCHEMA.into(), agent: agent_of(hook)?, what, detail: format!("{tool}\n{}", whole.chars().take(6000).collect::<String>()) })
}

/// His answer, as Claude Code wants it printed. A refusal carries his reason, which Claude is shown.
pub fn permission_answer(allow: bool, reason: &str) -> String {
    let decision = if allow {
        json!({ "behavior": "allow" })
    } else {
        json!({ "behavior": "deny", "message": if reason.is_empty() { "He said no from his phone.".to_string() } else { format!("He said no from his phone: {reason}") } })
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

/// Is this tool call Claude Code starting a subagent? The tool has been called both names.
fn is_subagent(hook: &Value) -> bool {
    matches!(hook["tool_name"].as_str(), Some("Agent" | "Task"))
}

/// Every picture file named in a tool's input, whatever the tool calls the field.
fn pictures_in(input: &Value, cwd: Option<&str>, out: &mut Vec<String>) {
    match input {
        Value::String(s) if is_picture(s) => {
            let path = std::path::Path::new(s);
            let path = if path.is_absolute() { path.to_path_buf() } else { std::path::Path::new(cwd.unwrap_or(".")).join(path) };
            if path.is_file() {
                out.push(path.display().to_string());
            }
        }
        Value::Array(list) => list.iter().for_each(|v| pictures_in(v, cwd, out)),
        Value::Object(map) => map.values().for_each(|v| pictures_in(v, cwd, out)),
        _ => {}
    }
}

/// Claude Code: installed for SessionStart, Stop, Notification, UserPromptSubmit and SessionEnd, and after the
/// tools that look at or hand over a picture.
pub fn events(hook: &Value) -> Vec<AgentEvent> {
    // scripts calling `claude -p` have nobody waiting on them
    if env("CLAUDE_CODE_ENTRYPOINT").as_deref() == Some("sdk-cli") && env("AIRBAND_INCLUDE_HEADLESS").is_none() {
        return Vec::new();
    }
    let text = |k: &str| hook[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    let events = match hook["hook_event_name"].as_str().unwrap_or("") {
        "SessionStart" => vec![Event::Checkin],
        "UserPromptSubmit" => vec![Event::Working],
        "Stop" => text("last_assistant_message").map(|text| Event::Said { text, kind: None }).into_iter().collect(),
        // "waiting for your input" only repeats the Stop that came just before it
        "Notification" => match text("message") {
            Some(message) if hook["notification_type"].as_str() != Some("idle_prompt") && !message.to_lowercase().contains("waiting for your input") => vec![Event::Notice { text: message }],
            _ => Vec::new(),
        },
        // it is handing part of the work to a subagent
        "PreToolUse" if is_subagent(hook) => vec![Event::Subagent { done: false, what: hook["tool_input"]["description"].as_str().unwrap_or("").to_string() }],
        "PostToolUse" if is_subagent(hook) => vec![Event::Subagent { done: true, what: hook["tool_input"]["description"].as_str().unwrap_or("").to_string() }],
        // it read a picture, took a screenshot, or sent him a file
        "PostToolUse" => {
            let mut paths = Vec::new();
            pictures_in(&hook["tool_input"], hook["cwd"].as_str(), &mut paths);
            let shown = hook["tool_name"].as_str() == Some("SendUserFile");
            paths.into_iter().map(|path| Event::Picture { path, shown }).collect()
        }
        "SessionEnd" => vec![Event::Signoff],
        _ => Vec::new(),
    };
    let Some(agent) = agent_of(hook) else { return Vec::new() };
    events.into_iter().map(|event| AgentEvent { schema: SCHEMA.into(), agent: agent.clone(), event }).collect()
}

// ---- setting Claude Code up to report: its hooks, in its own settings file ----

/// The Claude Code tools worth hearing about once they have run: the ones that look at a picture, make one, or
/// hand him one, and the one that runs a subagent.
const AFTER_TOOLS: &str = "Read|SendUserFile|Agent|Task|mcp__.*screenshot.*";
/// The ones worth hearing about as they start: a subagent.
const BEFORE_TOOLS: &str = "Agent|Task";
const HOOK_EVENTS: [&str; 5] = ["SessionStart", "Stop", "Notification", "UserPromptSubmit", "SessionEnd"];

fn settings_path() -> PathBuf {
    crate::config::home().join(".claude").join("settings.json")
}
fn read_settings() -> Result<Value, String> {
    let path = settings_path();
    match fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{} is not valid JSON; fix it first ({e})", path.display())),
        Err(_) => Ok(json!({})),
    }
}
fn write_settings(v: &Value) -> Result<(), String> {
    let path = settings_path();
    let tmp = path.with_extension("json.airband-tmp");
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(v).unwrap())).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
fn is_ours(hook: &Value) -> bool {
    hook["command"].as_str().is_some_and(|c| c.contains(".airband") && c.trim_end().ends_with(" hook claude-code"))
}
/// Take our hooks out, leaving everyone else's exactly as they were.
fn strip_hooks(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut() {
        if let Some(list) = groups.as_array_mut() {
            for g in list.iter_mut() {
                if let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) {
                    hs.retain(|h| !is_ours(h));
                }
            }
            list.retain(|g| g["hooks"].as_array().is_none_or(|h| !h.is_empty()));
        }
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|l| !l.is_empty()));
    if hooks.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
    }
}

/// Add this adapter's hooks to Claude Code's settings, replacing any from an earlier install and leaving
/// everyone else's as they were. Returns what to tell him.
pub fn install(exe: &std::path::Path) -> Result<String, String> {
    let mut settings = read_settings()?;
    strip_hooks(&mut settings);
    if !settings.is_object() {
        return Err(format!("{} is not a JSON object", settings_path().display()));
    }
    let command = format!("\"{}\" hook claude-code", exe.display());
    let hooks = settings.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({}));
    for ev in HOOK_EVENTS {
        let list = hooks.as_object_mut().ok_or("\"hooks\" in settings.json is not an object")?.entry(ev).or_insert_with(|| json!([]));
        list.as_array_mut().ok_or("a hooks entry in settings.json is not a list")?.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": 5 }] }));
    }
    // pictures: after the tools that read one, take one, or send him one
    let list = hooks.as_object_mut().unwrap().entry("PostToolUse").or_insert_with(|| json!([]));
    list.as_array_mut().ok_or("a hooks entry in settings.json is not a list")?.push(json!({ "matcher": AFTER_TOOLS, "hooks": [{ "type": "command", "command": command, "timeout": 5 }] }));
    let list = hooks.as_object_mut().unwrap().entry("PreToolUse").or_insert_with(|| json!([]));
    list.as_array_mut().ok_or("a hooks entry in settings.json is not a list")?.push(json!({ "matcher": BEFORE_TOOLS, "hooks": [{ "type": "command", "command": command, "timeout": 5 }] }));
    // permission: asked on the radio first. The time allowed is long because he is the one being waited for; the
    // tower says how long to wait and the hook gives up on its own well before this.
    let list = hooks.as_object_mut().unwrap().entry("PermissionRequest").or_insert_with(|| json!([]));
    list.as_array_mut().ok_or("a hooks entry in settings.json is not a list")?.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": 900 }] }));
    write_settings(&settings)?;
    Ok(format!("Hooks added to {}.", settings_path().display()))
}

pub fn uninstall() -> Result<(), String> {
    let mut settings = read_settings()?;
    strip_hooks(&mut settings);
    write_settings(&settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_stop_becomes_said() {
        let e = events(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "Stop", "last_assistant_message": " Done. " })).remove(0);
        assert_eq!(e.agent.id, "s1");
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None });
    }
    #[test]
    fn claude_code_idle_notification_is_dropped() {
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "message": "Claude is waiting for your input" })).is_empty());
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Stop", "last_assistant_message": "" })).is_empty());
    }
    #[test]
    fn the_remote_control_name_is_read_only_from_his_own_command() {
        let ran = json!({ "type": "system", "subtype": "local_command", "content": "<command-name>/remote-control</command-name>\n <command-message>remote-control</command-message>\n <command-args>shop 2</command-args>" });
        assert_eq!(remote_control_line(ran.to_string().as_bytes()), Some("shop 2".into()));
        let bare = json!({ "type": "system", "subtype": "local_command", "content": "<command-name>/remote-control</command-name>\n <command-args></command-args>" });
        assert_eq!(remote_control_line(bare.to_string().as_bytes()), None);
        // the same text quoted inside a tool's output is not him naming the session
        let quoted = json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": ran.to_string() }] } });
        assert_eq!(remote_control_line(quoted.to_string().as_bytes()), None);
    }
    #[test]
    fn a_permission_prompt_becomes_a_request_that_says_exactly_what_will_happen() {
        let ask = |tool: &str, input: Value| permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input })).unwrap().what;
        assert_eq!(ask("Bash", json!({ "command": "npm run deploy", "description": "Deploy" })), "run `npm run deploy`");
        assert_eq!(ask("Write", json!({ "file_path": "/x/shop/.env", "content": "…" })), "write the file /x/shop/.env");
        assert_eq!(ask("WebFetch", json!({ "url": "https://example.com/x" })), "fetch https://example.com/x");
        assert_eq!(ask("mcp__db__drop", json!({ "table": "orders" })), "use mcp__db__drop");
        // any other notification is not a request
        assert!(permission(&json!({ "session_id": "s1", "hook_event_name": "Stop" })).is_none());
        // the answers, as Claude Code reads them
        let yes: Value = serde_json::from_str(&permission_answer(true, "")).unwrap();
        assert_eq!(yes["hookSpecificOutput"]["decision"], json!({ "behavior": "allow" }));
        let no: Value = serde_json::from_str(&permission_answer(false, "use staging")).unwrap();
        assert_eq!(no["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(no["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().ends_with("use staging"));
    }
    #[test]
    fn subagents_starting_and_finishing_are_reported() {
        let call = |when: &str| events(&json!({ "session_id": "s1", "hook_event_name": when, "tool_name": "Agent", "tool_input": { "description": "map the columns", "prompt": "…" } })).remove(0).event;
        assert_eq!(call("PreToolUse"), Event::Subagent { done: false, what: "map the columns".into() });
        assert_eq!(call("PostToolUse"), Event::Subagent { done: true, what: "map the columns".into() });
    }
    #[test]
    fn a_picture_a_tool_touched_is_reported() {
        let dir = std::env::temp_dir().join(format!("airband-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.PNG"), b"x").unwrap();
        let cwd = dir.display().to_string();
        let looked = events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "shot.PNG" } }));
        assert_eq!(looked[0].event, Event::Picture { path: dir.join("shot.PNG").display().to_string(), shown: false });
        let sent = events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "SendUserFile", "tool_input": { "files": [dir.join("shot.PNG"), "notes.md", "gone.png"] } }));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].event, Event::Picture { path: dir.join("shot.PNG").display().to_string(), shown: true });
        assert!(events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "main.rs" } })).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn strip_removes_only_ours() {
        let mut s = json!({ "model": "x", "hooks": {
            "Stop": [
                { "hooks": [{ "type": "command", "command": "\"/Users/c/.airband/bin/airband\" hook claude-code" }] },
                { "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }
            ],
            "SessionStart": [{ "hooks": [{ "type": "command", "command": "\"/Users/c/.airband/bin/airband\" hook claude-code" }] }]
        }});
        strip_hooks(&mut s);
        assert_eq!(s, json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } }));
        let mut only = json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "\"C:\\Users\\c\\.airband\\bin\\airband.exe\" hook claude-code" }] }] } });
        strip_hooks(&mut only);
        assert_eq!(only, json!({ "model": "x" }));
    }

    #[test]
    fn the_latest_step_is_read_from_the_end_of_a_transcript() {
        let line = |content: &str| format!(r#"{{"type":"assistant","message":{{"role":"assistant","content":{content}}}}}"#);
        let transcript = [
            line(r#"[{"type":"tool_use","name":"Read","input":{"file_path":"/x/orders/payments/webhook.ts"}}]"#),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"…"}]}}"#.to_string(),
            line(r#"[{"type":"text","text":"Now the tests."},{"type":"tool_use","name":"Bash","input":{"command":"npm test -- payments","description":"Run the payments tests"}}]"#),
            line(r#"[{"type":"tool_use","name":"TodoWrite","input":{"todos":[]}}]"#),
            r#"{"type":"user","message":{"content":"it mentions \"tool_use\" in passing"}}"#.to_string(),
        ]
        .join("\n");
        // the bookkeeping after it is passed over, and so is a line that only mentions the words
        assert_eq!(step_in(&transcript).as_deref(), Some("Run the payments tests"));
        let s = |tool: &str, input: Value| step(tool, &input);
        assert_eq!(s("Read", json!({ "file_path": "/x/a/webhook.ts" })).as_deref(), Some("Reading webhook.ts"));
        assert_eq!(s("Edit", json!({ "file_path": "C:\\code\\refund.ts" })).as_deref(), Some("Editing refund.ts"));
        assert_eq!(s("Grep", json!({ "pattern": "charge_id" })).as_deref(), Some("Searching for charge_id"));
        assert_eq!(s("WebFetch", json!({ "url": "https://docs.stripe.com/refunds" })).as_deref(), Some("Reading docs.stripe.com"));
        assert_eq!(s("Bash", json!({ "command": "cargo test\n" })).as_deref(), Some("cargo test"));
        assert_eq!(s("mcp__chrome-devtools__take_screenshot", json!({})).as_deref(), Some("take screenshot"));
        assert_eq!(s("Agent", json!({ "description": "Map the Northway columns", "prompt": "…" })).as_deref(), Some("Map the Northway columns"));
        assert_eq!(s("Bash", json!({ "command": "x".repeat(200) })).map(|t| t.chars().count()), Some(64));
        assert_eq!(s("TodoWrite", json!({})), None);
        assert_eq!(step_in("nothing here"), None);
    }
}
