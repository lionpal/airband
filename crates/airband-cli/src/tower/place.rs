//! Which team an agent is in. A session joins the team of the folder it was started in, and that is sometimes
//! not the project it is working on: he opened it in one project and then had it work on another. He can move
//! one himself from a radio, and it stays where he put it. The tower also looks, on some of an agent's reports,
//! at the files and commands of its latest steps, and has a model say which project that work is part of; one
//! that is plainly working somewhere else is moved, and he is told.
use super::*;

/// He moved an agent to a team, from a radio. Returns what it is called now and the team it is in.
pub(super) fn by_him(app: &App, agent: &str, team: &str) -> Result<(String, String), String> {
    let mut inner = app.inner.lock().unwrap();
    if inner.find(agent).is_none() {
        return Err("That agent is no longer running.".into());
    }
    if airband_core::routing::Callsign::next_for(team, &[]).project == "session" && team.trim().to_lowercase() != "session" {
        return Err("Give the team a name.".into());
    }
    inner.move_to(agent, team, Mover::Him);
    app.save(&inner);
    app.kick.notify_one();
    Ok(inner.find(agent).map(|s| (s.callsign.text(), s.callsign.project.clone())).unwrap_or_default())
}

/// Is this report one to look at where the agent belongs on? Its first, so one in the wrong team is not left
/// there long; then a few more as it settles into its work; then now and then.
pub(super) fn due(reports: u32) -> bool {
    matches!(reports, 1 | 3 | 6) || (reports > 0 && reports % 12 == 0)
}

/// Look at what an agent has been doing and move it if its work is plainly part of another project.
pub(super) async fn look(app: Arc<App>, agent_id: String) {
    if matches!(app.cfg.summarizer, Summarizer::None) {
        return;
    }
    let (session, said, teams) = {
        let inner = app.inner.lock().unwrap();
        // one he placed himself is where he wants it
        let Some(s) = inner.find(&agent_id).filter(|s| s.moved != Some(Mover::Him)).cloned() else { return };
        let said: Vec<String> = inner.log.iter().rev().filter(|e| e.agent.as_deref() == Some(agent_id.as_str()) && e.kind != Some(Kind::Fyi)).take(3).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
        // his teams, each with its folder when the tower knows it
        let folders = inner.launchable(&app.cfg);
        let mut teams: Vec<String> = inner.sessions.iter().map(|s| s.callsign.project.clone()).chain(folders.keys().cloned()).collect();
        teams.sort();
        teams.dedup();
        let teams: Vec<String> = teams.into_iter().map(|t| match folders.get(&t) { Some(folder) => format!("- {t} (in {folder})"), None => format!("- {t}") }).collect();
        (s, said, teams)
    };
    let agent = session.agent.clone();
    let touched = tokio::task::spawn_blocking(move || crate::adapters::touched(&agent)).await.unwrap_or_default();
    // with nothing to go on but its words, it stays where it is
    if touched.is_empty() {
        return;
    }
    let team = session.callsign.project.clone();
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
         Reply with one line of JSON and nothing else: {{\"team\":\"<name>\",\"sure\":true or false}}\n\
         \"sure\": true only when its steps leave no doubt.",
        name = app.cfg.name,
        callsign = session.callsign.text(),
        folder = session.agent.cwd.as_deref().unwrap_or("a folder the tower does not know"),
        said = if said.is_empty() { "- nothing yet".to_string() } else { said.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n") },
        touched = touched.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n"),
        teams = teams.join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        #[serde(default)]
        team: String,
        #[serde(default)]
        sure: bool,
    }
    let answer = ask_model(&app.cfg, prompt.clone()).await;
    if std::env::var("AIRBAND_TRACE").is_ok() {
        log(format!("placing asked:\n{prompt}\nand was told: {answer:?}"));
    }
    let Some(out) = answer.ok().and_then(|out| serde_json::from_str::<Out>(out.get(out.find('{')?..=out.rfind('}')?)?).ok()) else { return };
    if !out.sure {
        return;
    }
    let mut inner = app.inner.lock().unwrap();
    // he may have moved it himself while the model was reading
    if inner.find(&agent_id).is_none_or(|s| s.moved == Some(Mover::Him) || s.callsign.project != team) {
        return;
    }
    let Some((was, now)) = inner.move_to(&agent_id, &out.team, Mover::Tower) else { return };
    // said in the chat, quietly: the name he knew it by has changed
    if let Some(s) = inner.find(&agent_id).cloned() {
        let line = format!("{} is now {}: it is working on {}.", was.spoken(), now.text(), now.project);
        inner.record(Entry { short: Some(line.clone()), ..Entry::said(config::random_hex(8), &s, Some(Kind::Fyi), line) });
    }
    app.save(&inner);
    app.kick.notify_one();
}
