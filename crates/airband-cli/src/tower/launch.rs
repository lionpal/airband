//! Starting a new agent. He picks a place the tower knows (one of his teams, or a folder his coding tools have
//! lately worked in) and says what the agent should do; the tower runs the configured launcher there. One the
//! tower runs itself is held by it; one handed off reports in like any other, through its own adapter.
//!
//! A launcher is a command template, so another agent tool is another entry in `launchers`, not code here.
use super::*;

/// A new agent he asked for: where, on what, and to do what. The place is nothing until it is known; the model
/// and effort are as he named them (`chosen`), or nothing for the tool's own defaults.
#[derive(Clone, Default, PartialEq)]
pub(super) struct Idea {
    /// The harness he asked for by name ("a <tool> agent"), when he did and a launcher starts it.
    pub(super) tool: Option<String>,
    pub(super) project: Option<Place>,
    pub(super) model: Option<Pick>,
    pub(super) effort: Option<Pick>,
    pub(super) task: String,
}

/// The task is his words: never in a Debug form, its length only.
impl std::fmt::Debug for Idea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Idea").field("tool", &self.tool).field("project", &self.project).field("model", &self.model).field("effort", &self.effort).field("task", &crate::unshown(&self.task)).finish()
    }
}

/// A model or an effort he asked for, as it came to the tower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pick {
    /// From the phone: the tool's own id for it.
    Id(String),
    /// From his words: the model or effort the routing model heard, as offered, whichever launcher offers it.
    /// Which launcher starts the agent is settled after, by rule (`starter`), never by where it was listed.
    Heard(Heard),
    /// From his words: something on no list, as he said it. Only an agent that lists its own settings can be
    /// asked for it, and that agent is told exactly this.
    Said(String),
}

/// What the routing model heard him name: every launcher's offer it said is that one, each with that
/// launcher's own id for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Heard {
    pub(super) offered: Vec<Offered>,
}

/// One launcher's offer of a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Offered {
    pub(super) launcher: LauncherId,
    pub(super) choice: airband_core::tools::Choice,
}

impl Heard {
    /// This launcher's offers of what was heard.
    fn of<'a>(&'a self, launcher: &'a LauncherId) -> impl Iterator<Item = &'a Offered> + 'a {
        self.offered.iter().filter(move |o| o.launcher == *launcher)
    }
    /// How it is said to him: each name it was offered under, once, for the wording alone.
    fn said(&self) -> String {
        let mut names: Vec<&str> = Vec::new();
        self.offered.iter().for_each(|o| if !names.contains(&o.choice.name.as_str()) { names.push(&o.choice.name) });
        either(&names)
    }
}

/// The one of `choices` he asked for: by the tool's own id, or by this launcher's own offer of what was heard.
/// Something on no list goes as he said it only to an agent that lists its own settings (`agent_says`).
/// Anything else is his word back, for telling him it is not there.
fn chosen(asked: Option<&Pick>, launcher: &LauncherId, choices: &[airband_core::tools::Choice], agent_says: bool) -> Result<Option<airband_core::tools::Choice>, String> {
    let found = |is: &dyn Fn(&airband_core::tools::Choice) -> bool| choices.iter().find(|c| is(c)).cloned();
    match asked {
        None => Ok(None),
        Some(Pick::Id(id)) => found(&|c| c.id == *id).map(Some).ok_or_else(|| id.clone()),
        // the id this launcher gave it: one another launcher offers under the same name is not this one's
        Some(Pick::Heard(heard)) => match heard.of(launcher).collect::<Vec<_>>().as_slice() {
            [one] => found(&|c| c.id == one.choice.id).map(Some).ok_or_else(|| heard.said()),
            _ => Err(heard.said()),
        },
        Some(Pick::Said(said)) if agent_says => Ok(Some(airband_core::tools::Choice { id: said.clone(), name: said.clone() })),
        Some(Pick::Said(said)) => Err(said.clone()),
    }
}

/// What was heard, of a pick from his words.
fn heard(pick: Option<&Pick>) -> Option<&Heard> {
    match pick {
        Some(Pick::Heard(heard)) => Some(heard),
        _ => None,
    }
}

/// Which launcher starts an agent on something he named, of those that offer it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Starts {
    This(LauncherId),
    /// None offers it.
    Nobody,
    /// Several do and none is his usual: he is asked which.
    Which(Vec<LauncherId>),
}

