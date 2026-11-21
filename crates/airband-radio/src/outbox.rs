//! Getting his typed messages to the tower over a connection that may be slow, dropping or absent.
use super::*;

/// Sends his typed messages in order, for as long as it takes. A slow or dropping connection (a plane, a
/// tunnel) only delays them; each carries an id, so one that got through but whose answer was lost is not
/// delivered twice when it is tried again.
pub(crate) fn deliver_outbox(link: Arc<Link>, store: Arc<Store>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, wake: Arc<Wake>) {
    let mut wait = 1;
    while !stop.load(Ordering::SeqCst) {
        // the oldest that is still being tried: one given up on stays in the list for him and holds up nothing
        let Some(next) = store.saved.lock().unwrap().outbox.iter().find(|o| o.failed.is_none()).cloned() else {
            // nothing to send: sleep until something is queued
            wake.wait(Duration::from_secs(30));
            continue;
        };
        // it went, or went elsewhere: it leaves the list
        let done = |note: Option<String>| {
            let waiting = store.change(|s| {
                s.outbox.retain(|o| o.id != next.id);
                s.outbox.clone()
            });
            host.outbox(waiting, note);
        };
        // it did not go and is not tried any more: it stays in the list with the reason, for him to send
        // again or let go. He is told once, wherever he is looking.
        let gave_up = |why: String| {
            let waiting = store.change(|s| {
                if let Some(o) = s.outbox.iter_mut().find(|o| o.id == next.id) {
                    o.failed = Some(why.clone());
                }
                s.outbox.clone()
            });
            host.outbox(waiting, Some(format!("Not sent: “{}”. {why}", opening(&next.text))));
        };
        if now_ms().saturating_sub(next.at) > OUTBOX_EXPIRY_MS {
            // with how the last try went, when there was one: "no connection" alone tells him nothing
            gave_up(expired(&next.trail));
            continue;
        }
        let (got, steps) = link.utterance_once(&next.id, &next.text, next.re.clone(), next.scope.clone(), next.label.clone(), next.attachments.clone(), &next.others);
        match got {
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
            Err(e) => {
                // how this try went, each way it was tried, stays with the message for him to see
                let trail = told(&steps);
                let waiting = store.change(|s| {
                    if let Some(o) = s.outbox.iter_mut().find(|o| o.id == next.id) {
                        (o.tries, o.last_tried, o.trail) = (o.tries + 1, now_ms(), trail.clone());
                    }
                    s.outbox.clone()
                });
                host.outbox(waiting, None);
                match (e.retry(), &e) {
                    // it cannot be sent however often it is tried
                    (Retry::Never, _) => {
                        wait = 1;
                        gave_up(sentence(&e.told()));
                    }
                    // his computer has it and is still answering: ask again shortly, as often as it takes
                    (_, LinkError::Working) => wake.wait(Duration::from_secs(2)),
                    (Retry::Soon, _) => {
                        wake.wait(Duration::from_secs(wait));
                        wait = (wait * 2).min(20);
                    }
                    (Retry::Later, _) => wake.wait(Duration::from_secs(60)),
                }
            }
        }
    }
}

/// The start of what he said, for a note about it.
fn opening(text: &str) -> String {
    if text.chars().count() > 80 { format!("{}…", text.chars().take(79).collect::<String>().trim_end()) } else { text.to_string() }
}

/// A cause as a sentence of its own.
fn sentence(told: &str) -> String {
    let mut c = told.chars();
    c.next().map(|f| format!("{}{}.", f.to_uppercase(), c.as_str())).unwrap_or_default()
}

/// How one try went, a sentence for each way it was tried that failed, without saying the same thing twice.
pub(crate) fn told(steps: &[Step]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for told in steps.iter().filter_map(|s| s.outcome.as_ref().err()).map(|e| sentence(&e.told())) {
        if !out.contains(&told) {
            out.push(told);
        }
    }
    out
}

