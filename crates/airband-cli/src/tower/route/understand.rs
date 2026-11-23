//! Who an untagged reply is for, or that it asks for a new agent. What is certain without reading comes first (the
//! message he tapped). After that a model reads his words: against what each agent has been saying, and against the
//! projects, models and efforts a new agent could be started with.
#![expect(clippy::disallowed_macros, reason = "the model's answer shapes")]
use super::*;

/// The room as the model is shown it: who has spoken since he last did, who he last spoke with, and the rest.
struct Scene {
    /// Every agent that has said something since he last spoke, newest first, with what it said.
    spoken: Vec<(Session, Entry)>,
    /// The agent he last spoke with, and the last thing he said, when he said something.
    focus: Option<(Session, Option<Spoke>)>,
    /// Every agent he could mean.
    all: Vec<Session>,
    /// The rest: agents that have not spoken since, each with the last thing it said, if any.
    quiet: Vec<(Session, Option<Entry>)>,
}

/// The last thing he said: when, to whom, and its opening.
struct Spoke {
    at: u64,
    to: Vec<String>,
    said: String,
}

/// Everyone he could mean, in the order the model is shown them. Each is listed under a number, and the model
/// answers with numbers: nothing it writes is matched against a name.
enum Listed<'a> {
    Here(&'a Session),
    Elsewhere(&'a airband_core::radio::Other),
}

/// What a new agent could be started with, across every tool this computer can start: which tool he means is
/// settled after.
struct Menus {
    tools: Vec<String>,
    models: Listing,
    efforts: Listing,
}

/// One menu across every launcher: each launcher's own choices, each listed apart under its own number and by
/// its name alone. Two launchers' choices are never one because their names are alike: the model answers with
/// every number that is the one he named, and which launcher starts the agent is a rule applied after
/// (`launch::starter`). No place on this list decides it.
#[derive(Default)]
struct Listing {
    /// Each choice as the routing model is shown it: its name as offered.
    names: Vec<String>,
    /// Each choice, in the same order: the launcher that offers it, with that launcher's own id for it.
    offered: Vec<launch::Offered>,
}
impl Listing {
    fn of(menus: &[(LauncherId, airband_core::tools::Offers)], of: fn(&airband_core::tools::Offers) -> &Vec<airband_core::tools::Choice>) -> Listing {
        let offered: Vec<launch::Offered> = menus.iter().flat_map(|(launcher, offers)| of(offers).iter().map(|c| launch::Offered { launcher: launcher.clone(), choice: c.clone() })).collect();
        Listing { names: offered.iter().map(|o| o.choice.name.clone()).collect(), offered }
    }
    /// A choice's name, for an example in the question.
    fn first_name(&self) -> Option<&str> {
        self.names.first().map(String::as_str)
    }
    fn last_name(&self) -> Option<&str> {
        self.names.last().map(String::as_str)
    }
    /// What the model's numbers for these come to: every launcher's offer it said is the one he named.
    fn picked(&self, numbers: &[usize]) -> Option<Pick> {
        let mut offered: Vec<launch::Offered> = Vec::new();
        for n in numbers {
            if let Some(o) = judge::listed(Some(*n), &self.offered)
                && !offered.contains(o)
            {
                offered.push(o.clone());
            }
        }
        (!offered.is_empty()).then_some(Pick::Heard(launch::Heard { offered }))
    }
}

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
    model: Vec<usize>,
    #[serde(default)]
    model_said: Option<String>,
    #[serde(default)]
    effort: Vec<usize>,
    #[serde(default)]
    effort_said: Option<String>,
    #[serde(default)]
    task: String,
}

/// The model's answer.
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

/// The shape of [`Out`], as the model is held to it.
fn out_shape() -> Value {
    let number = json!({ "type": ["integer", "null"] });
    let numbers = json!({ "type": "array", "items": { "type": "integer" } });
    let word = json!({ "type": ["string", "null"] });
    let new = json!({
        "type": "object",
        "properties": {
            "tool": number, "tool_said": word, "project": number, "project_said": word,
            "model": numbers, "model_said": word, "effort": numbers, "effort_said": word,
            "task": { "type": "string" }
        },
        "required": ["tool", "tool_said", "project", "project_said", "model", "model_said", "effort", "effort_said", "task"],
        "additionalProperties": false
    });
    json!({
        "type": "object",
        "properties": { "to": { "type": "array", "items": { "type": "integer" } }, "all": { "type": "boolean" }, "sure": { "type": "boolean" }, "new": { "anyOf": [new, { "type": "null" }] } },
        "required": ["to", "all", "sure", "new"],
        "additionalProperties": false
    })
}

