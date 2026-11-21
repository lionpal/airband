//! Starting a new agent. He names a project the tower knows and says what the agent should do; the tower runs
//! the configured launcher there. The new agent then reports in like any other, through its own adapter: the
//! tower does not own it, wrap it or watch it.
//!
//! Only into known projects: folders agents have already worked in, or ones listed in the configuration. A
//! launcher is a command template, so another agent tool is another entry in `launchers`, not code here.
use super::*;

pub(super) async fn launch(app: &Arc<App>, project: &str, idea: &Idea) -> Reply {
    let task = idea.task.as_str();
    let say = |s: String| Reply { say: s, listen: false, ..Reply::default() };
    let Some(cwd) = app.inner.lock().unwrap().launchable(&app.cfg).get(project).cloned() else {
        return say(format!("I do not know a project called {project}."));
    };
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
    let menu = held::offers(&app.inner.lock().unwrap(), &named, launcher);
    // An agent that lists its own models and efforts is the one that knows them: which efforts there are can
    // turn on the model, and the tower has only what one last said. So what he asked for goes to its holder
    // as he named it, by the tool's own word where the tower knows it, and an agent that has no such one does
    // not start and says so.
    let agent_says = matches!(launcher.held, Some(config::Held::Acp { .. }));
    let offers = |what: &str, choices: &[airband_core::tools::Choice]| match choices {
        [] => format!("Agents started this way cannot be given {what}."),
        choices => format!("It can be {}.", choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")),
    };
    let chosen = |said: Option<&String>, choices: &[airband_core::tools::Choice]| -> Result<Option<airband_core::tools::Choice>, String> {
        match said {
            None => Ok(None),
            Some(said) => match airband_core::tools::named(choices, said) {
                Some(choice) => Ok(Some(choice.clone())),
                None if agent_says => Ok(Some(airband_core::tools::Choice { id: said.clone(), name: said.clone() })),
                None => Err(said.clone()),
            },
        }
    };
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
    let mut options: Vec<String> = Vec::new();
    if let Some(m) = &model_id {
        options.extend(launcher.with_model.iter().map(|a| a.replace("{model}", m)));
    }
    if let Some(e) = &effort_id {
        options.extend(launcher.with_effort.iter().map(|a| a.replace("{effort}", e)));
    }
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
            Ok(_) => {
                log(format!("started {named}{with} in {project}, held by the tower: {task}"));
                let mut inner = app.inner.lock().unwrap();
                inner.record(Entry { id: config::random_hex(8), from: "you".into(), group: project.into(), to: vec![format!("a new agent in {project}")], short: Some(task.into()), full: task.into(), ..Entry::default() });
                app.save(&inner);
                say(format!("Launching in {project}{with}."))
            }
            Err(e) => {
                log(format!("launch in {project} failed: {e}"));
                say(format!("Could not launch in {project}: {}", spoken(&e).chars().take(120).collect::<String>()))
            }
        };
    }
    let stamp = now() / 1000 % 100_000;
    let session = format!("airband-{}-{stamp}", project.replace(' ', "-"));
    let fill = |a: &String| a.replace("{cwd}", &cwd).replace("{task}", task).replace("{project}", project).replace("{session}", &session);
    let argv: Vec<String> = launcher.argv.iter().flat_map(|a| if a == "{options}" { options.clone() } else { vec![fill(a)] }).collect();
    let Some((program, args)) = argv.split_first() else { return say("The launcher command is empty.".into()) };
    // AIRBAND_INTERNAL must not leak in: the new agent has to report to the tower
    let run = tokio::process::Command::new(program).args(args).current_dir(&cwd).env_remove("AIRBAND_INTERNAL").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(false).output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => {
            log(format!("launched {named}{with} in {project} ({session}): {task}"));
            let mut inner = app.inner.lock().unwrap();
            inner.record(Entry { id: config::random_hex(8), from: "you".into(), group: project.into(), to: vec![format!("a new agent in {project}")], short: Some(task.into()), full: task.into(), ..Entry::default() });
            app.save(&inner);
            say(format!("Launching in {project}{with}."))
        }
        Ok(Ok(out)) => {
            log(format!("launch in {project} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
            say(format!("Could not launch in {project}: {}", spoken(String::from_utf8_lossy(&out.stderr).trim()).chars().take(120).collect::<String>()))
        }
        Ok(Err(e)) => {
            log(format!("launch in {project} failed: cannot run {program}: {e}"));
            say(format!("Could not launch in {project}: {program} did not start."))
        }
        Err(_) => say(format!("Launching in {project} did not finish within twenty seconds.")),
    }
}
