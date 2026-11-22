//! Starting a new agent. He picks a place the tower knows (one of his teams, or a folder his coding tools have
//! lately worked in) and says what the agent should do; the tower runs the configured launcher there. One the
//! tower runs itself is held by it; one handed off reports in like any other, through its own adapter.
//!
//! A launcher is a command template, so another agent tool is another entry in `launchers`, not code here.
use super::*;

/// A new agent he asked for: where, on what, and to do what. The place is nothing until it is known; the model
/// and effort are as he named them (`chosen`), or nothing for the tool's own defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Idea {
    /// The harness he asked for by name ("a codex agent"), when he did and a launcher starts it.
    pub(super) tool: Option<String>,
    pub(super) project: Option<Place>,
    pub(super) model: Option<Pick>,
    pub(super) effort: Option<Pick>,
    pub(super) task: String,
}

/// A model or an effort he asked for, as it came to the tower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pick {
    /// From the phone: the tool's own id for it.
    Id(String),
    /// From his words: one on the list the routing model was shown, by the number it answered: its name, and
    /// each launcher that offers it with that launcher's own id for it.
    Listed { name: String, ids: Vec<(LauncherId, String)> },
    /// From his words: something on no list, as he said it. Only an agent that lists its own settings can be
    /// asked for it, and that agent is told exactly this.
    Said(String),
}

/// The one of `choices` he asked for: by the tool's own id, or by the name on the list he was offered, compared
/// whole. Something on no list goes as he said it only to an agent that lists its own settings (`agent_says`).
/// Anything else is his word back, for telling him it is not there.
fn chosen(asked: Option<&Pick>, launcher: &LauncherId, choices: &[airband_core::tools::Choice], agent_says: bool) -> Result<Option<airband_core::tools::Choice>, String> {
    let found = |is: &dyn Fn(&airband_core::tools::Choice) -> bool| choices.iter().find(|c| is(c)).cloned();
    match asked {
        None => Ok(None),
        Some(Pick::Id(id)) => found(&|c| c.id == *id).map(Some).ok_or_else(|| id.clone()),
        // the id this launcher gave it: one another launcher offers under the same name is not this one's
        Some(Pick::Listed { name, ids }) => ids.iter().find(|(l, _)| l == launcher).and_then(|(_, id)| found(&|c| c.id == *id)).map(Some).ok_or_else(|| name.clone()),
        Some(Pick::Said(said)) if agent_says => Ok(Some(airband_core::tools::Choice { id: said.clone(), name: said.clone() })),
        Some(Pick::Said(said)) => Err(said.clone()),
    }
}

