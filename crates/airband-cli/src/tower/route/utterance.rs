//! Something he said, from a radio: an answer to a request for leave, a new agent, a control, or words for agents.
//! Each step below either settles it and answers, or hands on to the next.
use super::*;

/// What an agent is told when he sent it files and said nothing: the tower's note, not his words.
const WORDLESS: &str = "(He sent this with no words.)";

fn say(s: String) -> Reply {
    reply(s, false)
}
fn ask(s: String) -> Reply {
    reply(s, true)
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
    /// Every agent's callsign, for asking him who it is for, and its id, in the same order.
    names: Vec<String>,
    everyone: Vec<String>,
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
    let (files, shown_files) = match attachments::describe(&said.attachments).await {
        Ok(described) => described,
        Err(why) => return Reply { undelivered: Some(Box::new(Undelivered::Files)), ..say(why) },
    };
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
        to.targets = stopped.into_iter().filter(|s| if to.named.is_empty() { s.in_room(room) } else { to.named.iter().any(|n| n.agent.id == s.agent.id) }).collect();
        if to.targets.is_empty() {
            return say("Nobody has stopped.".into());
        }
        control = None;
        // shown as it is sent: there is nothing more behind it to open
        to.rest = "Keep going.".into();
        shown = None;
    }
    if let Some(control) = control {
        log(format!("← radio: {} characters → the control {control:?}", text.chars().count()));
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
        // asked who among all of them, by id: a radio keeps his words as not delivered until he says
        return if to.names.is_empty() { nobody_running() } else { Reply { ask: to.everyone.clone(), ..ask(format!("Who is that for? {}.", either(&to.names))) } };
    }
    // his words by their length only, and who they are for by id: what he says is never written in the log.
    // What came of handing them over is logged once that is known (`send`, `kept`)
    log(format!("← radio: {} characters for {}", to.rest.chars().count(), ids_of(&to.targets)));
    // The line in the chat takes the id the radio gave what he said, when it is not in use: the radio can then
    // tell that the message it is still showing as on its way has arrived, and swap one for the other without a gap.
    let message_id = Some(said.id.as_str()).filter(|id| !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_alphanumeric()) && !app.read().log.iter().any(|e| e.id == *id)).map(str::to_string).unwrap_or_else(|| config::random_hex(8));
    let reached = send(app, &to, wordless, said.re.as_deref(), &message_id, &said.id).await;
    kept(app, radio, &to, message_id, reached, shown, shown_files)
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
        permit::Leave::Waiting => match judge::consent(&app.settings, &app.cfg().name, text).await {
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
    // Words of his own that answer the question by its id take the idea, and a model says with which project;
    // when they name none, the idea is let go. Words that answer nothing, or that came with a choice of his own
    // (a control, agents he picked), leave it held. So does a model that could not say: he is told, and can
    // answer again.
    // It is read here and let go only once his answer is settled, so nothing that fails on the way loses it.
    let held_idea = (said.act.is_none() && said.to.is_empty()).then(|| app.change().call(radio).unplaced_for(said.answers.as_ref().map(airband_core::ids::QuestionId::as_str))).flatten();
    if let Some(Unplaced { question, idea }) = held_idea {
        match named_project(app, &idea.task, text, projects).await {
            Ok(Some(place)) => {
                log(format!("← radio: {} characters → launch", text.chars().count()));
                // the idea is let go only once an agent has started with it: a launch that refuses leaves it
                // waiting, and he can answer again
                return Some(match launch(app, &place, &idea).await {
                    Ok(reply) => {
                        app.change().call(radio).resolve_unplaced(&question);
                        reply
                    }
                    Err(refused) => Reply { listen: true, question: Some(question.clone()), ..refused },
                });
            }
            Ok(None) => app.change().call(radio).resolve_unplaced(&question),
            Err(why) => {
                log(format!("which project he named could not be read: {why}"));
                return Some(Reply { say: "That could not be read just now, so the idea is still waiting. Say the project again.".into(), listen: true, question: Some(question.clone()), ..Reply::default() });
            }
        }
    }
    // the answer to "Which should start it?", the same way
    let unstarted = (said.act.is_none() && said.to.is_empty()).then(|| app.change().call(radio).unstarted_for(said.answers.as_ref().map(airband_core::ids::QuestionId::as_str))).flatten();
    if let Some(Unstarted { question, asked, place, idea, among }) = unstarted {
        match named_launcher(app, &asked, text, &among).await {
            Ok(Some(chosen)) => {
                log(format!("← radio: {} characters → launch by {chosen}", text.chars().count()));
                // what he chose is his usual from now on, kept in this computer's settings: he is asked once
                if let Err(why) = app.settings.change(|cfg| cfg.launcher = Some(chosen.clone())) {
                    log(format!("his usual launcher could not be kept: {why}"));
                }
                return Some(match launch::launch_by(app, &place, &idea, Some(&chosen)).await {
                    Ok(reply) => {
                        app.change().call(radio).resolve_unstarted(&question);
                        reply
                    }
                    Err(refused) => Reply { listen: true, question: Some(question.clone()), ..refused },
                });
            }
            Ok(None) => app.change().call(radio).resolve_unstarted(&question),
            Err(why) => {
                log(format!("which launcher he named could not be read: {why}"));
                return Some(Reply { say: "That could not be read just now, so the idea is still waiting. Say which again.".into(), listen: true, question: Some(question.clone()), ..Reply::default() });
            }
        }
    }
    let Some(Act::Launch(new)) = &said.act else { return None };
    // the team he picked, by its id, and the folder it works in
    let Some(place) = app.read().team_place(&new.team) else {
        return Some(Reply { undelivered: Some(Box::new(Undelivered::TeamGone)), ..say("That team is not on this computer any more, or its folder is gone.".into()) });
    };
    let idea = Idea { tool: new.tool.clone(), project: Some(place), model: new.model.clone().map(Pick::Id), effort: new.effort.clone().map(Pick::Id), task: format!("{}{files}", new.task) };
    Some(start(app, radio, text, idea, projects).await)
}

