//! The second half of the dispatcher: what his words are for. What needs no reading comes first: a typed
//! command, tags (airband-core's parser), the message he tapped. Everything he says in his own words a model
//! reads: which agent it is for (a callsign he says, a team, all of them), or that he wants a new agent, where,
//! and on which model and effort. Also the controls the tower answers itself (`/more`, `/status`, `/hold`).
use super::*;

pub(super) fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// How an untagged reply found its recipient.
pub(super) enum Routed {
    /// Certain: he named them, or the radio said which message he was answering.
    Sure(Vec<Session>),
    /// Nobody to choose from, or the choice could not be made: ask him.
    Ask(Vec<String>),
    /// It is not for anyone running: he wants a new agent started.
    New(Idea),
    /// It is for an agent on another of his computers, by this callsign.
    Elsewhere(String),
}

/// A typed command to start an agent: `/launch shop with opus: …`. It says what it is; everything he says in
/// his own words is the model's to read.
fn new_by_command(app: &App, text: &str, projects: &[String]) -> Option<Idea> {
    let (models, efforts) = app.cfg.launchers.get(&app.cfg.launcher).map(|l| (l.models.keys().cloned().collect::<Vec<_>>(), l.efforts.clone())).unwrap_or_default();
    let l = airband_core::routing::parse_launch(text, projects)?;
    Some(match airband_core::routing::parse_with(&l.task, &models, &efforts).filter(|w| !w.rest.is_empty()) {
        Some(w) => Idea { tool: None, project: l.project, model: Some(w.model), effort: w.effort, task: w.rest },
        None => Idea { tool: None, project: l.project, model: None, effort: None, task: l.task },
    })
}

