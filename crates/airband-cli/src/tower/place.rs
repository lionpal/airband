//! Which team an agent is in. A session joins the team of the folder it was started in, and that is sometimes
//! not the project it is working on: he opened it in one project and then had it work on another. He can move
//! one himself from a radio, and it stays where he put it. The tower also looks, on some of an agent's reports,
//! at the files and commands of its latest steps, and has a model say which project that work is part of; one
//! that is plainly working somewhere else is moved, and he is told.
use super::*;

/// He moved an agent to a team, from a radio: one of his teams by its id, or a new one he named. Returns what it
/// is called now, and the team it is in: what that is called and its id.
pub(super) fn by_him(app: &App, agent: &str, to: &airband_core::radio::TeamPick) -> Result<airband_core::radio::Moved, String> {
    let mut inner = app.change();
    if inner.find(agent).is_none() {
        return Err("That agent is no longer running.".into());
    }
    let team = match to {
        airband_core::radio::TeamPick::Team { id } if inner.teams.contains_key(id) => id.clone(),
        airband_core::radio::TeamPick::Team { .. } => return Err("That team is not on this computer any more.".into()),
        airband_core::radio::TeamPick::New { name } => inner.team_he_named(name)?,
    };
    inner.move_to(agent, &team, Mover::Him);
    inner.tell();
    inner.find(agent).map(|s| airband_core::radio::Moved { callsign: s.callsign.clone(), team: s.team.clone() }).ok_or_else(|| "That agent is no longer running.".into())
}

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
    let (session, said, teams) = {
        let inner = app.read();
        // one he placed himself is where he wants it
        let Some(s) = inner.find(&agent_id).filter(|s| s.moved != Some(Mover::Him)).cloned() else { return };
        let said: Vec<String> = inner.log.iter().rev().filter(|e| e.worth_from(&agent_id)).take(3).map(Entry::line).collect();
        // his teams by id, each with what it is called and its folder when it has one
        let teams: Vec<(String, Team)> = inner.teams.iter().map(|(id, t)| (id.clone(), t.clone())).collect();
        (s, said, teams)
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
         Which project is this agent's work part of? Go by where it makes its changes: reading another project's \
         files, or using its data, is not working on it. If its work belongs in the team it is in, or you cannot \
         tell, answer with that team. If it is plainly working on a different project, answer with that project's \
         team when he has one, and otherwise with a short lowercase name for the project, as its folder is called.\n\
         Reply with one line of JSON and nothing else: {{\"team\":<the number that team is listed under, or null>,\"new_team\":<a name for a project that is none of his teams, or null>,\"sure\":true or false}}\n\
         \"sure\": true only when its steps leave no doubt.",
        name = app.cfg().name,
        callsign = session.callsign.text(),
        folder = session.agent.cwd.as_deref().unwrap_or("a folder the tower does not know"),
        said = if said.is_empty() { "- nothing yet".to_string() } else { said.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n") },
        touched = touched.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n"),
        teams = teams.iter().enumerate().map(|(i, (_, t))| match &t.folder { Some(folder) => format!("{}. {} (in {folder})", i + 1, t.name), None => format!("{}. {}", i + 1, t.name) }).collect::<Vec<_>>().join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        #[serde(default)]
        team: Option<usize>,
        #[serde(default)]
        new_team: Option<String>,
        #[serde(default)]
        sure: bool,
    }
    let shape = json!({
        "type": "object",
        "properties": { "team": { "type": ["integer", "null"] }, "new_team": { "type": ["string", "null"] }, "sure": { "type": "boolean" } },
        "required": ["team", "new_team", "sure"],
        "additionalProperties": false
    });
    let Ok(out) = judge::ask_json::<Out>(&app.settings, "placing", prompt, Some(shape)).await else { return };
    if !out.sure {
        return;
    }
    // one of his teams by the number it was listed under, or a new one by the name the model gave it
    let listed = judge::listed(out.team, &teams).map(|(id, _)| id.clone());
    let mut inner = app.change();
    // he may have moved it himself while the model was reading
    if inner.find(&agent_id).is_none_or(|s| s.moved == Some(Mover::Him) || s.team != team_id) {
        return;
    }
    // a new team only by a name no team has, and not as his: the model named it; one of his teams is picked by
    // its number and never by a name
    let Some(to) = listed.filter(|id| inner.teams.contains_key(id)).or_else(|| out.new_team.as_deref().and_then(|n| inner.new_team(n, false).ok())) else { return };
    let Some((was, _)) = inner.move_to(&agent_id, &to, Mover::Tower) else { return };
    // said in the chat and not aloud: the name he knew it by has changed
    if let Some(s) = inner.find(&agent_id).cloned() {
        // airband's own note in the chat, under its new name: never the agent's words, and a question it has open
        // stays open
        let note = Entry { agent: None, note: Some(airband_core::radio::Note::Moved), ..Entry::said(config::random_hex(8), &s, Some(Kind::Report), was.text()) };
        inner.record(Entry { short: Some(note.said_whole()), ..note });
    }
    inner.tell();
}
