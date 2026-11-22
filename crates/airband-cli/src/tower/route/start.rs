//! Starting a new agent he asked for: where an idea belongs, and the answers to the questions asked about it.
use super::*;

/// A model's answer that picks one project from a numbered list, or none.
#[derive(Deserialize)]
pub(super) struct Picked {
    project: Option<usize>,
}

/// Which project a new agent's task belongs in, when he gave the idea and no project. With one project there is
/// nothing to choose. With several, a model reads the idea against what each project's agents have been doing and
/// picks the best fit: he is told where it started, so a fair guess beats a question. Only when nothing in the
/// idea points anywhere is he asked, and the idea is kept for his answer.
pub(super) async fn place_idea(app: &Arc<App>, idea: &str, projects: &[String]) -> Result<String, Reply> {
    let say = |s: String, listen: bool| Reply { say: s, listen, ..Reply::default() };
    if idea.is_empty() {
        return Err(say("What should it do? Say new idea, then the idea.".into(), true));
    }
    match projects {
        [] => return Err(say("There is no project to start it in yet. Run an agent in one first.".into(), false)),
        [only] => return Ok(only.clone()),
        _ => {}
    }
    let lines: Vec<String> = {
        let inner = app.read();
        let folders = inner.launchable();
        projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let lately: Vec<String> = inner.log.iter().rev().filter(|e| inner.team_called(p).is_some_and(|t| e.team == t) && !e.his && e.kind != Some(Kind::Fyi)).take(2).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
                format!("{}. {p} (folder {}){}", i + 1, folders.get(p).map(String::as_str).unwrap_or("?"), if lately.is_empty() { String::new() } else { format!(": lately, {}", lately.join(" / ")) })
            })
            .collect()
    };
    let prompt = format!(
        "{name} wants to start a new coding agent on this: \"{idea}\"\n\nThese are his projects:\n{lines}\n\n\
         Which project would this work be done in? Go by who the work is for and what it changes, more than by \
         what data it happens to read. Pick the best fit. He is told which project it started in and can stop \
         it, so a reasonable choice is better than asking him.\n\
         Reply with one line of JSON and nothing else: {{\"project\":<the number it is listed under, or null>}}\n\
         Null only if nothing in the idea points to any of these projects.",
        name = app.cfg.name,
        lines = lines.join("\n")
    );
    let picked = judge::ask_json::<Picked>(&app.cfg, "which project an idea is for", prompt, None).await.ok();
    match picked.and_then(|p| judge::listed(p.project, projects).cloned()) {
        Some(project) => Ok(project),
        None => Err(say(format!("Which project is that for? {}.", projects.join(", ")), true)),
    }
}

/// He was asked which project an idea belongs in, and has said something. The project his words name, or
/// nothing when they are not an answer to that (or no model said).
pub(super) async fn named_project(app: &Arc<App>, idea: &str, said: &str, projects: &[String]) -> Option<String> {
    let prompt = format!(
        "{name} wanted a new coding agent started on this: \"{idea}\"\nHe was asked which of his projects it is for:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         Does that answer the question, and if so with which project? An answer does little more than name one \
         (speech may write a name apart or misspell it). Anything that asks for other work or says something of \
         its own is a new message and not an answer, even when it mentions a project.\n\
         Reply with one line of JSON and nothing else: {{\"project\":<the number it is listed under, or null>}}\n\
         Null if he names none of them, rules one out without choosing another, or has gone on to \
         say something else.",
        name = app.cfg.name,
        list = projects.iter().enumerate().map(|(i, p)| format!("{}. {p}", i + 1)).collect::<Vec<_>>().join("\n"),
    );
    let picked: Picked = judge::ask_json(&app.cfg, "which project he named", prompt, None).await.ok()?;
    judge::listed(picked.project, projects).cloned()
}

/// He was asked who something he said was for, and has said something more. The agents his words name, or
/// nothing when they are not an answer to that (or no model said): then they are a message of their own.
pub(super) async fn named_agents(app: &Arc<App>, first: &str, said: &str, scope: Option<&str>) -> Option<Vec<Session>> {
    let room = scope;
    let all: Vec<Session> = app.read().sessions.iter().filter(|s| room.is_none_or(|g| s.team == g)).cloned().collect();
    let letters = |s: &Session| s.callsign.badge();
    let prompt = format!(
        "{name} said this to his coding agents: \"{first}\"\nIt was not clear who it was for, and he was asked. The agents:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         Does that answer the question, and if so who does he mean? An answer does little more than name who: a \
         callsign, an agent's letters, a number (speech may write these apart or as words that sound like them), \
         \"the first one\", \"both\", all of them, perhaps with a word or two around it. Anything that asks for \
         work, gives an instruction or says something of its own is a new message and not an answer, even when it \
         is about what one of the agents is working on.\n\
         Reply with one line of JSON and nothing else: {{\"to\":[the number each agent he means is listed under],\"all\":true or false}}\n\
         \"all\" is true when he means all of them. \"to\" is empty if he names nobody listed, or has gone on to say something else.",
        name = app.cfg.name,
        list = all.iter().enumerate().map(|(i, s)| format!("{}. {} (letters {})", i + 1, s.callsign.text(), letters(s))).collect::<Vec<_>>().join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        #[serde(default)]
        to: Vec<usize>,
        #[serde(default)]
        all: bool,
    }
    let judged: Out = judge::ask_json(&app.cfg, "who he meant", prompt, None).await.ok()?;
    let meant: Vec<Session> = if judged.all { all } else { judged.to.iter().filter_map(|n| judge::listed(Some(*n), &all).cloned()).collect() };
    (!meant.is_empty()).then_some(meant)
}

/// Start the new agent he asked for. With no project, a model is asked where the idea belongs when nobody has
/// judged that yet (`place`); if nothing points anywhere he is asked, and the idea is kept for his answer.
pub(super) async fn start(app: &Arc<App>, radio: &str, text: &str, idea: Idea, projects: &[String], place: bool) -> Reply {
    let project = match (&idea.project, place) {
        (Some(named), _) => named.clone(),
        (None, true) => match place_idea(app, &idea.task, projects).await {
            Ok(project) => project,
            Err(reply) => {
                if reply.listen {
                    app.passing().call(radio).unplaced_idea = Some((idea, Instant::now()));
                }
                log(format!("← radio: \"{text}\" → a new agent, project not clear"));
                return reply;
            }
        },
        (None, false) if idea.task.is_empty() || projects.is_empty() => return place_idea(app, &idea.task, projects).await.err().unwrap_or(Reply { say: String::new(), listen: false, ..Reply::default() }),
        (None, false) => {
            app.passing().call(radio).unplaced_idea = Some((idea, Instant::now()));
            log(format!("← radio: \"{text}\" → a new agent, project not clear"));
            return Reply { say: format!("Which project is that for? {}.", projects.join(", ")), listen: true, ..Reply::default() };
        }
    };
    log(format!("← radio: \"{text}\" → launch in {project}"));
    launch(app, &project, &idea).await
}
