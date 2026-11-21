//! What the tower remembers, and the rules about it: who is on the radio and what they are called, the chat log,
//! what is unheard, and each radio's place in a call. No I/O and no model here; everything that changes these
//! goes through a method below, and the rules are tested at the bottom of this file.
//!
//! An agent is known by its id, which never changes. Its callsign is only what it is called: it can change (he
//! names the session), and a later agent can be given a callsign an earlier one had. So a message records the id
//! of the agent that said it, and every question about "which agent" is answered by id.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Session {
    pub(super) agent: Agent,
    pub(super) callsign: Callsign,
    /// It finished a turn and is waiting on him.
    pub(super) waiting: bool,
    /// The line of the last thing he heard from it, for "say more" and "repeat".
    #[serde(default)]
    pub(super) last_heard: Option<String>,
    /// Pictures it has touched since it last spoke; they go with what it says next.
    #[serde(default)]
    pub(super) pictures: Vec<pictures::Seen>,
    /// Subagents it has running.
    #[serde(default)]
    pub(super) subagents: u32,
    /// What it is doing right now, while it works: its harness's words for its latest step. Never kept.
    #[serde(skip)]
    pub(super) doing: Option<String>,
    /// The model it runs on and how hard it is set to think, as last told. Kept, so one that has been waiting
    /// since before the tower restarted still shows them.
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) effort: Option<String>,
    /// Who put it in a team other than its folder's, if anyone did. One he placed stays where he put it.
    #[serde(default)]
    pub(super) moved: Option<Mover>,
    /// How many times it has reported. The tower looks at where it belongs on some of them.
    #[serde(default)]
    pub(super) reports: u32,
    /// How far into its harness's record the tower has read what he typed to it at its own keyboard.
    #[serde(default)]
    pub(super) told_upto: u64,
    /// For one the tower runs itself: what he has said to it while it was in the middle of something, by the
    /// id of the chat line and the words. Held here, where he can still take it back, until it stops.
    #[serde(default)]
    pub(super) queue: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(super) enum Mover {
    Him,
    Tower,
}

/// Where something an agent said stands with being read out to him on a call. It is kept on the line itself, so
/// there is one record of a message and of whether he has heard it. [`Inner::set_aloud`] is the only thing that
/// changes it, and it writes down every change with its reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum Aloud {
    /// Not for reading out: his own words.
    #[default]
    No,
    /// To be read out when a radio on a call is free.
    Waiting,
    /// Given to a radio to read out. It has not yet said whether he heard it.
    Handed { radio: String, at: u64 },
    Heard { radio: String, at: u64 },
    /// It will not be read out, and why.
    Dropped { why: Dropped },
}

/// Why something an agent said is not read out.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Dropped {
    /// The judge found nothing in it for his ears.
    Routine,
    /// Its agent said something newer before he heard it.
    Superseded,
    /// It was waiting on him, and he has since spoken to its agent, or the agent has gone back to work.
    Answered,
    /// He let the question go.
    Dismissed,
    /// It was waiting on him, and its agent has gone.
    AgentGone,
    /// It was said long before he turned reading aloud on.
    Stale,
    /// The request for leave it put to him is over.
    PermissionEnded,
}

impl Aloud {
    /// Still to be heard: waiting its turn, or with a radio that has not said he heard it.
    pub(super) fn unheard(&self) -> bool {
        matches!(self, Aloud::Waiting | Aloud::Handed { .. })
    }
    /// For the log: the state in a word or two.
    pub(super) fn words(&self) -> String {
        match self {
            Aloud::No => "not for reading".into(),
            Aloud::Waiting => "waiting".into(),
            Aloud::Handed { radio, .. } => format!("handed to {radio}"),
            Aloud::Heard { radio, .. } => format!("heard on {radio}"),
            Aloud::Dropped { why } => format!("dropped ({})", serde_json::to_value(why).ok().and_then(|v| v.as_str().map(|w| w.replace('_', " "))).unwrap_or_default()),
        }
    }
}

/// What the judge is given to read: one thing an agent said, as it stands in the log.
#[derive(Clone)]
pub(super) struct Item {
    pub(super) id: String,
    /// It needs him at the keyboard; a spoken reply cannot help.
    pub(super) notice: bool,
    pub(super) full: String,
    pub(super) kind: Option<Kind>,
    pub(super) say: Option<String>,
    /// The last thing he told this agent, if anything: what the judge needs to tell "done what you asked" from
    /// chatter along the way.
    pub(super) asked: Option<String>,
}

/// One line of the chat log. For an agent's message, `aloud` says where it stands with being read out.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) seq: u64,
    pub(super) id: String,
    pub(super) at: u64,
    /// What the sender was called when it said this, or "you". For showing a message whose agent has gone.
    pub(super) from: String,
    /// The project it belongs to.
    pub(super) group: String,
    /// For his messages: what the agents it went to were called.
    pub(super) to: Vec<String>,
    pub(super) kind: Option<Kind>,
    /// The short version, once it has been made.
    pub(super) short: Option<String>,
    pub(super) full: String,
    pub(super) detail: Option<String>,
    /// Answers he can give with one tap, when the judge found some.
    #[serde(default)]
    pub(super) options: Vec<String>,
    /// Which of them the agent itself recommended, if it did.
    #[serde(default)]
    pub(super) recommended: Option<u32>,
    /// What it holds that he should be able to read whole, by name, if the judge found something.
    #[serde(default)]
    pub(super) read: Option<String>,
    /// The agent said it takes this answer only from him in its own session (the judge read that in it).
    #[serde(default)]
    pub(super) own_session: bool,
    /// His words, waiting for the agent to stop what it is doing; and whether he took them back first.
    /// The agent that said this had stopped only to wait for work of its own still running. While this is the
    /// last thing it said, it counts as at work.
    #[serde(default)]
    pub(super) background: bool,
    /// For his words: when the agent took them in, once its harness shows that.
    #[serde(default)]
    pub(super) seen: Option<u64>,
    /// He let this question go without answering it.
    #[serde(default)]
    pub(super) dismissed: bool,
    #[serde(default)]
    pub(super) queued: bool,
    #[serde(default)]
    pub(super) cancelled: bool,
    /// Pictures that go with it.
    #[serde(default)]
    pub(super) pictures: Vec<airband_core::radio::Picture>,
    /// The id of the agent that said it. Nothing for his own messages.
    #[serde(default)]
    pub(super) agent: Option<String>,
    /// For his messages: the ids of the agents it went to.
    #[serde(default)]
    pub(super) to_agents: Vec<String>,
    /// The permission request this line puts to him, while it is waiting for his answer.
    #[serde(default, skip)]
    pub(super) permit: Option<String>,
    /// This line is, or was, a request for leave to do something on his computer. It stays true after the
    /// request has ended, so a late answer to it is known for what it is and never goes to the agent as words.
    #[serde(default)]
    pub(super) leave: bool,
    /// When this line last changed, as a count that only goes up across the whole log. A radio asks for
    /// everything changed since the count it last saw.
    #[serde(default)]
    pub(super) rev: u64,
    /// Where it stands with being read out on a call.
    #[serde(default)]
    pub(super) aloud: Aloud,
    /// It is a notice from the harness that the agent needs him at the keyboard: read to him as it is.
    #[serde(default)]
    pub(super) notice: bool,
}

impl Entry {
    /// Mask any credential in the words this line keeps. Returns whether there was one.
    pub(super) fn scrub(&mut self) -> bool {
        use airband_core::redact::redact;
        let mut changed = false;
        let mut clean = |t: &mut String| {
            let r = redact(t);
            if r != *t {
                *t = r;
                changed = true;
            }
        };
        clean(&mut self.full);
        if let Some(t) = self.short.as_mut() {
            clean(t);
        }
        if let Some(t) = self.detail.as_mut() {
            clean(t);
        }
        changed
    }

