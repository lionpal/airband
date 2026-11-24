//! The commands and skills a Codex agent takes, as Codex lists them for its session.
//!
//! Codex is held through its Agent Client Protocol adapter, which lists what a session takes
//! (`available_commands_update`): its own slash commands, and every skill it has loaded, each named `$` and the
//! skill's name (a plugin's as `$plugin:skill`), the way Codex mentions a skill. A prompt that starts with `/`
//! and one of those names is that command or skill. Codex reads skills from its home (`skills`, with the ones it
//! comes with under `skills/.system`), from `~/.agents/skills`, from `.agents/skills` in the agent's folder and
//! each one above it in its repository, and from the plugins it has installed (`plugins/cache`). Those files
//! say which listed name is which kind and where it comes from. A session open in Codex at his computer is given
//! his words through Codex's queue as a message: no command runs that way, and it lists none to the tower.
use super::{NAME, home};
use crate::adapters::commands::{Found, Written, from_listed, runs_by, skills, sorted, upward};
use airband_core::agent::Agent;
use airband_core::commands::{Listed, Source};
use std::path::{Path, PathBuf};

pub(super) fn commands(agent: &Agent, listed: Option<&[Listed]>) -> Option<Found> {
    let listed = listed?;
    let mut watch = Vec::new();
    let cwd = agent.cwd.as_deref().map(PathBuf::from);
    let written = written(cwd.as_deref(), &home(), &crate::config::home(), &mut watch);
    Some(Found { items: from_listed(listed, &written, runs_by(agent.deliver.as_ref())), watch })
}

/// Every skill Codex reads for this folder, each under the name its session lists it by.
fn written(cwd: Option<&Path>, home: &Path, user: &Path, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let skills_dir = crate::adapters::row_says(NAME, "skills_folder");
    let agents_skills = crate::adapters::row_says(NAME, "agents_skills");
    let mut found = Vec::new();
    for dir in cwd.map(upward).unwrap_or_default() {
        found.extend(skills(&dir.join(&agents_skills), Source::Project, watch));
    }
    found.extend(skills(&user.join(&agents_skills), Source::His, watch));
    found.extend(skills(&home.join(&skills_dir), Source::His, watch));
    found.extend(skills(&home.join(&skills_dir).join(crate::adapters::row_says(NAME, "system_skills")), Source::Tool, watch));
    found.extend(plugins(home, &skills_dir, watch));
    found.into_iter().map(|w| Written { name: format!("${}", w.name), ..w }).collect()
}

/// The skills of the plugins Codex has installed, each named `plugin:skill`: its cache holds each plugin by
/// where it came from, its name and its version, with the plugin's own record of its name.
fn plugins(home: &Path, skills_dir: &str, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        name: String,
    }
    let cache = home.join(crate::adapters::row_says(NAME, "plugins_cache"));
    watch.push(cache.clone());
    let mut found = Vec::new();
    for version in sorted(&cache).iter().flat_map(|from| sorted(from)).flat_map(|plugin| sorted(&plugin)) {
        let Some(plugin) = airband_io::files::read_text(&version.join(crate::adapters::row_says(NAME, "plugin_manifest"))).ok().flatten().and_then(|t| serde_json::from_str::<Manifest>(&t).ok()) else { continue };
        found.extend(skills(&version.join(skills_dir), Source::Plugin, watch).into_iter().map(|w| Written { name: format!("{}:{}", plugin.name, w.name), ..w }));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::commands::Kind;

    #[test]
    fn what_its_session_lists_is_given_kind_and_source_by_the_skill_files_codex_reads() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/adapters/tools/codex/fixtures/commands");
        let listed: Vec<Listed> = serde_json::from_str(&std::fs::read_to_string(root.join("listed.json")).unwrap()).unwrap();
        let mut watch = Vec::new();
        let written = written(Some(&root.join("project")), &root.join("codex-home"), &root.join("user"), &mut watch);
        let got = from_listed(&listed, &written, airband_core::commands::Runs::Here);
        let by = |n: &str| got.iter().find(|c| c.name == n).map(|c| (c.kind, c.source, c.hint.clone())).unwrap();
        assert_eq!(by("review-branch"), (Some(Kind::Command), Some(Source::Tool), Some("branch name".into())));
        assert_eq!(by("$ship"), (Some(Kind::Skill), Some(Source::Project), None));
        assert_eq!(by("$tidy"), (Some(Kind::Skill), Some(Source::His), None));
        assert_eq!(by("$imagegen"), (Some(Kind::Skill), Some(Source::Tool), None));
        assert_eq!(by("$sheets:budget"), (Some(Kind::Skill), Some(Source::Plugin), None));
        assert_eq!(got.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), listed.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), "in the tool's own order");
        assert!(watch.contains(&root.join("project/.agents/skills/ship/SKILL.md")));
    }

    #[test]
    fn a_session_whose_tool_has_listed_nothing_has_nothing_made_up_for_it() {
        let agent = crate::adapters::agent_with("a", NAME);
        assert!(commands(&agent, None).is_none());
    }
}
