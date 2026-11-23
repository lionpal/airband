//! Getting his typed messages to the tower over a connection that may be slow, dropping or absent.
use super::*;

/// How often the tower is asked again about something it has taken and not yet told the outcome of. The
/// outcome usually comes by the feed first; this is for when it does not (the tower restarted, the feed is behind).
const ASK_AGAIN: Duration = Duration::from_secs(15);
/// How often the same is asked about something left with the relay.
const ASK_AFTER_LEFT: Duration = Duration::from_secs(60);

/// Sends his typed messages in order, for as long as it takes. A slow or dropping connection (a plane, a
/// tunnel) only delays them; each carries an id, so one that got through but whose answer was lost is not
/// delivered twice when it is tried again.
///
/// A message is in the list until the tower says what came of it. The tower takes it at once and says so
/// (`Taken`); the outcome comes by the feed ([`settle`]) or to this asking again. The next message waits for
/// that, so his words reach his agents in the order he said them. One the relay is keeping for a computer
/// that is away (`Left`) holds up nothing: what he says next is left behind it.
pub(crate) fn deliver_outbox(link: Arc<Link>, session: Arc<Session>, store: Arc<Store>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, wake: Arc<Wake>) {
    let mut wait = 1;
    // when each message the tower has, or the relay keeps, is next asked after
    let mut ask_at: std::collections::HashMap<String, std::time::Instant> = Default::default();
    while !stop.load(Ordering::SeqCst) {
        // the turn first: something queued after the look below still ends the sleep
        let turn = wake.turn();
        let outbox = store.saved.lock().unwrap().outbox.clone();
        ask_at.retain(|id, _| outbox.iter().any(|o| o.id == *id));
        let now = std::time::Instant::now();
        let due = |o: &&Outgoing| ask_at.get(&o.id).is_none_or(|at| now >= *at);
        // the one whose turn it is: the oldest that is still being tried and is not with the relay. Failing
        // that, one the relay keeps that is due to be asked after.
        let in_turn = outbox.iter().find(|o| o.failed.is_none() && o.stage != Stage::Left);
        let Some(next) = in_turn.filter(due).or_else(|| outbox.iter().filter(|o| o.failed.is_none() && o.stage == Stage::Left).find(due)).cloned() else {
            // nothing to do now: sleep until something is queued or settled, or the next is due
            let soonest = ask_at.values().map(|at| at.saturating_duration_since(now)).min().unwrap_or(Duration::from_secs(30));
            wake.wait_since(turn, soonest.clamp(Duration::from_millis(200), Duration::from_secs(30)));
            continue;
        };
        // it did not go and is not tried any more: it stays in the list with the reason, for him to send
        // again or let go. He is told once, wherever he is looking.
        let gave_up = |why: String| fail(&store, &*host, &next.id, why);
        let staged = |stage: Stage| {
            store.change(|s| {
                if let Some(o) = s.outbox.iter_mut().find(|o| o.id == next.id) {
                    o.stage = stage;
                }
                s.outbox.clone()
            })
        };
        match next.stage {
            // with how the last try went, when there was one: "no connection" alone tells him nothing
            Stage::Trying if now_ms().saturating_sub(next.at) > OUTBOX_EXPIRY_MS => {
                gave_up(expired(&next.trail));
                continue;
            }
            Stage::Left if now_ms().saturating_sub(next.at) > airband_core::relay::HELD_FOR_MS => {
                gave_up("Your computer was away for more than a week, and the relay keeps a message that long.".into());
                continue;
            }
            _ => {}
        }
        let utterance = next.utterance();
        link.keeps_at_relay(outbox.iter().any(|o| o.stage == Stage::Left && o.failed.is_none() && o.id != next.id));
        let (got, steps) = link.utterance_once(&utterance);
        // It went straight to his computer while the relay still keeps earlier words of his for it: these
        // arrive first. Worth a line, for whoever wonders about the order.
        let kept = outbox.iter().filter(|o| o.stage == Stage::Left && o.failed.is_none() && o.id != next.id).count();
        if kept > 0 && got.is_ok() && steps.last().is_some_and(|s| s.way == Way::Near && s.outcome.is_ok()) {
            link.remark(format!("something he said went straight to his computer ahead of {kept} the relay still keeps for it"));
        }
        match got {
            Ok(Sent::Answered(reply)) => {
                wait = 1;
                session.asked(&reply);
                settle(&store, &*host, &next.id, airband_core::radio::Outcome::Answered { reply });
            }
            Ok(Sent::Taken) => {
                wait = 1;
                if next.stage != Stage::Taken {
                    host.outbox(staged(Stage::Taken), None);
                }
                ask_at.insert(next.id.clone(), now + ASK_AGAIN);
            }
            Ok(Sent::Left { away }) => {
                wait = 1;
                if next.stage != Stage::Left {
                    // said once, when it is first left: it will get there, and it has not yet
                    host.outbox(staged(Stage::Left), away.then(|| AWAY.to_string()));
                }
                ask_at.insert(next.id.clone(), now + ASK_AFTER_LEFT);
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
                match after_failure(e.retry(), next.stage) {
                    AfterFailure::GiveUp => {
                        wait = 1;
                        gave_up(e.said());
                    }
                    AfterFailure::AskAfter(then) => {
                        ask_at.insert(next.id.clone(), now + then);
                    }
                    AfterFailure::Soon => {
                        wake.wait_since(turn, Duration::from_secs(wait));
                        wait = (wait * 2).min(20);
                    }
                    AfterFailure::Later => wake.wait_since(turn, Duration::from_secs(60)),
                }
            }
        }
    }
}

