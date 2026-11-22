//! A team's name: as he chose it, as a folder suggests it, and with a computer's name in front.
use super::*;

/// The most characters a team's name may have: it is part of what its agents are called, and is said aloud.
pub(crate) const TEAM_NAME_MOST: usize = 24;

/// A team's name as it is kept: written the way a callsign writes it, never empty, and [`TEAM_NAME_MOST`]
/// characters at most. Every team name is made by [`TeamName::new`] (one he, a model or the configuration chose)
/// or by [`team_names`] and [`team_on`] (one a folder suggests), so a name that breaks the rule cannot be had.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TeamName(String);

impl TeamName {
    /// A name chosen for a team (a new one, or a rename), as it is kept. The one check every chosen name goes
    /// through; `Err` says what a name must be.
    pub fn new(text: &str) -> Result<TeamName, String> {
        team_name(text).filter(|n| n.chars().count() <= TEAM_NAME_MOST).map(TeamName).ok_or_else(|| format!("A team\u{2019}s name is letters and numbers, {TEAM_NAME_MOST} at most."))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
}
impl std::fmt::Display for TeamName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl PartialEq<str> for TeamName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}
impl PartialEq<&str> for TeamName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// A name written the way a callsign writes it, uncut. Only written so: no part of it is taken for a path
/// ("shop/front" is "shop front"). None when it has no letters or numbers.
pub(crate) fn team_name(text: &str) -> Option<String> {
    Some(norm(text)).filter(|name| !name.is_empty())
}

/// The names a team in this folder could take, plainest first: the folder's own name, then its parent's name
/// in front of it, and so on up the path ("shop", "work shop", "a work shop"). Each comes from the path's own
/// parts, as the system reads them, and is cut to the length every team's name keeps to ([`fit`]). The tower
/// takes the first that no other team of his has.
pub fn team_names(folder: &str) -> Vec<TeamName> {
    let parts: Vec<String> = std::path::Path::new(folder.trim()).components().filter_map(|c| match c {
        std::path::Component::Normal(part) => Some(norm(&part.to_string_lossy())),
        _ => None,
    }).filter(|part| !part.is_empty()).collect();
    let mut names: Vec<TeamName> = Vec::new();
    for name in (1..=parts.len()).map(|n| fit(&parts[parts.len() - n..].join(" "))) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A team's name with this computer's name in front: for a team whose folder's name a team on another of his
/// computers already has ("laptop shop"). As many of the computer's words as leave it within the length
/// every team's name keeps to; the team's name alone when not one does.
pub fn team_on(computer: &str, team: &TeamName) -> TeamName {
    let computer = norm(computer);
    let words: Vec<&str> = computer.split(' ').filter(|w| !w.is_empty()).collect();
    (1..=words.len()).rev().map(|n| norm(&format!("{} {team}", words[..n].join(" ")))).find(|on| on.chars().count() <= TEAM_NAME_MOST).map(TeamName).unwrap_or_else(|| team.clone())
}

/// A name cut to the length every team's name keeps to: its last words, as many as fit, since the last is the
/// folder's own; the first characters of the last word when even that one is longer.
fn fit(name: &str) -> TeamName {
    let words: Vec<&str> = name.split(' ').filter(|w| !w.is_empty()).collect();
    let mut kept: Vec<&str> = Vec::new();
    for word in words.iter().rev() {
        let with = kept.iter().map(|w| w.chars().count() + 1).sum::<usize>() + word.chars().count();
        if with > TEAM_NAME_MOST {
            break;
        }
        kept.insert(0, word);
    }
    TeamName(match (kept.is_empty(), words.last()) {
        (true, Some(last)) => last.chars().take(TEAM_NAME_MOST).collect(),
        _ => kept.join(" "),
    })
}
