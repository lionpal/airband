//! What a radio sets on this computer: accounts, where to wake the phone, the coding tools he uses.
use super::*;
use axum::Extension;

/// The accounts the harnesses here can run under, and how full each is. `fresh=true` asks again now.
pub(super) async fn accounts_list(State(app): State<Arc<App>>, Ask(q): Ask<at::Accounts>) -> Answer<at::Accounts> {
    Ok(Json(accounts::list(&app, q.fresh).await))
}

/// Switch a harness's sessions to another account. Answers with the accounts as they now stand, or why not.
pub(super) async fn accounts_use(State(app): State<Arc<App>>, Sent(to): Sent<at::UseAccount>) -> Answer<at::UseAccount> {
    accounts::switch(&app, to).await.map(Json).map_err(no)
}

/// A radio says where it can be woken when it is not listening (its address at a push service), or takes that back.
pub(super) async fn push_target(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Sent(target): Sent<at::Push>) -> Answer<at::Push> {
    push::register(&app, &radio, target);
    done()
}

/// A radio says which coding tools he uses, the one he uses most first. Only tools this computer can start
/// are kept.
/// The phone sends the same list to each of his computers, so a tool one of them does not have is no mistake.
pub(super) async fn uses_tools(State(app): State<Arc<App>>, Sent(said): Sent<at::Tools>) -> Answer<at::Tools> {
    let known = app.cfg.tools();
    let tools: Vec<String> = said.tools.into_iter().filter(|t| known.contains(t)).collect();
    let mut inner = app.change();
    if inner.tools != tools {
        inner.tools = tools;
    }
    done()
}
