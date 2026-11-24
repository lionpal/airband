//! The skills and commands Claude Code takes in an agent's folder, as Claude Code itself lists them.
//!
//! Claude Code says what it takes when asked over its own stream-json input (the `initialize` control request
//! its SDK sends): every skill and command it has loaded for that folder, his own, the project's, its plugins'
//! and its own, each with its description and the hint for what it is given, its own marked `builtin`. That
//! list is the one a held agent runs: a turn of his that starts with `/` and one of those names runs it. So the
//! list is asked of Claude Code, run for a moment in the agent's folder with nothing to do: no session kept, no
//! hook run, nothing sent to a model. Its files are read only to say which kind each is (a skill's folder, or a
//! command's file) and where it comes from, and to know when to ask again.
//!
//! A session open at his computer that the tower reaches through its messaging socket is given his words as a
//! message from another session, which Claude Code queues with its slash commands left unrun
//! (`skipSlashCommands`, Claude Code 2.1.331): listed, but none runs from here.
use super::{HARNESS, claude_home};
use crate::adapters::commands::{Found, Naming, Written, command_files, runs_by, skills, sorted, upward};
use airband_core::agent::Agent;
use airband_core::commands::{Command, Source};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long Claude Code is given to start and list what it takes.
const ASKING: Duration = Duration::from_secs(30);

/// The line that asks Claude Code what it takes: the control request its SDK opens a session with.
#[derive(Serialize)]
struct Ask {
    #[serde(rename = "type")]
    kind: &'static str,
    request_id: &'static str,
    request: Subtype,
}
#[derive(Serialize)]
struct Subtype {
    subtype: &'static str,
}
const ASKED: &str = "airband-commands";

/// A line Claude Code writes: only the answer to that request is read.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum Out {
    #[serde(rename = "control_response")]
    Control { response: Answer },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    response: Option<Init>,
}
#[derive(Deserialize)]
struct Init {
    #[serde(default)]
    commands: Vec<Said>,
}
/// One as Claude Code lists it.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub(super) struct Said {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "argumentHint")]
    hint: String,
    #[serde(default)]
    builtin: bool,
}

/// What Claude Code takes in this agent's folder.
pub(super) fn commands(agent: &Agent) -> Option<Found> {
    let cwd = PathBuf::from(agent.cwd.as_deref()?);
    let said = ask(&cwd)?;
    let mut watch = Vec::new();
    let written = written(&cwd, &claude_home(), &mut watch);
    Some(Found { items: listed(&said, &written, runs_by(agent.deliver.as_ref())), watch })
}

/// Claude Code, started in `cwd` for as long as it takes to answer, asked what it takes. Nothing when it could
/// not be started or did not answer.
fn ask(cwd: &Path) -> Option<Vec<Said>> {
    let row = crate::adapters::tool_row(HARNESS)?;
    let args = row.command("commands_ask", &[]);
    let mut child = airband_io::child::command(row.told("program"))
        .args(&args)
        .current_dir(cwd)
        // airband's own hooks say nothing for a program the tower runs for itself
        .env("AIRBAND_INTERNAL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let line = serde_json::to_string(&Ask { kind: "control_request", request_id: ASKED, request: Subtype { subtype: "initialize" } }).ok()?;
    let mut input = child.stdin.take()?;
    writeln!(input, "{line}").and_then(|()| input.flush()).ok()?;
    let out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(said) = answer(&line) {
                tx.send(said).ok();
                return;
            }
        }
    });
    let said = rx.recv_timeout(ASKING).ok();
    drop(input);
    child.kill().ok();
    child.wait().ok();
    said
}

/// The list, when this line is Claude Code's answer to the request.
fn answer(line: &str) -> Option<Vec<Said>> {
    match serde_json::from_str::<Out>(line).ok()? {
        Out::Control { response } if response.request_id.as_deref() == Some(ASKED) => Some(response.response?.commands),
        _ => None,
    }
}

