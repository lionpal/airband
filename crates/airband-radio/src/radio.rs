//! One paired computer's radio: its link, its jobs thread, and everything a host can ask of it.
use super::*;
mod asks;

/// What the radio's waiting threads sleep on: they are woken when there is something to do (a message to send,
/// a call starting, stopping), and do not wake up several times a second to look.
#[derive(Default)]
pub(crate) struct Wake {
    turn: Mutex<u64>,
    rung: std::sync::Condvar,
}

impl Wake {
    pub(crate) fn ring(&self) {
        *self.turn.lock().unwrap() += 1;
        self.rung.notify_all();
    }
    /// Sleep until rung, or for `at_most`.
    pub(crate) fn wait(&self, at_most: Duration) {
        let turn = self.turn.lock().unwrap();
        let was = *turn;
        let _ = self.rung.wait_timeout_while(turn, at_most, |now| *now == was);
    }
}

#[derive(uniffi::Object)]
pub struct Radio {
    pub(crate) jobs: Mutex<Sender<Job>>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) calling: Arc<AtomicBool>,
    pub(crate) link: Arc<Link>,
    pub(crate) store: Arc<Store>,
    pub(crate) host: Arc<dyn RadioHost>,
    pub(crate) wake: Arc<Wake>,
    /// What he chose to listen and speak with on this device.
    pub(crate) senses: Arc<Senses>,
    /// The roster as last seen, by the tower's number for it: sent back so a wait also ends when it changes.
    pub(crate) roster: std::sync::atomic::AtomicU64,
    /// Goes up each time a reading aloud is started or stopped: one that finds it changed leaves off.
    pub(crate) reading: std::sync::atomic::AtomicU64,
}

#[uniffi::export]
impl Radio {
    /// Start working with the tower this pairing link leads to (the link `airband pair` prints). Returns at
    /// once; fails only if the link is not a pairing link. `data_dir` is a folder of the app's own where
    /// unsent messages and the recent chat are kept across restarts; empty keeps nothing.
    #[uniffi::constructor]
    pub fn start(link: String, data_dir: String, host: Arc<dyn RadioHost>) -> Result<Arc<Self>, RadioError> {
        // a radio on its own is a text radio (a terminal): what is said is shown by the host and what he says
        // is a line he types. That is what it is started with, as its choice for each.
        let senses = Senses::default();
        senses.mouth.set(Some(mouth::Own::Host));
        senses.ears.set(Some(ears::Own::Host));
        Self::start_with(link, data_dir, host, Arc::new(senses))
    }

    /// Chat, for a bad connection: take what he typed and keep trying until the tower has it. Returns at
    /// once. The host hears about progress through `outbox`. Survives the app closing; gives up, and says
    /// so, after half an hour.
    ///
    /// Who it is for: tags at the start of the text if there are any; else the message `re` if he picked one;
    /// else the tower works it out from what he wrote, among the agents of the team `scope` names by its id
    /// (nothing for all of them).
    /// `label`, for a quick message, is the button's words: the chat shows those and the agent gets `text`.
    /// Returns the id it waits under, for following it.
    pub fn queue(&self, text: String, re: Option<String>, scope: Option<String>, label: Option<String>) -> String {
        self.queue_as(text, re, scope, label, String::new(), Vec::new(), None)
    }
}

