//! The Claude Code adapter. Claude Code runs `airband hook claude-code` from its own hooks (`HOOKS`: the
//! session's start and end, each prompt and stop, its notifications and permission requests, and the tools its
//! row gives a role, before and after they run) with the hook's JSON on standard input.
/// Claude Code's id: its row's, and the harness its agents report under.
pub(crate) const HARNESS: &str = "claude-code";
use crate::adapters::env;
use crate::adapters::hooks::{On, on, text};
use airband_core::agent::{Agent, AgentEvent, Deliver, Event, PermissionAsk};
use serde::Deserialize;
use serde_json::Value;

pub(crate) mod accounts;
mod commands;
mod sessions;
mod transcript;
use sessions::*;
use transcript::*;

/// The value of the variable the row names for `what`; nothing when the row names none.
fn row_env(what: &str) -> Option<String> {
    Some(crate::adapters::row_says(HARNESS, what)).filter(|name| !name.is_empty()).and_then(|name| env(&name))
}

/// The name he gave this session, if he gave one: with `/rename`, or as the name of its remote control
/// (`/remote-control shop 2`). A title Claude Code made up by itself is not used: those go stale.
fn session_name(hook: &Value) -> Option<String> {
    // Claude Code keeps one small file per running session, by process
    let renamed = row_env("pid_env")
        .and_then(|pid| record(&claude_home(), &pid))
        .filter(|s| s["sessionId"].as_str() == hook["session_id"].as_str())
        .and_then(|s| named_by_him(&s));
    renamed.or_else(|| remote_control_name(hook["transcript_path"].as_str()?, hook["session_id"].as_str()?))
}

/// The name in a session's record when he gave it; one Claude Code made up by itself is not used. The one place
/// a record's name is read.
fn named_by_him(record: &Value) -> Option<String> {
    (record["nameSource"].as_str() == Some("user")).then(|| record["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from)).flatten()
}

/// Who a notification is about: the session, where it works, how to reach it, and the name he gave it.
fn agent_of(hook: &Value) -> Option<Agent> {
    Some(Agent {
        cwd: text(hook, "cwd"),
        name: env("AIRBAND_NAME"),
        // one the tower holds keeps the name it has (`crate::adapters::as_held`)
        title: session_name(hook),
        pid: row_env("pid_env").and_then(|p| p.parse().ok()),
        // one the tower started and holds is reached through the input it holds, and no other way (`crate::adapters::as_held`)
        deliver: row_env("inbox_env").and_then(|s| s.strip_prefix(crate::adapters::row_says(HARNESS, "inbox_scheme").as_str()).map(|path| Deliver::Socket { path: path.to_string(), token: row_env("inbox_token_env") })),
        // where its transcript is, so the tower can ask what it is doing without a hook on every step
        meta: text(hook, "transcript_path").map(|t| serde_json::Map::from_iter([("transcript".to_string(), Value::String(t))])).unwrap_or_default(),
        ..crate::adapters::agent_with(text(hook, "session_id")?, HARNESS)
    })
}

/// Claude Code is about to show a permission prompt (its `PermissionRequest` hook): the request to put to him.
/// What he reads is the thing itself, not a description of it: the command, the file, the address.
pub(crate) fn permission(hook: &Value) -> Option<PermissionAsk> {
    let (tool, input) = crate::adapters::hooks::permission_hook(hook).filter(|_| !headless())?;
    // whole, never cut: what he reads is what will run
    let row = crate::adapters::hooks::tool_use(HARNESS, tool);
    let what = crate::adapters::hooks::asking(HARNESS, tool, input, None);
    // Its tool for asking him something is not a thing to allow: it is a question, put to him as one. One
    // question comes with the answers it offered; several are put together, for him to answer in his own words.
    let question = (row["role"].as_str() == Some("question")).then(|| {
        let asked: Vec<&Value> = input["questions"].as_array().into_iter().flatten().collect();
        let text = asked.iter().filter_map(|q| q["question"].as_str()).map(str::trim).collect::<Vec<_>>().join("\n\n");
        let choices = match asked.as_slice() {
            [one] => one["options"].as_array().into_iter().flatten().filter_map(|o| o["label"].as_str()).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
            _ => Vec::new(),
        };
        airband_core::agent::Asked { text, choices }
    })
    .filter(|q| !q.text.is_empty());
    // one too long to put in front of him whole is not asked on the phone: Claude Code asks at its keyboard
    let ask = crate::adapters::hooks::leave(agent_of(hook)?, tool, what, input, crate::adapters::hooks::AT_KEYBOARD, question)?;
    // where the prompt is seen settled at the keyboard, or by Claude Code itself
    Some(PermissionAsk { settled_in: call_record(hook), ..ask })
}

/// Where Claude Code writes down how the call a PermissionRequest hook is for comes out: the transcript of the
/// agent that made the call, the main one or a subagent's of its own. Nothing when there is no transcript, or a
/// subagent's id that is no file's name.
fn call_record(hook: &Value) -> Option<airband_core::agent::CallRecord> {
    let (tool, input) = crate::adapters::hooks::permission_hook(hook)?;
    let main = std::path::PathBuf::from(text(hook, "transcript_path")?);
    let path = match text(hook, "agent_id") {
        Some(agent) if file_safe(&agent) => main.with_extension("").join(crate::adapters::row_says(HARNESS, "subagent_transcript").replace("{agent}", &agent)),
        Some(_) => return None,
        None => main,
    };
    Some(airband_core::agent::CallRecord { record: path.display().to_string(), tool: tool.to_string(), input: input.clone() })
}

/// Who decided a call Claude Code asked leave for, as the transcript line with its result says
/// (`permissionDecision`).
#[derive(serde::Deserialize)]
struct Decided {
    decision: Option<Verdict>,
    source: Option<DecidedBy>,
}
#[derive(serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Reject,
    #[serde(other)]
    Other,
}
#[derive(serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DecidedBy {
    /// By its own rules, nobody's answer: what it writes when it blocks a request at the end of its countdown.
    Config,
    #[serde(other)]
    Other,
}

