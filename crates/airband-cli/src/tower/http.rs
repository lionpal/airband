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

/// Which radio is asking: the name of its pairing, exactly as the pairing has it. A request the relay link or
/// the near listener opened carries it from the pairing whose key opened it; one on the plain direct route says
/// it in its header. Every place that keeps something per radio (settled answers, presence, push addresses)
/// goes by this one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RadioId(pub String);

/// The answer for a request airband would not do: why, in words for him.
fn refused(why: impl std::fmt::Display) -> Value {
    json!({ "error": why.to_string() })
}

use airband_core::token::same;

/// Two tokens. The tower's own opens everything. A radio's opens `/v1/radio/` only: it must not be able to
/// report as an agent, because an agent can name a command for the tower to run.
async fn auth(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let given = airband_core::token::bearer(req.headers().get("authorization").and_then(|v| v.to_str().ok())).as_bytes();
    let own = same(given, app.cfg.token.as_bytes());
    let radio = same(given, app.cfg.radio_token.as_bytes()) && req.uri().path().starts_with(airband_core::radio::PATH);
    if !(own || radio) {
        return (StatusCode::UNAUTHORIZED, Json(refused("bad token"))).into_response();
    }
    let mut req = req;
    // A radio must say which one it is. The computer's own command line (the tower's token) is no radio: it
    // may ask what any radio may ask that is not kept per radio (accounts, the feed), and says no name.
    let named = req.headers().get(RADIO_HEADER).is_some();
    if req.uri().path().starts_with(airband_core::radio::PATH) && req.extensions().get::<RadioId>().is_none() && (radio || named) {
        match req.headers().get(RADIO_HEADER).and_then(|v| v.to_str().ok()).filter(|v| !v.is_empty()).map(String::from) {
            Some(name) => {
                req.extensions_mut().insert(RadioId(name));
            }
            None => return (StatusCode::BAD_REQUEST, Json(refused("the request does not say which radio it is from"))).into_response(),
        }
    }
    next.run(req).await
}

/// Every request from a radio says where the radio stands: on a call or not, showing the chat or not. It is
/// written down before the request is run, so every rule about whether he is there reads what the radio said.
async fn presence_said(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if req.uri().path().starts_with(airband_core::radio::PATH)
        && let Some(said) = req.headers().get(PRESENCE_HEADER).and_then(|v| v.to_str().ok()).and_then(|v| serde_json::from_str::<airband_core::radio::Presence>(v).ok())
        && let Some(RadioId(radio)) = req.extensions().get::<RadioId>() {
            app.passing().radio_says(radio, said);
        }
    next.run(req).await
}

/// Every route the tower serves. The relay link hands sealed requests to this same router, so a request is
/// handled identically however it arrived.
/// The route something he said comes by: the one request a relay may keep while the tower is away.
pub(super) const UTTERANCE: &str = "/v1/radio/utterance";

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
        .route(UTTERANCE, post(utterance))
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

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use tower::ServiceExt;

    /// A tower with nothing in it, its state in a folder of its own.
    pub(in crate::tower) fn app(name: &str) -> Arc<App> {
        let path = std::env::temp_dir().join(format!("airband-http-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).unwrap();
        let cfg: Config = serde_json::from_str(r#"{"token":"own","radio_token":"radio"}"#).unwrap();
        let push = push::Wake::load(&store).unwrap();
        Arc::new(App { push, cfg, store, inner: Mutex::new(Inner::default()), kick: Notify::new(), said: Mutex::new(HashMap::new()), accounts: accounts::Kept::default() })
    }

    fn presence(radio: Option<RadioId>, header: Option<&str>) -> Request {
        let mut req = Request::builder().method("POST").uri("/v1/radio/presence").header("authorization", "Bearer radio").header("content-type", "application/json");
        if let Some(h) = header {
            req = req.header(RADIO_HEADER, h);
        }
        if let Some(r) = radio {
            req = req.extension(r);
        }
        req.body(axum::body::Body::from(r#"{"on_call":true,"showing_chat":false}"#)).unwrap()
    }

    #[tokio::test]
    async fn a_radio_is_known_by_its_pairing_name_exactly_and_never_by_a_made_up_one() {
        let app = app("radio-id");
        // from a pairing whose name is not plain letters: kept under that name, as push and settled answers find it
        let res = router(app.clone()).oneshot(presence(Some(RadioId("Charles's phone ☎".into())), None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(app.read().calls.contains_key("Charles's phone ☎"));
        // a header cannot stand in for a pairing the link already named
        let res = router(app.clone()).oneshot(presence(Some(RadioId("kitchen".into())), Some("someone else"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(app.read().calls.contains_key("kitchen") && !app.read().calls.contains_key("someone else"));
        // the plain direct route says its name; saying none is refused, never called "radio"
        let res = router(app.clone()).oneshot(presence(None, Some("desk phone"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(app.read().calls.contains_key("desk phone"));
        let res = router(app.clone()).oneshot(presence(None, None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert!(!app.read().calls.contains_key("radio"));
    }

    #[tokio::test]
    async fn a_feed_asked_with_a_number_that_is_not_one_is_refused_and_not_read_as_none() {
        let app = app("feed-query");
        let ask = |query: &str| Request::builder().method("GET").uri(format!("/v1/radio/feed?{query}")).header("authorization", "Bearer radio").extension(RadioId("phone".into())).body(axum::body::Body::empty()).unwrap();
        assert_eq!(router(app.clone()).oneshot(ask("after=0&wait=0")).await.unwrap().status(), 200);
        assert_eq!(router(app.clone()).oneshot(ask("after=seven&wait=0")).await.unwrap().status(), 400);
    }
}
