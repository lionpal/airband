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
    let l = airband_core::routing::parse_launch(text, projects)?;
    // the models and efforts that can be named are the ones of the tool the command names, or of the usual one
    let menu = held::launcher_for(app, l.tool.as_deref()).map(|(name, l)| held::offers(&app.inner.lock().unwrap(), &name, &l)).unwrap_or_default();
    let (models, efforts) = (menu.model_words(), menu.effort_words());
    Some(match airband_core::routing::parse_with(&l.task, &models, &efforts).filter(|w| !w.rest.is_empty()) {
        Some(w) => Idea { tool: l.tool, project: l.project, model: Some(w.model), effort: w.effort, task: w.rest },
        None => Idea { tool: l.tool, project: l.project, model: None, effort: None, task: l.task },
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
            if let Some(s) = inner.sender(e).filter(|s| within(s))
                && !seen.iter().any(|(have, _)| have.agent.id == s.agent.id) {
                    seen.push((s.clone(), e.clone()));
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
    // any tool this computer can start will do for knowing that one can be: which one is settled when it starts
    let launcher = app.cfg.launchers.values().next();
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
            let now = now_ms();
            // each with the two letters the app shows for it: he may say those and not the name
            let letters = |s: &Session| airband_core::routing::badge(&s.callsign.project, &s.callsign.text());
            // Everyone he could mean, in the order the model is shown them. Each is listed under a number, and
            // the model answers with numbers: nothing it writes is matched against a name.
            enum Listed<'a> {
                Here(&'a Session),
                Elsewhere(&'a airband_core::radio::Other),
            }
            let line = |n: usize, s: &Session, e: &Entry, since: bool| {
                format!("{n}. {} (letters {}; {} ago{}{}): {}", s.callsign.text(), letters(s), ago(now.saturating_sub(e.at)), if since { "" } else { ", before he last spoke" }, if e.kind == Some(Kind::NeedsYou) { ", waiting on him" } else { "" }, e.short.clone().unwrap_or_else(|| opening(&e.full)))
            };
            // past the first dozen an agent is listed by its name alone: he can still name it
            let bare = |n: usize, s: &Session| format!("{n}. {} (letters {})", s.callsign.text(), letters(s));
            let mut listed: Vec<Listed> = Vec::new();
            let mut lines: Vec<String> = Vec::new();
            for (i, (s, e)) in several.iter().enumerate() {
                listed.push(Listed::Here(s));
                lines.push(if i < 6 { line(listed.len(), s, e, true) } else { bare(listed.len(), s) });
            }
            let room_left = 12usize.saturating_sub(several.len().min(6));
            for (i, (s, e)) in quiet.iter().enumerate() {
                listed.push(Listed::Here(s));
                lines.push(match e {
                    Some(e) if i < room_left => line(listed.len(), s, e, false),
                    None if i < room_left => format!("{}. {} (letters {}; has said nothing yet)", listed.len(), s.callsign.text(), letters(s)),
                    _ => bare(listed.len(), s),
                });
            }
            // the agents he can see that another of his computers has: his words may be for one of them
            for o in others.iter().take(8) {
                listed.push(Listed::Elsewhere(o));
                let (n, letters) = (listed.len(), airband_core::routing::badge(&o.group, &o.callsign));
                lines.push(if o.said.is_empty() { format!("{n}. {} (letters {letters}; on another of his computers; has said nothing yet)", o.callsign) } else { format!("{n}. {} (letters {letters}; on another of his computers; {} ago): {}", o.callsign, ago(o.ago * 1000), o.said) });
            }
            let last = match &focus {
                Some((_, Some((at, to, said)))) => format!("The last thing he said, {} ago, was to {}: \"{said}\"\n", ago(now.saturating_sub(*at)), list(to)),
                Some((s, None)) => format!("The agent he last spoke with is {}.\n", s.callsign.text()),
                None => String::new(),
            };
            // What a new agent could be started with. Each list is numbered, and the model answers with a number
            // from it, or with his own word for something he named that is on none of them.
            let numbered = |names: &[String]| names.iter().enumerate().map(|(i, n)| format!("{}. {n}", i + 1)).collect::<Vec<_>>().join(", ");
            let tools = app.cfg.tools();
            let (models, efforts): (Vec<String>, Vec<String>) = {
                // every model and effort any of the tools offers: which tool he means is settled after
                let inner = app.inner.lock().unwrap();
                let menus: Vec<airband_core::tools::Offers> = app.cfg.launchers.iter().map(|(name, l)| held::offers(&inner, name, l)).collect();
                let across = |of: fn(&airband_core::tools::Offers) -> &Vec<airband_core::tools::Choice>| {
                    let mut all: Vec<String> = Vec::new();
                    menus.iter().flat_map(of).for_each(|c| if !all.contains(&c.name) { all.push(c.name.clone()) });
                    all
                };
                (across(|m| &m.models), across(|m| &m.efforts))
            };
            // the other thing his words can be: a request for a new agent, on a model and at an effort he may name
            let starting = match launcher {
                None => String::new(),
                Some(_) => {
                    let lately: Vec<String> = {
                        let inner = app.inner.lock().unwrap();
                        projects
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                let said: Vec<String> = inner.log.iter().rev().filter(|e| e.group == *p && e.from != "you" && e.kind != Some(Kind::Fyi)).take(2).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
                                format!("{}. {p}{}", i + 1, if said.is_empty() { String::new() } else { format!(": lately, {}", said.join(" / ")) })
                            })
                            .collect()
                    };
                    // the example is written with a tool he has, by the name its row gives it
                    let tool = match tools.last().filter(|_| tools.len() > 1) {
                        Some(eg) => format!("\"tool\" is the number of the coding tool he asked for by name (\"start a {eg} agent\", \"have {eg} look into…\"), of {}; else null. \"tool_said\" is his own word for a tool he named that is not listed, else null. ", numbered(&tools)),
                        None => "\"tool\" is null and so is \"tool_said\". ".to_string(),
                    };
                    // what he named is passed on as he named it: one that cannot be had is refused in words,
                    // never quietly swapped for another
                    let offered = |what: &str, names: &[String]| {
                        if names.is_empty() {
                            format!("\"{what}\" is null; \"{what}_said\" is his own word for the {what} when he named one, else null. ")
                        } else {
                            format!("\"{what}\" is the number of the {what} he named, of {}; null when he named none or one that is not listed. \"{what}_said\" is his own word for it when he named one that is not listed, else null. ", numbered(names))
                        }
                    };
                    // the examples are written with a model and an effort he can ask for, by the names their rows give
                    let by_model = match (models.first(), efforts.last()) {
                        (Some(m), Some(e)) => format!(
                            ", or he asks for work to be done by a model he names (\"with {m}, look into…\", \"with {m} {e}, investigate…\", \"have {m} investigate…\", \"get {m} on {e} to review…\"). \
                             A remark about a model (\"with {m} it was too slow\") is not a request for one"
                        ),
                        (Some(m), None) => format!(", or he asks for work to be done by a model he names (\"with {m}, look into…\", \"have {m} investigate…\"). A remark about a model (\"with {m} it was too slow\") is not a request for one"),
                        (None, _) => String::new(),
                    };
                    format!(
                        "\n\nOr he is not talking to any of them: he wants a NEW agent started. Decide this first. He says so \
                         outright (\"new idea: …\", \"start an agent to…\", \"launch…\", \"spin up…\": words like these always \
                         mean a new agent, whatever the work is and whoever is already working on something like it){by_model}. Work he simply asks \
                         for, with no such sign, is for an agent already running. When he wants a new agent, \"to\" is empty and \"new\" says \
                         what to start: \"project\" is the number of the one he named, else of the one the work would be done in{room}, going by \
                         who the work is for and what it changes more than by what data it happens to read (a fair \
                         choice beats asking him: he is told where it started), else null when nothing points to \
                         any of them or he named one that is not listed (then \"project_said\" is his own word for it, else null); {tool}{models}{efforts}\"task\" is the work in his own words, without the words \
                         about starting an agent, the tool, the model or the effort. Otherwise \"new\" is null. The projects:\n{projects}",
                        room = room.as_ref().map(|r| format!(" (he is looking at {r})")).unwrap_or_default(),
                        tool = tool,
                        by_model = by_model,
                        models = offered("model", &models),
                        efforts = offered("effort", &efforts),
                        projects = lately.join("\n")
                    )
                }
            };
            let prompt = format!(
                "{name} runs several coding agents. Each is listed under a number, by its callsign, with the last thing it said, the ones that have spoken since he last did first, newest first:\n{lines}\n{last}\n\
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
                 pull main\") is each of that team's agents, and every agent there is (\"everyone, stop\", \"all of you, \
                 commit what you have\") is \"all\":true. A sentence that only mentions them all (\"everyone is waiting on \
                 the deploy\") addresses whoever its words are for, not all of them.{starting}\n\
                 Reply with one line of JSON and nothing else: {{\"to\":[the number each agent it is for is listed under, at the start of its line],\"all\":true or false,\"sure\":true or false{shape}}}\n\
                 \"sure\": false if a careful assistant would check with him before passing this on.",
                name = app.cfg.name,
                lines = if lines.is_empty() { "(none are running)".to_string() } else { lines.join("\n") },
                starting = starting,
                shape = if can_start { ",\"new\":null or {\"tool\":…,\"tool_said\":…,\"project\":…,\"project_said\":…,\"model\":…,\"model_said\":…,\"effort\":…,\"effort_said\":…,\"task\":\"…\"}" } else { "" }
            );
            let everyone: Vec<&Session> = several.iter().map(|(s, _)| s).chain(quiet.iter().map(|(s, _)| s)).collect();
            #[derive(Deserialize)]
            struct New {
                #[serde(default)]
                tool: Option<usize>,
                #[serde(default)]
                tool_said: Option<String>,
                #[serde(default)]
                project: Option<usize>,
                #[serde(default)]
                project_said: Option<String>,
                #[serde(default)]
                model: Option<usize>,
                #[serde(default)]
                model_said: Option<String>,
                #[serde(default)]
                effort: Option<usize>,
                #[serde(default)]
                effort_said: Option<String>,
                #[serde(default)]
                task: String,
            }
            #[derive(Deserialize)]
            struct Out {
                #[serde(default)]
                to: Vec<usize>,
                #[serde(default)]
                all: bool,
                #[serde(default)]
                sure: bool,
                #[serde(default)]
                new: Option<New>,
            }
            let judged = judge::ask_json::<Out>(&app.cfg, "routing", prompt, None).await.ok();
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
                // The model answers with the number a thing is listed under, and that is the thing. Something
                // he named that is on no list comes as his own word, kept as he said it so that starting it is
                // refused in words (`launch`): nothing is started on a guess at what he meant.
                let at = |n: Option<usize>, names: &[String]| judge::listed(n, names).cloned();
                let own = |said: Option<String>| said.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
                return Routed::New(Idea {
                    tool: at(new.tool, &tools).or(own(new.tool_said)),
                    // one he named that the tower does not know is not swapped for another: he is asked which.
                    // He named none, and there is one place it could be: that one.
                    project: at(new.project, &projects).or_else(|| (own(new.project_said).is_none() && projects.len() == 1).then(|| projects[0].clone())),
                    model: at(new.model, &models).or(own(new.model_said)),
                    effort: at(new.effort, &efforts).or(own(new.effort_said)),
                    // with no work named it is started with nothing to do yet, or he is asked what it should do
                    task: new.task.trim().to_string(),
                });
            }
            if all.is_empty() {
                return Routed::Ask(Vec::new());
            }
            // who the numbers are
            let meant: Vec<&Listed> = judged.to.iter().filter_map(|n| judge::listed(Some(*n), &listed)).collect();
            let here: Vec<Session> = meant.iter().filter_map(|l| if let Listed::Here(s) = l { Some((*s).clone()) } else { None }).collect();
            // one of the agents on another computer, and none here: the radio takes it there
            if let Some(o) = meant.iter().find_map(|l| if let Listed::Elsewhere(o) = l { Some(*o) } else { None }).filter(|_| judged.sure && here.is_empty()) {
                return Routed::Elsewhere(o.callsign.clone());
            }
            // all of them, said outright
            if judged.sure && judged.all {
                return Routed::Sure(all.clone());
            }
            if judged.sure && !here.is_empty() {
                Routed::Sure(here)
            } else {
                // the likeliest first, so "the first one" or a bare callsign settles it
                let mut names: Vec<String> = here.iter().map(|s| s.callsign.text()).collect();
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

/// A model's answer that picks one project from a numbered list, or none.
#[derive(Deserialize)]
struct Picked {
    project: Option<usize>,
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
            .enumerate()
            .map(|(i, p)| {
                let lately: Vec<String> = inner.log.iter().rev().filter(|e| e.group == *p && e.from != "you" && e.kind != Some(Kind::Fyi)).take(2).map(|e| e.short.clone().unwrap_or_else(|| opening(&e.full))).collect();
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
async fn named_project(app: &Arc<App>, idea: &str, said: &str, projects: &[String]) -> Option<String> {
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
async fn named_agents(app: &Arc<App>, first: &str, said: &str, scope: Option<&str>) -> Option<Vec<Session>> {
    let room = scope.and_then(|s| s.strip_prefix("group:"));
    let all: Vec<Session> = app.inner.lock().unwrap().sessions.iter().filter(|s| room.is_none_or(|g| s.callsign.project == g)).cloned().collect();
    let letters = |s: &Session| airband_core::routing::badge(&s.callsign.project, &s.callsign.text());
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
pub(super) async fn on_utterance(app: &Arc<App>, radio: &str, text: &str, said: &airband_core::radio::Utterance) -> Reply {
    let (uid, re, scope, label, attached, others) = (Some(said.id.as_str()), said.re.as_deref(), said.scope.as_deref(), said.label.as_deref(), said.attachments.as_slice(), said.others.as_slice());
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
        if !text.trim_start().starts_with(['/', '@'])
            && let Some(project) = named_project(app, &idea.task, text, &projects).await {
                log(format!("← radio: \"{text}\" → launch in {project}"));
                return launch(app, &project, &idea).await;
            }
        app.inner.lock().unwrap().call(radio).unplaced_idea = Some((idea, when));
    }
    // a typed command says what it is and needs no reading
    if text.trim_start().starts_with('/')
        && let Some(mut idea) = new_by_command(app, text, &projects) {
            idea.task.push_str(&files);
            return start(app, radio, text, idea, &projects, true).await;
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
        (parsed, named, inner.roster(), names, inner.unheard().count(), held)
    };
    // "Who is that for?" was just asked. A tag and nothing else answers it; so do words of his that a model
    // reads as naming someone. Either way what goes out is what he said the first time.
    let mut named = named;
    if let (Some((words, their_files)), None) = (&held, parsed.control) {
        if parsed.rest.is_empty() && !named.is_empty() {
            (parsed.rest, files) = (words.clone(), their_files.clone());
        } else if named.is_empty() && !parsed.rest.is_empty()
            && let Some(meant) = named_agents(app, words, &parsed.rest, scope).await {
                named = meant;
                (parsed.rest, files) = (words.clone(), their_files.clone());
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
            let last = targets.first().and_then(|s| s.last_heard.clone().map(|l| (s.clone(), l))).and_then(|(s, id)| app.inner.lock().unwrap().log.iter().find(|e| e.id == id).map(|e| (s, id, e.short.clone().unwrap_or_default())));
            let Some((s, last, said)) = last else { return say("Nothing to repeat yet.".into()) };
            let body = if c == Control::Repeat { said } else { detail_of(app, &last).await.unwrap_or_default() };
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
        let since = inner.log[at + 1..].iter().any(|e| e.agent.as_deref() == Some(agent.as_str()) && e.kind != Some(super::Kind::Fyi) || e.to_agents.contains(&agent));
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
            // One reached through its own tool takes his words as a turn of his own too, and the tool keeps
            // them in order itself while the agent is busy.
            Some(how @ Deliver::Harness { .. }) => deliver(app, how, &t.agent.id, &format!("{about}{}{files}", parsed.rest)).await,
            // Anything else gets them from outside, as a message: one line in front says who they are from.
            Some(how) => deliver(app, how, &t.agent.id, &content).await,
            None => Err("it was started in a way that gives airband nowhere to send it messages".into()),
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
        inner.no_longer_waiting(&id);
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
    let reply = say(match (sent_names.len(), failed.len()) {
        (0, _) => format!("Could not reach {}.", list(&failed)),
        (n, 0) if n > 3 => format!("Sent to all {n}."),
        (_, 0) => format!("Sent to {}.", list(&sent_names)),
        _ => format!("Sent to {}. Could not reach {}.", list(&sent_names), list(&failed)),
    });
    // it reached everyone it was for: the words only confirm that
    Reply { done: failed.is_empty() && !sent_names.is_empty(), ..reply }
}