/// How the call was settled, from the transcript line that holds its result.
fn settled_how(line: &Value) -> crate::adapters::Settled {
    match Decided::deserialize(&line["permissionDecision"]) {
        Ok(Decided { decision: Some(Verdict::Reject), source: Some(DecidedBy::Config) }) => crate::adapters::Settled::RefusedByTool,
        _ => crate::adapters::Settled::There,
    }
}

/// Is the prompt a request is for still waiting? Claude Code shows its own prompt while the hook waits, and
/// settles it there: answered at its keyboard, or refused by Claude Code itself (it blocks a request once its
/// countdown is up, and ends the hook that was asking with no word). Either way it writes the call's result into
/// the transcript of the agent that made the call, saying who decided it. Once a result for the call is written,
/// the prompt is over. Watched by the tower, so how the hook ended does not matter.
///
/// The record does not say which call it is (the hook's input has no call id), only the tool and its input. At
/// the first look it is the one call with this tool and input that has no result yet; when there is none, it is
/// the next such call written (the call is written about when the hook starts, sometimes just after). When two
/// are open with the same tool and input, which one is asked about cannot be known: then nothing is watched,
/// and the request stands until he answers or its time runs out.
fn settled(record: &airband_core::agent::CallRecord) -> crate::adapters::Watch {
    let path = std::path::PathBuf::from(&record.record);
    let (tool, input) = (record.tool.clone(), record.input.clone());
    let mut seen = Calls::default();
    Box::new(move || {
        if seen.unknowable {
            return None;
        }
        // the first look reads only the end: a call that is waiting on him now was made last
        let from = if seen.looked { seen.read } else { airband_io::files::metadata(&path).map_or(0, |m| m.len().saturating_sub(FIRST_LOOK)) };
        let (lines, end) = transcript_from(&path, from, !seen.looked && from > 0);
        if !seen.looked && from > 0 && end == from {
            // not one whole line in the end of it yet: looked at again from its end next time
            return None;
        }
        for line in &lines {
            for c in line["message"]["content"].as_array().into_iter().flatten() {
                let id = || c["id"].as_str().map(String::from);
                match c["type"].as_str() {
                    Some("tool_use") if c["name"] == tool.as_str() && c["input"] == input => {
                        if seen.looked && seen.call.is_none() {
                            seen.call = id();
                        }
                        seen.open.extend(id());
                    }
                    Some("tool_result") => {
                        if seen.call.as_deref().is_some_and(|call| c["tool_use_id"] == call) {
                            return Some(settled_how(line));
                        }
                        seen.open.retain(|open| c["tool_use_id"] != open.as_str());
                    }
                    _ => {}
                }
            }
        }
        if !seen.looked {
            seen.looked = true;
            match seen.open.as_slice() {
                [] => {}
                [one] => seen.call = Some(one.clone()),
                _ => {
                    seen.unknowable = true;
                    return None;
                }
            }
        }
        seen.read = end;
        None
    })
}

/// How much of the end of a transcript the first look reads.
const FIRST_LOOK: u64 = 1 << 20;

/// What has been read of a transcript while a prompt waits.
#[derive(Default)]
struct Calls {
    /// How far it has been read: always to the end of a whole line.
    read: u64,
    /// The first look is over.
    looked: bool,
    /// Calls with this tool and input that have no result yet.
    open: Vec<String>,
    /// The call the prompt is for, once it is known.
    call: Option<String>,
    /// More than one call could be the one asked about.
    unknowable: bool,
}

/// A transcript's whole lines from byte `from` on, each as JSON, and where the last whole line ended. A line
/// still being written (no newline yet) is left for the next look, whole. `mid_line` says `from` may be in the
/// middle of a line: what comes before the first newline is skipped.
fn transcript_from(path: &std::path::Path, from: u64, mid_line: bool) -> (Vec<Value>, u64) {
    let Ok(bytes) = airband_io::files::read_part(path, from, u64::MAX) else { return (Vec::new(), from) };
    let mut at = 0;
    if mid_line {
        match bytes.iter().position(|b| *b == b'\n') {
            Some(newline) => at = newline + 1,
            None => return (Vec::new(), from),
        }
    }
    let mut lines = Vec::new();
    while let Some(newline) = bytes[at..].iter().position(|b| *b == b'\n') {
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes[at..at + newline]) {
            lines.push(v);
        }
        at += newline + 1;
    }
    (lines, from + at as u64)
}

/// Which part of Claude Code's this tool call is, as its row has it.
fn role(hook: &Value) -> Option<String> {
    crate::adapters::hooks::tool_use(HARNESS, hook["tool_name"].as_str()?)["role"].as_str().map(String::from)
}
/// Is this tool call Claude Code starting a subagent?
fn is_subagent(hook: &Value) -> bool {
    role(hook).as_deref() == Some("subagent")
}

/// The files Claude Code's own tool for handing him a file was given: its `files` field, a list of paths.
/// Whether one is a picture is found out from the file itself, when the tower makes its copy.
/// A path relative to no folder (the hook said none) is left out, and said: it is never looked for in whatever
/// folder this hook happens to run in.
fn files_sent(hook: &Value) -> Vec<String> {
    let cwd = hook["cwd"].as_str().filter(|c| !c.is_empty());
    hook["tool_input"]["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|s| {
            let path = std::path::Path::new(s);
            match (path.is_absolute(), cwd) {
                (true, _) => Some(path.to_path_buf()),
                (false, Some(cwd)) => Some(std::path::Path::new(cwd).join(path)),
                (false, None) => {
                    eprintln!("airband: a file it handed over is named from its folder, and the hook did not say which folder that is; it was left out");
                    None
                }
            }
        })
        .filter(|path| path.is_file())
        .map(|path| path.display().to_string())
        .collect()
}

