//! What the app and its notification extension both keep that is no secret, in the folder they share (an iPhone
//! App Group's container): each team's colour, and the sets of buttons a notification can carry. Keys and
//! pairing links stay where the host keeps secrets.
//!
//! Two processes write here. Each kind is changed under a lock on a file beside it, held across processes, so
//! one never writes over what the other has just kept.
use airband_io::kept::{Keeper, Loaded};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// How many sets of answer buttons are kept, newest first. A question older than these shows its reply box and
/// no answers.
pub const KEPT_ANSWER_SETS: usize = 32;

/// The shared folder.
#[derive(uniffi::Object)]
pub struct SharedFolder {
    dir: PathBuf,
}

/// What a read of where the colours were kept before found.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum OldRead {
    Found { text: String },
    Nothing,
    /// The read failed (the phone not unlocked since it started, or any other error): nothing is known, and
    /// nothing there is forgotten.
    Failed,
}

/// Where team colours were kept before this folder: the host's Keychain item.
#[uniffi::export(with_foreign)]
pub trait OldColours: Send + Sync {
    fn read(&self) -> OldRead;
    /// Remove it. True once it is gone.
    fn forget(&self) -> bool;
}

/// What came of moving the colours here.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ColoursMoved {
    /// Nothing was kept the old way: nothing to move.
    Nothing,
    /// They are here and the old item is gone.
    Moved,
    /// Not finished (a read, a write or the removal failed): tried again the next time.
    NotYet,
}

impl SharedFolder {
    fn at(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn colours_file(&self) -> PathBuf {
        self.at("team-colours.json")
    }
    fn locked(&self, name: &str) -> Option<airband_io::files::Lock> {
        airband_io::files::lock(&self.at(name)).map_err(|e| eprintln!("airband: the lock {name} in the shared folder cannot be taken: {e}")).ok()
    }
    fn answer_sets_now(&self) -> (AnswerSets, Keeper) {
        let (kept, keeper, loaded) = Keeper::open::<AnswerSets>(Some(self.at("answer-sets.json")));
        if let Some(trouble) = loaded.trouble() {
            eprintln!("airband: {trouble}");
        }
        (kept, keeper)
    }
    fn colours_now(&self) -> (HashMap<String, u8>, Keeper, Loaded) {
        let (colours, keeper, loaded) = Keeper::open::<HashMap<String, u8>>(Some(self.colours_file()));
        if let Some(trouble) = loaded.trouble() {
            eprintln!("airband: {trouble}");
        }
        (colours, keeper, loaded)
    }
}

#[uniffi::export]
impl SharedFolder {
    /// The folder at `dir`, which the host has made.
    #[uniffi::constructor]
    pub fn new(dir: String) -> Arc<SharedFolder> {
        Arc::new(SharedFolder { dir: PathBuf::from(dir) })
    }

    /// Each team's colour as last kept: none for a team not in it.
    pub fn colours(&self) -> HashMap<String, u8> {
        self.colours_now().0
    }

    /// Keep each team's colour as [`crate::DeskView::colours`] gives it. False when it was not kept.
    pub fn keep_colours(&self, colours: HashMap<String, u8>) -> bool {
        let Some(_held) = self.locked("team-colours.lock") else { return false };
        let (_, keeper, _) = self.colours_now();
        keeper.save(&colours).into_result().map_err(|why| eprintln!("airband: {why}")).is_ok()
    }

    /// Move the colours from where they were kept before, once. What is here already is newer and stays; the old
    /// item is forgotten only once the colours are kept here, and nothing of it is left behind.
    pub fn move_colours(&self, old: Arc<dyn OldColours>) -> ColoursMoved {
        let Some(_held) = self.locked("team-colours.lock") else { return ColoursMoved::NotYet };
        let text = match old.read() {
            OldRead::Nothing => return ColoursMoved::Nothing,
            OldRead::Failed => return ColoursMoved::NotYet,
            OldRead::Found { text } => text,
        };
        let (_, keeper, loaded) = self.colours_now();
        match loaded {
            // kept here since: the old copy is older
            Loaded::Read => {}
            // nothing here yet: the old copy is what there is. One that does not read as colours holds none to keep
            Loaded::Fresh => match serde_json::from_str::<HashMap<String, u8>>(&text) {
                Ok(colours) => {
                    if let Err(why) = keeper.save(&colours).into_result() {
                        eprintln!("airband: {why}");
                        return ColoursMoved::NotYet;
                    }
                }
                Err(e) => eprintln!("airband: the team colours kept before do not read ({e}); they are given again"),
            },
            // what is here cannot be read: nothing is moved over it
            Loaded::SetAside { .. } | Loaded::LeftInPlace { .. } => return ColoursMoved::NotYet,
        }
        if old.forget() { ColoursMoved::Moved } else { ColoursMoved::NotYet }
    }

