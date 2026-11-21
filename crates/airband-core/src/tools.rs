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

/// How a held agent is driven: the two things a holder knows how to do. A row says which, by name (`by`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    Acp { program: Vec<String> },
}

/// A row written before there were two kinds says none: it is the first.
impl<'de> Deserialize<'de> for Held {
    fn deserialize<D: serde::Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "by", rename_all = "kebab-case")]
        enum Said {
            Lines {
                #[serde(default)]
                fresh: Vec<String>,
                #[serde(default)]
                resume: Vec<String>,
                line: String,
                #[serde(default)]
                stop: Option<String>,
            },
            Acp {
                program: Vec<String>,
            },
        }
        let mut row = serde_json::Value::deserialize(from)?;
        if let Some(fields) = row.as_object_mut() {
            fields.entry("by").or_insert_with(|| "lines".into());
        }
        Ok(match serde_json::from_value(row).map_err(serde::de::Error::custom)? {
            Said::Lines { fresh, resume, line, stop } => Held::Lines { fresh, resume, line, stop },
            Said::Acp { program } => Held::Acp { program },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_row_says_how_it_is_driven_and_an_older_one_is_the_first_kind() {
        let old: Held = serde_json::from_str(r#"{"fresh":[],"resume":[],"line":"{\"text\":{text}}"}"#).unwrap();
        assert!(matches!(old, Held::Lines { ref line, stop: None, .. } if line.contains("{text}")));
        let acp: Held = serde_json::from_str(r#"{"by":"acp","program":["opencode","acp"]}"#).unwrap();
        assert_eq!(acp, Held::Acp { program: vec!["opencode".into(), "acp".into()] });
        assert_eq!(serde_json::to_value(&acp).unwrap()["by"], "acp");
        // a kind a newer build added does not read here, and says so
        assert!(serde_json::from_str::<Held>(r#"{"by":"telepathy"}"#).is_err());
    }
}
