//! What every computer is told about this device: where it can be woken, and which coding tools he uses.
use super::*;

/// A push service this device can be woken through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PushService {
    /// Apple's.
    Apple,
}
impl PushService {
    pub(crate) fn wire(self) -> airband_core::relay::PushService {
        match self {
            PushService::Apple => airband_core::relay::PushService::Apns,
        }
    }
}

#[uniffi::export]
impl Desk {
    /// Where this device can be woken with a notification: the push service and the device's address there, as
    /// the system gave it. Every paired computer is told, now and whenever one is paired later. Call it each
    /// time the system gives the address; the same one again does nothing.
    pub fn set_push(&self, provider: PushService, token: String) {
        {
            let mut state = self.shared.state.lock().unwrap();
            let push = Some((provider, token));
            if state.push == push {
                return;
            }
            state.push = push;
            state.push_told.clear();
        }
        let shared = self.shared.clone();
        thread::spawn(move || {
            let (generation, radios): (u64, Vec<Arc<Radio>>) = {
                let state = shared.state.lock().unwrap();
                (state.generation, state.radios())
            };
            for (index, radio) in radios.iter().enumerate() {
                tell_push(&shared, radio, index, generation);
            }
        });
    }
}

/// Tell one computer where this device can be woken, if it has not been told. A computer that cannot be reached
/// or does not take it is told the next time round.
pub(super) fn tell_push(shared: &Arc<Shared>, radio: &Arc<Radio>, index: usize, generation: u64) {
    let (link, push) = {
        let state = shared.state.lock().unwrap();
        let Some(station) = state.stations.get(index).filter(|_| state.generation == generation) else { return };
        let Some(push) = state.push.clone().filter(|_| !state.push_told.contains(&station.link)) else { return };
        (station.link.clone(), push)
    };
    if radio.push_target(push.0, &push.1).is_ok() {
        let mut state = shared.state.lock().unwrap();
        // unless the address changed while this one was on its way
        if state.push.as_ref() == Some(&push) {
            state.push_told.insert(link);
        }
    }
}
/// Tell one computer which coding tools he uses, if it has not been told or he has since changed them: a new
/// agent he asks for without naming a tool is started with the one he uses most.
pub(super) fn tell_tools(shared: &Arc<Shared>, radio: &Arc<Radio>, index: usize, generation: u64) {
    let tools = shared.chooser.tools();
    let link = {
        let state = shared.state.lock().unwrap();
        let Some(station) = state.stations.get(index).filter(|_| state.generation == generation) else { return };
        if state.tools_told.get(&station.link) == Some(&tools) {
            return;
        }
        station.link.clone()
    };
    if radio.uses_tools(&tools).is_ok() {
        shared.state.lock().unwrap().tools_told.insert(link, tools);
    }
}