/// What is done with a message after a try at it failed.
#[derive(Debug, PartialEq)]
enum AfterFailure {
    /// It ends here, and he is told: it holds up nothing behind it.
    GiveUp,
    /// His computer has it, or the relay does: it is asked after again then.
    AskAfter(Duration),
    /// Tried again in a moment, a little later each time.
    Soon,
    /// Tried again in a minute.
    Later,
}

/// By how the failure is to be retried and how far the message had got.
fn after_failure(retry: Retry, stage: Stage) -> AfterFailure {
    match (retry, stage) {
        // it can never be sent or asked after, however often it is tried: one his computer took and then
        // refuses for good (it no longer knows this phone, it says it is too old) ends too
        (Retry::Never, _) => AfterFailure::GiveUp,
        (_, Stage::Taken) => AfterFailure::AskAfter(ASK_AGAIN),
        (_, Stage::Left) => AfterFailure::AskAfter(ASK_AFTER_LEFT),
        (Retry::Soon, Stage::Trying) => AfterFailure::Soon,
        (Retry::Later, Stage::Trying) => AfterFailure::Later,
    }
}

/// It is not tried any more, and why: it stays in the list for him to send again or let go, and he is told.
fn fail(store: &Store, host: &dyn RadioHost, id: &str, why: String) {
    let (text, waiting) = store.change(|s| {
        // quoted as he saw it
        let text = s.outbox.iter_mut().find(|o| o.id == id).map(|o| {
            o.failed = Some(why.clone());
            o.shown()
        });
        (text, s.outbox.clone())
    });
    if let Some(text) = text {
        host.outbox(waiting, Some(format!("Not sent: “{}”. {why}", opening(&text))));
    }
}

/// The tower has said what came of something he said: by the feed, or to the outbox asking. A message that
/// is not in the list (it was settled already, or was never typed) is nothing to do.
pub(crate) fn settle(store: &Store, host: &dyn RadioHost, id: &str, outcome: airband_core::radio::Outcome) {
    use airband_core::radio::Outcome;
    let Some(was) = store.saved.lock().unwrap().outbox.iter().find(|o| o.id == id).cloned() else { return };
    let gone = || {
        store.change(|s| {
            s.outbox.retain(|o| o.id != id);
            s.outbox.clone()
        })
    };
    match outcome {
        Outcome::Taken => {}
        // not delivered: it is for an agent on another computer, or the tower cannot tell who it is for.
        // The host takes it from here; it has left this radio's list either way
        Outcome::Answered { reply } if reply.elsewhere.is_some() || !reply.ask.is_empty() => {
            host.outbox(gone(), None);
            // the agents it might be for, by id
            host.unplaced(was, reply.elsewhere, reply.ask);
        }
        // what was done as he asked shows as his own line in the chat; anything else he has to see
        Outcome::Answered { reply } => host.outbox(gone(), (!reply.say.is_empty() && !reply.done).then_some(reply.say)),
        Outcome::Rejected { why } => fail(store, host, id, LinkError::Rejected(why).said()),
    }
}

/// The start of what he said, for a note about it.
fn opening(text: &str) -> String {
    airband_core::prose::clip(text, 80)
}

