//! Following one computer: its feed, for as long as it is paired.
use super::*;

/// Keeps one computer's part of the chat current. One request is always open with its tower.
/// `station` is the computer's id, and `epoch` the connection `radio` is: when the computer has another, this
/// one stops.
pub(super) fn follow(shared: Arc<Shared>, radio: Arc<Radio>, station: StationId, epoch: u64) {
    let mut first = true;
    let mut told_taken: Option<Vec<airband_core::routing::Callsign>> = None;
    loop {
        // a computer that has not been told where this device can be woken is told as soon as it is reached
        tell_owed(&shared, &radio, &station, epoch);
        // the tower is told whether the chat is in front of him: only then does having it count as seeing it
        radio.set_away(!shared.host.watching());
        // the first answer must come at once: it is what says the tower is there and who is on it
        let got = radio.follow(if first { 0 } else { 25 });
        let mut seen: Vec<String> = Vec::new();
        // this computer has just been reached, or says it now reads with something else
        let mut reached = false;
        // what reads there has just stopped working, in words for him
        let mut reads_trouble: Option<String> = None;
        // the team colours could not be kept, in words for him
        let mut colours_lost: Option<String> = None;
        {
            let mut state = shared.state.lock().unwrap();
            let calling = state.calling;
            let open: HashSet<String> = state.computers.iter().flat_map(|s| s.open.iter().cloned()).collect();
            let Some(computer) = state.live(&station, epoch) else { return };
            match &got {
                Ok(feed) => {
                    first = false;
                    computer.tried = true;
                    computer.fails = 0;
                    computer.failing_since = None;
                    computer.answered_at = Some(std::time::Instant::now());
                    reached = !computer.online || computer.reads != feed.reads || computer.reads_failed != feed.reads_failed;
                    computer.online = true;
                    if !feed.station.is_empty() && computer.name != feed.station {
                        computer.name = feed.station.clone();
                        computer.radio.store.change(|s| s.computer = feed.station.clone());
                    }
                    computer.reads = feed.reads.clone();
                    // what reads there has stopped working: he is told once, when it stops, and each message
                    // comes as it was written until it works again
                    if !feed.reads_failed.is_empty() && computer.reads_failed != feed.reads_failed {
                        reads_trouble = Some(feed.reads_failed.clone());
                    }
                    computer.reads_failed = feed.reads_failed.clone();
                    computer.agents = feed.agents.clone();
                    computer.open = feed.open.iter().cloned().collect();
                    computer.quick = feed.quick.clone();
                    // the radio keeps the chat itself, each line in its current form
                    computer.messages = radio.history();
                    computer.settle_rooms();
                    // on screen in front of him now: read, so a call started later does not read it out
                    if !calling && shared.host.watching() {
                        seen = feed.messages.iter().filter(|m| !m.his && !m.needs_you).map(|m| m.id.clone()).collect();
                    }
                }
                // one request lost is not a computer out of reach: it is asked again at once, and only a second
                // failure says so
                Err(_) => {
                    computer.fails += 1;
                    // a computer being restarted is back in a few seconds: it is only said to be out of reach
                    // when it has been for a while (or has never once answered)
                    let long = computer.failing_since.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(10);
                    if computer.fails >= 2 && (long || computer.answered_at.is_none()) {
                        computer.tried = true;
                        computer.online = false;
                    }
                }
            }
            // the teams this computer has are given their colours here, beside every other computer's
            if got.is_ok()
                && let Some(computer) = state.computer(&station) {
                let rooms: Vec<String> = computer.agents.iter().map(|a| a.room.clone()).chain(computer.messages.iter().map(|m| m.room.clone())).collect();
                colours_lost = state.colours.see(rooms);
                // once every paired computer has answered, a colour kept for a team none of them has is let go
                if state.computers.iter().all(|c| c.answered_at.is_some()) {
                    let all: std::collections::BTreeSet<String> = state.computers.iter().flat_map(|c| c.agents.iter().map(|a| a.room.clone()).chain(c.messages.iter().map(|m| m.room.clone()))).filter(|r| !r.is_empty()).collect();
                    colours_lost = colours_lost.or(state.colours.keep_only(&all));
                }
            }
            // once the tower agrees a message is answered, stop tracking it here
            let now_open: HashSet<String> = state.computers.iter().flat_map(|s| s.open.iter().cloned()).collect();
            if now_open != open {
                state.answered.retain(|id| now_open.contains(id));
            }
        }
        // the names he is likely to say, for whatever listens on this device: his agents and their teams
        if got.is_ok() {
            let names: Vec<String> = shared.state.lock().unwrap().computers.iter().flat_map(|s| s.agents.iter().flat_map(|a| [a.callsign.clone(), a.group.clone()])).collect();
            shared.senses.ears.expect(names);
        }
        shared.host.changed();
        if let Some(lost) = colours_lost {
            shared.host.note(lost);
        }
        if let Some(why) = reads_trouble {
            let name = shared.state.lock().unwrap().computer_name(&station);
            shared.host.note(format!("{name}: summaries are not working. {why} You are getting each message as it was written. To change what makes them, open Settings, then Summaries."));
        }
        if !seen.is_empty() {
            radio.seen(seen);
        }
        // What waits to write his summaries goes to it, and it is asked what it can read with: on a thread of
        // its own, because the computer tries a choice with a real call before it answers.
        if reached {
            let (shared, radio) = (shared.clone(), radio.clone());
            thread::spawn(move || crate::choices::computer_reached(&shared, &radio));
        }
        // Two computers number their agents without knowing of each other. The later one in his list is told
        // the names the earlier ones use, whenever those change, and keeps off them.
        if got.is_ok() {
            let (first, elsewhere): (bool, Vec<airband_core::routing::Callsign>) = {
                let state = shared.state.lock().unwrap();
                let mut names: Vec<airband_core::routing::Callsign> = Vec::new();
                // the computers before this one in his list, as it is now
                let before: Vec<&Computer> = state.computers.iter().take_while(|s| s.id() != station).collect();
                before.iter().flat_map(|s| s.agents.iter().map(|a| a.parts())).for_each(|c| if !names.contains(&c) { names.push(c) });
                (before.is_empty(), names)
            };
            if !first && told_taken.as_ref() != Some(&elsewhere) && radio.taken(&elsewhere) {
                told_taken = Some(elsewhere);
            }
        }
        if got.is_err() {
            let fails = shared.state.lock().unwrap().computer(&station).map(|s| s.fails).unwrap_or(1);
            thread::sleep(Duration::from_millis(if fails <= 1 { 300 } else { 2500 }));
        }
    }
}
