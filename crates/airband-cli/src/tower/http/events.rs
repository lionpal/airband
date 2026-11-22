//! What the coding tools report: their events, and the files they hand over.
use super::*;

/// An agent's event. It is written down exactly as it arrived before anything is made of it, so an event this
/// build cannot read is still on record for one that can.
/// Its body is read as [`at::Events`]'s only after that, so this one route takes its body as it came.
pub(super) async fn events(State(app): State<Arc<App>>, body: String) -> Answer<at::Events> {
    let raw = serde_json::from_str::<Value>(&body).map_err(|_| bad("not JSON"))?;
    app.store.event(&raw);
    let ev: <at::Events as Endpoint>::Body = serde_json::from_value(raw).map_err(|e| bad(format!("not an airband.agent/1 event: {e}")))?;
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
            done()
        }
        Err(e) => Err(bad(e)),
    }
}

/// One part of a file he is attaching. Answers with how much of the file is whole from its start.
pub(super) async fn attachment(Sent(part): Sent<at::Attachment>) -> Answer<at::Attachment> {
    match tokio::task::spawn_blocking(move || attachments::take(part)).await {
        Ok(Ok(next)) => Ok(Json(wire::AttachmentNext { next })),
        Ok(Err(why)) => Err(no(why)),
        Err(_) => Err(No(Refusal::NotNow { reason: "That part of the file could not be taken just now.".into() })),
    }
}
