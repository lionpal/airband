//! A file of ours, kept as JSON: read once when it is opened, saved again whole after each change. What happens
//! when it is damaged is decided here, once, for the tower, the phone and the relay:
//!
//! | what is found                         | what happens                                                        |
//! |---------------------------------------|---------------------------------------------------------------------|
//! | no file                               | [`Loaded::Fresh`]: the default, saved as usual                      |
//! | a file that is not the value          | moved aside beside it ([`Loaded::SetAside`]), the default in its place |
//! | one that cannot be moved aside        | left as it is ([`Loaded::LeftInPlace`]): every save is refused      |
//! | a read that fails (out of files, an error of the disk) | tried again twice; then left as it is, saves refused |
//!
//! So nothing kept is ever lost by being taken for empty and written over. A file that is a link is written
//! where it points ([`crate::files`]).
//!
//! [`Kept`] holds the value itself. [`Keeper`] is the same without it, for a value that lives inside something
//! else (the tower's state, a struct with more in it).
use crate::files::write_atomic;
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// What opening a kept file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// Nothing was kept there yet.
    Fresh,
    /// What was kept, read whole.
    Read,
    /// What was there could not be read as what is kept there. It was moved aside, under
    /// `<name>.unreadable-<random>`, and saves go to the path as usual. `why` says where it went, for a log.
    SetAside { why: String },
    /// What was there could not be read, and is left as it is: every save is refused, so it can still be had back.
    LeftInPlace { why: String },
}

impl Loaded {
    /// What is worth a line in a log, when something was wrong.
    pub fn trouble(&self) -> Option<&str> {
        match self {
            Loaded::Fresh | Loaded::Read => None,
            Loaded::SetAside { why } | Loaded::LeftInPlace { why } => Some(why),
        }
    }
}

/// What came of a save.
#[must_use = "a save that failed must be told"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Written whole.
    Kept,
    /// This keeps nothing (a test, a host with no folder).
    Nowhere,
    /// Not written. `first` is true for the first failure since the last save that worked: the one to tell him.
    Failed { why: String, first: bool },
}

impl Saved {
    /// Why, when this is the first failure in a row: what to tell, once.
    pub fn news(&self) -> Option<&str> {
        match self {
            Saved::Failed { why, first: true } => Some(why),
            _ => None,
        }
    }
    /// As a result, for a caller that tells every failure.
    pub fn into_result(self) -> Result<(), String> {
        match self {
            Saved::Kept | Saved::Nowhere => Ok(()),
            Saved::Failed { why, .. } => Err(why),
        }
    }
}

/// Where a kept value is saved, and whether it may be.
#[derive(Debug, Default)]
pub struct Keeper {
    path: Option<PathBuf>,
    /// Why nothing may be written there: what is there could not be read, and is still there.
    read_only: Option<String>,
    failing: AtomicBool,
}

