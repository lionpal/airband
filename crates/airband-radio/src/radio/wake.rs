//! What a radio's waiting threads sleep on.
use super::*;

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
