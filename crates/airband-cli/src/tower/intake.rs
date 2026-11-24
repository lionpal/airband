//! Agents → tower. One `airband.agent/1` event at a time: who is on the radio, what they last said, and what is
//! now stale. Nothing here talks to a model; a message is only recorded and queued.
use super::*;
use airband_core::radio::Note;

/// Take in one event of an agent's, and set going what follows from it. The one way in, for an event that
/// came over HTTP and for one a watcher inside the tower saw. Call it where a runtime can be reached.
pub(super) fn take(app: &Arc<App>, ev: AgentEvent) -> Result<(), String> {
    // a report its reporter sent again (it did not hear the tower take it in time) is the same report
    if !app.change().events_taken.first_time(&ev.id) {
        return Ok(());
    }
    // what a turn said is taken once, whichever of its tool's ways of saying it came first
    if let Event::Said { turn: Some(turn), .. } = &ev.event {
        let mut inner = app.change();
        if let Some(s) = inner.session(&ev.agent.id).filter(|s| s.turn.as_ref() == Some(turn)) {
            // that turn is over, whatever was heard of it since by the slower way
            s.waiting = true;
            return Ok(());
        }
    }
    // on some of an agent's reports the tower looks at whether it is in the right team
    let reported = matches!(ev.event, Event::Said { .. }).then(|| ev.agent.id.clone());
    let spoke = matches!(ev.event, Event::Said { .. } | Event::Notice { .. }).then(|| ev.agent.id.clone());
    // its turn is over, with or without words
    let ended = matches!(ev.event, Event::Said { .. } | Event::Stopped { .. }).then(|| ev.agent.id.clone());
    let id = ev.id.clone();
    if let Err(why) = on_event(app, ev) {
        // refused, so not taken: the same report sent again is tried again, not passed over as done
        app.change().events_taken.forget(&id);
        return Err(why);
    }
    // With a phone that can be woken and nobody reading the chat, what it said is read now: whether it
    // deserves a notification is not known until then. With a radio reading, that radio's own request
    // has it read.
    if let Some(id) = spoke.filter(|_| app.push.any()) {
        let unread: Vec<Entry> = {
            let inner = app.read();
            // the last thing this agent said, which is what was just recorded, if nobody has read it yet
            inner.log.iter().rev().find(|e| e.agent.as_deref() == Some(id.as_str())).filter(|e| inner.readers == 0 && e.short.is_none()).cloned().into_iter().collect()
        };
        judge_later(app, unread);
    }
    if let Some(id) = &ended {
        // one the tower runs has stopped: what he said to it meanwhile goes to it now
        if app.read().find(id).is_some_and(|s| !s.queue.is_empty()) {
            let (app, id) = (app.clone(), id.clone());
            tokio::spawn(async move { held::flush(&app, &id).await });
        }
    }
    if let Some(id) = reported.filter(|id| app.read().find(id).is_some_and(|s| place::due(s.reports))) {
        tokio::spawn(place::look(app.clone(), id));
    }
    Ok(())
}

/// The ids of the events lately taken in, the newest last. Enough to cover a reporter sending one again after
/// a timeout or from its kept folder; an event without an id is never taken for another.
#[derive(Default)]
pub(super) struct Taken {
    order: std::collections::VecDeque<String>,
    ids: HashSet<String>,
}

impl Taken {
    const MOST: usize = 2000;
    /// An event that was refused: it was not taken, and one sent again with its id is taken in.
    pub(super) fn forget(&mut self, id: &str) {
        if self.ids.remove(id) {
            self.order.retain(|o| o != id);
        }
    }
    /// Whether this is the first time the event with this id is seen; it is remembered from now on.
    pub(super) fn first_time(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return true;
        }
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > Self::MOST
            && let Some(old) = self.order.pop_front()
        {
            self.ids.remove(&old);
        }
        true
    }
}

/// The same for an event a watcher inside the tower saw: written down first, as one that came over HTTP is.
pub(super) fn record_and_take(app: &Arc<App>, ev: AgentEvent) -> Result<(), String> {
    app.store.event(&serde_json::to_value(&ev).unwrap_or_default());
    take(app, ev)
}

