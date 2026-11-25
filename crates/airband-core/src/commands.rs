//! The commands and skills an agent's tool takes from him: a turn that starts with `/` and the name, then what
//! it is given ("/review main"). Which ones there are is the tool's to say, for the folder the agent works in and
//! on that computer; whether one can be run from his phone is how that agent is reached. Both are told here as
//! data, and nothing is worked out from a name or a description.
use serde::{Deserialize, Serialize};

/// One the agent's tool itself listed for a session, in its own words: what an agent held through the Agent
/// Client Protocol says it takes (`available_commands_update`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub name: String,
    #[serde(default)]
    pub about: String,
    /// What it is given after its name, as the tool puts it ("branch name").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// What kind of thing it is, in the tool's own terms (each tool's row says what it calls each kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Written down as a skill: a folder with its `SKILL.md`.
    Skill,
    /// One of the tool's own commands, or one he wrote as a command file.
    Command,
}

/// Where it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// It comes with the tool.
    Tool,
    /// His own, for every folder.
    His,
    /// The project's: in the agent's folder or one above it.
    Project,
    /// A plugin he installed.
    Plugin,
}

/// Whether one picked from his phone runs, and when it does not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runs {
    /// What he sends reaches the agent as a turn of his own that its tool reads as a command, and what came of
    /// it comes back.
    Here,
    /// His words reach the agent only from outside, as a message, which its tool reads as words (a session he
    /// opened at the computer).
    AtComputer,
}

/// One he can pick.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Its name, as the tool takes it after `/`.
    pub name: String,
    /// What it does, in its own short words. Empty when it has none.
    #[serde(default)]
    pub about: String,
    /// What it is given after its name, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Nothing when neither the tool nor its files say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    pub runs: Runs,
}

/// `GET commands`'s answer: every one the agent's tool takes, in the tool's own order. Empty when its tool has
/// not said (an agent whose tool lists them only once its session is open, or a tool with no way to list them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commands {
    pub items: Vec<Command>,
}

/// `GET commands`: whose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandsQuery {
    pub agent: String,
}

/// What a tool calls these, from its row: the list as a whole, and each kind. Shown as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terms {
    /// The list as a whole, as a heading: "Skills".
    pub term: String,
    /// One that is a skill: "skill".
    #[serde(default)]
    pub skill: String,
    /// One that is a command: "command".
    #[serde(default)]
    pub command: String,
}

impl Terms {
    /// What one of this kind is called; nothing when its kind is not known or the row does not say.
    pub fn of(&self, kind: Option<Kind>) -> Option<&str> {
        let said = match kind? {
            Kind::Skill => &self.skill,
            Kind::Command => &self.command,
        };
        (!said.is_empty()).then_some(said.as_str())
    }
}

/// What a list is headed when it is not one tool's: his words may go to agents of several.
pub const NEUTRAL: &str = "Commands";

/// What the box holds once he picks one: its name after `/`, and a space for what he gives it.
pub fn typed(name: &str) -> String {
    format!("/{name} ")
}

/// The part of a name he has typed, while the box holds `/` and the start of a name and nothing more: the
/// tool's own way of saying a command. Nothing once there is a space (he is giving it what it takes) or the
/// box does not start that way.
pub fn naming(box_text: &str) -> Option<&str> {
    let rest = box_text.strip_prefix('/')?;
    (!rest.chars().any(char::is_whitespace)).then_some(rest)
}

/// The ones whose names hold what he has typed, those that start with it first, each group in the tool's own
/// order. Letters are compared without case.
pub fn matching<'a>(items: &'a [Command], typed: &str) -> Vec<&'a Command> {
    let typed = typed.to_lowercase();
    let lower: Vec<String> = items.iter().map(|c| c.name.to_lowercase()).collect();
    let starts = items.iter().zip(&lower).filter(|(_, n)| n.starts_with(&typed)).map(|(c, _)| c);
    let within = items.iter().zip(&lower).filter(|(_, n)| !n.starts_with(&typed) && n.contains(&typed)).map(|(c, _)| c);
    starts.chain(within).collect()
}

/// What it does is the tool's or his own writing: the debug form says how long, never what.
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command").field("name", &self.name).field("about", &crate::redact::Withheld(&self.about)).field("kind", &self.kind).field("source", &self.source).field("runs", &self.runs).finish()
    }
}
impl std::fmt::Debug for Listed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listed").field("name", &self.name).field("about", &crate::redact::Withheld(&self.about)).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Command {
        Command { name: name.into(), about: String::new(), hint: None, kind: None, source: None, runs: Runs::Here }
    }

    #[test]
    fn the_box_names_a_command_only_while_it_holds_a_slash_and_a_name() {
        assert_eq!(naming("/"), Some(""));
        assert_eq!(naming("/rev"), Some("rev"));
        assert_eq!(naming("/review main"), None, "past the name, he is giving it what it takes");
        assert_eq!(naming("review"), None);
        assert_eq!(naming(" /review"), None);
        assert_eq!(typed("review"), "/review ");
        assert_eq!(naming(typed("review").trim_end()), Some("review"));
    }

    #[test]
    fn names_that_start_with_what_he_typed_come_first_then_the_ones_that_hold_it_in_the_tools_order() {
        let items = [named("code-review"), named("review-branch"), named("Review"), named("compact")];
        let got: Vec<&str> = matching(&items, "rev").iter().map(|c| c.name.as_str()).collect();
        assert_eq!(got, ["review-branch", "Review", "code-review"]);
        assert_eq!(matching(&items, "").len(), 4, "nothing typed: all of them, as the tool lists them");
        assert!(matching(&items, "zzz").is_empty());
    }

    #[test]
    fn a_kind_is_called_what_the_tools_row_calls_it_and_nothing_is_made_up() {
        let terms = Terms { term: "Skills".into(), skill: "skill".into(), command: String::new() };
        assert_eq!(terms.of(Some(Kind::Skill)), Some("skill"));
        assert_eq!(terms.of(Some(Kind::Command)), None);
        assert_eq!(terms.of(None), None);
    }

    #[test]
    fn what_a_command_does_is_never_in_its_debug_form() {
        let c = Command { about: "secret runbook".into(), ..named("deploy") };
        let shown = format!("{c:?}");
        assert!(shown.contains("deploy") && !shown.contains("runbook"));
        let l = Listed { name: "deploy".into(), about: "secret runbook".into(), hint: None };
        assert!(!format!("{l:?}").contains("runbook"));
    }
}
