//! A JSON file of another tool's (its settings, where airband's hooks go). It is his, and may hold far more than
//! airband's part, so it is never set aside, never taken for empty when it cannot be read, and never written over
//! then: only a file that is not there starts empty. It can be written only once it has been read, which the
//! type makes sure of. A write keeps the permissions it had; a new one can be read only by its owner.
use crate::files::write_keeping;
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

pub struct Foreign<T> {
    path: PathBuf,
    /// What it holds, to change.
    pub value: T,
}

impl<T: Serialize + DeserializeOwned + Default> Foreign<T> {
    /// Read it. Err, saying why, when it is there and cannot be read or is not JSON of that shape: then there is
    /// nothing to write it with.
    pub fn open(path: &Path) -> Result<Foreign<T>, String> {
        let value = match std::fs::read(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
            Err(e) => return Err(format!("{} cannot be read ({e}), so nothing was changed", path.display())),
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{} is not valid JSON; fix it first ({e})", path.display()))?,
        };
        Ok(Foreign { path: path.to_path_buf(), value })
    }
    /// Write it back whole, laid out for a person to read, as tools' own files are.
    pub fn write(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.value).map_err(|e| e.to_string())?;
        write_keeping(&self.path, format!("{text}\n")).map_err(|e| format!("{} could not be written ({e})", self.path.display()))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::folder;
    use serde_json::{Value, json};

    #[test]
    fn only_a_file_that_is_not_there_starts_empty_and_an_unreadable_one_cannot_be_written() {
        let dir = folder();
        let none = Foreign::<Value>::open(&dir.join("settings.json")).unwrap();
        assert_eq!(none.value, Value::Null);
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ his, half").unwrap();
        assert!(Foreign::<Value>::open(&bad).is_err());
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ his, half");
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("unreadable")));
        let mut good = Foreign::<Value>::open(&dir.join("good.json")).unwrap();
        good.value = json!({"a": 1});
        good.write().unwrap();
        assert_eq!(Foreign::<Value>::open(&dir.join("good.json")).unwrap().value, json!({"a": 1}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
