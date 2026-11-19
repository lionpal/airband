//! Call mode: ask for the next transmission, speak it, listen, send what he said, say when free. One thing at
//! a time. The host only speaks and listens; the order of everything is decided here.
use super::*;

/// How many times in a row the radio reopens the microphone because the tower asked a follow-up.
pub(crate) const MAX_ROUNDS: usize = 4;

pub(crate) enum Job {
    Transmission(Transmission),
    /// He wants to speak: the message he is answering, if he picked one, and the room he is looking at.
    Talk(Option<String>, Option<String>),
}

/// Asks the tower for the next transmission, over and over. The tower only hands one out when the radio has
/// said it is ready, so this never gets ahead of the speaking.
pub(crate) fn poll(jobs: Sender<Job>, link: Arc<Link>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, busy: Arc<AtomicBool>, calling: Arc<AtomicBool>, wake: Arc<Wake>) {
    let mut online = false;
    let mut wait = 1;
    while !stop.load(Ordering::SeqCst) {
        if !calling.load(Ordering::SeqCst) {
            // not in call mode: ask for nothing, so the tower keeps transmissions for the chat or for later
            online = false;
            wake.wait(Duration::from_secs(30));
            continue;
        }
        match link.next() {
            Ok(next) => {
                if !online && !busy.load(Ordering::SeqCst) {
                    host.status(RadioStatus::Idle);
                }
                online = true;
                wait = 1;
                if let Some(t) = next {
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

pub(crate) fn work(jobs: Receiver<Job>, link: Arc<Link>, host: Arc<dyn RadioHost>, stop: Arc<AtomicBool>, busy: Arc<AtomicBool>) {
    let ears = ears::Ears::default();
    while let Ok(job) = jobs.recv() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        busy.store(true, Ordering::SeqCst);
        match job {
            Job::Transmission(t) => {
                host.status(RadioStatus::Speaking { callsign: t.callsign.clone() });
                voice::say(&link, &*host, &t.say, t.speech.as_deref());
                // "heard" is what takes it off the tower's list: if this is lost, he hears it once more, never zero times
                let _ = link.post("/heard", &Heard { id: t.id.clone() });
                // what he says next answers this transmission, whoever else has spoken meanwhile
                converse(&link, &*host, &ears, Some(t.id), None);
                let _ = link.post("/ready", &serde_json::json!({}));
            }
            Job::Talk(re, scope) => converse(&link, &*host, &ears, re, scope),
        }
        busy.store(false, Ordering::SeqCst);
        host.status(RadioStatus::Idle);
    }
}

/// Open the microphone; send what he says; speak the tower's answer; listen again if the tower asks.
pub(crate) fn converse(link: &Link, host: &dyn RadioHost, ears: &ears::Ears, mut re: Option<String>, scope: Option<String>) {
    for _ in 0..MAX_ROUNDS {
        host.status(RadioStatus::Listening);
        let Some(text) = ears.hear(link, host).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) else { return };
        host.status(RadioStatus::Sending);
        let reply = match link.utterance(&text, re.take(), scope.clone()) {
            Ok(r) => r,
            Err(_) => {
                host.status(RadioStatus::Speaking { callsign: "tower".into() });
                host.speak("No connection. That was not sent.".into());
                return;
            }
        };
        if !reply.say.is_empty() {
            host.status(RadioStatus::Speaking { callsign: "tower".into() });
            voice::say(link, host, &reply.say, reply.speech.as_deref());
        }
        if !reply.listen {
            return;
        }
    }
}
