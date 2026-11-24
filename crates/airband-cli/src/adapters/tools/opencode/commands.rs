//! The commands an OpenCode agent takes, as OpenCode lists them for its session.
//!
//! OpenCode is held through its own Agent Client Protocol server, which lists what a session takes
//! (`available_commands_update`): its own commands and the ones he wrote. A prompt that starts with `/` and one
//! of those names runs it. He writes one as a Markdown file in a `command` or `commands` folder (named by its
//! path there, `sub/name`) of his OpenCode config folder or of `.opencode` in the agent's folder and each one
//! above it in its repository, or under `command` in an `opencode.json` there (OpenCode 2.3.2). Those say which
//! listed name is his and where it comes from. A session open in OpenCode at his computer is given his words
//! through its server as a prompt, which OpenCode reads as words: no command runs that way.
//!
//! One sent to an agent the tower holds does run, but OpenCode 2.3.2's protocol server ends the turn as it hands
//! the command on (`session.command`), before it has run, and sends nothing of it after: the client is never told
//! when it is over or what it said. Its row says so (`held_commands`), and the list says these give no answer.
use super::NAME;
use crate::adapters::commands::{Found, Naming, Written, command_files, from_listed, runs_by, upward};
use airband_core::agent::Agent;
use airband_core::commands::{Kind, Listed, Runs, Source};
use std::path::{Path, PathBuf};

pub(super) fn commands(agent: &Agent, listed: Option<&[Listed]>) -> Option<Found> {
    let listed = listed?;
    let mut watch = Vec::new();
    let cwd = agent.cwd.as_deref().map(PathBuf::from);
    let written = written(cwd.as_deref(), &config_folder(), &mut watch);
    Some(Found { items: from_listed(listed, &written, held(runs_by(agent.deliver.as_ref()))), watch })
}

/// What one picked from his phone comes to for an agent the tower holds, as OpenCode's row says.
fn held(runs: Runs) -> Runs {
    match runs {
        Runs::Here => serde_json::from_value(serde_json::Value::String(crate::adapters::row_says(NAME, "held_commands"))).expect("OpenCode's row says what a command sent to a held agent comes to"),
        other => other,
    }
}

/// His OpenCode config folder: under the folder his system names for settings, or its usual place.
fn config_folder() -> PathBuf {
    let base = crate::adapters::env(&crate::adapters::row_says(NAME, "config_env")).map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(crate::adapters::row_says(NAME, "config_home")));
    base.join(crate::adapters::row_says(NAME, "config"))
}

/// Every command he wrote that OpenCode reads for this folder.
fn written(cwd: Option<&Path>, config: &Path, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let project = crate::adapters::row_says(NAME, "project_folder");
    let mut found = Vec::new();
    for dir in cwd.map(upward).unwrap_or_default() {
        found.extend(in_folder(&dir.join(&project), Source::Project, watch));
        found.extend(in_settings(&dir.join(crate::adapters::row_says(NAME, "settings")), Source::Project, watch));
    }
    found.extend(in_folder(config, Source::His, watch));
    found.extend(in_settings(&config.join(crate::adapters::row_says(NAME, "settings")), Source::His, watch));
    found
}

/// The command files in one of its folders, under each of the names OpenCode reads.
fn in_folder(folder: &Path, source: Source, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let names = crate::adapters::tool_row(NAME).map(|t| t.command("commands_folders", &[])).unwrap_or_default();
    names.iter().flat_map(|n| command_files(&folder.join(n), source, Naming::Path, watch)).collect()
}

/// The commands written under `command` in a settings file.
fn in_settings(file: &Path, source: Source, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    #[derive(serde::Deserialize)]
    struct Settings {
        #[serde(default)]
        command: std::collections::BTreeMap<String, Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(default)]
        description: String,
    }
    watch.push(file.to_path_buf());
    let Some(settings) = airband_io::files::read_text(file).ok().flatten().and_then(|t| serde_json::from_str::<Settings>(&t).ok()) else { return Vec::new() };
    settings.command.into_iter().map(|(name, e)| Written { name, about: e.description, hint: None, kind: Kind::Command, source }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sent_to_a_held_agent_gives_no_answer_and_one_at_the_computer_does_not_run() {
        assert_eq!(held(Runs::Here), Runs::NoAnswer);
        assert_eq!(held(Runs::AtComputer), Runs::AtComputer);
    }

    #[test]
    fn what_its_session_lists_is_given_its_source_by_the_command_files_and_settings_opencode_reads() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/adapters/tools/opencode/fixtures/commands");
        let listed: Vec<Listed> = serde_json::from_str(&std::fs::read_to_string(root.join("listed.json")).unwrap()).unwrap();
        let mut watch = Vec::new();
        let written = written(Some(&root.join("project")), &root.join("config"), &mut watch);
        let got = from_listed(&listed, &written, airband_core::commands::Runs::AtComputer);
        let by = |n: &str| got.iter().find(|c| c.name == n).map(|c| (c.kind, c.source)).unwrap();
        assert_eq!(by("init"), (Some(Kind::Command), Some(Source::Tool)));
        assert_eq!(by("release/notes"), (Some(Kind::Command), Some(Source::Project)), "named by its path in the commands folder");
        assert_eq!(by("standup"), (Some(Kind::Command), Some(Source::His)));
        assert_eq!(by("fix-tests"), (Some(Kind::Command), Some(Source::Project)), "from the settings file");
        assert!(got.iter().all(|c| c.runs == Runs::AtComputer), "reached as words: listed, none runs");
        assert!(watch.contains(&root.join("project/.opencode/commands/release/notes.md")));
    }
}
