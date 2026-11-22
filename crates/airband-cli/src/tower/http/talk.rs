//! Talking: what to say next, what he heard and said, and judging what agents said for him.
use super::*;
use axum::Extension;

/// The next transmission for this radio, held open up to [`HOLD`]: none (`null`) when nothing came.
pub(super) async fn next(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Ask(q): Ask<at::Next>) -> Answer<at::Next> {
    {
        let mut inner = app.change();
        // He has just turned reading aloud on, and the radio says so (`joined`). What was said long before is
        // in the chat for him to read: it is not read out now, hours late. A radio that only went quiet for a
        // while and is back says no such thing, and nothing is taken from the queue for it.
        if q.joined {
            inner.leave_old_news();
            inner.call(&radio).began = now_ms();
        }
        // The radio says which transmission it last got (`have`). If the tower gave it another since and has
        // heard nothing about it, the answer that carried it was lost on the way: it is given again now, not
        // after two minutes of silence.
        let call = inner.call(&radio);
        if let (Some(have), Some(lost)) = (q.have.as_ref(), call.handed.clone())
            && *have != lost.id && call.outbox.is_none() && !call.ready {
                log(format!("{radio} never got what it was handed ({}); handing it over again", lost.callsign));
                call.outbox = Some(lost);
            }
    }
    app.kick.notify_one();
    // tokio's clock, so the hold can be run with time paused
    let until = tokio::time::Instant::now() + HOLD;
    while tokio::time::Instant::now() < until {
        {
            let mut inner = app.change();
        let call = inner.call(&radio);
            if let Some(t) = call.outbox.take() {
                call.handed = Some(t.clone());
                return Ok(Json(Some(t)));
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(Json(None))
}

/// A radio was handed something to read out and no sound came of it. It goes back in the queue, the reason is
/// written down, and that radio is handed nothing more for a minute.
pub(super) async fn unheard(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Sent(u): Sent<at::Unheard>) -> Answer<at::Unheard> {
    let mut inner = app.change();
    let id = u.id.as_str();
    let who = inner.log.iter().find(|e| e.id == id).map(|e| inner.sender_name(e)).unwrap_or_else(|| "a line it no longer has".to_string());
    log(format!("{radio} could not read {who} aloud ({}); it stays queued (line {id}), and nothing more goes to that radio for a minute", u.why));
    if inner.log.iter().any(|e| e.id == id && matches!(&e.aloud, Aloud::Handed { radio: r, .. } if *r == radio)) {
        inner.set_aloud(id, Aloud::Waiting);
    }
    let call = inner.call(&radio);
    call.quiet_until = Some(Instant::now() + Duration::from_secs(60));
    call.ready = true;
    call.outbox = None;
    call.handed = None;
    done()
}

/// A radio says where it stands, when that changes and while it has nothing else to ask (he has the
/// microphone open on a call): so the tower does not take it for gone.
pub(super) async fn presence(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Sent(said): Sent<at::Presence>) -> Answer<at::Presence> {
    let name = super::push::radio_name(&app.cfg(), &radio);
    app.change().radio_named_says(&radio, name, said);
    app.kick.notify_one();
    done()
}

pub(super) async fn heard(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, Sent(h): Sent<at::Heard>) -> Answer<at::Heard> {
    let mut inner = app.change();
    // remember when he was last told, so an unanswered question comes round again later and not at once
    let times = inner.offered.get(&h.id).map(|(n, _)| *n).unwrap_or(0);
    inner.offered.insert(h.id.clone(), (times + 1, Instant::now()));
    let call = inner.call(&radio);
    if call.handed.as_ref().is_some_and(|t| t.id == h.id) {
        call.handed = None;
    }
    // a reminder of a question he has heard before is not in the queue; what was is now heard
    if let Some(agent_id) = inner.log.iter().find(|e| e.id == h.id && e.aloud.unheard()).map(|e| e.agent.clone().unwrap_or_default()) {
        inner.set_aloud(&h.id, Aloud::Heard { radio: radio.clone(), at: now_ms() });
        if let Some(s) = inner.session(&agent_id) {
            s.last_heard = Some(h.id.clone());
            inner.call(&radio).focus = Some(agent_id);
        }
    }
    done()
}

pub(super) async fn ready(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>) -> Answer<at::Ready> {
    let waiting = {
        let mut inner = app.change();
        let call = inner.call(&radio);
        call.ready = true;
        // ready for the next with no word about the last (he turned reading off part way, or its own voice
        // failed): the last is still to be heard
        inner.give_back(&radio);
        inner.unheard().count()
    };
    app.kick.notify_one();
    Ok(Json(wire::ReadyAnswer { waiting }))
}

/// An utterance that takes longer than this to answer is written down as slow.
pub(super) const SLOW: Duration = Duration::from_secs(10);

/// Something he said. It is answered at once: the tower has it, under its id. Working out who it is for and
/// handing it over can take a model a while, and no request waits on that: what comes of it is told in the
/// feed, and to anyone who asks again under the same id.
pub(super) async fn utterance(State(app): State<Arc<App>>, Extension(RadioId(radio)): Extension<RadioId>, headers: HeaderMap, Sent(u): Sent<at::Utterance>) -> Answer<at::Utterance> {
    use airband_core::radio::Outcome;
    // how it came: through the relay, straight over his own network, kept by the relay while the tower was
    // away, or from this computer
    let via = headers.get(VIA_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("this computer").to_string();
    if u.id.is_empty() {
        return Err(bad("what he said has to come with an id"));
    }
    {
        // the id is taken before anything is done, under the one lock, so the same utterance arriving twice
        // (a retry, or two ways at once) is taken once, whether or not the tower restarted in between
        let mut inner = app.change();
        if inner.answering.contains(&u.id) {
            log(format!("utterance {} from {radio} by {via}: asked after, still being answered", u.id));
            return Ok(Json(Outcome::Taken));
        }
        match app.store.begin_receipt(&u.id) {
            Receipt::Done(reply) => {
                log(format!("utterance {} from {radio} by {via}: already answered, the same answer given again", u.id));
                return Ok(Json(Outcome::Answered { reply }));
            }
            Receipt::Begun => {
                inner.answering.insert(u.id.clone());
            }
            // the radio keeps it and sends it again: answered now, it might be answered twice
            Receipt::Unknown(why) => {
                log(format!("utterance {} from {radio} by {via}: whether it was answered could not be read ({why})", u.id));
                return Err(No(Refusal::NotNow { reason: "This computer could not check its records just now. It is sent again in a moment.".into() }));
            }
        }
    }
    // answered in the order it came from this radio: several can arrive in one moment (what the relay kept
    // while the tower was away), and each is finished before the next is begun
    let line = app.said.lock().unwrap().entry(radio.clone()).or_insert_with(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>();
        tokio::spawn(async move {
            while let Some(next) = rx.recv().await {
                next.await;
            }
        });
        tx
    }).clone();
    let app_ = app.clone();
    let queued = line.send(Box::pin(async move {
        let app = app_;
        let began = Instant::now();
        // files with no words are still something sent
        let text = match (u.text.trim(), u.attachments.is_empty()) {
            ("", true) => None,
            (said, _) => Some(said),
        };
        let reply = match text {
            Some(text) => on_utterance(&app, &radio, text, &u).await,
            None => Reply::default(),
        };
        // one line for everything he says: its id (the same on the phone and at the relay), how it came, what
        // became of it and how long that took
        let took = began.elapsed();
        log(format!("utterance {} from {radio} by {via}: answered in {:.1} s{}{}", u.id, took.as_secs_f32(), if reply.say.is_empty() { String::new() } else { format!(" (\"{}\")", airband_core::prose::clip(&reply.say, 80)) }, if took > SLOW { ", which is slow" } else { "" }));
        // the line is written before the answer can be read back, so it is there whenever the answer is
        app.store.finish_receipt(&u.id, &reply);
        let mut inner = app.change();
        inner.answering.remove(&u.id);
        inner.settle(&radio, &u.id, Outcome::Answered { reply });
    }));
    // a line whose worker has gone takes nothing: he is told to send it again, never told it was taken
    if queued.is_err() {
        return Err(No(Refusal::NotNow { reason: "This computer could not take that just now. It is sent again in a moment.".into() }));
    }
    Ok(Json(Outcome::Taken))
}