pub(super) fn on_event(app: &App, ev: AgentEvent) -> Result<(), String> {
    if ev.schema != agent::SCHEMA {
        return Err(format!("this version of airband takes {}, not \"{}\"", agent::SCHEMA, ev.schema));
    }
    // copies of pictures are made, and what he typed at its keyboard is read, before anything is locked
    let kept = pictures_of(&ev);
    let typed = typed_at_keyboard(app, &ev);
    let mut inner = app.change();
    let id = ev.agent.id.clone();
    if let Some((told, with, upto)) = typed {
        inner.told(&id, told, with, upto);
    }
    if ev.event == Event::Signoff {
        if let Some(s) = inner.find(&id) {
            log(format!("{} signed off", short_id(&s.agent.id)));
        }
        inner.remove(&id);
        held::forget(&mut inner, &id);
        return Ok(());
    }
    if inner.check_in(&ev.agent).is_none() {
        return Err("airband cannot place an agent that has no name it can give a team and no folder with a name. Start it in a project\u{2019}s folder, or report it with --name.".into());
    }
    // the log names an agent by its id, never by its callsign, which carries its team's name
    let logged = short_id(&id).to_string();
    match ev.event {
        Event::Working => {
            let s = inner.session(&id).unwrap();
            s.waiting = false;
            s.doing = None;
            inner.no_longer_waiting(&id);
        }
        Event::Activity { text } => {
            let text: String = airband_core::prose::clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), 80);
            let s = inner.session(&id).unwrap();
            // it is doing something, so it is working, whatever it last said
            s.waiting = false;
            s.doing = (!text.is_empty()).then_some(text);
            // nothing to keep and nobody to wake: a radio that is looking sees it within a moment
            return Ok(());
        }
        Event::Model { name, effort } => {
            let short = |t: String| -> Option<String> { Some(airband_core::prose::clip(&t.split_whitespace().collect::<Vec<_>>().join(" "), 32)).filter(|t| !t.is_empty()) };
            let s = inner.session(&id).unwrap();
            (s.model, s.effort) = (short(name), effort.and_then(short));
            return Ok(());
        }
        Event::Said { text, kind, turn } => {
            let s = inner.session(&id).unwrap();
            if turn.is_some() {
                s.turn = turn;
            }
            s.waiting = true;
            s.doing = None;
            s.reports += 1;
            // its turn is over: whatever subagents it had are done with
            s.subagents = 0;
            let with = pictures::for_message(std::mem::take(&mut s.pictures), kept);
            if !with.is_empty() {
                log(format!("{logged}: {} with what it says next", if with.len() == 1 { "1 picture".to_string() } else { format!("{} pictures", with.len()) }));
            }
            enqueue(&mut inner, &id, false, text, kind, with, None);
        }
        Event::Picture { shown, .. } => {
            let s = inner.session(&id).unwrap();
            for picture in kept {
                s.pictures.retain(|p| p.picture.id != picture.id);
                s.pictures.push(pictures::Seen { picture, shown });
            }
            // an agent working through a folder of screenshots: only the latest are worth keeping
            let extra = s.pictures.len().saturating_sub(12);
            s.pictures.drain(..extra);
        }
        Event::Made { address, title, kind } => {
            log(format!("{logged}: its tool made something, shown with what it says next"));
            inner.links.made(&id, &address, title, kind);
        }
        Event::Subagent { done, .. } => {
            let s = inner.session(&id).unwrap();
            s.subagents = if done { s.subagents.saturating_sub(1) } else { s.subagents + 1 };
        }
        // the tool asks leave at its keyboard for what is on his phone already: he is not told twice
        Event::Notice { leave: true, .. } if inner.permits.values().any(|p| p.agent_id == id && p.decided.is_none()) => {
            log(format!("{logged}: its tool asks at the keyboard for what is on his phone already; not told again"));
        }
        Event::Notice { text, .. } => enqueue(&mut inner, &id, true, text, Some(Kind::NeedsYou), Vec::new(), Some(Note::AtKeyboard)),
        // its turn is over with nothing said: it is waiting, like after anything it says. A turn that failed
        // is something he must know, in the tool's own words.
        Event::Stopped { why } => {
            let s = inner.session(&id).unwrap();
            s.waiting = true;
            s.doing = None;
            s.subagents = 0;
            if let Some(why) = why.map(|w| w.trim().to_string()).filter(|w| !w.is_empty()) {
                // the tool's words can quote his: the log has their length only
                log(format!("{logged}: its turn failed ({} characters said why)", why.chars().count()));
                enqueue(&mut inner, &id, true, why, Some(Kind::NeedsYou), Vec::new(), Some(Note::TurnFailed));
            }
        }
        Event::Took { receipt } => {
            inner.took(&receipt, now_ms());
            return Ok(());
        }
        Event::Checkin | Event::Signoff => {}
    }
    inner.tell();
    Ok(())
}

/// Copies of the pictures an event brings: one the agent sent him, or ones its words show. Shrinking one takes a
/// moment, so this is done before anything is locked.
fn pictures_of(ev: &AgentEvent) -> Vec<airband_core::radio::Picture> {
    match &ev.event {
        // Only a picture the agent sent him is copied. One it only looked at is not shown to him, so the tower
        // has no business opening it: an agent going through photos on a card or a network drive would have the
        // system ask him, over and over, whether airband may read that drive.
        Event::Picture { path, shown: true } => pictures::keep(std::path::Path::new(path)).into_iter().collect(),
        Event::Said { text, .. } => pictures::in_markdown(text, ev.agent.cwd.as_deref()).iter().take(pictures::PER_MESSAGE).filter_map(|p| pictures::keep(p)).collect(),
        _ => Vec::new(),
    }
}