    /// Something an agent said.
    pub(super) fn said(id: String, s: &Session, kind: Option<Kind>, full: String) -> Entry {
        Entry { id, from: s.callsign.text(), group: s.callsign.project.clone(), kind, full, agent: Some(s.agent.id.clone()), ..Entry::default() }
    }
    /// Something he said, to these agents.
    pub(super) fn his(id: String, to: &[Session], text: String) -> Entry {
        let projects: HashSet<&String> = to.iter().map(|s| &s.callsign.project).collect();
        let group = if projects.len() == 1 { to[0].callsign.project.clone() } else { String::new() };
        Entry { id, from: "you".into(), group, to: to.iter().map(|s| s.callsign.text()).collect(), to_agents: to.iter().map(|s| s.agent.id.clone()).collect(), short: Some(text.clone()), full: text, ..Entry::default() }
    }
}

/// One radio's place in a call. Each radio has its own, so a phone and a desktop app on a call at the same time
/// do not take each other's transmissions or answer each other's questions.
/// A new agent he asked for: where, on what, and to do what. The project is nothing until it is known; the
/// model and effort are names the launcher offers, or nothing for its own defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Idea {
    /// The harness he asked for by name ("a codex agent"), when he did and a launcher starts it.
    pub(super) tool: Option<String>,
    pub(super) project: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) task: String,
}

#[derive(Default)]
pub(super) struct Call {
    /// The transmission the next `GET next` from this radio will take.
    pub(super) outbox: Option<Transmission>,
    pub(super) ready: bool,
    pub(super) epoch: u64,
    /// He said "stand by".
    pub(super) held: bool,
    /// The agent that last spoke on this radio or was last addressed from it.
    pub(super) focus: Option<String>,
    /// Whether it is there.
    pub(super) presence: Presence,
    /// What it took to read out and has not answered for yet (heard, not heard, or ready for the next). If the
    /// answer that carried it was lost on the way, it is given the same thing again.
    pub(super) handed: Option<Transmission>,
    /// When this radio last did anything at all: a radio that only reads the feed and types never polls.
    pub(super) seen: Option<Instant>,
    /// Words he said that the tower could not place, held while it asks him who they are for.
    pub(super) asked: Option<(String, String, Instant)>,
    /// An idea for a new agent whose project the tower could not work out, held while it asks him which.
    pub(super) unplaced_idea: Option<(Idea, Instant)>,
    /// How far this radio has the chat on screen: the highest change count it has asked for what follows of,
    /// while it was showing the chat. A line changed after this is one he may not have seen.
    pub(super) has: u64,
    /// When he last joined (turned listening on), in milliseconds since 1970. A call starts from there: he is
    /// not read what was asked before it.
    pub(super) began: u64,
    /// This radio makes the speech itself, from the words: the tower makes none for it.
    pub(super) speaks: bool,
    /// It was handed something and no sound came out of it: nothing more is handed to it before this moment,
    /// so a voice that is not answering does not run the whole queue past him in silence.
    pub(super) quiet_until: Option<Instant>,
}

/// Whether a radio is there, as it last showed. Every rule that turns on "is he there" reads this, by the one
/// clock [`THERE`]: who can be handed something to read out, whether there is anyone to ask for leave, whether
/// a notification is needed, whether it is worth asking the harnesses what their agents are doing.
#[derive(Default)]
pub(super) struct Presence {
    /// When it last asked for something to read out, or said it was still on the call.
    polled: Option<Instant>,
    /// When it last asked for the chat.
    followed: Option<Instant>,
    /// When it last asked for the chat with the chat in front of him.
    shown: Option<Instant>,
}

impl Presence {
    /// It is on a call: it has asked for something to read out lately.
    pub(super) fn on_call(&self) -> bool {
        self.polled.is_some_and(|t| t.elapsed() < THERE)
    }
    /// It is following the chat.
    pub(super) fn following(&self) -> bool {
        self.followed.is_some_and(|t| t.elapsed() < THERE)
    }
    /// It has the chat in front of him. One that follows the chat with the app put away does not.
    pub(super) fn showing(&self) -> bool {
        self.shown.is_some_and(|t| t.elapsed() < THERE)
    }
    pub(super) fn shows(&mut self) {
        self.shown = Some(Instant::now());
    }
    pub(super) fn polls(&mut self) {
        self.polled = Some(Instant::now());
    }
    /// It says it is still on the call while it asks for nothing (he has the microphone open). Nothing for a
    /// radio that was never on one.
    pub(super) fn still_on_call(&mut self) {
        if self.polled.is_some() {
            self.polls();
        }
    }
    pub(super) fn follows(&mut self) {
        self.followed = Some(Instant::now());
    }
}

impl Call {
    pub(super) fn connected(&self) -> bool {
        self.presence.on_call()
    }
    pub(super) fn can_send(&self) -> bool {
        self.connected() && self.ready && !self.held && self.outbox.is_none() && self.quiet_until.is_none_or(|t| Instant::now() >= t)
    }
}

/// How many log lines the tower keeps in memory. The database keeps them all.
pub(super) const LOG_KEEP: usize = 1000;

#[derive(Default)]
pub(super) struct Inner {
    pub(super) sessions: Vec<Session>,
    pub(super) log: Vec<Entry>,
    /// The last changes to what is read out, as the log has them: for `airband status`.
    pub(super) aloud_trail: std::collections::VecDeque<String>,
    pub(super) seq: u64,
    /// The change count: see [`Entry::rev`].
    pub(super) rev: u64,
    /// Every project an agent has been seen working in, by spoken name, with its folder. A new agent can be
    /// launched in any of these.
    pub(super) projects: BTreeMap<String, String>,
    /// Each team's colour: its place in a radio's list of team colours. Given when the team is first seen.
    pub(super) colours: BTreeMap<String, u8>,
    /// The agents the tower runs itself.
    pub(super) held: Vec<held::Kept>,
    /// Names agents go by on his other computers, as a radio last told it. None of this tower's numbered
    /// agents is called any of them.
    pub(super) elsewhere: Vec<Callsign>,
    /// Where each agent that was moved to another team was put, by its id. A session that goes away and comes
    /// back (the computer restarted, he resumed it) is put back there, not in its folder's team again.
    pub(super) placed: BTreeMap<String, (String, Mover)>,
    /// Each radio that has been heard from, by the id it gives.
    pub(super) calls: HashMap<String, Call>,
    /// When each open message was last put in front of him on a call, and how many times: for reminders.
    pub(super) offered: HashMap<String, (u32, Instant)>,
    /// Ids of messages that are new or changed since they were last written to the database.
    pub(super) unsaved: std::cell::RefCell<HashSet<String>>,
    /// Ids of messages whose short version is being made right now, so none is made twice.
    pub(super) judging: HashSet<String>,
    /// Requests for permission that are waiting for his answer, or have just had it, by id.
    pub(super) permits: HashMap<String, Permit>,
    /// Ids of the lines a notification has been decided for, so each is decided once.
    pub(super) pushed: HashSet<String>,
    /// How many requests for the whole chat are open right now: while there is one, it reads what agents say.
    pub(super) readers: u32,
}

/// One request for permission.
impl Permit {
    /// Is the harness still waiting on this? A decided one waits to be collected; an undecided one is over
    /// when the harness has stopped asking after it, or an hour has gone by.
    pub(super) fn live(&self) -> bool {
        self.asked.elapsed() < Duration::from_secs(3600) && (self.decided.is_some() || self.polled.elapsed() < Duration::from_secs(60))
    }
}