/// Every skill and command written in a file that Claude Code reads for this folder: the project's, from the
/// agent's folder up to the top of its repository; his own; and his plugins'.
fn written(cwd: &Path, home: &Path, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let project = crate::adapters::row_says(HARNESS, "project_folder");
    let (skills_dir, commands_dir) = (crate::adapters::row_says(HARNESS, "skills_folder"), crate::adapters::row_says(HARNESS, "commands_folder"));
    let mut found = Vec::new();
    for dir in upward(cwd) {
        let own = dir.join(&project);
        found.extend(skills(&own.join(&skills_dir), Source::Project, watch));
        found.extend(command_files(&own.join(&commands_dir), Source::Project, Naming::File, watch));
        watch.push(own.join(crate::adapters::row_says(HARNESS, "hooks_file")));
    }
    found.extend(skills_below(&home.join(&skills_dir), Source::His, watch));
    found.extend(command_files(&home.join(&commands_dir), Source::His, Naming::File, watch));
    watch.push(home.join(crate::adapters::row_says(HARNESS, "hooks_file")));
    found.extend(plugins(home, &skills_dir, &commands_dir, watch));
    found
}

/// His own skills: each folder that holds a `SKILL.md`, and in a folder that does not, the ones it holds (the
/// skills his account syncs are kept a folder down).
fn skills_below(dir: &Path, source: Source, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let mut found = skills(dir, source, watch);
    for sub in sorted(dir).into_iter().filter(|d| d.is_dir() && !d.join("SKILL.md").exists()) {
        found.extend(skills_below(&sub, source, watch));
    }
    found
}

/// The plugins he installed, as Claude Code's record of them has them: each one's skills and commands, named as
/// Claude Code names a plugin's (`plugin:name`).
fn plugins(home: &Path, skills_dir: &str, commands_dir: &str, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    #[derive(Deserialize)]
    struct Installed {
        #[serde(default)]
        plugins: std::collections::BTreeMap<String, Vec<Install>>,
    }
    #[derive(Deserialize)]
    struct Install {
        #[serde(rename = "installPath")]
        path: PathBuf,
    }
    #[derive(Deserialize)]
    struct Manifest {
        name: String,
    }
    let record = home.join(crate::adapters::row_says(HARNESS, "plugins_installed"));
    watch.push(record.clone());
    let Some(installed) = airband_io::files::read_text(&record).ok().flatten().and_then(|t| serde_json::from_str::<Installed>(&t).ok()) else { return Vec::new() };
    let mut found = Vec::new();
    for at in installed.plugins.values().flatten().map(|i| &i.path) {
        let Some(plugin) = airband_io::files::read_text(&at.join(crate::adapters::row_says(HARNESS, "plugin_manifest"))).ok().flatten().and_then(|t| serde_json::from_str::<Manifest>(&t).ok()) else { continue };
        let own = skills(&at.join(skills_dir), Source::Plugin, watch).into_iter().chain(command_files(&at.join(commands_dir), Source::Plugin, Naming::File, watch));
        found.extend(own.map(|w| Written { name: format!("{}:{}", plugin.name, w.name), ..w }));
    }
    found
}

