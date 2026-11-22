//! What a radio does to one agent: take it over, stop it, end it, let a question go, answer a request.
use super::*;

/// He asked the tower to take over an agent he started in a terminal.
pub(super) async fn takeover(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::TakeOver>) -> Json<Value> {
    Json(match held::take_over(&app, &to.agent).await {
        Ok(()) => json!({}),
        Err(why) => refused(why),
    })
}

/// He stopped what an agent is doing.
pub(super) async fn stop_agent(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::StopAgent>) -> Json<Value> {
    Json(match held::stop(&app, &to.agent).await {
        Ok(()) => json!({}),
        Err(why) => refused(why),
    })
}

/// He ended an agent the tower runs.
pub(super) async fn end_agent(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::EndAgent>) -> Json<Value> {
    let reached = app.read().find(&to.agent).map(|s| s.agent.deliver.clone());
    Json(match reached {
        None => refused("That agent is no longer running."),
        Some(Some(Deliver::Held)) if held::end(&app, &to.agent).await => json!({}),
        Some(Some(Deliver::Held)) => refused("It had already ended."),
        // a session of a tool's own is put away by that tool; the tower then sees it is no longer open
        Some(Some(Deliver::Harness { name, session })) => match tokio::task::spawn_blocking(move || crate::adapters::put_away(&name, &session)).await {
            Ok(Ok(())) => json!({}),
            Ok(Err(why)) => refused(why),
            Err(_) => refused("That could not be done."),
        },
        Some(_) => refused("Only an agent airband runs can be ended from here."),
    })
}

/// A radio says which names his other computers use.
pub(super) async fn taken(State(app): State<Arc<App>>, Json(t): Json<airband_core::radio::Taken>) -> Json<Value> {
    let mut inner = app.change();
    if inner.taken_elsewhere(&t.callsigns) {
        inner.tell();
    }
    Json(json!({}))
}

/// He let a question go without answering it.
pub(super) async fn dismiss(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::Dismiss>) -> Json<Value> {
    let mut inner = app.change();
    let Some(e) = inner.log.iter_mut().find(|e| e.id == to.message) else { return Json(refused("That message is gone.")) };
    e.held = Some(Held::Dismissed);
    // it is not read out either
    if e.aloud.unheard() {
        inner.set_aloud(&to.message, Aloud::dropped(Dropped::Dismissed));
    }
    inner.touch(&to.message);
    inner.tell();
    Json(json!({}))
}

/// He took back something he said that was still waiting its turn.
pub(super) async fn unsend(State(app): State<Arc<App>>, Json(to): Json<airband_core::radio::Unsend>) -> Json<Value> {
    Json(match held::unsend(&app, &to.message) {
        Ok(()) => json!({}),
        Err(why) => refused(why),
    })
}

/// End an agent the tower runs itself (`airband end <id>`).
pub(super) async fn held_end(State(app): State<Arc<App>>, Path(id): Path<String>) -> StatusCode {
    if held::end(&app, &id).await { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

pub(super) async fn permission_ask(State(app): State<Arc<App>>, Json(ask): Json<airband_core::agent::PermissionAsk>) -> Json<airband_core::agent::PermissionState> {
    Json(permit::ask(&app, ask))
}

/// How many seconds to wait for his answer: none when it is not given. A value that is not a number is
/// refused, never read as none.
#[derive(Deserialize)]
pub(super) struct WaitQuery {
    #[serde(default)]
    wait: u64,
}

pub(super) async fn permission_state(State(app): State<Arc<App>>, Path(id): Path<String>, Query(WaitQuery { wait }): Query<WaitQuery>) -> Json<airband_core::agent::PermissionState> {
    Json(permit::state(&app, &id, wait).await)
}

pub(super) async fn permission_withdraw(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    permit::withdraw(&app, &id);
    Json(json!({}))
}
