//! Following one computer: its feed, for as long as it is paired.
use super::*;

/// Keeps one computer's part of the chat current. One request is always open with its tower.
pub(super) fn follow(shared: Arc<Shared>, radio: Arc<Radio>, index: usize, generation: u64) {
    let mut first = true;
    let mut told_taken: Option<Vec<String>> = None;
    loop {
        // a computer that has not been told where this device can be woken is told as soon as it is reached
        tell_push(&shared, &radio, index, generation);
        tell_tools(&shared, &radio, index, generation);
        // the tower is told whether the chat is in front of him: only then does having it count as seeing it
        radio.set_away(!shared.host.watching());
        // the first answer must come at once: it is what says the tower is there and who is on it
        let got = radio.follow(if first { 0 } else { 25 });
        let mut seen: Vec<String> = Vec::new();
        // this computer has just been reached, or says it now reads with something else
        let mut reached = false;
        // what reads there has just stopped working, in words for him
        let mut reads_trouble: Option<String> = None;
        {
            let mut state = shared.state.lock().unwrap();
            if state.generation != generation {
                return;
            }
            let calling = state.calling;
            let open: HashSet<String> = state.stations.iter().flat_map(|s| s.open.iter().cloned()).collect();
            let Some(station) = state.stations.get_mut(index) else { return };
            match &got {
                Ok(feed) => {
                    first = false;
                    station.tried = true;
                    station.fails = 0;
                    station.failing_since = None;
                    station.answered_at = Some(std::time::Instant::now());
                    reached = !station.online || station.reads != feed.reads || station.reads_failed != feed.reads_failed;
                    station.online = true;
                    station.name = feed.station.clone();
                    station.reads = feed.reads.clone();
                    // what reads there has stopped working: he is told once, when it stops, and each message
                    // comes as it was written until it works again
                    if !feed.reads_failed.is_empty() && station.reads_failed != feed.reads_failed {
                        reads_trouble = Some(format!("Summaries are not working on {}. {} You are getting each message as it was written. To change what makes them, open Settings, then Summaries.", if feed.station.is_empty() { "your computer" } else { feed.station.as_str() }, feed.reads_failed));
                    }
                    station.reads_failed = feed.reads_failed.clone();
                    station.agents = feed.agents.clone();
                    station.open = feed.open.iter().cloned().collect();
                    station.quick = feed.quick.clone();
                    station.colours = feed.colours.clone();
                    // the radio keeps the chat itself, each line in its current form
                    station.messages = radio.history();
                    // on screen in front of him now: read, so a call started later does not read it out
                    if !calling && shared.host.watching() {
                        seen = feed.messages.iter().filter(|m| m.from != "you" && !m.needs_you).map(|m| m.id.clone()).collect();
                    }
                }
                // one request lost is not a computer out of reach: it is asked again at once, and only a second
                // failure says so
                Err(_) => {
                    station.fails += 1;
                    // a computer being restarted is back in a few seconds: it is only said to be out of reach
                    // when it has been for a while (or has never once answered)
                    let long = station.failing_since.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(10);
                    if station.fails >= 2 && (long || station.answered_at.is_none()) {
                        station.tried = true;
                        station.online = false;
                    }
                }
            }
            // once the tower agrees a message is answered, stop tracking it here
            let now_open: HashSet<String> = state.stations.iter().flat_map(|s| s.open.iter().cloned()).collect();
            if now_open != open {
                state.answered.retain(|id| now_open.contains(id));
            }
        }
        // the names he is likely to say, for whatever listens on this device: his agents and their teams
        if got.is_ok() {
            let names: Vec<String> = shared.state.lock().unwrap().stations.iter().flat_map(|s| s.agents.iter().flat_map(|a| [a.callsign.clone(), a.group.clone()])).collect();
            shared.senses.ears.expect(names);
        }
        shared.host.changed();
        if let Some(trouble) = reads_trouble {
            shared.host.note(trouble);
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
        if got.is_ok() && index > 0 {
            let elsewhere: Vec<String> = {
                let state = shared.state.lock().unwrap();
                let mut names: Vec<String> = state.stations.iter().take(index).flat_map(|s| s.agents.iter().map(|a| a.callsign.clone())).collect();
                names.sort();
                names.dedup();
                names
            };
            if told_taken.as_ref() != Some(&elsewhere) && radio.taken(&elsewhere) {
                told_taken = Some(elsewhere);
            }
        }
        if got.is_err() {
            let fails = shared.state.lock().unwrap().stations.get(index).map(|s| s.fails).unwrap_or(1);
            thread::sleep(Duration::from_millis(if fails <= 1 { 300 } else { 2500 }));
        }
    }
}