/// Who an untagged reply is for, or that it asks for a new agent. What is certain without reading comes first
/// (the message he tapped). After that a model reads his words: against what each agent has been saying, and
/// against the projects, models and efforts a new agent could be started with.
pub(super) async fn route(app: &Arc<App>, radio: &str, rest: &str, re: Option<&str>, scope: Option<&str>, others: &[airband_core::radio::Other]) -> Routed {
    // the room he was looking at, when he was in one: only its agents can be meant
    let room = scope.and_then(|s| s.strip_prefix("group:")).map(str::to_string);
    let within = |s: &Session| room.as_ref().is_none_or(|g| s.callsign.project == *g);
    // candidates: every agent that has said something since he last spoke, newest first
    let (candidates, focus, all, quiet) = {
        let inner = app.inner.lock().unwrap();
        // he picked the message he is answering: it goes to the agent that said it, whatever that agent is
        // called now, and to nobody if that agent has gone
        if let Some(target) = re.and_then(|id| inner.log.iter().find(|e| e.id == id)).and_then(|e| inner.sender(e)) {
            return Routed::Sure(vec![target.clone()]);
        }
        let since = inner.log.iter().rposition(|e| e.from == "you").map(|i| i + 1).unwrap_or(0);
        let mut seen: Vec<(Session, Entry)> = Vec::new();
        for e in inner.log[since..].iter().rev().filter(|e| e.kind != Some(Kind::Fyi)) {
            if let Some(s) = inner.sender(e).filter(|s| within(s)) {
                if !seen.iter().any(|(have, _)| have.agent.id == s.agent.id) {
                    seen.push((s.clone(), e.clone()));
                }
            }
        }
        let focus = inner.calls.get(radio).and_then(|c| c.focus.clone()).and_then(|f| inner.find(&f).filter(|s| within(s)).cloned());
        // the last thing he said and who it went to, so "and keep the price next to it" can be read as carrying on
        let before = inner.log.iter().rev().find(|e| e.from == "you" && !e.to_agents.is_empty()).and_then(|e| {
            let to: Vec<String> = e.to_agents.iter().filter_map(|id| inner.find(id)).filter(|s| within(s)).map(|s| s.callsign.text()).collect();
            (!to.is_empty()).then(|| (e.at, to, opening(&e.full)))
        });
        let all: Vec<Session> = inner.sessions.iter().filter(|s| within(s)).cloned().collect();
        // the rest of the room: agents that have not spoken since, each with the last thing it said, if any
        let quiet: Vec<(Session, Option<Entry>)> = all
            .iter()
            .filter(|s| !seen.iter().any(|(have, _)| have.agent.id == s.agent.id))
            .map(|s| (s.clone(), inner.last_said(&s.agent.id).cloned()))
            .collect();
        (seen, focus.map(|f| (f, before)), all, quiet)
    };
    // what a new agent could be started in and on, if anything
    let launcher = app.cfg.launchers.get(&app.cfg.launcher);
    let projects: Vec<String> = app.inner.lock().unwrap().launchable(&app.cfg).into_keys().collect();
    // with agents of another computer on his screen there is always more than one it could be for
    let can_start = (launcher.is_some() && !projects.is_empty()) || !others.is_empty();
    let launcher = launcher.filter(|_| !projects.is_empty());
    match (candidates.as_slice(), all.as_slice()) {
        (_, []) if !can_start => Routed::Ask(Vec::new()),
        // only one agent it could be, and nothing else it could mean
        (_, [only]) if !can_start => Routed::Sure(vec![only.clone()]),
        (several, _) => {
            // more than one agent could be meant. His words decide, read against what each has been saying:
            // "ship the login fix" goes to whoever is working on the login fix, whoever spoke last.
            let now = now();
            // each with the two letters the app shows for it: he may say those and not the name
            let letters = |s: &Session| airband_core::routing::badge(&s.callsign.project, &s.callsign.text());
            let line = |s: &Session, e: &Entry, since: bool| {
                format!("- {} (letters {}; {} ago{}{}): {}", s.callsign.text(), letters(s), ago(now.saturating_sub(e.at)), if since { "" } else { ", before he last spoke" }, if e.kind == Some(Kind::NeedsYou) { ", waiting on him" } else { "" }, e.short.clone().unwrap_or_else(|| opening(&e.full)))
            };
            let mut lines: Vec<String> = several.iter().take(6).map(|(s, e)| line(s, e, true)).collect();
            lines.extend(quiet.iter().take(12usize.saturating_sub(lines.len())).map(|(s, e)| match e {
                Some(e) => line(s, e, false),
                None => format!("- {} (letters {}; has said nothing yet)", s.callsign.text(), letters(s)),
            }));
            // the agents he can see that another of his computers has: his words may be for one of them
            lines.extend(others.iter().take(8).map(|o| {
                let letters = airband_core::routing::badge(&o.group, &o.callsign);
                if o.said.is_empty() { format!("- {} (letters {letters}; on another of his computers; has said nothing yet)", o.callsign) } else { format!("- {} (letters {letters}; on another of his computers; {} ago): {}", o.callsign, ago(o.ago * 1000), o.said) }
            }));
            let last = match &focus {
                Some((_, Some((at, to, said)))) => format!("The last thing he said, {} ago, was to {}: \"{said}\"\n", ago(now.saturating_sub(*at)), list(to)),
                Some((s, None)) => format!("The agent he last spoke with is {}.\n", s.callsign.text()),
                None => String::new(),
            };
            // the other thing his words can be: a request for a new agent, on a model and at an effort he may name
            let starting = match launcher {
                None => String::new(),
                Some(l) => {
                    let lately: Vec<String> = {
                        let inner = app.inner.lock().unwrap();
                        projects
                            .iter()
                            .map(|p| {
                                let said: Vec<String> = inner.log.iter().rev().filter(|e| e.group == *p && e.from != "you" && e.kind != Some(Kind::Fyi)).take(2).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
                                format!("- {p}{}", if said.is_empty() { String::new() } else { format!(": lately, {}", said.join(" / ")) })
                            })
                            .collect()
                    };
                    // every model and effort any of the tools offers: which tool he means is settled after
                    let across = |of: fn(&config::Launcher) -> Vec<String>| {
                        let mut all: Vec<String> = of(l);
                        app.cfg.launchers.values().flat_map(of).for_each(|n| if !all.contains(&n) { all.push(n) });
                        all
                    };
                    let tools = app.cfg.tools();
                    let tool = if tools.len() > 1 { format!("\"tool\" is one of {} when he asked for that tool by name (\"start a codex agent\", \"have codex look into…\"), else null. ", tools.join(", ")) } else { "\"tool\" is null. ".to_string() };
                    let offered = |what: &str, names: Vec<String>| if names.is_empty() { format!("\"{what}\" is null. ") } else { format!("\"{what}\" is one of {} when he named one, else null. ", names.join(", ")) };
                    format!(
                        "\n\nOr he is not talking to any of them: he wants a NEW agent started. Decide this first. He says so \
                         outright (\"new idea: …\", \"start an agent to…\", \"launch…\", \"spin up…\": words like these always \
                         mean a new agent, whatever the work is and whoever is already working on something like it), or he \
                         asks for work to be done by a model he names (\"with fable, look into…\", \"with opus high, \
                         investigate…\", \"have opus investigate…\", \"get sonnet on high to review…\"). Work he simply asks \
                         for, with no such sign, is for an agent already running. A remark about a model (\"with opus it was \
                         too slow\") is not a request for one. When he wants a new agent, \"to\" is empty and \"new\" says \
                         what to start: \"project\" is the one he named, else the one the work would be done in{room}, going by \
                         who the work is for and what it changes more than by what data it happens to read (a fair \
                         choice beats asking him: he is told where it started), else null when nothing points to \
                         any of them; {tool}{models}{efforts}\"task\" is the work in his own words, without the words \
                         about starting an agent, the tool, the model or the effort. Otherwise \"new\" is null. The projects:\n{projects}",
                        room = room.as_ref().map(|r| format!(" (he is looking at {r})")).unwrap_or_default(),
                        tool = tool,
                        models = offered("model", across(|l| l.models.keys().cloned().collect())),
                        efforts = offered("effort", across(|l| l.efforts.clone())),
                        projects = lately.join("\n")
                    )
                }
            };
            let prompt = format!(
                "{name} runs several coding agents. Each is listed by its callsign, with the last thing it said, the ones that have spoken since he last did first, newest first:\n{lines}\n{last}\n\
                 He now says: \"{rest}\"\n\n\
                 Who is it for? Each agent's letters are the label his phone shows for it. If he names one by its \
                 letters (\"to SR\", \"tell M3\", \"S R, go ahead\"; speech may write them apart, or as words that sound \
                 like them) it is for that agent and you can be sure of it. A team's name by itself (\"to catalog\", \"catalog, how is…\": the \
                 first word or words of its agents' callsigns) names the team, not one of its agents, even when a number \
                 stands near it: among that team's agents it is for the one whose work his words are about, and you ask \
                 if that is not plain. If his words are plainly about one agent's work or answer one agent's question, it is that agent, \
                 and you can be sure of it however recently another one spoke: \"refund them all\" is for whoever is working on refunds. \
                 Otherwise a reply right after an agent speaks is to that agent, and with nobody having spoken since he last did \
                 it carries on with the agent he last spoke with: words that only make sense as more of what he was just saying \
                 (\"and make it blue\", \"also the footer\") are for the same agent, and you can be sure of that too. \
                 A callsign he says in full (\"catalog two, leave it\"; speech may write its number as a figure, or as a word \
                 that sounds like it) is that agent, and you can be sure of it. \
                 Name more than one only if his words plainly address several: every agent of one team (\"all of catalog, \
                 pull main\") is each of that team's callsigns, and every agent there is (\"everyone, stop\", \"all of you, \
                 commit what you have\") is \"to\":[\"*\"]. A sentence that only mentions them all (\"everyone is waiting on \
                 the deploy\") addresses whoever its words are for, not all of them.{starting}\n\
                 Reply with one line of JSON and nothing else: {{\"to\":[\"<callsign in full, not its letters>\"],\"sure\":true or false{shape}}}\n\
                 \"sure\": false if a careful assistant would check with him before passing this on.",
                name = app.cfg.name,
                lines = if lines.is_empty() { "(none are running)".to_string() } else { lines.join("\n") },
                starting = starting,
                shape = if can_start { ",\"new\":null or {\"tool\":…,\"project\":…,\"model\":…,\"effort\":…,\"task\":\"…\"}" } else { "" }
            );
            let everyone: Vec<&Session> = several.iter().map(|(s, _)| s).chain(quiet.iter().map(|(s, _)| s)).collect();
            #[derive(Deserialize)]
            struct New {
                #[serde(default)]
                tool: Option<String>,
                project: Option<String>,
                model: Option<String>,
                effort: Option<String>,
                #[serde(default)]
                task: String,
            }
            #[derive(Deserialize)]
            struct Out {
                #[serde(default)]
                to: Vec<String>,
                #[serde(default)]
                sure: bool,
                #[serde(default)]
                new: Option<New>,
            }
            let answer = ask_model(&app.cfg, prompt.clone()).await;
            if std::env::var_os("AIRBAND_TRACE").is_some() {
                log(format!("routing asked:\n{prompt}\nand was told: {answer:?}"));
            }
            let judged = answer.ok().and_then(|out| serde_json::from_str::<Out>(judge::json_in(&out)?).ok());
            let Some(mut judged) = judged else {
                // No model answered. Nothing is read into his words: with one agent it is for that one, and
                // with several he is asked.
                return match all.as_slice() {
                    [] => Routed::Ask(Vec::new()),
                    [only] => Routed::Sure(vec![only.clone()]),
                    several => Routed::Ask(several.iter().take(3).map(|s| s.callsign.text()).collect()),
                };
            };
            if let Some(new) = judged.new.take().filter(|_| can_start) {
                // only what exists is taken from the model: a project the tower knows, a model and an effort the
                // launcher offers. Anything else it wrote is dropped, not passed to a command line.
                let same = |a: &str, b: &str| a.replace(' ', "").eq_ignore_ascii_case(&b.replace(' ', ""));
                let known = |said: Option<String>, names: Vec<String>| said.and_then(|x| names.into_iter().find(|n| same(n, x.trim())));
                // the tool he named, if a launcher starts it; its models and efforts are then the ones that count
                let tool = known(new.tool, app.cfg.tools());
                let launcher = tool.as_deref().and_then(|t| app.cfg.launcher_for(Some(t))).map(|(_, l)| l).or(launcher);
                return Routed::New(Idea {
                    tool,
                    project: known(new.project, projects.clone()).or_else(|| (projects.len() == 1).then(|| projects[0].clone())),
                    model: known(new.model, launcher.map(|l| l.models.keys().cloned().collect()).unwrap_or_default()),
                    effort: known(new.effort, launcher.map(|l| l.efforts.clone()).unwrap_or_default()),
                    task: if new.task.trim().is_empty() { rest.to_string() } else { new.task.trim().to_string() },
                });
            }
            if all.is_empty() {
                return Routed::Ask(Vec::new());
            }
            // one of the agents on another computer, and none here: the radio takes it there
            if let Some(o) = others.iter().find(|o| judged.to.iter().any(|c| c.trim().eq_ignore_ascii_case(&o.callsign))).filter(|_| judged.sure && !everyone.iter().any(|s| judged.to.iter().any(|c| c.trim().eq_ignore_ascii_case(&s.callsign.text())))) {
                return Routed::Elsewhere(o.callsign.clone());
            }
            // all of them, said outright
            if judged.sure && judged.to.iter().any(|c| c.trim() == "*") {
                return Routed::Sure(all.clone());
            }
            let judged = Some(judged);
            // the model names an agent by its callsign; its letters are taken too when they are one agent's alone
            let answers = |c: &str, s: &Session| {
                let c = c.trim();
                c.eq_ignore_ascii_case(&s.callsign.text()) || (c.eq_ignore_ascii_case(&letters(s)) && everyone.iter().filter(|o| letters(o).eq_ignore_ascii_case(c)).count() == 1)
            };
            let picked: Vec<Session> = judged.as_ref().map(|o| everyone.iter().filter(|s| o.to.iter().any(|c| answers(c, s))).map(|s| (*s).clone()).collect()).unwrap_or_default();
            if judged.is_some_and(|o| o.sure) && !picked.is_empty() {
                Routed::Sure(picked)
            } else {
                // the likeliest first, so "the first one" or a bare callsign settles it
                let mut names: Vec<String> = picked.iter().map(|s| s.callsign.text()).collect();
                everyone.iter().take(3).for_each(|s| if !names.contains(&s.callsign.text()) { names.push(s.callsign.text()) });
                Routed::Ask(names)
            }
        }
    }
}

