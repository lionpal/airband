//! What every computer is told about this device: where it can be woken, and which coding tools he uses. What
//! is wanted is kept with how each computer stands with it ([`intents`]), so one that says no or cannot be
//! reached changes nothing for the others.
use super::fanout::Reach;
use super::intents::{lines, Intent, Intents};
use super::*;
use airband_core::radio::PushTarget;
use airband_core::relay::PushService;

#[uniffi::export]
impl Desk {
    /// Where this device can be woken with a notification: the push service and the device's address there, as
    /// the system gave it. Every paired computer is told, now and whenever one is paired later. Call it each
    /// time the system gives the address; the same one again does nothing.
    pub fn set_push(&self, provider: PushService, token: String) {
        let (changed, lost) = self.shared.state.lock().unwrap().intents.change(|i| i.want_push(PushTarget { provider, token }));
        if let Some(lost) = lost {
            self.shared.host.note(lost);
        }
        if !changed {
            return;
        }
        let shared = self.shared.clone();
        thread::spawn(move || {
            let connections: Vec<(StationId, u64, Arc<Radio>)> = shared.state.lock().unwrap().computers.iter().map(|s| (s.id(), s.epoch, s.radio.clone())).collect();
            for (station, epoch, radio) in &connections {
                tell_push(&shared, radio, station, *epoch);
            }
        });
    }
}

impl Desk {
    /// What the computers do not all have yet, for the view.
    pub(super) fn intents_view(&self, state: &State) -> Vec<DeskIntent> {
        let reads = self.shared.chooser.reads_intent();
        let kinds = [
            (IntentKind::Reads, reads.as_ref().map(|i| lines(i, state))),
            (IntentKind::Push, state.intents.now.push.as_ref().map(|i| lines(i, state))),
            (IntentKind::Tools, state.intents.now.tools.as_ref().map(|i| lines(i, state))),
        ];
        kinds.into_iter().filter_map(|(kind, computers)| computers.filter(|c| !c.is_empty()).map(|computers| DeskIntent { kind, computers })).collect()
    }
}

/// Everything one computer is still owed of what every computer is to be told, sent now: for each computer
/// as it is reached, and again on each round while one is not taken.
pub(super) fn tell_owed(shared: &Arc<Shared>, radio: &Arc<Radio>, station: &StationId, epoch: u64) {
    tell_push(shared, radio, station, epoch);
    tell_tools(shared, radio, station, epoch);
}

/// What `slot` wants, when that computer is still owed it on this connection and is not being sent it now: its
/// rev and what is wanted. Asking and marking it as being sent are one step, so two rounds never send it
/// twice; [`settle`] lets it go.
pub(super) fn owed<T: Clone>(shared: &Shared, kind: IntentKind, station: &StationId, epoch: u64, slot: fn(&Intents) -> &Option<Intent<T>>) -> Option<(u64, T)> {
    let mut state = shared.state.lock().unwrap();
    state.live(station, epoch)?;
    if state.telling.iter().any(|(k, s)| *k == kind && s == station) {
        return None;
    }
    let owed = slot(&state.intents.now).as_ref().filter(|i| i.owed(station)).map(|i| (i.rev, i.wanted.clone()))?;
    state.telling.push((kind, station.clone()));
    Some(owed)
}

/// Keep what that computer did with `rev` of what `slot` wants, and let it be sent again. An answer to an
/// older one changes nothing.
pub(super) fn settle<T, R>(shared: &Shared, kind: IntentKind, station: &StationId, rev: u64, slot: fn(&mut Intents) -> &mut Option<Intent<T>>, reach: &Reach<R>) {
    let lost = {
        let mut state = shared.state.lock().unwrap();
        state.telling.retain(|(k, s)| !(*k == kind && s == station));
        state.intents.change(|i| if let Some(now) = slot(i).as_mut().filter(|n| n.rev == rev) { now.record(station, reach) }).1
    };
    if let Some(lost) = lost {
        shared.host.note(lost);
    }
}

/// Tell one computer where this device can be woken, if it has not taken it yet.
pub(super) fn tell_push(shared: &Arc<Shared>, radio: &Arc<Radio>, station: &StationId, epoch: u64) {
    let Some((rev, target)) = owed(shared, IntentKind::Push, station, epoch, |i| &i.push) else { return };
    let reach = Reach::of(radio.push_target(target.provider, &target.token));
    settle(shared, IntentKind::Push, station, rev, |i| &mut i.push, &reach);
}

/// Tell one computer which coding tools he uses, if it has not taken them yet: a new agent he asks for without
/// naming a tool is started with the one he uses most.
pub(super) fn tell_tools(shared: &Arc<Shared>, radio: &Arc<Radio>, station: &StationId, epoch: u64) {
    let tools = shared.chooser.tools();
    let lost = {
        let mut state = shared.state.lock().unwrap();
        let same = state.intents.now.tools.as_ref().is_some_and(|i| i.wanted == tools);
        if same { None } else { state.intents.change(|i| i.want_tools(tools)).1 }
    };
    if let Some(lost) = lost {
        shared.host.note(lost);
    }
    let Some((rev, tools)) = owed(shared, IntentKind::Tools, station, epoch, |i| &i.tools) else { return };
    let reach = Reach::of(radio.uses_tools(&tools));
    settle(shared, IntentKind::Tools, station, rev, |i| &mut i.tools, &reach);
}
