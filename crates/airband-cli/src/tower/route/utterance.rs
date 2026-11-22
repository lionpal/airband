//! Something he said, from a radio: an answer to a request for leave, a new agent, a control, or words for agents.
//! Each step below either settles it and answers, or hands on to the next.
use super::*;

/// What an agent is told when he sent it files and said nothing: the tower's note, not his words.
const WORDLESS: &str = "(He sent this with no words.)";

fn say(s: String) -> Reply {
    Reply { say: s, ..Reply::default() }
}
fn ask(s: String) -> Reply {
    Reply { say: s, listen: true, ..Reply::default() }
}

/// Who his words are for, as far as that has been worked out.
struct Addressed {
    /// His words, without the tags that named who they are for.
    rest: String,
    /// Where each file that came with them is on this computer, for whoever they go to.
    files: String,
    /// The agents he named, by tag or on his screen.
    named: Vec<Session>,
    /// Whoever it goes to.
    targets: Vec<Session>,
    roster: String,
    /// Every agent's callsign, for asking him who it is for.
    names: Vec<String>,
    /// How many messages are waiting on him.
    pending: usize,
}

/// `said.id` is the utterance's own id: deliveries are recorded against it, so a retry after a crash can tell that
/// the message already reached its agent. `said.label` is the button's words when this is a quick message: the
/// chat shows those, and the agent gets `text`.
pub(in crate::tower) async fn on_utterance(app: &Arc<App>, radio: &str, text: &str, said: &airband_core::radio::Utterance) -> Reply {
    if let Some(reply) = leave(app, text, said).await {
        return reply;
    }
    // files that came with his words: where each is on this computer, for whoever the words go to, and the
    // pictures to show beside them in the chat
    let (files, shown_files) = if said.attachments.is_empty() { (String::new(), Vec::new()) } else { attachments::describe(&said.attachments).await };
    // starting a new agent comes next: it is addressed to the tower, not to anyone on the radio
    let projects: Vec<Place> = app.read().launchable();
    if let Some(reply) = new_agent(app, radio, text, said, &files, &projects).await {
        return reply;
    }
    let control = match said.act {
        Some(Act::Control { control }) => Some(control),
        _ => None,
    };
    let mut to = match addressed(app, radio, text, said, control, files, &projects).await {
        Ok(to) => to,
        Err(reply) => return reply,
    };
    // A poke sets going again everyone who has stopped without finishing or asking: the ones he named, or in the
    // room he is looking at, or all of them. From here on it is an ordinary message to those agents.
    let mut shown = said.label.clone();
    let mut control = control;
    if control == Some(Control::Poke) {
        let stopped = app.read().stalled();
        let room = said.scope.as_deref();
        to.targets = stopped.into_iter().filter(|s| if to.named.is_empty() { room.is_none_or(|g| s.team == g) } else { to.named.iter().any(|n| n.agent.id == s.agent.id) }).collect();
        if to.targets.is_empty() {
            return say("Nobody has stopped.".into());
        }
        control = None;
        // shown as it is sent: there is nothing more behind it to open
        to.rest = "Keep going.".into();
        shown = None;
    }
    log(format!("← radio: \"{text}\" → {} {:?} \"{}\"", if to.targets.is_empty() { "nobody".into() } else { list(&to.targets.iter().map(|s| s.callsign.text()).collect::<Vec<_>>()) }, control, to.rest));
    if let Some(control) = control {
        return controlled(app, radio, control, &to).await;
    }
    // files with no words: the files are the message. The agent is told so by the tower, in the tower's own
    // voice; no words are put in his mouth, and the chat shows the files alone.
    let wordless = to.rest.is_empty() && !to.files.is_empty();
    if to.rest.is_empty() && !wordless {
        return match to.named.as_slice() {
            [one] => ask(format!("{}, go ahead.", one.callsign.spoken())),
            _ => say(String::new()),
        };
    }
    if to.targets.is_empty() {
        return if to.names.is_empty() { say("No agents are running.".into()) } else { ask(format!("Who is that for? {}.", to.names.join(", "))) };
    }
    // The line in the chat takes the id the radio gave what he said, when it is not in use: the radio can then
    // tell that the message it is still showing as on its way has arrived, and swap one for the other without a gap.
    let message_id = Some(said.id.as_str()).filter(|id| !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_alphanumeric()) && !app.read().log.iter().any(|e| e.id == *id)).map(str::to_string).unwrap_or_else(|| config::random_hex(8));
    let (sent, failed, queued) = send(app, &to, wordless, said.re.as_deref(), &message_id, &said.id).await;
    kept(app, radio, &to, message_id, sent, failed, queued, shown, shown_files)
}

