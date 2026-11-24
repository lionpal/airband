//! One paired computer's radio: its link, its jobs thread, and everything a host can ask of it.
use super::*;
use airband_core::radio::wire::{self, Nothing, at};
mod asks;
mod picture;
mod upload;
mod wake;
pub(crate) use upload::UploadEnd;
pub(crate) use wake::Wake;



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


/// What became of an answer given from a notification, with the words he is told when it did not go now.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum AnswerOutcome {
    /// His computer has it.
    Sent,
    /// It is kept on the phone and goes when it can.
    KeptForLater { why: String },
    /// It did not go, and why: it is kept in the chat as not delivered, to send again or let go.
    Refused { why: String },
    /// The computer it was for is no longer paired.
    NotPaired { why: String },
}
/// What he is told of an answer from a notification that his computer had and its agent did not get.
const NOT_DELIVERED_THERE: &str = "Not delivered. It\u{2019}s in the chat to try again.";
/// What follows why an answer from a notification did not go.
const KEPT_IN_CHAT: &str = "It\u{2019}s in the chat to try again.";

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
        let link = Pairing::parse(&link).map_err(|e| e.to_string()).and_then(|p| Link::new(p, &me, trail)).map_err(|reason| RadioError::BadLink { reason })?;
        let link = match transport {
            Some(transport) => link.through(transport),
            None => link,
        };
        if store.saved.lock().unwrap().relay_knew {
            link.relay_knew_before();
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let link = Arc::new(link);
        let busy = Arc::new(AtomicBool::new(false));
        let calling = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Wake::default());
        {
            let (link, session, store, wake, host, stop, busy, calling, senses) = (link.clone(), session.clone(), store.clone(), wake.clone(), host.clone(), stop.clone(), busy.clone(), calling.clone(), senses.clone());
            thread::spawn(move || work(call::Working { jobs: rx, link, session, store, wake, host, stop, busy, calling, senses }));
        }
        {
            let (tx, stop, link, calling, host, wake, senses) = (tx.clone(), stop.clone(), link.clone(), calling.clone(), host.clone(), wake.clone(), senses.clone());
            thread::spawn(move || poll(call::Polling { jobs: tx, link, host, stop, busy, calling, wake, senses }));
        }
        store.prune_pictures();
        {
            let (link, session, store, host, stop, wake, senses) = (link.clone(), session.clone(), store.clone(), host.clone(), stop.clone(), wake.clone(), senses.clone());
            thread::spawn(move || deliver_outbox(link, session, store, host, stop, wake, senses));
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
            self.host.told(voice::nothing_chosen());
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
    pub(crate) fn dictate(&self, aim: crate::takes::Aim, done: Box<dyn FnOnce(Listened) + Send>) {
        self.jobs.lock().unwrap().send(Job::Dictate(aim, done)).ok();
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
        self.enqueue(out);
        id
    }

    /// Put it at the end of what is waiting to go ([`enqueue`]).
    fn enqueue(&self, out: Outgoing) {
        enqueue(&self.store, &self.wake, &*self.host, out);
    }

    /// Try again something that was not delivered: it goes to the end of what is waiting, as if just said. One
    /// his computer answered before goes under `fresh` ([`Outgoing::again`]); `to`, when given, is who it is
    /// for now (an agent just taken over). Returns the id it now waits under, if it was there.
    pub(crate) fn send_again(&self, id: &str, fresh: &str, to: Option<String>) -> Option<String> {
        let (now, waiting) = self.store.change(|s| {
            let now = s.outbox.iter().position(|o| o.id == id && o.undelivered.is_some()).map(|i| {
                let was = s.outbox.remove(i);
                let under = if was.undelivered.as_ref().is_some_and(NotDelivered::answered) { fresh.to_string() } else { was.id.clone() };
                let again = was.again(under);
                let again = match &to {
                    Some(agent) => Outgoing { to: vec![agent.clone()], ..again },
                    None => again,
                };
                let under = again.id.clone();
                s.outbox.push(again);
                under
            });
            (now, s.outbox.clone())
        });
        self.wake.ring();
        self.host.outbox(waiting, None);
        now
    }
    /// The message waiting under this id, if it is here.
    pub(crate) fn unsent_one(&self, id: &str) -> Option<Outgoing> {
        self.store.saved.lock().unwrap().outbox.iter().find(|o| o.id == id).cloned()
    }
    /// Let go of something that was not sent. Returns it, if it was there, so its words can be put back for
    /// him to change.
    pub(crate) fn discard(&self, id: &str) -> Option<Outgoing> {
        let (gone, waiting) = self.store.change(|s| {
            let gone = s.outbox.iter().position(|o| o.id == id && o.undelivered.is_some()).map(|i| s.outbox.remove(i));
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
        self.followed(wait_seconds).map_err(Into::into)
    }

    /// The same, with what went wrong as it was: the desk tells a computer that refuses this phone from one
    /// it cannot reach.
    pub(crate) fn followed(&self, wait_seconds: u32) -> Result<ChatFeed, LinkError> {
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
            settle(&self.store, &*self.host, &self.senses.takes, &told.id, told.outcome.clone());
        }
        if !feed.answers.is_empty() {
            self.wake.ring();
        }
        // the tower's log is behind ours: it was reset. Start over rather than wait for numbers that will not come.
        let reset = feed.seq < after || feed.rev < since;
        self.store.change(|s| {
            s.relay_knew = self.link.relay_knew(s.relay_knew);
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
        Ok(self.session.say(&self.link, &text, re, None, None)?.said())
    }

    /// Start a new agent in one of his teams, by the team's id, from a command line. Returns the tower's
    /// one-line answer.
    pub fn launch(&self, team: String, tool: Option<String>, model: Option<String>, effort: Option<String>, task: String) -> Result<String, RadioError> {
        let shown = "New agent".to_string();
        Ok(self.session.say(&self.link, &shown, None, None, Some(Act::Launch(airband_core::radio::Launch { team: airband_core::ids::TeamId::new(team), tool, model, effort, task })))?.said())
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
                // Left with the relay for a computer that is away, or behind earlier words of his: it will get
                // there, and he should know it has not yet. The outbox follows it until his computer has it.
                Ok(Sent::Left { away }) => {
                    self.enqueue(Outgoing { stage: Stage::Left, ..out });
                    return AnswerOutcome::KeptForLater { why: if away { AWAY.into() } else { LinkError::Behind.said() } };
                }
                // an answer to a request for leave that had ended: it did nothing, and he is told so where he gave it
                Ok(Sent::Answered(reply)) if reply.request_over => return AnswerOutcome::Refused { why: reply.say },
                // his computer has it: that is the answer given, unless it says the agent did not get it. Then
                // it is kept in the chat as not delivered, as anything of his is
                Ok(Sent::Answered(reply)) if crate::outbox::keep_answered(&self.store, &*self.host, &self.senses.takes, out.clone(), &reply) => {
                    return AnswerOutcome::Refused { why: NOT_DELIVERED_THERE.into() };
                }
                Ok(Sent::Taken | Sent::Answered(_)) => return AnswerOutcome::Sent,
                // it cannot be sent however often it is tried: he is told why, and it is kept in the chat as not
                // delivered, to send again or let go
                Err(e) if e.retry() == Retry::Never => {
                    let why = RadioError::from(e).said();
                    self.enqueue(Outgoing { undelivered: Some(NotDelivered::Refused { why: why.clone() }), ..out });
                    return AnswerOutcome::Refused { why: format!("{why} {KEPT_IN_CHAT}") };
                }
                Err(e) => failed = e,
            }
        }
        self.enqueue(out);
        AnswerOutcome::KeptForLater { why: format!("{} It is on this phone and goes when it can.", failed.said()) }
    }

}