/// Who his words are for: the agents he picked on his screen or tagged, the answer to "Who is that for?" asked
/// just now, whoever he last heard for a control, or whoever a model reads them as for. `Err` with what to answer
/// when that settles it some other way (another computer, a new agent, a question back).
async fn addressed(app: &Arc<App>, radio: &str, text: &str, said: &airband_core::radio::Utterance, control: Option<Control>, files: String, projects: &[Place]) -> Result<Addressed, Reply> {
    let (parsed, mut named, roster, names, pending, held) = {
        app.reap();
        let mut inner = app.change();
        // a control carries no words; what it shows in the chat is not for anyone to read
        let parsed = if control.is_some() { airband_core::routing::Parsed { to: Vec::new(), rest: String::new() } } else { parse_utterance(text, &inner.callsigns()) };
        // the agents he picked on his screen, by id; else the ones his tags name
        let named: Vec<Session> = if said.to.is_empty() { parsed.to.iter().map(|&i| inner.sessions[i].clone()).collect() } else { said.to.iter().filter_map(|id| inner.find(id).cloned()).collect() };
        // the agents he picked are none of them here: his words are kept for him, as not delivered to them
        if !said.to.is_empty() && named.is_empty() {
            let missed = said.to.iter().map(|id| Missed { agent: id.clone(), why: NotReachable::Gone }).collect();
            return Err(Reply { undelivered: Some(Box::new(Undelivered::Missed { missed })), ..say(NOT_RUNNING.into()) });
        }
        if let [one] = named.as_slice() {
            inner.call(radio).focus = Some(one.agent.id.clone());
        }
        let names = inner.sessions.iter().map(|s| s.callsign.text()).collect::<Vec<_>>();
        let everyone = inner.sessions.iter().map(|s| s.agent.id.clone()).collect::<Vec<_>>();
        // only words that answer "Who is that for?" by its id read what it holds; it is let go once settled
        let held = inner.call(radio).asked_for(said.answers.as_ref().map(airband_core::ids::QuestionId::as_str));
        (parsed, named, inner.roster(), (names, everyone), inner.unheard().count(), held)
    };
    let (names, everyone) = names;
    let (mut rest, mut files) = (parsed.rest, files);
    // "Who is that for?" was just asked. A tag and nothing else answers it; so do words of his that a model reads
    // as naming someone. Either way what goes out is what he said the first time. Any answer to it settles it,
    // except one no model could read: then what he said first is kept, and he is asked again.
    if let Some(Asked { question, words, files: their_files }) = &held {
        if control.is_none() && rest.is_empty() && !named.is_empty() {
            (rest, files) = (words.clone(), their_files.clone());
        } else if control.is_none() && named.is_empty() && !rest.is_empty() {
            match named_agents(app, words, &rest, said.scope.as_deref()).await {
                Ok(Some(meant)) => {
                    named = meant;
                    (rest, files) = (words.clone(), their_files.clone());
                }
                Ok(None) => {}
                Err(why) => {
                    log(format!("who he meant could not be read: {why}"));
                    return Err(Reply { say: "That could not be read just now, so what you said first is still waiting. Say who it is for again.".into(), listen: true, question: Some(question.clone()), ..Reply::default() });
                }
            }
        }
        app.change().call(radio).resolve_asked(question);
    }
    // work out who it is for only when there is something to deliver
    let targets: Vec<Session> = if !named.is_empty() {
        named.clone()
    } else if rest.is_empty() && files.is_empty() && !matches!(control, Some(Control::More | Control::Repeat)) {
        Vec::new()
    } else if control.is_some() {
        // "say more" and "repeat" are about whoever he last heard
        let inner = app.read();
        inner.focused(radio).cloned().or((inner.sessions.len() == 1).then(|| inner.sessions[0].clone())).into_iter().collect()
    } else {
        match route(app, radio, &rest, said.re.as_deref(), said.scope.as_deref(), &said.others, projects).await {
            Routed::Sure(t) => t,
            Routed::Elsewhere(other) => {
                log(format!("← radio: {} characters → {}, on another computer", text.chars().count(), short_id(&other.id)));
                return Err(Reply { elsewhere: Some(other.id), ..Reply::default() });
            }
            Routed::New(mut idea) => {
                idea.task.push_str(&files);
                return Err(start(app, radio, text, idea, projects).await);
            }
            Routed::Ask(options) if options.is_empty() => return Err(nobody_running()),
            Routed::Ask(options) => {
                // held with its files, so they go along when he says who it was for
                let question = new_question();
                app.change().call(radio).ask(Asked { question: question.clone(), words: rest.clone(), files: files.clone() });
                let names: Vec<String> = options.iter().map(|s| s.callsign.text()).collect();
                log(format!("← radio: {} characters → unsure; asking between {}", text.chars().count(), options.iter().map(|s| short_id(&s.agent.id)).collect::<Vec<_>>().join(", ")));
                return Err(Reply { ask: options.iter().map(|s| s.agent.id.clone()).collect(), question: Some(question.clone()), ..ask(format!("Who is that for? {}.", either(&names))) });
            }
        }
    };
    Ok(Addressed { rest, files, named, targets, roster, names, everyone, pending })
}