pub(super) fn ago(ms: u64) -> String {
    match ms / 1000 {
        s if s < 90 => format!("{s} seconds"),
        s if s < 5400 => format!("{} minutes", s / 60),
        s => format!("{} hours", s / 3600),
    }
}

/// Which project a new agent's task belongs in, when he gave the idea and no project. With one project there is
/// nothing to choose. With several, a model reads the idea against what each project's agents have been doing and
/// picks the best fit: he is told where it started, so a fair guess beats a question. Only when nothing in the
/// idea points anywhere is he asked, and the idea is kept for his answer.
async fn place_idea(app: &Arc<App>, idea: &str, projects: &[String]) -> Result<String, Reply> {
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
        let inner = app.inner.lock().unwrap();
        let folders = inner.launchable(&app.cfg);
        projects
            .iter()
            .map(|p| {
                let lately: Vec<String> = inner.log.iter().rev().filter(|e| e.group == *p && e.from != "you" && e.kind != Some(Kind::Fyi)).take(2).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
                format!("- {p} (folder {}){}", folders.get(p).map(String::as_str).unwrap_or("?"), if lately.is_empty() { String::new() } else { format!(": lately, {}", lately.join(" / ")) })
            })
            .collect()
    };
    let prompt = format!(
        "{name} wants to start a new coding agent on this: \"{idea}\"\n\nThese are his projects:\n{lines}\n\n\
         Which project would this work be done in? Go by who the work is for and what it changes, more than by \
         what data it happens to read. Pick the best fit. He is told which project it started in and can stop \
         it, so a reasonable choice is better than asking him.\n\
         Reply with one line of JSON and nothing else: {{\"project\":\"<name exactly as listed>\"}}\n\
         Leave the name empty only if nothing in the idea points to any of these projects.",
        name = app.cfg.name,
        lines = lines.join("\n")
    );
    #[derive(Deserialize)]
    struct Out {
        project: String,
    }
    let raw = ask_model(&app.cfg, prompt).await;
    let judged = raw.as_ref().ok().and_then(|out| serde_json::from_str::<Out>(judge::json_in(out)?).ok());
    match judged.and_then(|o| projects.iter().find(|p| p.eq_ignore_ascii_case(o.project.trim())).cloned()) {
        Some(project) => Ok(project),
        None => Err(say(format!("Which project is that for? {}.", projects.join(", ")), true)),
    }
}

