//! Teams and folders: moving an agent, naming a team, walking the computer's folders, starting a team.
use super::*;

/// He moved an agent to another team.
pub(super) async fn team(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::MoveAgent>) -> Json<Value> {
    Json(match place::by_him(&app, &to.agent, &to.to) {
        Ok((callsign, team, id)) => json!({ "callsign": callsign, "team": team, "id": id }),
        Err(why) => json!({ "error": why }),
    })
}

/// He renamed one of his teams, from a radio.
pub(super) async fn team_rename(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::RenameTeam>) -> Json<Value> {
    let mut inner = app.change();
    Json(match inner.rename_team(&to.team, &to.to) {
        Ok(team) => {
            inner.tell();
            json!({ "team": team })
        }
        Err(why) => json!({ "error": why }),
    })
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

/// The folders inside one of his, for choosing where a new team works. A Mac asks him, at the Mac, the first
/// time a program opens some of his folders (the desktop, his documents, another disk), and the read waits for
/// his answer: so it is done to one side and given a few seconds, and he is told when it is the Mac that is
/// in the way.
pub(super) async fn folders(State(app): State<Arc<App>>, Json(ask): Json<airband_core::radio::FoldersAsk>) -> Json<Value> {
    let read = tokio::task::spawn_blocking(move || {
        folder_at(&ask.under).and_then(|at| {
            let entries = std::fs::read_dir(&at).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => "Your Mac does not let airband open this folder. To allow it, open System Settings on the Mac, then Privacy & Security, then Files and Folders.".to_string(),
                _ => "That folder cannot be opened from here.".to_string(),
            })?;
            let mut inside: Vec<String> = entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect();
            inside.sort_by_key(|n| n.to_lowercase());
            Ok(airband_core::radio::Folders { path: at.display().to_string(), shown: shown(&at), up: at.parent().map(|p| p.display().to_string()), folders: inside, team: String::new(), has_team: false })
        })
    });
    Json(match tokio::time::timeout(Duration::from_secs(5), read).await {
        Ok(Ok(Ok(mut f))) => {
            // the team that works here, or the name one would have
            let inner = app.read();
            let team = inner.team_at(&f.path);
            f.has_team = team.is_some();
            f.team = team.map(|id| inner.team_name(&id)).or_else(|| inner.name_for_folder(&f.path)).unwrap_or_default();
            serde_json::to_value(f).unwrap()
        }
        Ok(Ok(Err(why))) => json!({ "error": why }),
        Ok(Err(_)) => json!({ "error": "That folder cannot be opened from here." }),
        Err(_) => json!({ "error": "This folder is taking too long to open. Your Mac may be asking whether airband can open it. Answer at the Mac, then try again." }),
    })
}

/// The folders his coding tools have had sessions in, newest first, then his teams no tool has a record of.
pub(super) async fn recent(State(app): State<Arc<App>>) -> Json<airband_core::radio::Recent> {
    Json(airband_core::radio::Recent { folders: app.read().recent() })
}

/// Read again which folders his coding tools have had sessions in, and keep the answer. Reads their
/// records, so it is done off the lock; the tower does it when it starts and every minute after.
pub(in crate::tower) fn read_worked(app: &App) {
    let worked = crate::adapters::worked_all();
    app.passing().worked = worked;
}

/// He made a new team from a radio, or asked for another agent in one: a name, and the folder its agents
/// work in.
pub(super) async fn project(State(app): State<Arc<App>>, Json(new): Json<airband_core::radio::NewProject>) -> Json<Value> {
    // a team's name is part of what its agents are called, and is said aloud: short, and written as a
    // callsign is
    let Some(mut name) = airband_core::routing::team_name(&new.name).filter(|n| n.chars().count() <= 24) else {
        return Json(json!({ "error": "A team\u{2019}s name is letters and numbers, 24 at most." }));
    };
    let folder = if new.make {
        let wanted = std::path::PathBuf::from(&new.folder);
        match wanted.parent().map(|p| folder_at(&p.display().to_string())) {
            Some(Ok(parent)) => match wanted.file_name().filter(|n| !n.to_string_lossy().starts_with('.')) {
                Some(last) => {
                    let made = parent.join(last);
                    if let Err(e) = std::fs::create_dir(&made)
                        && !made.is_dir() {
                            return Json(json!({ "error": format!("That folder could not be made: {e}") }));
                        }
                    Ok(made)
                }
                None => Err("That is not a name for a folder.".to_string()),
            },
            Some(Err(why)) => Err(why),
            None => Err("That is not a folder.".to_string()),
        }
    } else if new.folder.trim().is_empty() {
        Err("Choose a folder for it to work in.".to_string())
    } else {
        folder_at(&new.folder)
    };
    let folder = match folder {
        Ok(f) => f,
        Err(why) => return Json(json!({ "error": why })),
    };
    let id = {
        let mut inner = app.change();
        let at = folder.display().to_string();
        match inner.team_at(&at) {
            // a folder that has a team: another agent joins it
            Some(team) => {
                name = inner.team_name(&team);
                team
            }
            None if inner.team_called(&name).is_some() => {
                return Json(json!({ "error": format!("There is already a team called {name}, in another folder.") }));
            }
            None => {
                // a name other than the one the folder would give is one he chose, and stays his
                let named = inner.name_for_folder(&at).as_deref() != Some(name.as_str());
                let id = sessions::real_folder(&at);
                inner.teams.insert(id.clone(), Team { name: name.clone(), folder: Some(id.clone()), named, configured: false });
                log(format!("a radio made the team {name}, in {at}"));
                id
            }
        }
    };
    // its first agent, with nothing to do yet, when the tower runs agents itself: the team is then there to see
    let held = held::launcher_for(&app, new.tool.as_deref()).is_some_and(|(_, l)| l.held.is_some());
    if !held {
        return Json(json!({ "say": format!("{name} is ready. Say launch {name}, then what it should do."), "team": name, "id": id }));
    }
    let reply = launch::launch(&app, &name, &Idea { tool: new.tool.clone(), project: Some(name.clone()), model: None, effort: None, task: String::new() }).await;
    Json(json!({ "say": reply.say, "team": name, "id": id }))
}