/// An answer to a request for permission goes back to the harness that asked, not to the agent as a message. The
/// buttons say yes or no outright. Anything else he says to a request for leave is read by a model, and only a
/// plain yes allows it; if his words do not settle it, nothing is decided. `None` when this was not about leave.
async fn leave(app: &Arc<App>, text: &str, said: &airband_core::radio::Utterance) -> Option<Reply> {
    let re = said.re.as_deref();
    let over = || say("That request is no longer waiting. Nothing was allowed.".into());
    let verdict = match permit::asked_leave(app, re) {
        permit::Leave::No => None,
        // an answer to a request that is over answers nothing, and is not passed to the agent as if it did
        permit::Leave::Over => return Some(over()),
        // a button's answer is what it is, and only a button gives one
        permit::Leave::Waiting if matches!(said.act, Some(Act::Verdict { .. })) => match said.act {
            Some(Act::Verdict { allow }) => Some(allow),
            _ => None,
        },
        // The model is given his words and nothing of the request, which the asking agent wrote.
        permit::Leave::Waiting => match judge::consent(&app.cfg, text).await {
            Some(true) => Some(true),
            // his own words, which are not a yes: they refuse it and go back to the agent as the reason
            Some(false) => None,
            None => return Some(say("That one needs a yes or a no from you.".into())),
        },
    };
    if let Some(reply) = permit::answer(app, text, re, verdict) {
        return Some(reply);
    }
    // a yes or a no that answers no request for leave answers nothing
    matches!(said.act, Some(Act::Verdict { .. })).then(over)
}

/// A new agent: the answer to "Which project?" asked about an idea just now, or one asked for from a radio's own
/// screen, which names its team by id and needs no reading. `None` when this is not about a new agent.
async fn new_agent(app: &Arc<App>, radio: &str, text: &str, said: &airband_core::radio::Utterance, files: &str, projects: &[Place]) -> Option<Reply> {
    // Words that answer the question by its id take the idea, and a model says with which project; when they
    // name none, the idea is let go. Words that answer nothing leave it held.
    let held_idea = {
        let mut inner = app.passing();
        let call = inner.call(radio);
        match (&call.unplaced_idea, &said.answers) {
            (Some((question, _)), Some(answers)) if question == answers => call.unplaced_idea.take().map(|(_, idea)| idea),
            _ => None,
        }
    };
    if let Some(idea) = held_idea
        && said.act.is_none() && said.to.is_empty()
        && let Some(place) = named_project(app, &idea.task, text, projects).await {
            log(format!("← radio: \"{text}\" → launch in {}", place.name));
            return Some(launch(app, &place, &idea).await);
        }
    let Some(Act::Launch(new)) = &said.act else { return None };
    // the team he picked, by its id, and the folder it works in
    let Some(place) = app.read().team_place(&new.team) else {
        return Some(say("That team is not on this computer any more, or its folder is gone.".into()));
    };
    let idea = Idea { tool: new.tool.clone(), project: Some(place), model: new.model.clone().map(Pick::Id), effort: new.effort.clone().map(Pick::Id), task: format!("{}{files}", new.task) };
    Some(start(app, radio, text, idea, projects).await)
}

