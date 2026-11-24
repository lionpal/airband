//! The skills and commands an Antigravity CLI agent takes, as `agy` lists them for its session.
//!
//! `agy acp` lists what a session takes (`available_commands_update`): its own slash commands and every skill
//! it has loaded. A prompt that starts with `/` and one of those names runs it. Skills are folders with a
//! `SKILL.md`, read (agy 1.5.15, its own `agy-customizations` guide) from `.agents/skills` (or `.agent`,
//! `_agents`, `_agent`) in the agent's folder and each one above it in its repository, from his config folder
//! (`~/.gemini/config/skills`), from the plugins there, and from the ones it comes with. Those say which listed
//! name is a skill and where it comes from. A session open in agy at his computer cannot be given his words from
//! here at all.
use super::{NAME, home};
use crate::adapters::commands::{Found, Written, from_listed, runs_by, skills, sorted, upward};
use airband_core::agent::Agent;
use airband_core::commands::{Listed, Source};
use std::path::{Path, PathBuf};

pub(super) fn commands(agent: &Agent, listed: Option<&[Listed]>) -> Option<Found> {
    let listed = listed?;
    let mut watch = Vec::new();
    let cwd = agent.cwd.as_deref().map(PathBuf::from);
    let written = written(cwd.as_deref(), &home(), &mut watch);
    Some(Found { items: from_listed(listed, &written, runs_by(agent.deliver.as_ref())), watch })
}

/// Every skill agy reads for this folder.
fn written(cwd: Option<&Path>, home: &Path, watch: &mut Vec<PathBuf>) -> Vec<Written> {
    let row = crate::adapters::tool_row(NAME);
    let roots = row.map(|t| t.command("customization_folders", &[])).unwrap_or_default();
    let skills_dir = crate::adapters::row_says(NAME, "skills_folder");
    let mut found = Vec::new();
    for dir in cwd.map(upward).unwrap_or_default() {
        for root in &roots {
            found.extend(skills(&dir.join(root).join(&skills_dir), Source::Project, watch));
        }
    }
    let config = home.join(crate::adapters::row_says(NAME, "config_folder"));
    found.extend(skills(&config.join(&skills_dir), Source::His, watch));
    let plugins = config.join(crate::adapters::row_says(NAME, "plugins_folder"));
    watch.push(plugins.clone());
    for plugin in sorted(&plugins) {
        found.extend(skills(&plugin.join(&skills_dir), Source::Plugin, watch));
    }
    let builtin = home.join(crate::adapters::row_says(NAME, "state")).join(crate::adapters::row_says(NAME, "builtin_folder"));
    found.extend(skills(&builtin.join(&skills_dir), Source::Tool, watch));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::commands::Kind;

    #[test]
    fn what_its_session_lists_is_given_kind_and_source_by_the_skill_folders_agy_reads() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/adapters/tools/antigravity_cli/fixtures/commands");
        let listed: Vec<Listed> = serde_json::from_str(&std::fs::read_to_string(root.join("listed.json")).unwrap()).unwrap();
        let mut watch = Vec::new();
        let written = written(Some(&root.join("project")), &root.join("gemini"), &mut watch);
        let got = from_listed(&listed, &written, true);
        let by = |n: &str| got.iter().find(|c| c.name == n).map(|c| (c.kind, c.source)).unwrap();
        assert_eq!(by("goal"), (Some(Kind::Command), Some(Source::Tool)));
        assert_eq!(by("migrate-db"), (Some(Kind::Skill), Some(Source::Project)));
        assert_eq!(by("weekly"), (Some(Kind::Skill), Some(Source::His)));
        assert_eq!(by("swarm-plan"), (Some(Kind::Skill), Some(Source::Plugin)));
        // named by its frontmatter, not its folder
        assert_eq!(by("antigravity-guide"), (Some(Kind::Skill), Some(Source::Tool)));
    }
}
