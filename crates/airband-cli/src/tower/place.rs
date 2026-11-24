//! Which team an agent is in. A session joins the team of the folder it was started in, and that is sometimes
//! not the project it is working on: he opened it in one project and then had it work on another. He can move
//! one himself from a radio, and it stays where he put it. The tower also looks, on some of an agent's reports,
//! at the files and commands of its latest steps, and has a model say which project that work is part of; one
//! that is plainly working somewhere else is moved, and he is told.
#![expect(clippy::disallowed_macros, reason = "the model's answer shape")]
use super::*;

/// He moved an agent to a team, from a radio: one of his teams by its id, or a new one he named. Returns what it
/// is called now, and the team it is in: what that is called and its id.
pub(super) fn by_him(app: &App, agent: &str, to: &airband_core::radio::TeamPick) -> Result<airband_core::radio::Moved, String> {
    let mut inner = app.change();
    if inner.find(agent).is_none() {
        return Err(NOT_RUNNING.into());
    }
    let team = match to {
        airband_core::radio::TeamPick::Team { id } if inner.teams.contains_key(id) => id.clone(),
        airband_core::radio::TeamPick::Team { .. } => return Err("That team is not on this computer any more.".into()),
        airband_core::radio::TeamPick::New { name } => inner.team_he_named(name)?,
    };
    inner.move_to(agent, &team, Mover::Him);
    inner.tell();
    inner.find(agent).map(|s| airband_core::radio::Moved { callsign: s.callsign.clone(), team: s.team.clone() }).ok_or_else(|| NOT_RUNNING.into())
}

/// How many of the folders his tools have worked in, newest first, the model is offered as projects.
const FOLDERS_SHOWN: usize = 20;

/// Is this report one to look at where the agent belongs on? Its first, so one in the wrong team is not left
/// there long; then a few more as it settles into its work; then now and then.
pub(super) fn due(reports: u32) -> bool {
    matches!(reports, 1 | 3 | 6) || (reports > 0 && reports.is_multiple_of(12))
}

