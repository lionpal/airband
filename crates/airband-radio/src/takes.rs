//! What he says, kept from its first sample to its last until his computer has it.
//!
//! Each time he talks is a [`Take`]: its sound, written to the phone as it is captured, and how far it has got
//! ([`Stage`]). The sound is written whatever the listening does: a line to the listening service that drops
//! partway does not stop the recording, and the take ends unheard with every sample kept. It is deleted for
//! good on one fact only: his computer took the message made from it (the outbox says so, [`Takes::taken`]), or
//! he let it go ([`Takes::discard`]). Never on a timer, and not when the words come back.
//!
//! ```text
//!  Recording ──▶ Heard ──▶ Sent ──▶ his computer took it: deleted
//!      │           └─────▶ Unsent ──(Try again)──▶ Sent
//!      └───────▶ Unheard ──(Try again)──▶ Hearing ──▶ Heard, or Unheard
//! ```
//!
//! Words that are in an outbox are the outbox's to send again: listening never runs twice for them. A take left
//! partway when the app closed is found when it opens again ([`Takes::open`]) and shown once more. At most
//! [`KEEP`] is kept; past that the oldest goes, and he is told when its words had not gone.
//!
//! The sound leaves the phone only for the listening service he chose. It is never written to a log, and is kept
//! in a folder of the app's own ([`takes_folder`]) that the host protects and leaves out of backups.
use super::*;
use airband_core::audio::{Endpointer, Heard, samples};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

/// How much of what he said the phone keeps at most: this many takes, and this much sound in all (about half
/// an hour). A take is at most four minutes, so the next one always has room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Keep {
    pub(crate) takes: usize,
    pub(crate) bytes: u64,
}
pub(crate) const KEEP: Keep = Keep { takes: 20, bytes: 64 * 1024 * 1024 };

/// The folder the sound of what he says is kept in, under the app's own folder: the host makes it its
/// strongest protection that still lets a take be written while the phone is locked (on a call), and leaves it
/// out of backups.
#[uniffi::export]
pub fn takes_folder(data_dir: String) -> String {
    Path::new(&data_dir).join("takes").display().to_string()
}

/// Why what he said was not heard.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Unheard {
    /// The listening service could not be reached.
    Unreachable,
    /// The service said why not, in its own words (it would not take his key, it refused the recording).
    Service { why: String },
    /// The line to it went partway, and could not be made again.
    Dropped,
    /// It did not answer in time.
    TimedOut,
    /// It heard him speak and gave back no words.
    NoWords,
    /// The model on the phone failed, in a sentence naming it.
    Model { why: String },
    /// The app closed before it was heard.
    Interrupted,
}

impl Unheard {
    /// In words for him.
    pub(crate) fn told(&self) -> String {
        match self {
            Unheard::Unreachable => "Couldn\u{2019}t reach the listening service.".into(),
            Unheard::Service { why } | Unheard::Model { why } => why.clone(),
            Unheard::Dropped => "The line to the listening service dropped.".into(),
            Unheard::TimedOut => "The listening service took too long to answer.".into(),
            Unheard::NoWords => "No words came back.".into(),
            Unheard::Interrupted => "The app closed before it was heard.".into(),
        }
    }
}

/// How far one take has got.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Stage {
    /// The microphone is open and the sound is going to the phone.
    Recording,
    /// Being heard again from the recording, after it was not heard (`was`).
    Hearing { was: Unheard },
    /// Heard, and on its way into an outbox.
    Heard { text: String },
    /// Heard, and not put in any outbox: there was no computer to send it to.
    Unsent { text: String },
    /// In the outbox under the take's id: the outbox sends it, and sends it again.
    Sent { text: String },
    /// Not heard, and why. The recording is whole.
    Unheard { why: Unheard },
}

impl std::fmt::Debug for Stage {
    /// His words are never printed: only how many there are.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let words = |name: &str, text: &str, f: &mut std::fmt::Formatter<'_>| write!(f, "{name}({} characters)", text.chars().count());
        match self {
            Stage::Recording => write!(f, "Recording"),
            Stage::Hearing { was } => write!(f, "Hearing({was:?})"),
            Stage::Heard { text } => words("Heard", text, f),
            Stage::Unsent { text } => words("Unsent", text, f),
            Stage::Sent { text } => words("Sent", text, f),
            Stage::Unheard { why } => write!(f, "Unheard({why:?})"),
        }
    }
}