impl Radio {
    /// As [`Radio::start`], listening and speaking with what he chose on this device.
    pub(crate) fn start_with(link: String, data_dir: String, host: Arc<dyn RadioHost>, senses: Arc<Senses>) -> Result<Arc<Self>, RadioError> {
        let store = Arc::new(Store::open(&data_dir));
        let me = store.saved.lock().unwrap().radio.clone();
        let me = if me.is_empty() { store.change(|s| { s.radio = format!("radio-{}", random_hex(4)); s.radio.clone() }) } else { me };
        // the record of what failed is kept beside the saved chat, on this device
        let trail = (!data_dir.is_empty()).then(|| std::path::Path::new(&data_dir).join("trail.json"));
        let link = Pairing::parse(&link).and_then(|p| Link::new(p, &me, trail)).map_err(|reason| RadioError::BadLink { reason })?;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let link = Arc::new(link);
        let busy = Arc::new(AtomicBool::new(false));
        let calling = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Wake::default());
        {
            let (link, host, stop, busy, calling, senses) = (link.clone(), host.clone(), stop.clone(), busy.clone(), calling.clone(), senses.clone());
            thread::spawn(move || work(rx, link, host, stop, busy, calling, senses));
        }
        {
            let (tx, stop, link, calling, host, wake, senses) = (tx.clone(), stop.clone(), link.clone(), calling.clone(), host.clone(), wake.clone(), senses.clone());
            thread::spawn(move || poll(call::Polling { jobs: tx, link, host, stop, busy, calling, wake, senses }));
        }
        store.prune_pictures();
        {
            let (link, store, host, stop, wake) = (link.clone(), store.clone(), host.clone(), stop.clone(), wake.clone());
            thread::spawn(move || deliver_outbox(link, store, host, stop, wake));
        }
        Ok(Arc::new(Radio { jobs: Mutex::new(tx), stop, calling, link, store, host, wake, senses, roster: std::sync::atomic::AtomicU64::new(0), reading: std::sync::atomic::AtomicU64::new(0) }))
    }

    /// Ask this computer something and read its answer. A reason it gives in words (`error`) is told to him
    /// as it is; anything else that is not the answer asked for is a refusal. Every request but his own words
    /// goes through here.
    fn asked<T: serde::de::DeserializeOwned>(&self, method: &str, path: &str, body: Option<Value>) -> Result<T, RadioError> {
        let (status, v) = self.link.request(method, path, body)?;
        match (status, v["error"].as_str()) {
            (200, Some(why)) => Err(RadioError::Told { reason: why.into() }),
            (200, None) => serde_json::from_value(v).map_err(|_| RadioError::Refused),
            _ => Err(RadioError::Refused),
        }
    }

    /// A request whose answer is nothing or a reason in words.
    fn ask(&self, path: &str, body: Value) -> Result<(), RadioError> {
        self.asked::<serde::de::IgnoredAny>("POST", path, Some(body)).map(|_| ())
    }

    /// Is this computer being reached straight, on its own network, and not through the relay?
    pub(crate) fn near(&self) -> bool {
        self.link.is_direct()
    }

    /// Read a whole message aloud in the voice he chose on this device. Blocks until it has been heard or
    /// [`Radio::hush`] is called. False when it was not read: he is told why.
    pub(crate) fn read_aloud(&self, id: &str) -> bool {
        self.read(id, None)
    }
    /// Read a question to him as he would be asked it on a call: who is asking, its short form, and the
    /// answers it offers. Blocks the same way.
    pub(crate) fn read_question(&self, id: &str) -> bool {
        self.read(id, Some(airband_core::radio::Part::Asked))
    }
    fn read(&self, id: &str, part: Option<airband_core::radio::Part>) -> bool {
        let mine = self.reading.fetch_add(1, Ordering::SeqCst) + 1;
        // With no voice chosen nothing can read it: he is told where to choose one, before anything is asked
        // of his computer.
        if !voice::can_speak(&self.senses.mouth) {
            self.host.told(voice::NOTHING_CHOSEN.into());
            return false;
        }
        // His computer is asked for the words as they are read, and they are said here. When the voice he chose
        // cannot speak he is told why (`say_until` does), and nothing else reads it in its place.
        let Ok((200, v)) = self.link.request("GET", &format!("/message/{id}?{}", airband_core::radio::MessageAsk { form: Some(airband_core::radio::Form::Spoken), part }.query()), None) else { return false };
        let Some(text) = v["text"].as_str() else { return false };
        matches!(voice::say_until(&*self.host, &self.senses.mouth, text, &|| self.reading.load(Ordering::SeqCst) != mine), voice::Said::Aloud)
    }
    /// Leave off reading aloud.
    pub(crate) fn hush(&self) {
        self.reading.fetch_add(1, Ordering::SeqCst);
    }

    /// He opened the microphone himself: listen until the host closes it and hand over what he said. Nothing
    /// is sent; the caller decides where it goes.
    pub(crate) fn dictate(&self, done: Box<dyn FnOnce(Listened) + Send>) {
        let _ = self.jobs.lock().unwrap().send(Job::Dictate(done));
    }

    /// As [`Radio::queue`], for the agents he picked (by id) and for what a button of his stood for. `text`
    /// is then what the chat shows for it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_as(&self, text: String, re: Option<String>, scope: Option<String>, label: Option<String>, others: String, to: Vec<String>, act: Option<Act>) -> String {
        let act = act.map(|a| serde_json::to_string(&a).unwrap_or_default()).unwrap_or_default();
        let id = random_hex(12);
        let waiting = self.store.change(|s| {
            // whatever he attached and has reached the tower goes with this
            // the next thing he says after his computer asked him something back is his answer to it
            let answers = self.link.take_question();
            s.outbox.push(Outgoing { to, act, answers, ..Outgoing::new(id.clone(), text, re, scope, label, self.link.take_armed(), others) });
            s.outbox.clone()
        });
        self.wake.ring();
        self.host.outbox(waiting, None);
        id
    }

    /// Try again something that was given up on: it goes to the end of what is waiting, as if just said.
    pub(crate) fn send_again(&self, id: &str) {
        let waiting = self.store.change(|s| {
            if let Some(i) = s.outbox.iter().position(|o| o.id == id && o.failed.is_some()) {
                let was = s.outbox.remove(i);
                s.outbox.push(Outgoing { at: now_ms(), tries: 0, last_tried: 0, trail: Vec::new(), failed: None, stage: Stage::Trying, ..was });
            }
            s.outbox.clone()
        });
        self.wake.ring();
        self.host.outbox(waiting, None);
    }
    /// Let go of something that was not sent. Returns it, if it was there, so its words can be put back for
    /// him to change.
    pub(crate) fn discard(&self, id: &str) -> Option<Outgoing> {
        let (gone, waiting) = self.store.change(|s| {
            let gone = s.outbox.iter().position(|o| o.id == id && o.failed.is_some()).map(|i| s.outbox.remove(i));
            (gone, s.outbox.clone())
        });
        // with a note of nothing to say: it did not go, so it is not shown as on its way to the chat
        self.host.outbox(waiting, Some(String::new()));
        gone
    }
    /// Every request that failed lately and every try at sending what he said, a line each, oldest first.
    /// Nothing he or an agent said is in it. It is kept on this device and sent nowhere.
    pub(crate) fn link_trail(&self) -> Vec<String> {
        self.link.trail()
    }
    /// How many of each kind of thing the tower is told in passing did not get there, since the app started.
    pub(crate) fn lost(&self) -> Vec<(String, u32)> {
        self.link.lost()
    }
    /// How this computer was last reached, or not, each way: through the relay, and at its own address.
    pub(crate) fn reach(&self) -> (Option<String>, Option<String>) {
        let told = |s: Step| match s.outcome {
            Ok(_) => "answered".to_string(),
            Err(e) => e.told(),
        };
        (self.link.last_by(if self.link.by_relay() { Way::Relay } else { Way::Paired }).map(told), self.link.last_by(Way::Near).map(told))
    }

    /// A file he attached has reached the tower: the next thing he says or sends from this radio takes it along.
    pub fn arm(&self, id: String) {
        self.link.arm(id);
    }
    /// He took it off again before sending.
    pub fn disarm(&self, id: String) {
        self.link.disarm(&id);
    }
    /// The attached files still waiting for something to go with.
    pub fn armed(&self) -> Vec<String> {
        self.link.armed()
    }

    /// What he typed that has not been dealt with yet: on its way, with the relay, or given up on.
    pub fn unsent(&self) -> Vec<Outgoing> {
        self.store.saved.lock().unwrap().outbox.clone()
    }

    /// The recent chat as last seen, from the device: there at once, with or without a connection.
    pub fn history(&self) -> Vec<ChatMessage> {
        self.store.saved.lock().unwrap().history.iter().cloned().map(chat).collect()
    }

    /// Chat: wait up to `wait_seconds` for anything new since the last call, across all agents, and return
    /// it. Keeps its own place, on the device, so nothing is fetched twice or missed between launches.
    /// Blocks: call it in a loop off the main thread.
    pub fn follow(&self, wait_seconds: u32) -> Result<ChatFeed, RadioError> {
        let (after, since) = {
            let saved = self.store.saved.lock().unwrap();
            (saved.after, saved.rev)
        };
        let roster = self.roster.load(Ordering::SeqCst);
        // with an address of the tower's still to try, this request does not wait for news: it only finds out
        // whether the tower is there, so the straight path is in use (and shown) within a moment
        let wait = if self.link.untried() { 0 } else { wait_seconds.min(25) };
        let feed: Feed = self.asked("GET", &format!("/feed?since={since}&roster={roster}&wait={wait}"), None)?;
        spoken_alike(&feed)?;
        self.roster.store(feed.roster_rev, Ordering::SeqCst);
        self.link.set_direct(feed.direct.clone());
        // what has come of things he said: a typed message leaves the outbox, and whoever is waiting on
        // something said aloud is given its answer
        for told in &feed.answers {
            if let airband_core::radio::Outcome::Answered { reply } = &told.outcome {
                self.link.answered(&told.id, reply);
            }
            settle(&self.store, &*self.host, &told.id, told.outcome.clone());
        }
        if !feed.answers.is_empty() {
            self.wake.ring();
        }
        // the tower's log is behind ours: it was reset. Start over rather than wait for numbers that will not come.
        let reset = feed.seq < after || feed.rev < since;
        self.store.change(|s| {
            if reset {
                s.history.clear();
                s.after = 0;
            }
            // after a reset this answer held nothing (it was asked for changes since a count the tower has not
            // reached): ask again from the start, and the newest lines come at once
            s.rev = if reset { 0 } else { feed.rev };
            s.after = feed.messages.iter().map(|m| m.seq).max().unwrap_or(s.after).max(s.after);
            // a line seen before is replaced by its current form: its short version was made, or its sender renamed
            for m in &feed.messages {
                match s.history.iter_mut().find(|h| h.id == m.id) {
                    Some(have) => *have = m.clone(),
                    None => s.history.push(m.clone()),
                }
            }
            s.history.sort_by_key(|m| m.seq);
            let extra = s.history.len().saturating_sub(HISTORY_KEEP);
            s.history.drain(..extra);
        });
        Ok(chat_feed(feed))
    }

    /// Call mode: transmissions are spoken one at a time and the microphone opens after each. Off by
    /// default; with it off the radio is silent and only the chat calls below do anything.
    pub fn set_call_mode(&self, on: bool) {
        self.calling.store(on, Ordering::SeqCst);
        self.wake.ring();
        self.stands(|p| p.on_call = on);
    }

    /// Chat: everything said since `after`, all of it or one team's (`team`) or one agent's (`agent`, its id),
    /// each message cut to a line or two. Waits up to `wait_seconds` (25 at most) for something new. Blocks:
    /// call it off the main thread.
    pub fn feed(&self, team: Option<String>, agent: Option<String>, after: u64, wait_seconds: u32) -> Result<ChatFeed, RadioError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        team.iter().for_each(|t| { query.append_pair("team", t); });
        agent.iter().for_each(|a| { query.append_pair("agent", a); });
        let feed: Feed = self.asked("GET", &format!("/feed?{}&after={after}&wait={}", query.finish(), wait_seconds.min(25)), None)?;
        spoken_alike(&feed)?;
        self.link.set_direct(feed.direct.clone());
        Ok(chat_feed(feed))
    }

    /// Chat: send what he typed. Leading tags say who it is for: "@shop2 leave them", "@shop stop", "@all".
    /// With no tag it goes to whoever spoke last. Returns the tower's one-line answer ("Sent to shop two.").
    /// `re` is the id of the message he is replying to, when he picked one.
    pub fn send(&self, text: String, re: Option<String>) -> Result<String, RadioError> {
        Ok(self.link.utterance(&text, re, None, None)?.map(|reply| reply.say).unwrap_or_default())
    }

    /// Start a new agent in one of his teams, by the team's id, from a command line. Returns the tower's
    /// one-line answer.
    pub fn launch(&self, team: String, tool: Option<String>, model: Option<String>, effort: Option<String>, task: String) -> Result<String, RadioError> {
        let shown = "New agent".to_string();
        Ok(self.link.utterance(&shown, None, None, Some(Act::Launch(airband_core::radio::Launch { team, tool, model, effort, task })))?.map(|reply| reply.say).unwrap_or_default())
    }

    /// A picture, as a JPEG file on the device: fetched from the tower the first time and kept, so it is there
    /// at once afterwards and without a connection. Returns the file's path. Blocks: call it off the main thread.
    pub fn picture(&self, id: String) -> Result<String, RadioError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RadioError::Refused);
        }
        let file = self.store.pictures().join(format!("{id}.jpg"));
        if file.is_file() {
            return Ok(file.display().to_string());
        }
        let (mut bytes, mut from) = (Vec::new(), 0u32);
        loop {
            let part: airband_core::radio::PicturePart = self.asked("GET", &format!("/picture/{id}?from={from}"), None)?;
            bytes.extend(airband_core::encode::unbase64(&part.data).ok_or(RadioError::Refused)?);
            // a part that moves nothing forward would loop for ever
            if part.done || part.next <= from {
                break;
            }
            from = part.next;
        }
        airband_core::kept::write_atomic(&file, &bytes).map_err(|_| RadioError::Refused)?;
        Ok(file.display().to_string())
    }

    /// He has read these in the chat, so call mode will not also read them out.
    pub fn seen(&self, ids: Vec<String>) {
        for id in ids {
            self.link.tell("/heard", &Heard { id });
        }
    }

    /// He wants to say something unprompted (a button, Siri, a shortcut): open the microphone when free. `re` is
    /// the id of the message he means to answer, when he picked one; without it the tower works out who it is for.
    /// `scope` is the team whose room he is looking at, by name, which narrows that; nothing for all agents.
    pub fn talk(&self, re: Option<String>, scope: Option<String>) {
        let _ = self.jobs.lock().unwrap().send(Job::Talk(re, scope));
    }

    /// Stop for good. A transmission already being spoken finishes.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.ring();
    }
}