/// He was asked which project an idea belongs in, and has said something. The project his words name, or
/// nothing when they are not an answer to that (or no model said).
async fn named_project(app: &Arc<App>, idea: &str, said: &str, projects: &[String]) -> Option<String> {
    let prompt = format!(
        "{name} wanted a new coding agent started on this: \"{idea}\"\nHe was asked which of his projects it is for:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         If that answers the question, which project does he mean? Speech may write a name apart or misspell it.\n\
         Reply with one line of JSON and nothing else: {{\"project\":\"<name exactly as listed>\"}}\n\
         Leave the name empty if he names none of them, rules one out without choosing another, or has gone on to \
         say something else.",
        name = app.cfg.name,
        list = projects.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        project: String,
    }
    let out = ask_model(&app.cfg, prompt).await.ok()?;
    let judged: Out = serde_json::from_str(judge::json_in(&out)?).ok()?;
    projects.iter().find(|p| p.eq_ignore_ascii_case(judged.project.trim())).cloned()
}

/// He was asked who something he said was for, and has said something more. The agents his words name, or
/// nothing when they are not an answer to that (or no model said): then they are a message of their own.
async fn named_agents(app: &Arc<App>, first: &str, said: &str, scope: Option<&str>) -> Option<Vec<Session>> {
    let room = scope.and_then(|s| s.strip_prefix("group:"));
    let all: Vec<Session> = app.inner.lock().unwrap().sessions.iter().filter(|s| room.is_none_or(|g| s.callsign.project == g)).cloned().collect();
    let letters = |s: &Session| airband_core::routing::badge(&s.callsign.project, &s.callsign.text());
    let prompt = format!(
        "{name} said this to his coding agents: \"{first}\"\nIt was not clear who it was for, and he was asked. The agents:\n{list}\n\n\
         He now says: \"{said}\"\n\n\
         If that answers the question, who does he mean? He may say a callsign, an agent's letters, or a number \
         (speech may write these apart or as words that sound like them), or all of them.\n\
         Reply with one line of JSON and nothing else: {{\"to\":[\"<callsign in full>\"]}}, with [\"*\"] for all of them.\n\
         \"to\" is empty if he names nobody listed, or has gone on to say something else.",
        name = app.cfg.name,
        list = all.iter().map(|s| format!("- {} (letters {})", s.callsign.text(), letters(s))).collect::<Vec<_>>().join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        to: Vec<String>,
    }
    let out = ask_model(&app.cfg, prompt).await.ok()?;
    let judged: Out = serde_json::from_str(judge::json_in(&out)?).ok()?;
    let meant: Vec<Session> = if judged.to.iter().any(|c| c.trim() == "*") { all } else { all.into_iter().filter(|s| judged.to.iter().any(|c| c.trim().eq_ignore_ascii_case(&s.callsign.text()))).collect() };
    (!meant.is_empty()).then_some(meant)
}