/// What one of Claude Code's hooks (`HOOKS`) reports.
pub(crate) fn events(hook: &Value) -> Vec<AgentEvent> {
    if headless() {
        return Vec::new();
    }
    let events = crate::adapters::hooks::session_events(hook).unwrap_or_else(|| match hook["hook_event_name"].as_str() {
        // the notice that it has been idle only repeats the Stop that came just before it. Claude Code says
        // which kind a notice is; none is told from its wording. One for a permission prompt follows the
        // PermissionRequest hook, which has put the request to him already when there was someone to ask.
        Some("Notification") => match (text(hook, "message"), hook["notification_type"].as_str()) {
            (Some(_), Some("idle_prompt")) | (None, _) => Vec::new(),
            (Some(message), kind) => vec![Event::Notice { text: message, leave: kind == Some("permission_prompt") }],
        },
        // it is handing part of the work to a subagent
        Some(at @ ("PreToolUse" | "PostToolUse")) if is_subagent(hook) => vec![Event::Subagent { done: at == "PostToolUse", what: hook["tool_input"]["description"].as_str().unwrap_or("").to_string() }],
        // it sent him a file
        Some("PostToolUse") if role(hook).as_deref() == Some("files") => files_sent(hook).into_iter().map(|path| Event::Picture { path, shown: true }).collect(),
        // it published a page: the tool's own answer says where it is and what it is called
        Some("PostToolUse") if role(hook).as_deref() == Some("artifact") => made_artifact(hook).into_iter().collect(),
        _ => Vec::new(),
    });
    // what its turn made that Claude Code wrote down itself comes first, so it goes with what the turn said
    let made = if matches!(hook["hook_event_name"].as_str(), Some("Stop" | "SessionStart")) { pull_requests(hook) } else { Vec::new() };
    crate::adapters::hooks::from_agent(agent_of(hook), made.into_iter().chain(events).collect())
}

