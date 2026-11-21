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
    let part = path.with_file_name(format!(".{name}.part-{}", crate::relay::random_hex(4)));
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

/// What a kept file holds, as JSON. Nothing when there is no file or it cannot be read as that.
pub fn load<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Keep a value as JSON, whole or not at all. One that cannot be written as JSON is not written, and the
/// old file stays.
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> std::io::Result<()> {
    write_atomic(path, serde_json::to_vec(value).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("airband-kept-{}", crate::relay::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_replaced_whole_and_nothing_is_left_beside_it() {
        let dir = folder();
        let file = dir.join("deep").join("marks.json");
        // its folder is made; a value comes back as it went in
        save(&file, &vec![1, 2, 3]).unwrap();
        assert_eq!(load::<Vec<u32>>(&file), Some(vec![1, 2, 3]));
        save(&file, &vec![4]).unwrap();
        assert_eq!(load::<Vec<u32>>(&file), Some(vec![4]));
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        // no file, or one that is not that: nothing
        assert_eq!(load::<Vec<u32>>(&dir.join("none.json")), None);
        std::fs::write(dir.join("half.json"), "[1, 2").unwrap();
        assert_eq!(load::<Vec<u32>>(&dir.join("half.json")), None);
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
