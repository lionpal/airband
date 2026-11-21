//! Agents → tower. One `airband.agent/1` event at a time: who is on the radio, what they last said, and what is
//! now stale. Nothing here talks to a model; a message is only recorded and queued.
use super::*;

pub(super) fn on_event(app: &App, ev: AgentEvent) -> Result<(), String> {
    if ev.schema != agent::SCHEMA {
        return Err(format!("this tower speaks {}, not \"{}\"", agent::SCHEMA, ev.schema));
    }
    // copies of pictures are made before anything is locked: shrinking one takes a moment
    let kept: Vec<airband_core::radio::Picture> = match &ev.event {
        // Only a picture the agent sent him is copied. One it only looked at is not shown to him, so the tower
        // has no business opening it: an agent going through photos on a card or a network drive would have the
        // system ask him, over and over, whether airband may read that drive.
        Event::Picture { path, shown: true } => pictures::keep(std::path::Path::new(path)).into_iter().collect(),
        Event::Said { text, .. } => pictures::named_in(text, ev.agent.cwd.as_deref()).iter().take(pictures::PER_MESSAGE).filter_map(|p| pictures::keep(p)).collect(),
        _ => Vec::new(),
    };
    // what he typed to it at its own keyboard, read before anything is locked. It goes in ahead of this event,
    // so his words come before the answer to them
    let typed = {
        let known = app.inner.lock().unwrap().find(&ev.agent.id).map(|s| (s.agent.clone(), s.told_upto));
        known.filter(|_| matches!(ev.event, Event::Working | Event::Said { .. })).map(|(agent, upto)| crate::adapters::told(&agent, upto)).map(|(told, upto)| {
            // pictures he pasted in with them, kept before anything is locked
            let with: Vec<Vec<airband_core::radio::Picture>> = told.iter().map(|t| if t.his { t.images.iter().filter_map(|bytes| pictures::keep_bytes(bytes)).collect() } else { Vec::new() }).collect();
            (told, with, upto)
        })
    };
    let mut inner = app.inner.lock().unwrap();
    let id = ev.agent.id.clone();
    if let Some((told, with, upto)) = typed {
        inner.told(&id, told, with, upto);
    }
    if ev.event == Event::Signoff {
        if let Some(s) = inner.find(&id) {
            log(format!("{} signed off", s.callsign.text()));
        }
        inner.remove(&id);
        held::forget(&mut inner, &id);
        app.save(&inner);
        return Ok(());
    }
    let callsign = inner.check_in(&ev.agent).text();
    match ev.event {
        Event::Working => {
            let s = inner.session(&id).unwrap();
            s.waiting = false;
            s.doing = None;
            inner.no_longer_waiting(&id);
        }
        Event::Activity { text } => {
            let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
            let s = inner.session(&id).unwrap();
            // it is doing something, so it is working, whatever it last said
            s.waiting = false;
            s.doing = (!text.is_empty()).then_some(text);
            // nothing to keep and nobody to wake: a radio that is looking sees it within a moment
            return Ok(());
        }
        Event::Model { name, effort } => {
            let short = |t: String| -> Option<String> { Some(t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(32).collect::<String>()).filter(|t| !t.is_empty()) };
            let s = inner.session(&id).unwrap();
            (s.model, s.effort) = (short(name), effort.and_then(short));
            app.save(&inner);
            return Ok(());
        }
        Event::Said { text, kind } => {
            let s = inner.session(&id).unwrap();
            s.waiting = true;
            s.doing = None;
            s.reports += 1;
            // its turn is over: whatever subagents it had are done with
            s.subagents = 0;
            let with = pictures::for_message(std::mem::take(&mut s.pictures), kept);
            if !with.is_empty() {
                log(format!("{callsign}: {} with what it says next", if with.len() == 1 { "1 picture".to_string() } else { format!("{} pictures", with.len()) }));
            }
            enqueue(&mut inner, &id, false, text, kind, with);
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
        Event::Subagent { done, .. } => {
            let s = inner.session(&id).unwrap();
            s.subagents = if done { s.subagents.saturating_sub(1) } else { s.subagents + 1 };
        }
        Event::Notice { text } => enqueue(&mut inner, &id, true, format!("Needs you at the keyboard: {text}"), Some(Kind::NeedsYou), Vec::new()),
        Event::Checkin | Event::Signoff | Event::Unknown => {}
    }
    app.save(&inner);
    app.kick.notify_one();
    Ok(())
}

/// An agent said something: it goes in the log, and in the queue of things he has not heard.
pub(super) fn enqueue(inner: &mut Inner, agent_id: &str, notice: bool, full: String, kind: Option<Kind>, pictures: Vec<airband_core::radio::Picture>) {
    let Some(session) = inner.find(agent_id).cloned() else { return };
    // only the latest thing each agent said is worth hearing
    inner.supersede(agent_id, notice);
    log(format!("{}: {} queued ({} characters)", session.callsign.text(), if notice { "notice" } else { "message" }, full.chars().count()));
    let id = config::random_hex(8);
    // a notice from the harness is read to him as it is: it has its line from the start and no model reads it
    let short = notice.then(|| spoken(&full));
    // the line is its own place in the reading queue (a key in it is masked as it is recorded, so the copy a
    // model reads to shorten it has none either)
    inner.record(Entry { pictures, short, notice, aloud: Aloud::Waiting, ..Entry::said(id, &session, kind, full) });
}
