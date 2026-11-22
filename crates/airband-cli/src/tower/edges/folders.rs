//! Folders on this computer's disk, as the tower asks after them: whether one is there, what it really is with
//! links followed, and how he would read its path.
use std::path::Path;
use std::sync::Arc;

pub(in crate::tower) trait Folders: Send + Sync {
    /// Whether this folder is there now.
    fn exists(&self, folder: &str) -> bool;
    /// The folder as it really is on disk, links followed: two ways of writing one folder are one team. One that
    /// is not there any more is read by its parts, as the system reads a path.
    fn real(&self, folder: &str) -> String;
    /// The folder as he would read it: under his home it starts with `~`.
    fn shown(&self, at: &Path) -> String;
}

/// This computer's own disk.
pub(in crate::tower) struct Disk;

impl Folders for Disk {
    fn exists(&self, folder: &str) -> bool {
        Path::new(folder).is_dir()
    }
    fn real(&self, folder: &str) -> String {
        let at = Path::new(folder);
        at.canonicalize().unwrap_or_else(|_| at.components().collect()).display().to_string()
    }
    fn shown(&self, at: &Path) -> String {
        let home = crate::config::home();
        shown_under(&[home.canonicalize().ok(), Some(home)].into_iter().flatten().collect::<Vec<_>>(), at)
    }
}

/// How a path reads under any of these homes.
fn shown_under(homes: &[std::path::PathBuf], at: &Path) -> String {
    match homes.iter().find_map(|home| at.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => at.display().to_string(),
    }
}

/// The disk the tower's state asks: this computer's, or a stand-in in tests.
#[derive(Clone)]
pub(in crate::tower) struct Disks(pub(in crate::tower) Arc<dyn Folders>);

impl Default for Disks {
    fn default() -> Self {
        Disks(Arc::new(Disk))
    }
}

impl std::ops::Deref for Disks {
    type Target = dyn Folders;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// A disk that holds exactly these folders, each its own real folder, with this home.
#[cfg(test)]
pub(in crate::tower) struct FakeFolders(pub(in crate::tower) Vec<String>);

#[cfg(test)]
impl Folders for FakeFolders {
    fn exists(&self, folder: &str) -> bool {
        self.0.iter().any(|f| f == folder)
    }
    fn real(&self, folder: &str) -> String {
        Path::new(folder).components().collect::<std::path::PathBuf>().display().to_string()
    }
    fn shown(&self, at: &Path) -> String {
        shown_under(&["/home/him".into()], at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_under_his_home_reads_from_the_home_and_one_elsewhere_whole() {
        let disk = FakeFolders(Vec::new());
        assert_eq!(disk.shown(Path::new("/home/him")), "~");
        assert_eq!(disk.shown(Path::new("/home/him/work/shop")), "~/work/shop");
        assert_eq!(disk.shown(Path::new("/srv/shop")), "/srv/shop");
        assert_eq!(disk.real("/work//shop/"), "/work/shop");
    }
}
