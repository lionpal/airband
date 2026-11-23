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
    /// From his words: one on the list the routing model was shown, by the number it answered: its name, the
    /// one launcher that offers it, and that launcher's own id for it. Another launcher's choice of the same
    /// name is another entry on that list, never this one.
    Listed { name: String, launcher: LauncherId, id: String },
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
        Some(Pick::Listed { name, launcher: offered, id }) => (offered == launcher).then(|| found(&|c| c.id == *id)).flatten().map(Some).ok_or_else(|| name.clone()),
        Some(Pick::Said(said)) if agent_says => Ok(Some(airband_core::tools::Choice { id: said.clone(), name: said.clone() })),
        Some(Pick::Said(said)) => Err(said.clone()),
    }
}

/// The launcher a listed pick (a model or an effort from the list the routing model was shown) is for.
fn listed_launcher(pick: Option<&Pick>) -> Option<&LauncherId> {
    match pick {
        Some(Pick::Listed { launcher, .. }) => Some(launcher),
        _ => None,
    }
}

/// The launcher that starts the agent. A listed choice he picked is one launcher's, and that launcher starts it;
/// a tool he named that it does not start, or a model and an effort of two launchers, is refused in words. With
/// no listed choice: the launcher for the tool he named, else the usual one (`held::launcher_for`).
fn starter(app: &App, idea: &Idea) -> Result<(String, config::Launcher), String> {
    let (model, effort) = (listed_launcher(idea.model.as_ref()), listed_launcher(idea.effort.as_ref()));
    let picked = match (model, effort) {
        (Some(m), Some(e)) if m != e => return Err("That model and that effort are offered by two ways of starting an agent. Ask for one of them.".into()),
        (m, e) => m.or(e),
    };
    let Some(picked) = picked else {
        return held::launcher_for(app, idea.tool.as_deref()).ok_or_else(|| match &idea.tool {
            Some(tool) => format!("This computer is not set up to start {tool} agents."),
            None => "Which coding agent should start it? Choose the ones you use on your phone, in Settings, under Coding agents, or name one when you ask.".to_string(),
        });
    };
    let cfg = app.cfg();
    let Some(launcher) = cfg.launchers.get(picked.as_str()) else { return Err("The way of starting that agent is no longer set up here.".into()) };
    match &idea.tool {
        Some(tool) if *tool != launcher.harness => Err(format!("That is not offered for {tool} agents.")),
        _ => Ok((picked.to_string(), launcher.clone())),
    }
}

