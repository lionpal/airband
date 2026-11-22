//! Each room's colour (a team on one computer, by its id), chosen here, on the phone, across every paired computer: two teams he can see at once are
//! not given the same colour by two computers that each chose alone. A team keeps its colour from then on, kept
//! in `colours.json` in the app's folder, so a notification shows it in the colour the chat does.
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Colours {
    /// Where they are kept: nowhere for a desk that keeps nothing (a test).
    keeper: airband_io::kept::Keeper,
    /// How many colours the host draws teams with ([`HostDevice::team_colours`]).
    count: u8,
    by_team: BTreeMap<String, u8>,
    /// The rooms seen since the app opened: a new one is given the colour the fewest of these have. Rooms kept
    /// from before that are not seen again (a team gone, a computer paired again) count for nothing.
    seen: std::collections::BTreeSet<String>,
    /// What he is to be told about the file when the desk opens: it could not be read.
    pub(crate) opened: Option<String>,
}

impl Colours {
    pub(crate) fn open(dir: &str, count: u8) -> Colours {
        let (by_team, keeper, opened) = crate::kept::open(crate::kept::file(dir, "colours.json"), "Team colours");
        Colours { keeper, count, by_team, seen: Default::default(), opened }
    }
    /// Teams seen in a computer's chat: one seen for the first time is given the colour the fewest teams on
    /// his screen have, the lowest of those when several tie. The words to tell him when the colours could
    /// not be kept.
    #[must_use]
    pub(crate) fn see(&mut self, teams: impl IntoIterator<Item = String>) -> Option<String> {
        let mut changed = false;
        for team in teams {
            if team.is_empty() {
                continue;
            }
            self.seen.insert(team.clone());
            if self.by_team.contains_key(&team) {
                continue;
            }
            let on_screen = |c: &u8| self.seen.iter().filter(|t| self.by_team.get(*t) == Some(c)).count();
            // a host with no colours gives none
            let Some(next) = (0..self.count).min_by_key(on_screen) else { continue };
            self.by_team.insert(team, next);
            changed = true;
        }
        if !changed {
            return None;
        }
        crate::kept::told(self.keeper.save(&self.by_team), "Team colours may change when the app opens again.")
    }
    /// Let go of the colour of every room not in `rooms`: what every paired computer has now, once each has
    /// answered, so the file holds only teams that are still there. The words to tell him when it could not be kept.
    #[must_use]
    pub(crate) fn keep_only(&mut self, rooms: &std::collections::BTreeSet<String>) -> Option<String> {
        let before = self.by_team.len();
        self.by_team.retain(|room, _| rooms.contains(room));
        if self.by_team.len() == before {
            return None;
        }
        crate::kept::told(self.keeper.save(&self.by_team), "Team colours may change when the app opens again.")
    }
    /// A room's colour: its place in the host's list of team colours. Nothing for one not seen yet.
    pub(crate) fn of(&self, room: &str) -> Option<u8> {
        self.by_team.get(room).copied()
    }
    pub(crate) fn all(&self) -> HashMap<String, u8> {
        self.by_team.iter().map(|(t, c)| (t.clone(), *c)).collect()
    }
}
