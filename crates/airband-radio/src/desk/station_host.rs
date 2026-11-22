//! What one computer's radio is given as its host: the desk's host, with one computer speaking at a time.
use super::*;

/// On a call only one computer may have the floor: its transmission, his answer, its reply. The others wait,
/// so two never talk over each other or take his answer for theirs.
#[derive(Default)]
pub(super) struct Floor {
    /// The computer that has it, by its id.
    pub(super) holder: Mutex<Option<StationId>>,
    pub(super) freed: Condvar,
}
impl Floor {
    pub(super) fn take(&self, computer: &StationId) {
        let mut holder = self.holder.lock().unwrap();
        while holder.as_ref().is_some_and(|h| h != computer) {
            holder = self.freed.wait(holder).unwrap();
        }
        *holder = Some(computer.clone());
    }
    /// Give it up if `computer` has it. Returns whether `computer` may show its status: it has the floor or
    /// nobody does.
    pub(super) fn release(&self, computer: &StationId, give_up: bool) -> bool {
        let mut holder = self.holder.lock().unwrap();
        if give_up && holder.as_ref() == Some(computer) {
            *holder = None;
            self.freed.notify_all();
        }
        holder.as_ref().is_none_or(|h| h == computer)
    }
    pub(super) fn clear(&self) {
        *self.holder.lock().unwrap() = None;
        self.freed.notify_all();
    }
}
/// What one computer's radio calls back into: it knows which computer it is, and the desk does the rest.
pub(super) struct ComputerHost {
    pub(super) shared: Arc<Shared>,
    /// The computer, by its id.
    pub(super) station: StationId,
    /// The connection this host belongs to: once the computer has another, this one's callbacks do nothing.
    pub(super) epoch: u64,
}
impl RadioHost for ComputerHost {
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
            // it was not delivered here: it is not on its way to this computer's chat
            let Some(s) = state.live(&self.station, self.epoch) else { return };
            s.landing.retain(|(o, _)| o.id != message.id);
            // the tower read his words as for an agent another computer has: they go there, addressed to it
            let there = elsewhere.as_ref().filter(|c| state.with_agent(c).is_some_and(|s| s.id() != self.station)).cloned();
            match (there, elsewhere) {
                (Some(agent), Some(_)) => {
                    drop(state);
                    place(&self.shared, &self.station, message, agent);
                }
                (_, gone) => {
                    state.asking = Some(Asking { message, guesses: gone.into_iter().chain(ask).collect(), at: self.station.clone() });
                }
            }
        }
        self.shared.host.changed();
    }
    fn play(&self, rate: u32, pcm: Vec<u8>, last: bool) {
        self.shared.floor.take(&self.station);
        self.shared.host.play(rate, pcm, last);
    }
    fn record_start(&self, rate: u32) -> bool {
        self.shared.floor.take(&self.station);
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
            if let Some(waiting) = state.computer(&self.station).map(|s| s.radio.armed()) {
                let before = state.uploads.len();
                let here = &self.station;
                state.uploads.retain(|u| !(&u.station == here && u.shown.state == UploadState::Ready && !waiting.contains(&u.shown.id)));
                if state.uploads.len() != before {
                    drop(state);
                    self.shared.host.changed();
                }
            }
        }
        if self.shared.floor.release(&self.station, ends) {
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
        self.shared.floor.take(&self.station);
        self.shared.host.words(text, starts, at_ms);
    }
    fn outbox(&self, waiting: Vec<Outgoing>, note: Option<String>) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let Some(s) = state.live(&self.station, self.epoch) else { return };
            // What has left the list was sent. It stays on screen as on its way until the chat has it (the
            // tower gives the message the same id), so it never vanishes for a moment in between. Not when
            // there is something to tell him about it: then it did not go.
            if note.is_none() {
                // (one that was given up on and has now been let go did not go anywhere)
                let gone: Vec<Outgoing> = s.was_waiting.iter().filter(|was| !waiting.iter().any(|w| w.id == was.id)).cloned().collect();
                let now = std::time::Instant::now();
                s.landing.retain(|(_, at)| at.elapsed() < Duration::from_secs(15));
                s.landing.extend(gone.into_iter().map(|o| (o, now)));
            }
            s.was_waiting = waiting;
        }
        self.shared.host.changed();
        if let Some(note) = note.filter(|n| !n.is_empty()) {
            self.shared.host.note(note);
        }
    }
}
