//! The commands and skills a coding tool's own files hold, read the way every such tool writes them: a skill is a
//! folder with a `SKILL.md`, a command he wrote is a Markdown file, and either opens with a few lines of
//! frontmatter (`name`, `description`, `argument-hint`). Which folders a tool reads, and in what order, is each
//! tool's own (its folder under `tools/`); reading one is here. Only names and the few words of the frontmatter
//! are read; nothing of what one says to do is kept or logged.
use airband_core::agent::Deliver;
use airband_core::commands::{Command, Kind, Runs, Source};
use std::path::{Path, PathBuf};

/// What an adapter found an agent's tool takes, and the files and folders whose change means it must be asked
/// again: the list is kept until one of those changes, or the tool lists anew.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub(crate) items: Vec<Command>,
    pub(crate) watch: Vec<PathBuf>,
}

/// One written down in a file: what its frontmatter says, and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Written {
    pub(crate) name: String,
    pub(crate) about: String,
    pub(crate) hint: Option<String>,
    pub(crate) kind: Kind,
    pub(crate) source: Source,
}

/// What the opening lines of a skill's or a command's file say.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Front {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) hint: Option<String>,
}

/// The frontmatter of a Markdown file: the lines between a first `---` and the next, as `key: value`, a value
/// folded over several indented lines (`>-`, `|`) read as one line. Only the three keys a list shows.
pub(crate) fn front(text: &str) -> Front {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Front::default();
    }
    let block: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let mut f = Front::default();
    let mut i = 0;
    while i < block.len() {
        let line = block[i];
        i += 1;
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let mut value = value.trim().to_string();
        // a value written on the indented lines that follow
        if value.is_empty() || value.starts_with('>') || value.starts_with('|') {
            let mut parts = Vec::new();
            while i < block.len() && (block[i].starts_with(char::is_whitespace) || block[i].trim().is_empty()) {
                parts.push(block[i].trim());
                i += 1;
            }
            value = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        }
        let value = unquoted(&value);
        let slot = match key.trim() {
            "name" => &mut f.name,
            "description" => &mut f.description,
            "argument-hint" => &mut f.hint,
            _ => continue,
        };
        *slot = Some(value).filter(|v| !v.is_empty());
    }
    f
}

/// A value as written, without the quotes around it.
fn unquoted(v: &str) -> String {
    let v = v.trim();
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    v.to_string()
}

/// The skills in one folder: each folder in it that holds a `SKILL.md`, named as its frontmatter says or else by
/// its folder. `watch` gets the folder and each file read.
pub(crate) fn skills(dir: &Path, source: Source, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    watch.push(dir.to_path_buf());
    let mut found: Vec<Written> = Vec::new();
    for sub in sorted(dir) {
        let file = sub.join("SKILL.md");
        let Ok(Some(text)) = airband_io::files::read_text(&file) else { continue };
        watch.push(file);
        let f = front(&text);
        let Some(name) = f.name.or_else(|| sub.file_name().map(|n| n.to_string_lossy().into_owned())) else { continue };
        found.push(Written { name, about: f.description.unwrap_or_default(), hint: f.hint, kind: Kind::Skill, source });
    }
    found
}

/// How a tool names a command written in a folder below its commands folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Naming {
    /// By its file alone.
    File,
    /// By its path from the commands folder: `sub/name`.
    Path,
}