/// His words were for agents, and none is running here: they are kept for him, as not delivered.
fn nobody_running() -> Reply {
    Reply { undelivered: Some(Box::new(Undelivered::NobodyRunning)), ..say("No agents are running.".into()) }
}

/// Agents for the log: by the start of each id, never by what they are called.
fn ids_of(agents: &[Session]) -> String {
    list(&agents.iter().map(|s| short_id(&s.agent.id).to_string()).collect::<Vec<_>>())
}

/// The one agent a control that reads one agent's line is for: none, or several, is said back to him.
fn one_named(targets: &[Session]) -> Result<&Session, &'static str> {
    match targets {
        [] => Err("Nothing to repeat yet."),
        [one] => Ok(one),
        _ => Err("Say which one: more than one agent is named."),
    }
}

/// A control button: the status, skip, hold, go on, or say more or again of whoever he last heard. A poke has
/// become an ordinary message before this.
async fn controlled(app: &Arc<App>, radio: &str, control: Control, to: &Addressed) -> Reply {
    match control {
        Control::Status => say(to.roster.clone()),
        Control::Skip => say(String::new()),
        Control::Hold => {
            app.change().call(radio).held = true;
            say("Standing by.".into())
        }
        Control::Resume => {
            app.change().call(radio).held = false;
            app.kick.notify_one();
            let pending = to.pending;
            say(if pending > 0 { format!("Go ahead. {pending} waiting.") } else { "Go ahead. Nothing waiting.".into() })
        }
        c @ (Control::More | Control::Repeat) => {
            // one agent's last line: with several named, which one is not picked by the order they were named in
            let one = match one_named(&to.targets) {
                Ok(one) => one,
                Err(why) => return say(why.into()),
            };
            let last = Some(one).and_then(|s| s.last_heard.clone().map(|l| (s.clone(), l))).and_then(|(s, id)| app.read().log.iter().find(|e| e.id == id).map(|e| (s, id, e.line())));
            let Some((s, last, said)) = last else { return say("Nothing to repeat yet.".into()) };
            let body = match c {
                Control::Repeat => said,
                _ => match detail_of(app, &last).await {
                    Some(more) => more,
                    None => return say("That message is no longer here.".into()),
                },
            };
            app.change().call(radio).focus = Some(s.agent.id.clone());
            ask(format!("{}. {body}", s.callsign.spoken()))
        }
        Control::Poke => say(String::new()),
    }
}