pub(super) async fn launch(app: &Arc<App>, place: &Place, idea: &Idea) -> Reply {
    let task = idea.task.as_str();
    let say = |s: String| Reply { say: s, listen: false, ..Reply::default() };
    let (project, cwd) = (place.name.as_str(), place.folder.clone());
    if !std::path::Path::new(&cwd).is_dir() {
        return say(format!("The folder for {project} is gone."));
    }
    // the tool he asked for, when he named one; otherwise the usual launcher
    let Some((named, launcher)) = held::launcher_for(app, idea.tool.as_deref()) else {
        return say(match &idea.tool {
            Some(tool) => format!("This computer is not set up to start {tool} agents."),
            None => "Which coding agent should start it? Choose the ones you use on your phone, in Settings, under Coding agents, or name one when you ask.".to_string(),
        });
    };
    let launcher = &launcher;
    // An agent the tower runs can be started with nothing to do yet: it is there at once, waiting for his first
    // word. One that is handed off needs its task to start with.
    if task.is_empty() && launcher.held.is_none() {
        return Reply { say: format!("What should it do? Say launch {project}, then the task."), listen: true, ..Reply::default() };
    }
    // The model and effort he asked for. One this launcher does not offer is not left out and the agent
    // started without it: he is told, and nothing is started.
    let menu = held::offers(&app.read(), &named, launcher);
    // An agent that lists its own models and efforts is the one that knows them: which efforts there are can
    // turn on the model, and the tower has only what one last said. So what he asked for goes to its holder
    // as he named it, by the tool's own word where the tower knows it, and an agent that has no such one does
    // not start and says so.
    let agent_says = matches!(launcher.held, Some(config::Held::Acp { .. }));
    let offers = |what: &str, choices: &[airband_core::tools::Choice]| match choices {
        [] => format!("Agents started this way cannot be given {what}."),
        choices => format!("It can be {}.", either(&choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>())),
    };
    let launcher_id = LauncherId::new(named.as_str());
    let chosen = |asked: Option<&Pick>, choices: &[airband_core::tools::Choice]| chosen(asked, &launcher_id, choices, agent_says);
    let model = match chosen(idea.model.as_ref(), &menu.models) {
        Ok(model) => model,
        Err(m) => return say(format!("I cannot start one on {m}. {}", offers("a model", &menu.models))),
    };
    let effort = match chosen(idea.effort.as_ref(), &menu.efforts) {
        Ok(effort) => effort,
        Err(e) => return say(format!("I cannot start one at {e} effort. {}", offers("an effort", &menu.efforts))),
    };
    // as its tool takes them
    let (model_id, effort_id) = (model.as_ref().map(|c| c.id.clone()), effort.as_ref().map(|c| c.id.clone()));
    let (model, effort) = (model.map(|c| c.name), effort.map(|c| c.name));
    let with = match (&model, &effort) {
        (Some(m), Some(e)) => format!(" with {m}, {e}"),
        (Some(m), None) => format!(" with {m}"),
        (None, Some(e)) => format!(" at {e} effort"),
        (None, None) => String::new(),
    };
    if launcher.held.is_some() {
        return match held::start(app, &named, &cwd, model_id, effort_id, task).await {
            Ok(_) if task.is_empty() => {
                log(format!("started {named}{with} in {project}, held by the tower, with nothing to do yet"));
                say(format!("A new agent is ready in {project}{with}."))
            }
            Ok(id) => {
                log(format!("started {named}{with} in {project}, held by the tower: {task}"));
                say(launched(app, place, &with, task, Some(id)))
            }
            Err(e) => {
                log(format!("launch in {project} failed: {e}"));
                say(format!("Could not launch in {project}: {}", airband_core::prose::clip(&spoken(&e), 120)))
            }
        };
    }
    let stamp = now_ms() / 1000 % 100_000;
    let session = format!("airband-{}-{stamp}", project.replace(' ', "-"));
    let fill = |a: &String| a.replace("{cwd}", &cwd).replace("{task}", task).replace("{project}", project).replace("{session}", &session);
    let argv = launcher.command(model_id.as_deref(), effort_id.as_deref(), fill);
    let Some((program, args)) = argv.split_first() else { return say("The launcher command is empty.".into()) };
    // AIRBAND_INTERNAL must not leak in: the new agent has to report to the tower
    let run = tokio::process::Command::new(program).args(args).current_dir(&cwd).env_remove("AIRBAND_INTERNAL").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(false).output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => {
            log(format!("launched {named}{with} in {project} ({session}): {task}"));
            // handed off to a program of its own: nothing says which agent it becomes when it reports in
            say(launched(app, place, &with, task, None))
        }
        Ok(Ok(out)) => {
            log(format!("launch in {project} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
            say(format!("Could not launch in {project}: {}", airband_core::prose::clip(&spoken(String::from_utf8_lossy(&out.stderr).trim()), 120)))
        }
        Ok(Err(e)) => {
            log(format!("launch in {project} failed: cannot run {program}: {e}"));
            say(format!("Could not launch in {project}: {program} did not start."))
        }
        Err(_) => say(format!("Launching in {project} did not finish within twenty seconds.")),
    }
}

/// Who his words are shown as going to when they start a new agent, before it has a name of its own.
pub(super) fn launch_label(project: &str) -> String {
    format!("a new agent in {project}")
}

/// A new agent was started with a task: the task is written into the chat as his words to it, under the team
/// of the folder it was started in (made now, when the folder had none), and he is told it is on its way.
/// `agent` is the new agent's id, when the tower runs it and so knows it: the task is then its first
/// instruction.
fn launched(app: &App, place: &Place, with: &str, task: &str, agent: Option<String>) -> String {
    let mut inner = app.change();
    let project = &place.name;
    match inner.folder_team(&place.folder) {
        Some(team) => inner.record(Entry { id: config::random_hex(8), from: HIM.into(), his: true, team, to: vec![launch_label(project)], to_agents: agent.into_iter().collect(), short: Some(task.into()), full: task.into(), ..Entry::default() }),
        None => log(format!("the task for the new agent in {project} is not written in the chat: its folder has no name to give a team")),
    }
    format!("Launching in {project}{with}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::tools::Choice;

    #[test]
    fn a_model_is_taken_by_its_id_or_this_launchers_own_id_for_the_listed_one_and_never_by_a_near_word() {
        let models = vec![Choice { id: "gpt-6.2-sol".into(), name: "6.2 Sol".into() }, Choice { id: "gpt-6-luna".into(), name: "6 Luna".into() }];
        let (here, other) = (LauncherId::new("codex"), LauncherId::new("held-codex"));
        let id = |r: Result<Option<Choice>, String>| r.map(|c| c.map(|c| c.id));
        let listed = |name: &str, ids: &[(&LauncherId, &str)]| Pick::Listed { name: name.into(), ids: ids.iter().map(|(l, i)| ((*l).clone(), i.to_string())).collect() };
        assert_eq!(id(chosen(Some(&Pick::Id("gpt-6-luna".into())), &here, &models, false)), Ok(Some("gpt-6-luna".into())));
        // a listed one is this launcher's by the id this launcher gave it
        assert_eq!(id(chosen(Some(&listed("6.2 Sol", &[(&other, "sol-2"), (&here, "gpt-6.2-sol")])), &here, &models, false)), Ok(Some("gpt-6.2-sol".into())));
        // one only another launcher offers under that name is not this launcher's, however its name reads
        assert_eq!(id(chosen(Some(&listed("6.2 Sol", &[(&other, "gpt-6.2-sol")])), &here, &models, false)), Err("6.2 Sol".into()));
        // nothing loosened: a name used as an id, or a word on no list, is not it
        assert_eq!(id(chosen(Some(&Pick::Id("6 Luna".into())), &here, &models, false)), Err("6 Luna".into()));
        assert_eq!(id(chosen(Some(&Pick::Said("luna".into())), &here, &models, false)), Err("luna".into()));
        // an agent that lists its own settings is asked for exactly what he said
        assert_eq!(id(chosen(Some(&Pick::Said("luna".into())), &here, &models, true)), Ok(Some("luna".into())));
        assert_eq!(id(chosen(None, &here, &models, false)), Ok(None));
    }


    #[test]
    fn a_task_started_in_a_folder_with_no_team_yet_is_written_under_that_folders_team() {
        let app = crate::tower::http::tests::app("launched");
        let folder = std::env::temp_dir().join(format!("airband-launched-{}", config::random_hex(4))).join("shop");
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.display().to_string();
        launched(&app, &Place { name: "shop".into(), folder: folder.clone() }, "", "fix the cart", None);
        let inner = app.read();
        let line = inner.log.last().unwrap();
        assert_eq!(line.team, real_folder(&folder));
        assert_eq!(inner.team_name(&line.team), Some("shop"));
    }
}