/// Where his words go, as he aimed them when he spoke: the message he answered, the room and the agent he was
/// looking at. Ids only.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Aim {
    pub(crate) re: Option<String>,
    pub(crate) room: Option<String>,
    pub(crate) agent: Option<String>,
}

/// One time he talked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Take {
    /// Also the id of the message made from it, in the outbox and in the chat.
    pub(crate) id: String,
    /// When he began, in milliseconds since 1970.
    pub(crate) at: u64,
    pub(crate) aim: Aim,
    pub(crate) stage: Stage,
    /// He was heard speaking in it (by [`Endpointer`]): a take with no speech and no words is nothing he said.
    pub(crate) spoke: bool,
    /// How much sound is kept for it. Read from its file when the app opens.
    #[serde(skip)]
    pub(crate) bytes: u64,
}

/// What `takes.json` holds.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct List {
    takes: Vec<Take>,
}

impl crate::kept::Rejoin for List {
    /// What changed while the file could not be read is newer than what it holds.
    fn rejoin(&mut self, since: List) {
        for t in since.takes {
            match self.takes.iter_mut().find(|have| have.id == t.id) {
                Some(have) => *have = t,
                None => self.takes.push(t),
            }
        }
        self.takes.sort_by_key(|t| t.at);
    }
}

/// How he is told something about what he said, by the host.
pub(crate) type Tell = Arc<dyn Fn(String) + Send + Sync>;

/// Every take on this device: one for the desk and all its radios.
#[derive(Default)]
pub(crate) struct Takes {
    /// Where the sound is kept. Nothing keeps no sound (a test, a terminal, where nothing is recorded).
    folder: Option<PathBuf>,
    list: Mutex<List>,
    keeper: airband_io::kept::Keeper,
    tell: Mutex<Option<Tell>>,
}

/// What trying a take again comes to.
#[derive(Debug, PartialEq)]
pub(crate) enum Again {
    /// Hear it again from its recording, then send it as aimed.
    Hear { aim: Aim },
    /// Its words were heard: send them, as aimed.
    Send { text: String, aim: Aim },
    /// Nothing to do: it is being heard or sent already, or it is gone.
    Nothing,
}

impl Takes {
    /// The takes kept in the app's folder `data_dir`: what was left partway when the app last closed is found
    /// and made something he can act on. Returns what he is to be told when the list could not be read.
    pub(crate) fn open(data_dir: &str) -> (Takes, Option<String>) {
        let folder = (!data_dir.is_empty()).then(|| PathBuf::from(takes_folder(data_dir.into())));
        let (mut list, keeper, damaged) = crate::kept::open::<List>(crate::kept::file(data_dir, "takes.json"), "What you said that has not gone yet");
        if let Some(folder) = &folder {
            airband_io::files::private_folder(folder).ok();
            // with the list unread, no sound is taken for having no take
            recover(&mut list, folder, damaged.is_none());
        }
        let takes = Takes { folder, list: Mutex::new(List::default()), keeper, tell: Mutex::new(None) };
        takes.change(|l| *l = list);
        (takes, damaged)
    }

    /// Who is told, from now on.
    pub(crate) fn told_by(&self, tell: Tell) {
        *self.tell.lock().unwrap_or_else(|e| e.into_inner()) = Some(tell);
    }
    fn say(&self, text: String) {
        let tell = self.tell.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(tell) = tell {
            tell(text);
        }
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        self.folder.as_ref().map(|f| f.join(format!("{id}.pcm")))
    }

    fn change<T>(&self, f: impl FnOnce(&mut List) -> T) -> T {
        let (out, told) = {
            let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
            crate::kept::again(&self.keeper, &mut *list);
            let out = f(&mut list);
            (out, crate::kept::told(self.keeper.save(&*list), "What you said is lost if the app closes."))
        };
        if let Some(told) = told {
            self.say(told);
        }
        out
    }

    /// Move one take on, when it is there.
    fn stage(&self, id: &str, f: impl FnOnce(&mut Take)) {
        self.change(|l| {
            if let Some(t) = l.takes.iter_mut().find(|t| t.id == id) {
                f(t);
            }
        });
    }