/// Claude Code's list, each given its kind and where it comes from by the file it was written in. One it marks
/// as its own came with it; one of no file it read, and not its own, is said as it is, with neither.
fn listed(said: &[Said], written: &[Written], runs: airband_core::commands::Runs) -> Vec<Command> {
    said.iter()
        .map(|s| {
            let file = written.iter().find(|w| w.name == s.name).filter(|_| !s.builtin);
            Command {
                name: s.name.clone(),
                about: s.description.clone(),
                hint: Some(s.hint.clone()).filter(|h| !h.is_empty()).or_else(|| file.and_then(|w| w.hint.clone())),
                kind: file.map(|w| w.kind),
                source: if s.builtin { Some(Source::Tool) } else { file.map(|w| w.source) },
                runs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::commands::Kind;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/adapters/tools/claude_code/fixtures/commands")
    }

    #[test]
    fn its_answer_is_read_from_the_line_that_answers_the_request_and_no_other() {
        let line = std::fs::read_to_string(fixture().join("initialize.jsonl")).unwrap();
        let mut lines = line.lines();
        assert!(answer(lines.next().unwrap()).is_none(), "a hook's line is not the answer");
        let said = answer(lines.next().unwrap()).unwrap();
        assert_eq!(said.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["greet", "deploy", "notes", "lint:check", "compact", "dataviz"]);
        assert!(answer(r#"{"type":"control_response","response":{"request_id":"someone-else","response":{"commands":[]}}}"#).is_none());
    }

    #[test]
    fn each_is_given_its_kind_and_source_by_the_file_it_was_written_in() {
        let root = fixture();
        let line = std::fs::read_to_string(root.join("initialize.jsonl")).unwrap();
        let said = answer(line.lines().nth(1).unwrap()).unwrap();
        let mut watch = Vec::new();
        // the fixture's home has a plugin record whose path is written relative to the fixture
        let home = std::env::temp_dir().join(format!("airband-cc-commands-{}", std::process::id()));
        std::fs::create_dir_all(home.join("plugins")).unwrap();
        copy_dir(&root.join("home/skills"), &home.join("skills"));
        std::fs::write(home.join("plugins/installed_plugins.json"), format!(r#"{{"version":2,"plugins":{{"lint@market":[{{"scope":"user","installPath":"{}"}}]}}}}"#, root.join("plugin").display())).unwrap();
        let written = written(&root.join("project"), &home, &mut watch);
        let got = listed(&said, &written, airband_core::commands::Runs::Here);
        let by = |n: &str| got.iter().find(|c| c.name == n).unwrap().clone();
        assert_eq!((by("greet").kind, by("greet").source, by("greet").hint.as_deref()), (Some(Kind::Skill), Some(Source::Project), Some("<name>")));
        assert_eq!((by("deploy").kind, by("deploy").source), (Some(Kind::Command), Some(Source::Project)));
        // a synced skill is kept a folder down in his own
        assert_eq!((by("notes").kind, by("notes").source), (Some(Kind::Skill), Some(Source::His)));
        assert_eq!((by("lint:check").kind, by("lint:check").source), (Some(Kind::Command), Some(Source::Plugin)));
        // its own: no file of it, and no kind made up for it
        assert_eq!((by("compact").kind, by("compact").source, by("compact").hint.as_deref()), (None, Some(Source::Tool), Some("<instructions>")));
        assert_eq!((by("dataviz").kind, by("dataviz").source), (None, Some(Source::Tool)));
        assert!(got.iter().all(|c| c.runs == airband_core::commands::Runs::Here));
        // what is watched: the folders and each file read, so a new skill or an edited one asks again
        assert!(watch.contains(&root.join("project/.claude/skills/greet/SKILL.md")));
        assert!(watch.contains(&home.join("skills")));
        assert!(watch.contains(&home.join("plugins/installed_plugins.json")));
        std::fs::remove_dir_all(home).unwrap();
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let p = e.unwrap().path();
            let dest = to.join(p.file_name().unwrap());
            if p.is_dir() { copy_dir(&p, &dest) } else { std::fs::copy(&p, &dest).unwrap(); }
        }
    }

    #[test]
    fn its_row_says_how_to_ask_and_where_its_files_are() {
        for fact in ["project_folder", "skills_folder", "commands_folder", "plugins_installed", "plugin_manifest"] {
            assert!(!crate::adapters::row_says(HARNESS, fact).is_empty(), "{fact}");
        }
        let ask = crate::adapters::tool_row(HARNESS).unwrap().command("commands_ask", &[]);
        // nothing kept and no hook run: a look, not a session
        assert!(ask.iter().any(|a| a == "--no-session-persistence"));
    }
}
