//! Reading aloud: where each thing an agent said stands on its way to being heard on a call, and what takes
//! it out of the queue before it is.
use super::*;

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
    /// It will not be read out, and why. `missed`: it was still to be read to a radio on a call when that was
    /// decided, so he was listening and did not hear it. [`Inner::set_aloud`] works that out; nobody else sets it.
    Dropped {
        why: Dropped,
        #[serde(default)]
        missed: bool,
    },
}
/// Why something an agent said is not read out: the same reasons a radio is told (`airband.radio/2`).
pub(super) use airband_core::radio::NotRead as Dropped;
/// Does a line dropped for this reason count as one he missed, when it was dropped while he had reading aloud
/// on? Not one that was never for his ears, and not one said before he turned reading on.
fn missed(why: Dropped) -> bool {
    !matches!(why, Dropped::Routine | Dropped::Stale)
}
impl Aloud {
    /// Still to be heard: waiting its turn, or with a radio that has not said he heard it.
    pub(super) fn unheard(&self) -> bool {
        matches!(self, Aloud::Waiting | Aloud::Handed { .. })
    }
    /// Not to be read out, for this reason.
    pub(super) fn dropped(why: Dropped) -> Aloud {
        Aloud::Dropped { why, missed: false }
    }
    /// The reason it was not read out, when he was listening and so missed it.
    pub(super) fn not_read(&self) -> Option<Dropped> {
        match self {
            Aloud::Dropped { why, missed: true } => Some(*why),
            _ => None,
        }
    }
    /// For the log: the state in a word or two.
    pub(super) fn words(&self) -> String {
        match self {
            Aloud::No => "not for reading".into(),
            Aloud::Waiting => "waiting".into(),
            Aloud::Handed { radio, .. } => format!("handed to {radio}"),
            Aloud::Heard { radio, .. } => format!("heard on {radio}"),
            Aloud::Dropped { why, .. } => format!("dropped ({})", serde_json::to_value(why).ok().and_then(|v| v.as_str().map(|w| w.replace('_', " "))).unwrap_or_default()),
        }
    }
}
impl Inner {
    /// The one way a line's place in the reading queue changes. Every change is written to the log with what it
    /// was and what it became, so "why was that not read to me" has an answer.
    pub(super) fn set_aloud(&mut self, id: &str, to: Aloud) {
        let on_call = self.connected();
        let Some(e) = self.log.iter_mut().find(|e| e.id == id) else { return };
        // dropped while it was still to be read to a radio on a call: he was listening and did not hear it
        let to = match to {
            Aloud::Dropped { why, .. } => Aloud::Dropped { why, missed: on_call && e.aloud.unheard() && missed(why) },
            other => other,
        };
        if e.aloud == to {
            return;
        }
        let line = format!("aloud: {} ({id}) {} → {}", e.from, e.aloud.words(), to.words());
        e.aloud = to;
        self.unsaved.borrow_mut().insert(id.to_string());
        // a radio showing the chat is sent the line again, to mark it
        if e.aloud.not_read().is_some() {
            self.rev += 1;
            e.rev = self.rev;
        }
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
    /// This agent is no longer waiting on him: he spoke to it, or it went back to work. What it was waiting
    /// for is not read out any more. A report he has not heard yet still is: he asked to hear what his
    /// agents report, and talking to one is not the same as having heard it.
    pub(super) fn no_longer_waiting(&mut self, agent_id: &str) {
        self.drop_asks(agent_id, Dropped::Answered);
    }
    pub(super) fn drop_asks(&mut self, agent_id: &str, why: Dropped) {
        let asks: Vec<String> = self.unheard().filter(|e| e.agent.as_deref() == Some(agent_id) && matches!(e.kind, Some(Kind::NeedsYou | Kind::Stalled))).map(|e| e.id.clone()).collect();
        for id in asks {
            self.set_aloud(&id, Aloud::dropped(why));
        }
    }
    /// This agent has said something newer: only the latest thing each agent said is read out. A notice from
    /// its harness takes the place of an earlier notice, and what it said of what it said before.
    pub(super) fn supersede(&mut self, agent_id: &str, notice: bool) {
        let older: Vec<String> = self.unheard().filter(|e| e.agent.as_deref() == Some(agent_id) && e.notice == notice).map(|e| e.id.clone()).collect();
        for id in older {
            self.set_aloud(&id, Aloud::dropped(Dropped::Superseded));
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
        let recent = now_ms().saturating_sub(STILL_NEWS.as_millis() as u64);
        let old: Vec<String> = self.unheard().filter(|e| e.at < recent).map(|e| e.id.clone()).collect();
        for id in old {
            self.set_aloud(&id, Aloud::dropped(Dropped::Stale));
        }
    }
    /// At start. Nothing is with a radio any more. What an agent that is gone was waiting on him for goes; what
    /// it reported in the last while is still his news.
    pub(super) fn settle_reading(&mut self) {
        let live: HashSet<String> = self.sessions.iter().map(|s| s.agent.id.clone()).collect();
        let recent = now_ms().saturating_sub(STILL_NEWS.as_millis() as u64);
        let unheard: Vec<(String, bool, bool, bool)> = self.unheard().map(|e| (e.id.clone(), e.agent.as_ref().is_some_and(|a| live.contains(a)), matches!(e.kind, Some(Kind::Report | Kind::Unjudged)), e.at >= recent)).collect();
        for (id, live, report, recent) in unheard {
            self.set_aloud(&id, match (live, report, recent) {
                (true, _, _) | (false, true, true) => Aloud::Waiting,
                (false, true, false) => Aloud::dropped(Dropped::Stale),
                (false, false, _) => Aloud::dropped(Dropped::AgentGone),
            });
        }
    }
}