/// `projects` are the places a new agent can be started in, listed once for what he said.
pub(in crate::tower) async fn route(app: &Arc<App>, radio: &str, rest: &str, re: Option<&str>, scope: Option<&str>, others: &[airband_core::radio::Other], projects: &[Place]) -> Routed {
    // the room he was looking at, when he was in one: only its agents can be meant
    let room = scope.map(str::to_string);
    let scene = match scene(app, radio, re, room.as_deref()) {
        Ok(scene) => scene,
        Err(answered) => return Routed::Sure(vec![*answered]),
    };
    // what a new agent could be started in and on, if anything
    // any tool this computer can start will do for knowing that one can be: which one is settled when it starts
    let any_launcher = !app.cfg().launchers.is_empty();
    // a new agent can be started here only with a launcher and a place to start it in
    let can_start = any_launcher && !projects.is_empty();
    // with agents of another computer on his screen there is always more than one it could be for
    let more_than_these = can_start || !others.is_empty();
    match scene.all.as_slice() {
        [] if !more_than_these => return Routed::Ask(Vec::new()),
        // only one agent it could be, and nothing else it could mean
        [only] if !more_than_these => return Routed::Sure(vec![only.clone()]),
        _ => {}
    }
    // More than one agent could be meant. His words decide, read against what each has been saying: "ship the
    // login fix" goes to whoever is working on the login fix, whoever spoke last.
    let now = now_ms();
    let (listed, lines) = listing(&scene, others, now);
    let menus = menus(app);
    // the other thing his words can be: a request for a new agent, on a model and at an effort he may name
    let starting = if can_start { starting(app, &menus, projects, room.as_deref()) } else { String::new() };
    let prompt = prompt(app, &lines, &last_said(&scene.focus, now), rest, &starting, can_start);
    let Ok(judged) = judge::ask_json::<Out>(&app.settings, prompt, Some(out_shape())).await else {
        return no_answer(app, &scene.all, can_start);
    };
    answered(judged, &scene, &listed, &menus, projects, can_start)
}

/// The room as it stands, or (as `Err`) the agent he is answering when he tapped one of its messages: that is
/// certain, and it goes to the agent that said it, whatever that agent is called now. When that agent has gone,
/// his words are read against the room like any others.
fn scene(app: &App, radio: &str, re: Option<&str>, room: Option<&str>) -> Result<Scene, Box<Session>> {
    let within = |s: &Session| s.in_room(room);
    let inner = app.read();
    if let Some(target) = re.and_then(|id| inner.log.iter().find(|e| e.id == id)).and_then(|e| inner.sender(e)) {
        return Err(Box::new(target.clone()));
    }
    let since = inner.log.iter().rposition(|e| e.his).map(|i| i + 1).unwrap_or(0);
    let mut spoken: Vec<(Session, Entry)> = Vec::new();
    for e in inner.log[since..].iter().rev().filter(|e| e.kind != Some(Kind::Fyi)) {
        if let Some(s) = inner.sender(e).filter(|s| within(s))
            && !spoken.iter().any(|(have, _)| have.agent.id == s.agent.id) {
                spoken.push((s.clone(), e.clone()));
            }
    }
    let focus = inner.focused(radio).filter(|s| within(s)).cloned();
    // the last thing he said and who it went to, so "and keep the price next to it" can be read as carrying on
    let before = inner.log.iter().rev().find(|e| e.his && !e.to_agents.is_empty()).and_then(|e| {
        let to: Vec<String> = e.to_agents.iter().filter_map(|id| inner.find(id)).filter(|s| within(s)).map(|s| s.callsign.text()).collect();
        (!to.is_empty()).then(|| Spoke { at: e.at, to, said: opening(&e.full) })
    });
    let all: Vec<Session> = inner.sessions.iter().filter(|s| within(s)).cloned().collect();
    let quiet: Vec<(Session, Option<Entry>)> = all
        .iter()
        .filter(|s| !spoken.iter().any(|(have, _)| have.agent.id == s.agent.id))
        .map(|s| (s.clone(), inner.last_said(&s.agent.id).cloned()))
        .collect();
    Ok(Scene { spoken, focus: focus.map(|f| (f, before)), all, quiet })
}

