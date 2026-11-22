//! What a radio sets on this computer: accounts, where to wake the phone, the coding tools he uses.
use super::*;
use axum::Extension;

/// The accounts the harnesses here can run under, and how full each is. `fresh=1` asks again now.
pub(super) async fn accounts_list(State(app): State<Arc<App>>, Query(q): Query<HashMap<String, String>>) -> Json<airband_core::radio::Accounts> {
    Json(accounts::list(&app, q.contains_key("fresh")).await)
}

/// Switch a harness's sessions to another account. Answers with the accounts as they now stand, or with
/// `{ "error": words for him }` when it could not be done.
pub(super) async fn accounts_use(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::UseAccount>) -> Json<Value> {
    Json(match accounts::switch(&app, to).await {
        Ok(now) => serde_json::to_value(now).unwrap_or_default(),
        Err(why) => refused(why),
    })
}

/// A radio says where it can be woken when it is not listening (its address at a push service), or takes that back.
pub(super) async fn push_target(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Json(target): Json<airband_core::radio::PushTarget>) -> Json<Value> {
    push::register(&app, &radio, target);
    Json(json!({}))
}

/// A radio says which coding tools he uses, the one he uses most first. Only tools this computer can start
/// are kept.
pub(super) async fn uses_tools(State(app): State<Arc<App>>, Json(said): Json<airband_core::radio::UsesTools>) -> Json<Value> {
    let known = app.cfg.tools();
    let tools: Vec<String> = said.tools.into_iter().filter(|t| known.contains(t)).collect();
    let mut inner = app.change();
    if inner.tools != tools {
        inner.tools = tools;
    }
    Json(json!({}))
}
