//! The tables of the tower's state that are kept from one run to the next, and the shape they were kept in.
use super::*;

/// One of his teams: what it is called, and the folder its agents work in when it has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::tower) struct Team {
    pub(in crate::tower) name: TeamName,
    #[serde(default)]
    pub(in crate::tower) folder: Option<String>,
    /// He chose this name (renamed it, made it from a radio, started an agent under it). It keeps it though a
    /// team on another of his computers has it. A name the tower, the configuration or a model gave is not his.
    #[serde(default)]
    pub(in crate::tower) named: bool,
}
impl Team {
    /// Its folder, when it has one that is still there.
    pub(in crate::tower) fn live_folder(&self) -> Option<String> {
        self.folder.clone().filter(|f| std::path::Path::new(f).is_dir())
    }
}
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Saved {
    pub(in crate::tower) sessions: Vec<Session>,
    #[serde(default)]
    pub(in crate::tower) teams: BTreeMap<TeamId, Team>,
    /// Kept by a tower from before teams had ids: teams by name with their folders, and the names he gave.
    /// Read once, by `settle_teams`, and never written again.
    #[serde(default, skip_serializing)]
    pub(in crate::tower) projects: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    pub(in crate::tower) named_teams: BTreeSet<String>,
    #[serde(default)]
    pub(in crate::tower) held: Vec<held::Kept>,
    #[serde(default)]
    pub(in crate::tower) menus: BTreeMap<String, airband_core::tools::Offers>,
    #[serde(default)]
    pub(in crate::tower) tools: Vec<String>,
    #[serde(default)]
    pub(in crate::tower) placed: BTreeMap<AgentId, (TeamId, Mover)>,
}

/// The tables kept from one run to the next, each under its own key in the database's state table, so one that
/// cannot be read is set aside alone and a change to one writes only that one.
pub(in crate::tower) const KEPT: [&str; 6] = ["sessions", "teams", "held", "menus", "tools", "placed"];

/// What is to be written after a change to the state: the kept tables whose contents changed, and the messages
/// that are new or changed. Made while the state is held, written by the saver after it is let go.
#[derive(Default)]
pub(in crate::tower) struct Batch {
    pub(in crate::tower) tables: Vec<(&'static str, String)>,
    pub(in crate::tower) messages: Vec<Entry>,
}
impl Batch {
    pub(in crate::tower) fn is_empty(&self) -> bool {
        self.tables.is_empty() && self.messages.is_empty()
    }
    /// This batch with a later one on top: a table or message in both is taken from the later one.
    pub(in crate::tower) fn then(mut self, later: Batch) -> Batch {
        for (key, text) in later.tables {
            self.tables.retain(|(k, _)| *k != key);
            self.tables.push((key, text));
        }
        for e in later.messages {
            self.messages.retain(|m| m.seq != e.seq);
            self.messages.push(e);
        }
        self
    }
}

impl Inner {
    /// Everything changed since the last time this was asked: each kept table whose contents differ from what
    /// was last written, and the messages touched since. A table counts as changed by what it holds, not by who
    /// changed it or how it was held, so no way of changing one can leave it unwritten.
    pub(in crate::tower) fn changes(&mut self) -> Batch {
        let tables = [
            ("sessions", serde_json::to_string(&self.sessions)),
            ("teams", serde_json::to_string(&self.teams)),
            ("held", serde_json::to_string(&self.held)),
            ("menus", serde_json::to_string(&self.menus)),
            ("tools", serde_json::to_string(&self.tools)),
            ("placed", serde_json::to_string(&self.placed)),
        ];
        let mut changed = Vec::new();
        for (key, text) in tables {
            match text {
                Ok(text) => {
                    let sum = sum_of(&text);
                    if self.written.insert(key, sum) != Some(sum) {
                        changed.push((key, text));
                    }
                }
                Err(e) => log(format!("database: '{key}' could not be put in writing ({e}); what was kept before stays")),
            }
        }
        let touched: HashSet<String> = self.unsaved.borrow_mut().drain().collect();
        let messages = self.log.iter().filter(|e| touched.contains(&e.id)).cloned().collect();
        Batch { tables: changed, messages }
    }
}

fn sum_of(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}
