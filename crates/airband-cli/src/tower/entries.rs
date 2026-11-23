//! The chat log's lines: what a line holds, how one is written down and changed, and how a radio is shown it.
use super::*;

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
/// What a message of his is shown as coming from. Only a label: whether a message is his is `Entry::his`.
pub(super) const HIM: &str = "you";

/// One line of the chat log. For an agent's message, `aloud` says where it stands with being read out.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) seq: u64,
    pub(super) id: String,
    pub(super) at: u64,
    /// What the sender was called when it said this. For showing a message whose agent has gone.
    pub(super) from: String,
    /// He said it. Nothing is told from the sender's name: an agent could be called anything.
    #[serde(default)]
    pub(super) his: bool,
    /// The team it belongs to, by the team's id. None for something he said to agents of several teams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) team: Option<TeamId>,
    /// For his messages: what the agents it went to were called.
    pub(super) to: Vec<String>,
    pub(super) kind: Option<Kind>,
    /// The short version, once it has been made.
    pub(super) short: Option<String>,
    /// The short version is cut down from the message: there is more of it to read. Set by whoever made the
    /// short version, from what it knows (the model's own verdict, or a cut it made), never by comparing words.
    #[serde(default)]
    pub(super) more: bool,
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
    /// The agent that said this had stopped only to wait for work of its own still running. While this is the
    /// last thing it said, it counts as at work.
    #[serde(default)]
    pub(super) background: bool,
    /// For his words: when the agent took them in, once its harness shows that.
    #[serde(default)]
    pub(super) seen: Option<u64>,
    /// Where it stands when it is held back: a question he let go, or his words waiting for the agent to stop,
    /// or taken back before they went. Only one at a time.
    #[serde(default)]
    pub(super) held: Option<Held>,
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
    pub(super) leave: Option<super::permit::Request>,
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
    /// What airband says about the agent around the tool's words (`full`), when it says something.
    #[serde(default)]
    pub(super) note: Option<airband_core::radio::Note>,
}
/// Where a line stands, as the `waiting` column keeps it.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Held {
    /// He let this question go.
    Dismissed,
    /// His words, waiting for the agent to stop.
    Queued,
    /// His words, taken back before they went.
    Cancelled,
}
impl Entry {
    /// Something this agent said that was worth his attention (not a passing note).
    pub(super) fn worth_from(&self, agent_id: &str) -> bool {
        self.agent.as_deref() == Some(agent_id) && self.kind != Some(Kind::Fyi)
    }
    /// The line in a few words: its summary, else the opening of what it says.
    pub(super) fn line(&self) -> String {
        self.short.clone().unwrap_or_else(|| opening(&self.said_whole()))
    }
    /// Whether there is more to the message than its line. A line with no short version yet is the message's
    /// opening: the words read out of it taken up to a place, so there is more exactly when that place comes
    /// before their end. Where it was cut is counted, never its words compared.
    pub(super) fn more_to_read(&self) -> bool {
        match &self.short {
            Some(_) => self.more,
            None => {
                let whole = self.said_whole();
                opening(&whole).chars().count() < spoken(&whole).chars().count()
            }
        }
    }
    /// The words as they are read and shown: the tool's, after airband's own note when there is one.
    pub(super) fn said_whole(&self) -> String {
        self.note.map_or_else(|| self.full.clone(), |n| n.said(&self.full))
    }
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
        Entry { id, from: s.callsign.text(), team: Some(s.team.clone()), kind, full, agent: Some(s.agent.id.clone()), ..Entry::default() }
    }
    /// Something he said, to these agents.
    pub(super) fn his(id: String, to: &[Session], text: String) -> Entry {
        let teams: HashSet<&TeamId> = to.iter().map(|s| &s.team).collect();
        let team = (teams.len() == 1).then(|| to[0].team.clone());
        Entry { id, from: HIM.into(), his: true, team, to: to.iter().map(|s| s.callsign.text()).collect(), to_agents: to.iter().map(|s| s.agent.id.clone()).collect(), short: Some(text.clone()), full: text, ..Entry::default() }
    }
}
/// The chat's id for something he typed at an agent's own keyboard: made from the agent and the harness's id
/// for it, whole, so two the harness tells apart are never one line here. Nothing without the harness's id.
fn typed_line_id(agent: &str, harness_id: &str) -> Option<String> {
    use sha2::Digest;
    if harness_id.is_empty() {
        return None;
    }
    let sum = sha2::Sha256::new().chain_update(agent.as_bytes()).chain_update([0]).chain_update(harness_id.as_bytes()).finalize();
    Some(format!("typed-{}", sum.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>()))
}

