//! How a coding tool's agents are started: the launchers a tool's row in the catalogue carries
//! (`catalog/tools/<id>.json`), and the ones he adds in his own settings.
//!
//! A launcher names a program that is run on his computer. So the ones that come with airband are part of
//! the build and never of a catalogue fetched later (`catalog::accept`).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One way to start a new agent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Launcher {
    /// What it is called: where he sets the usual one, and what the tower remembers an agent was started by.
    /// In his own settings the name is the key it is written under, and this is filled in from that.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The coding tool it starts, by that tool's id ("claude-code"): which adapter its agents report through,
    /// which launcher carries on an agent of that kind, and what he means when he asks for a tool by name. In a
    /// tool's own row it is that tool, and is filled in from the row.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub harness: String,
    /// The command. `{cwd}`, `{task}`, `{project}` and `{session}` (a fresh name for a terminal session) are
    /// filled in, and `{options}` is where the arguments for a model and an effort go. Not used by a launcher
    /// held through the Agent Client Protocol, whose program is in `held`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    /// The models he can ask for by name ("with opus, …"), each with what the tool calls it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// The efforts he can ask for ("with fable high, …"), as the tool takes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The arguments that choose a model, with `{model}` filled in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with_model: Vec<String>,
    /// The arguments that choose an effort, with `{effort}` filled in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with_effort: Vec<String>,
    /// When set, the tower does not hand the agent off: it keeps the process in a holder of its own
    /// (`airband hold`) and gives it his words as his own turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

impl Launcher {
    /// The command to run: `fill` gives each word of `argv` its values, and the arguments for the model and
    /// effort, as the row writes them, go where `{options}` stands.
    pub fn command(&self, model: Option<&str>, effort: Option<&str>, mut fill: impl FnMut(&String) -> String) -> Vec<String> {
        let mut options: Vec<String> = Vec::new();
        if let Some(m) = model {
            options.extend(self.with_model.iter().map(|a| a.replace("{model}", m)));
        }
        if let Some(e) = effort {
            options.extend(self.with_effort.iter().map(|a| a.replace("{effort}", e)));
        }
        self.argv.iter().flat_map(|a| if a == "{options}" { options.clone() } else { vec![fill(a)] }).collect()
    }

    /// What this launcher's own row says its agents can be started on.
    pub fn offers(&self) -> Offers {
        Offers {
            models: self.models.iter().map(|(name, id)| Choice { id: id.clone(), name: name.clone() }).collect(),
            efforts: self.efforts.iter().map(|e| Choice { id: e.clone(), name: e.clone() }).collect(),
        }
    }
}

/// One thing a new agent can be set to: a model, or an effort.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Choice {
    /// What the tool calls it, and takes.
    pub id: String,
    /// What it is called to him.
    pub name: String,
}

/// What a new agent of one launcher can be started on. For a program started with arguments it is what the
/// launcher's row lists. An agent that speaks the Agent Client Protocol lists its own when a session opens,
/// and its holder passes that on (`airband hold`), so the tower offers what the tool itself last offered.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Offers {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<Choice>,
}

/// How a held agent is driven: the two things a holder knows how to do. A row says which, by name (`by`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "kebab-case")]
pub enum Held {
    /// A program that reads one message a line on its standard input for as long as it runs, and reports by
    /// itself (its tool's hooks, or `airband report`). The launcher's `argv` starts it.
    Lines {
        /// Arguments that start a new session, with `{id}` the id the tower chose for the agent.
        #[serde(default)]
        fresh: Vec<String>,
        /// Arguments that carry on the session with that id.
        #[serde(default)]
        resume: Vec<String>,
        /// One line of input for one message of his, with `{text}` where his words go as a JSON string.
        line: String,
        /// One line of input that stops what it is doing, with `{id}` a new id for the request. Without one, an
        /// agent started this way cannot be stopped from a radio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop: Option<String>,
    },
    /// A program that speaks the Agent Client Protocol (agentclientprotocol.com) on its standard input and
    /// output. The holder speaks it back, and reports for the agent.
    Acp {
        program: Vec<String>,
        /// The mode to put a new session in, by the agent's own id for it ("read-only"), when not the one it
        /// opens in: how much it does without asking.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_row_says_how_it_is_driven() {
        let lines: Held = serde_json::from_str(r#"{"by":"lines","line":"{\"text\":{text}}"}"#).unwrap();
        assert!(matches!(lines, Held::Lines { ref line, stop: None, .. } if line.contains("{text}")));
        assert!(serde_json::from_str::<Held>(r#"{"line":"{text}"}"#).is_err());
        let acp: Held = serde_json::from_str(r#"{"by":"acp","program":["opencode","acp"]}"#).unwrap();
        assert_eq!(acp, Held::Acp { program: vec!["opencode".into(), "acp".into()], mode: None });
        assert_eq!(serde_json::to_value(&acp).unwrap()["by"], "acp");
        // a kind this build has no driver for does not read, and says so
        assert!(serde_json::from_str::<Held>(r#"{"by":"telepathy"}"#).is_err());
    }

    #[test]
    fn a_rows_models_are_offered_by_name_with_the_tools_own_id() {
        let row = Launcher { models: [("opus".to_string(), "claude-opus".to_string())].into(), efforts: vec!["high".into()], ..Launcher::default() };
        assert_eq!(row.offers(), Offers { models: vec![Choice { id: "claude-opus".into(), name: "opus".into() }], efforts: vec![Choice { id: "high".into(), name: "high".into() }] });
    }
}
