//! `airband hold`: keeps one agent's program running, apart from the tower, and drives it.
//!
//! The tower starts one of these for each agent it runs itself, and then only ever talks to it through a
//! socket. So the agent does not end when the tower does: a tower that is updated or restarted finds the socket
//! again and carries on, and whatever the agent was in the middle of goes on undisturbed.
//!
//!   airband hold --sock <path> --cwd <folder> --id <agent id> --harness <tool> --by <lines|acp> [--model <name>] [--effort <name>] […] -- <program> [arguments...]
//!
//! What the tower says to it is one JSON line a connection, the same whatever the program is ([`Tell`]): his
//! words for the agent, stop what you are doing, finish, and how are you. How that reaches the program is the
//! driver's business (`--by`, the two kinds a launcher's row can name, `airband_core::tools::Held`):
//!
//! - `lines`: the program reads one message a line on its standard input (`--line` is what a line looks like,
//!   `--stop` the line that stops it) and reports to the tower by itself.
//! - `acp`: the program speaks the Agent Client Protocol; `acp.rs` speaks it back and reports for the agent.
//!   The agent says what models and efforts it can be set to when its session opens. `--model` and `--effort`
//!   name the ones he asked for, and an agent that has no such one never starts.
//!
//! To "how are you" it answers where it has got to ([`State`]): still starting, ready, or failed and why, with
//! the last of what the program printed as an error. The tower tells him an agent is there only once it is.
//!
//! What the two say to each other is here, the same on every system; the holder itself (`holder.rs`) runs on
//! unix only.
#[cfg(unix)]
mod holder;
#[cfg(unix)]
pub(crate) use holder::{Held, Msg, run};

/// What the tower says to a holder.
pub(crate) enum Tell {
    /// His words, as the agent's next turn, and the id this delivery of them goes under.
    Say(String, String),
    /// Leave off what you are doing.
    Stop,
    /// Finish: the program's input is closed, and it ends.
    End,
}

/// One line from the tower to a holder, as it is written on the socket: typed both ways.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "line", rename_all = "kebab-case")]
enum ToHolder {
    Say { text: String, receipt: String },
    Stop,
    End,
    /// Where it has got to: answered with a [`State`].
    Status,
}

impl Tell {
    pub(crate) fn line(&self) -> String {
        let line = match self {
            Tell::Say(text, receipt) => ToHolder::Say { text: text.clone(), receipt: receipt.clone() },
            Tell::Stop => ToHolder::Stop,
            Tell::End => ToHolder::End,
        };
        serde_json::to_string(&line).expect("a line to a holder writes as JSON")
    }
}
/// A holder's answer to anything but a status ask: whether it took what it was told. The tower counts words as
/// given only on `Taken`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub(crate) enum Answer {
    Taken,
    Refused { why: String },
}

/// The line that asks a holder where it has got to. The answer is a [`State`].
pub(crate) fn status_line() -> String {
    serde_json::to_string(&ToHolder::Status).expect("a line to a holder writes as JSON")
}

/// Where a holder has got to with its program.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub(crate) enum State {
    Starting,
    /// The agent is there and takes his words. `session` is what the agent's own tool calls its session, when
    /// that is not the id the tower gave it: what carries it on later.
    /// `offers` is what the agent itself said a session of its can be set to, when it says.
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        #[serde(default)]
        offers: airband_core::tools::Offers,
    },
    /// It did not start, and never will: why, in the program's own words where it had any.
    Failed { why: String },
    /// It did not start because the agent has no such setting as he asked for: airband's own refusal, which
    /// he is told.
    NoSuch(NoSuch),
}

/// Which of an agent's settings he asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SettingKind {
    Model,
    Effort,
}

/// A setting he asked for that the agent does not have, with what the agent does have (its own ids for them;
/// none when it has no such setting at all).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NoSuch {
    pub(crate) setting: SettingKind,
    pub(crate) asked: String,
    #[serde(default)]
    pub(crate) can_be: Vec<String>,
}

impl NoSuch {
    /// Why it cannot be had, in words for him. What it can be is named when that is few enough to say.
    pub(crate) fn said(&self) -> String {
        let (noun, what) = match self.setting {
            SettingKind::Model => ("model", "a model"),
            SettingKind::Effort => ("effort", "an effort"),
        };
        let asked = &self.asked;
        match self.can_be.len() {
            0 => format!("it cannot be given {what}"),
            1..=8 => format!("it has no {noun} called {asked}. It can be {}", self.can_be.join(", ")),
            _ => format!("it has no {noun} called {asked}"),
        }
    }
}

/// Why a holder's program never became an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotStarted {
    /// The agent has no such setting as he asked for: he is told so.
    NoSuch(NoSuch),
    /// airband's own refusal, in its own words: he is told so.
    Told(String),
    /// The program or its holder failed: why, which can be the program's own words, for the log alone.
    Failed(String),
}

impl From<String> for NotStarted {
    fn from(why: String) -> NotStarted {
        NotStarted::Failed(why)
    }
}
impl From<&str> for NotStarted {
    fn from(why: &str) -> NotStarted {
        NotStarted::Failed(why.to_string())
    }
}
impl std::fmt::Display for NotStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotStarted::NoSuch(no) => f.write_str(&no.said()),
            NotStarted::Told(why) | NotStarted::Failed(why) => f.write_str(why),
        }
    }
}

/// How a holder drives its program: the two kinds a launcher's row can name (`airband_core::tools::Held`), by
/// the word that row's `by` uses. The one place the word on the command line is read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum By {
    Lines,
    Acp,
}
impl By {
    /// The kind a row of this shape names.
    pub(crate) fn of(held: &airband_core::tools::Held) -> By {
        match held {
            airband_core::tools::Held::Lines { .. } => By::Lines,
            airband_core::tools::Held::Acp { .. } => By::Acp,
        }
    }
    /// As `--by` gives it: the row's own word for the kind.
    pub(crate) fn word(self) -> &'static str {
        match self {
            By::Lines => "lines",
            By::Acp => "acp",
        }
    }
}
