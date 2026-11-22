//! Files written whole or not at all.
//!
//! Every file the tower, the radio and the relay write is replaced the same way: the new one is written under
//! another name beside it and takes its name once it is whole and on the disk. A write that is cut short (the
//! app is killed, the disk fills, the power goes) leaves the old file as it was, and nothing ever reads half
//! of one. This is the one place that is done.
use std::io::Write;
use std::path::{Path, PathBuf};

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

/// The file a path names, through any symbolic links: a file that is a link (into a folder of his dotfiles,
/// say) is written where the link points, so the link stays a link.
fn through_links(path: &Path) -> std::io::Result<PathBuf> {
    let mut at = path.to_path_buf();
    // as many links in a row as the system itself follows before it gives up
    for _ in 0..40 {
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let to = std::fs::read_link(&at)?;
                // a link's target is read from the folder the link is in; one written whole stays as it is
                at = at.parent().map_or_else(|| to.clone(), |dir| dir.join(&to));
            }
            _ => return Ok(at),
        }
    }
    Err(std::io::Error::other(format!("{} is a link to a link, too many times over", path.display())))
}

fn write(path: &Path, bytes: &[u8], leave: Leave) -> std::io::Result<()> {
    let path = &through_links(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(|| std::io::Error::other("not a file's name"))?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    // beside it, so taking its name is one step on the same disk; hidden, and never the same name twice
    let part = path.with_file_name(format!(".{name}.part-{}", airband_core::encode::random_hex(4)));
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
        std::fs::remove_file(&part).ok();
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

/// Remove what in `dir` was last changed longer than `age` ago: files, and folders with all they hold. A folder
/// that is not there has nothing to remove. What cannot be removed now is tried again the next time.
pub fn prune_older(dir: &Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > age) {
            if meta.is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) }.ok();
        }
    }
}

/// A folder of its own in the system's temporary folder, for a test.
#[cfg(test)]
pub(crate) fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("airband-io-{}", airband_core::encode::random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_replaced_whole_and_a_write_that_cannot_finish_leaves_nothing_beside_it() {
        let dir = folder();
        let file = dir.join("deep").join("marks.json");
        write_atomic(&file, b"1").unwrap();
        write_atomic(&file, b"2").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"2");
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        let taken = dir.join("taken");
        std::fs::create_dir_all(taken.join("in-the-way")).unwrap();
        assert!(write_atomic(&taken.join("in-the-way"), b"x").is_err());
        assert_eq!(std::fs::read_dir(&taken).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_a_link_is_written_where_it_points_and_stays_a_link() {
        let dir = folder();
        let real = dir.join("dotfiles").join("settings.json");
        write_atomic(&real, b"[1]").unwrap();
        let link = dir.join("settings.json");
        std::os::unix::fs::symlink("dotfiles/settings.json", &link).unwrap();
        write_keeping(&link, b"[2]").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"[2]");
        // a link to a file not made yet makes it there
        let ahead = dir.join("ahead.json");
        std::os::unix::fs::symlink(dir.join("dotfiles").join("new.json"), &ahead).unwrap();
        write_atomic(&ahead, b"[3]").unwrap();
        assert!(std::fs::symlink_metadata(&ahead).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(dir.join("dotfiles").join("new.json")).unwrap(), b"[3]");
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
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&secret, b"t").unwrap();
        assert_eq!(mode(&secret), 0o600);
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