    /// Every take, oldest first.
    pub(crate) fn all(&self) -> Vec<Take> {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).takes.clone()
    }
    fn get(&self, id: &str) -> Option<Take> {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).takes.iter().find(|t| t.id == id).cloned()
    }

    /// Is a recording kept for this message?
    pub(crate) fn kept(&self, id: &str) -> bool {
        self.folder.is_some() && self.get(id).is_some()
    }

    /// He has started talking: a new take, its sound written from the first sample. To make room the oldest
    /// takes go first, those whose words are in an outbox before the rest; he is told of any whose words had not.
    pub(crate) fn begin(&self, aim: Aim) -> Recorder {
        let id = random_hex(12);
        let gone = self.change(|l| {
            let gone = make_room(l);
            l.takes.push(Take { id: id.clone(), at: now_ms(), aim, stage: Stage::Recording, spoke: false, bytes: 0 });
            gone
        });
        for t in &gone {
            if let Some(path) = self.path(&t.id) {
                airband_io::files::remove(&path).ok();
            }
        }
        match gone.iter().filter(|t| !matches!(t.stage, Stage::Sent { .. })).count() {
            0 => {}
            1 => self.say("Your oldest recording that was not sent was deleted to make room.".into()),
            n => self.say(format!("Your {n} oldest recordings that were not sent were deleted to make room.")),
        }
        // the record first, then its file: a file is never without one
        let file = self.path(&id).map(|p| airband_io::files::appending(&p));
        let file = match file {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => {
                self.say(crate::kept::not_kept(&e.to_string()));
                None
            }
            None => None,
        };
        Recorder { id, file, end: Endpointer::new(crate::ears::RATE), spoke: false, finished: false, ended: false, bytes: 0 }
    }

    /// The microphone has closed on a take, and this is what the listening made of it.
    pub(crate) fn ended(&self, rec: Recorder, got: crate::ears::Got) -> Listened {
        let Recorder { id, spoke, bytes, .. } = rec;
        self.stage(&id, |t| (t.spoke, t.bytes) = (spoke, bytes));
        self.settle(&id, got, spoke)
    }

    /// Where a take that was heard, or not, goes: words, to be sent; no words with speech in it, unheard; no
    /// words and no speech, nothing he said, and deleted.
    fn settle(&self, id: &str, got: crate::ears::Got, spoke: bool) -> Listened {
        use crate::ears::Got;
        match got {
            Got::Words(Some(text)) if !text.trim().is_empty() => {
                let text = text.trim().to_string();
                self.stage(id, |t| t.stage = Stage::Heard { text: text.clone() });
                Listened::Said { text, take: Some(id.into()) }
            }
            Got::Words(_) if spoke => {
                self.stage(id, |t| t.stage = Stage::Unheard { why: Unheard::NoWords });
                Listened::Unheard { take: id.into() }
            }
            Got::Words(_) => {
                self.delete(id);
                Listened::Nothing
            }
            Got::Failed(why) => {
                self.stage(id, |t| t.stage = Stage::Unheard { why });
                Listened::Unheard { take: id.into() }
            }
        }
    }

    /// Its words are going into an outbox, under its id.
    pub(crate) fn sent(&self, id: &str) {
        self.stage(id, |t| {
            if let Stage::Heard { text } | Stage::Unsent { text } | Stage::Sent { text } = &t.stage {
                t.stage = Stage::Sent { text: text.clone() };
            }
        });
    }
    /// Its words were heard and could not be put in any outbox.
    pub(crate) fn unsent(&self, id: &str) {
        self.stage(id, |t| {
            if let Stage::Heard { text } | Stage::Sent { text } = &t.stage {
                t.stage = Stage::Unsent { text: text.clone() };
            }
        });
    }
    /// His computer took the message made from it: the one fact on which its sound is deleted.
    pub(crate) fn taken(&self, id: &str) {
        self.delete(id);
    }
    /// He let it go.
    pub(crate) fn discard(&self, id: &str) {
        self.delete(id);
    }
    /// Gone for good: the sound first, then the record, so a record may outlive its sound and never the
    /// other way round.
    fn delete(&self, id: &str) {
        if self.get(id).is_none() {
            return;
        }
        if let Some(path) = self.path(id) {
            airband_io::files::remove(&path).ok();
        }
        self.change(|l| l.takes.retain(|t| t.id != id));
    }

    /// He asked for it to be tried again. `waiting`: an outbox has its words now.
    pub(crate) fn again(&self, id: &str, waiting: bool) -> Again {
        self.change(|l| {
            let Some(t) = l.takes.iter_mut().find(|t| t.id == id) else { return Again::Nothing };
            match &t.stage {
                Stage::Unheard { why } => {
                    t.stage = Stage::Hearing { was: why.clone() };
                    Again::Hear { aim: t.aim.clone() }
                }
                Stage::Unsent { text } => Again::Send { text: text.clone(), aim: t.aim.clone() },
                // its outbox was let go of (he unpaired that computer): it goes again as heard
                Stage::Sent { text } if !waiting => Again::Send { text: text.clone(), aim: t.aim.clone() },
                Stage::Recording | Stage::Hearing { .. } | Stage::Heard { .. } | Stage::Sent { .. } => Again::Nothing,
            }
        })
    }

    /// The recording of a take being heard again; nothing when it cannot be read, and it is back as it was.
    pub(crate) fn replay(&self, id: &str) -> Option<Vec<u8>> {
        let sound = self.path(id).and_then(|p| airband_io::files::read_bytes(&p).ok().flatten());
        if sound.is_none() {
            self.back(id);
        }
        sound
    }
    /// A take that was to be heard again and could not be: unheard as before.
    pub(crate) fn back(&self, id: &str) {
        self.stage(id, |t| {
            if let Stage::Hearing { was } = &t.stage {
                t.stage = Stage::Unheard { why: was.clone() };
            }
        });
    }
    /// What hearing it again made of it.
    pub(crate) fn heard_again(&self, id: &str, got: crate::ears::Got) -> Listened {
        let Some(spoke) = self.get(id).map(|t| t.spoke) else { return Listened::Nothing };
        self.settle(id, got, spoke)
    }

    /// The recording, as a WAV file to play.
    pub(crate) fn sound(&self, id: &str) -> Option<Vec<u8>> {
        self.get(id)?;
        let pcm = self.path(id).and_then(|p| airband_io::files::read_bytes(&p).ok().flatten())?;
        let mut wav = crate::hears::wav_header(crate::ears::RATE, Some(pcm.len() as u32));
        wav.extend_from_slice(&pcm);
        Some(wav)
    }

    /// The microphone, with each piece of it read written to this take as well.
    pub(crate) fn tape<'a>(&'a self, host: &'a dyn RadioHost, aim: Aim) -> Tape<'a> {
        Tape { host, rec: Mutex::new(self.begin(aim)), takes: self }
    }
}

