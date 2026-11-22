//! What went wrong reaching the tower, and whether to try again.
use super::*;

/// What went wrong with one request. Each is a different thing to tell him and a different thing to do next;
/// none is folded into another.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum LinkError {
    /// Nothing answered that way: no network, or nothing at that address.
    Unreachable(Way),
    /// It was reached and gave no answer in the time allowed.
    TimedOut(Way),
    /// The relay is there and says his computer is not connected to it: asleep, off or offline. What he says
    /// can be left with the relay for it.
    Away,
    /// His computer is back and still being handed what was left for it: what he says now joins the end of that.
    Behind,
    /// The relay handed the request to his computer and no answer came back in the time the relay waits.
    TowerSilent,
    /// The relay answered, and not with his computer's answer.
    Relay(u16),
    /// The relay would not keep what he said.
    HoldRefused(u16),
    /// An answer came that this pairing's key does not open, or that was to another request.
    Unreadable(Way),
    /// His computer answered, and the answer was no: the status and its own explanation, when it gave one.
    Tower { status: u16, error: String },
    /// His computer took something he said and would not act on it, for the reason it gave.
    Rejected(airband_core::radio::Rejected),
}

/// What to do after a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Retry {
    /// It may well work in a moment.
    Soon,
    /// It will not work for a while: something is full, or was asked too often.
    Later,
    /// It will not work however often it is tried: he has to do something, or it cannot be sent.
    Never,
}

impl LinkError {
    pub(crate) fn retry(&self) -> Retry {
        match self {
            LinkError::Unreachable(_) | LinkError::TimedOut(_) | LinkError::Away | LinkError::Behind | LinkError::TowerSilent => Retry::Soon,
            // more than the relay carries, or not something it keeps: the same again gets the same answer
            LinkError::Relay(413) | LinkError::HoldRefused(400 | 413) => Retry::Never,
            // The relay does not know this phone for that computer. Just after pairing that is so for a few
            // seconds, until the computer has opened its side; the phone cannot tell that from a pairing that
            // was taken away, so it keeps trying and says what the relay said.
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => Retry::Soon,
            LinkError::Relay(500..=599) | LinkError::HoldRefused(500..=506 | 508..=599) => Retry::Soon,
            LinkError::Relay(_) | LinkError::HoldRefused(_) | LinkError::Unreadable(_) => Retry::Later,
            // the computer is in trouble of its own, which may pass; anything else it refuses, it refuses for good
            LinkError::Tower { status: 500..=599, .. } => Retry::Later,
            LinkError::Tower { .. } | LinkError::Rejected(_) => Retry::Never,
        }
    }
    /// His computer answered and said no.
    pub(crate) fn refused(&self) -> bool {
        matches!(self, LinkError::Tower { .. } | LinkError::Rejected(_))
    }
    /// What happened, as a sentence of its own.
    pub(crate) fn said(&self) -> String {
        format!("{}.", airband_core::prose::capital(&self.told()))
    }
    /// What happened, in words for him. No capital and no full stop: it is joined to others.
    pub(crate) fn told(&self) -> String {
        match self {
            LinkError::Unreachable(Way::Relay | Way::Kept) => "there was no connection to the relay".into(),
            LinkError::Unreachable(Way::Near) => "your computer did not answer on this network".into(),
            LinkError::Unreachable(Way::Paired) => "there was no connection to your computer".into(),
            LinkError::TimedOut(Way::Relay | Way::Kept) => "the relay did not answer in time".into(),
            LinkError::TimedOut(Way::Near | Way::Paired) => "your computer did not answer in time".into(),
            LinkError::Away => "the relay says your computer is not connected".into(),
            LinkError::Behind => "earlier messages are still on their way to your computer".into(),
            LinkError::TowerSilent => "your computer took it from the relay and has not answered yet".into(),
            LinkError::Relay(401 | 403) | LinkError::HoldRefused(401 | 403) => "the relay does not know this phone for that computer".into(),
            LinkError::Relay(413) | LinkError::HoldRefused(413) => "it is more than the relay carries in one message".into(),
            LinkError::Relay(429) | LinkError::HoldRefused(429) => "the relay was being asked too often".into(),
            LinkError::Relay(status) => format!("the relay answered {status}"),
            LinkError::HoldRefused(507) => "the relay is keeping as many messages as it can for your computer".into(),
            LinkError::HoldRefused(status) => format!("the relay could not keep it ({status})"),
            LinkError::Unreadable(Way::Near | Way::Paired) => "your computer sent an answer that could not be read".into(),
            LinkError::Unreadable(_) => "an answer came through the relay that could not be read".into(),
            LinkError::Tower { error, .. } if !error.is_empty() => format!("your computer refused it: {error}"),
            // trouble of its own, which may pass: not a no
            LinkError::Tower { status: status @ 500..=599, .. } => format!("your computer ran into trouble with it ({status})"),
            LinkError::Tower { status, .. } => format!("your computer refused it ({status})"),
            LinkError::Rejected(airband_core::radio::Rejected::TooOld) => "your computer got it more than a week after you said it, and did nothing with it".into(),
            LinkError::Rejected(airband_core::radio::Rejected::Unknown) => "your computer would not take it".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_will_never_go_is_told_as_such_and_not_as_waiting_for_a_connection() {
        let e: RadioError = LinkError::Relay(413).into();
        assert!(matches!(&e, RadioError::Told { reason } if reason == "It is more than the relay carries in one message."), "{e:?}");
        assert!(matches!(RadioError::from(LinkError::Away), RadioError::Offline));
    }
}
