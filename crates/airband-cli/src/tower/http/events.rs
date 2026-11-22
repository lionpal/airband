//! What the coding tools report: their events, and the files they hand over.
use super::*;

/// An agent's event. It is written down exactly as it arrived before anything is made of it, so an event this
/// build cannot read is still on record for one that can.
pub(super) async fn events(State(app): State<Arc<App>>, body: String) -> Response {
    let Ok(raw) = serde_json::from_str::<Value>(&body) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "not JSON" }))).into_response() };
    app.store.event(&raw);
    let ev: AgentEvent = match serde_json::from_value(raw) {
        Ok(ev) => ev,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("not an airband.agent/1 event: {e}") }))).into_response(),
    };
    let from = ev.agent.id.clone();
    let by_hook = ev.via == Some(airband_core::agent::Via::Hook);
    // an event with a picture is slow to take in (the picture is shrunk); the other requests must not wait behind it
    match tokio::task::block_in_place(|| take_event(&app, ev)) {
        Ok(()) => {
            // its tool's own hook reported it, and says so: this session's hooks report
            if by_hook && app.read().find(&from).is_some_and(|s| !s.hooked)
                && let Some(s) = app.change().session(&from) {
                    s.hooked = true;
                }
            Json(json!({})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// Take in one event of an agent's, and set going what follows from it. The one way in, for an event that
/// came over HTTP and for one a watcher inside the tower saw. Call it where a runtime can be reached.
pub(in crate::tower) fn take_event(app: &Arc<App>, ev: AgentEvent) -> Result<(), String> {
    // what a turn said is taken once, whichever of its tool's ways of saying it came first
    if let Event::Said { turn: Some(turn), .. } = &ev.event {
        let mut inner = app.passing();
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
    on_event(app, ev)?;
    // With a phone that can be woken and nobody reading the chat, what it said is read now: whether it
    // deserves a notification is not known until then. With a radio reading, that radio's own request
    // has it read.
    if let Some(id) = spoke.filter(|_| app.push.any()) {
        let unread: Vec<Entry> = {
            let inner = app.read();
            inner.log.iter().rev().take(8).filter(|e| inner.readers == 0 && e.short.is_none() && e.agent.as_deref() == Some(id.as_str())).take(1).cloned().collect()
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

/// One part of a file he is attaching. Answers with how much of the file is whole from its start.
pub(super) async fn attachment(Json(part): Json<airband_core::radio::AttachmentPart>) -> Response {
    match tokio::task::spawn_blocking(move || attachments::take(part)).await {
        Ok(Ok(next)) => Json(json!({ "next": next })).into_response(),
        Ok(Err(why)) => Json(json!({ "error": why })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