/// His words to each agent they are for: as his own turn to one the tower runs or one reached through its own
/// tool, and from outside with one line in front for the rest. What reached each, what did not, and which waited.
async fn send(app: &Arc<App>, to: &Addressed, wordless: bool, re: Option<&str>, message_id: &str, uid: &str) -> Reached {
    let files = &to.files;
    let many = if to.targets.len() > 1 { format!(" Sent to {} agents.", to.targets.len()) } else { String::new() };
    let rest = if wordless { WORDLESS } else { to.rest.as_str() };
    let content = format!("From {name}, on his phone.{many}\n\n{rest}{files}", name = app.cfg().name);
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
                    let mut inner = app.change();
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
                    deliver(app, &Deliver::Held, &t.agent.id, &words, &ids).await.map_err(|e| (NotReachable::Failed, e))
                }
            }
            // One reached through its own tool takes his words as a turn of his own too, and the tool keeps them
            // in order itself while the agent is busy.
            Some(how @ Deliver::Harness { .. }) => deliver(app, how, &t.agent.id, &format!("{about}{rest}{files}"), &ids).await.map_err(|e| (NotReachable::Failed, e)),
            // Anything else gets them from outside, as a message: one line in front says who they are from.
            Some(how) => deliver(app, how, &t.agent.id, &content, &ids).await.map_err(|e| (NotReachable::Failed, e)),
            None => Err((NotReachable::NoWayIn, NO_WAY_IN.to_string())),
        };
        app.store.delivery(message_id, Some(uid), &t.callsign.text(), &outcome.clone().map_err(|(_, e)| e));
        match outcome {
            Ok(()) => sent.push(t.clone()),
            Err((why, e)) => {
                log(format!("could not reach agent {}: {e}", crate::tower::short_id(&t.agent.id)));
                failed.push((t.clone(), why));
            }
        }
    }
    Reached { sent, failed, queued }
}

/// What the log and the delivery record say of an agent with no way in.
const NO_WAY_IN: &str = "it was started in a way that gives airband nowhere to send it messages";

/// What came of sending his words: the agents they reached, those they did not with why, and the ids of the
/// ones that took them to wait until they stop (among `sent`).
struct Reached {
    sent: Vec<Session>,
    failed: Vec<(Session, NotReachable)>,
    queued: Vec<String>,
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
        let since = inner.log[at + 1..].iter().any(|e| e.worth_from(&agent) || e.to_agents.contains(&agent));
        since.then(|| (agent, asked.line()))
    });
    move |agent: &str| answers.as_ref().filter(|(a, _)| a == agent).map(|(_, q)| format!("This answers what you asked earlier (as he was shown it): \"{q}\"\n\n")).unwrap_or_default()
}

