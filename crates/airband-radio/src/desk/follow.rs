//! Following one computer: its feed, for as long as it is paired.
use super::*;

/// Keeps one computer's part of the chat current. One request is always open with its tower.
/// `station` is the computer's id, and `epoch` the connection `radio` is: when the computer has another, this
/// one stops.
pub(super) fn follow(shared: Arc<Shared>, radio: Arc<Radio>, station: StationId, epoch: u64) {
    let mut first = true;
    // what writes his summaries is being sent on a thread of its own: one at a time
    let reads_busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut told_taken: Option<Vec<airband_core::routing::Callsign>> = None;
    loop {
        // a computer that has not been told where this device can be woken is told as soon as it is reached
        tell_owed(&shared, &radio, &station, epoch);
        // the tower is told whether the chat is in front of him: only then does having it count as seeing it
        radio.set_away(!shared.host.watching());
        // the first answer must come at once: it is what says the tower is there and who is on it
        let got = radio.followed(if first { 0 } else { 25 });
        first &= got.is_err();
        let Some(round) = take(&shared, &radio, &station, epoch, &got) else { return };
        // the names he is likely to say, for whatever listens on this device: his agents and their teams
        if got.is_ok() {
            let names: Vec<String> = shared.state.lock().unwrap().computers.iter().flat_map(|s| s.agents.iter().flat_map(|a| [a.callsign.clone(), a.group.clone()])).collect();
            shared.senses.ears.expect(names);
        }
        shared.host.changed();
        if let Some(lost) = round.colours_lost {
            shared.host.note(lost);
        }
        if let Some(why) = round.reads_trouble {
            let name = shared.state.lock().unwrap().computer_name(&station);
            shared.host.note(format!("{name}: summaries are not working. {why} You are getting each message as it was written. To change what makes them, open Settings, then Summaries."));
        }
        if !round.seen.is_empty() {
            radio.seen(round.seen);
        }
        // It refused this phone for good: asking again gets the same no. Nothing more is asked of it on this
        // connection; a new one is made when something changes that can change the answer: he pairs again,
        // asks for it afresh, or brings the app forward (audit 9, B-2). A newer app is a new launch.
        if round.refused {
            let state = shared.state.lock().unwrap();
            drop(shared.moved.wait_while(state, |s| s.live(&station, epoch).is_some()).unwrap());
            return;
        }
        // What waits to write his summaries goes to it when it is reached, and on each round while it is owed,
        // as push and tools are; when it is reached it is also asked what it can read with. On a thread of its
        // own, because the computer tries a choice with a real call before it answers.
        let owed = got.is_ok() && shared.chooser.reads_intent().is_some_and(|i| i.owed(&station));
        if (round.reached || owed) && !reads_busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let (shared, station, busy, reached) = (shared.clone(), station.clone(), reads_busy.clone(), round.reached);
            thread::spawn(move || {
                crate::choices::tell_reads(&shared, &station, epoch, reached);
                busy.store(false, std::sync::atomic::Ordering::SeqCst);
            });
        }
        match got {
            Ok(_) => tell_taken(&shared, &radio, &station, &mut told_taken),
            // Not reached: asked again at once after one failure, then every few seconds. A timer standing in
            // for a fact: the phone is not told when its network or the computer comes back.
            Err(_) => thread::sleep(Duration::from_millis(if round.fails <= 1 { 300 } else { 2500 })),
        }
    }
}

/// What one round of following came to, for what is done outside the lock.
struct Round {
    /// On screen in front of him now: read, so a call started later does not read it out.
    seen: Vec<String>,
    /// This computer has just been reached, or says it now reads with something else.
    reached: bool,
    /// What reads there has just stopped working, in words for him.
    reads_trouble: Option<String>,
    /// The team colours could not be kept, in words for him.
    colours_lost: Option<String>,
    /// It answered that it will never talk to this phone as things are.
    refused: bool,
    /// How many times in a row it has not answered.
    fails: u32,
}

