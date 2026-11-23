//! The tower's HTTP surface: `airband.agent/1` in, the radio protocol ([`airband_core::radio::SCHEMA`]) both
//! ways, and a status page for the command line. Handlers are thin: they lock, call into the other modules, and
//! answer.
//!
//! Every route is one endpoint of core's [`wire::at`], served by [`serve`]: its query and body are read as the
//! endpoint's types, and it answers with the endpoint's answer (a 200) or a [`Refusal`] (a 4xx). A 200 never
//! carries an error.
use super::*;
use airband_core::radio::SCHEMA_HEADER;
use airband_core::radio::wire::{self, Endpoint, Refusal, at};
use axum::extract::{FromRequest, FromRequestParts};
use axum::handler::Handler;

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

/// Which radio is asking: its pairing's relay channel (the id its secret gives), never its display name. A
/// request the relay link or the near listener opened carries it from the pairing whose key opened it; one on
/// the plain direct route says it in its header. Every place that keeps something per radio (settled answers, presence, push addresses)
/// goes by this one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RadioId(pub String);

/// A request the tower will not do, as it is sent: the refusal's own status, and the refusal.
#[derive(Debug)]
pub(super) struct No(pub(super) Refusal);

impl IntoResponse for No {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::BAD_REQUEST);
        (status, Json(self.0)).into_response()
    }
}

/// Why it could not be done, in words for him.
pub(super) fn no(why: impl std::fmt::Display) -> No {
    No(Refusal::told(why.to_string()))
}

/// What a route of endpoint `E` answers: its answer, or why not.
pub(super) type Answer<E> = Result<Json<<E as Endpoint>::Answer>, No>;

/// Done, for a route that has nothing to say back.
pub(super) fn done() -> Result<Json<wire::Done>, No> {
    Ok(Json(wire::Done {}))
}

/// What was sent the wrong way: a query value that is not a number, a body that is not the endpoint's.
pub(super) fn bad(why: impl std::fmt::Display) -> No {
    No(Refusal::BadAsk { reason: why.to_string() })
}

/// The query of endpoint `E`, read as its type. A value that does not read is refused, never read as its
/// default.
pub(super) struct Ask<E: Endpoint>(pub(super) E::Query);

impl<E: Endpoint, S: Send + Sync> FromRequestParts<S> for Ask<E>
where
    E::Query: Send,
{
    type Rejection = No;
    async fn from_request_parts(parts: &mut axum::http::request::Parts, state: &S) -> Result<Self, No> {
        Query::<E::Query>::from_request_parts(parts, state).await.map(|Query(q)| Ask(q)).map_err(|e| bad(e.body_text()))
    }
}

/// Which radio asked: a route kept per radio is refused, as a refusal, to a caller that names none (the
/// tower's own token without the radio header), never answered with a bare error.
impl<S: Send + Sync> FromRequestParts<S> for RadioId {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut axum::http::request::Parts, _: &S) -> Result<Self, Response> {
        parts.extensions.get::<RadioId>().cloned().ok_or_else(|| No(Refusal::NotAllowed).into_response())
    }
}

/// The body of endpoint `E`, read as its type. One that does not read is refused.
pub(super) struct Sent<E: Endpoint>(pub(super) E::Body);

impl<E: Endpoint, S: Send + Sync> FromRequest<S> for Sent<E>
where
    E::Body: Send,
{
    type Rejection = No;
    async fn from_request(req: Request, state: &S) -> Result<Self, No> {
        Json::<E::Body>::from_request(req, state).await.map(|Json(b)| Sent(b)).map_err(|e| bad(e.body_text()))
    }
}

/// Endpoint `E` served by `handler`, at its path and by its method. Two endpoints on one path (a GET and a
/// POST) share it.
fn serve<E: Endpoint, H: Handler<T, Arc<App>>, T: 'static>(router: Router<Arc<App>>, handler: H) -> Router<Arc<App>> {
    let method = match E::METHOD {
        wire::Method::Get => get(handler),
        wire::Method::Post => post(handler),
        wire::Method::Delete => axum::routing::delete(handler),
    };
    router.route(E::PATH, method)
}

use airband_core::token::same;