/// The rule, the same with a tool named or none: his usual launcher (for that tool, or at all) when it offers
/// it; else the one launcher that does; else he is asked. Where a launcher stands in any list decides nothing.
pub(super) fn which_launcher(usual: Option<&LauncherId>, offering: &[LauncherId]) -> Starts {
    if let Some(usual) = usual.filter(|u| offering.contains(u)) {
        return Starts::This(usual.clone());
    }
    match offering {
        [] => Starts::Nobody,
        [one] => Starts::This(one.clone()),
        several => Starts::Which(several.to_vec()),
    }
}

/// Why no launcher was settled on: words for him, or a question of which of several.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unsettled {
    Say(String),
    /// Several offer what he named and none is his usual: `what` he named, and each launcher with the name he
    /// can say it by.
    Which { what: String, among: Vec<(LauncherId, String)> },
}

impl Unsettled {
    /// As it is put to him.
    pub(super) fn words(&self) -> String {
        match self {
            Unsettled::Say(said) => said.clone(),
            Unsettled::Which { what, among } => format!("Which should start it on {what}: {}?", either(&among.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>())),
        }
    }
}

/// Each launcher by a name he can say back: its tool's name, from the tool's row (or the name he gave a
/// launcher of his own for a tool airband has no row for), and where two of these start the same tool, how
/// each runs it: by airband, or in a terminal of its own; two that run it alike by the names he gave them.
pub(super) fn sayable(cfg: &config::Config, among: &[LauncherId]) -> Vec<(LauncherId, String)> {
    let tool = |id: &LauncherId, l: &config::Launcher| crate::adapters::tool_row(&l.harness).map(|row| row.name.clone()).unwrap_or_else(|| id.to_string());
    among
        .iter()
        .filter_map(|id| cfg.launchers.get(id.as_str()).map(|l| (id, l)))
        .map(|(id, l)| {
            let alike: Vec<&config::Launcher> = among.iter().filter(|o| *o != id).filter_map(|o| cfg.launchers.get(o.as_str())).filter(|o| o.harness == l.harness && !l.harness.is_empty()).collect();
            let name = match (alike.is_empty(), alike.iter().all(|o| o.held.is_some() != l.held.is_some())) {
                (true, _) => tool(id, l),
                (false, true) if l.held.is_some() => format!("{} run by airband", tool(id, l)),
                (false, true) => format!("{} in a terminal", tool(id, l)),
                (false, false) => id.to_string(),
            };
            (id.clone(), name)
        })
        .collect()
}

/// The launcher that starts the agent. One he chose when asked which (`chosen`) starts it when it offers what
/// he named. Else, with nothing heard from his words to start it on: the launcher for the tool he named, else
/// the usual one (`held::launcher_for`). With a model or an effort heard: of the launchers that offer all of it
/// (only the named tool's, when he named one), by `which_launcher`. None offering it is said to him in words,
/// several and none usual is a question, and nothing starts.
pub(super) fn starter(app: &App, idea: &Idea, chosen: Option<&LauncherId>) -> Result<(String, config::Launcher), Unsettled> {
    let say = Unsettled::Say;
    let wanted: Vec<&Heard> = [idea.model.as_ref(), idea.effort.as_ref()].into_iter().filter_map(heard).collect();
    let usual = held::launcher_for(app, idea.tool.as_deref());
    if wanted.is_empty() {
        return usual.ok_or_else(|| say(match &idea.tool {
            Some(tool) => format!("This computer is not set up to start {tool} agents."),
            None => "Which coding agent should start it? Name one, or choose yours in Settings, under Coding agents.".to_string(),
        }));
    }
    let what = wanted.iter().map(|h| h.said()).collect::<Vec<_>>().join(" at ");
    let cfg = app.cfg();
    let offering: Vec<LauncherId> = cfg
        .launchers
        .iter()
        .filter(|(_, l)| idea.tool.as_ref().is_none_or(|tool| l.harness == *tool))
        .map(|(name, _)| LauncherId::new(name.as_str()))
        .filter(|id| wanted.iter().all(|h| h.of(id).next().is_some()))
        .collect();
    // the one he chose when asked, which must still offer it: the rule is not asked again
    let usual = match chosen {
        Some(chosen) if offering.contains(chosen) => Some(chosen.clone()),
        Some(_) => return Err(say(format!("That one does not start an agent on {what}."))),
        None => usual.map(|(name, _)| LauncherId::new(name)),
    };
    let picked = match which_launcher(usual.as_ref(), &offering) {
        Starts::This(picked) => picked,
        Starts::Nobody => {
            return Err(say(match &idea.tool {
                Some(tool) => format!("{tool} agents cannot be started on {what}."),
                None => format!("Nothing here starts an agent on {what}."),
            }));
        }
        Starts::Which(several) => return Err(Unsettled::Which { what, among: sayable(&cfg, &several) }),
    };
    // two of what was heard offered by this one launcher: he is asked which he meant
    if let Some(h) = wanted.iter().find(|h| h.of(&picked).nth(1).is_some()) {
        return Err(say(format!("Which one: {}?", either(&h.of(&picked).map(|o| o.choice.name.as_str()).collect::<Vec<_>>()))));
    }
    let Some(launcher) = cfg.launchers.get(picked.as_str()) else { return Err(say("The way of starting that agent is no longer set up here.".into())) };
    Ok((picked.to_string(), launcher.clone()))
}