/// What reached its agents is kept in the chat as his, they are no longer waiting on him, and he is told who it
/// reached.
fn kept(app: &App, radio: &str, to: &Addressed, message_id: String, Reached { sent, failed, queued }: Reached, shown: Option<String>, shown_files: Vec<airband_core::radio::Picture>) -> Reply {
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
    drop(inner);
    let sent_names: Vec<String> = sent.iter().map(|s| s.callsign.text()).collect();
    let failed_names: Vec<String> = failed.iter().map(|(s, _)| s.callsign.text()).collect();
    // what came of it, by ids: the one line that says whether it was delivered
    log(match (sent.is_empty(), failed.is_empty()) {
        (_, true) => format!("delivered to {}", ids_of(&sent)),
        (true, false) => format!("not delivered: {}", missed_of(&failed)),
        (false, false) => format!("delivered to {}; not delivered: {}", ids_of(&sent), missed_of(&failed)),
    });
    let reply = say(match (sent_names.len(), failed_names.len()) {
        (0, _) => format!("Could not reach {}.", list(&failed_names)),
        (n, 0) if n > 3 => format!("Sent to all {n}."),
        (_, 0) => format!("Sent to {}.", list(&sent_names)),
        _ => format!("Sent to {}. Could not reach {}.", list(&sent_names), list(&failed_names)),
    });
    // the ones it did not reach, each with why: the radio keeps his words for them
    let undelivered = (!failed.is_empty()).then(|| Box::new(Undelivered::Missed { missed: failed.iter().map(|(s, why)| Missed { agent: s.agent.id.clone(), why: *why }).collect() }));
    // it reached everyone it was for: the words only confirm that
    Reply { done: failed.is_empty() && !sent_names.is_empty(), undelivered, ..reply }
}

