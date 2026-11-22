//! Call mode's queue: hand the radio one transmission at a time, what needs him first, and never what is routine.
use super::*;

pub(super) async fn pump_loop(app: Arc<App>) {
    loop {
        app.kick.notified().await;
        pump(&app).await;
    }
}

pub(super) async fn pump(app: &Arc<App>) {
    let todo: Vec<Item> = {
        let mut inner = app.change();
        if inner.free_radio().is_none() {
            return;
        }
        // what waits to be read out and has no short version yet, unless one is being made already
        let todo: Vec<Item> = inner.unheard().filter(|e| e.short.is_none() && !inner.judging.contains(&e.id)).take(6).map(|e| inner.item(e)).collect();
        todo.iter().for_each(|item| {
            inner.judging.insert(item.id.clone());
        });
        todo
    };
    // all of them, so a blocking question further back can jump the queue
    let mut tasks = tokio::task::JoinSet::new();
    for item in todo {
        let app = app.clone();
        tasks.spawn(async move {
            let j = prepare(&app.cfg, &item).await;
            (item.id, j)
        });
    }
    let done = tasks.join_all().await;

    let mut inner = app.change();
    for (id, j) in done {
        // what is not worth his ears leaves the queue here, and stays in the chat, dimmed
        inner.fill(&id, &j);
        // a radio on a call hears it; another that is neither on a call nor showing the chat is woken
        push::consider(app, &mut inner, &id);
    }
    // what was just read is written down when this is let go, whatever follows
    // read out under what its agent is called now, or was called when it said it, if it has gone
    let ready: Vec<Entry> = inner.log.iter().filter(|e| e.aloud == Aloud::Waiting && e.short.is_some()).cloned().collect();
    // one transmission goes to one radio: whichever is on a call and free
    let Some(radio) = inner.free_radio() else { return };
    let Some(next) = ready.iter().min_by_key(|e| (e.kind != Some(Kind::NeedsYou), e.at)) else {
        // a reminder is a matter of the call, which is not kept
        return remind(app, &mut inner, &radio);
    };
    let callsign = inner.sender_name(next);
    let say = next.line();
    let kind = next.kind.unwrap_or(Kind::Unjudged);
    log(format!("→ {radio}: {callsign} ({kind:?}): {say}"));
    let spoken_line = format!("{}. {say}", airband_core::prose::capital(&callsign));
    let waiting = inner.unheard().count() as u32 - 1;
    inner.set_aloud(&next.id, Aloud::Handed { radio: radio.clone(), at: now_ms() });
    transmit(app, &mut inner, &radio, Transmission { id: next.id.clone(), agent: next.agent.clone().unwrap_or_default(), callsign, kind, say: spoken_line, waiting });
}

/// Hand a radio its next transmission. If it takes it and never says it is ready for another, it is assumed
/// free again after a while, and what it had goes back in the queue, so one radio that went quiet cannot
/// hold the queue or lose anything from it.
fn transmit(app: &Arc<App>, inner: &mut Inner, radio: &str, t: Transmission) {
    let call = inner.call(radio);
    call.outbox = Some(t);
    call.handed = None;
    call.ready = false;
    call.epoch += 1;
    let (epoch, app, radio) = (call.epoch, app.clone(), radio.to_string());
    tokio::spawn(async move {
        tokio::time::sleep(READY_TIMEOUT).await;
        let mut inner = app.change();
        let call = inner.call(&radio);
        if call.epoch == epoch && !call.ready {
            log(format!("{radio} never said it was ready; carrying on"));
            call.ready = true;
            call.outbox = None;
            inner.give_back(&radio);
            inner.tell();
        }
    });
}

/// How long after he last heard an unanswered question it is put to him again: ten minutes, then half an hour,
/// then every hour.
fn remind_after(times_offered: u32) -> Duration {
    // for trying reminders without waiting ten minutes
    if let Some(secs) = std::env::var("AIRBAND_REMIND_SECONDS").ok().and_then(|s| s.parse().ok()) {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(match times_offered {
        0 | 1 => 600,
        2 => 1800,
        _ => 3600,
    })
}

/// Nothing new to say on the call. If someone has been waiting on him long enough, say so again.
fn remind(app: &Arc<App>, inner: &mut Inner, radio: &str) {
    let open = inner.open_ids();
    // reminders are only owed for what is still open; the rest is forgotten, so this never grows
    inner.offered.retain(|id, _| open.contains(id));
    // only what was asked since he joined: turning listening on does not play back the afternoon. What was
    // asked before is in the chat, still marked as waiting on him.
    let began = inner.call(radio).began;
    let due = inner.log.iter().find(|e| e.at >= began && open.contains(&e.id) && e.short.is_some() && inner.offered.get(&e.id).is_none_or(|(n, when)| when.elapsed() >= remind_after(*n))).cloned();
    let Some(e) = due else { return };
    let first = !inner.offered.contains_key(&e.id);
    let say = e.line();
    let from = inner.sender_name(&e);
    log(format!("→ {radio}: {from} ({}): {say}", if first { "waiting" } else { "reminder" }));
    let spoken_line = if first { format!("{} is waiting. {say}", airband_core::prose::capital(&from)) } else { format!("Still waiting: {from}. {say}") };
    transmit(app, inner, radio, Transmission { id: e.id.clone(), agent: e.agent.clone().unwrap_or_default(), callsign: from, kind: Kind::NeedsYou, say: spoken_line, waiting: 0 });
}