/// How one try went, a sentence for each way it was tried that failed, without saying the same thing twice.
pub(crate) fn told(steps: &[Step]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for told in steps.iter().filter_map(|s| s.outcome.as_ref().err()).map(LinkError::said) {
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
    fn a_message_his_computer_took_and_then_refuses_for_good_ends_and_holds_up_nothing() {
        // (audit 8, M6)
        assert_eq!(after_failure(Retry::Never, Stage::Taken), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Never, Stage::Left), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Never, Stage::Trying), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Soon, Stage::Taken), AfterFailure::AskAfter(ASK_AGAIN));
        assert_eq!(after_failure(Retry::Later, Stage::Left), AfterFailure::AskAfter(ASK_AFTER_LEFT));
        assert_eq!(LinkError::Refused(airband_core::radio::wire::Refusal::NotPaired).retry(), Retry::Never);
    }

    #[test]
    fn a_quick_message_is_quoted_by_the_words_he_saw() {
        // (audit 7, P-N4)
        let mut o = Outgoing::new("a".into(), "Keep going with what you planned, and say when it is done.".into(), None, None, Some("Keep going".into()), Vec::new(), Vec::new());
        assert_eq!(o.shown(), "Keep going");
        o.label = None;
        assert!(o.shown().starts_with("Keep going with"));
    }

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
        fn unplaced(&self, message: Outgoing, _: Option<String>, ask: Vec<String>) {
            self.0.lock().unwrap().push((vec![message], Some(format!("unplaced: {}", ask.join(", ")))));
        }
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
        let second = radio.queue("the second thing".into(), None, None, None);
        assert!(until(&|o| o.len() == 2 && o[0].failed.is_some() && o[1].failed.is_none() && o[1].tries >= 1), "{:?}", outbox());
        // it is followed by the id it was queued under, not by being the last one waiting
        assert_eq!(outbox()[1].id, second);
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

    #[test]
    fn a_message_stays_until_the_tower_says_what_came_of_it() {
        use airband_core::radio::{Outcome, Rejected};
        let host = Watching::default();
        let store = Store::open("", None);
        let put = |id: &str, stage: Stage| {
            store.change(|s| s.outbox.push(Outgoing { stage, ..Outgoing::new(id.into(), format!("words of {id}"), None, None, None, Vec::new(), Vec::new()) }));
        };
        let ids = || store.saved.lock().unwrap().outbox.iter().map(|o| o.id.clone()).collect::<Vec<_>>();
        let last = || host.0.lock().unwrap().last().cloned().unwrap();
        for (id, stage) in [("a", Stage::Taken), ("b", Stage::Taken), ("c", Stage::Left), ("d", Stage::Taken), ("e", Stage::Left)] {
            put(id, stage);
        }
        // the tower has it and nothing more: it stays
        settle(&store, &host, "a", Outcome::Taken);
        assert_eq!(ids(), ["a", "b", "c", "d", "e"]);
        // done as he asked: it leaves the list, and there is nothing to tell him
        settle(&store, &host, "a", Outcome::Answered { reply: Reply { say: "Sent to shop two.".into(), done: true, ..Reply::default() } });
        assert_eq!((ids(), last().1), (vec!["b".to_string(), "c".into(), "d".into(), "e".into()], None));
        // not done as he asked: it leaves the list and he is told what the tower said
        settle(&store, &host, "b", Outcome::Answered { reply: Reply { say: "Nobody has stopped.".into(), ..Reply::default() } });
        assert_eq!(last().1.as_deref(), Some("Nobody has stopped."));
        // one the relay kept, answered when his computer came back: the same
        settle(&store, &host, "c", Outcome::Answered { reply: Reply { say: "Sent to shop two.".into(), done: true, ..Reply::default() } });
        assert_eq!(ids(), ["d", "e"]);
        // the tower cannot tell who it is for: it leaves this list and the host asks him
        settle(&store, &host, "d", Outcome::Answered { reply: Reply { ask: vec!["a2".into(), "a1".into()], ..Reply::default() } });
        assert_eq!((ids(), last().1.as_deref()), (vec!["e".to_string()], Some("unplaced: a2, a1")));
        // one his computer got too late: it stays, with the reason, to send again or let go, and he is told
        settle(&store, &host, "e", Outcome::Rejected { why: Rejected::TooOld });
        let kept = store.saved.lock().unwrap().outbox.clone();
        assert_eq!(kept[0].failed.as_deref(), Some("Your computer got it more than a week after you said it, and did nothing with it."));
        assert!(last().1.unwrap().starts_with("Not sent: “words of e”. Your computer got it more than a week"));
        // an outcome told twice, or for something this radio never typed, is nothing to do
        let before = host.0.lock().unwrap().len();
        settle(&store, &host, "a", Outcome::Answered { reply: Reply::default() });
        settle(&store, &host, "zz", Outcome::Rejected { why: Rejected::TooOld });
        assert_eq!(host.0.lock().unwrap().len(), before);
    }
}