/// What he typed to it at its own keyboard since last read, with the pictures he pasted in, read before anything
/// is locked. It goes in ahead of this event, so his words come before the answer to them.
fn typed_at_keyboard(app: &App, ev: &AgentEvent) -> Option<(Vec<crate::adapters::Told>, Vec<Vec<airband_core::radio::Picture>>, u64)> {
    let known = app.read().find(&ev.agent.id).map(|s| (s.agent.clone(), s.told_upto));
    known.filter(|_| matches!(ev.event, Event::Working | Event::Said { .. })).map(|(agent, upto)| typed_since(&agent, upto))
}

/// What he typed at an agent's own keyboard after `upto`, with the pictures he pasted in with it (shrunk and
/// kept here, before anything is locked), and how far that reaches. Blocks.
pub(super) fn typed_since(agent: &Agent, upto: u64) -> (Vec<crate::adapters::Told>, Vec<Vec<airband_core::radio::Picture>>, u64) {
    let (told, reached) = crate::adapters::told(agent, upto);
    let with = pictures::pasted(&told);
    (told, with, reached)
}

/// An agent said something: it goes in the log, and in the queue of things he has not heard.
pub(super) fn enqueue(inner: &mut Inner, agent_id: &str, notice: bool, full: String, kind: Option<Kind>, pictures: Vec<airband_core::radio::Picture>, note: Option<Note>) {
    let Some(session) = inner.find(agent_id).cloned() else { return };
    // only the latest thing each agent said is worth hearing
    inner.supersede(agent_id, notice);
    log(format!("{}: {} queued ({} characters)", short_id(agent_id), if notice { "notice" } else { "message" }, full.chars().count()));
    let id = config::random_hex(8);
    // a notice from the harness is read to him as it is: it has its line from the start and no model reads it
    let short = notice.then(|| spoken(&note.map_or_else(|| full.clone(), |n| n.said(&full))));
    inner.links.said(agent_id, &id, &full);
    // the line is its own place in the reading queue (a key in it is masked as it is recorded, so the copy a
    // model reads to shorten it has none either)
    inner.record(Entry { pictures, short, notice, note, aloud: Aloud::Waiting, ..Entry::said(id, &session, kind, full) });
}

#[cfg(test)]
mod taken_tests {
    use super::Taken;

    #[test]
    fn a_report_sent_again_is_taken_once_and_one_without_an_id_is_never_taken_for_another() {
        let mut t = Taken::default();
        assert!(t.first_time("e1"));
        assert!(!t.first_time("e1"), "the same report sent again is not taken twice");
        assert!(t.first_time("e2"));
        assert!(t.first_time("") && t.first_time(""), "no id: nothing to know it again by");
        for i in 0..Taken::MOST {
            t.first_time(&format!("x{i}"));
        }
        assert!(t.first_time("e1"), "only the latest are remembered");
    }

    #[tokio::test]
    async fn a_report_refused_is_not_taken_and_is_taken_when_sent_again() {
        let app = crate::tower::http::tests::app("refused-report");
        let agent = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") };
        let mut ev: airband_core::agent::AgentEvent = serde_json::from_value(serde_json::json!({ "schema": "airband.agent/0", "id": "e1", "agent": agent, "event": { "type": "checkin" } })).unwrap();
        assert!(super::take(&app, ev.clone()).is_err());
        ev.schema = airband_core::agent::SCHEMA.into();
        super::take(&app, ev).unwrap();
        assert!(app.read().find("a").is_some(), "the same report, sent again, is taken in");
    }

    #[test]
    fn a_failed_turns_words_are_never_written_in_the_log() {
        let app = crate::tower::http::tests::app("turn-failed-unlogged");
        let agent = airband_core::agent::Agent { cwd: Some("/x/shop".into()), ..crate::adapters::agent_with("a", "test") };
        let ev = |id: &str, event: serde_json::Value| -> airband_core::agent::AgentEvent { serde_json::from_value(serde_json::json!({ "schema": airband_core::agent::SCHEMA, "id": id, "agent": agent, "event": event })).unwrap() };
        super::take(&app, ev("e1", serde_json::json!({ "type": "checkin" }))).unwrap();
        crate::tower::logged::take();
        super::take(&app, ev("e2", serde_json::json!({ "type": "stopped", "why": "could not do the secret plan" }))).unwrap();
        let lines = crate::tower::logged::take();
        assert!(lines.iter().any(|l| l.contains("turn failed")) && lines.iter().all(|l| !l.contains("secret plan")), "{lines:?}");
    }
}