/// The commands he wrote in one folder: each Markdown file in it or in a folder below, named as the tool names
/// them. `watch` gets each folder and file read.
pub(crate) fn command_files(dir: &Path, source: Source, naming: Naming, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let mut found = Vec::new();
    files_below(dir, "", source, naming, watch, &mut found);
    found
}
fn files_below(dir: &Path, above: &str, source: Source, naming: Naming, watch: &mut Vec<PathBuf>, found: &mut Vec<Written>) {
    watch.push(dir.to_path_buf());
    for path in sorted(dir) {
        let Some(own) = path.file_stem().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if path.is_dir() {
            let Some(folder) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            files_below(&path, &format!("{above}{folder}/"), source, naming, watch, found);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(Some(text)) = airband_io::files::read_text(&path) else { continue };
        watch.push(path);
        let f = front(&text);
        let name = match naming {
            Naming::File => own,
            Naming::Path => format!("{above}{own}"),
        };
        found.push(Written { name, about: f.description.unwrap_or_default(), hint: f.hint, kind: Kind::Command, source });
    }
}

/// What is in a folder, in the order of its names, so a list reads the same each time. Nothing for a folder
/// that is not there or cannot be read.
pub(crate) fn sorted(dir: &Path) -> Vec<PathBuf> {
    let mut all = airband_io::files::entries(dir).unwrap_or_default();
    all.sort();
    all
}

/// The agent's folder and each one above it up to the top of its repository (the folder that holds `.git`),
/// nearest first. Outside a repository, its folder alone.
pub(crate) fn upward(cwd: &Path) -> Vec<PathBuf> {
    let mut up = Vec::new();
    for dir in cwd.ancestors() {
        up.push(dir.to_path_buf());
        if dir.join(".git").exists() {
            return up;
        }
    }
    vec![cwd.to_path_buf()]
}

/// Whether what he sends reaches an agent reached this way as a turn of his own that its tool reads as he
/// typed it, with nothing in front: only one the tower holds (`tower/route/utterance.rs` `send`). Anything else
/// is given his words from outside, after a line that says who they are from, so a `/` is not where its tool
/// would read it as a command.
pub(crate) fn runs_by(deliver: Option<&Deliver>) -> Runs {
    if matches!(deliver, Some(Deliver::Held)) { Runs::Here } else { Runs::AtComputer }
}

/// The list as a tool listed it for a session (`listed`), each named as the tool names it, and given its kind
/// and where it comes from by the files it was written in: `written` holds each file's under the name the tool
/// lists it by. One the files do not hold came with the tool.
pub(crate) fn from_listed(listed: &[airband_core::commands::Listed], written: &[Written], runs: Runs) -> Vec<Command> {
    listed
        .iter()
        .map(|l| {
            let file = written.iter().find(|w| w.name == l.name);
            Command {
                name: l.name.clone(),
                about: l.about.clone(),
                hint: l.hint.clone().or_else(|| file.and_then(|w| w.hint.clone())),
                kind: Some(file.map_or(Kind::Command, |w| w.kind)),
                source: Some(file.map_or(Source::Tool, |w| w.source)),
                runs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_gives_its_name_description_and_hint_folded_or_quoted() {
        let f = front("---\nname: greet\ndescription: >-\n  Say hello\n  in a fixed way\nargument-hint: \"<name>\"\nallowed-tools: Bash\n---\nBody: not read\n");
        assert_eq!(f, Front { name: Some("greet".into()), description: Some("Say hello in a fixed way".into()), hint: Some("<name>".into()) });
        assert_eq!(front("no frontmatter here"), Front::default());
        assert_eq!(front("---\ndescription: ''\n---\n"), Front::default());
    }

    #[test]
    fn a_repository_is_read_from_the_agents_folder_up_to_its_top_and_no_further() {
        let root = std::env::temp_dir().join(format!("airband-upward-{}", std::process::id()));
        let deep = root.join("repo/app/web");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        assert_eq!(upward(&deep), [deep.clone(), root.join("repo/app"), root.join("repo")]);
        let loose = root.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        assert_eq!(upward(&loose), std::slice::from_ref(&loose));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn what_runs_from_here_is_how_the_agent_is_reached() {
        assert_eq!(runs_by(Some(&Deliver::Held)), Runs::Here);
        assert_eq!(runs_by(Some(&Deliver::Keys { terminal: "t".into(), pane: "p".into() })), Runs::AtComputer, "typed after a line of who it is from");
        assert_eq!(runs_by(Some(&Deliver::Socket { path: "/s".into(), token: None })), Runs::AtComputer);
        assert_eq!(runs_by(Some(&Deliver::Harness { name: "t".into(), session: "s".into() })), Runs::AtComputer);
        assert_eq!(runs_by(None), Runs::AtComputer);
    }
}