pub(super) struct Permit {
    pub(super) agent_id: String,
    /// The log line that puts it to him.
    pub(super) entry: String,
    /// His answer, once he has given it: allowed or not, and his reason if he refused with one.
    pub(super) decided: Option<(bool, String)>,
    pub(super) asked: Instant,
    /// When the harness last asked where this stands. It asks every few seconds while it waits; one that has
    /// stopped asking has stopped waiting.
    pub(super) polled: Instant,
    /// It is a question of the agent's, not a request for leave: whatever he says is the answer.
    pub(super) question: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Saved {
    pub(super) sessions: Vec<Session>,
    #[serde(default)]
    pub(super) projects: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) colours: BTreeMap<String, u8>,
    #[serde(default)]
    pub(super) held: Vec<held::Kept>,
    #[serde(default)]
    pub(super) placed: BTreeMap<String, (String, Mover)>,
}

impl Inner {
    /// Is any radio there?
    pub(super) fn connected(&self) -> bool {
        self.calls.values().any(Call::connected)
    }
    /// Is any radio following the chat? Then there is someone to put a question to, and someone to see what
    /// the agents are doing.
    pub(super) fn following(&self) -> bool {
        self.calls.values().any(|c| c.presence.following())
    }
    /// Does anyone on a call need the tower's voice? Not when every radio there makes its own.
    pub(super) fn wants_voice(&self) -> bool {
        self.calls.values().any(|c| c.connected() && !c.speaks)
    }
    pub(super) fn call(&mut self, radio: &str) -> &mut Call {
        let call = self.calls.entry(radio.to_string()).or_default();
        call.seen = Some(Instant::now());
        call
    }
    /// A radio that is on a call and free to take a transmission.
    pub(super) fn free_radio(&self) -> Option<String> {
        let mut free: Vec<&String> = self.calls.iter().filter(|(_, c)| c.can_send()).map(|(id, _)| id).collect();
        free.sort();
        free.first().map(|id| id.to_string())
    }
    pub(super) fn session(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.agent.id == id)
    }
    pub(super) fn find(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.agent.id == id)
    }
    /// The agent that said a logged message, if it is still on the radio.
    pub(super) fn sender(&self, e: &Entry) -> Option<&Session> {
        e.agent.as_deref().and_then(|id| self.find(id))
    }

    /// The one way a line's place in the reading queue changes. Every change is written to the log with what it
    /// was and what it became, so "why was that not read to me" has an answer.
    pub(super) fn set_aloud(&mut self, id: &str, to: Aloud) {
        let Some(e) = self.log.iter_mut().find(|e| e.id == id) else { return };
        if e.aloud == to {
            return;
        }
        let line = format!("aloud: {} ({id}) {} → {}", e.from, e.aloud.words(), to.words());
        e.aloud = to;
        self.unsaved.borrow_mut().insert(id.to_string());
        // not to be read by the radio that had it (it is heard, or is not to be read after all, or goes back in
        // the queue): that radio is not waited on for it any more
        if !matches!(e.aloud, Aloud::Handed { .. }) {
            for call in self.calls.values_mut() {
                if call.outbox.as_ref().is_some_and(|t| t.id == id) {
                    call.outbox = None;
                    call.ready = true;
                }
                if call.handed.as_ref().is_some_and(|t| t.id == id) {
                    call.handed = None;
                }
            }
        }
        log(&line);
        self.aloud_trail.push_back(format!("{} {line}", clock()));
        while self.aloud_trail.len() > 20 {
            self.aloud_trail.pop_front();
        }
    }
    /// The lines still to be heard, oldest first.
    pub(super) fn unheard(&self) -> impl Iterator<Item = &Entry> {
        self.log.iter().filter(|e| e.aloud.unheard())
    }
    /// What the judge reads of a line.
    pub(super) fn item(&self, e: &Entry) -> Item {
        Item { asked: e.agent.as_deref().and_then(|a| self.last_instruction(a)), id: e.id.clone(), notice: e.notice, full: e.full.clone(), kind: e.kind, say: e.short.clone() }
    }

    /// This agent is no longer waiting on him: he spoke to it, or it went back to work. What it was waiting
    /// for is not read out any more. A report he has not heard yet still is: he asked to hear what his
    /// agents report, and talking to one is not the same as having heard it.
    pub(super) fn no_longer_waiting(&mut self, agent_id: &str) {
        self.drop_asks(agent_id, Dropped::Answered);
    }
    fn drop_asks(&mut self, agent_id: &str, why: Dropped) {
        let asks: Vec<String> = self.unheard().filter(|e| e.agent.as_deref() == Some(agent_id) && matches!(e.kind, Some(Kind::NeedsYou | Kind::Stalled))).map(|e| e.id.clone()).collect();
        for id in asks {
            self.set_aloud(&id, Aloud::Dropped { why });
        }
    }
    /// This agent has said something newer: only the latest thing each agent said is read out. A notice from
    /// its harness takes the place of an earlier notice, and what it said of what it said before.
    pub(super) fn supersede(&mut self, agent_id: &str, notice: bool) {
        let older: Vec<String> = self.unheard().filter(|e| e.agent.as_deref() == Some(agent_id) && e.notice == notice).map(|e| e.id.clone()).collect();
        for id in older {
            self.set_aloud(&id, Aloud::Dropped { why: Dropped::Superseded });
        }
    }
    /// A radio that took something to read out is not going to say he heard it (it came back to the call anew,
    /// said it was ready for the next thing, or went quiet): what it had goes back in the queue.
    pub(super) fn give_back(&mut self, radio: &str) {
        let had: Vec<String> = self.log.iter().filter(|e| matches!(&e.aloud, Aloud::Handed { radio: r, .. } if r == radio)).map(|e| e.id.clone()).collect();
        for id in had {
            self.set_aloud(&id, Aloud::Waiting);
        }
        if let Some(call) = self.calls.get_mut(radio) {
            call.handed = None;
        }
    }
    /// He has just turned reading aloud on: what was said long before is in the chat for him to read, and is
    /// not read out now, hours late.
    pub(super) fn leave_old_news(&mut self) {
        let recent = now().saturating_sub(STILL_NEWS.as_millis() as u64);
        let old: Vec<String> = self.unheard().filter(|e| e.at < recent).map(|e| e.id.clone()).collect();
        for id in old {
            self.set_aloud(&id, Aloud::Dropped { why: Dropped::Stale });
        }
    }
    /// At start. Nothing is with a radio any more. What an agent that is gone was waiting on him for goes; what
    /// it reported in the last while is still his news.
    pub(super) fn settle_reading(&mut self) {
        let live: HashSet<String> = self.sessions.iter().map(|s| s.agent.id.clone()).collect();
        let recent = now().saturating_sub(STILL_NEWS.as_millis() as u64);
        let unheard: Vec<(String, bool, bool, bool)> = self.unheard().map(|e| (e.id.clone(), e.agent.as_ref().is_some_and(|a| live.contains(a)), e.kind == Some(Kind::Report), e.at >= recent)).collect();
        for (id, live, report, recent) in unheard {
            self.set_aloud(&id, match (live, report, recent) {
                (true, _, _) | (false, true, true) => Aloud::Waiting,
                (false, true, false) => Aloud::Dropped { why: Dropped::Stale },
                (false, false, _) => Aloud::Dropped { why: Dropped::AgentGone },
            });
        }
    }

    pub(super) fn remove(&mut self, agent_id: &str) {
        // it has gone: what it was waiting on him for goes with it, and what it reported is still to be heard
        self.drop_asks(agent_id, Dropped::AgentGone);
        self.sessions.retain(|s| s.agent.id != agent_id);
        for call in self.calls.values_mut() {
            if call.focus.as_deref() == Some(agent_id) {
                call.focus = None;
            }
        }
    }
    pub(super) fn reap(&mut self) -> bool {
        let dead: Vec<(String, String)> = self.sessions.iter().filter(|s| s.agent.deliver != Some(Deliver::Held) && s.agent.pid.is_some_and(|p| !alive(p))).map(|s| (s.agent.id.clone(), s.callsign.text())).collect();
        for (id, callsign) in &dead {
            log(format!("{callsign} is gone"));
            self.remove(id);
        }
        // a permission request nobody is waiting on any more: its line stops offering an answer
        let over: Vec<String> = self.permits.iter().filter(|(_, p)| !p.live()).map(|(id, _)| id.clone()).collect();
        for id in over {
            self.close_permit(&id, permit::Ended::Gone);
        }
        // a radio not heard from in a day has nothing worth keeping
        self.calls.retain(|_, c| c.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(86_400)));
        !dead.is_empty()
    }

    /// An agent reported in. A new one is given a callsign; a known one has what it reported about itself
    /// brought up to date, and takes the name he gave its session if that is new. Returns its callsign.
    pub(super) fn check_in(&mut self, agent: &Agent) -> Callsign {
        let named = agent.name.clone().or(agent.cwd.clone()).unwrap_or_else(|| agent.harness.clone());
        // the name he gave the session, when it is new to the tower or has changed
        let wanted = agent.title.as_deref().filter(|t| self.find(&agent.id).is_none_or(|s| s.agent.title.as_deref() != Some(*t))).and_then(|t| Callsign::titled(&named, t)).map(|mut want| {
            // one that was moved to another team is renamed there, not back in its folder's
            if let Some(s) = self.find(&agent.id).filter(|s| s.moved.is_some()) {
                want.project = s.callsign.project.clone();
            }
            want
        });
        if let Some(want) = &wanted {
            self.make_room(&agent.id, want, &named);
        }
        let free = |inner: &Inner, c: &Callsign| !inner.sessions.iter().any(|s| s.callsign == *c);
        if self.find(&agent.id).is_none() {
            let callsign = wanted.clone().filter(|c| free(self, c)).unwrap_or_else(|| {
                // the next free number, among the names in use here and on his other computers (as his radio
                // shows them: "shop 2" there takes "shop two" here)
                let mut taken = self.callsigns();
                let mut c = Callsign::next_for(&named, &taken);
                while self.elsewhere.iter().any(|e| airband_core::routing::shown(&e.text()) == airband_core::routing::shown(&c.text())) {
                    taken.push(c.clone());
                    c = Callsign::next_for(&named, &taken);
                }
                c
            });
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            // remember where this project lives, so a new agent can be launched there later
            if let Some(cwd) = agent.cwd.clone().filter(|_| agent.name.is_none()) {
                self.projects.insert(callsign.project.clone(), cwd);
            }
            self.sessions.push(Session { agent: agent.clone(), callsign, waiting: false, last_heard: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0, queue: Vec::new() });
            // one that was moved to another team before it went away goes back there
            if let Some((team, by)) = self.placed.get(&agent.id).cloned() {
                self.move_to(&agent.id, &team, by);
            }
        } else if let Some(want) = wanted.filter(|c| free(self, c)) {
            self.rename(&agent.id, want);
        }
        self.colour_teams();
        let s = self.session(&agent.id).unwrap();
        // an adapter that cannot see something on this event must not erase what an earlier one reported
        if agent.title.is_some() {
            s.agent.title = agent.title.clone();
        }
        if agent.deliver.is_some() {
            s.agent.deliver = agent.deliver.clone();
        }
        if agent.pid.is_some() {
            s.agent.pid = agent.pid;
        }
        if !agent.meta.is_empty() {
            s.agent.meta = agent.meta.clone();
        }
        s.callsign.clone()
    }

    /// A session is about to take the callsign `want`. If another session holds it only because of the order it
    /// arrived in, that one moves to the next free number; one that was given the name by him keeps it.
    fn make_room(&mut self, id: &str, want: &Callsign, named: &str) {
        let Some(holder) = self.sessions.iter().find(|s| s.callsign == *want && s.agent.id != id).cloned() else { return };
        if holder.agent.title.as_deref().and_then(|t| Callsign::titled(named, t)).as_ref() == Some(want) {
            return;
        }
        let mut taken = self.callsigns();
        taken.push(want.clone());
        self.rename(&holder.agent.id, Callsign::next_for(named, &taken));
    }

    /// Put a session in another team, one that exists or a new one. Returns what it was called and what it is
    /// called now, or nothing when it is already there or the team's name says nothing.
    pub(super) fn move_to(&mut self, id: &str, team: &str, by: Mover) -> Option<(Callsign, Callsign)> {
        let was = self.find(id)?.callsign.clone();
        let others: Vec<Callsign> = self.sessions.iter().filter(|s| s.agent.id != id).map(|s| s.callsign.clone()).collect();
        let to = was.moved_to(team, &others)?;
        if to.project == was.project {
            // already there: if it was he who said so, it now stays
            if by == Mover::Him {
                self.session(id)?.moved = Some(Mover::Him);
            }
            return None;
        }
        self.rename(id, to.clone());
        self.session(id)?.moved = Some(by);
        self.placed.insert(id.to_string(), (to.project.clone(), by));
        // only the latest few hundred are remembered
        while self.placed.len() > 400 {
            let Some(first) = self.placed.keys().next().cloned() else { break };
            self.placed.remove(&first);
        }
        self.colour_teams();
        Some((was, to))
    }

    /// Call a session something else. Its messages are found by its id, so nothing has to be carried over;
    /// they are marked changed so every radio shows the new name.
    fn rename(&mut self, id: &str, to: Callsign) {
        let Some(s) = self.session(id) else { return };
        if s.callsign == to {
            return;
        }
        log(format!("{} is now {}", s.callsign.text(), to.text()));
        s.callsign = to;
        let touched: Vec<String> = self.log.iter().filter(|e| e.agent.as_deref() == Some(id) || e.to_agents.iter().any(|a| a == id)).map(|e| e.id.clone()).collect();
        for entry in touched {
            self.touch(&entry);
        }
    }

    /// What he said to an agent at its own keyboard, as its harness kept it: added to the chat as his words to
    /// it, each once, at the time he said it. `upto` is how far the harness's record has now been read.
    /// `pictures` are the ones that came with each thing told, already kept, in the same order.
    pub(super) fn told(&mut self, agent_id: &str, told: Vec<crate::adapters::Told>, pictures: Vec<Vec<airband_core::radio::Picture>>, upto: u64) {
        let Some(to) = self.find(agent_id).cloned() else { return };
        // nothing from before the chat he has; with no chat yet, the last day
        let oldest = self.log.first().map(|e| e.at).unwrap_or_else(|| now().saturating_sub(24 * 3600 * 1000));
        // what the tower passed on, now taken in by the agent: his message is marked read, the oldest unread first
        for t in told.iter().filter(|t| !t.his) {
            let read = self.log.iter().position(|e| e.from == "you" && e.seen.is_none() && !e.queued && !e.cancelled && e.to_agents.iter().any(|a| a == agent_id) && e.at <= t.at + 5000 && t.text.contains(e.full.trim()));
            if let Some(i) = read {
                self.log[i].seen = Some(t.at);
                let id = self.log[i].id.clone();
                self.touch(&id);
            }
        }
        for (t, pictures) in told.into_iter().zip(pictures.into_iter().chain(std::iter::repeat_with(Vec::new))).filter(|(t, _)| t.his && t.at >= oldest) {
            // the harness's own id for it, so the same words are never added twice
            let id: String = t.id.chars().filter(|c| c.is_ascii_alphanumeric()).take(40).collect();
            if id.is_empty() || self.log.iter().any(|e| e.id == id) {
                continue;
            }
            // his words as he typed them. Something very long (a pasted log) is shown by its opening and kept whole
            let short: String = if t.text.chars().count() > 1500 { format!("{}…", t.text.chars().take(1500).collect::<String>().trim_end()) } else { t.text.clone() };
            self.record_at(Entry { short: Some(short), pictures, ..Entry::his(id, std::slice::from_ref(&to), t.text) }, t.at);
        }
        if let Some(s) = self.session(agent_id) {
            s.told_upto = upto;
        }
    }

    /// Add a line to the log.
    pub(super) fn record(&mut self, e: Entry) {
        self.record_at(e, now());
    }
    /// Add a line that was said at a time already past: it takes its place among the others by that time.
    pub(super) fn record_at(&mut self, mut e: Entry, at: u64) {
        self.seq += 1;
        self.rev += 1;
        e.seq = self.seq;
        e.rev = self.rev;
        e.at = at.min(now());
        // a key or a token in what was said is not kept in the chat (the agent it went to has it as he said it)
        e.scrub();
        self.unsaved.borrow_mut().insert(e.id.clone());
        let place = self.log.partition_point(|have| have.at <= e.at);
        self.log.insert(place, e);
        if self.log.len() > LOG_KEEP {
            let extra = self.log.len() - LOG_KEEP;
            self.log.drain(..extra);
        }
    }
    /// A logged line changed: every radio should see it again, and it should be written.
    /// A request ends: he answered it, the harness stopped waiting and asks at its keyboard, it asked
    /// something newer, or it is gone. This is the one way any of those happens. The line stops offering an
    /// answer (and, unanswered, says where the question went), it leaves the call queue, and the agent is
    /// waiting on him from now only if it is asking at its keyboard.
    pub(super) fn close_permit(&mut self, id: &str, ended: permit::Ended) {
        let (agent_id, entry) = match &ended {
            // his answer waits here for the harness to collect it
            permit::Ended::Answered(allow, reason) => {
                let Some(permit) = self.permits.get_mut(id) else { return };
                permit.decided = Some((*allow, reason.clone()));
                (permit.agent_id.clone(), permit.entry.clone())
            }
            _ => {
                let Some(permit) = self.permits.remove(id) else { return };
                (permit.agent_id, permit.entry)
            }
        };
        match ended {
            permit::Ended::Answered(..) => {
                if let Some(e) = self.log.iter_mut().find(|e| e.id == entry) {
                    e.permit = None;
                    // answered: a request for leave offers nothing more
                    if e.leave {
                        e.options.clear();
                    }
                }
                if self.log.iter().any(|e| e.id == entry && e.aloud.unheard()) {
                    self.set_aloud(&entry, Aloud::Dropped { why: Dropped::Answered });
                }
                self.touch(&entry);
            }
            _ => self.close_line(&entry),
        }
        if let Some(s) = self.session(&agent_id) {
            s.waiting = matches!(ended, permit::Ended::AtKeyboard | permit::Ended::Replaced);
        }
    }
    /// At start: no harness is waiting on anything asked before the tower stopped, so every request for leave
    /// still offering an answer is over.
    pub(super) fn close_requests_from_before(&mut self) {
        let open: Vec<String> = self.log.iter().filter(|e| e.leave && !e.options.is_empty()).map(|e| e.id.clone()).collect();
        for id in open {
            self.close_line(&id);
        }
    }
    /// The line of a request that is over.
    pub(super) fn close_line(&mut self, entry: &str) {
        if let Some(e) = self.log.iter_mut().find(|e| e.id == entry) {
            e.permit = None;
            e.options.clear();
            if e.leave {
                e.short = e.short.as_ref().map(|s| s.strip_suffix(permit::ASKS).map(|line| format!("{line}{}", permit::MOVED)).unwrap_or_else(|| s.clone()));
            }
        }
        if self.log.iter().any(|e| e.id == entry && e.aloud.unheard()) {
            self.set_aloud(entry, Aloud::Dropped { why: Dropped::PermissionEnded });
        }
        self.touch(entry);
    }
    pub(super) fn touch(&mut self, id: &str) {
        self.rev += 1;
        let rev = self.rev;
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.rev = rev;
            self.unsaved.borrow_mut().insert(id.to_string());
        }
    }
    /// The short version of a message is ready: for the call queue and for the chat alike.
    pub(super) fn fill(&mut self, id: &str, kind: Kind, say: &str, options: &[String], recommended: Option<u32>, read: Option<&str>, background: bool, own_session: bool) {
        self.judging.remove(id);
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.kind = Some(kind);
            e.short = Some(airband_core::redact::redact(say));
            e.options = options.to_vec();
            e.recommended = recommended;
            e.read = read.map(String::from);
            e.background = background;
            e.own_session = own_session;
            self.touch(id);
        }
        // not worth his ears: it stays in the chat, dimmed, and is not read out
        if kind == Kind::Fyi && self.log.iter().any(|e| e.id == id && e.aloud.unheard()) {
            self.set_aloud(id, Aloud::Dropped { why: Dropped::Routine });
        }
    }

    /// A logged line as a radio sees it. The sender's name is what it is called now, if it is still here.
    pub(super) fn message(&self, e: &Entry) -> Message {
        let text = e.short.clone().unwrap_or_else(|| opening(&e.full));
        let sender = self.sender(e);
        let to = if e.to_agents.is_empty() { e.to.clone() } else { e.to_agents.iter().zip(&e.to).map(|(id, was)| self.find(id).map(|s| s.callsign.text()).unwrap_or_else(|| was.clone())).collect() };
        Message {
            seq: e.seq,
            id: e.id.clone(),
            at: e.at,
            from: sender.map(|s| s.callsign.text()).unwrap_or_else(|| e.from.clone()),
            // the team it belongs to now: its sender's, or for something he said, the team of who it went to
            group: sender.map(|s| s.callsign.project.clone()).unwrap_or_else(|| {
                let teams: HashSet<&String> = e.to_agents.iter().filter_map(|id| self.find(id)).map(|s| &s.callsign.project).collect();
                if teams.len() == 1 && e.to_agents.iter().all(|id| self.find(id).is_some()) { teams.into_iter().next().unwrap().clone() } else { e.group.clone() }
            }),
            to,
            kind: e.kind,
            long: spoken(&e.full) != text,
            text,
            options: e.options.clone(),
            recommended: e.recommended,
            read: e.read.clone(),
            own_session: e.own_session,
            seen: e.seen,
            queued: e.queued,
            cancelled: e.cancelled,
            // a question put through the same door is an ordinary question to him: any answer will do
            // only while the harness is still waiting on it
            permission: e.leave && e.permit.as_ref().is_some_and(|id| self.permits.get(id).is_some_and(|p| p.decided.is_none())),
            pictures: e.pictures.clone(),
            links: if e.from == "you" { Vec::new() } else { airband_core::speech::links(&e.full) },
            agent: e.agent.clone(),
            rev: e.rev,
            open: false,
        }
    }

    /// The messages that need him and that nothing has answered yet. One at most per agent: its latest, while
    /// it has said nothing since, he has sent it nothing since, and it is still on the radio.
    pub(super) fn open_ids(&self) -> HashSet<String> {
        // A question stays open until he says something to the agent that asked it, or it asks a newer one.
        // What the agent goes on to report does not close it at once: he still has not answered. But one it
        // has spoken past three times, or that is six hours old, is let go: by then it has decided for itself
        // or asked again, and it should not keep calling for him.
        const SPOKEN_PAST: u32 = 3;
        const TOO_OLD_MS: u64 = 6 * 3600 * 1000;
        let mut latest: HashMap<&str, (&str, u64, u32)> = HashMap::new();
        for e in &self.log {
            if let Some(agent) = e.agent.as_deref() {
                if e.kind == Some(Kind::NeedsYou) {
                    if e.dismissed {
                        latest.remove(agent);
                    } else {
                        latest.insert(agent, (&e.id, e.at, 0));
                    }
                } else if e.kind != Some(Kind::Fyi) {
                    if let Some(q) = latest.get_mut(agent) {
                        q.2 += 1;
                    }
                }
            } else {
                for to in &e.to_agents {
                    latest.remove(to.as_str());
                }
            }
        }
        let now = now();
        // a request for permission is open for as long as the harness is waiting on it, whatever its age
        let permits: HashSet<&str> = self.permits.values().filter(|p| p.decided.is_none()).map(|p| p.entry.as_str()).collect();
        latest
            .into_iter()
            .filter(|(agent, (id, at, past))| self.find(agent).is_some() && (permits.contains(id) || (*past < SPOKEN_PAST && now.saturating_sub(*at) < TOO_OLD_MS)))
            .map(|(_, (id, _, _))| id.to_string())
            .collect()
    }

    /// The last thing he said to an agent: a message addressed to it, or the task it was launched with.
    pub(super) fn last_instruction(&self, agent_id: &str) -> Option<String> {
        let launched = self.find(agent_id).map(|s| format!("a new agent in {}", s.callsign.project));
        self.log.iter().rev().find(|e| e.from == "you" && (e.to_agents.iter().any(|a| a == agent_id) || (e.to_agents.is_empty() && e.to.first() == launched.as_ref()))).map(|e| e.full.clone())
    }
    /// The last thing an agent said that was worth his attention.
    pub(super) fn last_said(&self, agent_id: &str) -> Option<&Entry> {
        self.log.iter().rev().find(|e| e.agent.as_deref() == Some(agent_id) && e.kind != Some(Kind::Fyi))
    }
    /// What an agent is doing, as one word: `working`, `asking` (a question of its is open), `stalled` (it
    /// stopped without finishing or asking), or `idle` (it finished and said so).
    pub(super) fn state_of(&self, s: &Session, open: &HashSet<String>) -> &'static str {
        if !s.waiting {
            return "working";
        }
        // It stopped only to wait for something of its own that is still running: that is still work, for a
        // while. If nothing has come of it after twenty minutes it is not coming back by itself: it has
        // stalled, and a poke is offered.
        if let Some(e) = self.log.iter().rev().find(|e| e.agent.as_deref() == Some(s.agent.id.as_str())).filter(|e| e.background) {
            return if now().saturating_sub(e.at) < 20 * 60 * 1000 { "working" } else { "stalled" };
        }
        match self.last_said(&s.agent.id) {
            Some(e) if open.contains(&e.id) => "asking",
            Some(e) if e.kind == Some(Kind::Stalled) => "stalled",
            _ => "idle",
        }
    }
    /// Every team on the radio has a colour: a new one takes the first that no team has, and when they have all
    /// been taken, the one fewest teams share.
    pub(super) fn colour_teams(&mut self) {
        const COLOURS: u8 = 6;
        let teams: Vec<String> = self.sessions.iter().map(|s| s.callsign.project.clone()).collect();
        for team in teams {
            if !self.colours.contains_key(&team) {
                let next = (0..COLOURS).min_by_key(|c| self.colours.values().filter(|have| *have == c).count()).unwrap_or(0);
                self.colours.insert(team, next);
            }
        }
    }
    /// A number that changes when anything a radio shows about the agents does: who is there, what each is
    /// called, its state, what it is doing.
    pub(super) fn roster_rev(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.roster_entries()).unwrap_or_default().hash(&mut h);
        // never 0: that is what a radio sends before it has seen one
        h.finish() | 1
    }
    /// The agents that have stopped without finishing or asking: the ones a poke is for.
    pub(super) fn stalled(&self) -> Vec<Session> {
        let open = self.open_ids();
        self.sessions.iter().filter(|s| self.state_of(s, &open) == "stalled").cloned().collect()
    }
    pub(super) fn roster_entries(&self) -> Vec<RosterEntry> {
        let open = self.open_ids();
        self.sessions.iter().map(|s| RosterEntry { id: s.agent.id.clone(), callsign: s.callsign.text(), group: s.callsign.project.clone(), harness: s.agent.harness.clone(), waiting: s.waiting, state: self.state_of(s, &open).into(), subagents: s.subagents, reachable: s.agent.deliver.is_some(), doing: s.doing.clone().filter(|_| !s.waiting), held: s.agent.deliver == Some(Deliver::Held), model: s.model.clone(), effort: s.effort.clone() }).collect()
    }
    /// Where a new agent can be started: what the configuration lists, then wherever agents have been seen.
    pub(super) fn launchable(&self, cfg: &Config) -> BTreeMap<String, String> {
        let mut all = self.projects.clone();
        all.extend(cfg.projects.iter().map(|(k, v)| (k.to_lowercase(), v.clone())));
        // a project whose folder has gone is not offered
        all.retain(|_, folder| std::path::Path::new(folder).is_dir());
        all
    }
    pub(super) fn callsigns(&self) -> Vec<Callsign> {
        self.sessions.iter().map(|s| s.callsign.clone()).collect()
    }
    /// A radio said which names his other computers use. Any agent here that was given one of them by number
    /// takes the next free number. Returns whether anything was renamed.
    pub(super) fn taken_elsewhere(&mut self, shown: &[String]) -> bool {
        use airband_core::routing::shown as as_shown;
        // as callsigns: the team is everything before the last word
        self.elsewhere = shown.iter().filter_map(|c| c.rsplit_once(' ')).map(|(project, number)| Callsign { project: project.to_string(), number: number.to_string() }).collect();
        let clashing: Vec<String> = self.sessions.iter().filter(|s| s.callsign.numbered() && shown.iter().any(|c| *c == as_shown(&s.callsign.text()))).map(|s| s.agent.id.clone()).collect();
        for id in &clashing {
            let Some(project) = self.find(id).map(|s| s.callsign.project.clone()) else { continue };
            // a name nobody has, here or there: numbers are compared as the radio shows them
            let mut n = 1;
            let free = loop {
                let c = Callsign::next_for(&project, &[]);
                let candidate = Callsign { project: c.project.clone(), number: airband_core::routing::number_name(n) };
                let text = as_shown(&candidate.text());
                if !shown.contains(&text) && !self.sessions.iter().any(|s| as_shown(&s.callsign.text()) == text) {
                    break candidate;
                }
                n += 1;
            };
            self.rename(id, free);
        }
        !clashing.is_empty()
    }
    pub(super) fn roster(&self) -> String {
        if self.sessions.is_empty() {
            return "No agents are running.".into();
        }
        let names = |waiting: bool| self.sessions.iter().filter(|s| s.waiting == waiting).map(|s| s.callsign.text()).collect::<Vec<_>>().join(", ");
        let (waiting, working) = (names(true), names(false));
        [(!waiting.is_empty()).then(|| format!("Waiting on you: {waiting}.")), (!working.is_empty()).then(|| format!("Working: {working}."))].into_iter().flatten().collect::<Vec<_>>().join(" ")
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, cwd: &str, title: Option<&str>) -> Agent {
        Agent { id: id.into(), harness: "test".into(), cwd: Some(cwd.into()), name: None, title: title.map(String::from), pid: None, deliver: None, meta: Default::default() }
    }
    fn says(inner: &mut Inner, agent_id: &str, kind: Kind, text: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        inner.record(Entry::said(id.clone(), &s, Some(kind), text.into()));
        id
    }
    fn names(inner: &Inner) -> Vec<String> {
        inner.sessions.iter().map(|s| s.callsign.text()).collect()
    }

    /// A request for leave from `agent_id`, as `permit::ask` records it.
    fn asks_leave(inner: &mut Inner, agent_id: &str, permit: &str, what: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        let line = format!("Wants to {what}.{}", permit::ASKS);
        inner.record(Entry { short: Some(line.clone()), options: vec![permit::ALLOW.into(), permit::DENY.into()], permit: Some(permit.into()), leave: true, ..Entry::said(id.clone(), &s, Some(Kind::NeedsYou), line) });
        inner.permits.insert(permit.into(), Permit { agent_id: agent_id.into(), entry: id.clone(), decided: None, asked: Instant::now(), polled: Instant::now(), question: false });
        id
    }
    fn card(inner: &Inner, id: &str) -> airband_core::radio::Message {
        inner.message(inner.log.iter().find(|e| e.id == id).unwrap())
    }

    #[test]
    fn a_request_for_leave_offers_an_answer_only_while_the_harness_waits() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let line = asks_leave(&mut inner, "a", "p1", "run `make`");
        assert!(card(&inner, &line).permission && card(&inner, &line).options.len() == 2);
        // the harness stopped asking after it: the next sweep closes it, and the line says where it went
        inner.permits.get_mut("p1").unwrap().polled = Instant::now().checked_sub(Duration::from_secs(61)).unwrap();
        inner.reap();
        let over = card(&inner, &line);
        assert!(!over.permission && over.options.is_empty() && inner.permits.is_empty());
        assert_eq!(over.text, format!("Wants to run `make`.{}", permit::MOVED));
        // it is still known as a request for leave, so a late answer is not taken for words to the agent
        assert!(inner.log.iter().any(|e| e.id == line && e.leave));
    }

    #[test]
    fn every_way_a_request_ends_closes_its_line_and_says_whether_the_agent_waits() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let waits = |inner: &mut Inner| inner.session("a").unwrap().waiting;
        let open = |inner: &mut Inner, permit: &str| {
            let line = asks_leave(inner, "a", permit, "run `make`");
            inner.set_aloud(&line, Aloud::Waiting);
            inner.session("a").unwrap().waiting = true;
            line
        };
        // he answered: his answer waits to be collected, the line offers nothing more, and the agent goes on
        let line = open(&mut inner, "p1");
        inner.close_permit("p1", permit::Ended::Answered(true, String::new()));
        assert_eq!(inner.permits["p1"].decided, Some((true, String::new())));
        assert!(card(&inner, &line).options.is_empty() && !card(&inner, &line).permission && inner.unheard().next().is_none() && !waits(&mut inner));
        assert_eq!(card(&inner, &line).text, format!("Wants to run `make`.{}", permit::ASKS));
        // the harness gave up waiting and asks at its keyboard: the line says so, and the agent still waits
        let line = open(&mut inner, "p2");
        inner.close_permit("p2", permit::Ended::AtKeyboard);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p2") && inner.unheard().next().is_none() && waits(&mut inner));
        assert_eq!(card(&inner, &line).text, format!("Wants to run `make`.{}", permit::MOVED));
        // it asked something newer: the older line is closed
        let line = open(&mut inner, "p3");
        inner.close_permit("p3", permit::Ended::Replaced);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p3") && inner.unheard().next().is_none());
        // nobody asks after it any more: it was answered at the keyboard, or the agent has gone
        let line = open(&mut inner, "p4");
        inner.close_permit("p4", permit::Ended::Gone);
        assert!(card(&inner, &line).options.is_empty() && !inner.permits.contains_key("p4") && inner.unheard().next().is_none() && !waits(&mut inner));
        // a request that is already over is not ended twice
        inner.close_permit("p4", permit::Ended::Answered(true, String::new()));
        assert!(!inner.permits.contains_key("p4"));
    }

    #[test]
    fn a_request_from_before_the_tower_started_is_over() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let line = asks_leave(&mut inner, "a", "p1", "run `make`");
        // what a restart leaves: the line as saved, and no record of anyone waiting
        inner.permits.clear();
        inner.log.iter_mut().for_each(|e| e.permit = None);
        inner.close_requests_from_before();
        let c = card(&inner, &line);
        assert!(!c.permission && c.options.is_empty(), "{c:?}");
        assert!(inner.log.iter().any(|e| e.id == line && e.leave));
    }

    #[test]
    fn numbers_in_order_and_a_name_he_gave_takes_its_number() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/shop", None));
        assert_eq!(names(&inner), ["shop one", "shop two"]);
        // he named a third session "shop 2": it takes two, and the one that had it by arrival moves on
        inner.check_in(&agent("c", "/x/shop", Some("shop 2")));
        assert_eq!(names(&inner), ["shop one", "shop three", "shop two"]);
        // the same name again changes nothing; a new name renames
        inner.check_in(&agent("c", "/x/shop", Some("shop 2")));
        inner.check_in(&agent("c", "/x/shop", Some("questions")));
        assert_eq!(names(&inner), ["shop one", "shop three", "shop questions"]);
        // a name another session was given by him is not taken from it
        inner.check_in(&agent("d", "/x/shop", Some("shop questions")));
        assert_eq!(names(&inner)[3], "shop two");
    }

    #[test]
    fn a_question_stays_open_through_a_rename_and_follows_the_agent() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/backoffice", None));
        let q = says(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        let before = inner.rev;
        inner.check_in(&agent("a", "/x/backoffice", Some("backoffice login fix")));
        assert!(inner.open_ids().contains(&q));
        let m = inner.message(&inner.log[0]);
        // the room is the project, however many words the name has
        assert_eq!((m.from.as_str(), m.group.as_str()), ("backoffice login fix", "backoffice"));
        // and radios are told the line changed
        assert!(inner.log[0].rev > before);
    }

    #[test]
    fn a_dead_agents_question_does_not_pass_to_whoever_takes_its_callsign() {
        let mut inner = Inner::default();
        inner.check_in(&agent("old", "/x/shop", None));
        let q = says(&mut inner, "old", Kind::NeedsYou, "Rewrite them?");
        inner.remove("old");
        inner.check_in(&agent("new", "/x/shop", None));
        assert_eq!(names(&inner), ["shop one"]);
        assert!(inner.open_ids().is_empty());
        // the old line still says who asked, and is nobody's now
        assert!(inner.sender(&inner.log.iter().find(|e| e.id == q).unwrap().clone()).is_none());
    }

    #[test]
    fn open_means_asked_and_not_yet_answered() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        let qa = says(&mut inner, "a", Kind::NeedsYou, "Rewrite them?");
        let qb = says(&mut inner, "b", Kind::NeedsYou, "Ship it?");
        says(&mut inner, "a", Kind::Fyi, "Still here.");
        assert_eq!(inner.open_ids(), HashSet::from([qa.clone(), qb.clone()]));
        // he answers one
        let to = vec![inner.find("a").unwrap().clone()];
        inner.record(Entry::his("r1".into(), &to, "rewrite".into()));
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        assert_eq!(inner.last_instruction("a").as_deref(), Some("rewrite"));
        // the other goes on to report something: its question is still his to answer
        says(&mut inner, "b", Kind::Report, "Also fixed the footer.");
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        // but once it has spoken past it three times it is let go: it has decided for itself by then
        says(&mut inner, "b", Kind::Report, "And the header.");
        assert_eq!(inner.open_ids(), HashSet::from([qb.clone()]));
        says(&mut inner, "b", Kind::Report, "And the sidebar.");
        assert!(inner.open_ids().is_empty());
        // a newer question of its own is open again
        let qb2 = says(&mut inner, "b", Kind::NeedsYou, "Ship it now, or after the weekend?");
        assert_eq!(inner.open_ids(), HashSet::from([qb2]));
    }

    #[test]
    fn what_an_agent_is_doing() {
        let mut inner = Inner::default();
        for id in ["w", "q", "s", "d"] {
            inner.check_in(&agent(id, &format!("/x/{id}"), None));
        }
        let q = says(&mut inner, "q", Kind::NeedsYou, "Ship it?");
        says(&mut inner, "s", Kind::Stalled, "Two done. I'll do the third next.");
        says(&mut inner, "d", Kind::Report, "Shipped.");
        for id in ["q", "s", "d"] {
            inner.session(id).unwrap().waiting = true;
        }
        let open = inner.open_ids();
        assert_eq!(open, HashSet::from([q]));
        let state = |id: &str| inner.state_of(inner.find(id).unwrap(), &open);
        assert_eq!([state("w"), state("q"), state("s"), state("d")], ["working", "asking", "stalled", "idle"]);
        assert_eq!(inner.stalled().iter().map(|s| s.agent.id.as_str()).collect::<Vec<_>>(), ["s"]);
    }

    #[test]
    fn each_radio_has_its_own_call() {
        let mut inner = Inner::default();
        for radio in ["phone", "desk"] {
            let c = inner.call(radio);
            c.presence.polls();
            c.ready = true;
        }
        inner.call("desk").held = true;
        assert_eq!(inner.free_radio().as_deref(), Some("phone"));
        inner.check_in(&agent("a", "/x/shop", None));
        let t = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        inner.call("phone").outbox = Some(Transmission { id: t.clone(), callsign: "shop one".into(), kind: Kind::NeedsYou, say: String::new(), waiting: 0, speech: None });
        inner.call("phone").ready = false;
        // the agent goes back to work: what the phone was about to say is withdrawn, and only there
        inner.no_longer_waiting("a");
        assert!(inner.call("phone").outbox.is_none() && inner.call("phone").ready);
    }

    /// Something an agent said that is in the reading queue, as `intake::enqueue` leaves it once judged.
    fn queued(inner: &mut Inner, agent_id: &str, kind: Kind, text: &str) -> String {
        let s = inner.find(agent_id).unwrap().clone();
        let id = format!("m{}", inner.seq + 1);
        inner.record(Entry { aloud: Aloud::Waiting, short: Some(text.into()), ..Entry::said(id.clone(), &s, Some(kind), text.into()) });
        id
    }
    fn aloud(inner: &Inner, id: &str) -> Aloud {
        inner.log.iter().find(|e| e.id == id).unwrap().aloud.clone()
    }
    fn dropped(why: Dropped) -> Aloud {
        Aloud::Dropped { why }
    }

    #[test]
    fn a_report_he_has_not_heard_is_kept_when_he_speaks_to_its_agent_and_its_question_is_not() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        let question = queued(&mut inner, "a", Kind::NeedsYou, "Ship the next one?");
        inner.no_longer_waiting("a");
        assert_eq!((aloud(&inner, &report), aloud(&inner, &question)), (Aloud::Waiting, dropped(Dropped::Answered)));
        // something newer from the same agent still takes the place of what it said before
        inner.supersede("a", false);
        assert_eq!(aloud(&inner, &report), dropped(Dropped::Superseded));
        assert_eq!(inner.unheard().count(), 0);
    }

    #[test]
    fn every_way_out_of_the_reading_queue_is_written_down_with_its_reason() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        // routine: the judge found nothing for his ears
        let routine = queued(&mut inner, "a", Kind::Report, "Still going.");
        inner.fill(&routine, Kind::Fyi, "Still going.", &[], None, None, false, false);
        assert_eq!(aloud(&inner, &routine), dropped(Dropped::Routine));
        // dismissed, and a request for leave that is over
        let leave = asks_leave(&mut inner, "a", "p1", "run `make`");
        inner.set_aloud(&leave, Aloud::Waiting);
        inner.close_permit("p1", permit::Ended::Gone);
        assert_eq!(aloud(&inner, &leave), dropped(Dropped::PermissionEnded));
        // its agent has gone: the question goes, the report is still his news
        let report = queued(&mut inner, "b", Kind::Report, "Refunds sent.");
        let question = queued(&mut inner, "b", Kind::NeedsYou, "Send the rest?");
        inner.remove("b");
        assert_eq!((aloud(&inner, &report), aloud(&inner, &question)), (Aloud::Waiting, dropped(Dropped::AgentGone)));
        // said long before he turned reading aloud on
        inner.log.iter_mut().find(|e| e.id == report).unwrap().at = 1;
        inner.leave_old_news();
        assert_eq!(aloud(&inner, &report), dropped(Dropped::Stale));
        // each of those is in the trail `airband status` shows, with what it was and what it became
        let trail: Vec<&String> = inner.aloud_trail.iter().collect();
        for reason in ["routine", "permission ended", "agent gone", "stale"] {
            assert!(trail.iter().any(|l| l.contains(&format!("waiting → dropped ({reason})"))), "{reason}: {trail:?}");
        }
        assert!(inner.unheard().next().is_none());
    }

    #[test]
    fn what_a_radio_took_and_did_not_hear_goes_back_in_the_queue() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        let report = queued(&mut inner, "a", Kind::Report, "Shipped.");
        inner.set_aloud(&report, Aloud::Handed { radio: "phone".into(), at: 1 });
        inner.call("phone").handed = Some(Transmission { id: report.clone(), callsign: "shop one".into(), kind: Kind::Report, say: String::new(), waiting: 0, speech: None });
        assert_eq!(inner.unheard().count(), 1);
        inner.give_back("phone");
        assert_eq!(aloud(&inner, &report), Aloud::Waiting);
        assert!(inner.call("phone").handed.is_none());
        // heard: it is out of the queue, and stays on record as heard
        inner.set_aloud(&report, Aloud::Heard { radio: "phone".into(), at: 2 });
        inner.give_back("phone");
        assert!(matches!(aloud(&inner, &report), Aloud::Heard { .. }) && inner.unheard().next().is_none());
    }

    #[test]
    fn at_start_a_gone_agents_question_goes_and_its_recent_report_stays() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        inner.check_in(&agent("b", "/x/backoffice", None));
        let live = queued(&mut inner, "a", Kind::NeedsYou, "Ship it?");
        let report = queued(&mut inner, "b", Kind::Report, "Refunds sent.");
        let old = queued(&mut inner, "b", Kind::Report, "Started.");
        let question = queued(&mut inner, "b", Kind::NeedsYou, "Send the rest?");
        inner.log.iter_mut().find(|e| e.id == old).unwrap().at = 1;
        inner.set_aloud(&live, Aloud::Handed { radio: "phone".into(), at: 1 });
        inner.sessions.retain(|s| s.agent.id != "b");
        inner.settle_reading();
        assert_eq!([aloud(&inner, &live), aloud(&inner, &report)], [Aloud::Waiting, Aloud::Waiting]);
        assert_eq!((aloud(&inner, &old), aloud(&inner, &question)), (dropped(Dropped::Stale), dropped(Dropped::AgentGone)));
    }

    #[test]
    fn a_radio_that_only_reads_and_types_keeps_who_it_was_talking_to() {
        let mut inner = Inner::default();
        inner.check_in(&agent("a", "/x/shop", None));
        // never on a call: it has not polled for anything to play
        inner.call("phone").focus = Some("a".into());
        inner.reap();
        assert_eq!(inner.calls.get("phone").and_then(|c| c.focus.as_deref()), Some("a"));
    }
}