/// Everyone he could mean, numbered, and a line for each: the ones that have spoken since he last did first, then
/// the rest of the room, then the agents another of his computers has.
fn listing<'a>(scene: &'a Scene, others: &'a [airband_core::radio::Other], now: u64) -> (Vec<Listed<'a>>, Vec<String>) {
    // each with the two letters the app shows for it: he may say those and not the name
    let letters = |s: &Session| s.callsign.badge();
    let line = |n: usize, s: &Session, e: &Entry, since: bool| {
        format!("{n}. {} (letters {}; {} ago{}{}): {}", s.callsign.text(), letters(s), ago(now.saturating_sub(e.at)), if since { "" } else { ", before he last spoke" }, if e.kind == Some(Kind::NeedsYou) { ", waiting on him" } else { "" }, e.line())
    };
    // past the first dozen an agent is listed by its name alone: he can still name it
    let bare = |n: usize, s: &Session| format!("{n}. {} (letters {})", s.callsign.text(), letters(s));
    let mut listed: Vec<Listed> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for (i, (s, e)) in scene.spoken.iter().enumerate() {
        listed.push(Listed::Here(s));
        lines.push(if i < 6 { line(listed.len(), s, e, true) } else { bare(listed.len(), s) });
    }
    let room_left = 12usize.saturating_sub(scene.spoken.len().min(6));
    for (i, (s, e)) in scene.quiet.iter().enumerate() {
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
        let (n, letters) = (listed.len(), o.badge.clone());
        lines.push(if o.said.is_empty() { format!("{n}. {} (letters {letters}; on another of his computers; has said nothing yet)", o.callsign) } else { format!("{n}. {} (letters {letters}; on another of his computers; {} ago): {}", o.callsign, ago(o.ago * 1000), o.said) });
    }
    (listed, lines)
}

/// What he last said and to whom, or who he last spoke with, for reading his words as carrying on.
fn last_said(focus: &Option<(Session, Option<Spoke>)>, now: u64) -> String {
    match focus {
        Some((_, Some(Spoke { at, to, said }))) => format!("The last thing he said, {} ago, was to {}: \"{said}\"\n", ago(now.saturating_sub(*at)), list(to)),
        Some((s, None)) => format!("The agent he last spoke with is {}.\n", s.callsign.text()),
        None => String::new(),
    }
}

/// Every tool, model and effort a new agent could be started with.
fn menus(app: &App) -> Menus {
    let inner = app.read();
    // in the settings' own order, which decides nothing: the model names what he heard, and which launcher
    // starts it is `launch::starter`'s rule
    let menus: Vec<(LauncherId, airband_core::tools::Offers)> = app.cfg().launchers.iter().map(|(name, l)| (LauncherId::new(name.as_str()), held::offers(&inner, name, l))).collect();
    Menus { tools: app.cfg().tools(), models: Listing::of(&menus, |m| &m.models), efforts: Listing::of(&menus, |m| &m.efforts) }
}

/// A numbered list on one line, for the model to answer with a number from.
fn numbered(names: &[String]) -> String {
    names.iter().enumerate().map(|(i, n)| format!("{}. {n}", i + 1)).collect::<Vec<_>>().join(", ")
}

