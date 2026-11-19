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
            done(Some(format!("Not sent, no connection for half an hour: “{}”", next.text)));
            continue;
        }
        match link.utterance_once(&next.id, &next.text, next.re.clone(), next.scope.clone(), next.label.clone()) {
            Ok(reply) => {
                wait = 1;
                // "Sent to …" shows up as his own line in the chat; anything else he has to see
                done((!reply.say.is_empty() && !reply.say.starts_with("Sent to")).then_some(reply.say));
            }
            Err(LinkError::Refused) => done(Some(format!("The tower refused: “{}”", next.text))),
            Err(LinkError::Offline) => {
                thread::sleep(Duration::from_secs(wait));
                wait = (wait * 2).min(20);
            }
        }
    }
}