/// Look at what an agent has been doing and move it if its work is plainly part of another project.
pub(super) async fn look(app: Arc<App>, agent_id: String) {
    if app.settings.reader().is_none() {
        return;
    }
    let (session, said, teams, folders) = {
        let inner = app.read();
        // one he placed himself is where he wants it
        let Some(s) = inner.find(&agent_id).filter(|s| s.moved != Some(Mover::Him)).cloned() else { return };
        let said: Vec<String> = inner.log.iter().rev().filter(|e| e.worth_from(&agent_id)).take(3).map(Entry::line).collect();
        // his teams by id, each with what it is called and its folder when it has one
        let teams: Vec<(TeamId, Team)> = inner.teams.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        // the folders his tools have worked in that no team works in yet, newest first: a project picked from
        // these becomes that folder's team, so a session he later opens in the folder joins it
        let folders: Vec<airband_core::radio::RecentFolder> = inner.recent().into_iter().filter(|r| r.team.is_none() && !r.name.is_empty()).take(FOLDERS_SHOWN).collect();
        (s, said, teams, folders)
    };
    let agent = session.agent.clone();
    let touched = tokio::task::spawn_blocking(move || crate::adapters::touched(&agent)).await.unwrap_or_default();
    // with nothing to go on but its words, it stays where it is
    if touched.is_empty() {
        return;
    }
    let team = session.callsign.project.clone();
    let team_id = session.team.clone();
    let prompt = format!(
        "{name} runs several coding agents, grouped into teams by the project each one works on. An agent starts in \
         the team of the folder it was opened in. Sometimes that is not the project it is working on: he opened it \
         in one project's folder and then had it work on another.\n\n\
         This agent is called \"{callsign}\", in the team \"{team}\". It was opened in {folder}.\n\
         The last things it reported, newest first:\n{said}\n\
         The files and commands of its latest steps, newest first:\n{touched}\n\n\
         His teams:\n{teams}\n\n\
         Folders of projects he has worked on that have no team yet:\n{folders}\n\n\
         Which project is this agent's work part of? Go by where it makes its changes: reading another project's \
         files, or using its data, is not working on it. If its work belongs in the team it is in, or you cannot \
         tell, answer with that team. If it is plainly working on a different project, answer with that project's \
         team when he has one; otherwise with that project's folder when it is listed; and otherwise with a short \
         lowercase name for the project, as its folder is called.\n\
         Reply with one line of JSON and nothing else: {{\"team\":<the number that team is listed under, or null>,\"folder\":<the number that folder is listed under, or null>,\"new_team\":<a name for a project that is none of his teams and none of the folders, or null>,\"sure\":true or false}}\n\
         \"sure\": true only when its steps leave no doubt.",
        name = app.cfg().name,
        callsign = session.callsign.text(),
        folder = session.agent.cwd.as_deref().unwrap_or("a folder the tower does not know"),
        said = if said.is_empty() { "- nothing yet".to_string() } else { said.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n") },
        touched = touched.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n"),
        teams = teams.iter().enumerate().map(|(i, (_, t))| match &t.folder { Some(folder) => format!("{}. {} (in {folder})", i + 1, t.name), None => format!("{}. {}", i + 1, t.name) }).collect::<Vec<_>>().join("\n"),
        folders = if folders.is_empty() { "- none".to_string() } else { folders.iter().enumerate().map(|(i, f)| format!("{}. {} (in {})", i + 1, f.name, f.folder)).collect::<Vec<_>>().join("\n") },
    );
    let shape = json!({
        "type": "object",
        "properties": { "team": { "type": ["integer", "null"] }, "folder": { "type": ["integer", "null"] }, "new_team": { "type": ["string", "null"] }, "sure": { "type": "boolean" } },
        "required": ["team", "folder", "new_team", "sure"],
        "additionalProperties": false
    });
    let Ok(out) = judge::ask_json::<Out>(&app.settings, prompt, Some(shape)).await else { return };
    let mut inner = app.change();
    if settle(&mut inner, &agent_id, &team_id, &out, &teams, &folders) {
        inner.tell();
    }
}

/// What the model answered about where an agent's work belongs.
#[derive(Deserialize)]
struct Out {
    #[serde(default)]
    team: Option<usize>,
    #[serde(default)]
    folder: Option<usize>,
    #[serde(default)]
    new_team: Option<String>,
    #[serde(default)]
    sure: bool,
}

/// Move the agent where the model said, among the `teams` and `folders` it was shown, numbered from one; it was
/// in `was_in` when the model was asked. Whether it was moved.
fn settle(inner: &mut Inner, agent_id: &str, was_in: &TeamId, out: &Out, teams: &[(TeamId, Team)], folders: &[airband_core::radio::RecentFolder]) -> bool {
    if !out.sure {
        return false;
    }
    // he may have moved it himself while the model was reading
    if inner.find(agent_id).is_none_or(|s| s.moved == Some(Mover::Him) || s.team != *was_in) {
        return false;
    }
    // One of his teams, or a folder of his, is picked by its number and never by a name. A folder picked becomes
    // its own team (the one already there, if one was made meanwhile), so what he opens there later joins this
    // agent: a team the model only named has no folder, and a session opened in the project's folder would make
    // a second team for it. A new team only by a name no team has, and not as his: the model named it.
    let listed = judge::listed(out.team, teams).map(|(id, _)| id.clone()).filter(|id| inner.teams.contains_key(id));
    let to = listed.or_else(|| judge::listed(out.folder, folders).and_then(|f| inner.folder_team(&f.folder)));
    let Some(to) = to.or_else(|| out.new_team.as_deref().and_then(|n| inner.new_team(n, false).ok())) else { return false };
    let Some((was, _)) = inner.move_to(agent_id, &to, Mover::Tower) else { return false };
    // said in the chat and not aloud: the name he knew it by has changed
    if let Some(s) = inner.find(agent_id).cloned() {
        // airband's own note in the chat, under its new name: never the agent's words, and a question it has open
        // stays open
        let note = Entry { agent: None, note: Some(airband_core::radio::Note::Moved), ..Entry::said(config::random_hex(8), &s, Some(Kind::Report), was.text()) };
        inner.record(Entry { short: Some(note.said_whole()), ..note });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, cwd: &str) -> airband_core::agent::Agent {
        airband_core::agent::Agent { cwd: Some(cwd.into()), ..crate::adapters::agent_with(id, "test") }
    }

    /// The case on his laptop: a session opened in one project was found to be working on another, whose folder
    /// had no team yet. Later he opened two sessions in that project's folder. They join the agent already working
    /// on it, in one team, and not a second team for the same project called apart from the first.
    #[test]
    fn work_found_in_a_folder_with_no_team_makes_that_folders_team_and_sessions_opened_there_join_it() {
        let here = std::env::temp_dir().join(format!("airband-place-{}", config::random_hex(6)));
        let folder = |name: &str| {
            std::fs::create_dir_all(here.join(name)).unwrap();
            // as the tower keeps a folder: by its real path
            std::fs::canonicalize(here.join(name)).unwrap().display().to_string()
        };
        let (lionpal, arcady) = (folder("lionpal"), folder("arcady"));
        let mut inner = Inner::default();
        inner.check_in(&agent("biking", &lionpal)).unwrap();
        inner.worked = vec![(arcady.clone(), vec!["some-tool".into()], 9), (lionpal.clone(), vec!["some-tool".into()], 5)];
        let was_in = inner.find("biking").unwrap().team.clone();
        let teams: Vec<(TeamId, Team)> = inner.teams.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        let folders: Vec<_> = inner.recent().into_iter().filter(|r| r.team.is_none()).collect();
        assert_eq!(folders.iter().map(|f| f.folder.as_str()).collect::<Vec<_>>(), [arcady.as_str()], "the folder with no team is offered");
        // the model picks the project's folder by its number
        assert!(settle(&mut inner, "biking", &was_in, &Out { team: None, folder: Some(1), new_team: None, sure: true }, &teams, &folders));
        let team = inner.find("biking").unwrap().team.clone();
        assert_eq!(inner.teams[&team].folder.as_deref(), Some(arcady.as_str()), "the team it is in now is the folder's own");
        assert_eq!(inner.find("biking").unwrap().callsign.project, "arcady");
        // two sessions he opens in that folder, one he names: both in that team
        inner.check_in(&agent("two", &arcady)).unwrap();
        inner.check_in(&airband_core::agent::Agent { title: Some("arcady biking three".into()), ..agent("three", &arcady) }).unwrap();
        for id in ["two", "three"] {
            assert_eq!(inner.find(id).unwrap().team, team, "{id} joins the team of the agent working there");
        }
        assert_eq!(inner.teams.len(), 2, "no second team for the same project");
        let _ = std::fs::remove_dir_all(&here);
    }

    /// One of his teams is picked over a folder, and a name only when nothing listed fits; what he placed himself
    /// stays.
    #[test]
    fn a_team_is_picked_by_its_number_and_one_he_placed_stays() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop")).unwrap();
        inner.check_in(&agent("b", "/x/garden")).unwrap();
        let (shop, garden) = (inner.team_at("/x/shop").unwrap(), inner.team_at("/x/garden").unwrap());
        let teams: Vec<(TeamId, Team)> = inner.teams.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        let n = teams.iter().position(|(id, _)| *id == garden).unwrap() + 1;
        assert!(!settle(&mut inner, "a", &shop, &Out { team: Some(n), folder: None, new_team: None, sure: false }, &teams, &[]), "not sure: it stays");
        assert!(settle(&mut inner, "a", &shop, &Out { team: Some(n), folder: None, new_team: Some("ignored".into()), sure: true }, &teams, &[]));
        assert_eq!(inner.find("a").unwrap().team, garden);
        inner.move_to("b", &shop, Mover::Him);
        assert!(!settle(&mut inner, "b", &shop, &Out { team: Some(n), folder: None, new_team: None, sure: true }, &teams, &[]), "he placed it");
    }
}
