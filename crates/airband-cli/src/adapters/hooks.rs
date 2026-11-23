//! For a tool that reports through hooks of the common shape: a JSON file of the tool's own
//! in which each event has a list of commands, each run with the event as JSON on its standard input, and for
//! a permission request the decision read as JSON from its standard output. An adapter for such a tool says
//! where the file is and which events it wants ([`File`]); putting them there and taking them out is here.
#![expect(clippy::disallowed_macros, reason = "writes hook entries in each tool's own settings shape")]
use airband_core::agent::{Agent, AgentEvent, Event, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use airband_io::foreign::Foreign;

/// A field of an event as text: trimmed, and nothing when it is absent, not text, or empty.
pub(crate) fn text(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from)
}

/// What the events every tool of this kind sends alike come to: a session starting and ending, his prompt
/// going in, a turn ending with what was said. Nothing for an event that is the tool's own to read. A tool
/// that names its turns says which turn ended (`turn_id`).
pub(crate) fn session_events(hook: &Value) -> Option<Vec<Event>> {
    Some(match hook["hook_event_name"].as_str()? {
        "SessionStart" => vec![Event::Checkin],
        "UserPromptSubmit" => vec![Event::Working],
        "Stop" => text(hook, "last_assistant_message").map(|said| Event::Said { text: said, kind: None, turn: text(hook, "turn_id") }).into_iter().collect(),
        "SessionEnd" => vec![Event::Signoff],
        _ => return None,
    })
}

/// Events as the tower takes them, each from this agent. None of them when the hook did not say who it is.
pub(crate) fn from_agent(agent: Option<Agent>, events: Vec<Event>) -> Vec<AgentEvent> {
    let Some(agent) = agent else { return Vec::new() };
    events.into_iter().map(|event| AgentEvent::new(agent.clone(), event).via(airband_core::agent::Via::Hook)).collect()
}

/// What a coding tool's row says about one of its own tools, by the name the tool calls it (`with.tools`):
/// what it is doing (`doing`), what it asks leave for (`asks`), whether it is bookkeeping, and its part
/// (`role`: subagent, question, files, artifact). Null when the row does not list it.
pub(crate) fn tool_use(harness: &str, tool: &str) -> Value {
    super::tool_row(harness).map(|t| t.with["tools"][tool].clone()).unwrap_or(Value::Null)
}

/// A command a tool asks leave to run, as he is asked it: in its row's words for its `shell` tool, else in
/// airband's own (a tool reached through the Agent Client Protocol says only that the call runs a command).
pub(crate) fn to_run(harness: &str, command: &str) -> String {
    fill(&tool_use(harness, "shell")["asks"], &serde_json::json!({ "command": command })).unwrap_or_else(|| format!("run `{command}`"))
}

/// A tool's `PermissionRequest` hook: the tool it asks for, and what that tool is given. Nothing for any other
/// hook, and nothing for one that does not name its tool: no name is made up for it.
pub(crate) fn permission_hook(hook: &Value) -> Option<(&str, &Value)> {
    (hook["hook_event_name"].as_str() == Some("PermissionRequest")).then_some(())?;
    Some((hook["tool_name"].as_str()?, &hook["tool_input"]))
}

/// What a tool asks leave to do, as he is asked it, whole: in its row's words when its row has them; else in
/// the tool's own words for it (`own`), when it gives any; else the tool by its name with exactly what it was
/// given. Never a summary made up here.
pub(crate) fn asking(harness: &str, tool: &str, input: &Value, own: Option<String>) -> String {
    fill(&tool_use(harness, tool)["asks"], input).or(own).unwrap_or_else(|| format!("{tool} {input}"))
}

/// What he said when he said no from his phone, for the tool to read.
pub(crate) fn said_no(reason: &str) -> String {
    if reason.is_empty() { "He said no from his phone.".to_string() } else { format!("He said no from his phone: {reason}") }
}

/// Where the rest of a request too long to show is: at the tool's own keyboard.
pub(crate) const AT_KEYBOARD: &str = " The rest is at the keyboard.";

