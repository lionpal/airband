//! One paired computer's radio: its link, its jobs thread, and everything a host can ask of it.
use super::*;
use airband_core::radio::wire::{self, Nothing, at};
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
    /// How many times it has been rung: taken before looking at what there is to do, so a ring that comes
    /// after the look and before the sleep still wakes it (`wait_since`).
    pub(crate) fn turn(&self) -> u64 {
        *self.turn.lock().unwrap()
    }
    /// Sleep until rung since `was` (from [`Self::turn`]), or for `at_most`. Returns at once if it was rung
    /// meanwhile.
    pub(crate) fn wait_since(&self, was: u64, at_most: Duration) {
        let turn = self.turn.lock().unwrap();
        self.rung.wait_timeout_while(turn, at_most, |now| *now == was).ok();
    }
}

/// What is kept for him on one computer's behalf and outlasts the radio talking to it: what he sends next takes
/// (the files he attached, the question it answers), and what is saved on the device (his unsent messages, the
/// recent chat). A desk starts a new radio for a computer on every reconnect and hands it the same, so nothing he
/// was about to send is lost, and only one copy of the saved file is ever written.
#[derive(Clone)]
pub(crate) struct Kept {
    pub(crate) session: Arc<Session>,
    pub(crate) store: Arc<Store>,
    /// The record of how requests went, beside the saved chat on this device.
    pub(crate) trail: Arc<crate::link::Trail>,
}
impl Kept {
    pub(crate) fn open(data_dir: &str, host: Arc<dyn RadioHost>) -> Kept {
        let trail = crate::link::Trail::open((!data_dir.is_empty()).then(|| std::path::Path::new(data_dir).join("trail.json")));
        Kept { session: Arc::default(), store: Arc::new(Store::open(data_dir, Some(host))), trail }
    }
}

pub struct Radio {
    pub(crate) jobs: Mutex<Sender<Job>>,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) calling: Arc<AtomicBool>,
    pub(crate) link: Arc<Link>,
    /// What the next thing he says takes with it: files he attached, the question it answers.
    pub(crate) session: Arc<Session>,
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

impl Radio {
    /// Start working with the tower this pairing link leads to (the link `airband pair` prints). Returns at
    /// once; fails only if the link is not a pairing link. `data_dir` is a folder of the app's own where
    /// unsent messages and the recent chat are kept across restarts; empty keeps nothing.
    pub fn start(link: String, data_dir: String, host: Arc<dyn RadioHost>) -> Result<Arc<Self>, RadioError> {
        // a radio on its own is a text radio (a terminal): what is said is shown by the host and what he says
        // is a line he types. That is what it is started with, as its choice for each.
        let senses = Senses::default();
        senses.mouth.set(Some(mouth::Own::Host));
        senses.ears.set(Some(ears::Own::Host));
        let kept = Kept::open(&data_dir, host.clone());
        Self::start_with(link, host, Arc::new(senses), kept, None)
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
        self.queue_as(Outgoing { re, scope, label, ..Outgoing::of(text) }, Carry::Answer)
    }
}

/// What a message in the outbox takes with it from what he has armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Carry {
    /// His own next words: the files he attached and the question he was asked go with them.
    Answer,
    /// Not his next words: it goes as it is.
    Nothing,
}

/// How sending a file ended when nothing went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadEnd {
    /// All of it is on his computer.
    Arrived,
    /// He took it off before it was all there.
    TakenOff,
}

/// What became of an answer given from a notification, with the words he is told when it did not go now.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AnswerOutcome {
    /// His computer has it.
    Sent,
    /// It is kept on the phone and goes when it can.
    KeptForLater { why: String },
    /// It will never go as it is; nothing is kept.
    Refused { why: String },
    /// The computer it was for is no longer paired.
    NotPaired { why: String },
}
impl AnswerOutcome {
    /// Whether the answer is given, now or later: only then is the question no longer waiting on him.
    pub(crate) fn given(&self) -> bool {
        matches!(self, AnswerOutcome::Sent | AnswerOutcome::KeptForLater { .. })
    }
}

