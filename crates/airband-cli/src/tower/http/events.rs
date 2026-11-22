//! What the coding tools report: their events, and the files they hand over.
use super::*;

/// An agent's event. It is written down exactly as it arrived before anything is made of it, so an event this
/// build cannot read is still on record for one that can.
pub(super) async fn events(State(app): State<Arc<App>>, body: String) -> Response {
    let Ok(raw) = serde_json::from_str::<Value>(&body) else { return (StatusCode::BAD_REQUEST, Json(refused("not JSON"))).into_response() };
    app.store.event(&raw);
    let ev: AgentEvent = match serde_json::from_value(raw) {
        Ok(ev) => ev,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(refused(format!("not an airband.agent/1 event: {e}")))).into_response(),
    };
    let from = ev.agent.id.clone();
    let by_hook = ev.via == Some(airband_core::agent::Via::Hook);
    // an event with a picture is slow to take in (the picture is shrunk); the other requests must not wait behind it
    match tokio::task::block_in_place(|| intake::take(&app, ev)) {
        Ok(()) => {
            // its tool's own hook reported it, and says so: this session's hooks report
            if by_hook && app.read().find(&from).is_some_and(|s| !s.hooked)
                && let Some(s) = app.change().session(&from) {
                    s.hooked = true;
                }
            Json(json!({})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(refused(e))).into_response(),
    }
}

/// One part of a file he is attaching. Answers with how much of the file is whole from its start.
pub(super) async fn attachment(Json(part): Json<airband_core::radio::AttachmentPart>) -> Response {
    match tokio::task::spawn_blocking(move || attachments::take(part)).await {
        Ok(Ok(next)) => Json(json!({ "next": next })).into_response(),
        Ok(Err(why)) => Json(refused(why)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
