//! The two ways to hold the tower's state: to look, and to change it. Whatever a change does to a kept table
//! is written when the hold is let go, however the change was made (see [`Inner::changes`]).
use super::*;

/// The tower's state, held to change it. When it is let go, whatever changed in a kept table and every message
/// touched is handed to the saver (written off the lock), and whoever waits on the tower is told if `tell` was
/// asked. So no path through a change can leave without saving: there is nothing to remember to call.
pub(super) struct Changing<'a> {
    app: &'a App,
    inner: std::sync::MutexGuard<'a, Inner>,
    tell: bool,
}

impl Changing<'_> {
    /// Those waiting on the tower (a phone holding a request open, the pump) are woken once this is let go.
    pub(super) fn tell(&mut self) {
        self.tell = true;
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
        &mut self.inner
    }
}

impl Drop for Changing<'_> {
    fn drop(&mut self) {
        // handed over while still held, so the saver writes changes in the order they were made
        self.app.saver.send(self.inner.changes());
        if self.tell {
            self.app.kick.notify_one();
        }
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
    /// Change the tower's state: what changed in it is written once the hold is let go.
    pub(super) fn change(&self) -> Changing<'_> {
        Changing { app: self, inner: self.inner.lock().unwrap(), tell: false }
    }
    /// Let go of the agents that have gone and the requests for leave nobody waits on: written down, and the
    /// radios told, when there were any.
    pub(super) fn reap(&self) {
        let mut inner = self.change();
        if inner.reap() {
            inner.tell();
        }
    }
    /// Wait until every change made so far is written to the database.
    pub(super) fn flush(&self) {
        self.saver.flush();
    }
}
