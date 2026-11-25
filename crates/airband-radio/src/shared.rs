//! What the app and its notification extension both keep that is no secret, in the folder they share (an iPhone
//! App Group's container): each team's colour, the sets of buttons a notification can carry, what became
//! of the latest notifications (for the connection record), and what the tile shows ([`crate::tile`]). Keys and
//! pairing links stay where the host keeps secrets.
//!
//! Two processes write here. Each kind is changed under a lock on a file beside it, held across processes, so
//! one never writes over what the other has just kept.
use airband_io::kept::Keeper;
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

impl SharedFolder {
    fn at(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn colours_file(&self) -> PathBuf {
        self.at("team-colours.json")
    }
    fn tile_file(&self) -> PathBuf {
        self.at("tile.json")
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
    fn notices_now(&self) -> (Vec<NoticeShown>, Keeper) {
        let (kept, keeper, loaded) = Keeper::open::<Vec<NoticeShown>>(Some(self.at("notices.json")));
        if let Some(trouble) = loaded.trouble() {
            eprintln!("airband: {trouble}");
        }
        (kept, keeper)
    }
    fn tile_now(&self) -> (crate::tile::TileKept, Keeper) {
        let (kept, keeper, loaded) = Keeper::open::<crate::tile::TileKept>(Some(self.tile_file()));
        if let Some(trouble) = loaded.trouble() {
            eprintln!("airband: {trouble}");
        }
        (kept, keeper)
    }
    /// Changes what the tile holds, under its lock. Whether what it shows changed; false when it could not be
    /// kept.
    fn change_tile(&self, change: impl FnOnce(&crate::tile::TileKept) -> crate::tile::TileKept) -> bool {
        let Some(_held) = self.locked("tile.lock") else { return false };
        let (kept, keeper) = self.tile_now();
        let now = change(&kept);
        if now == kept {
            return false;
        }
        keeper.save(&now).into_result().map_err(|why| eprintln!("airband: {why}")).is_ok() && now.shows != kept.shows
    }
    fn colours_now(&self) -> (HashMap<String, u8>, Keeper) {
        let (colours, keeper, loaded) = Keeper::open::<HashMap<String, u8>>(Some(self.colours_file()));
        if let Some(trouble) = loaded.trouble() {
            eprintln!("airband: {trouble}");
        }
        (colours, keeper)
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
        let (_, keeper) = self.colours_now();
        keeper.save(&colours).into_result().map_err(|why| eprintln!("airband: {why}")).is_ok()
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

    /// Keep what became of a notification, newest last, no more than [`KEPT_NOTICES`]. False when it was not kept.
    pub fn keep_notice(&self, shown: NoticeShown) -> bool {
        let Some(_held) = self.locked("notices.lock") else { return false };
        let (mut kept, keeper) = self.notices_now();
        kept.push(shown);
        let extra = kept.len().saturating_sub(KEPT_NOTICES);
        kept.drain(..extra);
        keeper.save(&kept).into_result().map_err(|why| eprintln!("airband: {why}")).is_ok()
    }

    /// What became of the latest notifications, oldest first.
    pub fn notices(&self) -> Vec<NoticeShown> {
        self.notices_now().0
    }

    /// The same, as lines for the connection record.
    pub fn notice_lines(&self) -> Vec<String> {
        self.notices().iter().map(NoticeShown::line).collect()
    }

    /// Keep what the app has from the chat for the tile ([`crate::Desk::tile`]), in place of what was kept, but
    /// for lines a notification has taken down since. True when what the tile shows changed: the host asks
    /// for it to be drawn again.
    pub fn keep_tile(&self, facts: crate::TileFacts) -> bool {
        self.change_tile(|kept| kept.with_chat(facts))
    }

    /// What a notification just opened ([`crate::open_notice`]) changes on the tile: a question or news put on,
    /// or a line that stopped being news taken off. True when what the tile shows changed.
    pub fn tile_heard(&self, opening: crate::NoticeOpening) -> bool {
        self.change_tile(|kept| kept.heard(&opening))
    }

    /// What the tile shows, as last kept.
    pub fn tile(&self) -> crate::TileView {
        self.tile_now().0.shows
    }

    /// Where the tile's file is: the host gives it the protection it needs, and the tile reads it.
    pub fn tile_path(&self) -> String {
        self.tile_file().display().to_string()
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

/// How many notifications' outcomes are kept.
pub const KEPT_NOTICES: usize = 20;

/// What the phone did with one notification: when, and what it showed. No words of it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct NoticeShown {
    /// Milliseconds since 1970.
    pub at: u64,
    pub how: NoticeHow,
}

/// How a notification was shown.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "kebab-case", tag = "how")]
pub enum NoticeHow {
    /// Opened and shown with its sender, its words and these buttons. `message` is the line's id.
    Opened { message: String, buttons: crate::NoticeButtonSet, permission: bool },
    /// The answers it offered could not be registered as buttons: shown with its reply box alone.
    AnswersNotRegistered { message: String },
    /// Not opened: shown in the push service's few words, for this reason.
    Plain { why: crate::NoticeTrouble },
    /// The system's time for opening it ran out first: shown in the push service's few words.
    OutOfTime,
    /// It took down what was shown for `message`, which was no longer news, for this reason.
    Withdrawn { message: String, why: airband_core::radio::Withdrawn },
}

impl NoticeShown {
    fn line(&self) -> String {
        let when = airband_core::clock::rfc3339(self.at);
        match &self.how {
            NoticeHow::Opened { message, buttons, permission } => {
                let what = if *permission { "request for leave" } else { "notification" };
                let with = match buttons {
                    crate::NoticeButtonSet::Permit => "Allow and Deny",
                    crate::NoticeButtonSet::PermitAfterReading => "Deny only (too long to allow unread)",
                    crate::NoticeButtonSet::Answers => "its answers",
                    crate::NoticeButtonSet::Reply => "a reply box",
                };
                format!("{when} {what} {message} shown with {with}")
            }
            NoticeHow::AnswersNotRegistered { message } => format!("{when} notification {message} shown with a reply box: its answers could not be made buttons"),
            NoticeHow::Plain { why } => format!("{when} a notification shown without its words: {why}"),
            NoticeHow::OutOfTime => format!("{when} a notification shown without its words: the phone ran out of time to open it"),
            NoticeHow::Withdrawn { message, why } => format!("{when} notification {message} taken down: {}", withdrawn(*why)),
        }
    }
}

/// The sets of answer buttons, newest first, and the number of the change that kept them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct AnswerSets {
    pub rev: u64,
    pub sets: Vec<Vec<String>>,
}

/// Why a notification was taken down, as the connection record says it.
fn withdrawn(why: airband_core::radio::Withdrawn) -> &'static str {
    use airband_core::radio::{NotRead, Withdrawn};
    match why {
        Withdrawn::Heard => "he heard it read aloud",
        Withdrawn::Read => "he had it on screen on another device",
        Withdrawn::NotRead { reason } => match reason {
            NotRead::Routine => "nothing in it needed him",
            NotRead::Superseded => "its agent said something newer",
            NotRead::Answered => "he answered its agent, or it went back to work",
            NotRead::Dismissed => "he let the question go",
            NotRead::AgentGone => "its agent stopped running",
            NotRead::Stale => "it was old news when he turned reading aloud on",
            NotRead::PermissionEnded => "the request was over",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder, for a test.
    fn folder() -> (Arc<SharedFolder>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("airband-shared-{}", airband_core::encode::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        (SharedFolder::new(dir.display().to_string()), dir)
    }

    #[test]
    fn what_became_of_each_notification_is_kept_newest_last_and_only_so_many() {
        let (shared, dir) = folder();
        assert!(shared.notices().is_empty());
        let plain = NoticeShown { at: 1_000, how: NoticeHow::Plain { why: crate::NoticeTrouble::KeysUnreadable { status: -25308 } } };
        for n in 0..KEPT_NOTICES + 3 {
            assert!(shared.keep_notice(NoticeShown { at: n as u64, how: NoticeHow::Opened { message: format!("m{n}"), buttons: crate::NoticeButtonSet::Permit, permission: true } }));
        }
        assert!(shared.keep_notice(plain.clone()));
        let kept = shared.notices();
        assert_eq!(kept.len(), KEPT_NOTICES);
        assert_eq!(kept.last(), Some(&plain));
        let lines = shared.notice_lines();
        assert!(lines.last().unwrap().ends_with("shown without its words: the pairings could not be read (error -25308)"), "{lines:?}");
        assert!(lines[0].ends_with("request for leave m4 shown with Allow and Deny"), "{lines:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_colours_kept_are_the_colours_read() {
        let (shared, dir) = folder();
        assert!(shared.colours().is_empty());
        let colours = HashMap::from([("e2e/shop".to_string(), 1), ("e2e/site".to_string(), 4)]);
        assert!(shared.keep_colours(colours.clone()));
        assert_eq!(shared.colours(), colours);
        std::fs::remove_dir_all(&dir).ok();
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

