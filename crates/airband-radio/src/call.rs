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
    /// from here; where it goes is the caller's to work out.
    Dictate(Box<dyn FnOnce(Listened) + Send>),
}

/// Asks the tower for the next transmission, over and over. The tower only hands one out when the radio has
/// said it is ready, so this never gets ahead of the speaking.
pub(crate) fn poll(jobs: Sender<Job>, link: Arc<Link>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, busy: Arc<AtomicBool>, calling: Arc<AtomicBool>, wake: Arc<Wake>, senses: Arc<Senses>) {
    let mut online = false;
    let mut wait = 1;
    let mut joined = true;
    // the last transmission this radio got: a tower that gave it another since, in an answer that never
    // arrived, gives that one again
    let mut have = String::new();
    while !stop.load(Ordering::SeqCst) {
        if !calling.load(Ordering::SeqCst) {
            // not in call mode: ask for nothing, so the tower keeps transmissions for the chat or for later
            online = false;
            joined = true;
            wake.wait(Duration::from_secs(30));
            continue;
        }
        // with no voice chosen there is nothing to read with: nothing is asked for, and it all stays in the chat
        if !voice::can_speak(&*host, &senses.mouth) {
            online = false;
            joined = true;
            wake.wait(Duration::from_secs(5));
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

pub(crate) fn work(jobs: Receiver<Job>, link: Arc<Link>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, busy: Arc<AtomicBool>, calling: Arc<AtomicBool>, senses: Arc<Senses>) {
    let ears = ears::Ears::new(senses.clone());
    {
        // the pass for the listening service is fetched before he first needs it, and again now and then, so
        // opening the microphone never waits on it
        let (ears, stop) = (ears.clone(), stop.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                ears.warm();
                thread::sleep(Duration::from_secs(240));
            }
        });
    }
    while let Ok(job) = jobs.recv() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        busy.store(true, Ordering::SeqCst);
        match job {
            // He turned reading aloud off: what was on its way is not read, and what is being read stops at
            // once. It is all in the chat.
            Job::Transmission(_) if !calling.load(Ordering::SeqCst) => {
                link.tell("/ready", &serde_json::json!({}));
            }
            Job::Transmission(t) => {
                host.status(RadioStatus::Speaking { callsign: t.callsign.clone() });
                let heard = voice::say_until(&*host, &senses.mouth, &t.say, &|| !calling.load(Ordering::SeqCst));
                // "heard" is what takes it off the tower's list: if this is lost, he hears it once more, never zero
                // times. What made no sound was not heard: the tower is told why and keeps it.
                match heard {
                    voice::Said::Aloud => {
                        link.tell("/heard", &Heard { id: t.id.clone() });
                    }
                    voice::Said::Nothing(why) => {
                        link.tell("/unheard", &serde_json::json!({ "id": t.id, "why": why }));
                    }
                }
                // A question is read to him and then waits in the chat, where he answers it when he chooses, by
                // its own buttons or its microphone. The microphone never opens by itself: he may be in the
                // middle of saying something to someone else.
                link.tell("/ready", &serde_json::json!({}));
            }
            Job::Talk(re, scope) => {
                let _here = still_here(&link, &calling);
                converse(&link, &*host, &ears, &senses.mouth, re, scope)
            }
            Job::Dictate(done) => {
                let _here = still_here(&link, &calling);
                host.status(RadioStatus::Listening);
                done(ears.hear(&*host, true));
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

/// Open the microphone; send what he says; speak the tower's answer; listen again if the tower asks.
pub(crate) fn converse(link: &Arc<Link>, host: &dyn RadioHost, ears: &ears::Ears, mouth: &mouth::Mouth, mut re: Option<String>, scope: Option<String>) {
    for _ in 0..MAX_ROUNDS {
        host.status(RadioStatus::Listening);
        let Listened::Said(text) = ears.hear(host, false) else { return };
        host.status(RadioStatus::Sending);
        let reply = match link.utterance(&text, re.take(), scope.clone()) {
            Ok(Some(reply)) => reply,
            // his computer has it and has not said what came of it yet: that is in the chat when it does
            Ok(None) => return,
            Err(e) => {
                // said aloud, with what went wrong
                host.status(RadioStatus::Speaking { callsign: "tower".into() });
                voice::say(host, mouth, &format!("That was not sent: {}.", e.told()));
                return;
            }
        };
        if !reply.say.is_empty() {
            host.status(RadioStatus::Speaking { callsign: "tower".into() });
            voice::say(host, mouth, &reply.say);
        }
        if !reply.listen {
            return;
        }
    }
}