impl Keeper {
    /// Open the file at `path` (none: keep nothing), with the value it holds: the default when there is none or
    /// it cannot be read.
    pub fn open<T: DeserializeOwned + Default>(path: Option<PathBuf>) -> (T, Keeper, Loaded) {
        let Some(at) = path.clone() else { return (T::default(), Keeper::default(), Loaded::Fresh) };
        let (value, loaded) = load(&at);
        let read_only = match &loaded {
            Loaded::LeftInPlace { why } => Some(why.clone()),
            _ => None,
        };
        (value.unwrap_or_default(), Keeper { path, read_only, failing: AtomicBool::new(false) }, loaded)
    }
    /// One that keeps nothing.
    pub fn nowhere() -> Keeper {
        Keeper::default()
    }
    /// Save `value`, whole or not at all.
    pub fn save<T: Serialize + ?Sized>(&self, value: &T) -> Saved {
        let Some(path) = &self.path else { return Saved::Nowhere };
        let done = match &self.read_only {
            Some(why) => Err(format!("{why}; nothing is written over it")),
            None => serde_json::to_vec(value)
                .map_err(std::io::Error::other)
                .and_then(|bytes| write_atomic(path, bytes))
                .map_err(|e| format!("{} could not be written ({e})", path.display())),
        };
        match done {
            Ok(()) => {
                self.failing.store(false, Ordering::SeqCst);
                Saved::Kept
            }
            Err(why) => Saved::Failed { why, first: !self.failing.swap(true, Ordering::SeqCst) },
        }
    }
    /// Where it is kept.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// A kept value and its file.
#[derive(Debug, Default)]
pub struct Kept<T> {
    keeper: Keeper,
    value: Mutex<T>,
}

impl<T: Serialize + DeserializeOwned + Default> Kept<T> {
    /// Open the file at `path` (none: keep nothing).
    pub fn open(path: Option<PathBuf>) -> (Kept<T>, Loaded) {
        let (value, keeper, loaded) = Keeper::open(path);
        (Kept { keeper, value: Mutex::new(value) }, loaded)
    }
    /// One that keeps nothing, holding `value`.
    pub fn nowhere(value: T) -> Kept<T> {
        Kept { keeper: Keeper::nowhere(), value: Mutex::new(value) }
    }
    /// The value, to read.
    pub fn get(&self) -> MutexGuard<'_, T> {
        self.value.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Change the value and save it, under one lock so two changes are never saved out of order.
    pub fn change<R>(&self, f: impl FnOnce(&mut T) -> R) -> (R, Saved) {
        let mut value = self.get();
        let out = f(&mut value);
        let saved = self.keeper.save(&*value);
        (out, saved)
    }
    /// Where it is kept.
    pub fn path(&self) -> Option<&Path> {
        self.keeper.path()
    }
}

/// What a file holds, for a reader that reads it again each time under a lock of its own (the relay's
/// folder store): a file that is not there is `Ok(None)`; one that cannot be read or is not the value is an
/// error saying why, so nothing writes over it as if it had been empty. Nothing is moved.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    parsed(path, read_trying(path))
}

/// Keep a value as JSON at `path`, whole or not at all, for a writer that read it with [`read`].
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> std::io::Result<()> {
    write_atomic(path, serde_json::to_vec(value).map_err(std::io::Error::other)?)
}

/// The value at `path` and what was found. Only a file that was read and is not the value is moved aside; a
/// read that failed leaves the file where it is.
fn load<T: DeserializeOwned>(path: &Path) -> (Option<T>, Loaded) {
    let bytes = match read_trying(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, Loaded::Fresh),
        Err(e) => return (None, Loaded::LeftInPlace { why: format!("{} cannot be read ({e}) and is left as it is", path.display()) }),
        Ok(bytes) => bytes,
    };
    let why = match serde_json::from_slice(&bytes) {
        Ok(value) => return (Some(value), Loaded::Read),
        Err(e) => format!("{} is not what was kept there ({e})", path.display()),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let aside = path.with_file_name(format!("{name}.unreadable-{}", airband_core::encode::random_hex(4)));
    match std::fs::rename(path, &aside) {
        Ok(()) => (None, Loaded::SetAside { why: format!("{why}; it was kept as {}", aside.display()) }),
        Err(e) => (None, Loaded::LeftInPlace { why: format!("{why}; it could not be moved aside ({e}) and is left as it is") }),
    }
}

/// Is this a failure that passes: the read was interrupted, or the system had no file to give for a moment?
fn brief(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{Interrupted, TimedOut, WouldBlock};
    #[cfg(unix)]
    if matches!(e.raw_os_error(), Some(libc::EMFILE | libc::ENFILE | libc::EAGAIN)) {
        return true;
    }
    matches!(e.kind(), Interrupted | WouldBlock | TimedOut)
}

fn read_trying(path: &Path) -> std::io::Result<Vec<u8>> {
    trying(|| std::fs::read(path))
}

/// `bytes`, with a brief failure tried twice more, a moment apart.
fn trying(mut bytes: impl FnMut() -> std::io::Result<Vec<u8>>) -> std::io::Result<Vec<u8>> {
    let mut tries = 0;
    loop {
        match bytes() {
            Err(e) if brief(&e) && tries < 2 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * tries));
            }
            got => return got,
        }
    }
}