/// Start a new agent in `place`. `Ok` once it has started, with what to tell him; `Err` with why nothing started.
pub(super) async fn launch(app: &Arc<App>, place: &Place, idea: &Idea) -> Result<Reply, Reply> {
    launch_by(app, place, idea, None).await
}

/// The same, by the launcher he chose when asked which.
pub(super) async fn launch_by(app: &Arc<App>, place: &Place, idea: &Idea, by: Option<&LauncherId>) -> Result<Reply, Reply> {
    let task = idea.task.as_str();
    let say = |s: String| reply(s, false);
    let (project, cwd) = (place.name.as_str(), place.folder.clone());
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(say(format!("The folder for {project} is gone.")));
    }
    let (named, launcher) = starter(app, idea, by).map_err(|e| say(e.words()))?;
    let launcher = &launcher;
    // An agent the tower runs can be started with nothing to do yet: it is there at once, waiting for his first
    // word. One that is handed off needs its task to start with.
    if task.is_empty() && launcher.held.is_none() {
        return Err(Reply { say: what_to_do(Some(project)), listen: true, ..Reply::default() });
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
                // what the holder or its program said can be the tool's own words: the log has it, he is told where
                log(format!("launch in {project} failed: {e}"));
                Err(say(format!("Could not launch in {project}: the agent did not start. This computer's airband log says why.")))
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
    // A launcher hands the agent off and ends: its ending is what says how it went, waited for as long as it
    // takes. Never cut short on a clock while it goes on, which could start a second agent when he asks again.
    let run = tokio::process::Command::new(program).args(args).current_dir(&cwd).env_remove(INTERNAL_ENV).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match run.await {
        Ok(out) if out.status.success() => {
            log(format!("launched {named}{with} in {project} ({session}), with a task of {} characters", task.chars().count()));
            // handed off to a program of its own: nothing says which agent it becomes when it reports in
            Ok(say(launched(app, place, &with, task, None, line)))
        }
        Ok(out) => {
            // what it wrote can hold its command line, and so his task: neither the log nor he is given it
            log(format!("launch in {project} failed: {program} ended with {} after writing {} bytes of errors", out.status, out.stderr.len()));
            Err(say(format!("Could not launch in {project}: {program} stopped with an error.")))
        }
        Err(e) => {
            log(format!("launch in {project} failed: cannot run {program}: {e}"));
            Err(say(format!("Could not launch in {project}: {program} did not start.")))
        }
    }
}

