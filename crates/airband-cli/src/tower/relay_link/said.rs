//! What the relay's HTTP status says, read in one place for every call the tower and `airband unpair` make to
//! it, and the one call `airband unpair` makes.
use airband_core::relay::{Keys, Route};
use airband_io::http::Ask;
use std::time::Duration;

/// What the relay's HTTP status says. What each means for a route is said where that route is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelaySaid {
    /// 200, 201: done, or (pulling) a request handed over.
    Done,
    /// 204: done with nothing to say, or (pulling) nothing waiting.
    Nothing,
    /// 401: an admission key is needed.
    NeedsAdmission,
    /// 403: the channel belongs to another computer.
    NotYours,
    /// 404: the relay does not know the channel.
    Unknown,
    /// 409: (pulling) the relay will not take this pull; its body says why
    /// ([`airband_core::relay::PullRefused`]).
    Refused,
    /// 410: closed for good (opening), or the radio stopped waiting (answering).
    Closed,
    /// 426: (pulling) the relay and this tower speak no version of the pull in common; its body says which is
    /// the older ([`airband_core::relay::PullRefused::OtherVersion`]).
    OtherVersion,
    /// Anything else, kept as the number for the log.
    Other(u16),
}

impl RelaySaid {
    pub(crate) fn of(status: u16) -> RelaySaid {
        match status {
            200 | 201 => RelaySaid::Done,
            204 => RelaySaid::Nothing,
            401 => RelaySaid::NeedsAdmission,
            403 => RelaySaid::NotYours,
            404 => RelaySaid::Unknown,
            409 => RelaySaid::Refused,
            410 => RelaySaid::Closed,
            426 => RelaySaid::OtherVersion,
            other => RelaySaid::Other(other),
        }
    }
}

/// Tell the relay a channel is finished with (`airband unpair`), so it does not keep it on record for ever.
pub(crate) fn close_channel(relay: &str, keys: &Keys) -> Result<RelaySaid, String> {
    let ask = Ask::delete(keys.to(relay, Route::Channel)).header("authorization", keys.as_tower());
    airband_io::http::Client::new(Duration::from_secs(8)).send(ask).map(|r| RelaySaid::of(r.status().as_u16())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relays_statuses_are_read_in_one_place() {
        assert_eq!(RelaySaid::of(201), RelaySaid::Done);
        assert_eq!(RelaySaid::of(204), RelaySaid::Nothing);
        assert_eq!(RelaySaid::of(409), RelaySaid::Refused);
        assert_eq!(RelaySaid::of(410), RelaySaid::Closed);
        assert_eq!(RelaySaid::of(426), RelaySaid::OtherVersion);
        assert_eq!(RelaySaid::of(418), RelaySaid::Other(418));
    }
}