/// The part of the question about starting a new agent: when his words ask for one, and what it is started with.
/// Each list is numbered, and the model answers with a number from it, or with his own word for something he
/// named that is on none of them.
fn starting(app: &App, menus: &Menus, projects: &[Place], room: Option<&str>) -> String {
    let lately: Vec<String> = {
        let inner = app.read();
        projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let said: Vec<String> = inner.lately(&p.folder);
                format!("{}. {}{}", i + 1, p.name, if said.is_empty() { String::new() } else { format!(": lately, {}", said.join(" / ")) })
            })
            .collect()
    };
    let (tools, models, efforts) = (&menus.tools, &menus.models.names, &menus.efforts.names);
    // the example is written with a tool he has, by the name its row gives it
    let tool = match tools.last().filter(|_| tools.len() > 1) {
        Some(eg) => format!("\"tool\" is the number of the coding tool he asked for by name (\"start a {eg} agent\", \"have {eg} look into…\"), of {}; else null. \"tool_said\" is his own word for a tool he named that is not listed, else null. ", numbered(tools)),
        None => "\"tool\" is null and so is \"tool_said\". ".to_string(),
    };
    // what he named is passed on as he named it: one that cannot be had is refused in words, never quietly
    // swapped for another
    let offered = |what: &str, names: &[String]| {
        if names.is_empty() {
            format!("\"{what}\" is empty; \"{what}_said\" is his own word for the {what} when he named one, else null. ")
        } else {
            format!("\"{what}\" lists the number of the {what} he named, of {}; several are listed with one name or alike, one for each way of starting an agent: give the number of every one that is the {what} he named; empty when he named none or one that is not listed. \"{what}_said\" is his own word for it when he named one that is not listed, else null. ", numbered(names))
        }
    };
    // the examples are written with a model and an effort he can ask for, by the names their rows give
    let by_model = match (menus.models.first_name(), menus.efforts.last_name()) {
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
        room = room.and_then(|r| app.read().team_name(&TeamId::new(r)).map(|n| format!(" (he is looking at {n})"))).unwrap_or_default(),
        tool = tool,
        by_model = by_model,
        models = offered("model", models),
        efforts = offered("effort", efforts),
        projects = lately.join("\n")
    )
}

/// The whole question put to the model.
fn prompt(app: &App, lines: &[String], last: &str, rest: &str, starting: &str, can_start: bool) -> String {
    format!(
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
        name = app.cfg().name,
        lines = if lines.is_empty() { "(none are running)".to_string() } else { lines.join("\n") },
        starting = starting,
        shape = if can_start { ",\"new\":null or {\"tool\":…,\"tool_said\":…,\"project\":…,\"project_said\":…,\"model\":…,\"model_said\":…,\"effort\":…,\"effort_said\":…,\"task\":\"…\"}" } else { "" }
    )
}

/// No model answered. Nothing is read into his words. With one agent there is only one thing they can be for,
/// unless words could also have started a new one: that takes a model to read them, so it is so only where one is
/// set up to read and did not answer. Then he is asked.
fn no_answer(app: &App, all: &[Session], can_start: bool) -> Routed {
    let could_be_new = can_start && !app.settings.reader().is_none();
    match all {
        [] => Routed::Ask(Vec::new()),
        [only] if !could_be_new => Routed::Sure(vec![only.clone()]),
        several => Routed::Ask(several.iter().take(3).cloned().collect()),
    }
}

/// What the model's numbers come to.
fn answered(mut judged: Out, scene: &Scene, listed: &[Listed], menus: &Menus, projects: &[Place], can_start: bool) -> Routed {
    if let Some(new) = judged.new.take().filter(|_| can_start) {
        // The model answers with the number a thing is listed under, and that is the thing. Something he named
        // that is on no list comes as his own word, kept as he said it so that starting it is refused in words
        // (`launch`): nothing is started on a guess at what he meant.
        let at = |n: Option<usize>, names: &[String]| judge::listed(n, names).cloned();
        let own = |said: Option<String>| said.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
        return Routed::New(Idea {
            tool: at(new.tool, &menus.tools).or(own(new.tool_said)),
            // one he named that the tower does not know is not swapped for another: he is asked which. He named
            // none, and there is one place it could be: that one.
            project: judge::listed(new.project, projects).cloned().or_else(|| (own(new.project_said).is_none() && projects.len() == 1).then(|| projects[0].clone())),
            model: menus.models.picked(&new.model).or(own(new.model_said).map(Pick::Said)),
            effort: menus.efforts.picked(&new.effort).or(own(new.effort_said).map(Pick::Said)),
            // with no work named it is started with nothing to do yet, or he is asked what it should do
            task: new.task.trim().to_string(),
        });
    }
    if scene.all.is_empty() {
        return Routed::Ask(Vec::new());
    }
    // who the numbers are
    let meant: Vec<&Listed> = judged.to.iter().filter_map(|n| judge::listed(Some(*n), listed)).collect();
    let here: Vec<Session> = meant.iter().filter_map(|l| if let Listed::Here(s) = l { Some((*s).clone()) } else { None }).collect();
    // one of the agents on another computer, and none here: the radio takes it there
    if let Some(o) = meant.iter().find_map(|l| if let Listed::Elsewhere(o) = l { Some(*o) } else { None }).filter(|_| judged.sure && here.is_empty()) {
        return Routed::Elsewhere(o.clone());
    }
    // all of them, said outright
    if judged.sure && judged.all {
        return Routed::Sure(scene.all.clone());
    }
    if judged.sure && !here.is_empty() {
        return Routed::Sure(here);
    }
    // the likeliest first, so "the first one" or a bare callsign settles it
    let everyone = scene.spoken.iter().map(|(s, _)| s).chain(scene.quiet.iter().map(|(s, _)| s));
    let mut likely: Vec<Session> = here;
    everyone.take(3).for_each(|s| if !likely.iter().any(|l| l.agent.id == s.agent.id) { likely.push(s.clone()) });
    Routed::Ask(likely)
}