/// The oldest takes, taken off until a new one fits in [`KEEP`]: those whose words are in an outbox first. One
/// still recording or being heard is never taken.
fn make_room(l: &mut List) -> Vec<Take> {
    let mut gone = Vec::new();
    while l.takes.len() >= KEEP.takes || l.takes.iter().map(|t| t.bytes).sum::<u64>() >= KEEP.bytes {
        let sent = l.takes.iter().position(|t| matches!(t.stage, Stage::Sent { .. }));
        let idle = || l.takes.iter().position(|t| !matches!(t.stage, Stage::Recording | Stage::Hearing { .. }));
        let Some(i) = sent.or_else(idle) else { break };
        gone.push(l.takes.remove(i));
    }
    gone
}

/// What was left partway when the app last closed: a take still recording or being heard is unheard, one heard
/// and not yet in an outbox is unsent. A take whose sound is gone was deleted and its record not yet: it goes.
/// Sound with no take is from a take that never was: it goes, when the list was read (`read`). A folder that
/// cannot be read is left as it is.
fn recover(list: &mut List, folder: &Path, read: bool) {
    let path = |id: &str| folder.join(format!("{id}.pcm"));
    list.takes.retain_mut(|t| {
        match airband_io::files::metadata(&path(&t.id)) {
            Ok(m) => t.bytes = m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => {}
        }
        t.stage = match std::mem::replace(&mut t.stage, Stage::Recording) {
            Stage::Recording => Stage::Unheard { why: Unheard::Interrupted },
            Stage::Hearing { was } => Stage::Unheard { why: was },
            Stage::Heard { text } => Stage::Unsent { text },
            other => other,
        };
        true
    });
    if !read {
        return;
    }
    let Ok(files) = airband_io::files::entries(folder) else { return };
    for file in files.iter().filter(|f| !list.takes.iter().any(|t| path(&t.id) == **f)) {
        airband_io::files::remove(file).ok();
    }
}

/// One take's sound on its way to the phone, as it is captured.
pub(crate) struct Recorder {
    pub(crate) id: String,
    file: Option<std::fs::File>,
    end: Endpointer,
    spoke: bool,
    /// He talked and stopped, or never started: what a reply opened for him (not by his own tap) ends on.
    finished: bool,
    /// The host stopped the microphone.
    ended: bool,
    bytes: u64,
}

