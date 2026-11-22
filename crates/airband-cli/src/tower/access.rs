//! The three ways to hold the tower's state: to look, to change what is kept (written when let go), and to
//! change what passes with this run.
use super::*;

/// The tower's state, held to change it. When it is let go, what was changed is written down, and whoever
/// waits on the tower is told if `tell` was asked. So no path through a change can leave without saving:
/// there is nothing to remember to call.
pub(super) struct Changing<'a> {
    app: &'a App,
    inner: std::sync::MutexGuard<'a, Inner>,
    changed: bool,
    tell: bool,
}

impl Changing<'_> {
    /// Those waiting on the tower (a phone holding a request open, the pump) are woken once this is let go.
    pub(super) fn tell(&mut self) {
        self.tell = true;
    }
    /// The state, to change something that is not kept (what an agent is doing this minute, a call's place
    /// in the queue): changing it this way alone writes nothing. For the few places that look often and
    /// change what is kept only now and then; they say `keep` when they have.
    pub(super) fn unkept(&mut self) -> &mut Inner {
        &mut self.inner
    }
    /// What was changed through `unkept` is to be written after all.
    pub(super) fn keep(&mut self) {
        self.changed = true;
    }
}

impl std::ops::Deref for Changing<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.inner
    }
}

impl std::ops::DerefMut for Changing<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        self.changed = true;
        &mut self.inner
    }
}

impl Drop for Changing<'_> {
    fn drop(&mut self) {
        if self.changed {
            self.app.save(&self.inner);
        }
        if self.tell {
            self.app.kick.notify_one();
        }
    }
}

/// The tower's state, held for something that is not kept: who is on a call, what a phone is in the middle
/// of saying, counts for this run. Nothing is written when it is let go.
pub(super) struct Passing<'a>(std::sync::MutexGuard<'a, Inner>);

impl std::ops::Deref for Passing<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl std::ops::DerefMut for Passing<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        &mut self.0
    }
}

/// The tower's state, held to look at.
pub(super) struct Reading<'a>(std::sync::MutexGuard<'a, Inner>);

impl std::ops::Deref for Reading<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl App {
    /// Look at the tower's state.
    pub(super) fn read(&self) -> Reading<'_> {
        Reading(self.inner.lock().unwrap())
    }
    /// Change the tower's state: written down when the hold is let go, if anything was changed.
    pub(super) fn change(&self) -> Changing<'_> {
        Changing { app: self, inner: self.inner.lock().unwrap(), changed: false, tell: false }
    }
    /// Change what is not kept from one run of the tower to the next. Nothing is written.
    pub(super) fn passing(&self) -> Passing<'_> {
        Passing(self.inner.lock().unwrap())
    }
    /// Write what has changed: new and updated messages, and who is on the radio.
    pub(super) fn save(&self, inner: &Inner) {
        let touched: HashSet<String> = inner.unsaved.borrow_mut().drain().collect();
        let entries: Vec<Entry> = inner.log.iter().filter(|e| touched.contains(&e.id)).cloned().collect();
        self.store.put_messages(&entries);
        self.store.save_state(&Saved { sessions: inner.sessions.clone(), teams: inner.teams.clone(), projects: BTreeMap::new(), named_teams: BTreeSet::new(), held: inner.held.clone(), menus: inner.menus.clone(), tools: inner.tools.clone(), placed: inner.placed.clone() });
    }
}
