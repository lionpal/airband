//! A team's name: as he chose it, as a folder suggests it, and with a computer's name in front.
use super::*;

/// The most characters a team's name may have: it is part of what its agents are called, and is said aloud.
pub(crate) const TEAM_NAME_MOST: usize = 24;

/// A name he chose for a team (a new one, or a rename), as it is kept: [`team_name`], and short. The one check
/// every place he names a team goes through.
pub fn chosen_team_name(text: &str) -> Result<String, String> {
    team_name(text).filter(|n| n.chars().count() <= TEAM_NAME_MOST).ok_or_else(|| format!("A team\u{2019}s name is letters and numbers, {TEAM_NAME_MOST} at most."))
}

/// A name he gave a team, written the way a callsign writes it. Only written so: no part of it is taken for a
/// path ("shop/front" is "shop front"). None when it has no letters or numbers.
pub fn team_name(text: &str) -> Option<String> {
    Some(norm(text)).filter(|name| !name.is_empty())
}

/// The names a team in this folder could take, plainest first: the folder's own name, then its parent's name
/// in front of it, and so on up the path ("shop", "work shop", "a work shop"). Each comes from the path's own
/// parts, as the system reads them, and is cut to the length every team's name keeps to ([`fit`]). The tower
/// takes the first that no other team of his has.
pub fn team_names(folder: &str) -> Vec<String> {
    let parts: Vec<String> = std::path::Path::new(folder.trim()).components().filter_map(|c| match c {
        std::path::Component::Normal(part) => Some(norm(&part.to_string_lossy())),
        _ => None,
    }).filter(|part| !part.is_empty()).collect();
    let mut names: Vec<String> = Vec::new();
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
pub fn team_on(computer: &str, team: &str) -> String {
    let (team, computer) = (fit(team), norm(computer));
    let words: Vec<&str> = computer.split(' ').filter(|w| !w.is_empty()).collect();
    (1..=words.len()).rev().map(|n| norm(&format!("{} {team}", words[..n].join(" ")))).find(|on| on.chars().count() <= TEAM_NAME_MOST).unwrap_or(team)
}

/// A name cut to the length every team's name keeps to: its last words, as many as fit, since the last is the
/// folder's own; the first characters of the last word when even that one is longer.
fn fit(name: &str) -> String {
    let words: Vec<&str> = name.split(' ').filter(|w| !w.is_empty()).collect();
    let mut kept: Vec<&str> = Vec::new();
    for word in words.iter().rev() {
        let with = kept.iter().map(|w| w.chars().count() + 1).sum::<usize>() + word.chars().count();
        if with > TEAM_NAME_MOST {
            break;
        }
        kept.insert(0, word);
    }
    match (kept.is_empty(), words.last()) {
        (true, Some(last)) => last.chars().take(TEAM_NAME_MOST).collect(),
        _ => kept.join(" "),
    }
}