impl Recorder {
    /// Keep this piece: on the disk before the next is read. The words to tell him when it could not be.
    fn keep(&mut self, bytes: &[u8]) -> Option<String> {
        for frame in samples(bytes).chunks(320) {
            if matches!(self.end.feed(frame), Heard::Finished | Heard::Silence) {
                self.finished = true;
            }
        }
        self.spoke |= self.end.speaking;
        self.bytes += bytes.len() as u64;
        let file = self.file.as_mut()?;
        match file.write_all(bytes).and_then(|()| file.sync_data()) {
            Ok(()) => None,
            Err(e) => {
                // told once: the rest of this take is not kept
                self.file = None;
                Some(crate::kept::not_kept(&e.to_string()))
            }
        }
    }
}

/// The host's microphone, with everything read from it written to a take as well: whatever listens reads
/// through this, so the recording is whole whether or not the listening goes well.
pub(crate) struct Tape<'a> {
    host: &'a dyn RadioHost,
    rec: Mutex<Recorder>,
    takes: &'a Takes,
}

impl Tape<'_> {
    /// The listening stopped before he did: read on to the end, so all he said is kept. To the end is until the
    /// host stops the microphone when he opened it himself (`manual`), and otherwise until he stops talking.
    pub(crate) fn rest(&self, manual: bool) {
        loop {
            {
                let rec = self.rec.lock().unwrap_or_else(|e| e.into_inner());
                if rec.ended || (!manual && rec.finished) {
                    return;
                }
            }
            self.record_read();
        }
    }
    pub(crate) fn done(self) -> Recorder {
        self.rec.into_inner().unwrap_or_else(|e| e.into_inner())
    }
}

impl RadioHost for Tape<'_> {
    fn record_read(&self) -> Vec<u8> {
        let bytes = self.host.record_read();
        let told = {
            let mut rec = self.rec.lock().unwrap_or_else(|e| e.into_inner());
            if bytes.is_empty() {
                rec.ended = true;
                None
            } else {
                rec.keep(&bytes)
            }
        };
        if let Some(told) = told {
            self.takes.say(told);
        }
        bytes
    }
    fn record_start(&self, rate: u32) -> bool {
        self.host.record_start(rate)
    }
    fn record_stop(&self) {
        self.host.record_stop();
    }
    fn speak(&self, text: String) {
        self.host.speak(text);
    }
    fn listen(&self) -> Option<String> {
        self.host.listen()
    }
    fn status(&self, status: RadioStatus) {
        self.host.status(status);
    }
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        self.host.play(rate, pcm, last);
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        self.host.outbox(waiting, note);
    }
    fn hearing(&self, text: String) {
        self.host.hearing(text);
    }
    fn words(&self, text: String, starts: Vec<u32>, at_ms: Vec<u32>) {
        self.host.words(text, starts, at_ms);
    }
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        self.host.run_model(engine, folder, job, input)
    }
    fn told(&self, text: String) {
        self.host.told(text);
    }
    fn unplaced(&self, message: Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        self.host.unplaced(message, elsewhere, ask);
    }
}

/// A recording played in place of the microphone, as fast as it is read: for hearing a take again. A second
/// at a time, so a live service is sent the whole of a long one without any of it waiting to be dropped.
pub(crate) struct Replay<'a> {
    host: &'a dyn RadioHost,
    left: Mutex<VecDeque<Vec<u8>>>,
}

impl<'a> Replay<'a> {
    pub(crate) fn of(host: &'a dyn RadioHost, pcm: &[u8]) -> Replay<'a> {
        Replay { host, left: Mutex::new(pcm.chunks(crate::ears::RATE as usize * 2).map(<[u8]>::to_vec).collect()) }
    }
}

impl RadioHost for Replay<'_> {
    fn record_start(&self, _: u32) -> bool {
        true
    }
    fn record_read(&self) -> Vec<u8> {
        self.left.lock().unwrap_or_else(|e| e.into_inner()).pop_front().unwrap_or_default()
    }
    fn record_stop(&self) {}
    // not the microphone and not a call: nothing for a call screen, and nothing to show as he speaks
    fn status(&self, _: RadioStatus) {}
    fn hearing(&self, _: String) {}
    fn speak(&self, text: String) {
        self.host.speak(text);
    }
    fn listen(&self) -> Option<String> {
        None
    }
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        self.host.play(rate, pcm, last);
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        self.host.outbox(waiting, note);
    }
    fn words(&self, text: String, starts: Vec<u32>, at_ms: Vec<u32>) {
        self.host.words(text, starts, at_ms);
    }
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        self.host.run_model(engine, folder, job, input)
    }
    fn told(&self, text: String) {
        self.host.told(text);
    }
    fn unplaced(&self, message: Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        self.host.unplaced(message, elsewhere, ask);
    }
}

#[cfg(test)]
pub(crate) mod tests;
