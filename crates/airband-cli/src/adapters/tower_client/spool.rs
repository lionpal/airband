//! Posting an event to the tower: kept in a folder while the tower is away, and handed over when it is back.
use super::*;

/// What came of posting an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Posted {
    /// The tower took it.
    Taken,
    /// The tower read it and refused it, saying why. Kept, it would only be refused again.
    Refused(String),
    /// The tower did not answer, and it is kept to be handed over when the tower is back.
    Kept,
    /// The tower did not answer, and it could not be kept.
    Lost,
}

/// Post one event to the tower on this machine. One the tower did not answer (it is being restarted or
/// updated) is kept in a folder and handed over when the tower is back: a report is not lost because it was
/// made in those few seconds.
pub(crate) fn post(event: &AgentEvent) -> Posted {
    match send(event) {
        Sent::Taken => Posted::Taken,
        Sent::Refused(why) => Posted::Refused(why),
        Sent::NoAnswer => spool(&config::dir().join("spool"), event),
    }
}

/// What came of handing an event to the tower.
pub(super) enum Sent {
    Taken,
    Refused(String),
    NoAnswer,
}

/// Keep an event the tower did not take in `dir`, under a name that sorts by when it was made. A full folder
/// (500 waiting) or one that cannot be written keeps nothing.
pub(super) fn spool(dir: &std::path::Path, event: &AgentEvent) -> Posted {
    let Ok(()) = airband_io::files::make_folder(dir) else { return Posted::Lost };
    let Ok(waiting) = airband_io::files::entries(dir).map(|d| d.iter().filter(|p| spooled(p)).count()) else { return Posted::Lost };
    if waiting >= 500 {
        return Posted::Lost;
    }
    let Ok(at) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else { return Posted::Lost };
    let Ok(body) = serde_json::to_vec(event) else { return Posted::Lost };
    match config::write_private(&dir.join(format!("{:020}-{}.json", at.as_nanos(), config::random_hex(4))), body) {
        Ok(_) => Posted::Kept,
        Err(_) => Posted::Lost,
    }
}

pub(super) fn send(event: &AgentEvent) -> Sent {
    let Ok(Some(reach)) = config::reach() else { return Sent::NoAnswer };
    match TowerHere::reached(&reach, Duration::from_millis(1500)).call::<at::Events>(None, &Nothing {}, Some(event)) {
        Ok(Said::Took(_)) => Sent::Taken,
        // it read the event and said no, saying why; one it cannot take now is kept for later
        Ok(Said::No(no)) if (400..500).contains(&no.status()) => Sent::Refused(no.reason().map_or_else(|| format!("it answered {}", no.status()), String::from)),
        // not there, not answering in time, or failing itself: kept for when it is back. One that took it and
        // answered too late hands it over twice, and the tower takes each event once, by its id.
        _ => Sent::NoAnswer,
    }
}

/// The tower takes what was kept for it, oldest first, by `take`: it is the tower that calls this, and it
/// takes each as it takes any event. One it refuses is let go, as one posted to it then would have been, so
/// it never holds up the rest. Something kept for more than ten minutes, or that does not read, is thrown
/// away: the tower was away for longer than an update takes, and what an agent said then is no longer news
/// to act on.
pub(crate) fn drain_spool(take: impl Fn(AgentEvent) -> Result<(), String>) {
    drain(&config::dir().join("spool"), take)
}
/// An event kept whole, by the name `spool` gives it. A file still being written is hidden beside it under
/// another name (`airband_io::files`), and is neither counted nor taken: its writer gives it its name when whole.
pub(super) fn spooled(path: &std::path::Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.') && n.ends_with(".json"))
}

pub(super) fn drain(dir: &std::path::Path, take: impl Fn(AgentEvent) -> Result<(), String>) {
    let Ok(mut kept) = airband_io::files::entries(dir).map(|d| d.into_iter().filter(|p| spooled(p)).collect::<Vec<_>>()) else { return };
    kept.sort();
    for path in kept {
        let old = airband_io::files::age(&path).is_some_and(|age| age > Duration::from_secs(600));
        if let Some(event) = airband_io::files::read_bytes(&path).ok().flatten().and_then(|b| serde_json::from_slice::<AgentEvent>(&b).ok()).filter(|_| !old)
            && let Err(why) = take(event) {
                eprintln!("airband: a kept report was refused: {why}");
            }
        airband_io::files::remove(&path).ok();
    }
}
