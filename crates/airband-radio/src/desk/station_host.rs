//! What one computer's radio is given as its host: the desk's host, with one computer speaking at a time.
use super::*;

/// On a call only one computer may have the floor: its transmission, his answer, its reply. The others wait,
/// so two never talk over each other or take his answer for theirs.
#[derive(Default)]
pub(super) struct Floor {
    pub(super) holder: Mutex<Option<usize>>,
    pub(super) freed: Condvar,
}
impl Floor {
    pub(super) fn take(&self, index: usize) {
        let mut holder = self.holder.lock().unwrap();
        while holder.is_some_and(|h| h != index) {
            holder = self.freed.wait(holder).unwrap();
        }
        *holder = Some(index);
    }
    /// Give it up if `index` has it. Returns whether `index` may show its status: it has the floor or nobody does.
    pub(super) fn release(&self, index: usize, give_up: bool) -> bool {
        let mut holder = self.holder.lock().unwrap();
        if give_up && *holder == Some(index) {
            *holder = None;
            self.freed.notify_all();
        }
        holder.is_none_or(|h| h == index)
    }
    pub(super) fn clear(&self) {
        *self.holder.lock().unwrap() = None;
        self.freed.notify_all();
    }
}
/// What one computer's radio calls back into: it knows which computer it is, and the desk does the rest.
pub(super) struct StationHost {
    pub(super) shared: Arc<Shared>,
    pub(super) index: usize,
    pub(super) generation: u64,
}
impl RadioHost for StationHost {
    // a phone's own voice and recognition are never used
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn told(&self, text: String) {
        // what he chose stays chosen and now says why it did not work: the screens that show it are told too
        self.shared.host.note(text);
        self.shared.host.changed();
    }
    fn unplaced(&self, message: Outgoing, elsewhere: Option<String>, ask: Vec<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.generation != self.generation {
                return;
            }
            // it was not delivered here: it is not on its way to this computer's chat
            if let Some(s) = state.stations.get_mut(self.index) {
                s.landing.retain(|(o, _)| o.id != message.id);
            }
            // the tower read his words as for an agent another computer has: they go there, addressed to it
            let there = elsewhere.as_ref().and_then(|c| state.stations.iter().enumerate().filter(|(i, _)| *i != self.index).find_map(|(_, s)| s.agents.iter().find(|a| a.id == *c).map(|a| (s.radio.clone(), a.id.clone()))));
            match (there, elsewhere) {
                (Some((radio, agent)), Some(_)) => {
                    drop(state);
                    radio.queue_as(message.text.clone(), None, None, message.label.clone(), String::new(), vec![agent], None);
                }
                (_, gone) => state.asking = Some(Asking { message, guesses: gone.into_iter().chain(ask).collect() }),
            }
        }
        self.shared.host.changed();
    }
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        self.shared.floor.take(self.index);
        self.shared.host.play(rate, pcm, last);
    }
    fn record_start(&self, rate: u32) -> bool {
        self.shared.floor.take(self.index);
        self.shared.host.record_start(rate)
    }
    fn record_read(&self) -> Vec<u8> {
        self.shared.host.record_read()
    }
    fn record_stop(&self) {
        self.shared.host.record_stop();
    }
    fn status(&self, status: RadioStatus) {
        // idle or offline ends that computer's turn
        let ends = matches!(status, RadioStatus::Idle | RadioStatus::Offline);
        // what he said has gone, and took with it the files that were waiting: they leave the tray
        if matches!(status, RadioStatus::Idle) {
            let mut state = self.shared.state.lock().unwrap();
            if let Some(waiting) = state.stations.get(self.index).map(|s| s.radio.armed()) {
                let before = state.uploads.len();
                state.uploads.retain(|u| !(u.station == self.index && u.shown.state == UploadState::Ready && !waiting.contains(&u.shown.id)));
                if state.uploads.len() != before {
                    drop(state);
                    self.shared.host.changed();
                }
            }
        }
        if self.shared.floor.release(self.index, ends) {
            self.shared.host.status(status);
        }
    }
    fn hearing(&self, text: String) {
        self.shared.host.hearing(text);
    }
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        self.shared.host.run_model(engine, folder, job, input)
    }
    fn words(&self, text: String, starts: Vec<u32>, at_ms: Vec<u32>) {
        // only the computer whose turn it is to speak says what is being said: another one's words wait with
        // its sound, or they would be shown over the voice of the one speaking
        self.shared.floor.take(self.index);
        self.shared.host.words(text, starts, at_ms);
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.generation != self.generation {
                return;
            }
            if let Some(s) = state.stations.get_mut(self.index) {
                // What has left the list was sent. It stays on screen as on its way until the chat has it (the
                // tower gives the message the same id), so it never vanishes for a moment in between. Not when
                // there is something to tell him about it: then it did not go.
                if note.is_none() {
                    // (one that was given up on and has now been let go did not go anywhere)
                    let gone: Vec<Outgoing> = s.sending.iter().filter(|was| !waiting.iter().any(|w| w.id == was.id)).cloned().collect();
                    let now = std::time::Instant::now();
                    s.landing.retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
                    s.landing.extend(gone.into_iter().map(|o| (o, now)));
                }
                s.sending = waiting;
            }
        }
        self.shared.host.changed();
        if let Some(note) = note.filter(|n| !n.is_empty()) {
            self.shared.host.note(note);
        }
    }
}
