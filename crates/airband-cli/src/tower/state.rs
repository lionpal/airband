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
    /// The last thing he heard from it, for "say more" and "repeat".
    pub(super) last: Option<Item>,
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(super) enum Mover {
    Him,
    Tower,
}

/// Something an agent said that he has not heard yet.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Item {
    pub(super) id: String,
    pub(super) agent_id: String,
    /// It needs him at the keyboard; a spoken reply cannot help.
    pub(super) notice: bool,
    pub(super) full: String,
    pub(super) kind: Option<Kind>,
    pub(super) say: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) created: u64,
    /// The last thing he told this agent, if anything: what the judge needs to tell "done what you asked" from
    /// chatter along the way.
    #[serde(default)]
    pub(super) asked: Option<String>,
}

/// One line of the chat log. An agent's message shares its id with the pending [`Item`] it came from.
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
    /// When this line last changed, as a count that only goes up across the whole log. A radio asks for
    /// everything changed since the count it last saw.
    #[serde(default)]
    pub(super) rev: u64,
}

impl Entry {
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
    pub(super) last_poll: Option<Instant>,
    /// When this radio last did anything at all: a radio that only reads the feed and types never polls.
    pub(super) seen: Option<Instant>,
    /// Words he said that the tower could not place, held while it asks him who they are for.
    pub(super) asked: Option<(String, Instant)>,
    /// An idea for a new agent whose project the tower could not work out, held while it asks him which.
    pub(super) unplaced_idea: Option<(Idea, Instant)>,
}

impl Call {
    pub(super) fn connected(&self) -> bool {
        self.last_poll.is_some_and(|t| t.elapsed() < RADIO_GONE)
    }
    pub(super) fn can_send(&self) -> bool {
        self.connected() && self.ready && !self.held && self.outbox.is_none()
    }
}

/// How many log lines the tower keeps in memory. The database keeps them all.
pub(super) const LOG_KEEP: usize = 1000;

#[derive(Default)]
pub(super) struct Inner {
    pub(super) sessions: Vec<Session>,
    pub(super) pending: Vec<Item>,
    pub(super) log: Vec<Entry>,
    pub(super) seq: u64,
    /// The change count: see [`Entry::rev`].
    pub(super) rev: u64,
    /// Every project an agent has been seen working in, by spoken name, with its folder. A new agent can be
    /// launched in any of these.
    pub(super) projects: BTreeMap<String, String>,
    /// Each team's colour: its place in a radio's list of team colours. Given when the team is first seen.
    pub(super) colours: BTreeMap<String, u8>,
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
    /// When a radio last asked for the feed: whether anyone is there to be asked something.
    pub(super) last_reader: Option<Instant>,
}

/// One request for permission.
pub(super) struct Permit {
    pub(super) agent_id: String,
    /// The log line that puts it to him.
    pub(super) entry: String,
    /// His answer, once he has given it: allowed or not, and his reason if he refused with one.
    pub(super) decided: Option<(bool, String)>,
    pub(super) asked: Instant,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Saved {
    pub(super) sessions: Vec<Session>,
    pub(super) pending: Vec<Item>,
    #[serde(default)]
    pub(super) projects: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) colours: BTreeMap<String, u8>,
}