/// Who his words are for: the agents he picked on his screen or tagged, the answer to "Who is that for?" asked
/// just now, whoever he last heard for a control, or whoever a model reads them as for. `Err` with what to answer
/// when that settles it some other way (another computer, a new agent, a question back).
async fn addressed(app: &Arc<App>, radio: &str, text: &str, said: &airband_core::radio::Utterance, control: Option<Control>, files: String, projects: &[Place]) -> Result<Addressed, Reply> {
    let (parsed, mut named, roster, names, pending, held) = {
        let mut inner = app.passing();
        inner.reap();
        // a control carries no words; what it shows in the chat is not for anyone to read
        let parsed = if control.is_some() { airband_core::routing::Parsed { to: Vec::new(), rest: String::new() } } else { parse_utterance(text, &inner.callsigns()) };
        // the agents he picked on his screen, by id; else the ones his tags name
        let named: Vec<Session> = if said.to.is_empty() { parsed.to.iter().map(|&i| inner.sessions[i].clone()).collect() } else { said.to.iter().filter_map(|id| inner.find(id).cloned()).collect() };
        if !said.to.is_empty() && named.is_empty() {
            return Err(say("That agent is no longer running.".into()));
        }
        if let [one] = named.as_slice() {
            inner.call(radio).focus = Some(one.agent.id.clone());
        }
        let names = inner.sessions.iter().map(|s| s.callsign.text()).collect::<Vec<_>>();
        // only words that answer "Who is that for?" by its id take what it holds
        let call = inner.call(radio);
        let held = match (&call.asked, &said.answers) {
            (Some((question, _, _)), Some(answers)) if question == answers => call.asked.take().map(|(_, t, f)| (t, f)),
            _ => None,
        };
        (parsed, named, inner.roster(), names, inner.unheard().count(), held)
    };
    let (mut rest, mut files) = (parsed.rest, files);
    // "Who is that for?" was just asked. A tag and nothing else answers it; so do words of his that a model reads
    // as naming someone. Either way what goes out is what he said the first time.
    if let (Some((words, their_files)), None) = (&held, control) {
        if rest.is_empty() && !named.is_empty() {
            (rest, files) = (words.clone(), their_files.clone());
        } else if named.is_empty() && !rest.is_empty()
            && let Some(meant) = named_agents(app, words, &rest, said.scope.as_deref()).await {
                named = meant;
                (rest, files) = (words.clone(), their_files.clone());
            }
    }
    // work out who it is for only when there is something to deliver
    let targets: Vec<Session> = if !named.is_empty() {
        named.clone()
    } else if rest.is_empty() && files.is_empty() && !matches!(control, Some(Control::More | Control::Repeat)) {
        Vec::new()
    } else if control.is_some() {
        // "say more" and "repeat" are about whoever he last heard
        let inner = app.read();
        inner.calls.get(radio).and_then(|c| c.focus.clone()).and_then(|f| inner.find(&f).cloned()).or((inner.sessions.len() == 1).then(|| inner.sessions[0].clone())).into_iter().collect()
    } else {
        match route(app, radio, &rest, said.re.as_deref(), said.scope.as_deref(), &said.others).await {
            Routed::Sure(t) => t,
            Routed::Elsewhere(other) => {
                log(format!("← radio: \"{text}\" → {}, on another computer", other.callsign));
                return Err(Reply { elsewhere: Some(other.id), ..Reply::default() });
            }
            Routed::New(mut idea) => {
                idea.task.push_str(&files);
                return Err(start(app, radio, text, idea, projects).await);
            }
            Routed::Ask(options) if options.is_empty() => return Err(say("No agents are running.".into())),
            Routed::Ask(options) => {
                // held with its files, so they go along when he says who it was for
                let question = config::random_hex(8);
                app.passing().call(radio).asked = Some((question.clone(), rest.clone(), files.clone()));
                let names: Vec<String> = options.iter().map(|s| s.callsign.text()).collect();
                log(format!("← radio: \"{text}\" → unsure; asking between {}", names.join(", ")));
                return Err(Reply { ask: options.iter().map(|s| s.agent.id.clone()).collect(), question: Some(question), ..ask(format!("Who is that for? {}.", names.join(", or "))) });
            }
        }
    };
    Ok(Addressed { rest, files, named, targets, roster, names, pending })
}

/// A control button: the status, skip, hold, go on, or say more or again of whoever he last heard. A poke has
/// become an ordinary message before this.
async fn controlled(app: &Arc<App>, radio: &str, control: Control, to: &Addressed) -> Reply {
    match control {
        Control::Status => say(to.roster.clone()),
        Control::Skip => say(String::new()),
        Control::Hold => {
            app.passing().call(radio).held = true;
            say("Standing by.".into())
        }
        Control::Resume => {
            app.passing().call(radio).held = false;
            app.kick.notify_one();
            let pending = to.pending;
            say(if pending > 0 { format!("Go ahead. {pending} waiting.") } else { "Go ahead. Nothing waiting.".into() })
        }
        c @ (Control::More | Control::Repeat) => {
            let last = to.targets.first().and_then(|s| s.last_heard.clone().map(|l| (s.clone(), l))).and_then(|(s, id)| app.read().log.iter().find(|e| e.id == id).map(|e| (s, id, e.line())));
            let Some((s, last, said)) = last else { return say("Nothing to repeat yet.".into()) };
            let body = if c == Control::Repeat { said } else { detail_of(app, &last).await.unwrap_or_default() };
            app.passing().call(radio).focus = Some(s.agent.id.clone());
            ask(format!("{}. {body}", s.callsign.spoken()))
        }
        Control::Poke => say(String::new()),
    }
}

