//! The second half of the dispatcher: what his words are for (`utterance`, `understand`, `start`). What needs no reading comes first: what a
//! button sent as what it is (a verdict, a control, a new agent), the agents he picked, tags (airband-core's
//! parser), the message he tapped. Everything he says in his own words a model reads: which agent it is for
//! (a callsign he says, a team, all of them), or that he wants a new agent, where, and on which model and
//! effort.
use super::*;

mod start;
mod understand;
mod utterance;
use start::*;
use understand::*;
pub(super) use utterance::on_utterance;

pub(super) fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// How an untagged reply found its recipient.
pub(super) enum Routed {
    /// Certain: he named them, or the radio said which message he was answering.
    Sure(Vec<Session>),
    /// Nobody to choose from, or the choice could not be made: ask him, between these, likeliest first.
    Ask(Vec<Session>),
    /// It is not for anyone running: he wants a new agent started.
    New(Idea),
    /// It is for an agent on another of his computers.
    Elsewhere(airband_core::radio::Other),
}

pub(super) fn ago(ms: u64) -> String {
    match ms / 1000 {
        s if s < 90 => format!("{s} seconds"),
        s if s < 5400 => format!("{} minutes", s / 60),
        s => format!("{} hours", s / 3600),
    }
}