/// Start the new agent he asked for. With no project, a model is asked where the idea belongs when nobody has
/// judged that yet (`place`); if nothing points anywhere he is asked, and the idea is kept for his answer.
async fn start(app: &Arc<App>, radio: &str, text: &str, idea: Idea, projects: &[String], place: bool) -> Reply {
    let project = match (&idea.project, place) {
        (Some(named), _) => named.clone(),
        (None, true) => match place_idea(app, &idea.task, projects).await {
            Ok(project) => project,
            Err(reply) => {
                if reply.listen {
                    app.inner.lock().unwrap().call(radio).unplaced_idea = Some((idea, Instant::now()));
                }
                log(format!("← radio: \"{text}\" → a new agent, project not clear"));
                return reply;
            }
        },
        (None, false) if idea.task.is_empty() || projects.is_empty() => return place_idea(app, &idea.task, projects).await.err().unwrap_or(Reply { say: String::new(), listen: false, ..Reply::default() }),
        (None, false) => {
            app.inner.lock().unwrap().call(radio).unplaced_idea = Some((idea, Instant::now()));
            log(format!("← radio: \"{text}\" → a new agent, project not clear"));
            return Reply { say: format!("Which project is that for? {}.", projects.join(", ")), listen: true, ..Reply::default() };
        }
    };
    log(format!("← radio: \"{text}\" → launch in {project}"));
    launch(app, &project, &idea).await
}