impl Drop for Radio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.ring();
    }
}

impl Radio {
    /// Whether the chat is in front of him, as the host last said.
    pub(crate) fn set_away(&self, away: bool) {
        self.stands(|p| p.showing_chat = !away);
    }
    /// Where this radio stands has changed, perhaps: every request says so from now on, and when it is news
    /// the tower is told at once, without waiting for the next thing there is to ask it.
    fn stands(&self, change: impl FnOnce(&mut airband_core::radio::Presence)) {
        if self.link.set_presence(change) {
            let link = self.link.clone();
            thread::spawn(move || link.say_presence());
        }
    }

    /// Send an answer now and wait for the tower to have it: for an answer given where there are only seconds
    /// to send it in (a notification's button, with the app behind the lock screen). If it cannot be sent in
    /// that time it is left in the outbox under the same id, to go when it can, and the caller is told.
    pub(crate) fn answer_now(&self, text: String, re: String, act: Option<Act>) -> Result<(), RadioError> {
        let id = random_hex(12);
        let mut failed = LinkError::Unreachable(Way::Relay);
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(2));
            }
            match self.link.utterance_once(&airband_core::radio::Utterance { text: text.clone(), id: id.clone(), re: Some(re.clone()), act: act.clone(), ..Default::default() }).0 {
                // left with the relay for a computer that is away: it will get there, and he should know it has not yet
                Ok(Sent::Left { away: true }) => return Err(RadioError::Told { reason: AWAY.into() }),
                // his computer has it: that is the answer given
                Ok(_) => return Ok(()),
                // it cannot be sent however often it is tried: he is told why, and nothing is left to try again
                Err(e) if e.retry() == Retry::Never => return Err(e.into()),
                Err(e) => failed = e,
            }
        }
        let waiting = self.store.change(|s| {
            s.outbox.push(Outgoing::new(id, text, Some(re), None, None, Vec::new(), String::new()));
            s.outbox.clone()
        });
        self.wake.ring();
        self.host.outbox(waiting, None);
        Err(failed.into())
    }

    /// Send a file to the tower in parts, under `id`. `sent` is told how far it has got after each part and
    /// returns false to give up. Blocks until it is all there, the tower refuses it, or it is given up on.
    pub(crate) fn upload(&self, id: &str, path: &str, name: &str, mut sent: impl FnMut(u64, u64) -> bool) -> Result<(), RadioError> {
        use std::io::{Read, Seek, SeekFrom};
        // 90 KB of file is 120 KB of text, which fits one sealed message through a relay
        const PART: u64 = 90_000;
        // Every part is a round trip to his computer and back, so they go several at once: a video is hundreds
        // of them. The first goes alone: a file his computer will not take is refused before the rest is sent.
        const AT_ONCE: usize = 6;
        let total = std::fs::metadata(path).map_err(|_| RadioError::Refused)?.len();
        let part_at = |from: u64| -> Result<airband_core::radio::AttachmentPart, RadioError> {
            let mut file = std::fs::File::open(path).map_err(|_| RadioError::Refused)?;
            file.seek(SeekFrom::Start(from)).map_err(|_| RadioError::Refused)?;
            let mut buf = Vec::with_capacity(PART as usize);
            file.take(PART).read_to_end(&mut buf).map_err(|_| RadioError::Refused)?;
            Ok(airband_core::radio::AttachmentPart { id: id.into(), name: name.into(), from, data: airband_core::encode::base64(&buf), total })
        };
        // a part lost on a bad connection is sent again, to the same place, a few times before giving up
        let send = |part: &airband_core::radio::AttachmentPart| -> Result<u64, RadioError> {
            let mut tries = 0;
            loop {
                match self.link.attachment_part(part) {
                    Ok(answer) => return Ok(answer),
                    Err(e) if e.refused() => return Err(e.into()),
                    Err(_) if tries < 6 => {
                        tries += 1;
                        thread::sleep(Duration::from_secs(tries.min(4)));
                    }
                    // a file cannot be left with the relay: it needs his computer there to take it
                    Err(_) => return Err(RadioError::Offline),
                }
            }
        };
        send(&part_at(0)?)?;
        let mut arrived = PART.min(total);
        if !sent(arrived, total) {
            return Err(RadioError::Refused);
        }
        if arrived >= total {
            return Ok(());
        }
        let starts: Vec<u64> = (1..total.div_ceil(PART)).map(|k| k * PART).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failed: Mutex<Option<RadioError>> = Mutex::new(None);
        let (tx, rx) = channel::<u64>();
        thread::scope(|scope| {
            for _ in 0..AT_ONCE.min(starts.len()) {
                let tx = tx.clone();
                let (next, failed, starts, part_at, send) = (&next, &failed, &starts, &part_at, &send);
                scope.spawn(move || {
                    while failed.lock().unwrap().is_none() {
                        let Some(from) = starts.get(next.fetch_add(1, Ordering::SeqCst)).copied() else { break };
                        match part_at(from).and_then(|part| send(&part).map(|_| part.data.len() as u64 * 3 / 4)) {
                            Ok(bytes) => {
                                let _ = tx.send(bytes);
                            }
                            Err(e) => *failed.lock().unwrap() = Some(e),
                        }
                    }
                });
            }
            drop(tx);
            // progress, as each part lands; he can take the file off while it goes
            for bytes in rx {
                arrived = (arrived + bytes).min(total);
                if !sent(arrived, total) {
                    *failed.lock().unwrap() = Some(RadioError::Refused);
                }
            }
        });
        match failed.into_inner().unwrap() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
