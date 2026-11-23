//! What he said aloud and has not gone yet ([`crate::takes`]), as the chat shows it: play it, try it again, let
//! it go.
use super::*;
use crate::takes::Again;

#[uniffi::export]
impl Desk {
    /// The recording of something he said, as a WAV file to play: one shown in [`DeskView::takes`], or one of
    /// his messages with [`DeskOutgoing::recording`]. Nothing when it is no longer kept, or cannot be read.
    pub fn take_sound(&self, id: String) -> Option<Vec<u8>> {
        self.shared.senses.takes.sound(&id)
    }

    /// Try again something he said that did not go. Not heard: it is heard again from its recording, by what he
    /// has chosen to listen with now, then sent as he aimed it. Heard and not sent: its words are sent, and
    /// nothing listens again. Words already on their way are the outbox's to send again
    /// ([`Desk::send_again`]): nothing is done here. Returns at once; [`DeskView::takes`] shows how it goes.
    pub fn take_again(&self, id: String) {
        let waiting = self.shared.state.lock().unwrap().computers.iter().any(|s| s.radio.unsent().iter().any(|o| o.id == id));
        match self.shared.senses.takes.again(&id, waiting) {
            Again::Send { text, aim } => {
                let _kept = deliver(&self.shared, text, None, aim.re, aim.room, aim.agent, Some(id));
            }
            Again::Hear { aim } => {
                let shared = self.shared.clone();
                thread::spawn(move || {
                    let side = Side { shared: shared.clone() };
                    let heard = crate::ears::Ears::new(shared.senses.clone()).hear_again(&side, &id);
                    match heard {
                        Listened::Said { text, take } => {
                            let _kept = deliver(&shared, text, None, aim.re, aim.room, aim.agent, take);
                        }
                        Listened::Nothing | Listened::Cannot | Listened::Unheard { .. } => {}
                    }
                    shared.host.changed();
                });
            }
            Again::Nothing => {}
        }
        self.shared.host.changed();
    }

    /// Let go of something he said that did not go: its recording is deleted for good.
    pub fn discard_take(&self, id: String) {
        self.shared.senses.takes.discard(&id);
        self.shared.host.changed();
    }
}

/// What hearing a take again is done for: the desk's host, for running a model and telling him. It is not the
/// microphone and not a call: no computer's turn is taken, and nothing is shown as he speaks.
struct Side {
    shared: Arc<Shared>,
}

impl RadioHost for Side {
    fn run_model(&self, engine: String, folder: String, job: ChoiceJob, input: Vec<u8>) -> Option<Vec<u8>> {
        self.shared.host.run_model(engine, folder, job, input)
    }
    fn told(&self, text: String) {
        self.shared.host.note(text);
        self.shared.host.changed();
    }
    fn speak(&self, _: String) {}
    fn listen(&self) -> Option<String> {
        None
    }
    fn status(&self, _: RadioStatus) {}
    fn play(&self, _: u32, _: Vec<u8>, _: bool) {}
    // the recording is read in its place ([`crate::takes::Replay`])
    fn record_start(&self, _: u32) -> bool {
        false
    }
    fn record_read(&self) -> Vec<u8> {
        Vec::new()
    }
    fn record_stop(&self) {}
    fn outbox(&self, _: Vec<Outgoing>, _: Option<String>) {}
    fn hearing(&self, _: String) {}
    fn words(&self, _: String, _: Vec<u32>, _: Vec<u32>) {}
    fn unplaced(&self, _: Outgoing, _: Option<String>, _: Vec<String>) {}
}
