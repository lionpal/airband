//! Folders on this computer's disk, as the tower asks after them.
use std::path::Path;

pub(in crate::tower) trait Folders: Sync {
    /// Whether this folder is there now.
    fn exists(&self, folder: &str) -> bool;
}

/// This computer's own disk.
pub(in crate::tower) struct Disk;

impl Folders for Disk {
    fn exists(&self, folder: &str) -> bool {
        Path::new(folder).is_dir()
    }
}

/// A disk that holds exactly these folders.
#[cfg(test)]
pub(in crate::tower) struct FakeFolders(pub(in crate::tower) Vec<String>);

#[cfg(test)]
impl Folders for FakeFolders {
    fn exists(&self, folder: &str) -> bool {
        self.0.iter().any(|f| f == folder)
    }
}
