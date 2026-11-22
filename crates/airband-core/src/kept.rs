//! Files that are kept: written whole or not at all.
//!
//! Every file the tower, the radio and the relay keep is replaced the same way: the new one is written under
//! another name beside it and takes its name once it is whole and on the disk. A write that is cut short (the
//! app is killed, the disk fills, the power goes) leaves the old file as it was, and nothing ever reads half
//! of one. This is the one place that is done.
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;
use std::path::Path;

/// Who may read the file once it is written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Leave {
    /// Whoever could read the one it replaces. A new one is as the system makes files.
    AsItWas,
    /// Whoever could read the one it replaces. A new one only its owner.
    AsItWasOrPrivate,
    /// Only its owner, whatever it replaces: it holds a secret.
    Private,
}

fn write(path: &Path, bytes: &[u8], leave: Leave) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(|| std::io::Error::other("not a file's name"))?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    // beside it, so taking its name is one step on the same disk; hidden, and never the same name twice
    let part = path.with_file_name(format!(".{name}.part-{}", crate::encode::random_hex(4)));
    let was = std::fs::metadata(path).ok().map(|m| m.permissions());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Only its owner can read it while it is being written, whenever it may hold what must stay private: it
    // replaces a file (which may be private), or it is to be private itself.
    #[cfg(unix)]
    if was.is_some() || leave != Leave::AsItWas {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&part).and_then(|mut f| {
        f.write_all(bytes)?;
        if let (Some(was), true) = (was, leave != Leave::Private) {
            f.set_permissions(was)?;
        }
        f.sync_all()
    });
    let written = written.and_then(|_| std::fs::rename(&part, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// Replace a file with these bytes, whole or not at all. It can be read by whoever could read the one it
/// replaces; a new one is as the system makes files.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    write(path, bytes.as_ref(), Leave::AsItWas)
}

/// The same, for a file of someone else's that may hold a secret (a tool's own login): it keeps the
/// permissions it had, and a new one can be read only by its owner.
pub fn write_keeping(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    write(path, bytes.as_ref(), Leave::AsItWasOrPrivate)
}

/// The same, for a file only its owner may read (it holds a token or a key). It is made that way from the
/// start: nobody else can read it at any moment.
pub fn write_private(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    write(path, bytes.as_ref(), Leave::Private)
}

/// What a kept file holds, as JSON, for a keeper that goes on with nothing when the file cannot be read. A file
/// that is not there is `Ok(None)`. One that is there and cannot be read, or is not the value, is first moved
/// aside (beside it, under `<name>.unreadable-<random>`), so whatever is written to the path next writes over
/// nothing; the error says why and where it went. Nothing kept is ever lost by being taken for empty.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let why = match read(path) {
        Ok(found) => return Ok(found),
        Err(why) => why,
    };
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let aside = path.with_file_name(format!("{name}.unreadable-{}", crate::encode::random_hex(4)));
    match std::fs::rename(path, &aside) {
        Ok(()) => Err(format!("{why}; it was kept as {}", aside.display())),
        Err(e) => Err(format!("{why}; it could not be moved aside ({e}), so nothing will be written over it")),
    }
}

/// A kept value where "not there yet" and "there but unreadable" must not be taken for each other: a file that
/// is not there is `Ok(None)`; one that cannot be read or is not the value is an error saying why, so nothing
/// writes over it as if it had been empty.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} cannot be read: {e}", path.display())),
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{} is not what was kept there: {e}", path.display())),
    }
}

/// Keep a value as JSON, whole or not at all. One that cannot be written as JSON is not written, and the
/// old file stays.
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> std::io::Result<()> {
    write_atomic(path, serde_json::to_vec(value).map_err(std::io::Error::other)?)
}

/// Remove what in `dir` was last changed longer than `age` ago: files, and folders with all they hold. A folder
/// that is not there has nothing to remove.
pub fn prune_older(dir: &Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > age) {
            let _ = if meta.is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("airband-kept-{}", crate::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_unreadable_file_is_kept_so_nothing_writes_over_it() {
        let dir = folder();
        let half = dir.join("half.json");
        std::fs::write(&half, "[1, 2").unwrap();
        let _ = load::<Vec<u32>>(&half);
        save(&half, &vec![9]).unwrap();
        let kept = std::fs::read_dir(&dir).unwrap().flatten().any(|e| std::fs::read_to_string(e.path()).unwrap_or_default() == "[1, 2");
        assert!(kept, "what was there was written over");
        let _ = std::fs::remove_dir_all(&dir);
    }


    #[test]
    fn a_file_is_replaced_whole_and_nothing_is_left_beside_it() {
        let dir = folder();
        let file = dir.join("deep").join("marks.json");
        // its folder is made; a value comes back as it went in
        save(&file, &vec![1, 2, 3]).unwrap();
        assert_eq!(load::<Vec<u32>>(&file), Ok(Some(vec![1, 2, 3])));
        save(&file, &vec![4]).unwrap();
        assert_eq!(load::<Vec<u32>>(&file), Ok(Some(vec![4])));
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        // no file: nothing, and no error
        assert_eq!(load::<Vec<u32>>(&dir.join("none.json")), Ok(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_is_kept_aside_so_nothing_writes_over_it() {
        let dir = folder();
        let half = dir.join("half.json");
        std::fs::write(&half, "[1, 2").unwrap();
        let said = load::<Vec<u32>>(&half).unwrap_err();
        assert!(said.contains("kept as"), "{said}");
        // the path is free, and what was there is beside it, as it was
        assert!(!half.exists());
        let aside: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(&aside[0]).unwrap(), "[1, 2");
        // so what the keeper writes next takes the free name and leaves it alone
        save(&half, &vec![9]).unwrap();
        assert_eq!(std::fs::read_to_string(&aside[0]).unwrap(), "[1, 2");
        // a write that cannot finish leaves the old file as it was, and no part of a new one
        let taken = dir.join("taken");
        std::fs::create_dir_all(taken.join("in-the-way")).unwrap();
        assert!(write_atomic(&taken.join("in-the-way"), b"x").is_err());
        assert_eq!(std::fs::read_dir(&taken).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn who_can_read_it_is_kept_and_a_secret_is_private_from_the_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = folder();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let secret = dir.join("token");
        write_private(&secret, b"s").unwrap();
        assert_eq!(mode(&secret), 0o600);
        // one that was readable by others stays private once it holds a secret
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&secret, b"t").unwrap();
        assert_eq!(mode(&secret), 0o600);
        // someone else's file keeps what it had; a new one of that kind is private
        let theirs = dir.join("settings.json");
        std::fs::write(&theirs, "{}").unwrap();
        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_keeping(&theirs, b"{\"a\":1}").unwrap();
        assert_eq!((mode(&theirs), std::fs::read(&theirs).unwrap()), (0o640, b"{\"a\":1}".to_vec()));
        write_keeping(&dir.join("login.json"), b"{}").unwrap();
        assert_eq!(mode(&dir.join("login.json")), 0o600);
        write_atomic(&theirs, b"{}").unwrap();
        assert_eq!(mode(&theirs), 0o640);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