/// The page the Artifact tool just published, from the tool's own answer. The tool says which of its actions
/// it was asked for (publishing is the one it does when none is named); reading a page, listing them or
/// adding a file to one publishes nothing.
fn made_artifact(hook: &Value) -> Option<Event> {
    let asked = &hook["tool_input"];
    if !matches!(asked["action"].as_str(), None | Some("publish")) || asked["asset"].as_bool() == Some(true) {
        return None;
    }
    let answer = &hook["tool_response"];
    let address = answer["url"].as_str().filter(|a| airband_core::links::secure(a))?;
    let title = answer["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
    Some(Event::Made { address: address.to_string(), title, kind: Some(crate::adapters::row_says(HARNESS, "artifact_link")) })
}

/// Is this a script calling `claude -p`? Those have nobody waiting on them, unless he asked to hear them or the
/// tower runs it.
fn headless() -> bool {
    row_env("entry_env") == Some(crate::adapters::row_says(HARNESS, "headless_entry")) && crate::adapters::stand_in("AIRBAND_INCLUDE_HEADLESS").is_none() && !crate::adapters::held()
}

/// A session id that can name a file: nothing in it may leave the folder.
fn file_safe(session: &str) -> bool {
    !session.is_empty() && session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

// ---- setting Claude Code up to report: its hooks, in its own settings file ----

/// Where Claude Code keeps its settings, sessions and transcripts, as its row has it.
fn claude_home() -> std::path::PathBuf {
    crate::adapters::tool_home(HARNESS)
}

/// The Claude Code tools worth hearing about once they have run, by its row: the one that hands him a file,
/// the one that publishes a page, and the one that runs a subagent.
fn after_tools() -> String {
    crate::adapters::hooks::with_role(HARNESS, &["files", "artifact", "subagent"]).join("|")
}
/// The ones worth hearing about as they start: a subagent.
fn before_tools() -> String {
    crate::adapters::hooks::with_role(HARNESS, &["subagent"]).join("|")
}
/// Permission is asked on the radio first. The time allowed is long because he is the one being waited for; the
/// tower says how long to wait and the hook gives up by itself well before this.
const HOOKS: &[On] = &[
    on("SessionStart", 5),
    on("Stop", 5),
    on("Notification", 5),
    on("UserPromptSubmit", 5),
    on("SessionEnd", 5),
    On { event: "PostToolUse", timeout: 5, only: Some(after_tools) },
    On { event: "PreToolUse", timeout: 5, only: Some(before_tools) },
    on("PermissionRequest", 900),
];

static ACCOUNTS: crate::adapters::AccountBook = crate::adapters::AccountBook { list: accounts::list, switch: accounts::switch, save: accounts::save, add: accounts::add, forget: accounts::forget };

pub(crate) struct ClaudeCode;

impl crate::adapters::Harness for ClaudeCode {
    fn name(&self) -> &'static str {
        HARNESS
    }
    fn events(&self, hook: &Value) -> Vec<AgentEvent> {
        events(hook)
    }
    fn hooks(&self) -> Option<crate::adapters::hooks::File> {
        Some(crate::adapters::hooks::File { name: self.name(), path: claude_home().join(crate::adapters::row_says(HARNESS, "hooks_file")), events: HOOKS, shape: crate::adapters::hooks::Shape::Common })
    }
    fn permission(&self, hook: &Value) -> Option<PermissionAsk> {
        permission(hook)
    }
    fn settled(&self, record: &airband_core::agent::CallRecord) -> Option<crate::adapters::Watch> {
        Some(settled(record))
    }
    fn accounts(&self) -> Option<&'static crate::adapters::AccountBook> {
        Some(&ACCOUNTS)
    }
    fn glance(&self, agent: &Agent) -> crate::adapters::Glance {
        glance(agent)
    }
    fn touched(&self, agent: &Agent) -> Vec<String> {
        touched(agent)
    }
    fn told(&self, agent: &Agent, from: u64) -> (Vec<crate::adapters::Told>, u64) {
        told(agent, from)
    }
    fn steps(&self, agent: &Agent, since: u64) -> Vec<String> {
        steps(agent, since)
    }
    fn open(&self) -> Option<crate::adapters::Listing> {
        open_in(&claude_home(), &running)
    }
    fn worked(&self) -> Vec<crate::adapters::Worked> {
        worked_in(&claude_home())
    }
    fn commands(&self, agent: &Agent, _listed: Option<&[airband_core::commands::Listed]>) -> Option<crate::adapters::commands::Found> {
        commands::commands(agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_control_name_is_counted_in_characters_not_bytes() {
        let line = |name: &str| format!(r#"{{"type":"system","subtype":"local_command","commandRun":{{"command":"remote-control","args":"{name}"}}}}"#);
        let wide = "é".repeat(40);
        assert_eq!(remote_control_line(line(&wide).as_bytes()), Some(wide.clone()), "40 characters is 80 bytes");
        assert_eq!(remote_control_line(line(&"é".repeat(61)).as_bytes()), None);
    }

    #[test]
    fn a_relative_file_with_no_folder_is_left_out_never_looked_for_here() {
        let dir = std::env::temp_dir().join(format!("airband-sent-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        let hook = |cwd: Option<&str>| {
            let mut h: Value = serde_json::from_str(r#"{"tool_input":{"files":["a.png"]}}"#).unwrap();
            if let Some(cwd) = cwd {
                h["cwd"] = Value::String(cwd.into());
            }
            h
        };
        assert_eq!(files_sent(&hook(Some(dir.to_str().unwrap()))), [dir.join("a.png").display().to_string()]);
        assert!(files_sent(&hook(None)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
    use serde_json::json;

    #[test]
    fn a_moved_home_is_where_hooks_and_sessions_are_looked_for_as_the_accounts_look() {
        // the variable is the row's, the same one the accounts and a sign-in use
        assert_eq!(crate::adapters::row_says(HARNESS, "home_env"), "CLAUDE_CONFIG_DIR");
        assert_eq!(crate::adapters::home_given(HARNESS, Some("/elsewhere/claude".into())), std::path::PathBuf::from("/elsewhere/claude"));
        assert_eq!(crate::adapters::home_given(HARNESS, None), crate::config::home().join(".claude"));
    }

    /// A hook is read with the environment and home the test gives it, never this process's: run from inside one of
    /// his sessions (or under `claude -p`), nothing of that session's is read.
    #[test]
    fn a_hook_reads_only_the_environment_and_home_it_is_given() {
        crate::adapters::test_env::clear();
        let home = std::env::temp_dir().join(format!("airband-home-{}", crate::config::random_hex(6)));
        crate::config::tests_home::set(home.clone());
        assert_eq!(claude_home(), home.join(".claude"));
        let hook = json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "Stop", "last_assistant_message": "Done." });
        let plain = agent_of(&hook).unwrap();
        assert_eq!((plain.pid, plain.name.as_deref(), plain.deliver.as_ref()), (None, None, None));
        // a session record under the given home names the session; nothing is read from his own
        std::fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        std::fs::write(home.join(".claude/sessions/77.json"), json!({ "sessionId": "s1", "name": "shop", "nameSource": "user" }).to_string()).unwrap();
        crate::adapters::test_env::set(&crate::adapters::row_says(HARNESS, "pid_env"), "77");
        assert_eq!(agent_of(&hook).unwrap().title.as_deref(), Some("shop"));
        // a one-off run, by what the test says, reports nothing
        crate::adapters::test_env::set(&crate::adapters::row_says(HARNESS, "entry_env"), &crate::adapters::row_says(HARNESS, "headless_entry"));
        assert!(events(&hook).is_empty());
        crate::adapters::test_env::clear();
        assert_eq!(events(&hook).len(), 1);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn whole_lines_are_read_across_parts_and_a_line_still_being_written_is_left() {
        let path = std::env::temp_dir().join(format!("airband-lines-{}.jsonl", crate::config::random_hex(6)));
        let long = "x".repeat(1024 * 1024 + 300);
        std::fs::write(&path, format!("one\n{long}\nthree\nhalf")).unwrap();
        let mut seen = Vec::new();
        let at = whole_lines(&path, 0, |line| seen.push(line.len())).unwrap();
        assert_eq!(seen, [4, long.len() + 1, 6]);
        assert_eq!(at, (4 + long.len() + 1 + 6) as u64, "the half line is left for next time");
        assert!(whole_lines(&path.with_extension("gone"), 0, |_| {}).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn claude_code_stop_becomes_said() {
        let e = events(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "Stop", "last_assistant_message": " Done. " })).remove(0);
        assert_eq!(e.agent.id, "s1");
        assert_eq!(e.event, Event::Said { text: "Done.".into(), kind: None, turn: None });
    }
    #[test]
    fn a_session_open_in_a_terminal_is_found_from_its_record_and_can_be_reached() {
        let claude = std::env::temp_dir().join(format!("airband-claude-{}", crate::config::random_hex(6)));
        let (sessions, project) = (claude.join("sessions"), claude.join("projects").join("-x-shop"));
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let record = |pid: u32, id: &str, more: Value| {
            let mut v = serde_json::json!({ "pid": pid, "sessionId": id, "cwd": "/x/shop", "procStart": format!("Sun Nov 22 10:00:{pid:02} 2026"), "kind": "interactive", "entrypoint": "cli", "messagingSocketPath": format!("/tmp/cc-socks/{pid}.sock"), "name": "shop-a1", "nameSource": "derived", "status": "idle" });
            v.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
            std::fs::write(sessions.join(format!("{pid}.json")), v.to_string()).unwrap();
        };
        record(11, "open-one", serde_json::json!({}));
        std::fs::write(sessions.join("11.abc.key"), serde_json::json!({ "peerToken": "tok-11", "procStart": "Sun  Nov 22 10:00:11 2026" }).to_string()).unwrap();
        std::fs::write(project.join("open-one.jsonl"), "{}\n").unwrap();
        // one he named, with a key file left by an earlier process of the same number
        record(12, "open-two", serde_json::json!({ "name": "shop removals", "nameSource": "user" }));
        std::fs::write(sessions.join("12.def.key"), serde_json::json!({ "peerToken": "stale", "procStart": "Sun Nov 22 09:00:12 2026" }).to_string()).unwrap();
        // not his to talk to: a one-off run, one whose process has gone, one with no way in
        record(13, "one-off", serde_json::json!({ "entrypoint": "sdk-cli" }));
        record(14, "gone", serde_json::json!({}));
        record(15, "no-way-in", serde_json::json!({ "messagingSocketPath": "" }));
        // one Claude Code rewrote shorter in place, leaving the old file's last byte after the object: still found;
        // and one cut short: no record
        record(16, "rewritten", serde_json::json!({}));
        let rewritten = sessions.join("16.json");
        std::fs::write(&rewritten, format!("{}}}", std::fs::read_to_string(&rewritten).unwrap())).unwrap();
        record(17, "cut-short", serde_json::json!({}));
        let cut = sessions.join("17.json");
        let whole = std::fs::read_to_string(&cut).unwrap();
        std::fs::write(&cut, &whole[..whole.len() / 2]).unwrap();
        let running = |pid: u32, started: &str| pid != 14 && started == format!("Sun Nov 22 10:00:{pid:02} 2026");
        let listing = open_in(&claude, &running).unwrap();
        // the one cut short is not read, and the look says it is not whole: nobody missing from it has ended
        assert!(!listing.whole);
        let mut open = listing.open;
        open.sort_by(|a, b| a.agent.id.cmp(&b.agent.id));
        let ids: Vec<&str> = open.iter().map(|o| o.agent.id.as_str()).collect();
        assert_eq!(ids, ["open-one", "open-two", "rewritten"]);
        let one = &open[0].agent;
        assert_eq!((one.harness.as_str(), one.cwd.as_deref(), one.pid, one.title.as_deref()), ("claude-code", Some("/x/shop"), Some(11), None));
        assert_eq!(one.deliver, Some(Deliver::Socket { path: "/tmp/cc-socks/11.sock".into(), token: Some("tok-11".into()) }));
        assert_eq!(one.meta.get("transcript").and_then(Value::as_str), Some(project.join("open-one.jsonl").display().to_string().as_str()));
        let two = &open[1].agent;
        assert_eq!(two.title.as_deref(), Some("shop removals"));
        assert_eq!(two.deliver, Some(Deliver::Socket { path: "/tmp/cc-socks/12.sock".into(), token: None }));
        // with no record kept at all, Claude Code cannot say what is open: nothing is signed off on that
        assert!(open_in(&claude.join("nowhere"), &running).is_none());
        let _ = std::fs::remove_dir_all(&claude);
    }

    #[test]
    fn where_claude_code_has_worked_is_read_from_the_newest_transcript_of_each_project() {
        let claude = std::env::temp_dir().join(format!("airband-claude-{}", crate::config::random_hex(6)));
        let line = |cwd: &str| format!("{}\n{}\n", serde_json::json!({ "type": "queue-operation" }), serde_json::json!({ "type": "user", "cwd": cwd }));
        for (project, file, cwd) in [("-x-shop", "a.jsonl", "/x/shop"), ("-x-shop", "b.jsonl", "/x/shop"), ("-x-garden", "c.jsonl", "/x/garden")] {
            std::fs::create_dir_all(claude.join("projects").join(project)).unwrap();
            std::fs::write(claude.join("projects").join(project).join(file), line(cwd)).unwrap();
        }
        // a project folder with no transcript in it says nothing
        std::fs::create_dir_all(claude.join("projects").join("-x-empty")).unwrap();
        let mut worked: Vec<String> = worked_in(&claude).into_iter().inspect(|w| assert!(w.at > 0)).map(|w| w.folder).collect();
        worked.sort();
        assert_eq!(worked, ["/x/garden", "/x/shop"]);
        let _ = std::fs::remove_dir_all(&claude);
    }

    #[test]
    fn claude_code_idle_notification_is_dropped() {
        // by the kind Claude Code says it is, whatever its words: the same words under another kind are passed on
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "idle_prompt", "message": "Claude is waiting for your input" })).is_empty());
        let other = events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "auth_success", "message": "Claude is waiting for your input" }));
        assert_eq!(other[0].event, Event::Notice { text: "Claude is waiting for your input".into(), leave: false });
        let prompt = events(&json!({ "session_id": "s1", "hook_event_name": "Notification", "notification_type": "permission_prompt", "message": "Claude needs your permission" }));
        assert_eq!(prompt[0].event, Event::Notice { text: "Claude needs your permission".into(), leave: true });
        assert!(events(&json!({ "session_id": "s1", "hook_event_name": "Stop", "last_assistant_message": "" })).is_empty());
    }
    #[test]
    fn the_remote_control_name_is_read_only_from_his_own_command() {
        // as Claude Code writes it: the command and what he gave it are fields of the entry
        let ran = json!({ "type": "system", "subtype": "local_command", "content": "<local-command-stdout></local-command-stdout>", "commandRun": { "command": "remote-control", "args": "shop 2" } });
        assert_eq!(remote_control_line(ran.to_string().as_bytes()), Some("shop 2".into()));
        let bare = json!({ "type": "system", "subtype": "local_command", "commandRun": { "command": "remote-control", "args": "" } });
        assert_eq!(remote_control_line(bare.to_string().as_bytes()), None);
        // another command, and words that only quote this one, name nothing
        let other = json!({ "type": "system", "subtype": "local_command", "commandRun": { "command": "rename", "args": "shop 2" } });
        assert_eq!(remote_control_line(other.to_string().as_bytes()), None);
        let quoted = json!({ "type": "user", "message": { "content": "<command-name>/remote-control</command-name> <command-args>shop 9</command-args> \"commandRun\"" } });
        assert_eq!(remote_control_line(quoted.to_string().as_bytes()), None);
    }
    /// The watch on a hook's call, asked as the tower asks it: true while its prompt still waits.
    fn still_asking(hook: &Value) -> Option<impl Fn() -> bool + use<>> {
        let watch = std::cell::RefCell::new(settled(&call_record(hook)?));
        Some(move || (watch.borrow_mut())().is_none())
    }

    #[test]
    fn who_settled_a_prompt_is_read_from_the_line_with_its_result() {
        // the lines as Claude Code 2.1.331 writes them (their other fields left out)
        let allowed = json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": false }] }, "permissionDecision": { "decision": "accept", "source": "user_temporary", "grantScope": "once", "reasonCode": "unknown" } });
        let refused = json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": true }] }, "toolDenialKind": "user-rejected" });
        let blocked = json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": true }] }, "toolDenialKind": "permission-rule",
                              "permissionDecision": { "decision": "reject", "source": "config", "reasonType": "safetyCheck", "reasonCode": "safetyCheck", "circuitBreaker": "dangerousRemoval" } });
        assert_eq!(settled_how(&allowed), crate::adapters::Settled::There);
        assert_eq!(settled_how(&refused), crate::adapters::Settled::There);
        // blocked once its countdown was up: by its own rules, with nobody's answer
        assert_eq!(settled_how(&blocked), crate::adapters::Settled::RefusedByTool);
        // and the watch says so of the call it is for
        let dir = std::env::temp_dir().join(format!("airband-settled-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("s1.jsonl");
        let input = json!({ "command": "rm -rf \"$(printf %s /x)\"" });
        std::fs::write(&main, json!({ "message": { "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": input }] } }).to_string() + "\n").unwrap();
        let hook = json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": input, "transcript_path": main });
        let record = call_record(&hook).unwrap();
        assert_eq!(permission(&json!({ "cwd": "/x/shop", "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": input, "transcript_path": main })).unwrap().settled_in, Some(record.clone()));
        let mut watch = settled(&record);
        assert_eq!(watch(), None);
        std::fs::write(&main, airband_io::files::read_text(&main).unwrap().unwrap() + &blocked.to_string() + "\n").unwrap();
        assert_eq!(watch(), Some(crate::adapters::Settled::RefusedByTool));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_prompt_settled_at_the_keyboard_is_seen_in_the_transcript_of_the_agent_that_asked() {
        let dir = std::env::temp_dir().join(format!("airband-still-{}", crate::config::random_hex(4)));
        std::fs::create_dir_all(dir.join("s1/subagents")).unwrap();
        let main = dir.join("s1.jsonl");
        let input = json!({ "command": "touch made", "description": "Make a file" });
        let used = |id: &str, input: &Value| json!({ "message": { "content": [{ "type": "tool_use", "id": id, "name": "Bash", "input": input }] } }).to_string() + "\n";
        let result = |id: &str| json!({ "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "is_error": true }] } }).to_string() + "\n";
        let append = |path: &std::path::Path, line: String| {
            let had = airband_io::files::read_text(path).unwrap().unwrap_or_default();
            airband_io::files::write_atomic(path, had + &line).unwrap();
        };
        // an earlier call with the same input, settled long ago: the request is for the last one
        append(&main, used("t0", &input));
        append(&main, result("t0"));
        append(&main, used("t1", &input));
        let hook = json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": input, "transcript_path": main });
        let still = still_asking(&hook).expect("the call is found");
        assert!(still(), "nothing settled it yet");
        // another call's result settles nothing
        append(&main, result("t9"));
        assert!(still());
        // refused at the keyboard (or by the tool itself): its result is written, and the prompt is over
        append(&main, result("t1"));
        assert!(!still());
        // a subagent's call is in its own transcript, by its id
        let sub = dir.join("s1/subagents/agent-a7.jsonl");
        append(&sub, used("t2", &input));
        let mut from_sub = hook.clone();
        from_sub["agent_id"] = json!("a7");
        let still = still_asking(&from_sub).unwrap();
        assert!(still());
        append(&sub, result("t2"));
        assert!(!still());
        // a call written down only after the hook started is the one asked about; one settled before it is not
        let ls = json!({ "command": "ls" });
        append(&main, used("t3", &ls));
        append(&main, result("t3"));
        let still = still_asking(&json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": ls, "transcript_path": main })).unwrap();
        assert!(still(), "the settled one is not this prompt");
        append(&main, used("t4", &ls));
        append(&main, result("t4"));
        assert!(!still(), "written and settled between two looks");

        // a result caught half written is read whole on the next look, not skipped; a character cut in two at the
        // end of the file does not move where the next look starts
        let pwd = json!({ "command": "pwd", "description": "Where é is" });
        append(&main, used("t5", &pwd));
        let still = still_asking(&json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": pwd, "transcript_path": main })).unwrap();
        assert!(still());
        let whole = json!({ "message": { "content": [{ "type": "tool_result", "tool_use_id": "t5", "content": "é" }] } }).to_string() + "\n";
        let cut = whole.bytes().position(|b| b == 0xC3).unwrap() + 1;
        let had = airband_io::files::read_text(&main).unwrap().unwrap_or_default().into_bytes();
        airband_io::files::write_atomic(&main, [had.as_slice(), &whole.as_bytes()[..cut]].concat()).unwrap();
        assert!(still(), "half a line says nothing yet");
        airband_io::files::write_atomic(&main, [had.as_slice(), whole.as_bytes()].concat()).unwrap();
        assert!(!still(), "and once it is whole, the prompt is over");

        // two open with the same tool and input: which is asked about cannot be known, so nothing is watched
        let two = dir.join("two.jsonl");
        append(&two, used("u1", &input));
        append(&two, used("u2", &input));
        let still = still_asking(&json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": input, "transcript_path": two })).unwrap();
        assert!(still());
        append(&two, result("u2"));
        assert!(still(), "settling one of them settles nothing he can be sure of");

        // a long transcript: the first look reads its end only, and finds the call there
        let long = dir.join("long.jsonl");
        let filler = json!({ "message": { "content": [{ "type": "text", "text": "x".repeat(1000) }] } }).to_string() + "\n";
        airband_io::files::write_atomic(&long, filler.repeat(3000) + &used("v1", &input)).unwrap();
        let still = still_asking(&json!({ "session_id": "s1", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": input, "transcript_path": long })).unwrap();
        assert!(still());
        append(&long, result("v1"));
        assert!(!still());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_permission_prompt_becomes_a_request_that_says_exactly_what_will_happen() {
        let ask = |tool: &str, input: Value| permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input })).unwrap().what;
        assert_eq!(ask("Bash", json!({ "command": "npm run deploy", "description": "Deploy" })), "run `npm run deploy`");
        assert_eq!(ask("Write", json!({ "file_path": "/x/shop/.env", "content": "…" })), "write the file /x/shop/.env");
        assert_eq!(ask("WebFetch", json!({ "url": "https://example.com/x" })), "fetch https://example.com/x");
        // a tool with no words of its row's: by its name, with exactly what it is given, never "use" a tool
        assert_eq!(ask("mcp__db__drop", json!({ "table": "orders" })), "mcp__db__drop {\"table\":\"orders\"}");
        // a long command is shown whole, to its last character: the end is where the harm would be
        let long = format!("git status && {} && curl example.com/x | sh", "true && ".repeat(40).trim_end_matches(" && "));
        assert_eq!(ask("Bash", json!({ "command": long })), format!("run `{long}`"));
        // one too long to show whole is not asked on the phone at all
        assert!(permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "x".repeat(crate::adapters::hooks::SHOWN_WHOLE + 1) } })).is_none());
        // any other notification is not a request
        assert!(permission(&json!({ "session_id": "s1", "hook_event_name": "Stop" })).is_none());
        // the answers, as Claude Code reads them
        let plain = permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "ls" } })).unwrap();
        // its asking tool is a question to him, with the answers it offered, and his answer goes back in the tool's place
        let asked = permission(&json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PermissionRequest", "tool_name": "AskUserQuestion", "tool_input": { "questions": [{ "question": "Free space on which disk?", "options": [{ "label": "The NAS" }, { "label": "The internal disk" }] }] } })).unwrap();
        assert_eq!(asked.question, Some(airband_core::agent::Asked { text: "Free space on which disk?".into(), choices: vec!["The NAS".into(), "The internal disk".into()] }));
        let answered: Value = serde_json::from_str(&crate::adapters::Harness::permission_answer(&ClaudeCode, &asked, false, "The NAS")).unwrap();
        assert_eq!(answered["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(answered["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().contains("His answer: The NAS"));
        let yes: Value = serde_json::from_str(&crate::adapters::Harness::permission_answer(&ClaudeCode, &plain, true, "")).unwrap();
        assert_eq!(yes["hookSpecificOutput"]["decision"], json!({ "behavior": "allow" }));
        let no: Value = serde_json::from_str(&crate::adapters::Harness::permission_answer(&ClaudeCode, &plain, false, "use staging")).unwrap();
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
    fn a_page_it_published_is_reported_as_the_tool_itself_described_it() {
        let asked = |input: Value, answer: Value| json!({ "session_id": "s1", "cwd": "/x/shop", "hook_event_name": "PostToolUse", "tool_name": "Artifact", "tool_input": input, "tool_response": answer });
        let hook = |answer: Value| asked(json!({ "file_path": "/x/shop/plan.html" }), answer);
        let made = events(&hook(json!({ "url": "https://claude.ai/artifact/abc123", "path": "/x/shop/plan.html", "artifact_id": "1687a527-01d7-4833-9328-96aa99574dec", "title": "Refund plan", "updated": false, "owned": true, "icon": "test", "audience": "owner", "seq": 1, "version": "1795306244-abb6", "contract": "0.0.0", "liveSubscription": "arming", "editsApplied": 1 })));
        assert_eq!(made[0].event, Event::Made { address: "https://claude.ai/artifact/abc123".into(), title: Some("Refund plan".into()), kind: Some("claude-artifact".into()) });
        // looking one up publishes nothing
        assert!(events(&asked(json!({ "action": "read", "url": "https://claude.ai/artifact/abc123" }), json!({ "url": "https://claude.ai/artifact/abc123", "title": "Refund plan" }))).is_empty());
        assert!(events(&hook(json!({ "error": "refused" }))).is_empty());
    }

    #[test]
    fn a_pull_request_is_read_from_claude_codes_own_record_of_it_and_from_nothing_else() {
        // the line a real session wrote when its `gh pr create` came back (the names changed, nothing else)
        let real = br#"{"type":"pr-link","sessionId":"08613ff5-7162-4f55-a77a-580db37aef01","prNumber":194,"prUrl":"https://github.com/acme/shop/pull/194","prRepository":"acme/shop","timestamp":"2026-11-22T00:47:59.530Z"}"#;
        assert_eq!(pull_request_line(real), Some(Event::Made { address: "https://github.com/acme/shop/pull/194".into(), title: Some("acme/shop #194".into()), kind: Some("pull-request".into()) }));
        // the same address in a tool's output or in what the agent wrote is not a record of one
        let output = br#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"https://github.com/acme/shop/pull/194"}]}}"#;
        let wrote = br#"{"type":"assistant","message":{"content":[{"type":"text","text":"The draft PR is open at https://github.com/acme/shop/pull/194."}]}}"#;
        assert_eq!((pull_request_line(output), pull_request_line(wrote)), (None, None));
        assert_eq!(pull_request_line(br#"{"type":"pr-link","prNumber":194}"#), None);
    }

    #[test]
    fn a_picture_a_tool_touched_is_reported() {
        let dir = std::env::temp_dir().join(format!("airband-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.PNG"), b"x").unwrap();
        let cwd = dir.display().to_string();
        // one it only read is not reported: only what it handed him, by the field its tool gives the files in
        assert!(events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "shot.PNG" } })).is_empty());
        let sent = events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "SendUserFile", "tool_input": { "files": [dir.join("shot.PNG"), "notes.md", "gone.png"] } }));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].event, Event::Picture { path: dir.join("shot.PNG").display().to_string(), shown: true });
        assert!(events(&json!({ "session_id": "s1", "cwd": cwd, "hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": { "file_path": "main.rs" } })).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn what_he_typed_is_read_from_the_transcript_and_nothing_else_is() {
        let transcript = [
            r#"{"type":"user","uuid":"a1","timestamp":"2026-11-19T17:37:34.021Z","origin":{"kind":"human"},"message":{"role":"user","content":"ok so how do i get a qr code"}}"#,
            // passed on by the tower, sent by another agent, the harness's own notice, a command: none is him typing
            r#"{"type":"user","uuid":"a2","timestamp":"2026-11-19T17:38:00.000Z","origin":{"kind":"peer","from":"uds:/tmp/x.sock"},"message":{"role":"user","content":"Message from Charles, sent through Airband: a human said this"}}"#,
            r#"{"type":"user","uuid":"a3","timestamp":"2026-11-19T17:38:01.000Z","origin":{"kind":"task-notification"},"message":{"role":"user","content":"done"}}"#,
            // as Claude Code writes a command of its own: with no word on who it came from
            r#"{"type":"user","uuid":"a4","timestamp":"2026-11-19T17:38:02.000Z","message":{"role":"user","content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"user","uuid":"a5","timestamp":"2026-11-19T17:38:03.000Z","message":{"role":"user","content":[{"type":"tool_result","content":"a \"human\" wrote this file"}]}}"#,
            // sent while it was working, with a picture
            r#"{"type":"attachment","uuid":"a6","attachment":{"type":"queued_command","prompt":[{"type":"image","source":{"data":"…"}},{"type":"text","text":"Swiping on the pop-up should get rid of it."}],"source_uuid":"q6","origin":{"kind":"human"},"timestamp":"2026-11-19T17:40:46.000Z"}}"#,
        ]
        .join("\n");
        let all = told_in(&transcript);
        let told: Vec<_> = all.iter().filter(|t| t.his).collect();
        assert_eq!(told.iter().map(|t| (t.id.as_str(), t.text.as_str())).collect::<Vec<_>>(), vec![("a1", "ok so how do i get a qr code"), ("q6", "Swiping on the pop-up should get rid of it.")]);
        assert_eq!(told[0].at, 1795109854021);
        // what the tower passed on is seen too, as taken in and not as something he typed
        assert_eq!(all.iter().filter(|t| !t.his).map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a2"]);
    }

    #[test]
    fn what_a_session_touched_is_its_files_and_commands_newest_first() {
        let transcript = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/work/shop/cart.ts"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cd /work/warehouse && cargo test\nsecond line","description":"Run the tests"}}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/elsewhere/x"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/warehouse/src/stock.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/warehouse/src/stock.rs"}}]}}"#,
        ]
        .join("\n");
        assert_eq!(touched_in(&transcript), vec!["/work/warehouse/src/stock.rs", "cd /work/warehouse && cargo test", "/work/shop/cart.ts"]);
    }

    #[test]
    fn the_model_and_effort_are_those_of_the_latest_answer() {
        let transcript = [
            r#"{"type":"assistant","isSidechain":false,"effort":"medium","message":{"model":"claude-opus-4-7","content":[]}}"#,
            r#"{"type":"user","message":{"content":"switch to fable"}}"#,
            r#"{"type":"assistant","isSidechain":false,"effort":"high","message":{"model":"claude-fable-5-2","content":[]}}"#,
            // a subagent on another model, and a line the harness wrote itself: neither is what the session runs on
            r#"{"type":"assistant","isSidechain":true,"effort":"low","message":{"model":"claude-haiku-4-5-20251001","content":[]}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"<synthetic>","content":[]}}"#,
        ]
        .join("\n");
        assert_eq!(engine_in(&transcript), Some(("Fable 5.2".to_string(), Some("high".to_string()))));
        assert_eq!(engine_in(r#"{"type":"user","message":{"content":"hello"}}"#), None);
        // a name comes from the row; an id the row does not have is shown as written, never taken apart
        assert_eq!(model_name("claude-haiku-4-5-20251001").as_deref(), Some("Haiku 4.5"));
        assert_eq!(model_name("some-new-model-9").as_deref(), Some("some-new-model-9"));
        assert_eq!(model_name("<synthetic>"), None);
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
        assert_eq!(s("Edit", json!({ "file_path": "/x/code/refund.ts" })).as_deref(), Some("Editing refund.ts"));
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

#[cfg(test)]
mod golden;
#[cfg(test)]
mod row;
