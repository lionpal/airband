//! Reading aloud and looking closer: call mode, a message whole, a picture, and how each computer is reached.
use super::*;

#[uniffi::export]
impl Desk {
    /// Call mode on every computer: transmissions are spoken one at a time and the microphone opens after each.
    pub fn set_call_mode(&self, on: bool) {
        let radios: Vec<Arc<Radio>> = {
            let mut state = self.shared.state.lock().unwrap();
            state.calling = on;
            state.radios()
        };
        if !on {
            self.shared.floor.clear();
        }
        // with no voice chosen there is nothing to read aloud with: he is told now, where he turned it on
        if on && !self.shared.senses.mouth.ready() {
            self.shared.host.note(crate::voice::nothing_chosen());
        }
        radios.iter().for_each(|r| r.set_call_mode(on));
    }
    /// Exactly what the agent wrote. Blocks: call it off the main thread.
    pub fn full(&self, message_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or_else(|| RadioError::gone("message"))?.more(message_id, true)
    }
    /// Read a message to him, a question before he answers it or the latest an agent said, as a call reads it:
    /// who said it, then its line in the chat's words, in the voice he chose on this device. Blocks until it has
    /// been heard or [`Desk::stop_reading`] is called. False when it was not read.
    pub fn read_question(&self, message_id: String) -> bool {
        self.radio_of(&message_id).is_some_and(|radio| radio.read_question(&message_id))
    }
    /// Leave off reading aloud. The host stops what is already playing.
    pub fn stop_reading(&self) {
        let radios = self.shared.state.lock().unwrap().radios();
        radios.iter().for_each(|r| r.hush());
    }
    /// One of a message's pictures, as a file on the device. Blocks: call it off the main thread.
    pub fn picture(&self, message_id: String, picture_id: String) -> Result<String, RadioError> {
        self.radio_of(&message_id).ok_or_else(|| RadioError::gone("message"))?.picture(picture_id)
    }
    /// For each computer: how it was last reached, or not, through the relay and at its own address on the
    /// network the phone is on. From what this phone has seen itself; nothing is asked of anyone to make it.
    pub fn reach(&self) -> Vec<DeskReach> {
        let radios = self.shared.state.lock().unwrap().named_radios();
        radios
            .into_iter()
            .map(|(computer, radio)| {
                let (relay, near) = radio.reach();
                DeskReach { id: computer.id, computer: computer.name, relay, near }
            })
            .collect()
    }
    /// What to copy when something is wrong with the connection: for each computer, every request that failed
    /// lately and every try at sending something he said, with the time, the way it went, how it ended and how
    /// long it took; and what became of the latest notifications, from the folder the host shares with what
    /// opens them (`notices`). Nothing that he or an agent said is in it, and no key or address. It is made on
    /// this device from what this device kept, and goes nowhere unless he pastes it somewhere.
    pub fn diagnostics(&self, notices: Option<Arc<crate::SharedFolder>>) -> String {
        let radios = self.shared.state.lock().unwrap().named_radios();
        let mut out = vec![format!("airband {} connection record", env!("CARGO_PKG_VERSION"))];
        for (computer, radio) in radios {
            let trail = radio.link_trail();
            out.push(String::new());
            out.push(computer.name);
            if trail.is_empty() {
                out.push("nothing has failed".into());
            }
            out.extend(trail);
            // what the computer is told in passing (he heard this, he saw that) and did not get
            let lost = radio.lost();
            if !lost.is_empty() {
                out.push(format!("told in passing and not delivered, since the app was opened: {}", lost.iter().map(|(what, n)| format!("{what} {n}")).collect::<Vec<_>>().join(", ")));
            }
        }
        let timings = self.shared.chooser.timings.lines();
        if !timings.is_empty() {
            out.push(String::new());
            out.push("models on this phone".into());
            out.extend(timings);
        }
        if let Some(folder) = notices {
            out.push(String::new());
            out.push("notifications".into());
            let lines = folder.notice_lines();
            if lines.is_empty() {
                out.push("none has come since this was last installed".into());
            }
            out.extend(lines);
        }
        out.join("\n")
    }
}