/// What he is told of a message that half an hour of trying did not send.
fn expired(trail: &[String]) -> String {
    match trail {
        [] => "Half an hour of trying did not get it through.".into(),
        last => format!("Half an hour of trying did not get it through. {}", last.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_try_is_told_a_sentence_for_each_way_it_failed() {
        let step = |way, outcome| Step { at: 0, what: "POST /v1/radio/utterance".into(), way, outcome, ms: 3 };
        let steps = [step(Way::Relay, Err(LinkError::Away)), step(Way::Near, Err(LinkError::Unreachable(Way::Near))), step(Way::Kept, Err(LinkError::HoldRefused(507))), step(Way::Kept, Err(LinkError::HoldRefused(507))), step(Way::Near, Ok(200))];
        assert_eq!(told(&steps), ["The relay says your computer is not connected.", "Your computer did not answer on this network.", "The relay is keeping as many messages as it can for your computer."]);
        assert_eq!(expired(&[]), "Half an hour of trying did not get it through.");
        assert_eq!(expired(&told(&steps[..1])), "Half an hour of trying did not get it through. The relay says your computer is not connected.");
    }

    /// A host that only keeps what it is told about the outbox.
    #[derive(Default)]
    struct Watching(Mutex<Vec<(Vec<Outgoing>, Option<String>)>>);
    impl RadioHost for Watching {
        fn told(&self, _: String) {}
        fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
            self.0.lock().unwrap().push((waiting, note));
        }
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn own_voice(&self) -> bool {
            false
        }
        fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
        fn speak(&self, _: String) {}
        fn listen(&self) -> Option<String> {
            None
        }
        fn status(&self, _: RadioStatus) {}
        fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
        fn record_start(&self, _: u32) -> bool {
            false
        }
        fn record_read(&self) -> Vec<u8> {
            Vec::new()
        }
        fn record_stop(&self) {}
        fn hearing(&self, _: String) {}
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn a_message_that_cannot_go_is_kept_with_its_reason_and_holds_up_nothing() {
        let host = Arc::new(Watching::default());
        // a relay nobody is at
        let radio = Radio::start(format!("airband://pair?relay=http%3A%2F%2F127.0.0.1%3A9&secret={}", "ab".repeat(32)), String::new(), host.clone()).unwrap();
        let outbox = || radio.store.saved.lock().unwrap().outbox.clone();
        let until = |what: &dyn Fn(&[Outgoing]) -> bool| {
            for _ in 0..200 {
                if what(&outbox()) {
                    return true;
                }
                thread::sleep(Duration::from_millis(50));
            }
            false
        };
        radio.queue("the first thing".into(), None, None, None);
        // tried, and how the try went is with the message
        assert!(until(&|o| o.first().is_some_and(|m| m.tries >= 1 && m.last_tried > 0 && m.trail == ["There was no connection to the relay."])), "{:?}", outbox());
        // half an hour on, it is given up on: it stays, with the reason, and he is told once
        radio.store.change(|s| s.outbox[0].at = 1);
        radio.wake.ring();
        assert!(until(&|o| o[0].failed.as_deref() == Some("Half an hour of trying did not get it through. There was no connection to the relay.")), "{:?}", outbox());
        let notes: Vec<String> = host.0.lock().unwrap().iter().filter_map(|(_, note)| note.clone()).collect();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("Not sent: “the first thing”. Half an hour"), "{notes:?}");
        // what he says next is tried all the same
        radio.queue("the second thing".into(), None, None, None);
        assert!(until(&|o| o.len() == 2 && o[0].failed.is_some() && o[1].failed.is_none() && o[1].tries >= 1), "{:?}", outbox());
        // sent again, the first goes to the end as if just said
        let first = outbox()[0].id.clone();
        radio.send_again(&first);
        assert!(until(&|o| o.len() == 2 && o[1].id == first && o[1].failed.is_none() && o[1].at > 1), "{:?}", outbox());
        // and one that was given up on can be let go, which hands back its words
        radio.store.change(|s| s.outbox[0].at = 1);
        radio.wake.ring();
        assert!(until(&|o| o[0].failed.is_some()));
        let second = outbox()[0].id.clone();
        assert_eq!(radio.discard(&second).map(|o| o.text), Some("the second thing".into()));
        assert!(outbox().iter().all(|o| o.id != second));
        // one still being tried is not let go that way
        assert!(radio.discard(&first).is_none());
        radio.stop();
    }
}