/// The one way he is asked what a new agent should do: with the project when it is known.
pub(super) fn what_to_do(project: Option<&str>) -> String {
    match project {
        Some(project) => format!("What should it do? Say launch {project}, then the task."),
        None => "What should it do? Say launch, then the task.".to_string(),
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

    /// What the routing model heard: one name, offered by each of these launchers under its own id.
    fn heard(name: &str, by: &[(&LauncherId, &str)]) -> Pick {
        Pick::Heard(Heard { offered: by.iter().map(|(l, id)| Offered { launcher: (*l).clone(), choice: Choice { id: id.to_string(), name: name.into() } }).collect() })
    }

    #[test]
    fn a_model_is_taken_by_its_id_or_this_launchers_own_id_for_the_heard_one_and_never_by_a_near_word() {
        let models = vec![Choice { id: "gpt-6.2-sol".into(), name: "6.2 Sol".into() }, Choice { id: "gpt-6-luna".into(), name: "6 Luna".into() }];
        let (here, other) = (LauncherId::new("codex"), LauncherId::new("held-codex"));
        let id = |r: Result<Option<Choice>, String>| r.map(|c| c.map(|c| c.id));
        assert_eq!(id(chosen(Some(&Pick::Id("gpt-6-luna".into())), &here, &models, false)), Ok(Some("gpt-6-luna".into())));
        // what was heard is this launcher's by the id this launcher gave it
        assert_eq!(id(chosen(Some(&heard("6.2 Sol", &[(&other, "sol-2"), (&here, "gpt-6.2-sol")])), &here, &models, false)), Ok(Some("gpt-6.2-sol".into())));
        // one only another launcher offers under that name is not this launcher's, however its name reads
        assert_eq!(id(chosen(Some(&heard("6.2 Sol", &[(&other, "gpt-6.2-sol")])), &here, &models, false)), Err("6.2 Sol".into()));
        // nothing loosened: a name used as an id, or a word on no list, is not it
        assert_eq!(id(chosen(Some(&Pick::Id("6 Luna".into())), &here, &models, false)), Err("6 Luna".into()));
        assert_eq!(id(chosen(Some(&Pick::Said("luna".into())), &here, &models, false)), Err("luna".into()));
        // an agent that lists its own settings is asked for exactly what he said
        assert_eq!(id(chosen(Some(&Pick::Said("luna".into())), &here, &models, true)), Ok(Some("luna".into())));
        assert_eq!(id(chosen(None, &here, &models, false)), Ok(None));
    }

    #[test]
    fn the_launcher_is_his_usual_when_it_offers_it_else_the_only_one_else_he_is_asked_and_no_order_decides() {
        let (a, b, c) = (LauncherId::new("a"), LauncherId::new("b"), LauncherId::new("c"));
        // his usual, wherever it stands among those that offer it
        assert_eq!(which_launcher(Some(&b), &[a.clone(), b.clone()]), Starts::This(b.clone()));
        assert_eq!(which_launcher(Some(&b), &[b.clone(), a.clone()]), Starts::This(b.clone()));
        // his usual does not offer it: the one that does
        assert_eq!(which_launcher(Some(&c), std::slice::from_ref(&a)), Starts::This(a.clone()));
        assert_eq!(which_launcher(None, std::slice::from_ref(&a)), Starts::This(a.clone()));
        // several, and none is his usual: he is asked, never given the first
        assert_eq!(which_launcher(Some(&c), &[a.clone(), b.clone()]), Starts::Which(vec![a.clone(), b.clone()]));
        assert_eq!(which_launcher(None, &[b.clone(), a.clone()]), Starts::Which(vec![b.clone(), a.clone()]));
        assert_eq!(which_launcher(Some(&c), &[]), Starts::Nobody);
    }

    #[test]
    fn a_model_he_names_is_started_by_the_tool_he_named_else_his_usual_else_the_only_one_else_he_is_asked() {
        let app = with_launchers("starter", "a", serde_json::json!({
            "a": { "harness": "tool-one", "argv": ["true"], "models": { "Sol": "sol-a" } },
            "b": { "harness": "tool-two", "argv": ["true"], "models": { "Sol": "sol-b", "Luna": "luna-b" }, "efforts": ["high"] },
            "c": { "harness": "tool-three", "argv": ["true"], "models": { "Luna": "luna-c", "Nova": "nova-c" } },
            "d": { "harness": "tool-four", "argv": ["true"], "models": { "Nova": "nova-d" } },
        }));
        let (a, b, c, d) = (LauncherId::new("a"), LauncherId::new("b"), LauncherId::new("c"), LauncherId::new("d"));
        let started = |idea: Idea| starter(&app, &idea, None).map(|(name, _)| name).map_err(|e| e.words());
        let sol = |by: &[(&LauncherId, &str)]| Some(heard("Sol", by));
        // (a) the tool he named, when it offers it, though his usual offers it too
        assert_eq!(started(Idea { tool: Some("tool-two".into()), model: sol(&[(&a, "sol-a"), (&b, "sol-b")]), ..Idea::default() }), Ok("b".into()));
        // (a) the tool he named does not offer it: refused in words, nothing swapped
        assert_eq!(started(Idea { tool: Some("tool-one".into()), model: Some(heard("Luna", &[(&b, "luna-b"), (&c, "luna-c")])), ..Idea::default() }), Err("tool-one agents cannot be started on Luna.".into()));
        // (b) no tool named: his usual, whichever way round the offers came
        assert_eq!(started(Idea { model: sol(&[(&a, "sol-a"), (&b, "sol-b")]), ..Idea::default() }), Ok("a".into()));
        assert_eq!(started(Idea { model: sol(&[(&b, "sol-b"), (&a, "sol-a")]), ..Idea::default() }), Ok("a".into()));
        // (c) his usual does not offer it and one launcher does
        assert_eq!(started(Idea { model: Some(heard("Sol", &[(&b, "sol-b")])), ..Idea::default() }), Ok("b".into()));
        // a model and an effort: only a launcher that offers both
        assert_eq!(started(Idea { model: Some(heard("Luna", &[(&b, "luna-b"), (&c, "luna-c")])), effort: Some(heard("high", &[(&b, "high")])), ..Idea::default() }), Ok("b".into()));
        assert_eq!(started(Idea { model: sol(&[(&a, "sol-a")]), effort: Some(heard("high", &[(&b, "high")])), ..Idea::default() }), Err("Nothing here starts an agent on Sol at high.".into()));
        // (d) several offer it and none is his usual: he is asked which, by name, either way round
        assert_eq!(started(Idea { model: Some(heard("Nova", &[(&c, "nova-c"), (&d, "nova-d")])), ..Idea::default() }), Err("Which should start it on Nova: c or d?".into()));
        assert_eq!(started(Idea { model: Some(heard("Nova", &[(&d, "nova-d"), (&c, "nova-c")])), ..Idea::default() }), Err("Which should start it on Nova: c or d?".into()));
        // nothing heard: the tool he named, else the usual
        assert_eq!(started(Idea { tool: Some("tool-two".into()), ..Idea::default() }), Ok("b".into()));
        assert_eq!(started(Idea { model: Some(Pick::Said("luna".into())), ..Idea::default() }), Ok("a".into()));
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
    fn launchers_are_offered_by_names_he_can_say_back_and_the_one_he_chose_starts_it() {
        let held = serde_json::json!({ "by": "lines", "fresh": [], "resume": [], "line": "{text}", "stop": "{id}" });
        let app = with_launchers("spoken", "mine", serde_json::json!({
            "mine": { "harness": "tool-one", "argv": ["true"] },
            "x": { "harness": "claude-code", "argv": ["true"], "models": { "Nova": "nova-x" }, "held": held },
            "y": { "harness": "claude-code", "argv": ["true"], "models": { "Nova": "nova-y" } },
            "z": { "harness": "tool-four", "argv": ["true"], "models": { "Nova": "nova-z" } },
        }));
        let (x, y, z) = (LauncherId::new("x"), LauncherId::new("y"), LauncherId::new("z"));
        let named = sayable(&app.cfg(), &[x.clone(), y.clone(), z.clone()]);
        // by the tool's name from its row, and how each runs it where two start the same tool; one of his own for
        // a tool with no row by the name he gave it
        let row = crate::adapters::tool_row("claude-code").unwrap().name.clone();
        assert_eq!(named, vec![(x.clone(), format!("{row} run by airband")), (y.clone(), format!("{row} in a terminal")), (z.clone(), "z".into())]);
        assert_eq!(sayable(&app.cfg(), &[x.clone(), z.clone()])[0].1, row);
        let nova = || Some(heard("Nova", &[(&x, "nova-x"), (&y, "nova-y"), (&z, "nova-z")]));
        let idea = Idea { model: nova(), ..Idea::default() };
        // several and none usual: a question naming each, typed
        assert!(matches!(starter(&app, &idea, None), Err(Unsettled::Which { among, .. }) if among == named));
        // his answer: the one he chose starts it, and one that does not offer it is refused
        assert_eq!(starter(&app, &idea, Some(&y)).map(|(n, _)| n), Ok("y".into()));
        assert_eq!(starter(&app, &Idea { model: Some(heard("Nova", &[(&x, "nova-x")])), ..Idea::default() }, Some(&y)).map(|(n, _)| n).map_err(|e| e.words()), Err("That one does not start an agent on Nova.".into()));
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