/// The mark on a hook airband put in a tool's settings: an argument the hook command passes over.
const OURS: &str = "--put-there-by-airband";

/// The tools a row gives one of these parts, by their own names.
pub(crate) fn with_role(harness: &str, roles: &[&str]) -> Vec<String> {
    let tools = super::tool_row(harness).and_then(|t| t.with["tools"].as_object().cloned()).unwrap_or_default();
    tools.into_iter().filter(|(_, v)| v["role"].as_str().is_some_and(|r| roles.contains(&r))).map(|(k, _)| k).collect()
}

/// A row's words for a tool call, filled from what the call was given. `{a}` is the field `a`; `{a|b}` the
/// first of them given; `.file` after it takes a path's own name, `.line` its first line, `.site` an address's
/// site. A list is tried in order and the first whose fields are all given is the one; None when none is.
pub(crate) fn fill(words: &Value, input: &Value) -> Option<String> {
    let one = |t: &str| -> Option<String> {
        let mut out = String::new();
        let mut rest = t;
        while let Some(open) = rest.find('{') {
            let close = open + rest[open..].find('}')?;
            out.push_str(&rest[..open]);
            let inner = &rest[open + 1..close];
            let (fields, how) = inner.split_once('.').unwrap_or((inner, ""));
            let given = fields.split('|').find_map(|f| text(input, f).or_else(|| input[f].as_array().map(|parts| parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).filter(|c| !c.is_empty())))?;
            let made = match how {
                "file" => std::path::Path::new(&given).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(given),
                "line" => given.lines().next()?.to_string(),
                "site" => Some(airband_core::links::site(&given)).filter(|s| !s.is_empty())?.to_string(),
                _ => given,
            };
            out.push_str(&made);
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        Some(out)
    };
    match words {
        Value::String(t) => one(t),
        Value::Array(list) => list.iter().filter_map(Value::as_str).find_map(one),
        _ => None,
    }
}

/// One event a tool is asked to run `airband hook <name>` on.
pub(crate) struct On {
    pub(crate) event: &'static str,
    /// How long the tool waits for the hook, in seconds.
    pub(crate) timeout: u32,
    /// Only for the tools this names, in the tool's own pattern language, made from its row.
    pub(crate) only: Option<fn() -> String>,
}
pub(crate) const fn on(event: &'static str, timeout: u32) -> On {
    On { event, timeout, only: None }
}

/// A tool's hooks file, and what of ours goes in it.
pub(crate) struct File {
    /// The adapter's name: the hook is `airband hook <name>`.
    pub(crate) name: &'static str,
    pub(crate) path: PathBuf,
    pub(crate) events: &'static [On],
}

impl File {
    /// The tool's file, to change: his, so one that cannot be read is never written over (`airband_io::foreign`).
    /// One that is not there is an empty object.
    fn open(&self) -> Result<Foreign<Value>, String> {
        let mut file = Foreign::<Value>::open(&self.path)?;
        if file.value.is_null() {
            file.value = json!({});
        }
        Ok(file)
    }
    /// The command a hook of ours runs: this program, as `hook <name>`, with the mark that says airband put it
    /// there. The mark is what knows it again, wherever the program is installed and whatever it is called.
    fn command(&self, exe: &Path) -> String {
        format!("\"{}\" hook {} {OURS}", exe.display(), self.name)
    }
    /// A hook of ours: one carrying our mark, or the command this program wrote before it had one.
    fn is_ours(&self, hook: &Value, exe: &Path) -> bool {
        let Some(command) = hook["command"].as_str() else { return false };
        command.trim_end().ends_with(&format!(" hook {} {OURS}", self.name)) || command == format!("\"{}\" hook {}", exe.display(), self.name)
    }
    /// Take our hooks out, leaving everyone else's exactly as they were.
    pub(super) fn strip(&self, file: &mut Value, exe: &Path) {
        let Some(hooks) = file.get_mut("hooks").and_then(Value::as_object_mut) else { return };
        for groups in hooks.values_mut() {
            if let Some(list) = groups.as_array_mut() {
                for group in list.iter_mut() {
                    if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        inner.retain(|h| !self.is_ours(h, exe));
                    }
                }
                list.retain(|g| g["hooks"].as_array().is_none_or(|h| !h.is_empty()));
            }
        }
        hooks.retain(|_, groups| groups.as_array().is_none_or(|l| !l.is_empty()));
        if hooks.is_empty() {
            file.as_object_mut().unwrap().remove("hooks");
        }
    }
    /// Put our hooks in, replacing any from an earlier install and leaving everyone else's as they were.
    /// Returns whether the file changed: a tool that asks him to trust its hooks asks again only then.
    pub(crate) fn install(&self, exe: &Path) -> Result<bool, String> {
        let mut kept = self.open()?;
        let file = &mut kept.value;
        let before = file.clone();
        self.strip(file, exe);
        let whole = file.as_object_mut().ok_or_else(|| format!("{} is not a JSON object", self.path.display()))?;
        let hooks = whole.entry("hooks").or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| format!("\"hooks\" in {} is not an object", self.path.display()))?;
        let command = self.command(exe);
        for on in self.events {
            let list = hooks.entry(on.event).or_insert_with(|| json!([])).as_array_mut().ok_or_else(|| format!("a hooks entry in {} is not a list", self.path.display()))?;
            let mut group = json!({ "hooks": [{ "type": "command", "command": command, "timeout": on.timeout }] });
            if let Some(only) = on.only {
                group["matcher"] = only().into();
            }
            list.push(group);
        }
        if *file == before {
            return Ok(false);
        }
        kept.write().map(|()| true)
    }
    pub(crate) fn uninstall(&self, exe: &Path) -> Result<(), String> {
        if !self.path.is_file() {
            return Ok(());
        }
        let mut kept = self.open()?;
        let before = kept.value.clone();
        self.strip(&mut kept.value, exe);
        if kept.value == before { Ok(()) } else { kept.write() }
    }
}

