//! Teams and folders: moving an agent, naming a team, walking the computer's folders, starting a team.
use super::*;

/// He moved an agent to another team.
pub(super) async fn team(State(app): State<Arc<App>>, Sent(to): Sent<at::Team>) -> Answer<at::Team> {
    place::by_him(&app, &to.agent, &to.to).map(Json).map_err(no)
}

/// He renamed one of his teams, from a radio.
pub(super) async fn team_rename(State(app): State<Arc<App>>, Sent(to): Sent<at::RenameTeam>) -> Answer<at::RenameTeam> {
    let mut inner = app.change();
    let team = inner.rename_team(&TeamId::new(to.team.as_str()), &to.to).map_err(no)?;
    inner.tell();
    Ok(Json(airband_core::radio::TeamRenamed { team: team.into_string() }))
}

/// A folder on this computer, as it really is on disk (links followed). With none named, his home folder:
/// that is where a walk starts, and from there he can go up to the top and into other disks.
pub(super) fn folder_at(path: &str) -> Result<std::path::PathBuf, String> {
    let at = if path.trim().is_empty() { config::home().canonicalize().map_err(|e| e.to_string())? } else { std::path::Path::new(path).canonicalize().map_err(|_| "That folder is not there.".to_string())? };
    if !at.is_dir() {
        return Err("That is not a folder.".into());
    }
    Ok(at)
}

/// What he is told when the computer refuses airband a folder: the system said so, and its settings decide it.
const FOLDER_REFUSED: &str = "This computer does not let airband open this folder. Its privacy settings decide which folders airband can open.";
/// What he is told when a folder does not open in time. Nothing says why, so he is told only that.
const FOLDER_SLOW: &str = "This folder did not open within five seconds.";

/// The folders inside one of his, for choosing where a new team works. Some computers ask him, at the computer,
/// the first time a program opens some of his folders, and the read waits for his answer: so it is done to one
/// side and given five seconds.
pub(super) async fn folders(State(app): State<Arc<App>>, Sent(ask): Sent<at::Folders>) -> Answer<at::Folders> {
    let read = tokio::task::spawn_blocking(move || {
        folder_at(&ask.under).and_then(|at| {
            let entries = std::fs::read_dir(&at).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FOLDER_REFUSED.to_string(),
                _ => "That folder cannot be opened from here.".to_string(),
            })?;
            let mut inside: Vec<airband_core::radio::Folder> = entries.flatten().filter(|e| e.path().is_dir())
                .filter_map(|e| Some(airband_core::radio::Folder { name: e.file_name().into_string().ok()?, path: e.path().display().to_string() }))
                .filter(|f| !f.name.starts_with('.')).collect();
            inside.sort_by_key(|f| f.name.to_lowercase());
            Ok(airband_core::radio::Folders { path: at.display().to_string(), shown: shown(&at), up: at.parent().map(|p| p.display().to_string()), folders: inside, team: String::new(), has_team: false })
        })
    });
    match tokio::time::timeout(Duration::from_secs(5), read).await {
        Ok(Ok(Ok(mut f))) => {
            // the team that works here, or the name one would have
            let (name, has_team) = app.read().folder_called(&f.path);
            (f.team, f.has_team) = (name.unwrap_or_default(), has_team.is_some());
            Ok(Json(f))
        }
        Ok(Ok(Err(why))) => Err(no(why)),
        Ok(Err(_)) => Err(no("That folder cannot be opened from here.")),
        Err(_) => Err(no(FOLDER_SLOW)),
    }
}

/// The folders his coding tools have had sessions in, newest first, then his teams no tool has a record of.
pub(super) async fn recent(State(app): State<Arc<App>>) -> Answer<at::Recent> {
    Ok(Json(airband_core::radio::Recent { folders: app.read().recent() }))
}

/// He made a new team from a radio, or asked for another agent in one: a name, and the folder its agents
/// work in.
pub(super) async fn project(State(app): State<Arc<App>>, Sent(new): Sent<at::Project>) -> Answer<at::Project> {
    // a team's name is part of what its agents are called, and is said aloud: short, and written as a
    // callsign is
    let name = TeamName::new(&new.name).map_err(no)?;
    let folder = project_folder(&new).map_err(no)?;
    // a tool he named that this computer cannot start is refused before anything is made
    let launcher = held::launcher_for(&app, new.tool.as_deref());
    if new.tool.is_some() && launcher.is_none() {
        return Err(no("This computer cannot start that tool."));
    }
    let at = folder.display().to_string();
    let (id, name) = project_team(&app, &at, name).map_err(no)?;
    // its first agent, with nothing to do yet, when the tower runs agents itself: the team is then there to see
    if !launcher.is_some_and(|(_, l)| l.held.is_some()) {
        return Ok(Json(airband_core::radio::ProjectMade { say: format!("{name} is ready. Say launch {name}, then what it should do."), team: name, id: id.into() }));
    }
    let place = Place { name: name.clone(), folder: at };
    let reply = launch::launch(&app, &place, &Idea { tool: new.tool.clone(), project: Some(place.clone()), model: None, effort: None, task: String::new() }).await;
    Ok(Json(airband_core::radio::ProjectMade { say: reply.say, team: name, id: id.into() }))
}

/// The folder a new team works in: one he made, called exactly as he wrote the name (one part of a path, not
/// hidden), or one he chose.
fn project_folder(new: &airband_core::radio::NewProject) -> Result<std::path::PathBuf, String> {
    if !new.make {
        return if new.folder.trim().is_empty() { Err("Choose a folder for it to work in.".to_string()) } else { folder_at(&new.folder) };
    }
    let called = std::path::Path::new(new.name.trim());
    let one_part = matches!(called.components().collect::<Vec<_>>().as_slice(), [std::path::Component::Normal(_)]);
    let last = called.file_name().filter(|n| one_part && !n.to_string_lossy().starts_with('.')).ok_or("That is not a name for a folder.")?;
    let made = folder_at(&new.folder)?.join(last);
    if let Err(e) = std::fs::create_dir(&made)
        && !made.is_dir() {
            return Err(format!("That folder could not be made: {e}"));
        }
    Ok(made)
}

/// The team a new one in this folder is: the one that already works there (another agent joins it), or a new
/// one with his name for it. Its id and what it is called.
fn project_team(app: &App, at: &str, name: TeamName) -> Result<(TeamId, String), String> {
    let mut inner = app.change();
    if let Some(team) = inner.team_at(at) {
        let called = inner.team_name(&team).map_or_else(|| name.to_string(), String::from);
        return Ok((team, called));
    }
    // the name came from his radio, so he chose it, even where it is the one the folder would give, and it
    // stays his
    let chosen = inner.chosen_team(name.as_str(), None)?;
    log(format!("a radio made the team {chosen}, in {at}"));
    let called = chosen.to_string();
    Ok((inner.new_folder_team(at, chosen, true), called))
}
