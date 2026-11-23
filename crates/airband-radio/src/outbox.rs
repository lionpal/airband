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
///
/// What he said aloud goes under the id of its recording: when his computer takes it, the recording is deleted
/// ([`crate::takes::Takes::taken`]), and not before.
pub(crate) fn deliver_outbox(link: Arc<Link>, session: Arc<Session>, store: Arc<Store>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, wake: Arc<Wake>, senses: Arc<Senses>) {
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
        let in_turn = outbox.iter().find(|o| o.undelivered.is_none() && o.stage != Stage::Left);
        let Some(next) = in_turn.filter(due).or_else(|| outbox.iter().filter(|o| o.undelivered.is_none() && o.stage == Stage::Left).find(due)).cloned() else {
            // nothing to do now: sleep until something is queued or settled, or the next is due
            let soonest = ask_at.values().map(|at| at.saturating_duration_since(now)).min().unwrap_or(Duration::from_secs(30));
            wake.wait_since(turn, soonest.clamp(Duration::from_millis(200), Duration::from_secs(30)));
            continue;
        };
        // it did not go and is not tried any more: it stays in the list with the reason, for him to send
        // again or let go. He is told once, wherever he is looking.
        let gave_up = |why: NotDelivered| fail(&store, &*host, &next.id, why);
        let staged = |stage: Stage| {
            store.change(|s| {
                if let Some(o) = s.outbox.iter_mut().find(|o| o.id == next.id) {
                    o.stage = stage;
                    // when his computer first had it, not when he wrote it: a message that waited hours with
                    // the relay is given its full half hour from when his computer took it
                    if stage == Stage::Taken && o.taken_at.is_none() {
                        o.taken_at = Some(now_ms());
                    }
                }
                s.outbox.clone()
            })
        };
        match next.stage {
            // with how the last try went, when there was one: "no connection" alone tells him nothing
            Stage::Trying if now_ms().saturating_sub(next.at) > OUTBOX_EXPIRY_MS => {
                gave_up(NotDelivered::Expired);
                continue;
            }
            // His computer took it half an hour ago and, asked again and again, still says only that it has it: it
            // ends, and holds up nothing behind it. Sent again, it goes under the same id, so his computer never
            // acts on it twice.
            Stage::Taken if next.taken_at.is_some_and(|taken| now_ms().saturating_sub(taken) > OUTBOX_EXPIRY_MS) => {
                gave_up(NotDelivered::NeverSettled);
                continue;
            }
            Stage::Left if now_ms().saturating_sub(next.at) > airband_core::relay::HELD_FOR_MS => {
                gave_up(NotDelivered::AwayTooLong);
                continue;
            }
            _ => {}
        }
        let utterance = next.utterance();
        link.keeps_at_relay(outbox.iter().any(|o| o.stage == Stage::Left && o.undelivered.is_none() && o.id != next.id));
        let (got, steps) = link.utterance_once(&utterance);
        // It went straight to his computer while the relay still keeps earlier words of his for it: these
        // arrive first. Worth a line, for whoever wonders about the order.
        let kept = outbox.iter().filter(|o| o.stage == Stage::Left && o.undelivered.is_none() && o.id != next.id).count();
        if kept > 0 && got.is_ok() && steps.last().is_some_and(|s| s.way == Way::Near && s.outcome.is_ok()) {
            link.remark(format!("something he said went straight to his computer ahead of {kept} the relay still keeps for it"));
        }
        match got {
            Ok(Sent::Answered(reply)) => {
                wait = 1;
                session.asked(&reply);
                settle(&store, &*host, &senses.takes, &next.id, airband_core::radio::Outcome::Answered { reply });
            }
            Ok(Sent::Taken) => {
                wait = 1;
                // his computer has it: the recording it was heard from has done its job
                senses.takes.taken(&next.id);
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
                        gave_up(NotDelivered::Refused { why: e.said() });
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

/// Put it at the end of what is waiting to go, wake the sender, and show him what waits: the one way into the
/// outbox.
pub(crate) fn enqueue(store: &Store, wake: &Wake, host: &dyn RadioHost, out: Outgoing) {
    let waiting = store.change(|s| {
        s.outbox.push(out);
        s.outbox.clone()
    });
    wake.ring();
    host.outbox(waiting, None);
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

/// It is not delivered and not tried any more, and why: it stays in the list for him to send again or let go,
/// and he is told.
fn fail(store: &Store, host: &dyn RadioHost, id: &str, why: NotDelivered) {
    let (text, waiting) = store.change(|s| {
        // quoted as he saw it
        let text = s.outbox.iter_mut().find(|o| o.id == id).map(|o| {
            o.undelivered = Some(why.clone());
            o.shown()
        });
        (text, s.outbox.clone())
    });
    if let Some(text) = text {
        host.outbox(waiting, Some(format!("Not delivered: “{}”.", opening(&text))));
    }
}

/// The tower has said what came of something he said: by the feed, or to the outbox asking. A message that
/// is not in the list (it was settled already, or was never typed) is nothing more to do. His computer having
/// it, whatever came of it, is the fact the recording it was heard from is deleted on, before it leaves the list.
pub(crate) fn settle(store: &Store, host: &dyn RadioHost, takes: &crate::takes::Takes, id: &str, outcome: airband_core::radio::Outcome) {
    use airband_core::radio::Outcome;
    match &outcome {
        Outcome::Taken | Outcome::Answered { .. } => takes.taken(id),
        Outcome::Rejected { .. } => {}
    }
    let Some(was) = store.saved.lock().unwrap().outbox.iter().find(|o| o.id == id).cloned() else { return };
    let gone = || {
        store.change(|s| {
            s.outbox.retain(|o| o.id != id);
            s.outbox.clone()
        })
    };
    let reply = match outcome {
        Outcome::Taken => return,
        Outcome::Rejected { why } => return fail(store, host, id, NotDelivered::Refused { why: LinkError::Rejected(why).said() }),
        Outcome::Answered { reply } => reply,
    };
    match kept_for(&reply) {
        // The agents it was for did not get it, all or some: it stays, with why, wherever he sent it from. It
        // leaves only when he lets it go, or sends it again and they get it.
        Some(why @ NotDelivered::Agents { .. }) => fail(store, host, id, why),
        // Not delivered: it is for an agent on another computer, or the tower cannot tell who it is for. It
        // stays here as not delivered while the host offers who it is for (and sends it on to an agent another
        // computer has), and leaves once it has gone there.
        Some(why) => {
            let waiting = store.change(|s| {
                s.outbox.iter_mut().filter(|o| o.id == id).for_each(|o| o.undelivered = Some(why.clone()));
                s.outbox.clone()
            });
            host.outbox(waiting, None);
            // the agents it might be for, by id
            host.unplaced(Outgoing { undelivered: Some(why), ..was }, reply.elsewhere, reply.ask);
        }
        // what was done as he asked shows as his own line in the chat; anything else he has to see
        None => host.outbox(gone(), (!reply.say.is_empty() && !reply.done).then_some(reply.say)),
    }
}

/// Why an answer of his computer's leaves his words not delivered, when it does: the agents did not get them, or
/// the tower read them as for an agent another computer has, or could not tell who they were for.
fn kept_for(reply: &Reply) -> Option<NotDelivered> {
    match (&reply.undelivered, &reply.elsewhere) {
        (Some(why), _) => Some(NotDelivered::Agents { why: (**why).clone() }),
        (None, Some(agent)) => Some(NotDelivered::Elsewhere { agent: agent.clone() }),
        (None, None) if !reply.ask.is_empty() => Some(NotDelivered::WhoFor { guesses: reply.ask.clone() }),
        (None, None) => None,
    }
}

/// Something of his that was answered at once, never having been in the outbox (said on a call, an answer from
/// a notification): when the answer leaves it not delivered, it is kept in the outbox as such, as anything of
/// his is, and the host is told as for anything settled. True when it was kept.
pub(crate) fn keep_answered(store: &Store, host: &dyn RadioHost, takes: &crate::takes::Takes, out: Outgoing, reply: &Reply) -> bool {
    let Some(why) = kept_for(reply) else { return false };
    let id = out.id.clone();
    store.change(|s| {
        if !s.outbox.iter().any(|o| o.id == id) {
            s.outbox.push(Outgoing { undelivered: Some(why), stage: Stage::Taken, ..out });
        }
    });
    settle(store, host, takes, &id, airband_core::radio::Outcome::Answered { reply: reply.clone() });
    true
}

/// Why a message of his was not delivered, in a sentence for him. `named` says what an agent is called, by id.
pub(crate) fn not_delivered_why(o: &Outgoing, named: &dyn Fn(&str) -> String) -> String {
    use airband_core::radio::{NotReachable, Undelivered};
    let Some(why) = &o.undelivered else { return String::new() };
    match why {
        NotDelivered::Expired => expired(&o.trail),
        NotDelivered::NeverSettled => NEVER_SETTLED.into(),
        NotDelivered::AwayTooLong => "Your computer was away for more than a week, and the relay keeps a message that long.".into(),
        NotDelivered::Refused { why } => why.clone(),
        NotDelivered::Agents { why: Undelivered::NobodyRunning } => "No agents are running.".into(),
        NotDelivered::Agents { why: Undelivered::Files } => "Your files hadn\u{2019}t arrived.".into(),
        NotDelivered::Agents { why: Undelivered::TeamGone } => "That team isn\u{2019}t on your computer anymore.".into(),
        NotDelivered::Agents { why: Undelivered::Missed { missed } } => {
            // a sentence for each reason, naming the agents it holds for
            let of = |kind: NotReachable| missed.iter().filter(|m| m.why == kind).map(|m| named(&m.agent)).collect::<Vec<_>>();
            let mut said = Vec::new();
            for (kind, says) in [(NotReachable::NoWayIn, "airband can\u{2019}t send messages to"), (NotReachable::Gone, "No longer running:"), (NotReachable::Failed, "Handing it over failed for")] {
                let names = of(kind);
                if !names.is_empty() {
                    said.push(format!("{says} {}.", and_list(&names)));
                }
            }
            said.join(" ")
        }
        NotDelivered::Elsewhere { .. } => "That agent isn\u{2019}t running anymore.".into(),
        NotDelivered::WhoFor { .. } => "airband couldn\u{2019}t tell who it was for.".into(),
    }
}

/// Names as said in English: "a", "a and b", "a, b and c".
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
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

/// What he is told of a message his computer took and, in half an hour, never said what came of.
pub(crate) const NEVER_SETTLED: &str = "Your computer took it and in half an hour did not say what came of it.";

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
        assert_eq!(after_failure(Retry::Never, Stage::Taken), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Never, Stage::Left), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Never, Stage::Trying), AfterFailure::GiveUp);
        assert_eq!(after_failure(Retry::Soon, Stage::Taken), AfterFailure::AskAfter(ASK_AGAIN));
        assert_eq!(after_failure(Retry::Later, Stage::Left), AfterFailure::AskAfter(ASK_AFTER_LEFT));
        assert_eq!(LinkError::Refused(airband_core::radio::wire::Refusal::NotPaired).retry(), Retry::Never);
    }

    #[test]
    fn a_quick_message_is_quoted_by_the_words_he_saw() {
        let mut o = Outgoing::new("a".into(), "Keep going with what you planned, and say when it is done.".into(), None, None, Some("Keep going".into()), Vec::new(), Vec::new());
        assert_eq!(o.shown(), "Keep going");
        o.label = None;
        assert!(o.shown().starts_with("Keep going with"));
    }

    #[test]
    fn a_try_is_told_a_sentence_for_each_way_it_failed() {
        let step = |way, outcome| Step { at: 0, what: "POST /v1/radio/utterance".into(), way, outcome, ms: 3 };
        let steps = [step(Way::Relay, Err(LinkError::Away)), step(Way::Near, Err(LinkError::Unreachable(Way::Near))), step(Way::Kept, Err(LinkError::HoldRefused(507))), step(Way::Kept, Err(LinkError::HoldRefused(507))), step(Way::Near, Ok(200))];
        // each cause once, in the order tried; the try that worked says nothing
        assert_eq!(told(&steps), [LinkError::Away.said(), LinkError::Unreachable(Way::Near).said(), LinkError::HoldRefused(507).said()]);
        assert_eq!(expired(&[]), "Half an hour of trying did not get it through.");
        assert_eq!(expired(&told(&steps[..1])), format!("{} {}", expired(&[]), LinkError::Away.said()));
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
        let radio = Radio::start(format!("airband://pair?relay=http%3A%2F%2F127.0.0.1%3A9&secret={}&name=Mac", "ab".repeat(32)), String::new(), host.clone()).unwrap();
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
        assert!(until(&|o| o.first().is_some_and(|m| m.tries >= 1 && m.last_tried > 0 && m.trail == [LinkError::Unreachable(Way::Relay).said()])), "{:?}", outbox());
        // half an hour on, it is given up on: it stays, with the reason, and he is told once
        radio.store.change(|s| s.outbox[0].at = 1);
        radio.wake.ring();
        assert!(until(&|o| o[0].undelivered == Some(NotDelivered::Expired) && not_delivered_why(&o[0], &|a| a.to_string()) == expired(&[LinkError::Unreachable(Way::Relay).said()])), "{:?}", outbox());
        let notes: Vec<String> = host.0.lock().unwrap().iter().filter_map(|(_, note)| note.clone()).collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0], "Not delivered: “the first thing”.");
        // what he says next is tried all the same
        let second = radio.queue("the second thing".into(), None, None, None);
        assert!(until(&|o| o.len() == 2 && o[0].undelivered.is_some() && o[1].undelivered.is_none() && o[1].tries >= 1), "{:?}", outbox());
        // it is followed by the id it was queued under, not by being the last one waiting
        assert_eq!(outbox()[1].id, second);
        // sent again, the first goes to the end as if just said
        let first = outbox()[0].id.clone();
        radio.send_again(&first, "fresh", None);
        assert!(until(&|o| o.len() == 2 && o[1].id == first && o[1].undelivered.is_none() && o[1].at > 1), "{:?}", outbox());
        // and one that was given up on can be let go, which hands back its words
        radio.store.change(|s| s.outbox[0].at = 1);
        radio.wake.ring();
        assert!(until(&|o| o[0].undelivered.is_some()));
        let second = outbox()[0].id.clone();
        assert_eq!(radio.discard(&second).map(|o| o.text), Some("the second thing".into()));
        assert!(outbox().iter().all(|o| o.id != second));
        // one still being tried is not let go that way
        assert!(radio.discard(&first).is_none());
        radio.stop();
    }

    #[test]
    fn words_the_agents_did_not_get_stay_with_why_until_he_lets_them_go_or_they_get_there() {
        use airband_core::radio::{Missed, NotReachable, Outcome, Undelivered};
        let host = Watching::default();
        let store = Store::open("", None);
        let takes = crate::takes::Takes::default();
        let put = |id: &str| store.change(|s| s.outbox.push(Outgoing { stage: Stage::Taken, sent_from: SentFrom { room: Some("r1".into()), agent: None }, ..Outgoing::new(id.into(), format!("words of {id}"), None, None, None, Vec::new(), Vec::new()) }));
        let kept = |id: &str| store.saved.lock().unwrap().outbox.iter().find(|o| o.id == id).cloned();
        // for an agent airband has no way to send to: it stays, marked, and he is told once
        put("a");
        let missed = vec![Missed { agent: "x1".into(), why: NotReachable::NoWayIn }, Missed { agent: "x2".into(), why: NotReachable::Gone }];
        settle(&store, &host, &takes, "a", Outcome::Answered { reply: Reply { say: "Could not reach shop one.".into(), undelivered: Some(Box::new(Undelivered::Missed { missed: missed.clone() })), ..Reply::default() } });
        let a = kept("a").expect("kept");
        assert_eq!(a.undelivered, Some(NotDelivered::Agents { why: Undelivered::Missed { missed } }));
        assert_eq!(host.0.lock().unwrap().last().unwrap().1.as_deref(), Some("Not delivered: “words of a”."));
        // why, in words, naming each agent
        let named = |id: &str| if id == "x1" { "shop one".to_string() } else { "shop two".to_string() };
        assert_eq!(not_delivered_why(&a, &named), "airband can\u{2019}t send messages to shop one. No longer running: shop two.");
        // told again (the feed and the asking both say it): nothing changes
        settle(&store, &host, &takes, "a", Outcome::Answered { reply: Reply { undelivered: Some(Box::new(Undelivered::NobodyRunning)), ..Reply::default() } });
        assert!(kept("a").is_some());
        // tried again: under a new id, since his computer has had this one, to the agents it did not reach, from
        // where he sent it
        let again = a.clone().again("b".into());
        assert_eq!((again.id.as_str(), again.to.clone(), again.undelivered.clone(), again.sent_from.room.as_deref(), again.text.as_str()), ("b", vec!["x1".to_string(), "x2".into()], None, Some("r1"), "words of a"));
        // one that never reached his computer goes under its own id again
        let expired = Outgoing { undelivered: Some(NotDelivered::Expired), ..a.clone() };
        assert!(!expired.undelivered.as_ref().unwrap().answered());
        // nobody running, files that had not arrived: kept the same way
        for (id, why) in [("c", Undelivered::NobodyRunning), ("d", Undelivered::Files)] {
            put(id);
            settle(&store, &host, &takes, id, Outcome::Answered { reply: Reply { undelivered: Some(Box::new(why.clone())), ..Reply::default() } });
            assert_eq!(kept(id).and_then(|o| o.undelivered), Some(NotDelivered::Agents { why }));
        }
        // for an agent another computer has: kept as not delivered while the host sends it there
        put("e");
        settle(&store, &host, &takes, "e", Outcome::Answered { reply: Reply { elsewhere: Some("far".into()), ..Reply::default() } });
        assert_eq!(kept("e").and_then(|o| o.undelivered), Some(NotDelivered::Elsewhere { agent: "far".into() }));
        // delivered: it leaves the list
        put("f");
        settle(&store, &host, &takes, "f", Outcome::Answered { reply: Reply { say: "Sent to shop one.".into(), done: true, ..Reply::default() } });
        assert!(kept("f").is_none());
    }

    #[test]
    fn something_answered_at_once_that_did_not_get_there_is_kept_as_anything_is() {
        use airband_core::radio::{Missed, NotReachable, Undelivered};
        let host = Watching::default();
        let store = Store::open("", None);
        let takes = crate::takes::Takes::default();
        let out = Outgoing::of("said on a call".into());
        // delivered: nothing is kept
        assert!(!keep_answered(&store, &host, &takes, out.clone(), &Reply { done: true, ..Reply::default() }));
        assert!(store.saved.lock().unwrap().outbox.is_empty());
        // not delivered: kept, marked, and never tried again by itself
        let reply = Reply { undelivered: Some(Box::new(Undelivered::Missed { missed: vec![Missed { agent: "x1".into(), why: NotReachable::Failed }] })), ..Reply::default() };
        assert!(keep_answered(&store, &host, &takes, out.clone(), &reply));
        let kept = store.saved.lock().unwrap().outbox.clone();
        assert_eq!((kept.len(), kept[0].id.clone(), kept[0].undelivered.is_some()), (1, out.id.clone(), true));
    }

    #[test]
    fn a_message_stays_until_the_tower_says_what_came_of_it() {
        use airband_core::radio::{Outcome, Rejected};
        let host = Watching::default();
        let store = Store::open("", None);
        let takes = crate::takes::Takes::default();
        let put = |id: &str, stage: Stage| {
            store.change(|s| s.outbox.push(Outgoing { stage, ..Outgoing::new(id.into(), format!("words of {id}"), None, None, None, Vec::new(), Vec::new()) }));
        };
        let ids = || store.saved.lock().unwrap().outbox.iter().map(|o| o.id.clone()).collect::<Vec<_>>();
        let last = || host.0.lock().unwrap().last().cloned().unwrap();
        for (id, stage) in [("a", Stage::Taken), ("b", Stage::Taken), ("c", Stage::Left), ("d", Stage::Taken), ("e", Stage::Left)] {
            put(id, stage);
        }
        // the tower has it and nothing more: it stays
        settle(&store, &host, &takes, "a", Outcome::Taken);
        assert_eq!(ids(), ["a", "b", "c", "d", "e"]);
        // done as he asked: it leaves the list, and there is nothing to tell him
        settle(&store, &host, &takes, "a", Outcome::Answered { reply: Reply { say: "Sent to shop two.".into(), done: true, ..Reply::default() } });
        assert_eq!((ids(), last().1), (vec!["b".to_string(), "c".into(), "d".into(), "e".into()], None));
        // not done as he asked: it leaves the list and he is told what the tower said
        settle(&store, &host, &takes, "b", Outcome::Answered { reply: Reply { say: "Nobody has stopped.".into(), ..Reply::default() } });
        assert_eq!(last().1.as_deref(), Some("Nobody has stopped."));
        // one the relay kept, answered when his computer came back: the same
        settle(&store, &host, &takes, "c", Outcome::Answered { reply: Reply { say: "Sent to shop two.".into(), done: true, ..Reply::default() } });
        assert_eq!(ids(), ["d", "e"]);
        // the tower cannot tell who it is for: the host asks him, and it stays as not delivered meanwhile
        settle(&store, &host, &takes, "d", Outcome::Answered { reply: Reply { ask: vec!["a2".into(), "a1".into()], ..Reply::default() } });
        assert_eq!((ids(), last().1.as_deref()), (vec!["d".to_string(), "e".into()], Some("unplaced: a2, a1")));
        assert_eq!(store.saved.lock().unwrap().outbox[0].undelivered, Some(NotDelivered::WhoFor { guesses: vec!["a2".into(), "a1".into()] }));
        store.change(|s| s.outbox.retain(|o| o.id != "d"));
        // one his computer got too late: it stays, with the reason, to send again or let go, and he is told
        settle(&store, &host, &takes, "e", Outcome::Rejected { why: Rejected::TooOld });
        let kept = store.saved.lock().unwrap().outbox.clone();
        assert_eq!(kept[0].undelivered, Some(NotDelivered::Refused { why: LinkError::Rejected(Rejected::TooOld).said() }));
        assert_eq!(last().1.as_deref(), Some("Not delivered: “words of e”."));
        // an outcome told twice, or for something this radio never typed, is nothing to do
        let before = host.0.lock().unwrap().len();
        settle(&store, &host, &takes, "a", Outcome::Answered { reply: Reply::default() });
        settle(&store, &host, &takes, "zz", Outcome::Rejected { why: Rejected::TooOld });
        assert_eq!(host.0.lock().unwrap().len(), before);
    }
}