/// The longest thing he is asked to allow from the phone. It is shown whole or not asked there at all.
pub(crate) const SHOWN_WHOLE: usize = 4000;
/// How much of a request's whole input goes with it, for when he wants to see all of it.
const DETAIL: usize = 6000;

/// A request for leave, as he is asked it, whatever tool it came from. `what` finishes "wants to …" and is
/// the thing itself (the command, the file, the address), whole and never cut, because what he reads is what
/// will run: one too long to put in front of him whole on a phone is not asked there at all (nothing comes
/// back), and the tool asks in its own way. `input` is everything the tool said about the request; a long one
/// is cut, and says so, with `elsewhere` saying where the rest can be read.
pub(crate) fn leave(agent: Agent, tool: &str, what: String, input: &Value, elsewhere: &str, question: Option<airband_core::agent::Asked>) -> Option<PermissionAsk> {
    if what.chars().count() > SHOWN_WHOLE {
        return None;
    }
    let whole = serde_json::to_string_pretty(input).unwrap_or_default();
    let detail = match whole.chars().count() {
        n if n > DETAIL => format!("{}\n\n(The first 6,000 of {n} characters.{elsewhere})", whole.chars().take(DETAIL).collect::<String>()),
        _ => whole,
    };
    Some(PermissionAsk { schema: SCHEMA.into(), agent, what, detail: format!("{tool}\n{detail}"), question })
}

