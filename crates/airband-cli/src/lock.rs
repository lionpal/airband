//! One thing at a time across every airband process on this computer, by a lock file. A lock that cannot be
//! taken is an error: the work is never done without it.

use std::path::Path;

/// Held while the work runs; let go when dropped.
pub(crate) struct Held {
    _file: std::fs::File,
}

/// Wait for the lock at `path` and hold it.
pub(crate) fn take(path: &Path) -> Result<Held, String> {
    if let Some(dir) = path.parent() {
        airband_io::files::make_folder(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(|e| format!("cannot open the lock {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is the open file's own, and it stays open while the lock is held
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!("cannot take the lock {}: {}", path.display(), std::io::Error::last_os_error()));
        }
    }
    Ok(Held { _file: file })
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
