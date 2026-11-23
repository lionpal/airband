use super::*;
use crate::ears::Got;

/// A folder of its own for one test, removed when it ends.
pub(crate) struct Folder(pub(crate) PathBuf);
impl Folder {
    pub(crate) fn new() -> Folder {
        let dir = std::env::temp_dir().join(format!("airband-takes-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        Folder(dir)
    }
    pub(crate) fn path(&self) -> String {
        self.0.display().to_string()
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A second of him speaking: loud enough to be heard as speech.
pub(crate) fn speech(seconds: usize) -> Vec<u8> {
    (0..16000 * seconds).flat_map(|i| ((((i as f32) * 0.07).sin() * 9000.0) as i16).to_le_bytes()).collect()
}

/// A take recorded with this sound, ended with what the listening made of it.
fn recorded(takes: &Takes, sound: &[u8], got: Got) -> (String, Listened) {
    let mut rec = takes.begin(Aim { re: Some("q1".into()), ..Aim::default() });
    for piece in sound.chunks(3200) {
        assert!(rec.keep(piece).is_none());
    }
    let id = rec.id.clone();
    (id, takes.ended(rec, got))
}

fn stage(takes: &Takes, id: &str) -> Option<Stage> {
    takes.get(id).map(|t| t.stage)
}
fn on_disk(dir: &Folder, id: &str) -> Option<usize> {
    std::fs::read(Path::new(&takes_folder(dir.path())).join(format!("{id}.pcm"))).ok().map(|b| b.len())
}

#[test]
fn a_take_goes_from_recording_to_heard_to_sent_and_is_deleted_only_when_his_computer_took_it() {
    let dir = Folder::new();
    let (takes, _) = Takes::open(&dir.path());
    let sound = speech(2);
    let mut rec = takes.begin(Aim::default());
    rec.keep(&sound[..32000]);
    // on the disk as it is captured, before it is heard
    assert_eq!((stage(&takes, &rec.id), on_disk(&dir, &rec.id)), (Some(Stage::Recording), Some(32000)));
    rec.keep(&sound[32000..]);
    let id = rec.id.clone();
    let heard = takes.ended(rec, Got::Words(Some(" ship it ".into())));
    assert!(matches!(heard, Listened::Said { ref text, take: Some(ref t) } if text == "ship it" && *t == id));
    // the words came back: the recording stays
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (Some(Stage::Heard { text: "ship it".into() }), Some(64000)));
    takes.sent(&id);
    assert_eq!(stage(&takes, &id), Some(Stage::Sent { text: "ship it".into() }));
    assert!(takes.kept(&id));
    // it can be played, as a WAV of all of it
    assert_eq!(takes.sound(&id).map(|w| w.len()), Some(44 + 64000));
    takes.taken(&id);
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (None, None));
}

#[test]
fn a_take_that_was_not_heard_keeps_all_its_sound_and_says_why() {
    let dir = Folder::new();
    let (takes, _) = Takes::open(&dir.path());
    let (id, heard) = recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
    assert!(matches!(heard, Listened::Unheard { ref take } if *take == id));
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (Some(Stage::Unheard { why: Unheard::Dropped }), Some(32000)));
    // speech in it and no words back: not heard either
    let (id, _) = recorded(&takes, &speech(1), Got::Words(None));
    assert_eq!(stage(&takes, &id), Some(Stage::Unheard { why: Unheard::NoWords }));
    // no speech and no words: nothing he said, and nothing kept
    let (id, heard) = recorded(&takes, &vec![0u8; 32000], Got::Words(Some("  ".into())));
    assert!(matches!(heard, Listened::Nothing));
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (None, None));
}

#[test]
fn trying_again_hears_what_was_not_heard_and_sends_what_was_never_listened_to_twice() {
    let dir = Folder::new();
    let (takes, _) = Takes::open(&dir.path());
    let (id, _) = recorded(&takes, &speech(1), Got::Failed(Unheard::Unreachable));
    assert_eq!(takes.again(&id, false), Again::Hear { aim: Aim { re: Some("q1".into()), ..Aim::default() } });
    assert_eq!(stage(&takes, &id), Some(Stage::Hearing { was: Unheard::Unreachable }));
    // a second tap while it is being heard does nothing
    assert_eq!(takes.again(&id, false), Again::Nothing);
    // what it is heard from is the recording, whole
    assert_eq!(takes.replay(&id).map(|p| p.len()), Some(32000));
    // heard again and failed again: unheard, by the new reason
    assert!(matches!(takes.heard_again(&id, Got::Failed(Unheard::TimedOut)), Listened::Unheard { .. }));
    assert_eq!(stage(&takes, &id), Some(Stage::Unheard { why: Unheard::TimedOut }));
    // nothing could listen this time: back as it was
    takes.again(&id, false);
    takes.back(&id);
    assert_eq!(stage(&takes, &id), Some(Stage::Unheard { why: Unheard::TimedOut }));
    // heard: its words go
    takes.again(&id, false);
    assert!(matches!(takes.heard_again(&id, Got::Words(Some("ship it".into()))), Listened::Said { .. }));
    // with no computer to take them, they wait: trying again sends the words, and listens to nothing
    takes.unsent(&id);
    assert_eq!(takes.again(&id, false), Again::Send { text: "ship it".into(), aim: Aim { re: Some("q1".into()), ..Aim::default() } });
    takes.sent(&id);
    // in an outbox, it is the outbox's to send again: nothing here
    assert_eq!(takes.again(&id, true), Again::Nothing);
    // an outbox that is gone (the computer was unpaired): the words go again
    assert_eq!(takes.again(&id, false), Again::Send { text: "ship it".into(), aim: Aim { re: Some("q1".into()), ..Aim::default() } });
    takes.discard(&id);
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (None, None));
}

