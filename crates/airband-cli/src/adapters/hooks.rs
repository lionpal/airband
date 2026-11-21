//! For a tool that reports through hooks of the shape Claude Code made common: a JSON file of the tool's own
//! in which each event has a list of commands, each run with the event as JSON on its standard input, and for
//! a permission request the decision read as JSON from its standard output. An adapter for such a tool says
//! where the file is and which events it wants ([`File`]); putting them there and taking them out is here.
use airband_core::agent::{Agent, PermissionAsk, SCHEMA};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// One event a tool is asked to run `airband hook <name>` on.
pub struct On {
    pub event: &'static str,
    /// How long the tool waits for the hook, in seconds.
    pub timeout: u32,
    /// Only for the tools this names, in the tool's own pattern language.
    pub only: Option<&'static str>,
}
pub const fn on(event: &'static str, timeout: u32) -> On {
    On { event, timeout, only: None }
}

/// A tool's hooks file, and what of ours goes in it.
pub struct File {
    /// The adapter's name: the hook is `airband hook <name>`.
    pub name: &'static str,
    pub path: PathBuf,
    pub events: &'static [On],
}

impl File {
    fn read(&self) -> Result<Value, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{} is not valid JSON; fix it first ({e})", self.path.display())),
            Err(_) => Ok(json!({})),
        }
    }
    /// Whole or not at all, so the tool never reads half of one, and with the permissions the file had: it is
    /// his file, and may hold more than hooks.
    fn write(&self, v: &Value) -> Result<(), String> {
        airband_core::kept::write_atomic(&self.path, format!("{}\n", serde_json::to_string_pretty(v).unwrap())).map_err(|e| e.to_string())
    }
    /// A hook of ours: the `airband` binary, wherever it is installed (~/.airband/bin, or Homebrew's folder), run
    /// as `hook <name>`.
    fn is_ours(&self, hook: &Value) -> bool {
        hook["command"].as_str().and_then(|c| c.trim_end().strip_suffix(&format!(" hook {}", self.name))).is_some_and(|program| {
            let file = program.trim().trim_matches('"').rsplit(['/', '\\']).next().unwrap_or("");
            file == "airband" || file == "airband.exe"
        })
    }
    /// Take our hooks out, leaving everyone else's exactly as they were.
    pub(super) fn strip(&self, file: &mut Value) {
        let Some(hooks) = file.get_mut("hooks").and_then(Value::as_object_mut) else { return };
        for groups in hooks.values_mut() {
            if let Some(list) = groups.as_array_mut() {
                for group in list.iter_mut() {
                    if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        inner.retain(|h| !self.is_ours(h));
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
    pub fn install(&self, exe: &Path) -> Result<bool, String> {
        let mut file = self.read()?;
        let before = file.clone();
        self.strip(&mut file);
        let whole = file.as_object_mut().ok_or_else(|| format!("{} is not a JSON object", self.path.display()))?;
        let hooks = whole.entry("hooks").or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| format!("\"hooks\" in {} is not an object", self.path.display()))?;
        let command = format!("\"{}\" hook {}", exe.display(), self.name);
        for on in self.events {
            let list = hooks.entry(on.event).or_insert_with(|| json!([])).as_array_mut().ok_or_else(|| format!("a hooks entry in {} is not a list", self.path.display()))?;
            let mut group = json!({ "hooks": [{ "type": "command", "command": command, "timeout": on.timeout }] });
            if let Some(only) = on.only {
                group["matcher"] = only.into();
            }
            list.push(group);
        }
        if file == before {
            return Ok(false);
        }
        self.write(&file).map(|()| true)
    }
    pub fn uninstall(&self) -> Result<(), String> {
        if !self.path.is_file() {
            return Ok(());
        }
        let mut file = self.read()?;
        let before = file.clone();
        self.strip(&mut file);
        if file == before { Ok(()) } else { self.write(&file) }
    }
}

/// The longest thing he is asked to allow from the phone. It is shown whole or not asked there at all.
pub const SHOWN_WHOLE: usize = 4000;
/// How much of a request's whole input goes with it, for when he wants to see all of it.
const DETAIL: usize = 6000;

/// A request for leave, as he is asked it, whatever tool it came from. `what` finishes "wants to …" and is
/// the thing itself (the command, the file, the address), whole and never cut, because what he reads is what
/// will run: one too long to put in front of him whole on a phone is not asked there at all (nothing comes
/// back), and the tool asks in its own way. `input` is everything the tool said about the request; a long one
/// is cut, and says so, with `elsewhere` saying where the rest can be read.
pub fn leave(agent: Agent, tool: &str, what: String, input: &Value, elsewhere: &str, question: Option<airband_core::agent::Asked>) -> Option<PermissionAsk> {
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
pub fn decision(allow: bool, reason: &str, answered: bool) -> String {
    let decision = if answered {
        json!({ "behavior": "deny", "message": format!("He answered this from his phone, through Airband. His answer: {reason}\nTake that as his answer to the question and carry on. Do not ask it again.") })
    } else if allow {
        json!({ "behavior": "allow" })
    } else {
        json!({ "behavior": "deny", "message": if reason.is_empty() { "He said no from his phone.".to_string() } else { format!("He said no from his phone: {reason}") } })
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[On] = &[on("Stop", 5), On { event: "PostToolUse", timeout: 5, only: Some("Read") }];
    fn file(dir: &Path) -> File {
        File { name: "claude-code", path: dir.join("settings.json"), events: EVENTS }
    }

    #[test]
    fn strip_removes_only_ours() {
        let f = file(Path::new("/nowhere"));
        let mut s = json!({ "model": "x", "hooks": {
            "Stop": [
                { "hooks": [{ "type": "command", "command": "\"/Users/c/.airband/bin/airband\" hook claude-code" }] },
                { "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }
            ],
            "SessionStart": [{ "hooks": [{ "type": "command", "command": "\"/Users/c/.airband/bin/airband\" hook claude-code" }] }]
        }});
        f.strip(&mut s);
        assert_eq!(s, json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } }));
        let mut only = json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "\"C:\\Users\\c\\.airband\\bin\\airband.exe\" hook claude-code" }] }] } });
        f.strip(&mut only);
        assert_eq!(only, json!({ "model": "x" }));
        // Homebrew's copy is ours too; a different program that happens to take the same arguments is not, and
        // neither is our hook for another tool
        let mut brewed = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "\"/opt/homebrew/opt/airband/bin/airband\" hook claude-code" }] },
            { "hooks": [{ "type": "command", "command": "\"/usr/local/bin/other-airband\" hook claude-code" }] },
            { "hooks": [{ "type": "command", "command": "\"/opt/homebrew/opt/airband/bin/airband\" hook codex" }] }
        ] } });
        f.strip(&mut brewed);
        assert_eq!(brewed["hooks"]["Stop"].as_array().unwrap().len(), 2);
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
        f.uninstall().unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&f.path).unwrap()).unwrap();
        assert_eq!(after, json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay ding.aiff" }] }] } }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_request_is_put_whole_or_not_at_all_and_a_long_input_says_it_was_cut() {
        let agent = || Agent { id: "a".into(), harness: "x".into(), cwd: None, name: None, title: None, pid: None, deliver: None, meta: Default::default() };
        assert!(leave(agent(), "Bash", "x".repeat(SHOWN_WHOLE + 1), &json!({}), "", None).is_none());
        let long = leave(agent(), "Bash", "run `x`".into(), &json!({ "command": "y".repeat(7000) }), " The rest is at the keyboard.", None).unwrap();
        assert!(long.detail.starts_with("Bash\n") && long.detail.ends_with("characters. The rest is at the keyboard.)"));
        assert!(decision(false, "not now", false).contains("not now") && decision(true, "", false).contains("\"behavior\":\"allow\""));
    }
}
