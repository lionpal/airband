//! One thing at a time across every airband process on this computer, by a lock file. A lock that cannot be
//! taken is an error: the work is never done without it.

use std::path::Path;

/// Held while the work runs; let go when dropped.
pub(crate) type Held = airband_io::files::Lock;

/// Wait for the lock at `path` and hold it.
pub(crate) fn take(path: &Path) -> Result<Held, String> {
    airband_io::files::lock(path).map_err(|e| format!("cannot take the lock {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_is_held_until_it_is_let_go_and_one_that_cannot_be_opened_is_an_error() {
        let dir = std::env::temp_dir().join(format!("airband-lock-{}", std::process::id()));
        let path = dir.join("one.lock");
        let held = take(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let other = path.clone();
        std::thread::spawn(move || {
            let _second = take(&other).unwrap();
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "a second waits while the first holds it");
        drop(held);
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("and takes it once it is let go");
        // a lock whose folder is a file cannot be made: an error, not work done without it
        std::fs::write(dir.join("file"), "").unwrap();
        assert!(take(&dir.join("file").join("x.lock")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