/// How many log lines the tower keeps in memory. The database keeps them all.
pub(super) const LOG_KEEP: usize = 1000;
impl Inner {
    /// What the judge reads of a line.
    pub(super) fn item(&self, e: &Entry) -> Item {
        Item { asked: e.agent.as_deref().and_then(|a| self.last_instruction(a)), id: e.id.clone(), notice: e.notice, full: e.said_whole(), kind: e.kind, say: e.short.clone() }
    }
    /// The agent has taken in what was delivered under `receipt`: the lines that delivery carried are marked read.
    pub(super) fn took(&mut self, receipt: &str, at: u64) {
        for line in self.receipts.remove(receipt).unwrap_or_default() {
            if let Some(e) = self.log.iter_mut().find(|e| e.id == line && e.seen.is_none() && e.held != Some(Held::Cancelled)) {
                e.seen = Some(at);
                self.touch(&line);
            }
        }
    }
    /// What he said to an agent at its own keyboard, as its harness kept it: added to the chat as his words to
    /// it, each once, at the time he said it. `upto` is how far the harness's record has now been read.
    /// `pictures` are the ones that came with each thing told, already kept, in the same order.
    pub(super) fn told(&mut self, agent_id: &str, told: Vec<crate::adapters::Told>, pictures: Vec<Vec<airband_core::radio::Picture>>, upto: u64) {
        let Some(to) = self.find(agent_id).cloned() else { return };
        // nothing from before the chat he has; with no chat yet, the last day
        let oldest = self.log.first().map(|e| e.at).unwrap_or_else(|| now_ms().saturating_sub(24 * 3600 * 1000));
        // What the tower passed on, now taken in by the agent: the agent's record names the id the delivery went
        // under, and the lines that delivery carried are marked read. Nothing is told from the words.
        for t in told.iter().filter(|t| !t.his) {
            self.took(&t.id, t.at);
        }
        for (t, pictures) in told.into_iter().zip(pictures.into_iter().chain(std::iter::repeat_with(Vec::new))).filter(|(t, _)| t.his && t.at >= oldest) {
            // the harness's own id for it, so the same words are never added twice; one without an id is not
            // known to be new, and is left out
            let Some(id) = typed_line_id(agent_id, &t.id) else { continue };
            if self.log.iter().any(|e| e.id == id) {
                continue;
            }
            // his words as he typed them. Something very long (a pasted log) is shown by its opening and kept whole
            let (short, more) = (airband_core::prose::clip(&t.text, 1500), t.text.chars().count() > 1500);
            self.record_at(Entry { short: Some(short), more, pictures, ..Entry::his(id, std::slice::from_ref(&to), t.text) }, t.at);
        }
        if let Some(s) = self.session(agent_id) {
            s.told_upto = upto;
        }
    }
    /// Add a line to the log.
    pub(super) fn record(&mut self, e: Entry) {
        self.record_at(e, now_ms());
    }
    /// Add a line that was said at a time already past: it takes its place among the others by that time.
    pub(super) fn record_at(&mut self, mut e: Entry, at: u64) {
        self.seq += 1;
        self.rev += 1;
        e.seq = self.seq;
        e.rev = self.rev;
        e.at = at.min(now_ms());
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
    pub(super) fn touch(&mut self, id: &str) {
        self.rev += 1;
        let rev = self.rev;
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.rev = rev;
            self.unsaved.borrow_mut().insert(id.to_string());
        }
    }
    /// The short version of a message is ready: for the call queue and for the chat alike.
    pub(super) fn fill(&mut self, id: &str, j: &judge::Judged) {
        self.judging.remove(id);
        if let Some(e) = self.log.iter_mut().find(|e| e.id == id) {
            e.kind = Some(j.kind);
            e.short = Some(airband_core::redact::redact(&j.say));
            e.more = j.cut;
            e.options = j.options.clone();
            e.recommended = j.recommended;
            e.read = j.read.clone();
            e.background = j.background;
            e.own_session = j.own_session;
            self.touch(id);
        }
        // not worth his ears: it stays in the chat, dimmed, and is not read out
        if j.kind == Kind::Fyi && self.log.iter().any(|e| e.id == id && e.aloud.unheard()) {
            self.set_aloud(id, Aloud::dropped(Dropped::Routine));
        }
    }
    /// The team a line belongs to now, by id: its sender's, or for something he said, the team of the agents it
    /// went to while they are all here and in one team; otherwise the team it was written down under.
    pub(super) fn line_team(&self, e: &Entry) -> Option<TeamId> {
        if let Some(s) = self.sender(e) {
            return Some(s.team.clone());
        }
        let teams: HashSet<&TeamId> = e.to_agents.iter().filter_map(|id| self.find(id)).map(|s| &s.team).collect();
        if teams.len() == 1 && e.to_agents.iter().all(|id| self.find(id).is_some()) { teams.into_iter().next().cloned() } else { e.team.clone() }
    }
    /// What the agents of the team in this folder last said, newest first, in a few words each: for a model
    /// choosing where new work goes. Nothing for a folder with no team yet.
    pub(super) fn lately(&self, folder: &str) -> Vec<String> {
        let Some(id) = self.team_at(folder) else { return Vec::new() };
        self.log.iter().rev().filter(|e| e.team.as_ref() == Some(&id) && !e.his && e.kind != Some(Kind::Fyi)).take(2).map(Entry::line).collect()
    }
    /// A logged line as a radio sees it. The sender's name is what it is called now, if it is still here.
    pub(super) fn message(&self, e: &Entry) -> Message {
        let text = e.line();
        let sender = self.sender(e);
        // a callsign is shown from its parts while its agent is here; one that has gone keeps what it was called
        let to = if e.to_agents.is_empty() { e.to.clone() } else { e.to_agents.iter().zip(&e.to).map(|(id, was)| self.find(id).map(|s| s.callsign.shown()).unwrap_or_else(|| was.clone())).collect() };
        let badge = match (sender, e.to_agents.as_slice()) {
            (Some(s), _) => s.callsign.badge(),
            (None, [one]) if e.his => self.find(one).map(|s| s.callsign.badge()).unwrap_or_default(),
            _ => String::new(),
        };
        Message {
            seq: e.seq,
            his: e.his,
            id: e.id.clone(),
            at: e.at,
            from: sender.map(|s| s.callsign.shown()).unwrap_or_else(|| e.from.clone()),
            badge,
            to_agents: e.to_agents.clone(),
            // a line whose team has gone has no team name to show
            group: self.line_team(e).and_then(|t| self.team_name(&t)).unwrap_or_default().to_string(),
            team: self.line_team(e),
            to,
            kind: e.kind,
            long: e.more_to_read(),
            text,
            options: e.options.clone(),
            recommended: e.recommended,
            read: e.read.clone(),
            own_session: e.own_session,
            seen: e.seen,
            queued: e.held == Some(Held::Queued),
            cancelled: e.held == Some(Held::Cancelled),
            // a question put through the same door is an ordinary question to him: any answer will do
            // only while the harness is still waiting on it
            permission: e.leave.is_some() && e.permit.as_ref().is_some_and(|id| self.permits.get(id).is_some_and(|p| p.decided.is_none())),
            pictures: e.pictures.clone(),
            links: if e.his { Vec::new() } else { self.links.links(&airband_core::catalog::current().links, &e.id, &e.full) },
            agent: e.agent.clone(),
            rev: e.rev,
            open: false,
            not_read: e.aloud.not_read(),
            note: e.note,
        }
    }
    /// Does this line belong in one team's view (`team`), or one agent's (`agent`, its id)? With neither, every
    /// line does. His words belong with the agents they went to.
    pub(super) fn in_view(&self, e: &Entry, team: Option<&TeamId>, agent: Option<&str>) -> bool {
        let of = |id: &String| self.find(id).map(|s| &s.team);
        let by_agent = agent.is_none_or(|a| e.agent.as_deref() == Some(a) || e.to_agents.iter().any(|t| t == a));
        let by_team = team.is_none_or(|t| self.line_team(e).as_ref() == Some(t) || e.to_agents.iter().any(|id| of(id) == Some(t)));
        by_agent && by_team
    }
    /// The messages that need him and that nothing has answered yet. One at most per agent: its latest, while
    /// it has said nothing since, he has sent it nothing since, and it is still on the radio.
    pub(super) fn open_ids(&self) -> HashSet<String> {
        // A question stays open until he says something to the agent that asked it, or it asks a newer one.
        // What the agent goes on to report does not close it at once: he still has not answered. But one it
        // has spoken past three times, or that is six hours old, is let go: by then it has decided for itself
        // or asked again, and it should not keep calling for him.
        // That last part is a rule of this product and not a fact about the agent: nothing an agent's tool
        // reports says that a question no longer stands.
        const SPOKEN_PAST: u32 = 3;
        const TOO_OLD_MS: u64 = 6 * 3600 * 1000;
        let mut latest: HashMap<&str, (&str, u64, u32)> = HashMap::new();
        for e in &self.log {
            if let Some(agent) = e.agent.as_deref() {
                if e.kind == Some(Kind::NeedsYou) {
                    if e.held == Some(Held::Dismissed) {
                        latest.remove(agent);
                    } else {
                        latest.insert(agent, (&e.id, e.at, 0));
                    }
                } else if e.kind != Some(Kind::Fyi)
                    && let Some(q) = latest.get_mut(agent) {
                        q.2 += 1;
                    }
            } else {
                for to in &e.to_agents {
                    latest.remove(to.as_str());
                }
            }
        }
        let now = now_ms();
        // a request for permission is open for as long as the harness is waiting on it, whatever its age
        let permits: HashSet<&str> = self.permits.values().filter(|p| p.decided.is_none()).map(|p| p.entry.as_str()).collect();
        latest
            .into_iter()
            .filter(|(agent, (id, at, past))| self.find(agent).is_some() && (permits.contains(id) || (*past < SPOKEN_PAST && now.saturating_sub(*at) < TOO_OLD_MS)))
            .map(|(_, (id, _, _))| id.to_string())
            .collect()
    }
    /// The last thing he said to an agent: a message addressed to it by its id. The task an agent the tower
    /// runs was started with is one (`launch::launched`).
    pub(super) fn last_instruction(&self, agent_id: &str) -> Option<String> {
        self.log.iter().rev().find(|e| e.his && e.to_agents.iter().any(|a| a == agent_id)).map(|e| e.full.clone())
    }
    /// The last thing an agent said that was worth his attention.
    pub(super) fn last_said(&self, agent_id: &str) -> Option<&Entry> {
        self.log.iter().rev().find(|e| e.worth_from(agent_id))
    }
}

#[cfg(test)]
mod typed_tests {
    use super::typed_line_id;

    #[test]
    fn two_things_the_harness_tells_apart_are_never_one_line() {
        // the same once letters and digits are kept, and the same in their first forty
        let long = "a".repeat(40);
        assert_ne!(typed_line_id("a1", "msg-1_2"), typed_line_id("a1", "msg-12"));
        assert_ne!(typed_line_id("a1", &format!("{long}x")), typed_line_id("a1", &format!("{long}y")));
        assert_ne!(typed_line_id("a1", "m1"), typed_line_id("a2", "m1"), "two agents' records are their own");
        assert_eq!(typed_line_id("a1", "m1"), typed_line_id("a1", "m1"), "the same one read again is the same line");
        assert_eq!(typed_line_id("a1", ""), None);
    }
}