/// What came back is taken into the computer, under the lock. Nothing when `epoch` is no longer its
/// connection.
fn take(shared: &Shared, radio: &Radio, station: &StationId, epoch: u64, got: &Result<ChatFeed, crate::link::LinkError>) -> Option<Round> {
    let mut state = shared.state.lock().unwrap();
    let calling = state.calling;
    let open: HashSet<String> = state.computers.iter().flat_map(|s| s.open.iter().cloned()).collect();
    let computer = state.live(station, epoch)?;
    let mut round = Round { seen: Vec::new(), reached: false, reads_trouble: None, colours_lost: None, refused: false, fails: 0 };
    match got {
        Ok(feed) => {
            computer.tried = true;
            computer.fails = 0;
            computer.failing_since = None;
            computer.answered_at = Some(std::time::Instant::now());
            computer.refused = None;
            round.reached = !computer.online || computer.reads != feed.reads || computer.reads_failed != feed.reads_failed;
            computer.online = true;
            if !feed.station.is_empty() && computer.name != feed.station {
                computer.name = feed.station.clone();
                computer.radio.store.change(|s| s.computer = feed.station.clone());
            }
            computer.reads = feed.reads.clone();
            // what reads there has stopped working: he is told once, when it stops, and each message comes as
            // it was written until it works again
            if !feed.reads_failed.is_empty() && computer.reads_failed != feed.reads_failed {
                round.reads_trouble = Some(feed.reads_failed.clone());
            }
            computer.reads_failed = feed.reads_failed.clone();
            computer.agents = feed.agents.clone();
            computer.open = feed.open.iter().cloned().collect();
            computer.quick = feed.quick.clone();
            // the radio keeps the chat itself, each line in its current form
            computer.messages = radio.history();
            computer.settle_rooms();
            // a team or agent he was looking at that has gone is let go
            state.keep_focus();
            if !calling && shared.host.watching() {
                round.seen = feed.messages.iter().filter(|m| !m.his && !m.needs_you).map(|m| m.id.clone()).collect();
            }
            round.colours_lost = colour_teams(&mut state, station);
        }
        // it answered, and will never talk to this phone as things are: he is told what to do, at once
        Err(e) if e.retry() == Retry::Never => {
            computer.tried = true;
            computer.online = false;
            computer.refused = Some(e.said());
            round.refused = true;
        }
        // one request lost is not a computer out of reach: it is asked again at once, and only a second failure
        // says so
        Err(_) => {
            computer.fails += 1;
            round.fails = computer.fails;
            // A computer being restarted is back in a few seconds: it is only said to be out of reach when it
            // has been for a while (or has never once answered). A timer standing in for a fact: a computer
            // going away does not say whether it is coming back.
            let long = computer.failing_since.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(10);
            if computer.fails >= 2 && (long || computer.answered_at.is_none()) {
                computer.tried = true;
                computer.online = false;
            }
        }
    }
    // once the tower agrees a message is answered, stop tracking it here
    let now_open: HashSet<String> = state.computers.iter().flat_map(|s| s.open.iter().cloned()).collect();
    if now_open != open {
        state.answered.retain(|id| now_open.contains(id));
    }
    Some(round)
}

/// The teams this computer has are given their colours, beside every other computer's. Once every paired
/// computer has answered, a colour kept for a team none of them has is let go. The words for him when the
/// colours could not be kept.
fn colour_teams(state: &mut State, station: &StationId) -> Option<String> {
    let computer = state.computer(station)?;
    let rooms: Vec<String> = computer.agents.iter().map(|a| a.room.clone()).chain(computer.messages.iter().map(|m| m.room.clone())).collect();
    let lost = state.colours.see(rooms);
    if !state.computers.iter().all(|c| c.answered_at.is_some()) {
        return lost;
    }
    let all: std::collections::BTreeSet<String> = state.computers.iter().flat_map(|c| c.agents.iter().map(|a| a.room.clone()).chain(c.messages.iter().map(|m| m.room.clone()))).filter(|r| !r.is_empty()).collect();
    lost.or(state.colours.keep_only(&all))
}

/// Two computers number their agents without knowing of each other. The later one in his list is told the
/// names the earlier ones use, whenever those change, and keeps off them. `told` is what it was told last.
fn tell_taken(shared: &Shared, radio: &Radio, station: &StationId, told: &mut Option<Vec<airband_core::routing::Callsign>>) {
    let (earliest, elsewhere): (bool, Vec<airband_core::routing::Callsign>) = {
        let state = shared.state.lock().unwrap();
        let mut names: Vec<airband_core::routing::Callsign> = Vec::new();
        // the computers before this one in his list, as it is now
        let before: Vec<&Computer> = state.computers.iter().take_while(|s| s.id() != *station).collect();
        before.iter().flat_map(|s| s.agents.iter().map(|a| a.parts())).for_each(|c| if !names.contains(&c) { names.push(c) });
        (before.is_empty(), names)
    };
    if !earliest && told.as_ref() != Some(&elsewhere) && radio.taken(&elsewhere) {
        *told = Some(elsewhere);
    }
}
