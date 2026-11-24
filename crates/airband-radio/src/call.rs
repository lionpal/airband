//! Call mode: ask for the next transmission, speak it, listen, send what he said, say when free. One thing at
//! a time. The host only speaks and listens; the order of everything is decided here.
use super::*;

/// How many times in a row the radio reopens the microphone because the tower asked a follow-up.
pub(crate) const MAX_ROUNDS: usize = 4;

pub(crate) enum Job {
    Transmission(Transmission),
    /// He wants to speak: the message he is answering, if he picked one, and the room he is looking at.
    Talk(Option<String>, Option<String>),
    /// He opened the microphone himself: listen until he closes it, and hand over what he said. Nothing is sent
    /// from here; where it goes is the caller's to work out, and is kept with the recording as aimed.
    Dictate(crate::takes::Aim, Box<dyn FnOnce(Listened) + Send>),
}

/// Everything the polling thread is given to work with.
pub(crate) struct Polling {
    pub(crate) jobs: Sender<Job>,
    pub(crate) link: Arc<Link>,
    pub(crate) host: Arc<dyn RadioHost>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) busy: Arc<AtomicBool>,
    pub(crate) calling: Arc<AtomicBool>,
    pub(crate) wake: Arc<Wake>,
    pub(crate) senses: Arc<Senses>,
}

/// Asks the tower for the next transmission, over and over. The tower only hands one out when the radio has
/// said it is ready, so this never gets ahead of the speaking.
pub(crate) fn poll(with: Polling) {
    let Polling { jobs, link, host, stop, busy, calling, wake, senses } = with;
    let mut online = false;
    let mut wait = 1;
    let mut joined = true;
    // the last transmission this radio got: a tower that gave it another since, in an answer that never
    // arrived, gives that one again
    let mut have = String::new();
    while !stop.load(Ordering::SeqCst) {
        // the turn first: a call started after the look below still ends the sleep
        let turn = wake.turn();
        if !calling.load(Ordering::SeqCst) {
            // not in call mode: ask for nothing, so the tower keeps transmissions for the chat or for later
            online = false;
            joined = true;
            wake.wait_since(turn, Duration::from_secs(30));
            continue;
        }
        // with no voice chosen there is nothing to read with: nothing is asked for, and it all stays in the chat
        if !senses.mouth.ready() {
            online = false;
            joined = true;
            wake.wait_since(turn, Duration::from_secs(5));
            continue;
        }
        // the first request after he turned reading aloud on says so, and only that one: the tower leaves old
        // news for the chat when he joins, and never because a radio was quiet for a while
        match link.next(joined, &have) {
            Ok(next) => {
                joined = false;
                if !online && !busy.load(Ordering::SeqCst) {
                    host.status(RadioStatus::Idle);
                }
                online = true;
                wait = 1;
                if let Some(t) = next {
                    have = t.id.clone();
                    if jobs.send(Job::Transmission(t)).is_err() {
                        return;
                    }
                }
            }
            Err(_) => {
                if online || wait == 1 {
                    host.status(RadioStatus::Offline);
                }
                online = false;
                thread::sleep(Duration::from_secs(wait));
                wait = (wait * 2).min(15);
            }
        }
    }
}

/// Everything the thread that does the jobs is given to work with.
pub(crate) struct Working {
    pub(crate) jobs: Receiver<Job>,
    pub(crate) link: Arc<Link>,
    pub(crate) session: Arc<Session>,
    /// The outbox: what he said on a call that his computer did not take there and then goes from it.
    pub(crate) store: Arc<Store>,
    pub(crate) wake: Arc<Wake>,
    pub(crate) host: Arc<dyn RadioHost>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) busy: Arc<AtomicBool>,
    pub(crate) calling: Arc<AtomicBool>,
    pub(crate) senses: Arc<Senses>,
}

