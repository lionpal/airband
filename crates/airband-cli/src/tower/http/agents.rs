//! What a radio does to one agent: take it over, stop it, end it, let a question go, answer a request.
use super::*;

/// He asked the tower to take over an agent he started in a terminal.
pub(super) async fn takeover(State(app): State<Arc<App>>, Sent(to): Sent<at::TakeOver>) -> Answer<at::TakeOver> {
    held::take_over(&app, &to.agent).await.map_err(no)?;
    done()
}

/// He stopped what an agent is doing.
pub(super) async fn stop_agent(State(app): State<Arc<App>>, Sent(to): Sent<at::Stop>) -> Answer<at::Stop> {
    held::stop(&app, &to.agent).await.map_err(no)?;
    done()
}

/// He ended an agent the tower runs.
pub(super) async fn end_agent(State(app): State<Arc<App>>, Sent(to): Sent<at::End>) -> Answer<at::End> {
    let reached = app.read().find(&to.agent).map(|s| s.agent.deliver.clone());
    match reached {
        None => Err(no(NOT_RUNNING)),
        Some(Some(Deliver::Held)) if held::end(&app, &to.agent).await => done(),
        Some(Some(Deliver::Held)) => Err(no("It had already ended.")),
        // a session of a tool's own is put away by that tool; the tower then sees it is no longer open
        Some(Some(Deliver::Harness { name, session })) => match tokio::task::spawn_blocking(move || crate::adapters::put_away(&name, &session)).await {
            Ok(Ok(())) => done(),
            Ok(Err(why)) => Err(no(why)),
            Err(_) => Err(no("That could not be done.")),
        },
        Some(_) => Err(no("Only an agent airband runs can be ended from here.")),
    }
}

/// A radio says which names his other computers use.
pub(super) async fn taken(State(app): State<Arc<App>>, Sent(t): Sent<at::Taken>) -> Answer<at::Taken> {
    let mut inner = app.change();
    if inner.taken_elsewhere(&t.callsigns) {
        inner.tell();
    }
    done()
}

/// He let a question go without answering it.
pub(super) async fn dismiss(State(app): State<Arc<App>>, Sent(to): Sent<at::Dismiss>) -> Answer<at::Dismiss> {
    let mut inner = app.change();
    let Some(e) = inner.log.iter_mut().find(|e| e.id == to.message) else { return Err(no("That message is gone.")) };
    e.held = Some(Held::Dismissed);
    // it is not read out either
    if e.aloud.unheard() {
        inner.set_aloud(&to.message, Aloud::dropped(Dropped::Dismissed));
    }
    inner.touch(&to.message);
    inner.tell();
    done()
}

/// He took back something he said that was still waiting its turn.
pub(super) async fn unsend(State(app): State<Arc<App>>, Sent(to): Sent<at::Unsend>) -> Answer<at::Unsend> {
    held::unsend(&app, &to.message).map_err(no)?;
    done()
}

pub(super) async fn permission_ask(State(app): State<Arc<App>>, Sent(ask): Sent<at::Permission>) -> Answer<at::Permission> {
    Ok(Json(permit::ask(&app, ask)))
}

/// His answer to a request, waiting up to `wait` seconds for it. A value that is not a number is refused,
/// never read as none.
pub(super) async fn permission_state(State(app): State<Arc<App>>, Path(id): Path<String>, Ask(wire::WaitQuery { wait }): Ask<at::PermissionAnswer>) -> Answer<at::PermissionAnswer> {
    Ok(Json(permit::state(&app, &id, wait).await))
}

pub(super) async fn permission_withdraw(State(app): State<Arc<App>>, Path(id): Path<String>) -> Answer<at::PermissionWithdraw> {
    permit::withdraw(&app, &id);
    done()
}