/// He was asked who something he said was for, and has said something more. The agents his words name, or
/// nothing when they are not an answer to that: then they are a message of their own. `Err` when no model
/// could say.
pub(super) async fn named_agents(app: &Arc<App>, first: &str, said: &str, scope: Option<&str>) -> Result<Option<Vec<Session>>, String> {
    let all: Vec<Session> = app.read().sessions.iter().filter(|s| s.in_room(scope)).cloned().collect();
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
        name = app.cfg().name,
        list = all.iter().enumerate().map(|(i, s)| format!("{}. {} (letters {})", i + 1, s.callsign.text(), letters(s))).collect::<Vec<_>>().join("\n"),
    );
    #[derive(Deserialize)]
    struct Out {
        #[serde(default)]
        to: Vec<usize>,
        #[serde(default)]
        all: bool,
    }
    let shape = json!({
        "type": "object",
        "properties": { "to": { "type": "array", "items": { "type": "integer" } }, "all": { "type": "boolean" } },
        "required": ["to", "all"],
        "additionalProperties": false
    });
    let judged: Out = judge::ask_json(&app.settings, prompt, Some(shape)).await?;
    let meant: Vec<Session> = if judged.all { all } else { judged.to.iter().filter_map(|n| judge::listed(Some(*n), &all).cloned()).collect() };
    Ok((!meant.is_empty()).then_some(meant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::tools::{Choice, Offers};

    #[tokio::test]
    async fn the_menus_are_read_without_holding_the_state_twice() {
        // reading the menus takes the state once: taken twice while held, it never returned
        let app = crate::tower::http::tests::app("menus-once");
        let done = std::thread::spawn(move || drop(menus(&app)));
        for _ in 0..100 {
            if done.is_finished() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("reading the menus did not finish");
    }

    #[test]
    fn each_launchers_choices_are_listed_apart_and_never_merged_by_a_name_alike() {
        let offers = |models: &[(&str, &str)]| Offers { models: models.iter().map(|(id, name)| Choice { id: id.to_string(), name: name.to_string() }).collect(), ..Offers::default() };
        let (a, b) = (LauncherId::new("codex"), LauncherId::new("held-codex"));
        let menus = vec![(a.clone(), offers(&[("gpt-6.2-sol", "6.2 Sol"), ("gpt-6-luna", "6 Luna")])), (b.clone(), offers(&[("sol-2", "6.2 Sol")]))];
        let listing = Listing::of(&menus, |m| &m.models);
        // each launcher's own, under a number of its own, and no launcher named to the model
        assert_eq!(listing.names, ["6.2 Sol", "6 Luna", "6.2 Sol"]);
        let offer = |l: &LauncherId, id: &str, name: &str| launch::Offered { launcher: l.clone(), choice: Choice { id: id.into(), name: name.into() } };
        let heard = |offered: Vec<launch::Offered>| Some(Pick::Heard(launch::Heard { offered }));
        // one number is one launcher's offer, with that launcher's id alone
        assert_eq!(listing.picked(&[1]), heard(vec![offer(&a, "gpt-6.2-sol", "6.2 Sol")]));
        assert_eq!(listing.picked(&[3]), heard(vec![offer(&b, "sol-2", "6.2 Sol")]));
        // every number the model gave for what he said, each once
        assert_eq!(listing.picked(&[3, 1, 3]), heard(vec![offer(&b, "sol-2", "6.2 Sol"), offer(&a, "gpt-6.2-sol", "6.2 Sol")]));
        assert_eq!(listing.picked(&[4]), None);
        assert_eq!(listing.picked(&[]), None);
    }
}
