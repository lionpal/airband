//! The one route a radio uses to reach the tower without the relay: the sealed request it would have given the
//! relay, answered with the sealed answer. Whoever sends it needs the pairing key to make a request the tower
//! will open, and to read what comes back; nothing else is served here.
use super::super::App;
use axum::Router;
use axum::extract::State;
use std::sync::Arc;

pub(crate) fn sealed_router(app: Arc<App>) -> Router {
    Router::new().route(airband_core::relay::SEALED, axum::routing::post(sealed)).with_state(app)
}

async fn sealed(State(app): State<Arc<App>>, axum::extract::Path(channel): axum::extract::Path<String>, body: axum::body::Bytes) -> axum::response::Response {
    use axum::response::IntoResponse;
    let serve = app.served.get(&channel);
    let Some(serve) = serve else { return axum::http::StatusCode::NOT_FOUND.into_response() };
    match serve.answer_here(&body).await {
        Some(sealed) => sealed.into_response(),
        None => axum::http::StatusCode::BAD_REQUEST.into_response(),
    }
}
