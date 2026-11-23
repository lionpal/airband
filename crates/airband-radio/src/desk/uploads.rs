//! Files he adds to what he says: sent to a computer ahead of his words, and shown in the tray meanwhile.
use super::*;

pub(super) struct Upload {
    pub(super) shown: DeskUpload,
    /// Which computer it is going to, by its id: what he sends next goes there too.
    pub(super) station: StationId,
    /// He took it off, or it is being sent somewhere else instead: stop sending.
    pub(super) dropped: Arc<AtomicBool>,
    /// The file on the device and what to call it, kept so it can be sent to another computer if his words turn
    /// out to be for an agent there.
    pub(super) file_path: String,
    pub(super) name: String,
}
/// Send one attached file to a computer, keeping the tray's picture of it up to date. When it is there it is
/// armed on that computer's radio: the next thing he sends there takes it along.
pub(super) fn send_file(shared: Arc<Shared>, radio: Arc<Radio>, id: String, file_path: String, name: String, dropped: Arc<AtomicBool>) {
    thread::spawn(move || {
        let last_told = Mutex::new(std::time::Instant::now());
        let step = |sent: u64, _total: u64| {
            if dropped.load(Ordering::SeqCst) {
                return false;
            }
            if let Some(u) = shared.state.lock().unwrap().uploads.iter_mut().find(|u| u.shown.id == id && Arc::ptr_eq(&u.dropped, &dropped)) {
                u.shown.sent = sent;
            }
            // the screen is told a few times a second at most, however fast the parts land
            let mut told = last_told.lock().unwrap();
            if told.elapsed() > Duration::from_millis(250) {
                *told = std::time::Instant::now();
                drop(told);
                shared.host.changed();
            }
            true
        };
        let outcome = radio.upload(&id, &file_path, &name, step);
        let after = {
            let mut state = shared.state.lock().unwrap();
            // only if this is still the sending that counts: it may have been taken off, or started again elsewhere
            if let Some(u) = state.uploads.iter_mut().find(|u| u.shown.id == id && Arc::ptr_eq(&u.dropped, &dropped)) {
                let arrived = matches!(outcome, Ok(crate::radio::UploadEnd::Arrived));
                u.shown.state = if arrived { UploadState::Ready } else { UploadState::Failed };
                // why it did not get there stays with it, for him to see
                u.shown.why = outcome.as_ref().err().map(RadioError::said);
                if arrived {
                    radio.arm(id.clone());
                }
            }
            settled(&mut state)
        };
        shared.host.changed();
        if let Some(after) = after {
            after.go(&shared);
        }
    });
}
/// Words of his that wait for his files to reach the computer they are going to.
pub(super) struct AfterFiles {
    pub(super) text: String,
    pub(super) label: Option<String>,
    pub(super) re: Option<String>,
    pub(super) room: Option<String>,
    pub(super) agent: Option<String>,
    /// Words he said aloud: the take they were heard from.
    pub(super) said: Option<String>,
}
impl AfterFiles {
    fn go(self, shared: &Arc<Shared>) {
        deliver(shared, self.text, self.label, self.re, self.room, self.agent, self.said);
    }
}
/// The words waiting for his files, taken, once none of the files is still on its way: each has got there,
/// failed (the tray says which), or been taken off.
pub(super) fn settled(state: &mut State) -> Option<AfterFiles> {
    if state.uploads.iter().any(|u| u.shown.state == UploadState::Sending) { None } else { state.after_files.take() }
}
/// His words are for a computer other than the one his files went to: each file is sent there afresh (the tray
/// shows how each one does), and the words go when the last has got there or failed.
pub(super) fn move_files(shared: &Arc<Shared>, state: &mut State, from: &StationId, to: &StationId, words: AfterFiles) -> bool {
    let (Some(to_radio), Some(from_radio)) = (state.computer(to).map(|s| s.radio.clone()), state.computer(from).map(|s| s.radio.clone())) else { return false };
    let mut moved = Vec::new();
    for u in state.uploads.iter_mut() {
        u.dropped.store(true, Ordering::SeqCst);
        u.dropped = Arc::new(AtomicBool::new(false));
        moved.push((u.shown.id.clone(), u.file_path.clone(), u.name.clone(), u.dropped.clone()));
        u.station = to.clone();
        u.shown.sent = 0;
        u.shown.state = UploadState::Sending;
    }
    // words that were waiting already wait no more: said aloud, they wait in the chat to be sent again
    if let Some(take) = state.after_files.replace(words).and_then(|was| was.said) {
        shared.senses.takes.unsent(&take);
    }
    for (id, file_path, name, dropped) in moved {
        from_radio.disarm(id.clone());
        send_file(shared.clone(), to_radio.clone(), id, file_path, name, dropped);
    }
    true
}
/// Where the files he attached are, among his computers now: everything attached to one message goes to one
/// computer, the first file's.
pub(super) fn files_at(state: &State) -> Option<StationId> {
    state.uploads.first().filter(|u| state.computer(&u.station).is_some()).map(|u| u.station.clone())
}
#[uniffi::export]
impl Desk {
    /// Attach a file from the device (a photo, a video): it starts on its way to the computer at once, and goes
    /// to whoever his next words go to. `name` is what to call it there; its extension says what it is. Returns
    /// its id straight away; [`DeskView::uploads`] shows how far it has got.
    pub fn attach(&self, file_path: String, name: String, re: Option<String>, room: Option<String>, agent: Option<String>) -> String {
        let id = random_hex(12);
        let dropped = Arc::new(AtomicBool::new(false));
        let total = airband_io::files::metadata(std::path::Path::new(&file_path)).map(|m| m.len()).unwrap_or(0);
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            if state.computers.is_empty() {
                return id;
            }
            // everything attached to one message goes to one computer: the first file's
            let Some(station) = files_at(&state).or_else(|| pick_station(&state, "", re.as_deref(), room.as_deref(), agent.as_deref())) else { return id };
            let Some(radio) = state.computer(&station).map(|s| s.radio.clone()) else { return id };
            state.uploads.push(Upload { shown: DeskUpload { id: id.clone(), name: name.clone(), sent: 0, total, state: UploadState::Sending, why: None }, station, dropped: dropped.clone(), file_path: file_path.clone(), name: name.clone() });
            radio
        };
        self.shared.host.changed();
        send_file(self.shared.clone(), radio, id.clone(), file_path, name, dropped);
        id
    }
    /// Take an attached file off again before it is sent.
    pub fn detach(&self, id: String) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            let Some(i) = state.uploads.iter().position(|u| u.shown.id == id) else { return };
            let gone = state.uploads.remove(i);
            gone.dropped.store(true, Ordering::SeqCst);
            (state.computer(&gone.station).map(|s| s.radio.clone()), settled(&mut state))
        };
        if let (Some(radio), _) = &radio {
            radio.disarm(id);
        }
        self.shared.host.changed();
        if let (_, Some(after)) = radio {
            after.go(&self.shared);
        }
    }
}
