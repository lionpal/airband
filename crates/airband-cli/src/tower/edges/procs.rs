//! Processes on this computer, as the tower asks after them: whether one is still running, and asking one to
//! end. The tower never kills a process; it asks, and waits.
use std::time::Duration;

pub(in crate::tower) trait Procs: Sync {
    /// Whether a process with this id is running.
    fn alive(&self, pid: u32) -> bool;
    /// Ask it to end, so it can finish what it must and pass the word on to its own.
    fn terminate(&self, pid: u32);
}

/// This computer's own processes.
pub(in crate::tower) struct SystemProcs;

impl Procs for SystemProcs {
    #[cfg(unix)]
    fn alive(&self, pid: u32) -> bool {
        // signal 0 only asks; "not permitted" still means it exists
        unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(not(unix))]
    fn alive(&self, _pid: u32) -> bool {
        true // not checked on this platform yet: an agent leaves when it signs off or a delivery fails
    }
    #[cfg(unix)]
    fn terminate(&self, pid: u32) {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    fn terminate(&self, _pid: u32) {}
}

/// How long a process asked to end is waited for, and how often it is looked at meanwhile.
const ENDS_WITHIN: Duration = Duration::from_secs(10);
const LOOKED_AT_EVERY: Duration = Duration::from_millis(200);

/// Ask a process to end and wait until it is gone. Err when it is still running after [`ENDS_WITHIN`].
pub(in crate::tower) async fn end_and_wait(procs: &dyn Procs, pid: u32) -> Result<(), ()> {
    procs.terminate(pid);
    let until = tokio::time::Instant::now() + ENDS_WITHIN;
    while procs.alive(pid) {
        if tokio::time::Instant::now() >= until {
            return Err(());
        }
        tokio::time::sleep(LOOKED_AT_EVERY).await;
    }
    Ok(())
}

/// Processes that end when asked, after being looked at `lingers` more times; or never, when `lingers` is
/// `None`.
#[cfg(test)]
pub(in crate::tower) struct FakeProcs {
    pub(in crate::tower) lingers: Option<usize>,
    pub(in crate::tower) asked_to_end: std::sync::atomic::AtomicBool,
    looked: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl FakeProcs {
    pub(in crate::tower) fn ending_after(lingers: Option<usize>) -> FakeProcs {
        FakeProcs { lingers, asked_to_end: Default::default(), looked: Default::default() }
    }
}

#[cfg(test)]
impl Procs for FakeProcs {
    fn alive(&self, _pid: u32) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        if !self.asked_to_end.load(SeqCst) {
            return true;
        }
        let looked = self.looked.fetch_add(1, SeqCst);
        self.lingers.is_none_or(|n| looked < n)
    }
    fn terminate(&self, _pid: u32) {
        self.asked_to_end.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_process_asked_to_end_is_waited_for_and_one_that_stays_is_said_to() {
        let ends = FakeProcs::ending_after(Some(3));
        assert_eq!(end_and_wait(&ends, 1).await, Ok(()));
        assert!(ends.asked_to_end.load(std::sync::atomic::Ordering::SeqCst));
        let stays = FakeProcs::ending_after(None);
        assert_eq!(end_and_wait(&stays, 1).await, Err(()));
    }
}
