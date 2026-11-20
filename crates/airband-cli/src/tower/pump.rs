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
        let inner = app.inner.lock().unwrap();
        if inner.free_radio().is_none() {
            return;
        }
        let mut todo: Vec<Item> = inner.pending.iter().filter(|p| p.say.is_none()).take(6).cloned().collect();
        for item in todo.iter_mut() {
            item.asked = inner.last_instruction(&item.agent_id);
        }
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

    let mut inner = app.inner.lock().unwrap();
    for (id, j) in done {
        inner.fill(&id, j.kind, &j.say, &j.options, j.recommended, j.read.as_deref(), j.background);
        // a radio on a call hears it; another that is neither on a call nor showing the chat is woken
        push::consider(app, &mut inner, &id);
    }
    // not worth his ears: it stays in the chat log, dimmed, and leaves the call queue
    inner.pending.retain(|p| p.kind != Some(Kind::Fyi) || p.say.is_none());
    // start the voice on everything that will be spoken, now, so each clip exists before its turn comes
    for p in inner.pending.iter().filter(|p| p.say.is_some() && inner.wants_voice()) {
        if let Some(callsign) = inner.find(&p.agent_id).map(|s| s.callsign.spoken()) {
            app.speech.start(&app.voice(), &format!("{callsign}. {}", p.say.as_deref().unwrap_or_default()));
        }
    }
    // one transmission goes to one radio: whichever is on a call and free
    let Some(radio) = inner.free_radio() else { return };
    let Some(next) = inner.pending.iter().filter(|p| p.say.is_some()).min_by_key(|p| (p.kind != Some(Kind::NeedsYou), p.created)).cloned() else {
        return remind(app, &mut inner, &radio);
    };
    let Some(callsign) = inner.find(&next.agent_id).map(|s| s.callsign.clone()) else {
        inner.pending.retain(|p| p.id != next.id);
        return;
    };
    let say = next.say.unwrap_or_default();
    let kind = next.kind.unwrap_or(Kind::Report);
    log(format!("→ {radio}: {} ({kind:?}): {say}", callsign.text()));
    let spoken_line = format!("{}. {say}", callsign.spoken());
    let waiting = inner.pending.len() as u32 - 1;
    let speech = app.speech.start(&app.voice_to(inner.call(&radio).speaks), &spoken_line);
    transmit(app, &mut inner, &radio, Transmission { id: next.id, callsign: callsign.text(), kind, speech, say: spoken_line, waiting });
    app.save(&inner);
}

/// Hand a radio its next transmission. If it takes it and never says it is ready for another, it is assumed
/// free again after a while, so one radio that went quiet cannot hold the queue.
fn transmit(app: &Arc<App>, inner: &mut Inner, radio: &str, t: Transmission) {
    let call = inner.call(radio);
    call.outbox = Some(t);
    call.ready = false;
    call.epoch += 1;
    let (epoch, app, radio) = (call.epoch, app.clone(), radio.to_string());
    tokio::spawn(async move {
        tokio::time::sleep(READY_TIMEOUT).await;
        let mut inner = app.inner.lock().unwrap();
        let call = inner.call(&radio);
        if call.epoch == epoch && !call.ready {
            log(format!("{radio} never said it was ready; carrying on"));
            call.ready = true;
            call.outbox = None;
            app.kick.notify_one();
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
    let say = e.short.clone().unwrap_or_default();
    let from = inner.sender(&e).map(|s| s.callsign.text()).unwrap_or_else(|| e.from.clone());
    log(format!("→ {radio}: {from} ({}): {say}", if first { "waiting" } else { "reminder" }));
    let spoken_line = if first { format!("{} is waiting. {say}", cap(&from)) } else { format!("Still waiting: {from}. {say}") };
    let speech = app.speech.start(&app.voice_to(inner.call(radio).speaks), &spoken_line);
    transmit(app, inner, radio, Transmission { id: e.id.clone(), callsign: from, kind: Kind::NeedsYou, speech, say: spoken_line, waiting: 0 });
}

fn cap(t: &str) -> String {
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