/// His answer to a request, as a tool with hooks of this shape wants it printed. A refusal carries his reason.
/// `answered` is for a question of the agent's that he answered: the asking tool is not run (its own screen
/// would only ask again), and his answer goes back in its place.
pub(crate) fn decision(allow: bool, reason: &str, answered: bool) -> String {
    let decision = if answered {
        json!({ "behavior": "deny", "message": format!("He answered this from his phone. His answer: {reason}\nTake that as his answer to the question and carry on. Do not ask it again.") })
    } else if allow {
        json!({ "behavior": "allow" })
    } else {
        json!({ "behavior": "deny", "message": said_no(reason) })
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read() -> String {
        "Read".into()
    }
    const EVENTS: &[On] = &[on("Stop", 5), On { event: "PostToolUse", timeout: 5, only: Some(read) }];
    fn file(dir: &Path) -> File {
        File { name: "some-tool", path: dir.join("settings.json"), events: EVENTS }
    }

    #[test]
    fn strip_removes_only_ours() {
        let f = file(Path::new("/nowhere"));
        let exe = Path::new("/Users/c/.airband/bin/airband");
        let mut s = json!({ "model": "x", "hooks": {
            "Stop": [
                { "hooks": [{ "type": "command", "command": "\"/opt/homebrew/opt/airband/bin/airband\" hook some-tool --put-there-by-airband" }] },
                { "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }
            ],
            // what this program wrote before hooks carried the mark
            "SessionStart": [{ "hooks": [{ "type": "command", "command": "\"/Users/c/.airband/bin/airband\" hook some-tool" }] }]
        }});
        f.strip(&mut s, exe);
        assert_eq!(s, json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } }));
        // a program that happens to take the same arguments is not ours, and neither is our hook for another tool
        let mut other = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "\"/usr/local/bin/other-airband\" hook some-tool" }] },
            { "hooks": [{ "type": "command", "command": "\"/x/airband\" hook other-tool --put-there-by-airband" }] }
        ] } });
        f.strip(&mut other, exe);
        assert_eq!(other["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn installing_twice_changes_nothing_the_second_time_and_keeps_his_own_hooks() {
        let dir = std::env::temp_dir().join(format!("airband-hooks-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let f = file(&dir);
        std::fs::write(&f.path, r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"afplay ding.aiff"}]}]}}"#).unwrap();
        assert!(f.install(Path::new("/x/airband")).unwrap());
        assert!(!f.install(Path::new("/x/airband")).unwrap());
        let now: Value = serde_json::from_str(&std::fs::read_to_string(&f.path).unwrap()).unwrap();
        assert_eq!(now["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(now["hooks"]["PostToolUse"][0]["matcher"], "Read");
        f.uninstall(Path::new("/x/airband")).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&f.path).unwrap()).unwrap();
        assert_eq!(after, json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_request_is_put_whole_or_not_at_all_and_a_long_input_says_it_was_cut() {
        let agent = || crate::adapters::agent_with("a", "x");
        assert!(leave(agent(), "Bash", "x".repeat(SHOWN_WHOLE + 1), &json!({}), "", None).is_none());
        let long = leave(agent(), "Bash", "run `x`".into(), &json!({ "command": "y".repeat(7000) }), AT_KEYBOARD, None).unwrap();
        assert!(long.detail.starts_with("Bash\n") && long.detail.ends_with("characters. The rest is at the keyboard.)"));
        assert!(decision(false, "not now", false).contains("not now") && decision(true, "", false).contains("\"behavior\":\"allow\""));
    }

    #[test]
    fn what_a_tool_with_no_words_of_its_own_asks_is_shown_as_it_was_given() {
        let input = serde_json::json!({ "path": "/w/shop/cart.rs" });
        assert_eq!(asking("no-such-tool", "Frobnicate", &input, None), "Frobnicate {\"path\":\"/w/shop/cart.rs\"}");
        assert_eq!(asking("no-such-tool", "Frobnicate", &input, Some("tidy the cart".into())), "tidy the cart");
    }


    // N41: a hooks file that is there and cannot be read is not taken as no file: installing says so and
    // leaves his file as it was.
    #[cfg(unix)]
    #[test]
    fn n41_a_hooks_file_that_cannot_be_read_is_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("airband-hooks-{}", crate::config::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let f = file(&dir);
        std::fs::write(&f.path, r#"{"model":"his own"}"#).unwrap();
        std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let installed = f.install(Path::new("/x/airband"));
        std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let now = std::fs::read_to_string(&f.path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(installed.is_err(), "installing went ahead over a file it could not read");
        assert_eq!(now, r#"{"model":"his own"}"#);
    }
}