/// His words to each agent they are for: as his own turn to one the tower runs or one reached through its own
/// tool, and from outside with one line in front for the rest. What reached each, what did not, and which waited.
async fn send(app: &Arc<App>, to: &Addressed, wordless: bool, re: Option<&str>, message_id: &str, uid: &str) -> (Vec<Session>, Vec<String>, Vec<String>) {
    let files = &to.files;
    let many = if to.targets.len() > 1 { format!(" Sent to {} agents.", to.targets.len()) } else { String::new() };
    let rest = if wordless { WORDLESS } else { to.rest.as_str() };
    let content = format!("From {name}, on his phone through Airband.{many}\n\n{rest}{files}", name = app.cfg.name);
    let answering = answering(app, re);
    let (mut sent, mut failed, mut queued) = (Vec::new(), Vec::new(), Vec::new());
    for t in &to.targets {
        let about = answering(&t.agent.id);
        let content = if about.is_empty() { content.clone() } else { content.replacen("\n\n", &format!("\n\n{about}"), 1) };
        let ids = [message_id.to_string()];
        let outcome = match &t.agent.deliver {
            // One the tower runs itself gets his words as his own turn, with nothing in front. If it is in the
            // middle of something they wait here, where he can still take them back, until it stops.
            Some(Deliver::Held) => {
                let words = format!("{about}{rest}{files}");
                let busy = {
                    let mut inner = app.passing();
                    match inner.session(&t.agent.id).filter(|s| !s.waiting) {
                        Some(s) => {
                            s.queue.push((message_id.to_string(), words.clone()));
                            true
                        }
                        None => false,
                    }
                };
                if busy {
                    queued.push(t.agent.id.clone());
                    Ok(())
                } else {
                    deliver(app, &Deliver::Held, &t.agent.id, &words, &ids).await
                }
            }
            // One reached through its own tool takes his words as a turn of his own too, and the tool keeps them
            // in order itself while the agent is busy.
            Some(how @ Deliver::Harness { .. }) => deliver(app, how, &t.agent.id, &format!("{about}{rest}{files}"), &ids).await,
            // Anything else gets them from outside, as a message: one line in front says who they are from.
            Some(how) => deliver(app, how, &t.agent.id, &content, &ids).await,
            None => Err("it was started in a way that gives airband nowhere to send it messages".into()),
        };
        app.store.delivery(message_id, Some(uid), &t.callsign.text(), &outcome);
        match outcome {
            Ok(()) => sent.push(t.clone()),
            Err(e) => {
                log(format!("could not reach {}: {e}", t.callsign.text()));
                failed.push(t.callsign.text());
            }
        }
    }
    (sent, failed, queued)
}

/// An answer to a question the agent has since spoken past (or he has since said other things to it) would reach
/// it with nothing to say what it answers: "Yes", to what? Then the question goes with it, for that agent only.
/// When the question is the last thing between them, the answer needs no help.
fn answering(app: &App, re: Option<&str>) -> impl Fn(&str) -> String {
    let answers: Option<(String, String)> = re.and_then(|id| {
        let inner = app.read();
        let at = inner.log.iter().position(|e| e.id == id)?;
        let asked = &inner.log[at];
        let agent = asked.agent.clone()?;
        let since = inner.log[at + 1..].iter().any(|e| e.agent.as_deref() == Some(agent.as_str()) && e.kind != Some(super::Kind::Fyi) || e.to_agents.contains(&agent));
        since.then(|| (agent, asked.line()))
    });
    move |agent: &str| answers.as_ref().filter(|(a, _)| a == agent).map(|(_, q)| format!("This answers what you asked earlier (as he was shown it): \"{q}\"\n\n")).unwrap_or_default()
}

/// What reached its agents is kept in the chat as his, they are no longer waiting on him, and he is told who it
/// reached.
#[allow(clippy::too_many_arguments)]
fn kept(app: &App, radio: &str, to: &Addressed, message_id: String, sent: Vec<Session>, failed: Vec<String>, queued: Vec<String>, shown: Option<String>, shown_files: Vec<airband_core::radio::Picture>) -> Reply {
    let mut inner = app.change();
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
        let entry = Entry::his(message_id, &sent, to.rest.clone());
        inner.record(Entry { short: shown.or(entry.short.clone()), pictures: shown_files, held: (!queued.is_empty()).then_some(Held::Queued), ..entry });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repeat_of_a_line_with_no_short_version_says_its_words() {
        let app = crate::tower::http::tests::app("repeat");
        let s = {
            let mut inner = app.change();
            let a = Agent { id: "a".into(), harness: "test".into(), cwd: Some("/x/shop".into()), name: None, title: None, pid: None, deliver: None, meta: Default::default() };
            let _ = inner.check_in(&a);
            inner.log.push(Entry { id: "l1".into(), agent: Some("a".into()), full: "The deploy is done.".into(), ..Entry::default() });
            let s = inner.session("a").unwrap();
            s.last_heard = Some("l1".into());
            s.clone()
        };
        let to = Addressed { rest: String::new(), files: String::new(), named: Vec::new(), targets: vec![s], roster: String::new(), names: Vec::new(), pending: 0 };
        let reply = controlled(&app, "radio", Control::Repeat, &to).await;
        assert!(reply.say.ends_with("The deploy is done."), "{}", reply.say);
    }
}