fn parsed<T: DeserializeOwned>(path: &Path, got: std::io::Result<Vec<u8>>) -> Result<Option<T>, String> {
    match got {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} cannot be read: {e}", path.display())),
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{} is not what was kept there: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::folder;

    #[test]
    fn a_value_comes_back_as_it_went_in_and_no_file_is_fresh() {
        let dir = folder();
        let file = dir.join("deep").join("marks.json");
        let (kept, loaded) = Kept::<Vec<u32>>::open(Some(file.clone()));
        assert_eq!(loaded, Loaded::Fresh);
        assert_eq!(kept.change(|v| v.push(1)).1, Saved::Kept);
        let (again, loaded) = Kept::<Vec<u32>>::open(Some(file));
        assert_eq!((loaded, again.get().clone()), (Loaded::Read, vec![1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_the_value_is_moved_aside_and_the_next_save_leaves_it_alone() {
        let dir = folder();
        let half = dir.join("half.json");
        std::fs::write(&half, "[1, 2").unwrap();
        let (kept, loaded) = Kept::<Vec<u32>>::open(Some(half.clone()));
        assert!(matches!(loaded, Loaded::SetAside { .. }), "{loaded:?}");
        assert!(!half.exists());
        assert_eq!(kept.change(|v| v.push(9)).1, Saved::Kept);
        let aside: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p != &half).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(&aside[0]).unwrap(), "[1, 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn one_that_cannot_be_moved_aside_is_left_and_every_save_is_refused_and_told_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = folder();
        let file = dir.join("half.json");
        std::fs::write(&file, "[1,").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let (kept, loaded) = Kept::<Vec<u32>>::open(Some(file.clone()));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(loaded, Loaded::LeftInPlace { .. }), "{loaded:?}");
        let first = kept.change(|v| v.push(1)).1;
        let second = kept.change(|v| v.push(2)).1;
        let left = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first.news().is_some() && second.news().is_none(), "{first:?} {second:?}");
        assert_eq!(left, "[1,");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_that_fails_for_a_moment_is_tried_again_and_one_that_goes_on_failing_sets_nothing_aside() {
        let mut first = true;
        let got = trying(|| if std::mem::take(&mut first) { Err(std::io::Error::from_raw_os_error(libc::EMFILE)) } else { Ok(b"[7]".to_vec()) });
        assert_eq!(got.unwrap(), b"[7]");
        // a file that cannot be read at all (here, a folder by that name) is left where it is, never set aside
        let dir = folder();
        let file = dir.join("marks.json");
        std::fs::create_dir_all(&file).unwrap();
        let (kept, loaded) = Kept::<Vec<u32>>::open(Some(file.clone()));
        assert!(matches!(loaded, Loaded::LeftInPlace { .. }), "{loaded:?}");
        assert!(matches!(kept.change(|_| ()).1, Saved::Failed { .. }));
        assert!(file.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_works_again_is_told_again_when_it_next_fails() {
        let dir = folder();
        let file = dir.join("in").join("marks.json");
        let (kept, _) = Kept::<Vec<u32>>::open(Some(file.clone()));
        // a file where its folder should be: saves fail
        std::fs::write(dir.join("in"), b"").unwrap();
        assert!(kept.change(|_| ()).1.news().is_some());
        assert!(kept.change(|_| ()).1.news().is_none());
        std::fs::remove_file(dir.join("in")).unwrap();
        assert_eq!(kept.change(|_| ()).1, Saved::Kept);
        std::fs::write(dir.join("in.tmp"), b"").unwrap();
        std::fs::remove_dir_all(dir.join("in")).unwrap();
        std::fs::write(dir.join("in"), b"").unwrap();
        assert!(kept.change(|_| ()).1.news().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_that_keeps_nothing_saves_nowhere_and_read_takes_nothing_for_empty() {
        assert_eq!(Kept::nowhere(vec![1u32]).change(|v| v.push(2)).1, Saved::Nowhere);
        let dir = folder();
        assert_eq!(read::<Vec<u32>>(&dir.join("none.json")), Ok(None));
        std::fs::write(dir.join("half.json"), "[1").unwrap();
        assert!(read::<Vec<u32>>(&dir.join("half.json")).is_err());
        assert!(dir.join("half.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