impl Radio {
    /// As [`Radio::start`], listening and speaking with what he chose on this device, and with what is kept for
    /// him on this computer's behalf ([`Kept`]).
    /// `transport`, when given, carries every request in place of the network ([`crate::link::Transport`]).
    pub(crate) fn start_with(link: String, host: Arc<dyn RadioHost>, senses: Arc<Senses>, kept: Kept, transport: Option<Arc<dyn crate::link::Transport>>) -> Result<Arc<Self>, RadioError> {
        let Kept { session, store, trail } = kept;
        let me = store.saved.lock().unwrap().radio.clone();
        let me = if me.is_empty() { store.change(|s| { s.radio = format!("radio-{}", random_hex(4)); s.radio.clone() }) } else { me };
        let link = Pairing::parse(&link).and_then(|p| Link::new(p, &me, trail)).map_err(|reason| RadioError::BadLink { reason })?;
        let link = match transport {
            Some(transport) => link.through(transport),
            None => link,
        };
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let link = Arc::new(link);
        let busy = Arc::new(AtomicBool::new(false));
        let calling = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Wake::default());
        {
            let (link, session, host, stop, busy, calling, senses) = (link.clone(), session.clone(), host.clone(), stop.clone(), busy.clone(), calling.clone(), senses.clone());
            thread::spawn(move || work(call::Working { jobs: rx, link, session, host, stop, busy, calling, senses }));
        }
        {
            let (tx, stop, link, calling, host, wake, senses) = (tx.clone(), stop.clone(), link.clone(), calling.clone(), host.clone(), wake.clone(), senses.clone());
            thread::spawn(move || poll(call::Polling { jobs: tx, link, host, stop, busy, calling, wake, senses }));
        }
        store.prune_pictures();
        {
            let (link, session, store, host, stop, wake) = (link.clone(), session.clone(), store.clone(), host.clone(), stop.clone(), wake.clone());
            thread::spawn(move || deliver_outbox(link, session, store, host, stop, wake));
        }
        Ok(Arc::new(Radio { jobs: Mutex::new(tx), stop, calling, link, session, store, host, wake, senses, roster: std::sync::atomic::AtomicU64::new(0), reading: std::sync::atomic::AtomicU64::new(0) }))
    }

    /// What this radio keeps for him that outlasts it: the next radio started for this computer takes it over.
    pub(crate) fn kept(&self) -> Kept {
        Kept { session: self.session.clone(), store: self.store.clone(), trail: self.link.trail.clone() }
    }

    /// Is this computer being reached straight, on its own network, and not through the relay?
    pub(crate) fn near(&self) -> bool {
        self.link.is_direct()
    }

    /// Read a question to him as he would be asked it on a call: who is asking, its short form, and the
    /// answers it offers, in the voice he chose on this device. Blocks until it has been heard or
    /// [`Radio::hush`] is called. False when it was not read: he is told why.
    pub(crate) fn read_question(&self, id: &str) -> bool {
        let part = Some(airband_core::radio::Part::Asked);
        let mine = self.reading.fetch_add(1, Ordering::SeqCst) + 1;
        // With no voice chosen nothing can read it: he is told where to choose one, before anything is asked
        // of his computer.
        if !self.senses.mouth.ready() {
            self.host.told(voice::NOTHING_CHOSEN.into());
            return false;
        }
        // His computer is asked for the words as they are read, and they are said here. When the voice he chose
        // cannot speak he is told why (`say_until` does), and nothing else reads it in its place.
        let words = match self.link.call::<at::Message>(Some(id), &wire::MessageQuery { form: Some(airband_core::radio::Form::Spoken), part }, &Nothing {}) {
            Ok(words) => words,
            Err(e) => {
                self.host.told(match RadioError::from(e) {
                    RadioError::Told { reason } => reason,
                    e => format!("That could not be read aloud. {}", e.said()),
                });
                return false;
            }
        };
        matches!(voice::say_until(&*self.host, &self.senses.mouth, &words.text, &|| self.reading.load(Ordering::SeqCst) != mine), voice::Said::Aloud)
    }
    /// Leave off reading aloud.
    pub(crate) fn hush(&self) {
        self.reading.fetch_add(1, Ordering::SeqCst);
    }

    /// He opened the microphone himself: listen until the host closes it and hand over what he said. Nothing
    /// is sent; the caller decides where it goes.
    pub(crate) fn dictate(&self, done: Box<dyn FnOnce(Listened) + Send>) {
        self.jobs.lock().unwrap().send(Job::Dictate(done)).ok();
    }

    /// Put something in the outbox to go to the tower. What it takes with it is said by `carry`: his own next
    /// words take whatever he attached that has reached the tower and answer the question his computer last
    /// asked him back; anything else (a button, a command, words sent on to another agent) goes as it is and
    /// leaves both for what he says next. Returns the id it waits under.
    pub(crate) fn queue_as(&self, out: Outgoing, carry: Carry) -> String {
        let mut out = out;
        if carry == Carry::Answer {
            out.attachments.extend(self.session.take_armed());
            out.answers = out.answers.or_else(|| self.session.take_question());
        }
        let id = out.id.clone();
        let waiting = self.store.change(|s| {
            s.outbox.push(out);
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
    pub(crate) fn lost(&self) -> Vec<(&'static str, u32)> {
        self.link.lost()
    }
    /// How this computer was last reached, or not, each way: through the relay, and at its own address.
    pub(crate) fn reach(&self) -> (Option<String>, Option<String>) {
        let told = |s: Step| match s.outcome {
            Ok(_) => "Answered.".to_string(),
            Err(e) => e.said(),
        };
        (self.link.last_by(if self.link.by_relay() { Way::Relay } else { Way::Paired }).map(told), self.link.last_by(Way::Near).map(told))
    }

    /// A file he attached has reached the tower: the next thing he says or sends from this radio takes it along.
    pub(crate) fn arm(&self, id: String) {
        self.session.arm(id);
    }
    /// He took it off again before sending.
    pub(crate) fn disarm(&self, id: String) {
        self.session.disarm(&id);
    }
    /// The attached files still waiting for something to go with.
    pub(crate) fn armed(&self) -> Vec<String> {
        self.session.armed()
    }

    /// What he typed that has not been dealt with yet: on its way, with the relay, or given up on.
    pub fn unsent(&self) -> Vec<Outgoing> {
        self.store.saved.lock().unwrap().outbox.clone()
    }

    /// The recent chat as last seen, from the device: there at once, with or without a connection.
    pub(crate) fn history(&self) -> Vec<ChatMessage> {
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
        let wait = if self.link.untried() { 0 } else { wait_seconds };
        let ask = wire::FeedQuery { since: Some(since), roster: Some(roster), wait, ..Default::default() };
        let feed = self.link.call::<at::Feed>(None, &ask, &Nothing {})?;
        self.roster.store(feed.roster_rev, Ordering::SeqCst);
        self.link.set_direct(feed.direct.clone());
        // what has come of things he said: a typed message leaves the outbox, and whoever is waiting on
        // something said aloud is given its answer
        for told in &feed.answers {
            if let airband_core::radio::Outcome::Answered { reply } = &told.outcome {
                self.session.asked(reply);
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
        let ask = wire::FeedQuery { team: team.map(airband_core::ids::TeamId::new), agent: agent.map(airband_core::ids::AgentId::new), after, wait: wait_seconds, ..Default::default() };
        let feed = self.link.call::<at::Feed>(None, &ask, &Nothing {})?;
        self.link.set_direct(feed.direct.clone());
        Ok(chat_feed(feed))
    }

    /// Chat: send what he typed. Leading tags say who it is for: "@shop2 leave them", "@shop stop", "@all".
    /// With no tag it goes to whoever spoke last. Returns the tower's one-line answer ("Sent to shop two.").
    /// `re` is the id of the message he is replying to, when he picked one.
    pub fn send(&self, text: String, re: Option<String>) -> Result<String, RadioError> {
        Ok(self.session.say(&self.link, &text, re, None, None)?.map(|reply| reply.say).unwrap_or_default())
    }

    /// Start a new agent in one of his teams, by the team's id, from a command line. Returns the tower's
    /// one-line answer.
    pub fn launch(&self, team: String, tool: Option<String>, model: Option<String>, effort: Option<String>, task: String) -> Result<String, RadioError> {
        let shown = "New agent".to_string();
        Ok(self.session.say(&self.link, &shown, None, None, Some(Act::Launch(airband_core::radio::Launch { team: airband_core::ids::TeamId::new(team), tool, model, effort, task })))?.map(|reply| reply.say).unwrap_or_default())
    }

    /// A picture, as a JPEG file on the device: fetched from the tower the first time and kept, so it is there
    /// at once afterwards and without a connection. Returns the file's path. Blocks: call it off the main thread.
    pub(crate) fn picture(&self, id: String) -> Result<String, RadioError> {
        // a picture's id is the hex its computer gave it: anything else names no picture there
        if !airband_core::encode::is_hex(&id, 1..=128) {
            return Err(RadioError::gone("picture"));
        }
        let file = self.store.pictures().join(format!("{id}.jpg"));
        if file.is_file() {
            return Ok(file.display().to_string());
        }
        let (mut bytes, mut from) = (Vec::new(), 0u32);
        loop {
            let part = self.link.call::<at::Picture>(Some(&id), &wire::PartQuery { from: from as usize }, &Nothing {})?;
            bytes.extend(airband_core::encode::unbase64(&part.data).ok_or_else(|| crate::choices::told("Your computer sent a picture that could not be read."))?);
            // a part that moves nothing forward would loop for ever
            if part.done || part.next <= from {
                break;
            }
            from = part.next;
        }
        airband_io::files::write_atomic(&file, &bytes).map_err(|_| RadioError::here("keep that picture"))?;
        Ok(file.display().to_string())
    }

    /// He has read these in the chat, so call mode will not also read them out.
    pub(crate) fn seen(&self, ids: Vec<String>) {
        for id in ids {
            self.link.tell::<at::Heard>(&Heard { id });
        }
    }

    /// He wants to say something unprompted (a button, Siri, a shortcut, a line typed in the terminal host):
    /// open the microphone when free. `re` is the id of the message he means to answer, when he picked one;
    /// without it the tower works out who it is for. `scope` is the team whose room he is looking at, which
    /// narrows that; nothing for all agents.
    pub fn talk(&self, re: Option<String>, scope: Option<String>) {
        self.jobs.lock().unwrap().send(Job::Talk(re, scope)).ok();
    }

    /// Stop for good. A transmission already being spoken finishes.
    pub(crate) fn stop(&self) {
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
    pub(crate) fn answer_now(&self, text: String, re: String, act: Option<Act>) -> AnswerOutcome {
        // one record of it, tried now and kept as it is: a button's answer stays what the button stood for
        let out = Outgoing::new(random_hex(12), text, Some(re), None, None, Vec::new(), Vec::new()).with_act(act.as_ref());
        let u = out.utterance();
        let mut failed = LinkError::Unreachable(Way::Relay);
        for attempt in 0..3 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(2));
            }
            match self.link.utterance_once(&u).0 {
                // left with the relay for a computer that is away: it will get there, and he should know it has not yet
                Ok(Sent::Left { away: true }) => return AnswerOutcome::KeptForLater { why: AWAY.into() },
                // his computer has it: that is the answer given
                Ok(_) => return AnswerOutcome::Sent,
                // it cannot be sent however often it is tried: he is told why, and nothing is left to try again
                Err(e) if e.retry() == Retry::Never => return AnswerOutcome::Refused { why: RadioError::from(e).said() },
                Err(e) => failed = e,
            }
        }
        let waiting = self.store.change(|s| {
            s.outbox.push(out);
            s.outbox.clone()
        });
        self.wake.ring();
        self.host.outbox(waiting, None);
        AnswerOutcome::KeptForLater { why: format!("{} It is on this phone and goes when it can.", failed.said()) }
    }

    /// Send a file to the tower in parts, under `id`. `sent` is told how far it has got after each part and
    /// returns false to give up. Blocks until it is all there, the tower refuses it, or it is given up on.
    pub(crate) fn upload(&self, id: &str, path: &str, name: &str, mut sent: impl FnMut(u64, u64) -> bool) -> Result<UploadEnd, RadioError> {
        // 90 KB of file is 120 KB of text, which fits one sealed message through a relay
        const PART: u64 = 90_000;
        // Every part is a round trip to his computer and back, so they go several at once: a video is hundreds
        // of them. The first goes alone: a file his computer will not take is refused before the rest is sent.
        const AT_ONCE: usize = 6;
        let unread = |_| RadioError::here("read that file");
        let total = std::fs::metadata(path).map_err(unread)?.len();
        let part_at = |from: u64| -> Result<airband_core::radio::AttachmentPart, RadioError> {
            let buf = airband_io::files::read_part(std::path::Path::new(path), from, PART).map_err(unread)?;
            Ok(airband_core::radio::AttachmentPart { id: id.into(), name: name.into(), from, data: airband_core::encode::base64(&buf), total })
        };
        // a part lost on a bad connection is sent again, to the same place, a few times before giving up
        let send = |part: &airband_core::radio::AttachmentPart| -> Result<u64, RadioError> {
            let mut tries = 0;
            loop {
                match self.link.attachment_part(part) {
                    Ok(answer) => return Ok(answer),
                    // a no, or what will never go however often it is sent (more than the relay carries): at once
                    Err(e) if e.retry() == Retry::Never => return Err(e.into()),
                    Err(_) if tries < 6 => {
                        tries += 1;
                        thread::sleep(Duration::from_secs(tries.min(4)));
                    }
                    // a file cannot be left with the relay: it needs his computer there to take it
                    Err(e) => return Err(e.into()),
                }
            }
        };
        send(&part_at(0)?)?;
        let mut arrived = PART.min(total);
        if !sent(arrived, total) {
            return Ok(UploadEnd::TakenOff);
        }
        if arrived >= total {
            return Ok(UploadEnd::Arrived);
        }
        let starts: Vec<u64> = (1..total.div_ceil(PART)).map(|k| k * PART).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let failed: Mutex<Option<RadioError>> = Mutex::new(None);
        let taken_off = std::sync::atomic::AtomicBool::new(false);
        let (tx, rx) = channel::<u64>();
        thread::scope(|scope| {
            for _ in 0..AT_ONCE.min(starts.len()) {
                let tx = tx.clone();
                let (next, failed, starts, part_at, send, taken_off) = (&next, &failed, &starts, &part_at, &send, &taken_off);
                scope.spawn(move || {
                    while failed.lock().unwrap().is_none() && !taken_off.load(Ordering::SeqCst) {
                        let Some(from) = starts.get(next.fetch_add(1, Ordering::SeqCst)).copied() else { break };
                        match part_at(from).and_then(|part| send(&part).map(|_| part.data.len() as u64 * 3 / 4)) {
                            Ok(bytes) => {
                                tx.send(bytes).ok();
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
                    taken_off.store(true, Ordering::SeqCst);
                }
            }
        });
        match failed.into_inner().unwrap() {
            Some(e) => Err(e),
            None if taken_off.into_inner() => Ok(UploadEnd::TakenOff),
            None => Ok(UploadEnd::Arrived),
        }
    }
}