    /// Add a question's answers to the sets of buttons kept, newest first, under the lock, which is let go
    /// before this returns: the host then registers what it gives with its system, holding nothing, and once
    /// its system has them reads [`SharedFolder::answer_sets_rev`]. A different number means another process
    /// kept a change meanwhile and may have registered before this one: the host registers the sets again, as
    /// they are now. With no answers, the sets as they are (the app registers them with its fixed ones when it
    /// starts). None when the lock cannot be taken or the sets cannot be kept: nothing is registered then.
    pub fn answer_sets(&self, adding: Option<Vec<String>>) -> Option<AnswerSets> {
        let _held = self.locked("answer-sets.lock")?;
        let (kept, keeper) = self.answer_sets_now();
        match adding {
            Some(options) => {
                let sets = AnswerSets { rev: kept.rev + 1, sets: newest_first(kept.sets, options) };
                keeper.save(&sets).into_result().map_err(|why| eprintln!("airband: {why}")).ok()?;
                Some(sets)
            }
            None => Some(kept),
        }
    }

    /// The number of the change last kept to the answer sets. None when they cannot be read.
    pub fn answer_sets_rev(&self) -> Option<u64> {
        let _held = self.locked("answer-sets.lock")?;
        Some(self.answer_sets_now().0.rev)
    }
}

/// `options` first, the rest after in the order they were kept, the same set never twice, and no more than
/// [`KEPT_ANSWER_SETS`].
fn newest_first(kept: Vec<Vec<String>>, options: Vec<String>) -> Vec<Vec<String>> {
    let mut sets: Vec<Vec<String>> = kept.into_iter().filter(|s| *s != options).collect();
    sets.insert(0, options);
    sets.truncate(KEPT_ANSWER_SETS);
    sets
}

/// The sets of answer buttons, newest first, and the number of the change that kept them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct AnswerSets {
    pub rev: u64,
    pub sets: Vec<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

/// The folder, for a test.
    fn folder() -> (Arc<SharedFolder>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("airband-shared-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        (SharedFolder::new(dir.display().to_string()), dir)
    }
    
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// A stand-in for the Keychain item: what it holds, whether reading it fails, whether removing it fails.
    struct Item {
        held: Mutex<Option<String>>,
        read_fails: bool,
        forget_fails: bool,
    }
    impl Item {
        fn new(held: Option<&str>) -> Arc<Item> {
            Arc::new(Item { held: Mutex::new(held.map(str::to_string)), read_fails: false, forget_fails: false })
        }
    }
    impl OldColours for Item {
        fn read(&self) -> OldRead {
            if self.read_fails {
                return OldRead::Failed;
            }
            match self.held.lock().unwrap().clone() {
                Some(text) => OldRead::Found { text },
                None => OldRead::Nothing,
            }
        }
        fn forget(&self) -> bool {
            if self.forget_fails {
                return false;
            }
            self.held.lock().unwrap().take();
            true
        }
    }

    #[test]
    fn the_colours_move_once_from_the_keychain_and_nothing_is_left_behind() {
        let (shared, dir) = folder();
        let item = Item::new(Some(r#"{"e2e/shop":1,"e2e/site":4}"#));
        assert_eq!(shared.move_colours(item.clone()), ColoursMoved::Moved);
        assert_eq!(shared.colours(), HashMap::from([("e2e/shop".to_string(), 1), ("e2e/site".to_string(), 4)]));
        // the old item is gone, and the folder holds the colours and their lock alone
        assert_eq!(*item.held.lock().unwrap(), None);
        assert_eq!(entries(&dir), ["team-colours.json", "team-colours.lock"]);
        // the next time there is nothing to move, and what is here stays
        assert_eq!(shared.move_colours(item.clone()), ColoursMoved::Nothing);
        assert_eq!(shared.colours().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn colours_kept_here_since_are_newer_than_the_old_item_which_is_still_forgotten() {
        let (shared, dir) = folder();
        assert!(shared.keep_colours(HashMap::from([("e2e/shop".to_string(), 2)])));
        let item = Item::new(Some(r#"{"e2e/shop":1}"#));
        assert_eq!(shared.move_colours(item.clone()), ColoursMoved::Moved);
        assert_eq!(shared.colours(), HashMap::from([("e2e/shop".to_string(), 2)]));
        assert_eq!(*item.held.lock().unwrap(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_move_that_cannot_finish_forgets_nothing_and_is_tried_again() {
        let (shared, dir) = folder();
        // the Keychain not readable yet: nothing done
        let locked = Arc::new(Item { held: Mutex::new(Some(r#"{"e2e/shop":1}"#.into())), read_fails: true, forget_fails: false });
        assert_eq!(shared.move_colours(locked.clone()), ColoursMoved::NotYet);
        assert!(locked.held.lock().unwrap().is_some() && shared.colours().is_empty());
        // kept here, but the old item would not go: tried again, and then it goes
        let stuck = Arc::new(Item { held: Mutex::new(Some(r#"{"e2e/shop":1}"#.into())), read_fails: false, forget_fails: true });
        assert_eq!(shared.move_colours(stuck.clone()), ColoursMoved::NotYet);
        assert_eq!(shared.colours(), HashMap::from([("e2e/shop".to_string(), 1)]));
        let later = Item::new(stuck.held.lock().unwrap().as_deref());
        assert_eq!(shared.move_colours(later.clone()), ColoursMoved::Moved);
        assert_eq!(*later.held.lock().unwrap(), None);
        // an old item that does not read as colours holds none: it is forgotten all the same
        let (other, other_dir) = folder();
        let damaged = Item::new(Some("not colours"));
        assert_eq!(other.move_colours(damaged.clone()), ColoursMoved::Moved);
        assert_eq!(*damaged.held.lock().unwrap(), None);
        assert!(other.colours().is_empty());
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&other_dir).ok();
    }

    #[test]
    fn answer_sets_are_kept_newest_first_each_once_and_only_so_many() {
        let set = |n: usize| vec![format!("yes {n}"), "no".to_string()];
        let (shared, dir) = folder();
        for n in 0..KEPT_ANSWER_SETS + 5 {
            shared.answer_sets(Some(set(n))).unwrap();
        }
        let sets = shared.answer_sets(None).unwrap().sets;
        assert_eq!(sets.len(), KEPT_ANSWER_SETS);
        assert_eq!(sets[0], set(KEPT_ANSWER_SETS + 4));
        assert_eq!(sets[KEPT_ANSWER_SETS - 1], set(5));
        // one asked again comes first, and is not kept twice
        let again = shared.answer_sets(Some(set(10))).unwrap().sets;
        assert_eq!(again[0], set(10));
        assert_eq!(again.iter().filter(|s| **s == set(10)).count(), 1);
        assert_eq!(again.len(), KEPT_ANSWER_SETS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_change_kept_while_another_process_registers_is_seen_by_its_number_and_none_is_lost() {
        let (app, dir) = folder();
        let extension = SharedFolder::new(dir.display().to_string());
        // the app keeps its change and lets the lock go before it registers
        let mine = app.answer_sets(Some(vec!["a".into()])).unwrap();
        // so the extension is not held up meanwhile: it keeps its own, on top of the app's
        let theirs = extension.answer_sets(Some(vec!["b".into()])).unwrap();
        assert_eq!(theirs.sets, vec![vec!["b".to_string()], vec!["a".to_string()]]);
        assert!(theirs.rev > mine.rev);
        // the app's system has its sets: the number has moved on, so it registers again, as they are now
        assert_ne!(app.answer_sets_rev(), Some(mine.rev));
        assert_eq!(app.answer_sets(None).unwrap(), theirs);
        // the extension, registering last, finds its own number and is done
        assert_eq!(extension.answer_sets_rev(), Some(theirs.rev));
        // and nothing holds the lock after: another change is kept at once
        assert!(app.answer_sets(Some(vec!["c".into()])).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