pub(crate) fn work(with: Working) {
    let Working { jobs, link, session, store, wake, host, stop, busy, calling, senses } = with;
    let ears = ears::Ears::new(senses.clone());
    while let Ok(job) = jobs.recv() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        busy.store(true, Ordering::SeqCst);
        match job {
            // He turned reading aloud off: what was on its way is not read, and what is being read stops at
            // once. It is all in the chat.
            Job::Transmission(_) if !calling.load(Ordering::SeqCst) => {
                link.tell::<airband_core::radio::wire::at::Ready>(&airband_core::radio::wire::Nothing {});
            }
            Job::Transmission(t) => {
                host.status(RadioStatus::Speaking { agent: Some(t.agent.clone()).filter(|a| !a.is_empty()), callsign: Some(t.callsign.clone()) });
                let heard = voice::say_until(&*host, &senses.mouth, &t.say, &|| !calling.load(Ordering::SeqCst));
                // "heard" is what takes it off the tower's list: if this is lost, he hears it once more, never zero
                // times. What made no sound was not heard: the tower is told why and keeps it.
                match heard {
                    voice::Said::Aloud => {
                        link.tell::<airband_core::radio::wire::at::Heard>(&Heard { id: t.id.clone() });
                    }
                    voice::Said::Nothing(why) => {
                        link.tell::<airband_core::radio::wire::at::Unheard>(&airband_core::radio::Unheard { id: t.id.clone(), why });
                    }
                }
                // A question is read to him and then waits in the chat, where he answers it when he chooses, by
                // its own buttons or its microphone. The microphone never opens by itself: he may be in the
                // middle of saying something to someone else.
                link.tell::<airband_core::radio::wire::at::Ready>(&airband_core::radio::wire::Nothing {});
            }
            Job::Talk(re, scope) => {
                let _here = still_here(&link, &calling);
                converse(&CallLine { link: &link, session: &session, store: &store, wake: &wake }, &*host, &ears, &senses.mouth, re, scope)
            }
            Job::Dictate(aim, done) => {
                let _here = still_here(&link, &calling);
                host.status(RadioStatus::Listening);
                done(ears.hear(&*host, true, aim));
            }
        }
        busy.store(false, Ordering::SeqCst);
        host.status(RadioStatus::Idle);
    }
}

/// While he has the microphone open on a call this line asks the tower for nothing, and a tower that hears
/// nothing for long takes the radio for gone. So it is told, every quarter of a minute, that the radio is
/// still there, until the returned guard is dropped.
fn still_here(link: &Arc<Link>, calling: &Arc<AtomicBool>) -> impl Drop {
    struct Until(Arc<AtomicBool>);
    impl Drop for Until {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let over = Arc::new(AtomicBool::new(false));
    if calling.load(Ordering::SeqCst) {
        let (link, over) = (link.clone(), over.clone());
        thread::spawn(move || {
            let mut waited = 0;
            while !over.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(500));
                waited += 1;
                if waited % 30 == 0 {
                    link.say_presence();
                }
            }
        });
    }
    Until(over)
}

/// What a call sends what he says by: the link now, and the outbox for what does not reach his computer now.
pub(crate) struct CallLine<'a> {
    pub(crate) link: &'a Link,
    pub(crate) session: &'a Session,
    pub(crate) store: &'a Store,
    pub(crate) wake: &'a Wake,
}

/// Open the microphone; send what he says; speak the tower's answer; listen again if the tower asks. What he
/// says goes under the id of its recording, now and every time it is tried again, so his computer acts on it
/// once. The recording is kept until his computer has the words: what the relay keeps for it, and what could
/// not be sent now, goes into the outbox under that id, which sends it on and lets the recording go only when
/// his computer takes it.
pub(crate) fn converse(line: &CallLine, host: &dyn RadioHost, ears: &ears::Ears, mouth: &mouth::Mouth, mut re: Option<String>, scope: Option<String>) {
    let speak = |text: &str| {
        host.status(RadioStatus::Speaking { agent: None, callsign: None });
        voice::say(host, mouth, text);
    };
    for _ in 0..MAX_ROUNDS {
        host.status(RadioStatus::Listening);
        let (text, take) = match ears.hear(host, false, crate::takes::Aim { re: re.clone(), ..Default::default() }) {
            Listened::Said { text, take } => (text, take),
            // he was told why with the tone (`ears.rs`); on a call he may not be looking, so it is said too
            Listened::Unheard { .. } => return speak(UNHEARD_ON_CALL),
            Listened::Nothing | Listened::Cannot => return,
        };
        host.status(RadioStatus::Sending);
        match send_said(line, host, ears.takes(), &text, take, re.take(), scope.clone()) {
            Went::Answered(reply) => {
                if !reply.say.is_empty() {
                    speak(&reply.say);
                }
                if !reply.listen {
                    return;
                }
            }
            Went::Say(text) => return speak(&text),
            Went::Quiet => return,
        }
    }
}

/// What came of sending something he said on a call.
#[derive(Debug)]
pub(crate) enum Went {
    /// His computer answered.
    Answered(Reply),
    /// It is not with his computer yet, and he is told so: it is in the outbox.
    Say(String),
    /// His computer has it and has not said what came of it yet: that is in the chat when it does.
    Quiet,
}

