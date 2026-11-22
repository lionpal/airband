//! Processes on this computer, as the tower asks after them: whether one is still running, and asking one to
//! end. The tower never kills a process; it asks, and waits.
use std::time::Duration;

pub(in crate::tower) trait Procs: Sync {
    /// Whether a process with this id is running.
    fn alive(&self, pid: u32) -> bool;
    /// Ask it to end, so it can finish what it must and pass the word on to its own.
    fn terminate(&self, pid: u32);
    /// Start a program in a session of its own, with nothing to read or write: whatever stops the tower and its
    /// children leaves it alone. It is collected when it ends, never waited for.
    fn start_apart(&self, run: Run) -> Result<(), String>;
}

/// A program to start: what, with which words, and what is set or taken out of its surroundings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::tower) struct Run {
    pub(in crate::tower) program: std::path::PathBuf,
    pub(in crate::tower) args: Vec<std::ffi::OsString>,
    pub(in crate::tower) set: Vec<(String, String)>,
    pub(in crate::tower) unset: Vec<String>,
}

impl Run {
    pub(in crate::tower) fn arg(&mut self, a: impl Into<std::ffi::OsString>) -> &mut Run {
        self.args.push(a.into());
        self
    }
    /// The words as text, for tests and the log.
    #[cfg(test)]
    pub(in crate::tower) fn words(&self) -> Vec<String> {
        self.args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }
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
    #[cfg(unix)]
    fn start_apart(&self, run: Run) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(&run.program);
        cmd.args(&run.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        for k in &run.unset {
            cmd.env_remove(k);
        }
        for (k, v) in &run.set {
            cmd.env(k, v);
        }
        // a session of its own: whatever stops the tower and its children leaves this alone
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        // not waited for as a child would be: collected when it ends, so it does not linger as a dead entry
        std::thread::spawn(move || {
            child.wait().ok();
        });
        Ok(())
    }
    #[cfg(not(unix))]
    fn start_apart(&self, _run: Run) -> Result<(), String> {
        Err("starting a program apart is not available on this system yet".into())
    }
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
    pub(in crate::tower) started: std::sync::Mutex<Vec<Run>>,
    pub(in crate::tower) lingers: Option<usize>,
    pub(in crate::tower) asked_to_end: std::sync::atomic::AtomicBool,
    looked: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl FakeProcs {
    pub(in crate::tower) fn ending_after(lingers: Option<usize>) -> FakeProcs {
        FakeProcs { started: Default::default(), lingers, asked_to_end: Default::default(), looked: Default::default() }
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
    fn start_apart(&self, run: Run) -> Result<(), String> {
        self.started.lock().unwrap().push(run);
        Ok(())
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
