//! Each room's colour (a team on one computer, by its id), chosen here, on the phone, across every paired computer: two teams he can see at once are
//! not given the same colour by two computers that each chose alone. A team keeps its colour from then on, kept
//! in `colours.json` in the app's folder, so a notification shows it in the colour the chat does.
use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How many team colours a host has (`Palette.team` in the app).
pub(crate) const COLOURS: u8 = 6;

#[derive(Default)]
pub(crate) struct Colours {
    /// Where they are kept. None for a desk that keeps nothing (a test).
    path: Option<PathBuf>,
    by_team: BTreeMap<String, u8>,
}

impl Colours {
    pub(crate) fn open(dir: &str) -> Colours {
        let path = (!dir.is_empty()).then(|| std::path::Path::new(dir).join("colours.json"));
        let by_team = path.as_deref().and_then(crate::store::kept).unwrap_or_default();
        Colours { path, by_team }
    }
    /// Teams seen in a computer's chat: one seen for the first time is given the colour the fewest teams have,
    /// the lowest of those when several tie.
    pub(crate) fn see(&mut self, teams: impl IntoIterator<Item = String>) {
        let mut changed = false;
        for team in teams {
            if team.is_empty() || self.by_team.contains_key(&team) {
                continue;
            }
            let next = (0..COLOURS).min_by_key(|c| self.by_team.values().filter(|have| *have == c).count()).unwrap_or(0);
            self.by_team.insert(team, next);
            changed = true;
        }
        if changed && let Some(path) = &self.path {
            let _ = airband_core::kept::save(path, &self.by_team);
        }
    }
    /// A room's colour: its place in the host's list of team colours. Nothing for one not seen yet.
    pub(crate) fn of(&self, room: &str) -> Option<u8> {
        self.by_team.get(room).copied()
    }
    pub(crate) fn all(&self) -> HashMap<String, u8> {
        self.by_team.iter().map(|(t, c)| (t.clone(), *c)).collect()
    }
}
