//! Asking him for leave on a tool's behalf, and waiting for his answer.
use super::*;

/// Requests for leave a watcher puts to him itself. Each is put by its owner (a session, or the request itself),
/// one at a time for each owner; an owner whose try found nobody there is tried again after a while. Each
/// request answered or left, by its own id, is not put again while its owner still waits.
#[derive(Default)]
pub(crate) struct Asking {
    busy: std::collections::HashSet<String>,
    pub(super) again: std::collections::HashMap<String, std::time::Instant>,
    /// Requests answered or left, each with its owner.
    done: std::collections::HashMap<String, String>,
}

impl Asking {
    /// How long one that found nobody there waits before it is tried again.
    const AGAIN: Duration = Duration::from_secs(20);
    /// Take this owner's requests to put to him now. False while one is being put, or before it is due again.
    pub(crate) fn take(&mut self, owner: &str) -> bool {
        let due = !self.busy.contains(owner) && self.again.get(owner).is_none_or(|at| *at <= std::time::Instant::now());
        if due {
            self.busy.insert(owner.to_string());
        }
        due
    }
    /// Was this request answered or left already?
    pub(crate) fn done(&self, request: &str) -> bool {
        self.done.contains_key(request)
    }
    /// What came of putting the owner's request to him. Nothing is a try that could not be made (the tool could
    /// not be followed, or his answer could not be given to it), and is tried again after a while.
    pub(crate) fn over(&mut self, owner: &str, request: &str, came: Option<&Answer>) {
        self.busy.remove(owner);
        match came {
            Some(Answer::Nobody) | None => {
                // what is past its time is no longer waited out: kept, the set would grow for the tower's life
                let now = std::time::Instant::now();
                self.again.retain(|_, until| *until > now);
                self.again.insert(owner.to_string(), now + Self::AGAIN);
            }
            Some(_) => {
                self.again.remove(owner);
                self.done.insert(request.to_string(), owner.to_string());
            }
        }
    }
    /// Forget the requests of owners that are no longer waiting: one waiting again has a new request.
    pub(crate) fn keep_only(&mut self, waiting: impl Fn(&str) -> bool) {
        self.done.retain(|_, owner| waiting(owner));
    }
}

/// What came of putting a request to him.
pub(crate) enum Answer {
    /// He answered: yes or no, and with a no the reason he gave, if any.
    Said(bool, String),
    /// Nobody was there to ask (no tower, or no radio), so nothing was put in front of anyone.
    Nobody,
    /// It was put to him and he did not answer in the time the tower allows, or it was answered elsewhere.
    Unanswered,
}

/// Put a permission request to him through the tower, and wait for his answer. Nothing when there is none to be
/// had: no tower, no radio there to ask, or he did not answer in the time the tower allows. Then the harness
/// asks at the keyboard as it always has. Waiting never turns into a yes.
pub(crate) fn ask_him(request: &PermissionAsk) -> Option<(bool, String)> {
    match ask_him_while(request, None) {
        Answer::Said(allow, reason) => Some((allow, reason)),
        Answer::Nobody | Answer::Unanswered => None,
    }
}

/// The same, for a request that can also be answered somewhere else meanwhile (at the tool's own screen):
/// `still` is asked every few seconds whether the tool is still waiting, and when it is not the question is
/// taken back.
pub(crate) fn ask_him_while(request: &PermissionAsk, still: Option<&dyn Fn() -> bool>) -> Answer {
    let Ok(Some(reach)) = config::reach() else { return Answer::Nobody };
    let here = TowerHere::reached(&reach, Duration::from_secs(40));
    let Ok(Said::Took(asked)) = here.call::<at::Permission>(None, &Nothing {}, Some(request)) else { return Answer::Nobody };
    if asked.state != airband_core::agent::Leave::Pending {
        return Answer::Nobody;
    }
    let until = std::time::Instant::now() + Duration::from_secs(asked.wait_seconds.max(5));
    while std::time::Instant::now() < until {
        if still.is_some_and(|still| !still()) {
            break;
        }
        let left = until.saturating_duration_since(std::time::Instant::now()).as_secs().clamp(1, if still.is_some() { 3 } else { 20 });
        // the tower went away mid-wait: stop waiting and let the harness ask
        let Ok(Said::Took(now)) = here.call::<at::PermissionAnswer>(Some(&asked.id), &WaitQuery { wait: left }, None) else { return Answer::Unanswered };
        use airband_core::agent::Leave;
        match now.state {
            Leave::Allow => return Answer::Said(true, String::new()),
            Leave::Deny => return Answer::Said(false, now.message),
            Leave::Pending => {}
            Leave::Nobody | Leave::Gone => return Answer::Unanswered,
        }
    }
    // he did not answer, or it was settled at the tool: take the question back at once, saying which, so the feed
    // does not offer an answer that no longer does anything
    let why = if still.is_some_and(|still| !still()) { wire::Withdrawn::SettledThere } else { wire::Withdrawn::Unanswered };
    here.call::<at::PermissionWithdraw>(Some(&asked.id), &wire::WithdrawQuery { why }, None).ok();
    Answer::Unanswered
}
