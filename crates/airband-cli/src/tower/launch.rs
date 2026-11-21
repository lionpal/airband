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
    let Some((named, launcher)) = app.cfg.launcher_for(idea.tool.as_deref()) else {
        return say(match &idea.tool {
            Some(tool) => format!("This computer is not set up to start {tool} agents."),
            None => format!("No launcher called {} is configured.", app.cfg.launcher),
        });
    };
    let named = named.clone();
    // An agent the tower runs can be started with nothing to do yet: it is there at once, waiting for his first
    // word. One that is handed off needs its task to start with.
    if task.is_empty() && launcher.held.is_none() {
        return Reply { say: format!("What should it do? Say launch {project}, then the task."), listen: true, ..Reply::default() };
    }
    // the model and effort he asked for, when this launcher offers them
    let model = idea.model.as_ref().filter(|m| launcher.models.contains_key(*m));
    let effort = idea.effort.as_ref().filter(|e| launcher.efforts.contains(*e));
    let mut options: Vec<String> = Vec::new();
    if let Some(m) = model {
        options.extend(launcher.with_model.iter().map(|a| a.replace("{model}", &launcher.models[m])));
    }
    if let Some(e) = effort {
        options.extend(launcher.with_effort.iter().map(|a| a.replace("{effort}", e)));
    }
    let with = match (model, effort) {
        (Some(m), Some(e)) => format!(" with {m}, {e}"),
        (Some(m), None) => format!(" with {m}"),
        (None, Some(e)) => format!(" at {e} effort"),
        (None, None) => String::new(),
    };
    if launcher.held.is_some() {
        return match held::start(app, &named, &cwd, options, task).await {
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
