//! What the tower remembers, and the rules about it: who is on the radio and what they are called, the chat log,
//! what is unheard, and each radio's place in a call. No I/O and no model here; everything that changes these
//! goes through a method below, and the rules are tested at the bottom of this file.
//!
//! An agent is known by its id, which never changes. Its callsign is only what it is called: it can change (he
//! names the session), and a later agent can be given a callsign an earlier one had. So a message records the id
//! of the agent that said it, and every question about "which agent" is answered by id.
use super::*;
/// A folder's path as he would read it: "~/Companies/shop" inside his home folder, whole anywhere else.
pub(super) fn shown(at: &std::path::Path) -> String {
    let homes = [config::home().canonicalize().ok(), Some(config::home())];
    match homes.iter().flatten().find_map(|home| at.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => at.display().to_string(),
    }
}
#[derive(Default)]
pub(super) struct Inner {
    /// By the id each delivery of his words went out under, the chat lines it carried. The agent's own record
    /// names that id when it takes the words in: that, and nothing about the words, is how a line is marked read.
    pub(super) receipts: HashMap<String, Vec<String>>,
    /// The events lately taken in, by their own ids: one sent again is taken once (see [`intake::Taken`]).
    pub(super) events_taken: super::intake::Taken,
    /// What is known about the things at the web addresses in the chat.
    pub(super) links: links::Book,
    pub(super) sessions: Vec<Session>,
    pub(super) log: Vec<Entry>,
    /// The last changes to what is read out, as the log has them: for `airband status`.
    pub(super) aloud_trail: std::collections::VecDeque<String>,
    pub(super) seq: u64,
    /// The change count: see [`Entry::rev`].
    pub(super) rev: u64,
    /// His teams on this computer, by id: a folder's team by the folder as it really is on disk, one with no
    /// folder by an id made when it was. What a team is called is only shown and said; everything kept or sent
    /// about a team goes by its id.
    pub(super) teams: BTreeMap<String, Team>,
    /// This computer's name, as a phone paired with several shows it. Never kept.
    pub(super) here: String,
    /// The folders his coding tools have had sessions in, as last read from their records
    /// (`adapters::worked_all`): the folder, the tools, when. Never kept: it is read again every minute.
    pub(super) worked: Vec<(String, Vec<String>, u64)>,
    /// The agents the tower runs itself.
    pub(super) held: Vec<held::Kept>,
    /// What an agent of each launcher that says so itself last said it can be set to, by the launcher's name
    /// (`held::offers`).
    pub(super) menus: BTreeMap<String, airband_core::tools::Offers>,
    /// The coding tools he uses, by their rows' ids, the one he uses most first: as his phone last said.
    pub(super) tools: Vec<String>,
    /// Names agents go by on his other computers, as a radio last told it. None of this tower's numbered
    /// agents is called any of them.
    pub(super) elsewhere: Vec<Callsign>,
    /// Where each agent that was moved to another team was put (the team's id), by the agent's id. A session
    /// that goes away and comes back (the computer restarted, he resumed it) is put back there, not in its
    /// folder's team again.
    pub(super) placed: BTreeMap<String, (String, Mover)>,
    /// Each radio that has been heard from, by the id it gives.
    pub(super) calls: HashMap<String, Call>,
    /// When each open message was last put in front of him on a call, and how many times: for reminders.
    pub(super) offered: HashMap<String, (u32, Instant)>,
    /// Ids of messages that are new or changed since they were last written to the database.
    pub(super) unsaved: std::cell::RefCell<HashSet<String>>,
    /// What each kept table held when it was last handed to be written, as a sum of its writing: a table is
    /// written again only when this differs (see [`Inner::changes`]).
    pub(super) written: HashMap<&'static str, u64>,
    /// Ids of messages whose short version is being made right now, so none is made twice.
    pub(super) judging: HashSet<String>,
    /// Requests for permission that are waiting for his answer, or have just had it, by id.
    pub(super) permits: HashMap<String, Permit>,
    /// Ids of the lines a notification has been decided for, so each is decided once.
    pub(super) pushed: HashSet<String>,
    /// How many requests for the whole chat are open right now: while there is one, it reads what agents say.
    pub(super) readers: u32,
    /// Ids of things he said that this tower is working out an answer to right now.
    pub(super) answering: HashSet<String>,
    /// What has come of the last things he said, for telling the radio that said each in its feed: the change
    /// count it was settled at, the radio, and the outcome under the utterance's id.
    pub(super) settled: VecDeque<(u64, String, airband_core::radio::Settled)>,
}

mod agents;
mod tables;
pub(super) use tables::*;

#[cfg(test)]
mod tests;