/// Send what he said, under the id of its take (`take`), and keep its recording until his computer has it.
pub(crate) fn send_said(line: &CallLine, host: &dyn RadioHost, takes: &crate::takes::Takes, text: &str, take: Option<String>, re: Option<String>, scope: Option<String>) -> Went {
    let out = Outgoing { spoken: true, ..line.session.outgoing(take.clone().unwrap_or_else(|| random_hex(12)), text, re, scope, None) };
    let to_outbox = |out: Outgoing| {
        if let Some(take) = &take {
            takes.sent(take);
        }
        enqueue(line.store, line.wake, host, out);
    };
    match line.session.send(line.link, &out) {
        Ok(Sent::Answered(reply)) => {
            take.iter().for_each(|t| takes.taken(t));
            // the agents did not get it, or it is for one another computer has: it is kept in the chat as not
            // delivered (and sent on there), as anything of his is. What the tower said is still said to him
            crate::outbox::keep_answered(line.store, host, takes, out, &reply);
            Went::Answered(reply)
        }
        Ok(Sent::Taken) => {
            take.iter().for_each(|t| takes.taken(t));
            Went::Quiet
        }
        // the relay keeps it for his computer: the outbox follows it from here
        Ok(Sent::Left { away }) => {
            to_outbox(Outgoing { stage: Stage::Left, ..out });
            if away { Went::Say(AWAY.into()) } else { Went::Quiet }
        }
        Err(e) => {
            // in the one wording every failure is told in, after what did not happen. One that can never go
            // waits in the chat with why, for him to send again or let go; any other goes when it can.
            let never = e.retry() == Retry::Never;
            let why = e.said();
            let said = if never { format!("That was not sent. {why}") } else { format!("That was not sent yet. {why} It goes when it can.") };
            to_outbox(Outgoing { undelivered: never.then(|| NotDelivered::Refused { why: e.said() }), ..out });
            Went::Say(said)
        }
    }
}

/// Said on a call when what he said was not heard: its recording waits in the chat.
pub(crate) const UNHEARD_ON_CALL: &str = "I didn't catch that. It's in the chat to try again.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Transport;
    use airband_core::radio::wire::Method;

    /// A tower that answers every utterance with this, or cannot be reached.
    struct Tower(Result<(u16, Value), LinkError>);
    impl Transport for Tower {
        fn exchange(&self, _: Method, _: &str, _: Option<Value>) -> Result<(u16, Value), LinkError> {
            self.0.clone()
        }
    }
    fn line_to(tower: Tower) -> (Link, Session, Store, Wake) {
        let link = Link::new(airband_core::pairing::Pairing::parse("airband://pair?url=http%3A%2F%2F127.0.0.1%3A9&token=t&name=Mac").unwrap(), "test", crate::link::Trail::open(None)).unwrap().through(Arc::new(tower));
        (link, Session::default(), Store::open("", None), Wake::default())
    }
    /// A take with words heard in it.
    fn heard(takes: &crate::takes::Takes) -> String {
        let rec = takes.begin(crate::takes::Aim::default());
        let id = rec.id.clone();
        takes.ended(rec, ears::Got::Words(Some("ship it".into())));
        id
    }
    struct Quiet;
    impl RadioHost for Quiet {
        fn told(&self, _: String) {}
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
        fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
        fn hearing(&self, _: String) {}
        fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
        fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
        fn run_model(&self, _: String, _: String, _: ChoiceJob, _: Vec<u8>) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn what_he_said_on_a_call_that_did_not_reach_his_computer_waits_in_the_outbox_under_its_takes_id() {
        let (link, session, store, wake) = line_to(Tower(Err(LinkError::Unreachable(Way::Paired))));
        let line = CallLine { link: &link, session: &session, store: &store, wake: &wake };
        let takes = crate::takes::Takes::default();
        let take = heard(&takes);
        let went = send_said(&line, &Quiet, &takes, "ship it", Some(take.clone()), None, Some("team-1".into()));
        assert!(matches!(went, Went::Say(_)), "{went:?}");
        // one id for the one thing he said: tried again from the outbox, his computer acts on it once
        let outbox = store.saved.lock().unwrap().outbox.clone();
        assert_eq!(outbox.iter().map(|o| (o.id.as_str(), o.scope.as_deref(), o.undelivered.is_none())).collect::<Vec<_>>(), [(take.as_str(), Some("team-1"), true)]);
        // its recording is kept, and it is the outbox's to send
        assert!(takes.all().iter().any(|t| t.id == take && matches!(t.stage, crate::takes::Stage::Sent { .. })));
    }

    #[test]
    fn what_his_computer_answered_on_a_call_lets_its_recording_go() {
        let answered = serde_json::to_value(airband_core::radio::Outcome::Answered { reply: Reply { say: "Sent to shop two.".into(), ..Reply::default() } }).unwrap();
        let (link, session, store, wake) = line_to(Tower(Ok((200, answered))));
        let line = CallLine { link: &link, session: &session, store: &store, wake: &wake };
        let takes = crate::takes::Takes::default();
        let take = heard(&takes);
        let went = send_said(&line, &Quiet, &takes, "ship it", Some(take), None, None);
        assert!(matches!(&went, Went::Answered(r) if r.say == "Sent to shop two."), "{went:?}");
        assert!(takes.all().is_empty() && store.saved.lock().unwrap().outbox.is_empty());
    }
}