/// Start a new agent in `place`. `Ok` once it has started, with what to tell him; `Err` with why nothing started.
pub(super) async fn launch(app: &Arc<App>, place: &Place, idea: &Idea) -> Result<Reply, Reply> {
    let task = idea.task.as_str();
    let say = |s: String| reply(s, false);
    let (project, cwd) = (place.name.as_str(), place.folder.clone());
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(say(format!("The folder for {project} is gone.")));
    }
    let (named, launcher) = starter(app, idea).map_err(say)?;
    let launcher = &launcher;
    // An agent the tower runs can be started with nothing to do yet: it is there at once, waiting for his first
    // word. One that is handed off needs its task to start with.
    if task.is_empty() && launcher.held.is_none() {
        return Err(Reply { say: format!("What should it do? Say launch {project}, then the task."), listen: true, ..Reply::default() });
    }
    // The model and effort he asked for. One this launcher does not offer is not left out and the agent
    // started without it: he is told, and nothing is started.
    let menu = held::offers(&app.read(), &named, launcher);
    // An agent that lists its own models and efforts is the one that knows them: which efforts there are can
    // turn on the model, and the tower has only what one last said. So what he asked for goes to its holder
    // as he named it, by the tool's own word where the tower knows it, and an agent that has no such one does
    // not start and says so.
    let agent_says = launcher.says_its_settings();
    let offers = |what: &str, choices: &[airband_core::tools::Choice]| match choices {
        [] => format!("Agents started this way cannot be given {what}."),
        choices => format!("It can be {}.", either(&choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>())),
    };
    let launcher_id = LauncherId::new(named.as_str());
    let chosen = |asked: Option<&Pick>, choices: &[airband_core::tools::Choice]| chosen(asked, &launcher_id, choices, agent_says);
    let model = match chosen(idea.model.as_ref(), &menu.models) {
        Ok(model) => model,
        Err(m) => return Err(say(format!("I cannot start one on {m}. {}", offers("a model", &menu.models)))),
    };
    let effort = match chosen(idea.effort.as_ref(), &menu.efforts) {
        Ok(effort) => effort,
        Err(e) => return Err(say(format!("I cannot start one at {e} effort. {}", offers("an effort", &menu.efforts)))),
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
    // the chat line the task is written as, named before the agent is told it, so its taking it in marks that line
    let line = config::random_hex(8);
    if launcher.held.is_some() {
        return match held::start(app, &named, &cwd, model_id, effort_id, task, &line).await {
            Ok(_) if task.is_empty() => {
                log(format!("started {named}{with} in {project}, held by the tower, with nothing to do yet"));
                Ok(say(format!("A new agent is ready in {project}{with}.")))
            }
            Ok(id) => {
                // the task is his words: the log has its length only
                log(format!("started {named}{with} in {project}, held by the tower, with a task of {} characters", task.chars().count()));
                Ok(say(launched(app, place, &with, task, Some(id), line)))
            }
            Err(e) => {
                log(format!("launch in {project} failed: {e}"));
                Err(say(format!("Could not launch in {project}: {}", airband_core::prose::clip(&spoken(&e), 120))))
            }
        };
    }
    let stamp = now_ms() / 1000 % 100_000;
    let session = format!("airband-{}-{stamp}", project.replace(' ', "-"));
    // one pass: a task with "{project}" in it stays as he said it
    let fill = |a: &String| airband_core::template::filled(a, &[("cwd", &cwd), ("task", task), ("project", project), ("session", &session)]);
    let argv = launcher.command(model_id.as_deref(), effort_id.as_deref(), fill);
    let Some((program, args)) = argv.split_first() else { return Err(say("The launcher command is empty.".into())) };
    // AIRBAND_INTERNAL must not leak in: the new agent has to report to the tower
    let run = tokio::process::Command::new(program).args(args).current_dir(&cwd).env_remove(INTERNAL_ENV).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(false).output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => {
            log(format!("launched {named}{with} in {project} ({session}), with a task of {} characters", task.chars().count()));
            // handed off to a program of its own: nothing says which agent it becomes when it reports in
            Ok(say(launched(app, place, &with, task, None, line)))
        }
        Ok(Ok(out)) => {
            // what it wrote can hold its command line, and so his task: neither the log nor he is given it
            log(format!("launch in {project} failed: {program} ended with {} after writing {} bytes of errors", out.status, out.stderr.len()));
            Err(say(format!("Could not launch in {project}: {program} stopped with an error.")))
        }
        Ok(Err(e)) => {
            log(format!("launch in {project} failed: cannot run {program}: {e}"));
            Err(say(format!("Could not launch in {project}: {program} did not start.")))
        }
        Err(_) => Err(say(format!("Launching in {project} did not finish within twenty seconds."))),
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
fn launched(app: &App, place: &Place, with: &str, task: &str, agent: Option<String>, line: String) -> String {
    let mut inner = app.change();
    let project = &place.name;
    match inner.folder_team(&place.folder) {
        Some(team) => inner.record(Entry { id: line, from: HIM.into(), his: true, team: Some(team), to: vec![launch_label(project)], to_agents: agent.into_iter().collect(), short: Some(task.into()), full: task.into(), ..Entry::default() }),
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
        let listed = |name: &str, launcher: &LauncherId, id: &str| Pick::Listed { name: name.into(), launcher: launcher.clone(), id: id.into() };
        assert_eq!(id(chosen(Some(&Pick::Id("gpt-6-luna".into())), &here, &models, false)), Ok(Some("gpt-6-luna".into())));
        // a listed one is this launcher's by the id this launcher gave it
        assert_eq!(id(chosen(Some(&listed("6.2 Sol", &here, "gpt-6.2-sol")), &here, &models, false)), Ok(Some("gpt-6.2-sol".into())));
        // one only another launcher offers under that name is not this launcher's, however its name reads
        assert_eq!(id(chosen(Some(&listed("6.2 Sol", &other, "gpt-6.2-sol")), &here, &models, false)), Err("6.2 Sol".into()));
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
        launched(&app, &Place { name: "shop".into(), folder: folder.clone() }, "", "fix the cart", None, "t1".into());
        let inner = app.read();
        let line = inner.log.last().unwrap();
        // under the id the agent was told it with, so its taking it in marks this line
        assert_eq!(line.id, "t1");
        assert_eq!(line.team, Some(TeamId::new(edges::folders::Folders::real(&edges::folders::Disk, &folder))));
        assert_eq!(line.team.as_ref().and_then(|t| inner.team_name(t)), Some("shop"));
    }

    fn with_launchers(name: &str, usual: &str, launchers: serde_json::Value) -> Arc<App> {
        crate::tower::http::tests::app_with(name, serde_json::json!({ "launcher": usual, "launchers": launchers }))
    }

    #[test]
    fn the_launcher_of_the_choice_he_picked_starts_the_agent_and_a_tool_it_does_not_start_is_refused() {
        let app = with_launchers("starter", "a", serde_json::json!({
            "a": { "harness": "tool-one", "argv": ["true"], "models": { "Sol": "sol-a" } },
            "b": { "harness": "tool-two", "argv": ["true"], "models": { "Sol": "sol-b" }, "efforts": ["high"] },
        }));
        let (a, b) = (LauncherId::new("a"), LauncherId::new("b"));
        let model = |launcher: &LauncherId| Some(Pick::Listed { name: "Sol".into(), launcher: launcher.clone(), id: "sol".into() });
        let started = |idea: Idea| starter(&app, &idea).map(|(name, _)| name);
        // no tool named: the picked choice's launcher, not the usual one
        assert_eq!(started(Idea { model: model(&b), ..Idea::default() }), Ok("b".into()));
        assert_eq!(started(Idea { tool: Some("tool-two".into()), model: model(&b), ..Idea::default() }), Ok("b".into()));
        // a tool he named that the picked choice's launcher does not start: refused, nothing swapped
        assert!(started(Idea { tool: Some("tool-one".into()), model: model(&b), ..Idea::default() }).is_err());
        // a model of one launcher and an effort of another: refused
        assert!(started(Idea { model: model(&a), effort: Some(Pick::Listed { name: "high".into(), launcher: b.clone(), id: "high".into() }), ..Idea::default() }).is_err());
        // nothing listed picked: the tool he named, else the usual
        assert_eq!(started(Idea { tool: Some("tool-two".into()), ..Idea::default() }), Ok("b".into()));
        assert_eq!(started(Idea { model: Some(Pick::Said("luna".into())), ..Idea::default() }), Ok("a".into()));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn his_task_and_what_the_tool_wrote_are_never_in_the_log_and_the_task_is_put_in_as_he_said_it() {
        let folder = std::env::temp_dir().display().to_string();
        let place = Place { name: "shop".into(), folder };
        // the task reaches the program whole: "{session}" in it is his, never filled in
        let ok = with_launchers("launch-log-ok", "mine", serde_json::json!({
            "mine": { "harness": "his-own-tool", "argv": ["sh", "-c", "case \"$1\" in *'{'session'}'*) exit 0;; *) exit 1;; esac", "sh", "{task}"] },
        }));
        crate::tower::logged::take();
        let idea = Idea { task: "the secret plan, in {session}".into(), ..Idea::default() };
        launch(&ok, &place, &idea).await.expect("started, with the task as he said it");
        let lines = crate::tower::logged::take();
        assert!(!lines.is_empty() && lines.iter().all(|l| !l.contains("secret plan")), "{lines:?}");
        // a launcher that fails, writing his task among its errors
        let failing = with_launchers("launch-log-failed", "mine", serde_json::json!({
            "mine": { "harness": "his-own-tool", "argv": ["sh", "-c", "echo \"$1\" >&2; exit 3", "sh", "{task}"] },
        }));
        crate::tower::logged::take();
        let refused = launch(&failing, &place, &idea).await.expect_err("the launcher failed");
        let lines = crate::tower::logged::take();
        assert!(!lines.is_empty() && lines.iter().all(|l| !l.contains("secret plan")), "{lines:?}");
        assert!(!refused.say.contains("secret plan"), "{}", refused.say);
    }

    #[tokio::test]
    async fn a_launch_that_starts_nothing_says_so_as_a_refusal() {
        let app = crate::tower::http::tests::app("launch-refused");
        let place = Place { name: "gone".into(), folder: "/nowhere/airband-gone".into() };
        let idea = Idea { project: Some(place.clone()), task: "fix it".into(), ..Idea::default() };
        // the idea waiting for "which project" is kept on this, so he can answer again
        let refused = launch(&app, &place, &idea).await.expect_err("nothing started in a folder that is gone");
        assert_eq!(refused.say, "The folder for gone is gone.");
    }
}
