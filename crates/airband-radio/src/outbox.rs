//! Getting his typed messages to the tower over a connection that may be slow, dropping or absent.
use super::*;

/// Sends his typed messages in order, for as long as it takes. A slow or dropping connection (a plane, a
/// tunnel) only delays them; each carries an id, so one that got through but whose answer was lost is not
/// delivered twice when it is tried again.
pub(crate) fn deliver_outbox(link: Arc<Link>, store: Arc<Store>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, wake: Arc<Wake>) {
    let mut wait = 1;
    while !stop.load(Ordering::SeqCst) {
        let Some(next) = store.saved.lock().unwrap().outbox.first().cloned() else {
            // nothing to send: sleep until something is queued
            wake.wait(Duration::from_secs(30));
            continue;
        };
        let done = |note: Option<String>| {
            let waiting = store.change(|s| {
                s.outbox.retain(|o| o.id != next.id);
                s.outbox.clone()
            });
            host.outbox(waiting, note);
        };
        if now_ms().saturating_sub(next.at) > OUTBOX_EXPIRY_MS {
            // with what went wrong each time it was tried, when that is known: "no connection" alone tells him nothing
            let why = link.trouble();
            done(Some(if why.is_empty() { format!("Not sent after half an hour of trying: “{}”", next.text) } else { format!("Not sent after half an hour of trying ({why}): “{}”", next.text) }));
            continue;
        }
        match link.utterance_once(&next.id, &next.text, next.re.clone(), next.scope.clone(), next.label.clone(), next.attachments.clone(), &next.others) {
            // not delivered: it is for an agent on another computer, or the tower cannot tell who it is for.
            // The host takes it from here; it has left this radio's list either way
            Ok(reply) if reply.elsewhere.is_some() || !reply.ask.is_empty() => {
                wait = 1;
                done(None);
                // names as the screen shows them
                host.unplaced(next.clone(), reply.elsewhere.map(|c| airband_core::routing::shown(&c)), reply.ask.iter().map(|c| airband_core::routing::shown(c)).collect());
            }
            Ok(reply) => {
                wait = 1;
                // what was done as he asked shows as his own line in the chat; anything else he has to see
                done((!reply.say.is_empty() && !reply.done).then_some(reply.say));
            }
            Err(LinkError::Refused) => done(Some(format!("The tower refused: “{}”", next.text))),
            // no network, or the relay could not keep it either: it stays here and is tried again
            Err(LinkError::Offline | LinkError::Away | LinkError::Behind) => {
                thread::sleep(Duration::from_secs(wait));
                wait = (wait * 2).min(20);
            }
        }
    }
}
