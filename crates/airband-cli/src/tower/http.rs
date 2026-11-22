//! The tower's HTTP surface: `airband.agent/1` in, `airband.radio/2` both ways, and a status page for the
//! command line. Handlers are thin: they lock, call into the other modules, and answer.
use super::*;

mod agents;
mod events;
mod feed;
mod settings;
mod talk;
mod teams;
use agents::*;
use events::*;
use feed::*;
use settings::*;
use talk::*;
use teams::*;
pub(super) use events::take_event;
pub(super) use teams::read_worked;

/// Which radio is asking.
fn radio_of(headers: &HeaderMap) -> String {
    headers.get(RADIO_HEADER).and_then(|v| v.to_str().ok()).map(|v| v.chars().filter(|c| c.is_ascii_alphanumeric() || "-_ .".contains(*c)).take(40).collect::<String>()).filter(|v| !v.is_empty()).unwrap_or_else(|| "radio".into())
}

/// Compare every byte whatever the outcome, so timing says nothing about a token.
fn same(given: &[u8], want: &[u8]) -> bool {
    given.len() == want.len() && given.iter().zip(want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Two tokens. The tower's own opens everything. A radio's opens `/v1/radio/` only: it must not be able to
/// report as an agent, because an agent can name a command for the tower to run.
async fn auth(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let given = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").as_bytes();
    let own = same(given, app.cfg.token.as_bytes());
    let radio = same(given, app.cfg.radio_token.as_bytes()) && req.uri().path().starts_with("/v1/radio/");
    if own || radio { next.run(req).await } else { (StatusCode::UNAUTHORIZED, Json(json!({ "error": "bad token" }))).into_response() }
}

/// Every request from a radio says where the radio stands: on a call or not, showing the chat or not. It is
/// written down before the request is run, so every rule about whether he is there reads what the radio said.
async fn presence_said(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if req.uri().path().starts_with("/v1/radio/")
        && let Some(said) = req.headers().get(PRESENCE_HEADER).and_then(|v| v.to_str().ok()).and_then(|v| serde_json::from_str::<airband_core::radio::Presence>(v).ok()) {
            app.passing().radio_says(&radio_of(req.headers()), said);
        }
    next.run(req).await
}

/// Every route the tower serves. The relay link hands sealed requests to this same router, so a request is
/// handled identically however it arrived.
pub(super) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/v1/agent/events", post(events))
        .route("/v1/agent/held/{id}", axum::routing::delete(held_end))
        .route("/v1/agent/permission", post(permission_ask))
        .route("/v1/agent/permission/{id}", get(permission_state).delete(permission_withdraw))
        .route("/v1/radio/next", get(next))
        .route("/v1/radio/heard", post(heard))
        .route("/v1/radio/presence", post(presence))
        .route("/v1/radio/unheard", post(unheard))
        .route("/v1/radio/ready", post(ready))
        .route("/v1/radio/utterance", post(utterance))
        .route("/v1/radio/feed", get(feed))
        .route("/v1/radio/picture/{id}", get(picture_part))
        .route("/v1/radio/reads", get(reads_offered).post(feed::reads))
        .route("/v1/radio/message/{id}", get(message))
        .route("/v1/radio/attachment", post(attachment))
        .route("/v1/radio/accounts", get(accounts_list))
        .route("/v1/radio/accounts/use", post(accounts_use))
        .route("/v1/radio/team", post(team))
        .route("/v1/radio/team/rename", post(team_rename))
        .route("/v1/radio/folders", post(folders))
        .route("/v1/radio/recent", get(recent))
        .route("/v1/radio/project", post(project))
        .route("/v1/radio/takeover", post(takeover))
        .route("/v1/radio/stop", post(stop_agent))
        .route("/v1/radio/unsend", post(unsend))
        .route("/v1/radio/dismiss", post(dismiss))
        .route("/v1/radio/taken", post(taken))
        .route("/v1/radio/end", post(end_agent))
        .route("/v1/radio/push", post(push_target))
        .route("/v1/radio/tools", post(uses_tools))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(app.clone(), presence_said))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app)
}