/// `uid` is the utterance's own id, when the radio gave one: deliveries are recorded against it, so a retry after
/// a crash can tell that the message already reached its agent.
/// `label` is the button's words when this is a quick message: the chat shows those, and the agent gets `text`.
pub(super) async fn on_utterance(app: &Arc<App>, radio: &str, uid: Option<&str>, text: &str, re: Option<&str>, scope: Option<&str>, label: Option<&str>, attached: &[String], others: &[airband_core::radio::Other]) -> Reply {
    // an answer to a request for permission goes back to the harness that asked, not to the agent as a message
    // The buttons say yes or no outright. Anything else he says to a request for leave is read by a model, and
    // only a plain yes allows it; if his words do not settle it, nothing is decided.
    let verdict = match permit::asked_leave(app, re) {
        permit::Leave::No => None,
        // an answer to a request that is over answers nothing, and is not passed to the agent as if it did
        permit::Leave::Over => return Reply { say: "That request is no longer waiting. Nothing was allowed.".into(), ..Reply::default() },
        permit::Leave::Waiting if text.trim() == permit::ALLOW => Some(true),
        permit::Leave::Waiting if text.trim() == permit::DENY => Some(false),
        // The model is given his words and nothing of the request, which the asking agent wrote.
        permit::Leave::Waiting => match judge::consent(&app.cfg, text).await {
            Some(true) => Some(true),
            // his own words, which are not a yes: they refuse it and go back to the agent as the reason
            Some(false) => None,
            None => return Reply { say: "That one needs a yes or a no from you.".into(), ..Reply::default() },
        },
    };
    if let Some(reply) = permit::answer(app, text, re, verdict) {
        return reply;
    }
    // files that came with his words: where each is on this computer, for whoever the words go to, and the
    // pictures to show beside them in the chat
    let (mut files, shown_files) = if attached.is_empty() { (String::new(), Vec::new()) } else { attachments::describe(attached).await };
    // starting a new agent comes next: it is addressed to the tower, not to anyone on the radio
    let projects: Vec<String> = app.inner.lock().unwrap().launchable(&app.cfg).into_keys().collect();
    // "Which project?" was just asked about an idea. A model says whether this answers it and with which one.
    // Words that are about something else leave the idea waiting for its answer, for as long as it would have.
    let held_idea = app.inner.lock().unwrap().call(radio).unplaced_idea.take().filter(|(_, when)| when.elapsed() < Duration::from_secs(90));
    if let Some((idea, when)) = held_idea {
        if !text.trim_start().starts_with(['/', '@']) {
            if let Some(project) = named_project(app, &idea.task, text, &projects).await {
                log(format!("← radio: \"{text}\" → launch in {project}"));
                return launch(app, &project, &idea).await;
            }
        }
        app.inner.lock().unwrap().call(radio).unplaced_idea = Some((idea, when));
    }
    // a typed command says what it is and needs no reading
    if text.trim_start().starts_with('/') {
        if let Some(mut idea) = new_by_command(app, text, &projects) {
            idea.task.push_str(&files);
            return start(app, radio, text, idea, &projects, true).await;
        }
    }
    let say = |s: String| Reply { say: s, ..Reply::default() };
    let ask = |s: String| Reply { say: s, listen: true, ..Reply::default() };
    let (mut parsed, named, roster, names, pending, held) = {
        let mut inner = app.inner.lock().unwrap();
        inner.reap();
        let parsed = parse_utterance(text, &inner.callsigns());
        let named: Vec<Session> = parsed.to.iter().map(|&i| inner.sessions[i].clone()).collect();
        if let [one] = named.as_slice() {
            inner.call(radio).focus = Some(one.agent.id.clone());
        }
        let names = inner.sessions.iter().map(|s| s.callsign.text()).collect::<Vec<_>>();
        let held = inner.call(radio).asked.take().filter(|(_, _, when)| when.elapsed() < Duration::from_secs(90)).map(|(t, f, _)| (t, f));
        (parsed, named, inner.roster(), names, inner.pending.len(), held)
    };
    // "Who is that for?" was just asked. A tag and nothing else answers it; so do words of his that a model
    // reads as naming someone. Either way what goes out is what he said the first time.
    let mut named = named;
    if let (Some((words, their_files)), None) = (&held, parsed.control) {
        if parsed.rest.is_empty() && !named.is_empty() {
            (parsed.rest, files) = (words.clone(), their_files.clone());
        } else if named.is_empty() && !parsed.rest.is_empty() {
            if let Some(meant) = named_agents(app, words, &parsed.rest, scope).await {
                named = meant;
                (parsed.rest, files) = (words.clone(), their_files.clone());
            }
        }
    }
    // work out who it is for only when there is something to deliver
    let mut targets: Vec<Session> = if !named.is_empty() {
        named.clone()
    } else if parsed.rest.is_empty() && !matches!(parsed.control, Some(Control::More | Control::Repeat)) {
        Vec::new()
    } else if parsed.control.is_some() {
        // "say more" and "repeat" are about whoever he last heard
        let inner = app.inner.lock().unwrap();
        inner.calls.get(radio).and_then(|c| c.focus.clone()).and_then(|f| inner.find(&f).cloned()).or((inner.sessions.len() == 1).then(|| inner.sessions[0].clone())).into_iter().collect()
    } else {
        match route(app, radio, &parsed.rest, re, scope, others).await {
            Routed::Sure(t) => t,
            Routed::Elsewhere(callsign) => {
                log(format!("← radio: \"{text}\" → {callsign}, on another computer"));
                return Reply { elsewhere: Some(callsign), ..Reply::default() };
            }
            Routed::New(mut idea) => {
                idea.task.push_str(&files);
                return start(app, radio, text, idea, &projects, false).await;
            }
            Routed::Ask(options) if options.is_empty() => return say("No agents are running.".into()),
            Routed::Ask(options) => {
                // held with its files, so they go along when he says who it was for
                app.inner.lock().unwrap().call(radio).asked = Some((parsed.rest.clone(), files.clone(), Instant::now()));
                log(format!("← radio: \"{text}\" → unsure; asking between {}", options.join(", ")));
                return Reply { ask: options.clone(), ..ask(format!("Who is that for? {}.", options.join(", or "))) };
            }
        }
    };
    // A poke sets going again everyone who has stopped without finishing or asking: the ones he named, or in the
    // room he is looking at, or all of them. From here on it is an ordinary message to those agents.
    let mut shown = label.map(String::from);
    if parsed.control == Some(Control::Poke) {
        let room = scope.and_then(|s| s.strip_prefix("group:"));
        let stopped = app.inner.lock().unwrap().stalled();
        targets = stopped.into_iter().filter(|s| if named.is_empty() { room.is_none_or(|g| s.callsign.project == g) } else { named.iter().any(|n| n.agent.id == s.agent.id) }).collect();
        if targets.is_empty() {
            return say("Nobody has stopped.".into());
        }
        parsed.control = None;
        // shown as it is sent: there is nothing more behind it to open
        parsed.rest = "Keep going.".into();
        shown = None;
    }
    log(format!("← radio: \"{text}\" → {} {:?} \"{}\"", if targets.is_empty() { "nobody".into() } else { list(&targets.iter().map(|s| s.callsign.text()).collect::<Vec<_>>()) }, parsed.control, parsed.rest));

    match parsed.control {
        Some(Control::Status) => return say(roster),
        Some(Control::Skip) => return say(String::new()),
        Some(Control::Hold) => {
            app.inner.lock().unwrap().call(radio).held = true;
            return say("Standing by.".into());
        }
        Some(Control::Resume) => {
            app.inner.lock().unwrap().call(radio).held = false;
            app.kick.notify_one();
            return say(if pending > 0 { format!("Go ahead. {pending} waiting.") } else { "Go ahead. Nothing waiting.".into() });
        }
        Some(c @ (Control::More | Control::Repeat)) => {
            let Some((s, last)) = targets.first().and_then(|s| s.last.clone().map(|l| (s.clone(), l))) else { return say("Nothing to repeat yet.".into()) };
            let body = if c == Control::Repeat { last.say.clone().unwrap_or_default() } else { detail_of(app, &last.id).await.unwrap_or_default() };
            app.inner.lock().unwrap().call(radio).focus = Some(s.agent.id.clone());
            return ask(format!("{}. {body}", s.callsign.spoken()));
        }
        // a poke became an ordinary message above
        Some(Control::Poke) | None => {}
    }
    if parsed.rest.is_empty() {
        return match named.as_slice() {
            [one] => ask(format!("{}, go ahead.", one.callsign.spoken())),
            _ => say(String::new()),
        };
    }
    if targets.is_empty() {
        return if names.is_empty() { say("No agents are running.".into()) } else { ask(format!("Who is that for? {}.", names.join(", "))) };
    }
    // His words as he said them, with one line in front that says who they are from and how to answer.
    let many = if targets.len() > 1 { format!(" Sent to {} agents.", targets.len()) } else { String::new() };
    let content = format!("From {name}, on his phone through Airband.{many}\n\n{rest}{files}", name = app.cfg.name, rest = parsed.rest);
    // The line in the chat takes the id the radio gave what he said, when it gave one and it is not in use: the
    // radio can then tell that the message it is still showing as on its way has arrived, and swap one for the
    // other without a gap.
    let message_id = uid.map(str::to_string).filter(|id| !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_alphanumeric()) && !app.inner.lock().unwrap().log.iter().any(|e| e.id == *id)).unwrap_or_else(|| config::random_hex(8));
    // An answer to a question the agent has since spoken past (or he has since said other things to it) would
    // reach it with nothing to say what it answers: "Yes", to what? Then the question goes with it. When the
    // question is the last thing between them, the answer needs no help.
    let answers: Option<(String, String)> = re.and_then(|id| {
        let inner = app.inner.lock().unwrap();
        let at = inner.log.iter().position(|e| e.id == id)?;
        let asked = &inner.log[at];
        let agent = asked.agent.clone()?;
        let since = inner.log[at + 1..].iter().any(|e| e.agent.as_deref() == Some(agent.as_str()) && e.kind != Some(super::Kind::Fyi) || e.to_agents.iter().any(|t| *t == agent));
        since.then(|| (agent, asked.short.clone().unwrap_or_else(|| asked.full.clone())))
    });
    let answering = |agent: &str| answers.as_ref().filter(|(a, _)| a == agent).map(|(_, q)| format!("This answers what you asked earlier (as he was shown it): \"{q}\"\n\n")).unwrap_or_default();
    let (mut sent, mut failed) = (Vec::new(), Vec::new());
    let mut queued: Vec<String> = Vec::new();
    for t in &targets {
        let about = answering(&t.agent.id);
        let content = if about.is_empty() { content.clone() } else { content.replacen("\n\n", &format!("\n\n{about}"), 1) };
        let outcome = match &t.agent.deliver {
            // One the tower runs itself gets his words as his own turn, with nothing in front. If it is in the
            // middle of something they wait here, where he can still take them back, until it stops.
            Some(Deliver::Held) => {
                let words = format!("{about}{}{files}", parsed.rest);
                let busy = {
                    let mut inner = app.inner.lock().unwrap();
                    match inner.session(&t.agent.id).filter(|s| !s.waiting) {
                        Some(s) => {
                            s.queue.push((message_id.clone(), words.clone()));
                            true
                        }
                        None => false,
                    }
                };
                if busy {
                    queued.push(t.agent.id.clone());
                    Ok(())
                } else {
                    deliver(app, &Deliver::Held, &t.agent.id, &words).await
                }
            }
            Some(how) => deliver(app, how, &t.agent.id, &content).await,
            None => Err("its harness gave no way to reach it".into()),
        };
        app.store.delivery(&message_id, uid, &t.callsign.text(), &outcome);
        match outcome {
            Ok(()) => sent.push(t.clone()),
            Err(e) => {
                log(format!("could not reach {}: {e}", t.callsign.text()));
                failed.push(t.callsign.text());
            }
        }
    }
    let mut inner = app.inner.lock().unwrap();
    for t in &sent {
        let id = t.agent.id.clone();
        if let Some(s) = inner.session(&id) {
            s.waiting = false;
        }
        inner.drop_pending(&id, None);
    }
    if let [one] = sent.as_slice() {
        inner.call(radio).focus = Some(one.agent.id.clone());
    }
    if !sent.is_empty() {
        // a quick message shows as the button he tapped; the whole instruction is still what was sent and kept
        // what he sees of his own message is his words, without the list of file paths that the agent got
        let entry = Entry::his(message_id, &sent, parsed.rest.clone());
        inner.record(Entry { short: shown.or(entry.short.clone()), pictures: shown_files, queued: !queued.is_empty(), ..entry });
    }
    app.save(&inner);
    let sent_names: Vec<String> = sent.iter().map(|s| s.callsign.text()).collect();
    say(match (sent_names.len(), failed.len()) {
        (0, _) => format!("Could not reach {}.", list(&failed)),
        (n, 0) if n > 3 => format!("Sent to all {n}."),
        (_, 0) => format!("Sent to {}.", list(&sent_names)),
        _ => format!("Sent to {}. Could not reach {}.", list(&sent_names), list(&failed)),
    })
}