/// Agents his words did not reach, for the log: each by the start of its id, with why by kind.
fn missed_of(failed: &[(Session, NotReachable)]) -> String {
    list(&failed.iter().map(|(s, why)| format!("{} ({why:?})", short_id(&s.agent.id))).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use airband_core::ids::QuestionId;

    #[test]
    fn say_more_with_several_agents_named_asks_which_and_never_takes_the_first() {
        let one = |id: &str| Session::new(airband_core::agent::Agent { id: id.into(), ..crate::adapters::agent_with("", "t") }, airband_core::routing::Callsign { project: "shop".into(), number: id.into(), named: false }, TeamId::new("/shop"));
        assert!(one_named(&[]).is_err());
        assert_eq!(one_named(&[one("a")]).map(|s| s.agent.id.clone()), Ok("a".into()));
        assert!(one_named(&[one("a"), one("b")]).is_err());
    }

    #[tokio::test]
    async fn repeat_of_a_line_with_no_short_version_says_its_words() {
        let app = crate::tower::http::tests::app("repeat");
        let s = {
            let mut inner = app.change();
            let a = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") };
            let _ = inner.check_in(&a);
            inner.log.push(Entry { id: "l1".into(), agent: Some("a".into()), full: "The deploy is done.".into(), ..Entry::default() });
            let s = inner.session("a").unwrap();
            s.last_heard = Some("l1".into());
            s.clone()
        };
        let to = Addressed { rest: String::new(), files: String::new(), named: Vec::new(), targets: vec![s], roster: String::new(), names: Vec::new(), everyone: Vec::new(), pending: 0 };
        let reply = controlled(&app, "radio", Control::Repeat, &to).await;
        assert!(reply.say.ends_with("The deploy is done."), "{}", reply.say);
    }

    #[tokio::test]
    async fn an_agent_found_gone_while_he_speaks_is_let_go_for_good_and_the_radios_told() {
        let app = crate::tower::http::tests::app("reaped");
        {
            let mut inner = app.change();
            let a = airband_core::agent::Agent { cwd: Some("/x/shop".into()), pid: Some(i32::MAX as u32), ..crate::adapters::agent_with("a", "test") };
            inner.check_in(&a).unwrap();
        }
        app.flush();
        assert_eq!(app.store.load(10).unwrap().0.sessions.len(), 1);
        let _ = addressed(&app, "radio", "hello", &airband_core::radio::Utterance { text: "hello".into(), ..Default::default() }, None, String::new(), &[]).await;
        assert!(app.read().sessions.is_empty());
        app.flush();
        assert!(app.store.load(10).unwrap().0.sessions.is_empty(), "written down as gone");
    }

    /// What the tower logged while he said `words` to it, with these agents checked in.
    async fn logged_saying(app: &Arc<App>, agents: &[&str], words: &str, others: Vec<airband_core::radio::Other>) -> Vec<String> {
        {
            let mut inner = app.change();
            for id in agents {
                let a = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with(*id, "test") };
                inner.check_in(&a).unwrap();
            }
        }
        crate::tower::logged::take();
        let _ = on_utterance(app, "radio", words, &airband_core::radio::Utterance { text: words.into(), id: "u1".into(), others, ..Default::default() }).await;
        crate::tower::logged::take()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn his_words_are_never_written_in_the_log() {
        let words = "please ship the secret plan";
        let unlogged = |lines: &[String]| !lines.is_empty() && lines.iter().all(|l| !l.contains("secret plan"));
        // for the one agent there is
        let lines = logged_saying(&crate::tower::http::tests::app("words-unlogged"), &["a"], words, Vec::new()).await;
        assert!(unlogged(&lines), "{lines:?}");
        // unsure, with two agents and no model: he is asked which
        let lines = logged_saying(&crate::tower::http::tests::app("words-unlogged-ask"), &["a", "b"], words, Vec::new()).await;
        assert!(lines.iter().any(|l| l.contains("unsure")) && unlogged(&lines), "{lines:?}");
        // for an agent on another computer, as a reader of his own says
        #[cfg(unix)]
        {
            let reader = serde_json::json!({ "summarizer": { "kind": "command", "offer": "own", "argv": ["sh", "-c", "cat >/dev/null; echo '{\"to\":[2],\"all\":false,\"sure\":true,\"new\":null}'"] } });
            let other = airband_core::radio::Other { id: "far".into(), callsign: "far one".into(), group: "far".into(), team: airband_core::ids::TeamId::new("/far"), badge: String::new(), said: String::new(), ago: 0 };
            let lines = logged_saying(&crate::tower::http::tests::app_with("words-unlogged-far", reader), &["a"], words, vec![other]).await;
            assert!(lines.iter().any(|l| l.contains("on another computer")) && unlogged(&lines), "{lines:?}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn his_agents_names_are_never_written_in_the_log() {
        // an agent named for its folder, that cannot be reached, and then is found gone
        let app = crate::tower::http::tests::app("names-unlogged");
        {
            let mut inner = app.change();
            let a = airband_core::agent::Agent { cwd: Some("/x/billing-refunds".into()), ..crate::adapters::agent_with("a", "test") };
            inner.check_in(&a).unwrap();
        }
        crate::tower::logged::take();
        let _ = on_utterance(&app, "radio", "hello", &airband_core::radio::Utterance { text: "hello".into(), id: "u1".into(), ..Default::default() }).await;
        {
            let mut inner = app.change();
            let gone = airband_core::agent::Agent { cwd: Some("/x/billing-refunds".into()), pid: Some(i32::MAX as u32), ..crate::adapters::agent_with("b", "test") };
            inner.check_in(&gone).unwrap();
            inner.reap();
        }
        let lines = crate::tower::logged::take();
        assert!(lines.iter().any(|l| l.contains("is gone")), "{lines:?}");
        assert!(lines.iter().all(|l| !l.contains("billing") && !l.contains("refunds")), "{lines:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn words_for_an_agent_with_no_way_in_are_refused_by_kind_and_the_log_says_so_by_id() {
        let app = crate::tower::http::tests::app("no-way-in");
        {
            let mut inner = app.change();
            let a = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("c02ccdbf-1", "test") };
            inner.check_in(&a).unwrap();
        }
        crate::tower::logged::take();
        let said = airband_core::radio::Utterance { text: "fifteen letters".into(), id: "u1".into(), to: vec!["c02ccdbf-1".into()], ..Default::default() };
        let reply = on_utterance(&app, "radio", "fifteen letters", &said).await;
        assert_eq!(reply.undelivered.as_deref(), Some(&Undelivered::Missed { missed: vec![Missed { agent: "c02ccdbf-1".into(), why: NotReachable::NoWayIn }] }));
        assert!(!reply.done);
        let lines = crate::tower::logged::take();
        // no line reads as if it went: the one about it says it was not delivered, by id and kind
        assert!(lines.iter().all(|l| !l.contains("passed on") && !l.contains("None")), "{lines:?}");
        assert!(lines.iter().any(|l| l == "not delivered: c02ccdbf (NoWayIn)"), "{lines:?}");
        // and the chat has no line of his for it: nothing reached anyone
        assert!(app.read().log.iter().all(|e| !e.his));
        // the roster says so before he types, and that this computer cannot take it over (no launcher holds its tool)
        assert_eq!(app.read().roster_entries(&[])[0].reach, Reach::NoWayIn { take_over: false });
    }

    #[tokio::test(flavor = "current_thread")]
    async fn words_for_agents_that_are_gone_or_for_nobody_are_refused_by_kind() {
        let app = crate::tower::http::tests::app("gone-or-nobody");
        // nobody is running
        let reply = on_utterance(&app, "radio", "hello", &airband_core::radio::Utterance { text: "hello".into(), id: "u1".into(), ..Default::default() }).await;
        assert_eq!(reply.undelivered.as_deref(), Some(&Undelivered::NobodyRunning));
        // the agent he picked has gone
        let reply = on_utterance(&app, "radio", "hello", &airband_core::radio::Utterance { text: "hello".into(), id: "u2".into(), to: vec!["gone".into()], ..Default::default() }).await;
        assert_eq!(reply.undelivered.as_deref(), Some(&Undelivered::Missed { missed: vec![Missed { agent: "gone".into(), why: NotReachable::Gone }] }));
        // files that had not arrived
        let reply = on_utterance(&app, "radio", "hello", &airband_core::radio::Utterance { text: "hello".into(), id: "u3".into(), attachments: vec!["never-sent".into()], ..Default::default() }).await;
        assert_eq!(reply.undelivered.as_deref(), Some(&Undelivered::Files));
    }

    #[test]
    fn an_agent_with_a_way_in_reaches_and_one_without_can_be_taken_over_only_where_a_launcher_carries_its_tool() {
        let app = crate::tower::http::tests::app("reach");
        let mut inner = app.change();
        let open = airband_core::agent::Agent { cwd: Some("/x/shop".into()), pid: Some(std::process::id()), deliver: Some(Deliver::Socket { path: "/tmp/x.sock".into(), token: None }), ..crate::adapters::agent_with("a", "test") };
        let shut = airband_core::agent::Agent { cwd: Some("/x/shop".into()), pid: Some(std::process::id()), ..crate::adapters::agent_with("b", "test") };
        let no_pid = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("c", "test") };
        for a in [&open, &shut, &no_pid] {
            inner.check_in(a).unwrap();
        }
        fn reach(inner: &Inner, carried: &[String]) -> Vec<(String, Reach)> {
            inner.roster_entries(carried).into_iter().map(|r| (r.id, r.reach)).collect()
        }
        assert_eq!(reach(&inner, &["test".into()]), [("a".into(), Reach::Reaches), ("b".into(), Reach::NoWayIn { take_over: true }), ("c".into(), Reach::NoWayIn { take_over: false })]);
        assert_eq!(reach(&inner, &[])[1], ("b".into(), Reach::NoWayIn { take_over: false }));
        // a later report that cannot see its way in does not take it away
        inner.check_in(&airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") }).unwrap();
        assert_eq!(reach(&inner, &[])[0], ("a".into(), Reach::Reaches));
    }

    fn answering(text: &str, question: &str) -> airband_core::radio::Utterance {
        airband_core::radio::Utterance { text: text.into(), answers: Some(airband_core::ids::QuestionId::new(question)), ..Default::default() }
    }

    #[tokio::test]
    async fn an_idea_waiting_for_its_project_is_kept_when_no_model_can_read_his_answer() {
        // no model is set here, so asking one fails
        let app = crate::tower::http::tests::app("idea-kept");
        let idea = Idea { task: "add a cart".into(), ..Idea::default() };
        app.change().call("radio").hold_unplaced(Unplaced { question: QuestionId::new("q1"), idea: idea.clone() });
        let places = [Place { name: "shop".into(), folder: "/w/shop".into() }, Place { name: "garden".into(), folder: "/w/garden".into() }];
        let reply = new_agent(&app, "radio", "the shop", &answering("the shop", "q1"), "", &places).await;
        let reply = reply.expect("he is told");
        assert_eq!(reply.question.as_ref().map(airband_core::ids::QuestionId::as_str), Some("q1"), "{}", reply.say);
        assert_eq!(app.change().call("radio").unplaced_for(Some("q1")), Some(Unplaced { question: QuestionId::new("q1"), idea }));
    }

    #[tokio::test]
    async fn an_idea_waiting_for_its_project_is_not_taken_by_words_that_pick_an_agent() {
        let app = crate::tower::http::tests::app("idea-picked");
        app.change().call("radio").hold_unplaced(Unplaced { question: QuestionId::new("q1"), idea: Idea { task: "add a cart".into(), ..Idea::default() } });
        let said = airband_core::radio::Utterance { to: vec!["a".into()], ..answering("hello", "q1") };
        assert!(new_agent(&app, "radio", "hello", &said, "", &[]).await.is_none());
        assert!(app.change().call("radio").unplaced_for(Some("q1")).is_some());
    }

    #[tokio::test]
    async fn his_first_words_are_kept_when_no_model_can_read_who_he_says_they_are_for() {
        let app = crate::tower::http::tests::app("asked-kept");
        app.change().call("radio").ask(asked("q1"));
        let Err(reply) = addressed(&app, "radio", "the first one", &answering("the first one", "q1"), None, String::new(), &[]).await else { panic!("it is not sent anywhere") };
        assert_eq!(reply.question.as_ref().map(airband_core::ids::QuestionId::as_str), Some("q1"), "{}", reply.say);
        assert_eq!(app.change().call("radio").asked_for(Some("q1")), Some(asked("q1")));
    }

    fn asked(question: &str) -> Asked {
        Asked { question: QuestionId::new(question), words: "ship it".into(), files: "\n[file]".into() }
    }

    #[test]
    fn a_question_is_let_go_only_by_its_own_id_and_a_newer_one_is_never_taken_for_it() {
        let mut call = super::super::Call::default();
        call.ask(asked("q2"));
        // an answer to an older question reads nothing, and settling that older question lets nothing go
        assert_eq!(call.asked_for(Some("q1")), None);
        call.resolve_asked(&QuestionId::new("q1"));
        assert_eq!(call.asked_for(Some("q2")), Some(asked("q2")));
        // its own answer reads it without taking it; only settling it lets it go
        assert_eq!(call.asked_for(Some("q2")), Some(asked("q2")));
        assert!(call.asked_for(Some("q2")).is_some());
        call.resolve_asked(&QuestionId::new("q2"));
        assert_eq!(call.asked_for(Some("q2")), None);
        let idea = Idea { task: "add a cart".into(), ..Idea::default() };
        call.hold_unplaced(Unplaced { question: QuestionId::new("q3"), idea });
        call.resolve_unplaced(&QuestionId::new("q1"));
        assert!(call.unplaced_for(Some("q3")).is_some() && call.unplaced_for(None).is_none());
        call.resolve_unplaced(&QuestionId::new("q3"));
        assert_eq!(call.unplaced_for(Some("q3")), None);
    }

    #[tokio::test]
    async fn an_answer_that_names_no_one_settles_the_question_and_lets_his_first_words_go() {
        // a tag alone answers it: his first words go to the agent tagged, and nothing is left held
        let app = crate::tower::http::tests::app("asked-settled");
        app.change().call("radio").ask(asked("q1"));
        let _ = addressed(&app, "radio", "", &airband_core::radio::Utterance { to: vec!["gone".into()], ..answering("", "q1") }, None, String::new(), &[]).await;
        // the agent picked is not running, so he is told; the question is still his to answer
        assert_eq!(app.change().call("radio").asked_for(Some("q1")), Some(asked("q1")));
        // a control that comes with the answer settles it
        let _ = addressed(&app, "radio", "", &answering("", "q1"), Some(Control::Skip), String::new(), &[]).await;
        assert_eq!(app.change().call("radio").asked_for(Some("q1")), None);
    }
}
