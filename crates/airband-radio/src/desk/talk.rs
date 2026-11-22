//! He opens the microphone himself: what he says is heard here and sent like anything he types.
use super::*;

#[uniffi::export]
impl Desk {
    /// He wants to speak: open the microphone when free. What he says goes to the message `re` if he picked
    /// one; otherwise the tower works out who it is for, among the agents of `room` or all of them.
    pub fn talk(&self, re: Option<String>, room: Option<String>, agent: Option<String>) {
        let radio = {
            let mut state = self.shared.state.lock().unwrap();
            if state.computers.is_empty() {
                return;
            }
            // the message he answers is answered when his words go (`deliver`), not when the microphone opens: he
            // may say nothing, or take it back
            state.discard = false;
            // any computer that is there can do the listening; where the words go is worked out when he sends them
            let pick = files_at(&state).or_else(|| pick_station(&state, "", re.as_deref(), room.as_deref(), agent.as_deref()));
            let online = |id: &StationId| state.computer(id).is_some_and(|s| s.online);
            let Some(pick) = pick.clone().filter(online).or_else(|| state.computers.iter().find(|s| s.online).map(Computer::id)).or(pick) else { return };
            let Some(computer) = state.computer(&pick) else { return };
            computer.radio.clone()
        };
        let shared = self.shared.clone();
        radio.dictate(Box::new(move |heard| {
            shared.host.hearing(String::new());
            if std::mem::take(&mut shared.state.lock().unwrap().discard) {
                return;
            }
            match heard {
                Listened::Said(text) => deliver(&shared, text, None, re, room, agent),
                Listened::Nothing => {}
                // why was said where it was found out (`ears.rs`): nothing chosen, what he chose could not, or
                // the microphone did not open
                Listened::Cannot => {}
            }
        }));
    }
    /// Why the microphone would be opened for nothing as things are, in words for him: nothing is chosen to
    /// listen with, or what he chose cannot be used. A host asks before it opens the microphone, and says
    /// this in its place. Nothing when something can listen.
    pub fn cannot_listen(&self) -> Option<String> {
        self.shared.senses.ears.cannot()
    }
    /// He took back what he was saying. The host closes the microphone next; nothing is sent.
    pub fn cancel_talk(&self) {
        self.shared.state.lock().unwrap().discard = true;
    }
}