/// Two tokens. The tower's own opens everything. A radio's opens `/v1/radio/` only: it must not be able to
/// report as an agent, because an agent can name a command for the tower to run.
async fn auth(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let given = airband_core::token::bearer(req.headers().get("authorization").and_then(|v| v.to_str().ok())).as_bytes();
    let own = same(given, app.cfg().token.as_bytes());
    let radio = same(given, app.cfg().radio_token.as_bytes()) && req.uri().path().starts_with(airband_core::radio::PATH);
    if !(own || radio) {
        // a token that opens nothing: 401, so the radio knows its pairing is wrong, not that this one route is shut
        return No(Refusal::NotPaired).into_response();
    }
    // everything a radio's route is asked says which version of the protocol it is in; another, or none, is
    // told so and goes no further
    if req.uri().path().starts_with(airband_core::radio::PATH) {
        let speaks = req.headers().get(SCHEMA_HEADER).and_then(|v| v.to_str().ok()).unwrap_or_default();
        if speaks != airband_core::radio::SCHEMA {
            log(format!("a request said it speaks {}, and this version of airband takes {}", if speaks.is_empty() { "no version" } else { speaks }, airband_core::radio::SCHEMA));
            return No(Refusal::OtherVersion { speaks: airband_core::radio::SCHEMA.into() }).into_response();
        }
    }
    let mut req = req;
    // A radio must say which one it is. A caller with the tower's own token (the hooks, `airband status`) is no
    // radio: it may ask, naming none, what is not kept per radio (accounts); the feed is per radio, and needs one.
    let named = req.headers().get(RADIO_HEADER).is_some();
    if req.uri().path().starts_with(airband_core::radio::PATH) && req.extensions().get::<RadioId>().is_none() && (radio || named) {
        match req.headers().get(RADIO_HEADER).and_then(|v| v.to_str().ok()).filter(|v| !v.is_empty()).map(String::from) {
            Some(name) => {
                req.extensions_mut().insert(RadioId(name));
            }
            None => return bad("the request does not say which radio it is from").into_response(),
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
            let name = push::radio_name(&app.cfg(), radio);
            app.change().radio_named_says(radio, name, said);
        }
    next.run(req).await
}

/// Every route the tower serves: each endpoint of [`wire::at`], and the status page for the command line. The
/// relay link hands sealed requests to this same router, so a request is handled identically however it arrived.
pub(super) fn router(app: Arc<App>) -> Router {
    let r = Router::new();
    let r = serve::<at::Events, _, _>(r, events);
    let r = serve::<at::Permission, _, _>(r, permission_ask);
    let r = serve::<at::PermissionAnswer, _, _>(r, permission_state);
    let r = serve::<at::PermissionWithdraw, _, _>(r, permission_withdraw);
    let r = serve::<at::Next, _, _>(r, next);
    let r = serve::<at::Heard, _, _>(r, heard);
    let r = serve::<at::Presence, _, _>(r, presence);
    let r = serve::<at::Unheard, _, _>(r, unheard);
    let r = serve::<at::Ready, _, _>(r, ready);
    let r = serve::<at::Utterance, _, _>(r, utterance);
    let r = serve::<at::Feed, _, _>(r, feed);
    let r = serve::<at::Picture, _, _>(r, picture_part);
    let r = serve::<at::ReadsOffered, _, _>(r, reads_offered);
    let r = serve::<at::Reads, _, _>(r, feed::reads);
    let r = serve::<at::Message, _, _>(r, message);
    let r = serve::<at::Attachment, _, _>(r, attachment);
    let r = serve::<at::Accounts, _, _>(r, accounts_list);
    let r = serve::<at::UseAccount, _, _>(r, accounts_use);
    let r = serve::<at::Team, _, _>(r, team);
    let r = serve::<at::RenameTeam, _, _>(r, team_rename);
    let r = serve::<at::Folders, _, _>(r, folders);
    let r = serve::<at::Recent, _, _>(r, recent);
    let r = serve::<at::Project, _, _>(r, project);
    let r = serve::<at::TakeOver, _, _>(r, takeover);
    let r = serve::<at::Stop, _, _>(r, stop_agent);
    let r = serve::<at::Unsend, _, _>(r, unsend);
    let r = serve::<at::Dismiss, _, _>(r, dismiss);
    let r = serve::<at::Taken, _, _>(r, taken);
    let r = serve::<at::End, _, _>(r, end_agent);
    let r = serve::<at::Push, _, _>(r, push_target);
    let r = serve::<at::Tools, _, _>(r, uses_tools);
    // the command line's page about the tower: its type is the command line's own, not part of the wire
    r.route(super::STATUS_PATH, get(status))
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
        app_with(name, serde_json::json!({}))
    }

    /// A tower with nothing in it, with these settings beside its tokens.
    pub(in crate::tower) fn app_with(name: &str, settings: serde_json::Value) -> Arc<App> {
        app_on(name, settings, Arc::new(edges::procs::SystemProcs), None)
    }

    /// A tower with nothing in it whose processes and ways to agents are these stand-ins.
    pub(in crate::tower) fn app_on(name: &str, mut settings: serde_json::Value, procs: Arc<dyn edges::procs::Procs + Send>, deliverer: Option<Arc<dyn edges::deliver::Deliverer + Send>>) -> Arc<App> {
        settings["token"] = "own".into();
        settings["radio_token"] = "radio".into();
        let path = std::env::temp_dir().join(format!("airband-http-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(Store::open(&path).unwrap());
        let cfg: Config = serde_json::from_value(settings).unwrap();
        let push = push::Wake::load(&store, &[]).unwrap();
        let saver = saver::Saver::start(store.clone());
        Arc::new(App { push, settings: Settings::held(cfg), store, saver, inner: Mutex::new(Inner::default()), kick: Notify::new(), changed: tokio::sync::watch::channel(0).0, said: Mutex::new(HashMap::new()), accounts: accounts::Kept::default(), served: Default::default(), reviving: Default::default(), procs, deliverer, listening: Default::default() })
    }

    fn presence(radio: Option<RadioId>, header: Option<&str>) -> Request {
        let mut req = Request::builder().method("POST").uri("/v1/radio/presence").header("authorization", "Bearer radio").header(SCHEMA_HEADER, airband_core::radio::SCHEMA).header("content-type", "application/json");
        if let Some(h) = header {
            req = req.header(RADIO_HEADER, h);
        }
        if let Some(r) = radio {
            req = req.extension(r);
        }
        req.body(axum::body::Body::from(r#"{"on_call":true,"showing_chat":false}"#)).unwrap()
    }

    /// `airband status` reads the page as the type the tower writes it from, at the one path both use.
    #[tokio::test(flavor = "multi_thread")]
    async fn airband_status_reads_the_towers_page_as_its_type() {
        let app = app("status-page");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(axum::serve(listener, router(app.clone())).into_future());
        let cfg: Config = serde_json::from_value(serde_json::json!({ "token": "own", "radio_token": "radio", "port": port })).unwrap();
        let status = tokio::task::spawn_blocking(move || crate::adapters::TowerHere::here(&cfg, Duration::from_secs(5)).status()).await.unwrap();
        let status = status.expect("the page reads as a Status");
        assert!(!status.phone && status.sessions.is_empty());
        // with the wrong token the tower does not take it, and nothing is read
        let wrong: Config = serde_json::from_value(serde_json::json!({ "token": "not-it", "radio_token": "radio", "port": port })).unwrap();
        assert!(tokio::task::spawn_blocking(move || crate::adapters::TowerHere::here(&wrong, Duration::from_secs(5)).status()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_query_value_that_is_not_a_number_is_refused_not_read_as_none() {
        let app = app("query-numbers");
        let get = |uri: &str, token: &str| Request::builder().method("GET").uri(uri).header("authorization", format!("Bearer {token}")).header(SCHEMA_HEADER, airband_core::radio::SCHEMA).extension(RadioId("kitchen".into())).body(axum::body::Body::empty()).unwrap();
        let res = router(app.clone()).oneshot(get("/v1/radio/picture/abcd?from=middle", "radio")).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = router(app.clone()).oneshot(get("/v1/agent/permission/p1?wait=soon", "own")).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
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

    /// A request as a radio of this version makes it, with an empty object for a body.
    fn as_radio(method: wire::Method, path: &str) -> Request {
        let path = path.replace("{id}", "x");
        let req = Request::builder().method(method.as_str()).uri(path).header("authorization", "Bearer own").header(SCHEMA_HEADER, airband_core::radio::SCHEMA).header("content-type", "application/json").extension(RadioId("phone".into()));
        req.body(axum::body::Body::from(if method == wire::Method::Get { "" } else { "{}" })).unwrap()
    }

    #[tokio::test]
    async fn every_route_of_the_wire_is_served_by_its_method() {
        let app = app("every-route");
        // the routes that wait for something new wait with time paused
        tokio::time::pause();
        for (name, method, path) in at::ALL {
            // a route that is served answers: anything but "no such route" (404 with no body) or "not that method"
            let res = router(app.clone()).oneshot(as_radio(*method, path)).await.unwrap();
            let status = res.status();
            let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            assert!(status != StatusCode::METHOD_NOT_ALLOWED && !(status == StatusCode::NOT_FOUND && body.is_empty()), "{name} ({method} {path}) is not served: {status}");
        }
    }

    #[tokio::test]
    async fn a_refusal_is_a_4xx_carrying_the_refusal_and_never_an_error_in_a_200() {
        let app = app("refusals");
        let mut req = as_radio(wire::Method::Post, "/v1/radio/dismiss");
        *req.body_mut() = axum::body::Body::from(r#"{"message":"no such line"}"#);
        let res = router(app.clone()).oneshot(req).await.unwrap();
        assert_eq!(res.status().as_u16(), 409);
        let body: Refusal = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(body, Refusal::told("That message is gone."));
        // a body that is not the endpoint's is the request's fault, said as such
        let mut req = as_radio(wire::Method::Post, "/v1/radio/dismiss");
        *req.body_mut() = axum::body::Body::from(r#"{"messag":1}"#);
        let res = router(app.clone()).oneshot(req).await.unwrap();
        assert_eq!(res.status().as_u16(), 400);
        let body: Refusal = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert!(matches!(body, Refusal::BadAsk { .. }), "{body:?}");
        // what has nothing to say back says only that it was done
        let res = router(app.clone()).oneshot(as_radio(wire::Method::Post, "/v1/radio/ready")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ready: wire::ReadyAnswer = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(ready.waiting, 0);
    }

    #[tokio::test]
    async fn a_token_that_opens_nothing_is_told_as_not_paired_and_a_route_kept_per_radio_needs_one_named() {
        let app = app("not-paired");
        let req = Request::builder().method("POST").uri("/v1/radio/ready").header("authorization", "Bearer wrong").header(RADIO_HEADER, "phone").header(SCHEMA_HEADER, airband_core::radio::SCHEMA);
        let res = router(app.clone()).oneshot(req.body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body: Refusal = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(body, Refusal::NotPaired);
        // the tower's own token naming no radio, on a route kept per radio: a refusal, not a bare error
        let req = Request::builder().method("POST").uri("/v1/radio/ready").header("authorization", "Bearer own").header(SCHEMA_HEADER, airband_core::radio::SCHEMA);
        let res = router(app.clone()).oneshot(req.body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status().as_u16(), 403);
        let body: Refusal = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(body, Refusal::NotAllowed);
    }

    #[tokio::test]
    async fn a_radio_that_speaks_another_version_or_none_is_told_which_this_one_is() {
        let app = app("versions");
        for speaks in [Some("airband.radio/3"), None] {
            let mut req = Request::builder().method("POST").uri("/v1/radio/ready").header("authorization", "Bearer radio").header(RADIO_HEADER, "phone");
            if let Some(v) = speaks {
                req = req.header(SCHEMA_HEADER, v);
            }
            let res = router(app.clone()).oneshot(req.body(axum::body::Body::empty()).unwrap()).await.unwrap();
            assert_eq!(res.status().as_u16(), 426);
            let body: Refusal = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
            assert_eq!(body, Refusal::OtherVersion { speaks: airband_core::radio::SCHEMA.into() });
        }
        // the tower's own reporters are not radios: they speak the agent protocol, and are not asked this
        let req = Request::builder().method("DELETE").uri("/v1/agent/permission/p1").header("authorization", "Bearer own").body(axum::body::Body::empty()).unwrap();
        assert_eq!(router(app.clone()).oneshot(req).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn nothing_to_hand_over_is_null_in_a_200_not_an_empty_answer() {
        let app = app("next-null");
        // held open for nothing the whole time, with time paused
        tokio::time::pause();
        let res = router(app.clone()).oneshot(as_radio(wire::Method::Get, "/v1/radio/next")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let next: Option<Transmission> = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn a_feed_asked_with_a_number_that_is_not_one_is_refused_and_not_read_as_none() {
        let app = app("feed-query");
        let ask = |query: &str| Request::builder().method("GET").uri(format!("/v1/radio/feed?{query}")).header("authorization", "Bearer radio").header(SCHEMA_HEADER, airband_core::radio::SCHEMA).extension(RadioId("phone".into())).body(axum::body::Body::empty()).unwrap();
        assert_eq!(router(app.clone()).oneshot(ask("after=0&wait=0")).await.unwrap().status(), 200);
        assert_eq!(router(app.clone()).oneshot(ask("after=seven&wait=0")).await.unwrap().status(), 400);
    }

    #[tokio::test]
    async fn a_radio_is_told_only_the_addresses_the_tower_is_listening_on() {
        let app = app("feed-direct");
        app.listening.lock().unwrap().push(std::net::Ipv4Addr::new(10, 0, 0, 5));
        let req = Request::builder().method("GET").uri("/v1/radio/feed?after=0&wait=0").header("authorization", "Bearer radio").header(SCHEMA_HEADER, airband_core::radio::SCHEMA).extension(RadioId("phone".into())).body(axum::body::Body::empty()).unwrap();
        let res = router(app.clone()).oneshot(req).await.unwrap();
        let feed: Feed = serde_json::from_slice(&axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(feed.direct, [format!("http://10.0.0.5:{}", app.cfg().port)]);
    }
}