impl Inner {
    /// Is any radio there?
    pub(super) fn connected(&self) -> bool {
        self.calls.values().any(Call::connected)
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

    /// What this agent last said is stale: he answered it, or it went back to work.
    pub(super) fn drop_pending(&mut self, agent_id: &str, only_notice: Option<bool>) {
        let gone: HashSet<String> = self.pending.iter().filter(|p| p.agent_id == agent_id && only_notice.is_none_or(|n| p.notice == n)).map(|p| p.id.clone()).collect();
        self.pending.retain(|p| !gone.contains(&p.id));
        for call in self.calls.values_mut() {
            if call.outbox.as_ref().is_some_and(|t| gone.contains(&t.id)) {
                call.outbox = None;
                call.ready = true;
            }
        }
    }
    pub(super) fn remove(&mut self, agent_id: &str) {
        self.drop_pending(agent_id, None);
        self.sessions.retain(|s| s.agent.id != agent_id);
        for call in self.calls.values_mut() {
            if call.focus.as_deref() == Some(agent_id) {
                call.focus = None;
            }
        }
    }
    pub(super) fn reap(&mut self) -> bool {
        let dead: Vec<(String, String)> = self.sessions.iter().filter(|s| s.agent.pid.is_some_and(|p| !alive(p))).map(|s| (s.agent.id.clone(), s.callsign.text())).collect();
        for (id, callsign) in &dead {
            log(format!("{callsign} is gone"));
            self.remove(id);
        }
        // a permission request nobody came back for
        self.permits.retain(|_, p| p.asked.elapsed() < Duration::from_secs(3600));
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
            let callsign = wanted.clone().filter(|c| free(self, c)).unwrap_or_else(|| Callsign::next_for(&named, &self.callsigns()));
            log(format!("{} checked in ({}, {})", callsign.text(), agent.harness, agent.cwd.as_deref().unwrap_or("no folder")));
            // remember where this project lives, so a new agent can be launched there later
            if let Some(cwd) = agent.cwd.clone().filter(|_| agent.name.is_none()) {
                self.projects.insert(callsign.project.clone(), cwd);
            }
            self.sessions.push(Session { agent: agent.clone(), callsign, waiting: false, last: None, pictures: Vec::new(), subagents: 0, doing: None, model: None, effort: None, moved: None, reports: 0, told_upto: 0 });
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
    pub(super) fn told(&mut self, agent_id: &str, told: Vec<crate::adapters::Told>, upto: u64) {
        let Some(to) = self.find(agent_id).cloned() else { return };
        // nothing from before the chat he has; with no chat yet, the last day
        let oldest = self.log.first().map(|e| e.at).unwrap_or_else(|| now().saturating_sub(24 * 3600 * 1000));
        for t in told.into_iter().filter(|t| t.at >= oldest) {
            // the harness's own id for it, so the same words are never added twice
            let id: String = t.id.chars().filter(|c| c.is_ascii_alphanumeric()).take(40).collect();
            if id.is_empty() || self.log.iter().any(|e| e.id == id) {
                continue;
            }
            // his words as he typed them. Something very long (a pasted log) is shown by its opening and kept whole
            let short: String = if t.text.chars().count() > 600 { format!("{}…", t.text.chars().take(600).collect::<String>().trim_end()) } else { t.text.clone() };
            self.record_at(Entry { short: Some(short), ..Entry::his(id, std::slice::from_ref(&to), t.text) }, t.at);
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
        self.unsaved.borrow_mut().insert(e.id.clone());
        let place = self.log.partition_point(|have| have.at <= e.at);
        self.log.insert(place, e);
        if self.log.len() > LOG_KEEP {
            let extra = self.log.len() - LOG_KEEP;
            self.log.drain(..extra);
        }
    }
    /// A logged line changed: every radio should see it again, and it should be written.
    pub(super) fn touch(&mut self, id: &str) {
        self.rev += 1;
        let rev = self.rev;
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.rev = rev;
            self.unsaved.borrow_mut().insert(id.to_string());
        }
    }
    /// The short version of a message is ready: for the call queue and for the chat alike.
    pub(super) fn fill(&mut self, id: &str, kind: Kind, say: &str, options: &[String], recommended: Option<u32>, read: Option<&str>) {
        self.judging.remove(id);
        if let Some(p) = self.pending.iter_mut().find(|p| p.id == id) {
            p.kind = Some(kind);
            p.say = Some(say.to_string());
        }
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.kind = Some(kind);
            e.short = Some(say.to_string());
            e.options = options.to_vec();
            e.recommended = recommended;
            e.read = read.map(String::from);
            self.touch(id);
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
            permission: e.permit.is_some(),
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
        let mut latest: HashMap<&str, &str> = HashMap::new();
        for e in &self.log {
            if let Some(agent) = e.agent.as_deref() {
                if e.kind == Some(Kind::NeedsYou) {
                    latest.insert(agent, &e.id);
                } else if e.kind != Some(Kind::Fyi) {
                    // it reported something, or said something not yet judged: it has moved on from its question
                    latest.remove(agent);
                }
            } else {
                for to in &e.to_agents {
                    latest.remove(to.as_str());
                }
            }
        }
        latest.into_iter().filter(|(agent, _)| self.find(agent).is_some()).map(|(_, id)| id.to_string()).collect()
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
        self.sessions.iter().map(|s| RosterEntry { id: s.agent.id.clone(), callsign: s.callsign.text(), group: s.callsign.project.clone(), harness: s.agent.harness.clone(), waiting: s.waiting, state: self.state_of(s, &open).into(), subagents: s.subagents, reachable: s.agent.deliver.is_some(), doing: s.doing.clone().filter(|_| !s.waiting), model: s.model.clone(), effort: s.effort.clone() }).collect()
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
    pub(super) fn roster(&self) -> String {
        if self.sessions.is_empty() {
            return "No agents are running.".into();
        }
        let names = |waiting: bool| self.sessions.iter().filter(|s| s.waiting == waiting).map(|s| s.callsign.text()).collect::<Vec<_>>().join(", ");
        let (waiting, working) = (names(true), names(false));
        [(!waiting.is_empty()).then(|| format!("Waiting on you: {waiting}.")), (!working.is_empty()).then(|| format!("Working: {working}."))].into_iter().flatten().collect::<Vec<_>>().join(" ")
    }

    /// Lines written before ids were kept name their agent only by callsign. Where that callsign is still held
    /// by the same session, give them its id; the rest stay as history.
    pub(super) fn adopt_old_lines(&mut self) {
        let by_name: HashMap<String, String> = self.sessions.iter().map(|s| (s.callsign.text(), s.agent.id.clone())).collect();
        for e in self.log.iter_mut() {
            if e.from != "you" && e.agent.is_none() {
                e.agent = by_name.get(&e.from).cloned();
            } else if e.from == "you" && e.to_agents.is_empty() && !e.to.is_empty() && e.to.iter().all(|t| by_name.contains_key(t)) {
                e.to_agents = e.to.iter().map(|t| by_name[t].clone()).collect();
            }
        }
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
    fn open_means_asked_and_neither_answered_nor_moved_on() {
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
        // the other reports something: it has moved on from its question
        says(&mut inner, "b", Kind::Report, "Shipped anyway.");
        assert!(inner.open_ids().is_empty());
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
            c.last_poll = Some(Instant::now());
            c.ready = true;
        }
        inner.call("desk").held = true;
        assert_eq!(inner.free_radio().as_deref(), Some("phone"));
        inner.check_in(&agent("a", "/x/shop", None));
        inner.pending.push(Item { id: "t".into(), agent_id: "a".into(), notice: false, full: String::new(), kind: None, say: None, detail: None, created: 0, asked: None });
        inner.call("phone").outbox = Some(Transmission { id: "t".into(), callsign: "shop one".into(), kind: Kind::Report, say: String::new(), waiting: 0, speech: None });
        inner.call("phone").ready = false;
        // the agent goes back to work: what the phone was about to say is withdrawn, and only there
        inner.drop_pending("a", None);
        assert!(inner.call("phone").outbox.is_none() && inner.call("phone").ready);
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