#[test]
fn what_was_left_partway_when_the_app_closed_is_found_again() {
    let dir = Folder::new();
    let (recording, heard, sent, gone, hearing) = {
        let (takes, _) = Takes::open(&dir.path());
        // still recording when the app was killed: the file has what was captured
        let mut rec = takes.begin(Aim::default());
        rec.keep(&speech(1));
        let recording = rec.id.clone();
        std::mem::forget(rec);
        let (heard, _) = recorded(&takes, &speech(1), Got::Words(Some("heard".into())));
        let (sent, _) = recorded(&takes, &speech(1), Got::Words(Some("sent".into())));
        takes.sent(&sent);
        // its sound was deleted and its record was not yet
        let (gone, _) = recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
        std::fs::remove_file(Path::new(&takes_folder(dir.path())).join(format!("{gone}.pcm"))).unwrap();
        let (hearing, _) = recorded(&takes, &speech(1), Got::Failed(Unheard::Unreachable));
        takes.again(&hearing, false);
        (recording, heard, sent, gone, hearing)
    };
    // sound that belongs to no take
    std::fs::write(Path::new(&takes_folder(dir.path())).join("stray.pcm"), b"xx").unwrap();
    let (takes, damaged) = Takes::open(&dir.path());
    assert!(damaged.is_none());
    assert_eq!(stage(&takes, &recording), Some(Stage::Unheard { why: Unheard::Interrupted }));
    assert_eq!(on_disk(&dir, &recording), Some(32000));
    assert_eq!(stage(&takes, &heard), Some(Stage::Unsent { text: "heard".into() }));
    assert_eq!(stage(&takes, &sent), Some(Stage::Sent { text: "sent".into() }));
    assert_eq!(stage(&takes, &gone), None);
    assert_eq!(stage(&takes, &hearing), Some(Stage::Unheard { why: Unheard::Unreachable }));
    assert!(!Path::new(&takes_folder(dir.path())).join("stray.pcm").exists());
    // what was found is kept as found
    let (again, _) = Takes::open(&dir.path());
    assert_eq!(again.all().len(), 4);
}

#[test]
fn past_the_limit_the_oldest_goes_those_on_their_way_first_and_he_is_told_of_the_rest() {
    let told = Arc::new(Mutex::new(Vec::<String>::new()));
    let takes = Takes::default();
    let heard = told.clone();
    takes.told_by(Arc::new(move |t| heard.lock().unwrap().push(t)));
    let mut ids = Vec::new();
    for i in 0..KEEP.takes {
        let (id, _) = recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
        if i == 3 {
            takes.stage(&id, |t| t.stage = Stage::Sent { text: "on its way".into() });
        }
        ids.push(id);
    }
    // the one on its way goes first, and he need not be told: its words are still going
    recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
    assert!(stage(&takes, &ids[3]).is_none() && stage(&takes, &ids[0]).is_some());
    assert!(told.lock().unwrap().is_empty());
    // then the oldest, and he is told
    recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
    assert!(stage(&takes, &ids[0]).is_none());
    assert_eq!(*told.lock().unwrap(), ["Your oldest recording that was not sent was deleted to make room."]);
    assert_eq!(takes.all().len(), KEEP.takes);
    // a take still recording is never what goes
    let rec = takes.begin(Aim::default());
    takes.stage(&rec.id, |t| t.bytes = KEEP.bytes);
    recorded(&takes, &speech(1), Got::Failed(Unheard::Dropped));
    assert!(stage(&takes, &rec.id).is_some());
}

#[test]
fn only_his_computer_taking_it_deletes_the_recording() {
    use airband_core::radio::{Outcome, Rejected};
    let dir = Folder::new();
    let (takes, _) = Takes::open(&dir.path());
    let store = Store::open("", None);
    let host = crate::ears_stream::Tape::of(&[], false);
    let (id, _) = recorded(&takes, &speech(1), Got::Words(Some("ship it".into())));
    takes.sent(&id);
    store.change(|s| s.outbox.push(Outgoing::new(id.clone(), "ship it".into(), None, None, None, Vec::new(), Vec::new())));
    // refused: it stays, to send again
    settle(&store, &host, &takes, &id, Outcome::Rejected { why: Rejected::TooOld });
    assert!(on_disk(&dir, &id).is_some());
    // his computer has it
    settle(&store, &host, &takes, &id, Outcome::Taken);
    assert_eq!((stage(&takes, &id), on_disk(&dir, &id)), (None, None));
    // answered straight away is taken too
    let (id, _) = recorded(&takes, &speech(1), Got::Words(Some("again".into())));
    settle(&store, &host, &takes, &id, Outcome::Answered { reply: Reply::default() });
    assert_eq!(on_disk(&dir, &id), None);
}

#[test]
fn his_words_are_never_printed() {
    let t = Take { id: "a".into(), at: 0, aim: Aim::default(), stage: Stage::Sent { text: "secret plans".into() }, spoke: true, bytes: 0 };
    let shown = format!("{t:?}");
    assert!(!shown.contains("secret") && shown.contains("12 characters"), "{shown}");
}
